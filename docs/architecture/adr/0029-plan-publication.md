# ADR-0029: Immutable plan publications

**Status:** Accepted
**Date:** 2026-10-17
**Related:** [ADR-0007](0007-forecast-value-representation.md), [ADR-0014](0014-manual-only-overrides.md), [ADR-0022](0022-weekly-planning-workflow-and-run-comparison.md), [ADR-0025](0025-xlsx-export.md), [ADR-0027](0027-planner-decision-memory.md), [ADR-0028](0028-exception-inbox-policy.md)

## Context

Runs stay exploratory: the planner keeps adjusting overrides, and an ordinary export is only a snapshot of the saved run at export time. There is no durable "this is what we committed to for this history cutoff" record. Retrospective evaluation needs exactly that boundary — a plan that was fixed before the target weeks happened — otherwise later edits could rewrite the past.

## Decision Drivers

- Separate exploration from commitment without weakening ADR-0026: runs remain exploratory; publishing is an explicit, manual act.
- Reproduce the published values exactly, even after same-second edits: pins must name exact rows, not reconstruct from timestamps.
- Store references, not duplicated values: baselines are immutable already; the published effective plan is derived.
- One honest commitment per weekly cycle; no retroactive rewriting, no silent substitution of ordinary exports.

## Decision

- Add `plan_publication` and `publication_override` (migration 006), both immutable (UPDATE and DELETE rejected). One publication per `(profile_id, cutoff_week)`; publishing a cutoff again fails with `already_published`.
- Publishing is legal only for a completed run of the active profile's data whose revision cutoff equals the run's horizon start minus one week, and only when that cutoff is a finished week: history reaching into the current unfinished week fails with `future_history`.
- At publication time, in one serialized writer transaction, the publication pins the exact latest override revision of every overridden cell of the run (including NULL clearing revisions) and records the highest `import_revision.id` of the profile as the publication's revision high-water mark — the hindsight boundary for later retrospective evaluation. `published_at` is the actual time; backdating is impossible.
- The published plan is derived at read time from `publication_override` plus the immutable baselines: a pinned non-null override wins, a pinned clear falls through to the baseline, and a missing baseline stays missing. Nothing is copied.
- Publication exports reuse the ordinary export machinery with the same columns (ADR-0025), read pinned values, and record `export_record.publication_id`; the XLSX Summary gains publication identity lines (id, published_at, cutoff, high-water). Ordinary run exports keep their live-saved-run semantics and stay unlinked.
- The UI publishes through an explicit two-step confirmation ("Publish plan" then "Confirm publish"); once published, the selected cut-off shows its publication instead of the publish control.

## Consequences

Positive:

- A trustworthy plan-of-record per weekly cycle that later edits cannot change.
- Retrospective scoring has a predefined boundary: publication time and revision high-water, both stored.
- No duplicated plan values; storage stays small (overridden cells plus headers).

Negative:

- Correcting a published plan for the same cutoff requires publishing a newer cutoff instead; this is deliberate and matches the weekly cadence.
- A publication pins only overridden cells; baseline changes are impossible by construction (baselines are immutable per run), so this is complete.

## Alternatives Considered

- **Freeze on first export:** rejected; exports include exploratory files, so the commitment would be accidental.
- **Reconstruct the plan from override timestamps at publish time:** rejected; one-second timestamp precision cannot name exact same-second revisions.
- **Store a full effective-plan copy:** rejected; duplicates derived state, grows linearly with series, and drifts from the pinned references.
- **Allow replacing a publication:** rejected; superseding mid-week would let hindsight rewrite the plan of record.
