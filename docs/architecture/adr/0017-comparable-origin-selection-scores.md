# ADR-0017: Comparable-Origin Selection Scores

**Status:** Accepted
**Date:** 2026-10-14

## Context

Milestone 3 runs three models (Naive, SeasonalNaive with `season_length=52`, AutoETS with `season_length=52`) for every eligible series, so one series can produce evaluation evidence for several models at once. ADR-0006 and ADR-0012 approved the champion rule "lowest mean WAPE over completed origins, with a per-origin MAE fallback when `sum|A| = 0`". That rule has three problems at three-model scale:

- Different models complete different numbers of origins on the same series (for example, at 140 usable weeks Naive has four origins while the seasonal models have three), so averaging over unequal evidence compares incomparable windows.
- A mean over per-origin metric values mixes WAPE ratios with MAE values in one number, and weights origins with small actual denominators too heavily.
- The AutoETS fit minimum was an open hypothesis in ADR-0006 and needed verification against the pinned library before relying on the 104-week floor.

Verified against `statsforecast==2.1.1`: `SeasonalNaive(season_length=52)` produces NaN forecasts below 52 observations, and the 104-week floor keeps two full cycles, so the approved floor stays valid. `AutoETS(season_length=52)` fits histories from 12 weeks and handles zero, constant, negative, and intermittent series, so no library-driven floor increase is required; 104 weeks remains the approved policy floor.

## Decision Drivers

- Fair comparisons: never treat unequal backtest evidence as if it were shared.
- One unit per ranking: a comparison must not average WAPE ratios with MAE values.
- Deterministic and reproducible from stored rows alone.
- Minimal machinery: no configurable metric engine, no scoring weights.

## Decision

- Champion eligibility stays as approved in ADR-0006: a model needs at least two completed backtest origins.
- The comparable evaluation set for one series is the intersection, over all champion-eligible candidates, of the origin dates those candidates completed. Every ranked candidate is scored on exactly that set. A candidate's extra origins remain stored as evidence but do not enter the comparison.
- The comparable score pools held-out error sums over that set. It is pooled WAPE, `sum(abs(actual-predicted)) / sum(abs(actual))`, when the pooled actual magnitude is positive; otherwise it is pooled MAE, `sum(abs(actual-predicted)) / evaluated_points`, which keeps one unit across the comparison because the set and horizon are identical. Lower wins.
- Exact ties break by documented model complexity order: `naive`, `seasonal_naive_52`, `auto_ets_52`.
- When no champion-eligible candidate exists, or no origin date is common to all champion-eligible candidates, no champion, score, or rank is recorded. The fallback chain still runs; the run never invents a comparison.
- Production fallback order: ranked candidates best first, then Naive when production-eligible. A model that was never ranked is never used for production, because it has no comparable evidence; when nothing in the chain succeeds, the series is recorded as `model_failure`. `selection_rank` and `is_champion` record the statistical preference; `forecast_series_status.selected_model_id` and `forecast.selected_model_id` record the model that actually produced the forecast, and `reason` states when the champion failed production or when only the Naive baseline was available.
- Forecast bias is a diagnostic, never a ranking input: `sum(predicted-actual)/sum(abs(actual))` over a model's completed origins, positive meaning overforecasting, unavailable (null) when the actual magnitude is zero.
- Per-origin WAPE and MAE remain stored in `series_model_origin` for diagnosis; `metric_kind`/`metric_value` keep the per-origin fallback behavior unchanged.

## Consequences

Positive:

- Cross-model rankings always compare identical evaluation windows.
- Zero-denominator series produce one consistent comparison (pooled MAE) instead of mixing units.
- Selection is reproducible from `series_model_evaluation` rows, and the underlying evidence stays in `series_model_origin` and `backtest_forecast`.

Negative:

- The comparable set shrinks to the candidate with the fewest completed origins; the comparison is conservative by design.
- Pooled diagnostics in the comparison API replay error sums from `backtest_forecast` joined with actuals; in a future aggregates-only storage mode those columns show as unavailable while the persisted selection score and rank remain authoritative.

## Alternatives Considered

- **Mean of per-origin metric values** (previous rule): mixes units and unequal evidence; rejected.
- **Union of origins with imputed missing evidence**: imputes evidence that was never observed; rejected.
- **Requiring identical planned origin counts to rank**: discards the seasonal evidence that exists at 104-152 weeks; rejected.
- **Per-origin rank voting or weighted scores**: more moving parts with no accuracy benefit; rejected.
- **Bias in ranking**: accuracy is the selection target; bias stays diagnostic; rejected.
