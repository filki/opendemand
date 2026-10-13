# ADR-0006: Forecasting Models, Eligibility Mathematics, and Backtesting

**Status:** Accepted
**Date:** 2026-10-13
**Extended by:** [ADR-0012](0012-series-model-evaluation-and-selection.md) — per-series evaluation granularity, canonical `k = max(0, min(4, floor((H - f) / 12)))`, and aggregates-only retention.

## Context

The MVP must produce weekly forecasts for up to 10,000 SKUs with honest, explainable model selection and explicit behavior for series that are too short, fail to fit, or fall back. "Minimum history" must be defined mathematically per model and per backtest configuration so eligibility is deterministic and testable.

## Decision Drivers

- Deterministic eligibility: given `H`, the outcome for every series is computable without judgment calls.
- Honest model selection: a champion is only chosen when it has enough backtest evidence.
- Explicit failure behavior: no silent gaps, no invented values.
- MVP scope: three models that run reliably on modest hardware.

## Decision

Models and ids: `naive`, `seasonal_naive_52` (SeasonalNaive, season 52), `auto_ets_52` (AutoETS, season 52). Croston, TSB, ADIDA, and AutoARIMA are out of MVP scope.

Constants: window `W = 12` weeks; maximum windows `K = 4`; training floors `f` per model (see table). `f_seasonal = 104` is two full 52-week cycles; the statsforecast AutoETS fit minimum must be verified during implementation and `f` raised if the library requires more (recorded via a new ADR).

Eligibility mathematics (per series, with trailing usable history `H` from ADR-0005):

- Production eligibility: a model is considered iff `H >= f_model`.
- Backtest windows: `k_model = min(4, floor((H - f_model) / 12))` non-overlapping trailing windows of 12 weeks; each window forecasts the 12 weeks immediately after its origin; the earliest origin's training size is `H - 12 * k_model`, which is `>= f_model` by construction.
- Champion eligibility: a model qualifies only with at least two completed origins (`c_model >= 2`).
- Champion: the champion-eligible model with the lowest mean WAPE across its completed origins, where per-origin WAPE = `sum|A - F| / sum|A|`; if `sum|A| = 0` for an origin, use MAE for that origin. Tie-break by model complexity: `naive < seasonal_naive_52 < auto_ets_52`.

Thresholds (`H >=`):

| Model | Id | Floor f | Production | First window | Champion-eligible | Full 4 windows |
|-------|----|---------|------------|--------------|-------------------|----------------|
| Naive | `naive` | 12 | 12 | 24 | 36 | 60 |
| SeasonalNaive(52) | `seasonal_naive_52` | 104 | 104 | 116 | 128 | 152 |
| AutoETS(52) | `auto_ets_52` | 104 | 104 | 116 | 128 | 152 |

Outcomes:

- Series qualification (per revision): `eligible`, `insufficient_history` (`H < 12`), `invalid_data`.
- Run-level series outcome: `ok` (champion forecast), `fallback_naive` (no champion; Naive forecast, flagged), `insufficient_history`, `model_failure` (all candidates failed at production), `invalid_data`.
- Model-per-origin result: `completed` or `failed` with reason in {`fit_error`, `predict_error`, `timeout`, `invalid_input`}.
- Production fallback chain: champion, then the next champion-eligible model by metric, then Naive, then `model_failure` with no forecast.

Missing forecasts: exported as blank cells with the series outcome as the reason; never zero-filled. Coverage per run = share of series with outcome `ok` or `fallback_naive`.

## Consequences

Positive:

- Every series has a computable outcome from `H` alone; tests can cover the threshold table exactly.
- Champions require at least two origins of evidence; weak evidence cannot crown a model.
- Failures and fallbacks are explicit and exportable.

Negative:

- Seasonal champions are only possible from `H >= 128`; between 104 and 127 seasonal models run as candidates but cannot win (Naive serves as champion).
- Short series (12-35 weeks) always produce flagged Naive forecasts.
- WAPE needs a zero-denominator guard (MAE fallback), which must be implemented consistently.

## Alternatives Considered

- **AutoARIMA**: better on some series, much slower; out of MVP.
- **Croston / TSB / ADIDA**: needed for intermittent demand, but adds scope; deferred.
- **Single holdout**: cheaper, but one origin is weak evidence; rejected in favor of four non-overlapping windows.
- **MAPE/RMSE as the selection metric**: MAPE breaks on zeros; RMSE overweights large series; WAPE chosen for robustness with the MAE guard.
