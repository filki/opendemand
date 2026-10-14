# ADR-0012: Per-Series Model Evaluation and Selection

**Status:** Accepted
**Date:** 2026-10-13

## Context

The original evaluation design stored selection evidence at run level: `run_model` carried a champion status and an aggregate metric, and `backtest_origin` was keyed only by `(run_id, model_id, origin_index)`. Origin dates, train sizes, and metric outcomes vary per series, so a run-level origin cannot honestly represent them, champion selection cannot be reproduced per series, and the aggregates-only storage mode would lose the evidence that justified selection. The Phase 1 review required normalizing evaluation before freezing migration 001.

## Decision Drivers

- Reproducible selection: the champion for every series must be recomputable from stored rows alone.
- Honest granularity: origins and metrics belong to a series × model × origin, not to a run.
- One owner: model selection policy stays in Rust; Python only computes tasks and metrics.
- Bounded storage: aggregates-only mode must stay possible without losing selection evidence.

## Decision

- `run_model` is a run-level execution summary only: status `completed | partial | failed | not_run`, task counters (`tasks_planned`, `tasks_completed`, `tasks_failed`), and failure code/detail. It stores no champion, no selection metric, and no per-series data.
- `series_model_evaluation`, keyed by `(run_id, product_id, location_id, model_id)`, stores `eligibility_status` (`not_eligible | production_only | champion_eligible`), `origins_planned`, `origins_completed`, nullable `selection_score` and `selection_rank`, `is_champion`, `production_status` (`not_run | completed | failed`), and `failure_code` / `failure_detail`. A partial unique index (`uq_series_model_evaluation_champion`) allows at most one `is_champion` row per run/series.
- `series_model_origin`, keyed by `(run_id, product_id, location_id, model_id, origin_index)`, stores `origin_week_start`, `train_size`, status `completed | failed`, `wape` (nullable), `mae`, `metric_kind` (`wape` or `mae_fallback`), `metric_value`, and `failure_code` / `failure_detail`. It references the matching `series_model_evaluation` row.
- `backtest_forecast` references the full per-series origin key.
- `forecast_series_status.selected_model_id` and `forecast.selected_model_id` reference the matching `series_model_evaluation` row; duplicated champion metrics and origin counters are removed from `forecast_series_status` and queried from the evaluation row.
- Window count is canonical: `k = max(0, min(4, floor((H - f) / 12)))` (identical to ADR-0006 for `H >= f`). The champion rule stays as approved in ADR-0006: lowest mean WAPE over completed origins, with the per-origin MAE fallback when `sum|A| = 0`; `metric_kind` records which metric was used. The mixed WAPE/MAE rule is acknowledged, not redesigned.
- Rust derives eligibility, computes selection score/rank, and applies champion/fallback policy; Python executes the Rust-provided task plan and returns per-series/origin metrics and forecasts (see `../contracts/ARROW_SCHEMAS.md`).
- Aggregates-only mode may omit per-point `backtest_forecast` rows; `series_model_evaluation` and `series_model_origin` are always retained.

## Consequences

Positive:

- Champion selection is reproducible per series from stored evidence.
- Two series sharing a model and origin index keep different origin dates, train sizes, and outcomes.
- Aggregates-only storage cannot silently change selection results.

Negative:

- More rows than the run-level design (one evaluation per series × model).
- Selection queries join series-level tables instead of reading one run-level row.

## Alternatives Considered

- **Run-level champion and aggregate metric** (previous design): cannot represent per-series origins; rejected.
- **Python-side selection**: moves policy out of the single Rust owner and makes results engine-version dependent; rejected.
- **Storing champion metrics in `forecast_series_status`**: duplicates state that must then stay consistent; rejected in favor of querying `series_model_evaluation`.

---

**Partially superseded by [ADR-0017](0017-comparable-origin-selection-scores.md):** the champion-rule sentence ("lowest mean WAPE over completed origins ... acknowledged, not redesigned") is replaced by comparable-origin pooled scores. The schema and evaluation-granularity decisions stand.
