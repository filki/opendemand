"""Task parsing, window validation, and per-task model execution.

Rust owns planning: it sends one row per production/backtest task with a
consecutive weekly training segment and a model id. This module validates that
contract strictly, dispatches to the StatsForecast model, fails individual
tasks closed with codes (never the whole job), and produces
task_status / origin_metrics / forecast_points rows.
"""

from __future__ import annotations

import datetime as dt
import math
from dataclasses import dataclass
from pathlib import Path

import pyarrow as pa

from . import SUPPORTED_MODELS
from .errors import (
    CONTRACT_VIOLATION,
    FAIL_INSUFFICIENT_HISTORY,
    FAIL_INTERNAL_ERROR,
    FAIL_INVALID_GEOMETRY,
    FAIL_MISSING_BACKTEST_ACTUALS,
    FAIL_MISSING_TRAINING_DATA,
    FAIL_NON_FINITE_VALUES,
    FAIL_DUPLICATE_OBSERVATION,
    FAIL_UNSUPPORTED_MODEL,
    EngineError,
    TaskFailure,
)
from .metrics import compute_metrics
from .modeling import MODEL_FLOOR, forecast

HORIZON_WEEKS = 12
BACKTEST_ORIGIN_RANGE = (1, 4)
KIND_PRODUCTION = "production"
KIND_BACKTEST = "backtest"
STATUS_COMPLETED = "completed"
STATUS_FAILED = "failed"

# Input tables are read in bounded row chunks so that the whole table is never
# materialised as Python objects at once. At benchmark scale (1.56M observation
# rows) a single `to_pylist()` peaks hundreds of megabytes per process; a chunk
# keeps the transient to a few megabytes while preserving row order exactly.
ROW_CHUNK = 65_536


def _iter_rows(table: pa.Table, chunk_size: int = ROW_CHUNK):
    """Yield one Python dict per row, chunk by chunk, in file order."""
    for batch in table.to_batches(max_chunksize=chunk_size):
        yield from batch.to_pylist()


@dataclass(frozen=True)
class TaskRow:
    task_id: str
    product_id: str
    location_id: str
    model_id: str
    kind: str
    origin_index: int | None
    train_start_week: dt.date
    train_end_week: dt.date
    forecast_start_week: dt.date
    train_size: int
    horizon_weeks: int


@dataclass(frozen=True)
class TaskOutcome:
    task: TaskRow
    status_row: dict
    metric_row: dict | None
    point_rows: list[dict]


class ObservationIndex:
    """Series -> week -> quantity lookup with duplicate detection."""

    def __init__(self, values: dict, duplicates: set):
        self._values = values
        self._duplicates = duplicates

    @classmethod
    def from_table(cls, table: pa.Table) -> "ObservationIndex":
        values: dict[tuple[str, str], dict[dt.date, float | None]] = {}
        duplicates: set[tuple[str, str, dt.date]] = set()
        for row in _iter_rows(table):
            key = (row["product_id"], row["location_id"])
            week = row["week_start"]
            series = values.setdefault(key, {})
            if week in series:
                duplicates.add((key[0], key[1], week))
            else:
                series[week] = row["quantity"]
        return cls(values, duplicates)

    def lookup(self, product_id: str, location_id: str, week: dt.date) -> tuple[str, float | None]:
        """Return ``(kind, value)`` with kind in value|missing|absent|duplicate."""
        if (product_id, location_id, week) in self._duplicates:
            return "duplicate", None
        series = self._values.get((product_id, location_id))
        if series is None or week not in series:
            return "absent", None
        quantity = series[week]
        if quantity is None:
            return "missing", None
        return "value", float(quantity)


def parse_tasks(table: pa.Table) -> list[TaskRow]:
    """Parse the tasks table; structural row problems are whole-job violations."""
    seen: set[str] = set()
    tasks: list[TaskRow] = []
    for row in _iter_rows(table):
        task_id = row["task_id"]
        if not isinstance(task_id, str) or not task_id:
            raise EngineError(CONTRACT_VIOLATION, "tasks file contains an empty task_id")
        if task_id in seen:
            raise EngineError(CONTRACT_VIOLATION, "tasks file contains a duplicate task_id")
        seen.add(task_id)
        tasks.append(
            TaskRow(
                task_id=task_id,
                product_id=row["product_id"],
                location_id=row["location_id"],
                model_id=row["model_id"],
                kind=row["kind"],
                origin_index=row["origin_index"],
                train_start_week=row["train_start_week"],
                train_end_week=row["train_end_week"],
                forecast_start_week=row["forecast_start_week"],
                train_size=row["train_size"],
                horizon_weeks=row["horizon_weeks"],
            )
        )
    return tasks


def load_observation_index(job_dir) -> "ObservationIndex":
    """Read ``observations.arrow`` from ``job_dir`` and build the lookup index.

    Used by a pool worker to obtain its own index without transferring the whole
    observation table over the IPC pipe.
    """
    from . import arrowio

    table = arrowio.read_input_table(
        Path(job_dir), arrowio.OBSERVATIONS_FILE, arrowio.OBSERVATIONS_SCHEMA, "observations"
    )
    return ObservationIndex.from_table(table)


def run_task(task: TaskRow, index: ObservationIndex) -> TaskOutcome:
    """Run one task and convert a task-level failure into a failed outcome.

    This is the single entry point shared by the sequential path and the pool
    worker, so both produce byte-identical outcomes for the same inputs.
    """
    try:
        return execute_task(task, index)
    except TaskFailure as failure:
        return failed_outcome(task, failure.code, failure.detail)
    except Exception:
        return failed_outcome(task, FAIL_INTERNAL_ERROR, "unexpected task failure")


def failed_outcome(task: TaskRow, code: str, detail: str) -> TaskOutcome:
    return TaskOutcome(
        task=task,
        status_row={
            "task_id": task.task_id,
            "status": STATUS_FAILED,
            "failure_code": code,
            "failure_detail": detail,
        },
        metric_row=None,
        point_rows=[],
    )


def _fail(code: str, detail: str) -> TaskFailure:
    return TaskFailure(code, detail)


def _require_monday(value: dt.date) -> None:
    if value.weekday() != 0:
        raise _fail(FAIL_INVALID_GEOMETRY, "task week dates must be Mondays")


def _validate_geometry(task: TaskRow) -> None:
    if task.kind not in (KIND_PRODUCTION, KIND_BACKTEST):
        raise _fail(FAIL_INVALID_GEOMETRY, "kind must be production or backtest")
    if task.kind == KIND_PRODUCTION and task.origin_index is not None:
        raise _fail(FAIL_INVALID_GEOMETRY, "production tasks must not carry an origin_index")
    if task.kind == KIND_BACKTEST:
        low, high = BACKTEST_ORIGIN_RANGE
        if task.origin_index is None or not (low <= task.origin_index <= high):
            raise _fail(FAIL_INVALID_GEOMETRY, "backtest tasks require origin_index in 1..4")
    if task.horizon_weeks != HORIZON_WEEKS:
        raise _fail(FAIL_INVALID_GEOMETRY, "horizon_weeks must be exactly 12")

    _require_monday(task.train_start_week)
    _require_monday(task.train_end_week)
    _require_monday(task.forecast_start_week)

    span_days = (task.train_end_week - task.train_start_week).days
    expected_size = span_days // 7 + 1 if span_days >= 0 and span_days % 7 == 0 else None
    if expected_size is None or expected_size != task.train_size:
        raise _fail(FAIL_INVALID_GEOMETRY, "train_size does not match the training window")
    if task.forecast_start_week != task.train_end_week + dt.timedelta(days=7):
        raise _fail(FAIL_INVALID_GEOMETRY, "forecast_start_week must follow train_end_week by one week")


def _require_history_floor(task: TaskRow) -> None:
    floor = MODEL_FLOOR[task.model_id]
    if task.train_size < floor:
        raise _fail(
            FAIL_INSUFFICIENT_HISTORY,
            f"training segment is shorter than the {task.model_id} history floor of {floor} weeks",
        )


def _read_numeric_window(
    index: ObservationIndex,
    task: TaskRow,
    start: dt.date,
    size: int,
    missing_code: str,
    window: str,
) -> list[float]:
    values: list[float] = []
    for offset in range(size):
        week = start + dt.timedelta(days=7 * offset)
        kind, value = index.lookup(task.product_id, task.location_id, week)
        if kind == "duplicate":
            raise _fail(FAIL_DUPLICATE_OBSERVATION, f"duplicate weekly observation inside the {window} window")
        if kind in ("absent", "missing") or value is None:
            raise _fail(missing_code, f"incomplete {window} window for the series")
        if not math.isfinite(value):
            raise _fail(FAIL_NON_FINITE_VALUES, f"non-finite observation inside the {window} window")
        values.append(value)
    return values


def execute_task(task: TaskRow, index: ObservationIndex) -> TaskOutcome:
    """Run one task; raise TaskFailure for task-level problems."""
    if task.model_id not in SUPPORTED_MODELS:
        raise _fail(FAIL_UNSUPPORTED_MODEL, f"unknown model id: {task.model_id}")
    _validate_geometry(task)
    _require_history_floor(task)

    history = _read_numeric_window(
        index, task, task.train_start_week, task.train_size, FAIL_MISSING_TRAINING_DATA, "training"
    )
    actuals: list[float] | None = None
    if task.kind == KIND_BACKTEST:
        actuals = _read_numeric_window(
            index,
            task,
            task.forecast_start_week,
            task.horizon_weeks,
            FAIL_MISSING_BACKTEST_ACTUALS,
            "backtest",
        )

    forecasts = forecast(task.model_id, history, task.horizon_weeks)

    metric_row: dict | None = None
    if actuals is not None:
        wape, mae, metric_kind, metric_value = compute_metrics(actuals, forecasts)
        metric_row = {
            "task_id": task.task_id,
            "origin_week_start": task.train_end_week,
            "train_size": task.train_size,
            "wape": wape,
            "mae": mae,
            "metric_kind": metric_kind,
            "metric_value": metric_value,
        }

    point_rows = [
        {
            "task_id": task.task_id,
            "week_start": task.forecast_start_week + dt.timedelta(days=7 * offset),
            "value": value,
        }
        for offset, value in enumerate(forecasts)
    ]
    return TaskOutcome(
        task=task,
        status_row={
            "task_id": task.task_id,
            "status": STATUS_COMPLETED,
            "failure_code": None,
            "failure_detail": None,
        },
        metric_row=metric_row,
        point_rows=point_rows,
    )


# ---------------------------------------------------------------------------
# Pool worker entry points
# ---------------------------------------------------------------------------
#
# A worker builds the observation index once, in ``init_worker``, then serves
# tasks through ``run_task_in_worker``. Both are module-level so the spawn start
# method can pickle them by name on every platform. The worker never touches
# SQLite, the cancel flag, or the output files: it only fits and predicts.

_worker_index: "ObservationIndex | None" = None


def init_worker(job_dir: str) -> None:
    """ProcessPoolExecutor initializer: build this worker's observation index."""
    global _worker_index
    _worker_index = load_observation_index(Path(job_dir))


def run_task_in_worker(task: TaskRow) -> TaskOutcome:
    """ProcessPoolExecutor task target: run one task with the worker's index."""
    if _worker_index is None:  # pragma: no cover - requires a broken initializer
        return failed_outcome(task, FAIL_INTERNAL_ERROR, "worker was not initialized")
    return run_task(task, _worker_index)
