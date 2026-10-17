# ADR-0026: Keep forecasting exploratory; move weekly orchestration to Pipelines

**Status:** Accepted
**Date:** 2026-10-17
**Supersedes:** ADR-0022's user-facing weekly orchestration only; its run-comparison semantics remain accepted.
**Related:** [ADR-0022](0022-weekly-planning-workflow-and-run-comparison.md), [ADR-0023](0023-model-input-imputation.md)

## Context

The forecasting workspace exposed a one-click weekly update that imported new sales, generated or reused a forecast, and opened the comparison. Weekly updates will belong to the planned Pipelines feature. Until then, the forecasting workspace must remain an exploratory surface where users import, forecast, inspect, compare, and export as separate actions.

## Decision Drivers

- Keep forecast generation explicit and user-controlled.
- Do not couple imports to forecast generation in the forecasting workspace.
- Preserve the read-only, derived comparison rules from ADR-0022.
- Ask for missing-week imputation per forecast run; never change imported history (ADR-0023).
- Do not add schema or IPC changes for this UI boundary.

## Decision

- Remove the one-click weekly update path from the forecasting workspace.
- Keep quick CSV import and guided import available as separate actions. A completed import does not start a forecast.
- Keep forecast generation as a separate, on-demand action. Each request asks whether to estimate missing weeks and applies any selected method to that run only.
- Keep run comparison as an explicit, read-only action. It compares the latest import with the immediately previous import under ADR-0022's existing alignment and availability rules; it does not start or trigger a forecast.
- The planned Pipelines feature owns future weekly import-and-forecast orchestration. Its design and implementation are out of scope for this ADR.

## Consequences

Positive:

- Users can inspect and compare forecasts without starting an import or a weekly workflow.
- Import, forecast, and comparison remain visible and independently controlled.
- Existing data and comparison contracts stay unchanged.

Negative:

- A user who imports new history must start its forecast explicitly before comparing it.
- The forecasting workspace no longer offers a single action for the weekly sequence.

## Alternatives Considered

- **Keep the one-click weekly update:** rejected because orchestration belongs in Pipelines, not the exploratory forecasting workspace.
- **Remove run comparison:** rejected because comparing stored runs is still useful for exploration and does not require orchestration.
