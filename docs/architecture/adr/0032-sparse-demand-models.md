# ADR-0032: Opt-in sparse-demand models (Croston-SBA, TSB)

**Status:** Accepted
**Date:** 2026-10-17
**Related:** [ADR-0006](0006-model-eligibility-and-training-floors.md), [ADR-0017](0017-multi-model-comparison-and-champion-selection.md), [ADR-0023](0023-missing-week-imputation.md)

## Context

ADR-0006 deferred Croston, TSB, and ADIDA because intermittent demand needed evidence before it could join the approved set. The pinned `statsforecast==2.1.1` now ships Croston-SBA and TSB, and distributors need at least a testable candidate for slow movers — without changing what existing runs produce.

## Decision Drivers

- The default set is an approved policy; new candidates must never change old runs or surprise the planner.
- Sparse candidates consume a different kind of history: zeros are real demand observations, negatives and gaps are not.
- Freeze what a run planned so results stay reproducible.

## Decision

- Add a model-set id to every forecast run, frozen in `config_json` with the exact `(model_id, training floor)` list. `default` remains Naive (12), SeasonalNaive(52) (104), AutoETS(52) (104) — byte-identical behavior to before. `sparse` appends `croston_sba` (12) and `tsb` (12); unknown ids fail with `invalid_model_set`.
- TSB smoothing parameters are frozen policy, not fitted: `alpha_d = 0.1`, `alpha_p = 0.1`; Croston-SBA uses the pinned library implementation. Both run only through the pinned statsforecast classes.
- Sparse candidates are planned only on windows that are complete, finite, non-negative, and free of imputed weeks (per task, production and backtest alike). Rust enforces this with the in-memory imputation mask; Python re-validates each supplied window and fails that task closed with `sparse_unsupported`. Series that fail eligibility keep the default models with an explicit evaluation row (`not_eligible`, `not_run`) — never a silently dropped model and never zero-filled inputs.
- The `run_model` vocabulary extends through migration 009: SQLite cannot alter a CHECK constraint, so the whole run-evidence chain (`run_model`, `series_model_evaluation`, `series_model_origin`, `backtest_forecast`, `forecast_series_status`, `forecast`) is rebuilt in dependency order and restored from temporary backups inside the migration transaction. No deferred foreign keys and no dangling references.
- Selection policy is unchanged: comparable-origin pooled WAPE, pooled-MAE fallback, at least two completed origins for champion eligibility, Naive production fallback. The deterministic complexity order becomes Naive, SeasonalNaive, Croston-SBA, TSB, AutoETS; sparse models are ordinary candidates, never privileged.
- The forecast dialog exposes the choice ("Naive, SeasonalNaive, AutoETS" or "Add Croston-SBA and TSB"); the run record keeps the truth.

## Consequences

Positive:

- Intermittent series get purpose-built candidates that compete on the same evidence as everything else.
- Old runs and existing workspaces are unchanged; the migration is proven on populated data inside one transaction.
- The frozen configuration makes every sparse result reproducible.

Negative:

- Two more tasks per eligible series in a sparse run; engine wall time grows roughly with planned tasks (AutoETS remains dominant).
- A sparse run that also uses imputation excludes imputed series from sparse candidates by design; the planner sees `not_eligible` instead of a guess.

## Alternatives Considered

- **Plan sparse models for every series:** rejected; negatives and filled weeks violate their input contract.
- **Owner60-fitted smoothing parameters:** rejected; parameters would depend on held-out targets and stop being reproducible policy.
- **Making sparse models the default:** rejected; the approved set changes only through a deliberate product decision, not an implementation detail.
- **Relaxing the CHECK by editing migration 001:** forbidden; 001 is frozen.
