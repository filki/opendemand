# ADR-0031: Inventory inputs and explainable replenishment

**Status:** Accepted
**Date:** 2026-10-17
**Related:** [ADR-0005](0005-missing-data-semantics.md), [ADR-0019](0019-guided-import-preprocessing.md), [ADR-0025](0025-xlsx-export.md), [ADR-0029](0029-plan-publication.md), [ADR-0030](0030-retrospective-fva.md), [ADR-0026](0026-forecast-workspace-exploration-and-pipeline-orchestration.md)

## Context

Distributors need purchasing decisions, not only forecast curves. The workspace stores none of the inputs a safe order suggestion needs — stock, inbound, lead times, MOQs — and history is net sales, so the tool must never imply unconstrained demand or a service-level guarantee it cannot deliver.

## Decision Drivers

- Local-first, inspectable inputs: versioned files, retained raw bytes, immutable snapshots, no ERP APIs.
- Explicit arithmetic the planner can check term by term, in the series' own unit; no hidden scores.
- Missing data blocks a suggestion with a named reason; it is never read as zero.
- Stay weekly and single-location; no daily allocation, transfers, or horizon extension.

## Decision

- Add one versioned local contract, `inventory/1` (migration 008): three files (`stock.csv`, `receipts.csv`, `policy.csv`) or one workbook with the exact sheets `Stock`, `Receipts`, `Policy`, with pinned headers — no fuzzy mapping, like `canonical/1`. The batch commits as one immutable `inventory_snapshot`.
- Scope and confirmations: stock and policy must name exactly the same SKUs, all against series of the latest import at the default location (inventory never creates products); receipts must sit inside that scope; an empty receipts table and unit consistency (stock and receipts in the declared unit, forecast in the same unit) require explicit confirmations. Every SKU has a unit code; no conversions and no currency.
- Malformed rows, duplicate keys, non-Monday dates, unknown SKUs, overflow numbers, negative stock, invalid policies, and scope mismatches are blocking findings. Overdue open receipts (dated before the snapshot) are retained with the `overdue` state, warn in the preview, and block that SKU's suggestion until a corrected snapshot arrives; receipts beyond the 12-week horizon are retained and disclosed but never counted inside the protection window. Identical content under the same contract is a no-op.
- The derived replenishment plan (nothing stored) pins one publication and one snapshot of the same profile. The snapshot Monday is the anchor and must lie inside the published horizon; the protection window is `P = lead + review` whole weeks starting at the anchor, and every window week must have a published effective value. `raw = max(0, demand over P + user safety stock − opening stock − open receipts due inside the window)`; `order = pack × ceil(max(raw, MOQ) / pack)`, zero when raw is zero. One order per series; multi-supplier policies, order netting, and customer backlogs are out of scope and disclosed.
- The row returns every term (stock, inbound, demand, need, MOQ, pack, rounding, unit, L/R/P), a 12-week projection where receipts arrive at the beginning of their availability week and the suggested order arrives after the lead time, and explicit blocks (`not_in_publication`, `missing_policy`, `overdue_receipt`, `anchor_before_horizon`, `protection_too_long`, `missing_forecast`) and warnings (`imputed_input`, `beyond_horizon_receipt`, `early_shortage`). Pre-arrival shortages are disclosed; a later order never hides an earlier service failure.
- Low/high demand multipliers are user-entered sensitivities (`0 ≤ low ≤ 1 ≤ high`) that scale demand only — never stock or safety stock, never called prediction intervals. The XLSX export writes `Recommendations`, `Projection`, and `Summary` sheets (columns and ordinary export rules unchanged), records `export_record.inventory_snapshot_id` plus the publication link, and pins the calculation label `replenishment/1`.
- Honest limitations stay visible: history is net sales, stockouts can suppress observed sales, and no demand uncensoring, reservations, backlog, or service probability is provided. Whether sales equal demand remains governed by ADR-0005; this feature does not reopen it.

## Consequences

Positive:

- A purchasing suggestion a planner can audit line by line, reproducible from pinned inputs and exportable with provenance.
- Inputs are testable files with retained bytes; mistakes become findings, not wrong orders.
- Nothing about the plan is stored, so later overrides or edits cannot silently change an exported recommendation.

Negative:

- The exact-header contract is stricter than the guided import wizard; a planner renaming columns gets a schema error instead of a mapping UI (deliberate first step).
- The anchor rule (snapshot inside the published horizon) requires the planner to snapshot stock on the horizon's first Monday for the cleanest results; earlier snapshots block explicitly.

## Alternatives Considered

- **ERP connectors:** rejected now; credentials, synchronization, and failure behavior are a separate project.
- **Guided mapping wizard for inventory:** deferred; the pinned-header contract keeps v1 exact and testable.
- **Service-level (cycle service) policy:** rejected for v1; it needs calibrated uncertainty, especially for intermittent demand, and would imply guarantees the data cannot support.
- **Counting receipts by assuming overdue ones arrived:** rejected; never infer an event the data does not state.
- **Storing computed recommendations:** rejected; duplicates derived state and would drift from pinned inputs.
