"""OpenDemand Python forecasting sidecar (Milestone 3, Naive/SeasonalNaive/AutoETS)."""

from __future__ import annotations

__version__ = "0.5.3"

PROTOCOL_ID = "opendemand.sidecar/1"
ARROW_CONTRACT_ID = "opendemand.arrow/1"
SUPPORTED_MODELS = ("naive", "seasonal_naive_52", "auto_ets_52", "croston_sba", "tsb")

OBSERVATIONS_FILE = "observations.arrow"
TASKS_FILE = "tasks.arrow"
TASK_STATUS_FILE = "task_status.arrow"
ORIGIN_METRICS_FILE = "origin_metrics.arrow"
FORECAST_POINTS_FILE = "forecast_points.arrow"
CANCEL_FILE = "cancel.flag"

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
    "CANCEL_FILE",
]
