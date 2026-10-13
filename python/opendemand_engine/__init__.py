"""OpenDemand Python forecasting sidecar (Milestone 1, Naive engine)."""

from __future__ import annotations

__version__ = "0.0.1"

PROTOCOL_ID = "opendemand.sidecar/1"
ARROW_CONTRACT_ID = "opendemand.arrow/1"
SUPPORTED_MODELS = ("naive",)

OBSERVATIONS_FILE = "observations.arrow"
TASKS_FILE = "tasks.arrow"
TASK_STATUS_FILE = "task_status.arrow"
ORIGIN_METRICS_FILE = "origin_metrics.arrow"
FORECAST_POINTS_FILE = "forecast_points.arrow"

__all__ = [
    "__version__",
    "PROTOCOL_ID",
    "ARROW_CONTRACT_ID",
    "SUPPORTED_MODELS",
    "OBSERVATIONS_FILE",
    "TASKS_FILE",
    "TASK_STATUS_FILE",
    "ORIGIN_METRICS_FILE",
    "FORECAST_POINTS_FILE",
]
