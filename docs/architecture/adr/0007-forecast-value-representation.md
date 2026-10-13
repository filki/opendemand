# ADR-0007: Forecast Value Representation — Baseline, Override, Effective

**Status:** Accepted
**Date:** 2026-10-13
**Extended by:** [ADR-0014](0014-manual-only-overrides.md) — manual-only overrides and their guardrails.

## Context

Planners need to adjust model output for specific cells without corrupting the model result, losing the original, or silently propagating edits into future runs. Exports must make it obvious whether a number is model-produced, manually edited, or missing, and edits must be auditable.

## Decision Drivers

- The model result is evidence; it must stay intact for comparison and audit.
- Edits are scoped to one forecast run; they never leak into later runs.
- One unambiguous definition of "the number the business will use".
- Missing stays visible; nothing is zero-filled.

## Decision

Three layers:

- **baseline**: the model-produced value per (run, product, location, week); stored in `forecast`; immutable once the run is persisted.
- **override**: append-only revisions per cell in `forecast_override`; each edit appends a new revision; a revision with NULL value clears the override; the highest revision per cell is effective; overrides are immutable within a run and are never auto-carried to a later run.
- **effective** = latest override ?? baseline; derived at query and export time, never stored.

Export columns (CSV and XLSX): `run_id`, `product_sku`, `product_name`, `location_code`, `week_start`, `horizon`, `baseline`, `override`, `effective`, `value_source` in {`model`, `override`, `missing`}, `series_status`, `model_id`. Missing forecasts are blank cells with the series outcome as the reason; never zero-filled. XLSX adds a Summary sheet with coverage, outcome counts, and champion distribution.

## Consequences

Positive:

- The baseline is always available next to the effective value; audits are trivial.
- Override history is complete (append-only), so "who changed what" is answerable from data.
- No hidden state: effective is a pure derivation with one definition.

Negative:

- Two extra columns in every export and grid view; the UI must teach the three-layer model.
- Override history grows with edits (small: edits are rare and revision rows are tiny).

## Alternatives Considered

- **Mutable overrides** (update in place): loses history; rejected.
- **Storing the effective value**: duplicates state and creates reconciliation bugs; rejected.
- **Carrying overrides into the next run**: silently mutates fresh evidence; rejected (a future explicit "copy overrides" action may be added with its own ADR).
