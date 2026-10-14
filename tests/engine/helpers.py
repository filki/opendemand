"""Shared helpers for engine tests.

The Arrow schemas below are an independent transcription of
``docs/architecture/contracts/ARROW_SCHEMAS.md`` so that engine schema drift is
caught by the tests. Nothing here imports the engine package at module import
time (the red phase must fail at test execution, not collection).
"""

from __future__ import annotations

import datetime as dt
import json
import os
import queue
import subprocess
import sys
import threading
import time
import uuid
from pathlib import Path

import pyarrow as pa

REPO_ROOT = Path(__file__).resolve().parents[2]
PYTHON_DIR = REPO_ROOT / "python"
DEMO_CSV = REPO_ROOT / "tests" / "fixtures" / "demo.csv"

PROTOCOL_ID = "opendemand.sidecar/1"
CONTRACT_ID = "opendemand.arrow/1"


def meta(table: str) -> dict[bytes, bytes]:
    return {
        b"opendemand.contract": CONTRACT_ID.encode("utf-8"),
        b"opendemand.table": table.encode("utf-8"),
    }


OBSERVATIONS_SCHEMA = pa.schema(
    [
        pa.field("product_id", pa.utf8(), nullable=False),
        pa.field("location_id", pa.utf8(), nullable=False),
        pa.field("week_start", pa.date32(), nullable=False),
        pa.field("quantity", pa.float64(), nullable=True),
        pa.field("status", pa.utf8(), nullable=False),
    ],
    metadata=meta("observations"),
)

TASKS_SCHEMA = pa.schema(
    [
        pa.field("task_id", pa.utf8(), nullable=False),
        pa.field("product_id", pa.utf8(), nullable=False),
        pa.field("location_id", pa.utf8(), nullable=False),
        pa.field("model_id", pa.utf8(), nullable=False),
        pa.field("kind", pa.utf8(), nullable=False),
        pa.field("origin_index", pa.int32(), nullable=True),
        pa.field("train_start_week", pa.date32(), nullable=False),
        pa.field("train_end_week", pa.date32(), nullable=False),
        pa.field("forecast_start_week", pa.date32(), nullable=False),
        pa.field("train_size", pa.int32(), nullable=False),
        pa.field("horizon_weeks", pa.int32(), nullable=False),
    ],
    metadata=meta("tasks"),
)

TASK_STATUS_SCHEMA = pa.schema(
    [
        pa.field("task_id", pa.utf8(), nullable=False),
        pa.field("status", pa.utf8(), nullable=False),
        pa.field("failure_code", pa.utf8(), nullable=True),
        pa.field("failure_detail", pa.utf8(), nullable=True),
    ],
    metadata=meta("task_status"),
)

ORIGIN_METRICS_SCHEMA = pa.schema(
    [
        pa.field("task_id", pa.utf8(), nullable=False),
        pa.field("origin_week_start", pa.date32(), nullable=False),
        pa.field("train_size", pa.int32(), nullable=False),
        pa.field("wape", pa.float64(), nullable=True),
        pa.field("mae", pa.float64(), nullable=False),
        pa.field("metric_kind", pa.utf8(), nullable=False),
        pa.field("metric_value", pa.float64(), nullable=False),
    ],
    metadata=meta("origin_metrics"),
)

FORECAST_POINTS_SCHEMA = pa.schema(
    [
        pa.field("task_id", pa.utf8(), nullable=False),
        pa.field("week_start", pa.date32(), nullable=False),
        pa.field("value", pa.float64(), nullable=False),
    ],
    metadata=meta("forecast_points"),
)

OUTPUT_SCHEMAS = {
    "task_status": (TASK_STATUS_SCHEMA, "task_status.arrow"),
    "origin_metrics": (ORIGIN_METRICS_SCHEMA, "origin_metrics.arrow"),
    "forecast_points": (FORECAST_POINTS_SCHEMA, "forecast_points.arrow"),
}


# ---------------------------------------------------------------------------
# Time helpers
# ---------------------------------------------------------------------------


def monday(iso: str) -> dt.date:
    value = dt.date.fromisoformat(iso)
    assert value.weekday() == 0, iso
    return value


def add_weeks(value: dt.date, count: int) -> dt.date:
    return value + dt.timedelta(days=7 * count)


def week_grid(start: dt.date, size: int) -> list[dt.date]:
    return [add_weeks(start, i) for i in range(size)]


def status_for(quantity: float | None) -> str:
    if quantity is None:
        return "missing"
    if quantity == 0:
        return "zero"
    if quantity < 0:
        return "return"
    return "known"


def weekly_rows(
    product_id: str,
    location_id: str,
    start: dt.date,
    values: list[float | None],
) -> list[dict]:
    return [
        {
            "product_id": product_id,
            "location_id": location_id,
            "week_start": add_weeks(start, i),
            "quantity": value,
            "status": status_for(value),
        }
        for i, value in enumerate(values)
    ]


def series_uuid(label: str) -> str:
    return str(uuid.uuid5(uuid.NAMESPACE_URL, "opendemand/test/" + label))


# ---------------------------------------------------------------------------
# Task row builder
# ---------------------------------------------------------------------------


def make_task(
    task_id: str,
    product_id: str,
    location_id: str,
    kind: str,
    train_start: dt.date,
    train_end: dt.date,
    forecast_start: dt.date | None = None,
    train_size: int | None = None,
    model_id: str = "naive",
    origin_index: int | None = None,
    horizon_weeks: int = 12,
) -> dict:
    if forecast_start is None:
        forecast_start = add_weeks(train_end, 1)
    if train_size is None:
        train_size = ((train_end - train_start).days // 7) + 1 if train_start <= train_end else 0
    return {
        "task_id": task_id,
        "product_id": product_id,
        "location_id": location_id,
        "model_id": model_id,
        "kind": kind,
        "origin_index": origin_index,
        "train_start_week": train_start,
        "train_end_week": train_end,
        "forecast_start_week": forecast_start,
        "train_size": train_size,
        "horizon_weeks": horizon_weeks,
    }


def backtest_task(
    task_id: str,
    product_id: str,
    location_id: str,
    weeks: list[dt.date],
    origin_index: int,
    train_size: int,
    **overrides,
) -> dict:
    train_end = weeks[train_size - 1]
    return make_task(
        task_id,
        product_id,
        location_id,
        kind="backtest",
        train_start=weeks[0],
        train_end=train_end,
        forecast_start=weeks[train_size],
        train_size=train_size,
        origin_index=origin_index,
        **overrides,
    )


def production_task(
    task_id: str,
    product_id: str,
    location_id: str,
    weeks: list[dt.date],
    **overrides,
) -> dict:
    return make_task(
        task_id,
        product_id,
        location_id,
        kind="production",
        train_start=weeks[0],
        train_end=weeks[-1],
        forecast_start=add_weeks(weeks[-1], 1),
        train_size=len(weeks),
        **overrides,
    )


# ---------------------------------------------------------------------------
# Arrow IO helpers
# ---------------------------------------------------------------------------


def write_rows(path: Path, schema: pa.Schema, rows: list[dict]) -> None:
    table = pa.Table.from_pylist(rows, schema=schema)
    write_table(path, schema, table)


def write_table(path: Path, schema: pa.Schema, table: pa.Table) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with open(path, "wb") as handle:
        with pa.ipc.new_file(handle, schema) as writer:
            writer.write_table(table)


def write_ipc_stream(path: Path, schema: pa.Schema, rows: list[dict]) -> None:
    table = pa.Table.from_pylist(rows, schema=schema)
    with open(path, "wb") as handle:
        with pa.ipc.new_stream(handle, schema) as writer:
            writer.write_table(table)


def replace_field(schema: pa.Schema, name: str, field: pa.Field) -> pa.Schema:
    fields = [field if f.name == name else f for f in schema]
    return pa.schema(fields, metadata=schema.metadata)


def without_metadata(schema: pa.Schema) -> pa.Schema:
    return pa.schema(list(schema))


def read_table(path: Path) -> pa.Table:
    return pa.ipc.open_file(str(path)).read_all()


def read_schema(path: Path) -> pa.Schema:
    return pa.ipc.open_file(str(path)).schema


def decoded_metadata(schema: pa.Schema) -> dict[str, str]:
    return {key.decode("utf-8"): value.decode("utf-8") for key, value in (schema.metadata or {}).items()}


def write_inputs(
    job_dir: Path,
    observation_rows: list[dict],
    task_rows: list[dict],
    observations_schema: pa.Schema = OBSERVATIONS_SCHEMA,
    tasks_schema: pa.Schema = TASKS_SCHEMA,
) -> None:
    write_rows(job_dir / "observations.arrow", observations_schema, observation_rows)
    write_rows(job_dir / "tasks.arrow", tasks_schema, task_rows)


def standard_job(
    job_dir: Path,
    observation_rows: list[dict],
    task_rows: list[dict],
    **kwargs,
) -> None:
    write_inputs(job_dir, observation_rows, task_rows, **kwargs)


# ---------------------------------------------------------------------------
# Protocol helpers
# ---------------------------------------------------------------------------


def job_request(
    job_id: str = "job-1",
    protocol: str = PROTOCOL_ID,
    contract: str = CONTRACT_ID,
    input_files: dict | None = None,
    message_type: str = "job.request",
) -> str:
    envelope = {
        "protocol": protocol,
        "type": message_type,
        "job_id": job_id,
        "contract": contract,
        "input_files": input_files
        if input_files is not None
        else {"observations": "observations.arrow", "tasks": "tasks.arrow"},
    }
    return json.dumps(envelope, separators=(",", ":"))


def cancel_message(job_id: str = "job-1", protocol: str = PROTOCOL_ID) -> str:
    return json.dumps(
        {"protocol": protocol, "type": "job.cancel", "job_id": job_id},
        separators=(",", ":"),
    )


CANCEL_FILE = "cancel.flag"


def request_cancel(job_dir: Path) -> None:
    """Request cancellation the way the Rust core does: create the flag file."""
    (Path(job_dir) / CANCEL_FILE).touch()


TERMINAL_TYPES = ("job.result", "job.error", "job.cancelled")


class FakeStdin:
    def __init__(self, lines: list[str] | None = None):
        self._queue: queue.Queue = queue.Queue()
        for line in lines or []:
            self.feed(line)

    def feed(self, line: str) -> None:
        self._queue.put(line if line.endswith("\n") else line + "\n")

    def eof(self) -> None:
        self._queue.put(None)

    def readline(self) -> str:
        item = self._queue.get()
        return "" if item is None else item

    def unread(self) -> int:
        """Lines (not EOF markers) still waiting to be read."""
        return sum(1 for item in list(self._queue.queue) if item is not None)


class FakeStdout:
    def __init__(self) -> None:
        self._lock = threading.Lock()
        self._buffer = ""
        self.lines: list[str] = []

    def write(self, text: str) -> int:
        with self._lock:
            self._buffer += text
            while "\n" in self._buffer:
                line, self._buffer = self._buffer.split("\n", 1)
                if line:
                    self.lines.append(line)
        return len(text)

    def flush(self) -> None:
        pass

    def envelopes(self) -> list[dict]:
        return [json.loads(line) for line in list(self.lines)]

    def terminal(self) -> dict:
        for envelope in reversed(self.envelopes()):
            if envelope.get("type") in TERMINAL_TYPES:
                return envelope
        raise AssertionError("no terminal envelope on stdout")


def run_inproc(
    job_dir: Path,
    send_lines: list[str] | None = None,
    eof: bool = True,
    heartbeat_interval: float = 15.0,
) -> tuple[int, FakeStdout, str]:
    """Run the public engine entry point over fake text streams."""
    stdin = FakeStdin(send_lines)
    if eof:
        stdin.eof()
    return run_inproc_streams(job_dir, stdin, heartbeat_interval=heartbeat_interval)


def run_inproc_streams(
    job_dir: Path, stdin: "FakeStdin", heartbeat_interval: float = 15.0
) -> tuple[int, FakeStdout, str]:
    from opendemand_engine import engine

    stdout = FakeStdout()
    import io

    stderr = io.StringIO()
    code = engine.run_job(
        Path(job_dir), stdin, stdout, stderr, heartbeat_interval=heartbeat_interval
    )
    return code, stdout, stderr.getvalue()


def output_files_present(job_dir: Path) -> list[str]:
    return sorted(
        name
        for name in ("task_status.arrow", "origin_metrics.arrow", "forecast_points.arrow")
        if (Path(job_dir) / name).exists()
    )


# ---------------------------------------------------------------------------
# Real-process driver
# ---------------------------------------------------------------------------


class EngineProcess:
    """Spawns the real CLI (``python -m opendemand_engine --job-dir ...``)."""

    def __init__(self, job_dir: Path, args_extra: list[str] | None = None, cwd: Path | None = None):
        self.job_dir = Path(job_dir)
        env = dict(os.environ)
        existing = env.get("PYTHONPATH")
        env["PYTHONPATH"] = str(PYTHON_DIR) if not existing else str(PYTHON_DIR) + os.pathsep + existing
        command = [sys.executable, "-m", "opendemand_engine", "--job-dir", str(self.job_dir)]
        if args_extra:
            command.extend(args_extra)
        self.proc = subprocess.Popen(
            command,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            bufsize=1,
            cwd=str(cwd if cwd is not None else self.job_dir),
            env=env,
        )
        self.envelopes: list[dict] = []
        self._stdout_queue: queue.Queue = queue.Queue()
        self._stderr_parts: list[str] = []
        self._stdout_thread = threading.Thread(target=self._pump_stdout, daemon=True)
        self._stderr_thread = threading.Thread(target=self._pump_stderr, daemon=True)
        self._stdout_thread.start()
        self._stderr_thread.start()

    def _pump_stdout(self) -> None:
        assert self.proc.stdout is not None
        for line in self.proc.stdout:
            self._stdout_queue.put(line)
        self._stdout_queue.put(None)

    def _pump_stderr(self) -> None:
        assert self.proc.stderr is not None
        for line in self.proc.stderr:
            self._stderr_parts.append(line)

    def send(self, line: str) -> None:
        assert self.proc.stdin is not None
        self.proc.stdin.write(line + "\n")
        self.proc.stdin.flush()

    def close_stdin(self) -> None:
        if self.proc.stdin is not None and not self.proc.stdin.closed:
            self.proc.stdin.close()

    def read_envelope(self, timeout: float = 120.0) -> dict | None:
        try:
            item = self._stdout_queue.get(timeout=timeout)
        except queue.Empty as exc:
            raise AssertionError("timed out waiting for engine stdout") from exc
        if item is None:
            return None
        envelope = json.loads(item)
        self.envelopes.append(envelope)
        return envelope

    def read_until_terminal(self, timeout: float = 180.0) -> dict:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            envelope = self.read_envelope(timeout=max(1.0, deadline - time.monotonic()))
            if envelope is None:
                raise AssertionError("engine exited before a terminal envelope")
            if envelope.get("type") in TERMINAL_TYPES:
                return envelope
        raise AssertionError("timed out waiting for terminal envelope")

    @property
    def stderr_text(self) -> str:
        return "".join(self._stderr_parts)

    def drain_stdout(self, timeout: float = 5.0) -> list[str]:
        """Return any raw stdout lines left after the terminal envelope."""
        leftovers: list[str] = []
        while True:
            try:
                item = self._stdout_queue.get(timeout=timeout)
            except queue.Empty:
                break
            if item is None:
                break
            leftovers.append(item)
        return leftovers

    def finish(self, timeout: float = 60.0) -> int:
        code = self.proc.wait(timeout=timeout)
        self._stdout_thread.join(timeout=5.0)
        self._stderr_thread.join(timeout=5.0)
        for stream in (self.proc.stdin, self.proc.stdout, self.proc.stderr):
            try:
                if stream is not None and not stream.closed:
                    stream.close()
            except OSError:
                pass
        return code


# ---------------------------------------------------------------------------
# Demo fixture helpers
# ---------------------------------------------------------------------------


def load_demo_series() -> dict[str, dict[dt.date, float]]:
    """Read tests/fixtures/demo.csv (60/20/6 weeks) into series->week->quantity."""
    import csv

    series: dict[str, dict[dt.date, float]] = {}
    with open(DEMO_CSV, newline="", encoding="utf-8") as handle:
        for row in csv.DictReader(handle):
            weeks = series.setdefault(row["series_key"], {})
            weeks[monday(row["week_start"])] = float(row["quantity"])
    return series


def demo_observation_rows(series: dict[str, dict[dt.date, float]]) -> list[dict]:
    rows: list[dict] = []
    for key, weeks in series.items():
        product_id = series_uuid(key)
        location_id = series_uuid("location/default")
        for week, quantity in sorted(weeks.items()):
            rows.append(
                {
                    "product_id": product_id,
                    "location_id": location_id,
                    "week_start": week,
                    "quantity": quantity,
                    "status": status_for(quantity),
                }
            )
    return rows


def plan_demo_tasks(series: dict[str, dict[dt.date, float]]) -> list[dict]:
    """Mirror the ADR-0006 plan: k=max(0,min(4,floor((H-12)/12))) trailing windows."""
    tasks: list[dict] = []
    for key in sorted(series):
        weeks = sorted(series[key])
        product_id = series_uuid(key)
        location_id = series_uuid("location/default")
        history_size = len(weeks)
        windows = max(0, min(4, (history_size - 12) // 12))
        for origin in range(1, windows + 1):
            train_size = history_size - 12 * (windows - origin + 1)
            tasks.append(
                backtest_task(
                    series_uuid(f"{key}/backtest/{origin}"),
                    product_id,
                    location_id,
                    weeks,
                    origin,
                    train_size,
                )
            )
        tasks.append(
            production_task(
                series_uuid(f"{key}/production"),
                product_id,
                location_id,
                weeks,
            )
        )
    return tasks
