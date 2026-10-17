# ADR-0027: Planner decision memory (notes, reviews, override reasons)

**Status:** Accepted
**Date:** 2026-10-17
**Related:** [ADR-0007](0007-forecast-value-representation.md), [ADR-0014](0014-manual-only-overrides.md), [ADR-0022](0022-weekly-planning-workflow-and-run-comparison.md), [ADR-0026](0026-forecast-workspace-exploration-and-pipeline-orchestration.md)

## Context

Planners repeat weekly reviews of the same series. Today the workspace persists only outcomes, reasons, and append-only overrides; it cannot record why an override exists, when a decision should be revisited, or that a planner already reviewed an exception. Every review starts from zero, and the intent behind manual plan changes is lost at the next forecast run.

## Decision Drivers

- Record planner intent as durable, user-authored state; never invent or infer it.
- Keep overrides run-scoped and manual-only (ADR-0014); decision memory must not carry plan values between runs.
- Keep derived state derived: exception membership stays a read-time derivation (ADR-0014, ADR-0022); comparisons are never stored.
- Keep every existing contract intact: baselines, overrides, clears, effective values, exports, and comparison semantics do not change.
- Append-only history with stable identities; a new write is a new row, never an edit.

## Decision

- Add three persisted, append-only tables (migration 004):
  - `series_note`: profile-scoped notes for one product x location, with an optional context run. Notes survive across runs; they never change forecast values or coverage.
  - `series_review`: review events for one run and series. Each event records the action (`reviewed` or `reopened`), a rule/policy context label, and a deterministic evidence fingerprint computed at event time. The current review state is derived at read time as `reviewed` only when the latest event is an acknowledgement whose fingerprint still matches the run's current evidence; new evidence invalidates the acknowledgement without rewriting it.
  - `override_meta`: immutable metadata for one exact `forecast_override` revision, holding a required `reason_code` (`promotion`, `customer_information`, `supply_constraint`, `data_quality`, `other`) and an optional `review_date`. The free-text note stays in the existing `forecast_override.note` column on that revision. `other` requires a note. Historical overrides without metadata render as legacy (`reason not recorded`); they are never backfilled or rewritten.
- Review dates are reminders only. A due date surfaces while its override revision is still the latest revision of that cell; it never clears, expires, or edits an override. Non-null values and clears both require a reason on new writes; an explicit NULL revision remains the only way to clear an override.
- The evidence fingerprint for a review acknowledgement covers the run's outcome/reason for that series and the cell override revision map (`week:revision`, sorted) at acknowledgement time. Later override revisions — or a different run — no longer match, so the series shows as unreviewed again at read time.
- Review and notes never modify coverage, outcomes, exports, or comparisons. Exception membership remains derived; no rule output, comparison count, or priority is stored.

## Consequences

Positive:

- Rationale survives across runs; the next weekly review starts from recorded context.
- Review acknowledgements are honest: changed evidence reopens a series automatically.
- Existing data stays valid; the migration is additive and gate-protected (ADR-0008, ADR-0021).

Negative:

- The evidence fingerprint is a versioned rule label (`review/1`); changing what counts as evidence invalidates earlier acknowledgements at read time and requires a deliberate version bump.
- Two writes (override revision plus metadata) must commit in one transaction to keep the pairing exact.

## Alternatives Considered

- **Mutable decision rows (edited in place):** rejected; the workspace's history model is append-only, and edits would break the audit trail.
- **Storing exception membership or comparison deltas on the decision:** rejected; duplicating derived state drifts and contradicts ADR-0022.
- **Automatic override expiry at the review date:** rejected; it would mutate plan values without a planner action and weaken ADR-0014.
- **Carrying review acknowledgements forward automatically into a new run:** rejected; a new run is new evidence and must be reviewed on its own.
