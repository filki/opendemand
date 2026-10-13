"""WAPE/MAE metric computation with explicit guards.

WAPE = sum|actual - predicted| / sum|actual|. When the actuals are all zero the
denominator is zero: WAPE is null and ``mae_fallback`` carries the MAE instead.
MAE is always retained, and every emitted metric must be finite and nonnegative.
"""

from __future__ import annotations

import math
from typing import Sequence

from .errors import FAIL_METRIC_ERROR, TaskFailure

WAPE = "wape"
MAE_FALLBACK = "mae_fallback"


def _metric_failure(detail: str) -> TaskFailure:
    return TaskFailure(FAIL_METRIC_ERROR, detail)


def compute_metrics(
    actuals: Sequence[float], predictions: Sequence[float]
) -> tuple[float | None, float, str, float]:
    """Return ``(wape_or_none, mae, metric_kind, metric_value)``."""
    if not actuals or len(actuals) != len(predictions):
        raise _metric_failure("actual and predicted series must be non-empty and equal length")

    total_error = 0.0
    total_actual = 0.0
    for actual, predicted in zip(actuals, predictions):
        try:
            actual_value = float(actual)
            predicted_value = float(predicted)
        except (TypeError, ValueError) as exc:
            raise _metric_failure("metric inputs must be numeric") from exc
        if not (math.isfinite(actual_value) and math.isfinite(predicted_value)):
            raise _metric_failure("metric inputs must be finite")
        total_error += abs(actual_value - predicted_value)
        total_actual += abs(actual_value)

    mae = total_error / len(actuals)
    if total_actual == 0.0:
        wape: float | None = None
        kind = MAE_FALLBACK
        value = mae
    else:
        wape = total_error / total_actual
        kind = WAPE
        value = wape

    if not math.isfinite(mae) or mae < 0.0:
        raise _metric_failure("MAE must be finite and nonnegative")
    if wape is not None and (not math.isfinite(wape) or wape < 0.0):
        raise _metric_failure("WAPE must be finite and nonnegative")
    if not math.isfinite(value) or value < 0.0:
        raise _metric_failure("metric value must be finite and nonnegative")

    return wape, mae, kind, value
