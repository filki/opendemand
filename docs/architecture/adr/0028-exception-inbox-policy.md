# ADR-0028: Exception inbox ranking and material-change policy

**Status:** Accepted
**Date:** 2026-10-17
**Related:** [ADR-0005](0005-missing-data-semantics.md), [ADR-0014](0014-manual-only-overrides.md), [ADR-0017](0017-multi-model-comparison-and-champion-selection.md), [ADR-0022](0022-weekly-planning-workflow-and-run-comparison.md), [ADR-0027](0027-planner-decision-memory.md)

## Context

The run overview lists per-series outcomes, and decision memory records reviews and notes, but a planner still cannot see "what deserves my attention today" in one ranked list. Deciding which exceptions matter must be transparent, reproducible, and honest about missing evidence; it must not invent monetary impact or hidden scores.

## Decision Drivers

- Keep every rule output derived at read time; persist only user thresholds and user actions (ADR-0022, ADR-0027).
- Make ranking deterministic and explainable; no cross-SKU aggregation of incomparable units and no currency without reliable price data.
- Reuse existing immutable evidence: outcomes, reasons, imputed counts, baselines, override decisions, review events.
- Never claim a rule fired when its evidence is missing; never silently skip an unforecast revision.

## Decision

- Add one persisted, append-only, profile-scoped `review_policy` table (migration 005) holding the material-change thresholds: an absolute threshold in the series' own units and a relative threshold as a fraction of the previous baseline magnitude. Both are NULL together (rule disabled) or set; versions never update.
- Derive the inbox at read time for one completed run with a fixed rule priority:
  1. `model_failure`, 2. `invalid_data`, 3. `insufficient_history`, 4. `imputed_input`, 5. `production_fallback` (including an `ok` outcome with a recorded fallback reason), 6. `material_change`, 7. `review_due`. Row order follows the highest-priority rule, then series key, location code, and identities.
- Material-change evidence compares the selected run with the newest completed run of the immediately preceding import revision (ADR-0022 adjacency; an unforecast revision is never skipped): the sum of absolute baseline deltas over common calendar weeks, and that sum divided by the previous baseline magnitude over the same weeks. The rule fires only when the change is positive, the absolute threshold is met, and — when the previous magnitude is positive — the relative threshold is met. A zero previous magnitude discloses the unavailable percentage and decides on the absolute threshold alone.
- `review_due` fires while an override revision with a due review date is the latest revision of a cell with a non-null value; it is a reminder and changes nothing (ADR-0027).
- Review state is a filter, never suppression: reviewed rows stay listed with their evidence; new evidence reopens them (ADR-0027). Search and the review filter apply before pagination in one read snapshot; totals are independent of pagination.
- Threshold-based rules stay disabled until configured. No monetary scoring, no cross-series risk index; units stay per series.

## Consequences

Positive:

- One ranked, explainable work list per run; every chip maps to stored evidence.
- Thresholds are auditable versions; changing them cannot rewrite history.
- No new stored derived state; the inbox stays consistent with comparisons and decision memory.

Negative:

- The absolute threshold is a per-series unit value, so a single default cannot fit all SKUs; the rule stays off until the planner sets thresholds.
- Change evidence depends on an adjacent completed run; early revisions show the rule as unavailable rather than guessing.

## Alternatives Considered

- **Storing inbox rows or computed priority:** rejected; duplicates derived state and drifts (ADR-0022 precedent).
- **Monetary impact ranking:** rejected for now; requires prices, currencies, and comparable unit economics that the workspace does not store.
- **Skipping an unforecast previous revision to find an older comparison:** rejected; ADR-0022's adjacency stance stays intact.
- **Default thresholds:** rejected; any default would silently mark ordinary weeks as "material" across incomparable SKUs.
