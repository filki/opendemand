# ADR-0023: Model-Input Imputation of Missing Weeks

**Status:** Accepted
**Date:** 2026-10-14
**Extends:** [ADR-0005](0005-missing-data-semantics.md) — the rejection of interpolation applies to stored data; this record allows run-scoped, non-persisted imputation of model input.

## Context

Gaps in weekly history cut usable history (`H`, ADR-0005) and can make SeasonalNaive and AutoETS (floor 104) ineligible even when years of data exist around the gaps. The planner wanted gap filling, but ADR-0005 rejected interpolation because inventing demand distorts stored evidence. Both needs are satisfiable if imputation never becomes data.

## Decision Drivers

- Honest data: stored observations, exports, and the series view keep gaps; imputed values are never presented as observations.
- Deterministic eligibility: the filled series is a pure function of stored observations, the cutoff, and the selected method.
- Comparable evidence: metric scoring must not grade models against invented target values.

## Decision

- **Run-scoped, input-only.** A forecast run may request one imputation method. Imputation is applied in Rust while planning the run, to the series copy sent to the engine. Nothing is written back: `observation` rows, `H` in `series_qualification`, exports, and the series chart are unchanged. The run record stores the method and per-series filled-week counts.
- **Methods** (`imputation_method` id):
  - `last` — last known value carried forward (LOCF).
  - `last_seasonal_52` — the value of the same week one year earlier (52 weeks); if that week was not originally known, step back by whole years (104, 156, …); only originally known weeks qualify. If no year is available, fall back to `last`.
  - `mean_4` — arithmetic mean of up to the four most recent originally known values before the missing week; with fewer than four known values, their mean.
- **Scope.** The effective calendar runs from the first originally known week to the cutoff; internal and trailing gaps inside it are filled. Weeks before the first known week (leading gap) stay missing and are excluded, so a leading gap can still shorten the series.
- **Eligibility.** Model floors (ADR-0006) apply to the filled length; a filled series that reaches a floor becomes plannable in that run. The stored qualification is unaffected.
- **Backtesting.** Origins may train on imputed weeks, but any origin whose 12-week target window contains an imputed week is **not planned and not scored**; champion selection therefore never uses invented targets. If this leaves fewer than two scored origins, ADR-0017 fallback rules apply unchanged.
- **Reproducibility.** The method id and the filled-week counts persist with the run (migration 003); a run without imputation stores NULL/zero. Method ids and floor behaviour are pinned by tests.

## Consequences

Positive:

- Long gaps no longer disqualify seasonal models when surrounding history is sufficient.
- Stored evidence and exports remain exactly as imported (ADR-0005 respected).
- Metric comparisons stay honest: imputed targets are excluded.

Negative:

- Two series can have different effective histories in the same run, depending on gaps.
- Diagnostic surfaces must explain that forecasts used filled input, or a fallback will look inconsistent with the stored `H`.

## Alternatives Considered

- **Persist imputed observations (new status).** Changes stored semantics, exports, comparisons, and migrations; rejected as disproportionate.
- **Zero-fill.** Biases demand and hides data problems (ADR-0005); rejected.
- **Impute in the Python sidecar.** Splits eligibility/`H` logic across two languages and leaves the run record unable to describe what the engine saw; rejected.
- **Score imputed targets.** Grades models against invented numbers; rejected.
