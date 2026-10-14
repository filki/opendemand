# ADR-0022: Weekly planning workflow and run comparison

**Status:** Accepted
**Date:** 2026-10-14
**Related:** [ADR-0005](0005-missing-data-semantics.md), [ADR-0007](0007-forecast-value-representation.md), [ADR-0013](0013-canonical-import-and-mapping-idempotency.md), [ADR-0014](0014-manual-only-overrides.md), [ADR-0021](0021-import-config-persistence-and-migration-gate-mechanism.md)

## Context

The weekly planning workflow reuses a saved import configuration, imports updated sales, forecasts the resulting revision, compares it with the previous run, and exports the current plan. The comparison needs rules that stay honest about what changed: forecasts align to calendar weeks, not horizon indices; a rolling 12-week horizon shifts every week; and overrides are decisions scoped to one run, not model evidence. The workspace database already stores everything the comparison needs as immutable rows, so a stored comparison entity would duplicate derivable state (ADR-0007 principle: derived over duplicated).

## Decision Drivers

- Immutable revisions and runs are never rewritten; comparisons are read-only views over them.
- Missing is not zero (ADR-0005); no zero-fill in any comparison output.
- Overrides are run-scoped and never carry forward (ADR-0014); a comparison must not imply that they do.
- No schema change without a confirmed requirement and the ADR-0021 backup gate.
- Deterministic, reproducible outputs from stored rows alone.

## Decision

- **Pair selection**: the default comparison pairs the newest completed run of the latest import revision with the newest completed run of the **immediately preceding** import revision of the same source profile. An unforecast previous revision is not skipped and produces a "comparison unavailable" state, never an older run. If the latest revision has no completed run, there is no current run and no comparison.
- **Identity and alignment**: series match by `product_id × location_id` UUIDs; demand cells match by `week_start`; forecast values match by `week_start` against immutable baselines. Horizon indices are never used for alignment. Sessions compare baselines only; the current run's effective plan is displayed separately, and earlier overrides never influence a delta.
- **Demand changes** are derived at read time by comparing observation cells of the two revisions: `added` (current only), `removed` (previous only), or `changed` (quantity or status differs, with `NULL` distinguished from every value and from another `NULL` cell). No percentage or aggregate summaries are computed.
- **Forecast changes**: a common week counts as changed only when baseline values differ. A week present in one run but inside the other run's horizon counts as an availability change with no numeric delta. A week outside the other horizon is a non-comparable horizon edge and counts as no change. Delta values are shown only when both baselines exist.
- **Config suggestion**: the wizard preselects a saved configuration only when exactly one current configuration's stored descriptor JSON equals the latest preprocessed revision's frozen `descriptor_json`, computed by string equality over the deterministic serialization (ADR-0020). Zero or multiple matches preselect nothing; comparison never requires a stored config-to-revision reference.
- **Orchestration**: a completed non-duplicate import starts a forecast for the returned revision. A duplicate import reuses the revision's newest completed run and starts a forecast only when none exists. Duplicates never create redundant runs.
- **No new schema**: all comparison read models are derived in Rust queries; no tables, columns, or stored comparison artifacts are added.

## Consequences

Positive:

- The weekly flow is one user-triggered path with existing gates: preview findings still block imports, jobs still serialize, and exports still never overwrite.
- Comparison output is reproducible from stored rows; no migration and no backup gate are needed.
- Horizon shifts do not mislabel nearly every series as changed, because deltas exist only for common calendar weeks.

Negative:

- Changed-series lists require full cell comparisons across two revisions per query; acceptable at benchmark scale but re-measured during implementation.
- "Current" is defined as the newest completed run of the latest revision; a user who wants to compare two arbitrary runs must wait for a later milestone.

## Alternatives Considered

- **Stored comparison snapshots**: rejected; duplicates derivable state and needs a migration without a confirmed requirement.
- **Comparing two most recent runs regardless of revision**: rejected in planning; it can compare two executions of identical data and calls that a weekly change.
- **Aligning by horizon index**: rejected; both horizons are 12 weeks, so index alignment hides the one-week shift and compares different calendar weeks.
- **Effective-to-effective deltas**: rejected; overrides are run-scoped decisions, and mixing them into a "forecast change" obscures model movement (ADR-0014).
