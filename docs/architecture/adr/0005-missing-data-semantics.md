# ADR-0005: Missing-Data Semantics and Observation Statuses

**Status:** Accepted
**Date:** 2026-10-13
**Extended by:** [ADR-0013](0013-canonical-import-and-mapping-idempotency.md) — `canonical/1` derives `known`/`zero`/`return` from numeric quantities; [ADR-0014](0014-manual-only-overrides.md) — a cleared override without a baseline yields `missing`.

## Context

Demand data is messy: weeks can be absent, zero demand can be real or unknown, stockouts truncate demand, returns and corrections distort net demand, and the newest week may be partial. Treating absence as zero silently biases forecasts downward; treating every value as certain hides data quality. Forecasting eligibility also needs an unambiguous definition of usable history.

## Decision Drivers

- Honest data: absence is not zero unless the source says so.
- Deterministic eligibility: one rule for what counts as usable history (`H`).
- Auditability: every deviation from a plain value is visible as a status.
- MVP simplicity: statuses are stored and exported, but modeling treatment stays simple.

## Decision

- **Unknown by default**: a week with no source value is `missing`, never zero.
- **Zero only when explicit**: store `zero` only when source semantics define zero demand.
- Observation status vocabulary: `known`, `zero`, `missing`, `return`, `correction`, `stockout`, `partial`.
- Modeling input is the **net weekly quantity**: `known`, `zero`, `return`, `correction`, and `stockout` rows all produce a numeric value (returns are negative net demand). `missing` and `partial` have no numeric value.
- `H` is the length of the **trailing run of consecutive weeks with numeric values**, ending at the last completed week; a `missing` or `partial` week breaks the run.
- `partial` applies only to the newest week until the source marks it complete; it never counts toward `H`.
- `stockout` values stay numeric and are flagged only; censor-aware correction is out of MVP scope.
- **Duplicates at import**: identical duplicate rows are collapsed (info issue); conflicting duplicates are excluded (error issue) and surfaced in the import report.

## Consequences

Positive:

- Eligibility and coverage are computable exactly from stored statuses.
- Data quality problems are visible in exports and reports instead of hidden in numbers.
- The modeling contract (net numeric series, `H` definition) is unambiguous.

Negative:

- More statuses to handle in import, UI, and export code.
- Stockouts remain biased low in training data until a future censor-aware model.
- Users must understand status semantics; the UI must explain them.

## Alternatives Considered

- **Zero-fill unknowns**: biases forecasts low and hides data problems; rejected.
- **Interpolation of gaps**: invents demand; rejected.
- **Dropping unknown weeks silently**: loses auditability; rejected (statuses are stored instead).
