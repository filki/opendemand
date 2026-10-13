"""StatsForecast Naive modelling with explicit history floors and guards.

ADR-0006 defines the Naive training floor as ``f_naive = 12`` weeks and fixes
the forecast horizon at 12 weeks; Naive repeats the last training value.
"""

from __future__ import annotations

import math
from typing import Sequence

from .errors import FAIL_FIT_ERROR, FAIL_PREDICT_ERROR, TaskFailure

MODEL_FLOOR = {"naive": 12}


def forecast_naive(history: Sequence[float], horizon: int) -> list[float]:
    """Fit StatsForecast ``Naive`` on ``history`` and forecast ``horizon`` weeks.

    Handles the library import lazily so that protocol hello and validation
    never wait on StatsForecast. numpy is a hard dependency of statsforecast
    (pinned through uv.lock) and is used here only to marshal the vector.
    """
    import numpy as np
    from statsforecast.models import Naive

    try:
        model = Naive()
    except Exception as exc:
        raise TaskFailure(FAIL_FIT_ERROR, "naive model could not be initialised") from exc

    values = np.asarray(list(history), dtype=np.float64)
    try:
        result = model.forecast(y=values, h=horizon)
        mean = result["mean"]
    except Exception as exc:
        raise TaskFailure(FAIL_PREDICT_ERROR, "naive model failed to forecast") from exc

    try:
        forecasts = [float(value) for value in mean]
    except (TypeError, ValueError) as exc:
        raise TaskFailure(FAIL_PREDICT_ERROR, "naive model returned a malformed forecast") from exc
    if len(forecasts) != horizon:
        raise TaskFailure(FAIL_PREDICT_ERROR, "naive forecast length does not match the horizon")
    if not all(math.isfinite(value) for value in forecasts):
        raise TaskFailure(FAIL_PREDICT_ERROR, "naive forecast produced non-finite values")
    return forecasts
