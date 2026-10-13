"""Job orchestration: hello -> job.request -> progress -> terminal envelope.

Design constraints honoured here:

- Stdlib-only imports at module level; PyArrow and StatsForecast are imported
  only after ``hello`` has been flushed so the Rust orchestrator sees liveness
  well within its 5 second hello budget.
- Strict protocol and job correlation; cancellation is honoured between tasks
  and before any output is accepted.
- Outputs are written to temp names, closed and fsynced, then atomically
  renamed before ``job.result``; the manifest carries fixed paths, row counts
  and lowercase SHA-256 digests.
- stderr carries only phases, counts and codes - never business values.
"""

from __future__ import annotations

import threading
from pathlib import Path

from . import (
    ARROW_CONTRACT_ID,
    OBSERVATIONS_FILE,
    PROTOCOL_ID,
    SUPPORTED_MODELS,
    TASKS_FILE,
    __version__,
)
from .errors import (
    CONTRACT_VIOLATION,
    ENGINE_START_FAILED,
    FAIL_INTERNAL_ERROR,
    INTERNAL_ERROR,
    PROTOCOL_MISMATCH,
    EngineError,
    JobCancelled,
    TaskFailure,
)
from .protocol import EnvelopeWriter, InboxReader

EXIT_OK = 0
EXIT_JOB_ERROR = 1
EXIT_NO_REQUEST = 2

EXPECTED_INPUT_FILES = {"observations": OBSERVATIONS_FILE, "tasks": TASKS_FILE}


# ---------------------------------------------------------------------------
# Public entry point
# ---------------------------------------------------------------------------


def run_job(
    job_dir,
    stdin,
    stdout,
    stderr,
    *,
    heartbeat_interval: float = 15.0,
) -> int:
    """Run one sidecar job over text streams; return the process exit code."""
    job_dir = Path(job_dir)
    if not job_dir.is_dir():
        raise EngineError(ENGINE_START_FAILED, "job directory does not exist or is not a directory")

    writer = EnvelopeWriter(stdout)
    writer.send(_hello_envelope())
    _log(stderr, "hello sent")

    reader = InboxReader(stdin)
    reader.start()
    kind, payload = reader.wait_initial()
    _log(stderr, f"initial {kind}")

    if kind == "eof":
        _log(stderr, "stdin closed before job.request")
        return EXIT_NO_REQUEST
    if kind == "error":
        job_id = _usable_job_id(payload)
        if job_id is None:
            _log(stderr, "protocol error before a usable job.request")
            return EXIT_NO_REQUEST
        writer.send(_error_envelope(job_id, PROTOCOL_MISMATCH, "invalid sidecar protocol message"))
        _log(stderr, f"terminal type=job.error code={PROTOCOL_MISMATCH}")
        return EXIT_JOB_ERROR

    request = payload or {}
    job_id = _usable_job_id(request)
    if job_id is None:
        _log(stderr, "job.request without a usable job_id")
        return EXIT_NO_REQUEST
    try:
        _validate_request(request)
    except EngineError as error:
        writer.send(_error_envelope(job_id, error.code, error.message, error.retryable))
        _log(stderr, f"terminal type=job.error code={error.code}")
        return EXIT_JOB_ERROR

    tracker = ProgressTracker(writer, job_id)
    heartbeat = Heartbeat(heartbeat_interval, tracker.flush)
    heartbeat.start()
    written: list[Path] = []
    try:
        from . import arrowio  # PyArrow only after hello is on the wire
        from .tasks import ObservationIndex, execute_task, failed_outcome, parse_tasks
        _log(stderr, "imports ready")

        _raise_on_interrupt(reader)
        _log(stderr, "interrupt check done")
        tasks_table = arrowio.read_input_table(
            job_dir, arrowio.TASKS_FILE, arrowio.TASKS_SCHEMA, "tasks"
        )
        _log(stderr, "tasks table read")
        task_rows = parse_tasks(tasks_table)
        _log(stderr, "tasks parsed")
        total = len(task_rows)
        tracker.update("validate_input", 0, total)
        _log(stderr, f"phase validate_input tasks={total}")

        _raise_on_interrupt(reader)
        observations_table = arrowio.read_input_table(
            job_dir, arrowio.OBSERVATIONS_FILE, arrowio.OBSERVATIONS_SCHEMA, "observations"
        )
        index = ObservationIndex.from_table(observations_table)
        _raise_on_interrupt(reader)

        outcomes = _execute_all(task_rows, index, reader, tracker, stderr)
        tracker.update("write_output", total, total)
        _log(stderr, f"phase write_output completed={total}")

        _raise_on_interrupt(reader)
        manifest, written = arrowio.write_output_tables(job_dir, _collect_rows(outcomes))
        _raise_on_interrupt(reader)

        writer.send(_result_envelope(job_id, manifest))
        _log(stderr, "terminal type=job.result")
        return EXIT_OK
    except JobCancelled:
        _remove_written(written)
        writer.send({"protocol": PROTOCOL_ID, "type": "job.cancelled", "job_id": job_id})
        _log(stderr, "terminal type=job.cancelled")
        return EXIT_OK
    except EngineError as error:
        _remove_written(written)
        writer.send(_error_envelope(job_id, error.code, error.message, error.retryable))
        _log(stderr, f"terminal type=job.error code={error.code}")
        return EXIT_JOB_ERROR
    except Exception:
        _remove_written(written)
        writer.send(_error_envelope(job_id, INTERNAL_ERROR, "unexpected engine failure"))
        _log(stderr, f"terminal type=job.error code={INTERNAL_ERROR}")
        return EXIT_JOB_ERROR
    finally:
        heartbeat.stop()


# ---------------------------------------------------------------------------
# Task execution
# ---------------------------------------------------------------------------


def _execute_all(task_rows, index, reader, tracker, stderr):
    from .tasks import execute_task, failed_outcome

    total = len(task_rows)
    ordered = list(enumerate(task_rows))
    groups = (
        ("backtest", [(position, task) for position, task in ordered if task.kind == "backtest"]),
        ("production", [(position, task) for position, task in ordered if task.kind != "backtest"]),
    )
    outcomes: list = [None] * total
    done = 0
    for phase, group in groups:
        if not group:
            continue
        _log(stderr, f"phase {phase} tasks={len(group)}")
        tracker.update(phase, done, total)
        for position, task in group:
            _raise_on_interrupt(reader)
            try:
                outcome = execute_task(task, index)
            except TaskFailure as failure:
                outcome = failed_outcome(task, failure.code, failure.detail)
            except Exception:
                outcome = failed_outcome(task, FAIL_INTERNAL_ERROR, "unexpected task failure")
            outcomes[position] = outcome
            done += 1
            tracker.update(phase, done, total)
            _raise_on_interrupt(reader)
    return [outcome for outcome in outcomes]


def _collect_rows(outcomes) -> dict[str, list[dict]]:
    return {
        "task_status": [outcome.status_row for outcome in outcomes],
        "origin_metrics": [
            outcome.metric_row for outcome in outcomes if outcome.metric_row is not None
        ],
        "forecast_points": [
            row for outcome in outcomes for row in outcome.point_rows
        ],
    }


# ---------------------------------------------------------------------------
# Interruption and request validation
# ---------------------------------------------------------------------------


def _raise_on_interrupt(reader: InboxReader) -> None:
    if reader.cancel_event.is_set():
        raise JobCancelled()
    if reader.protocol_error is not None:
        raise EngineError(
            PROTOCOL_MISMATCH,
            "sidecar protocol correlation was violated after job.request",
        )


def _usable_job_id(obj) -> str | None:
    if not isinstance(obj, dict):
        return None
    job_id = obj.get("job_id")
    if isinstance(job_id, str) and job_id:
        return job_id
    return None


def _validate_request(request: dict) -> None:
    if request.get("protocol") != PROTOCOL_ID:
        raise EngineError(PROTOCOL_MISMATCH, "unsupported sidecar protocol id")
    if request.get("type") != "job.request":
        raise EngineError(PROTOCOL_MISMATCH, "expected a job.request message")
    if request.get("contract") != ARROW_CONTRACT_ID:
        raise EngineError(CONTRACT_VIOLATION, "unsupported arrow contract id")
    input_files = request.get("input_files")
    if not isinstance(input_files, dict) or set(input_files.keys()) != set(EXPECTED_INPUT_FILES):
        raise EngineError(
            CONTRACT_VIOLATION,
            "input_files must name exactly the observations and tasks files",
        )
    for key, expected in EXPECTED_INPUT_FILES.items():
        if input_files.get(key) != expected:
            raise EngineError(
                CONTRACT_VIOLATION,
                f"input_files.{key} must be the fixed filename {expected}",
            )


# ---------------------------------------------------------------------------
# Envelopes, progress and logging
# ---------------------------------------------------------------------------


def _hello_envelope() -> dict:
    return {
        "protocol": PROTOCOL_ID,
        "type": "hello",
        "job_id": None,
        "engine_version": __version__,
        "contracts": [ARROW_CONTRACT_ID],
        "models": list(SUPPORTED_MODELS),
    }


def _error_envelope(job_id: str, code: str, message: str, retryable: bool = False) -> dict:
    return {
        "protocol": PROTOCOL_ID,
        "type": "job.error",
        "job_id": job_id,
        "code": code,
        "message": message,
        "retryable": bool(retryable),
    }


def _result_envelope(job_id: str, manifest: dict) -> dict:
    return {
        "protocol": PROTOCOL_ID,
        "type": "job.result",
        "job_id": job_id,
        "output_files": manifest,
    }


class ProgressTracker:
    def __init__(self, writer: EnvelopeWriter, job_id: str):
        self._writer = writer
        self._job_id = job_id
        self._lock = threading.Lock()
        self._state: tuple[str, int, int] | None = None

    def update(self, phase: str, completed: int, total: int) -> None:
        with self._lock:
            self._state = (phase, completed, total)
        self.flush()

    def flush(self) -> None:
        with self._lock:
            state = self._state
        if state is None:
            return
        phase, completed, total = state
        self._writer.send(
            {
                "protocol": PROTOCOL_ID,
                "type": "job.progress",
                "job_id": self._job_id,
                "phase": phase,
                "completed": completed,
                "total": total,
            }
        )


class Heartbeat(threading.Thread):
    """Re-emits the last progress at a fixed interval (<= 30s per contract)."""

    def __init__(self, interval: float, flush):
        super().__init__(name="opendemand-heartbeat", daemon=True)
        self._interval = max(0.1, float(interval))
        self._flush = flush
        self._stop_requested = threading.Event()

    def run(self) -> None:  # pragma: no cover - timing thread
        while not self._stop_requested.wait(self._interval):
            try:
                self._flush()
            except Exception:
                pass

    def stop(self) -> None:
        self._stop_requested.set()
        self.join(timeout=5.0)


def _remove_written(paths: list[Path]) -> None:
    for path in paths:
        try:
            path.unlink()
        except OSError:
            pass


def _log(stderr, message: str) -> None:
    try:
        stderr.write(f"opendemand-engine: {message}\n")
        stderr.flush()
    except Exception:
        pass
