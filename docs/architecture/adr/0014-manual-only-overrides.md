# ADR-0014: Manual-Only Forecast Overrides

**Status:** Accepted
**Date:** 2026-10-13

## Context

ADR-0007 defined overrides as append-only revisions over a stored baseline, assuming every overridden cell has model output. Planners also need a forecast for a series with no baseline (for example `insufficient_history`), which the original rules could not represent without inventing baseline rows. The review approved manual-only overrides with explicit guardrails.

## Decision Drivers

- Honest data: a manual value must never be disguised as model output.
- Bounded scope: overrides must stay inside one completed run and its persisted horizon.
- Auditable history: append-only, sequential revisions; no silent updates or deletes.
- Missing stays missing: clearing a manual-only cell must not fabricate a value.

## Decision

An override may target any `forecast_series_status` row of a run, even when no baseline `forecast` row exists, under these guardrails:

- the run is `completed`;
- the series is inside the run scope (a `forecast_series_status` row exists);
- the week lies inside the persisted 12-week horizon starting at `forecast_run.horizon_start_week`;
- the first override revision for a cell is non-null;
- revisions are sequential and append-only; UPDATE and DELETE are rejected;
- a later NULL revision clears the override: with a baseline, effective falls back to the baseline; without a baseline, effective becomes `missing`.

The rules are enforced with SQLite triggers in migration 001 where practical (insert guardrails plus UPDATE/DELETE rejection) and repeated in Rust before the write.

## Consequences

Positive:

- Manual-only cells are representable and export as `override`/`effective` with no fabricated baseline.
- Clearing a manual-only cell is honest: the cell returns to missing.
- All edit history stays in `forecast_override`; no mutation path exists.

Negative:

- More trigger and validation surface to test (covered by the migration integration tests).
- The UI must explain why some cells have no baseline behind the override.

## Alternatives Considered

- **Requiring a baseline**: blocks manual-only forecasting; rejected.
- **Allowing a NULL first revision**: indistinguishable from "no override"; rejected.
- **Inserting synthetic baseline rows**: disguises manual values as model output; rejected.
- **Mutable overrides**: loses history; rejected (ADR-0007 stands).
