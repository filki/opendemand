# ADR-0030: Retrospective planner FVA and persistent-bias evidence

**Status:** Accepted
**Date:** 2026-10-17
**Related:** [ADR-0005](0005-missing-data-semantics.md), [ADR-0017](0017-multi-model-comparison-and-champion-selection.md), [ADR-0022](0022-weekly-planning-workflow-and-run-comparison.md), [ADR-0027](0027-planner-decision-memory.md), [ADR-0028](0028-exception-inbox-policy.md), [ADR-0029](0029-plan-publication.md)

## Context

Backtest evidence says how models would have done; it cannot say whether the planner's committed plan worked. Only a publication freezes "what we decided" before the target weeks happen, and only a user confirmation can declare which actuals revision is complete enough to judge against. Both must be pinned, or the score can be rewritten by later edits.

## Decision Drivers

- Score only prospective commitments: publication strictly before the target week started, actuals from a revision later than the publication's high-water mark.
- Make every unusable point an explicit exclusion; never zero-fill, never silently skip.
- Keep metrics in the series' own units; no cross-SKU accuracy score, no significance claims.
- Store confirmations and thresholds only; all scoring stays derived at read time.

## Decision

- Add `actuals_confirmation` (migration 007, append-only): a user confirmation that one actuals revision is complete through a finished week. The latest confirmation of the profile is current; corrections and retractions append new rows. Confirmations beyond the last finished week are refused, and the default horizon step is h=2 because a normal Monday publication is too late for h=1.
- Derive per-point verdicts at read time. For one publication (pinned run, horizon start, `published_at`, revision high-water mark) and one horizon step `h`, the target week is `horizon_start + (h-1) weeks`, and each series receives exactly one verdict:
  - `late_published`: `published_at` on or after the target week start (all points of the publication);
  - `future`: the target week is beyond the confirmed range (all points);
  - `no_forecast`: the published plan has no baseline or effective value;
  - `absent`, `missing_or_partial`, `stockout`: the confirmed revision has no numeric, usable actual;
  - `hindsight`: the target actual existed in any profile revision at or below the high-water mark — the planner could have seen it;
  - otherwise **scored**, using the pinned published values and the exact confirmed revision.
- Report per series (pooled over the series' own scored points across publications, same units): WAPE, MAE, bias for baseline and effective, and FVA as baseline WAPE minus effective WAPE in percentage points (positive = the adjustment helped). When the pooled actual magnitude is zero, WAPE, bias, and the percentage-point FVA are unavailable and MAE improvement stays available in quantity units. Publications report counts only (scored, adjusted, improved/harmful/unchanged per series, exclusions, imputed-input disclosure, low-sample flag below five points); a per-publication detail lists every point with its inputs and verdict.
- Persistent-bias inbox rule: a versioned policy group (horizon step, mature-week window, minimum weeks ≥ 2, bias threshold > 0). Evidence uses published effective values against confirmed actuals at the configured horizon, newest publications first, distinct target weeks only. The rule fires only when there are at least the minimum distinct mature weeks, every error shares one nonzero sign (a zero or reversed error breaks persistence), and the pooled bias magnitude exceeds the threshold. Rule positions in the inbox priority sit before `review_due`; the review evidence fingerprint (`review/2`) includes the bias token, so changed bias evidence reopens acknowledgements.
- Nothing about scoring is stored: no evaluation rows, no aggregates, no per-week masks beyond the disclosure of a run's persisted imputation method and weekly count.

## Consequences

Positive:

- Honest, reproducible scoring: both the plan and the actuals are pinned, and every ineligible point is visible with a reason.
- The weekly habit has a measurable payoff loop: adjustments are scored in the same units the planner works in.
- Bias evidence turns repeated over/underforecasting into an inbox rule without any new stored state.

Negative:

- Exact hindsight detection scans observation rows for the target week at or below the high-water mark; bounded per publication but not free on very large workspaces.
- Single-point publication detail is descriptive, not statistical; the low-sample flag exists precisely so nobody reads one week as a result.

## Alternatives Considered

- **Score every run without publications:** rejected; exploratory runs would conflate drafts with commitments.
- **Score against the newest actuals always:** rejected; results would change day to day and later corrections could flip past scores.
- **A blended cross-SKU accuracy score:** rejected; quantities are not comparable across SKUs and the number would invite false confidence.
- **Exclude imputed-input runs from scoring:** rejected; a committed plan is scored as committed — imputation is disclosed per point and in counts instead.
