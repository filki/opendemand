"""Engine error vocabulary and exception types.

Whole-job error codes follow ``docs/architecture/contracts/SIDECAR_PROTOCOL.md``.
Task-level failure codes annotate ``task_status`` rows; ADR-0006 maps the
per-origin reasons onto ``{fit_error, predict_error, timeout, invalid_input}``,
so Rust can fold the finer-grained codes below when persisting origins:

- ``unsupported_model`` / ``insufficient_history`` / ``invalid_geometry`` /
  ``missing_training_data`` / ``missing_backtest_actuals`` /
  ``non_finite_values`` / ``duplicate_observation`` -> ``invalid_input``
- ``metric_error`` -> ``fit_error``
"""

from __future__ import annotations

PROTOCOL_MISMATCH = "protocol_mismatch"
CONTRACT_VIOLATION = "contract_violation"
INVALID_INPUT = "invalid_input"
ENGINE_START_FAILED = "engine_start_failed"
UNEXPECTED_EOF = "unexpected_eof"
TIMEOUT = "timeout"
CANCELLED = "cancelled"
INTERNAL_ERROR = "internal_error"

FAIL_UNSUPPORTED_MODEL = "unsupported_model"
FAIL_INSUFFICIENT_HISTORY = "insufficient_history"
FAIL_INVALID_GEOMETRY = "invalid_geometry"
FAIL_MISSING_TRAINING_DATA = "missing_training_data"
FAIL_MISSING_BACKTEST_ACTUALS = "missing_backtest_actuals"
FAIL_NON_FINITE_VALUES = "non_finite_values"
FAIL_DUPLICATE_OBSERVATION = "duplicate_observation"
FAIL_FIT_ERROR = "fit_error"
FAIL_PREDICT_ERROR = "predict_error"
FAIL_METRIC_ERROR = "metric_error"
FAIL_INTERNAL_ERROR = "internal_error"

RETRYABLE_JOB_ERRORS = frozenset({ENGINE_START_FAILED, UNEXPECTED_EOF})


class EngineError(Exception):
    """A whole-job failure that terminates the sidecar run."""

    def __init__(self, code: str, message: str, retryable: bool = False):
        super().__init__(message)
        self.code = code
        self.message = message
        self.retryable = retryable


class TaskFailure(Exception):
    """A single task failing; becomes a ``task_status`` row, not a job error."""

    def __init__(self, code: str, detail: str):
        super().__init__(detail)
        self.code = code
        self.detail = detail


class JobCancelled(Exception):
    """Raised internally when ``job.cancel`` arrives; never escapes run_job."""
