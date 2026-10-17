"""StatsForecast model dispatch with explicit history floors and guards.

ADR-0006 fixes one training floor per model and the 12-week horizon; Milestone 3
runs Naive, SeasonalNaive(52), and AutoETS(52) through one small dispatch table.

Verified against the pinned statsforecast==2.1.1: SeasonalNaive needs at least
52 observations to forecast without NaN (the 104-week floor keeps two full
cycles); AutoETS fits short, zero, constant, negative, and intermittent
histories, so the 104-week AutoETS floor stays the approved policy floor and no
library-driven floor increase is required.
"""

from __future__ import annotations

import math
from typing import Callable, Sequence

from .errors import FAIL_FIT_ERROR, FAIL_PREDICT_ERROR, FAIL_UNSUPPORTED_MODEL, TaskFailure

MODEL_FLOOR = {"naive": 12, "seasonal_naive_52": 104, "auto_ets_52": 104, "croston_sba": 12, "tsb": 12}
SEASON_LENGTH = 52


def _naive():
    from statsforecast.models import Naive

    return Naive()


def _seasonal_naive():
    from statsforecast.models import SeasonalNaive

    return SeasonalNaive(season_length=SEASON_LENGTH)


def _auto_ets():
    from statsforecast.models import AutoETS

    return AutoETS(season_length=SEASON_LENGTH)


def _croston_sba():
    from statsforecast.models import CrostonSBA

    return CrostonSBA()


def _tsb():
    from statsforecast.models import TSB

    # Frozen policy parameters (ADR-0032): not fitted against held-out data.
    return TSB(alpha_d=0.1, alpha_p=0.1)


FACTORIES: dict[str, Callable[[], object]] = {
    "naive": _naive,
    "seasonal_naive_52": _seasonal_naive,
    "auto_ets_52": _auto_ets,
    "croston_sba": _croston_sba,
    "tsb": _tsb,
}


def forecast(model_id: str, history: Sequence[float], horizon: int) -> list[float]:
    """Fit the model for ``model_id`` on ``history`` and forecast ``horizon`` weeks.

    Models are imported lazily so that protocol hello and validation never wait
    on StatsForecast. numpy is a hard dependency of statsforecast (pinned through
    uv.lock) and is used here only to marshal the vector.
    """
    factory = FACTORIES.get(model_id)
    if factory is None:
        raise TaskFailure(FAIL_UNSUPPORTED_MODEL, f"unknown model id: {model_id}")
    try:
        model = factory()
    except Exception as exc:
        raise TaskFailure(FAIL_FIT_ERROR, f"{model_id} model could not be initialised") from exc

    import numpy as np

    values = np.asarray(list(history), dtype=np.float64)
    try:
        model.fit(values)
    except Exception as exc:
        raise TaskFailure(FAIL_FIT_ERROR, f"{model_id} model failed to fit") from exc
    try:
        result = model.predict(h=horizon)
        mean = result["mean"]
    except Exception as exc:
        raise TaskFailure(FAIL_PREDICT_ERROR, f"{model_id} model failed to forecast") from exc

    try:
        forecasts = [float(value) for value in mean]
    except (TypeError, ValueError) as exc:
        raise TaskFailure(FAIL_PREDICT_ERROR, f"{model_id} model returned a malformed forecast") from exc
    if len(forecasts) != horizon:
        raise TaskFailure(
            FAIL_PREDICT_ERROR, f"{model_id} forecast length does not match the horizon"
        )
    if not all(math.isfinite(value) for value in forecasts):
        raise TaskFailure(FAIL_PREDICT_ERROR, f"{model_id} forecast produced non-finite values")
    return forecasts
