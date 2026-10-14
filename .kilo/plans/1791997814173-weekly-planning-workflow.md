# M6 Weekly Planning Workflow

## Goal

Add one repeatable, user-triggered workflow that selects a new planner export, reuses the last unambiguously matching saved import configuration, validates and imports the file, forecasts the resulting revision, compares that run with the run for the immediately preceding revision, and lets the user export the current effective plan.

## Confirmed Decisions

- A successful non-duplicate import automatically starts a forecast for the returned revision.
- A duplicate import reuses that revision's newest completed run; it starts a forecast only when no completed run exists.
- The default comparison is the newest completed run for the latest revision versus the newest completed run for the immediately preceding revision in the same source profile. Do not skip an unforecast previous revision to find an older run.
- Forecast deltas compare immutable baselines. The current effective plan remains visible separately; overrides never carry between runs.
- Demand changes compare revision cells by stable `product_id × location_id × week_start`, including quantity and observation-status changes. Preserve missing versus explicit zero; do not compute percentage deltas.
- Forecast deltas compare the same calendar `week_start` only. Horizon edge weeks are visible as non-comparable but do not count as changes.
- “SKU level” means the existing `Series` grain: product/SKU × location. Do not aggregate locations.
- The comparison screen is a paginated list of changed series with a selected-series weekly detail.
- Preselect a saved import configuration only when exactly one current config descriptor equals the latest preprocessed revision's frozen `descriptor_json`. Otherwise require selection. No schema migration is needed.
- Export remains an explicit user action and uses the existing CSV contract for the current completed run.

## Domain And Contracts

Keep all comparison data derived at read time. Do not add tables or columns.

Add typed Rust/Specta read models in `src-tauri/src/types.rs` (names may follow repository conventions):

- `WeeklyComparisonContext`: current `RunSummary` and optional previous `RunSummary`.
- `SeriesChangeSummary`: product/location identities and display codes, demand-change count, forecast-change count, and series-presence state (`both`, `current_only`, `previous_only`).
- `DemandChangePoint`: week, previous/current quantity and status, and change kind (`added`, `removed`, `changed`).
- `ForecastChangePoint`: week, previous/current baseline, numeric delta only when both values exist, and comparability (`common`, `previous_only`, `current_only`).
- `SeriesComparisonDetail`: identities, previous/current run outcome metadata, demand changes, forecast comparison points, and the current run's existing planning points for baseline/override/effective display.

Add narrow typed IPC commands:

- `import_config_suggestion() -> Option<i32>`: return a config id only for one exact descriptor match.
- `weekly_comparison_context() -> WeeklyComparisonContext`: select the pair using the confirmed adjacent-revision rule and return `previous = null` when comparison is unavailable.
- `weekly_comparison_series_list(current_run_id, previous_run_id, search, offset, limit)`: validate the supplied frozen pair and return only changed series in stable SKU/location order.
- `weekly_comparison_series_detail(current_run_id, previous_run_id, product_id, location_id)`: return the weekly old/new/delta detail and current effective plan.

Rust must validate that both runs exist, are completed, belong to the same source profile, and represent adjacent revisions in the correct order. Cap list pagination consistently with `series_list`. Match identities by UUID, never SKU text. Use union-style SQL over persisted observations/forecasts so series and cells present on one side remain visible. Do not zero-fill absent observations or forecasts.

## User Workflow

1. Expose “Weekly update” from an open workspace even after revisions exist. Disable it while a job runs, an export runs, or the planner has dirty/saving edits.
2. Reuse `ImportWizard.svelte` preview, findings, mapping, and saved-config controls. On open, apply the uniquely suggested config if present; otherwise show the config selector. The user still selects the new file and confirms a fresh preview.
3. Start the import with the confirmed descriptor and monitor it through the existing job infrastructure.
4. On import completion, use the exact `ImportResult.revision_id`. For a new revision, start forecasting automatically. For a duplicate, select its newest completed run or start one when absent.
5. If forecasting completes, refresh workspace/runs, select that run as current, load comparison context, and open the change list. If no immediately previous revision/run exists, show a clear first-run empty state.
6. Selecting a changed series opens an accessible table of demand changes and baseline forecast old/new/delta, including non-comparable horizon edges, plus the current plan's baseline/override/effective values.
7. “Export current plan” invokes the existing non-overwriting CSV export for the current completed run.

## Failure And Edge Behavior

- Blocking preview findings stop before import; warnings remain confirmable under current rules.
- Import failure/cancellation starts no forecast and changes no committed revision.
- Forecast failure leaves the committed revision visible and offers retry for that exact revision.
- A duplicate with an existing completed run performs no new forecast work.
- A duplicate without a completed run starts one forecast.
- Missing previous revision/run is not an error; comparison is unavailable and export/current planning still work.
- A previous revision with no completed run is not skipped.
- A series present on only one side is listed; its unavailable forecast values have null deltas.
- Common forecast weeks with one missing baseline are changes in availability with null numeric delta; two missing baselines are unchanged.
- Exact config matching that finds zero or multiple rows does not preselect anything.
- Switching comparison identity must discard comparison UI selection/drafts but must not alter persisted planner overrides.

## Acceptance Criteria

- Given a saved matching config and a new valid export, when the user confirms preview, then one import and one forecast complete and the comparison opens for the new run.
- Given identical bytes and descriptor, when the workflow runs, then no new revision is created and an existing completed run is reused.
- Given a duplicate revision without a completed run, then exactly one forecast is started.
- Given adjacent revisions with corrected, added, removed, zero, and missing observations, then the changed-series list and detail preserve those distinctions.
- Given two shifted 12-week horizons, then deltas exist only for common calendar weeks and edge weeks are non-comparable.
- Given overrides in the earlier run, then baseline deltas ignore them and the current effective plan shows only current-run overrides.
- Given no previous completed run for the immediately previous revision, then comparison shows an empty state without selecting an older run.
- Given a completed current run, then CSV export retains the established columns, full-run scope, blank missing values, and no-overwrite behavior.

## Ordered Vertical-Slice Tickets

1. **Record comparison semantics.** Add the next ADR (currently ADR-0022) for adjacent-revision selection, UUID/week alignment, baseline-only forecast deltas, horizon-edge behavior, read-time derivation, and no override carry. Update `GLOSSARY.md`, `DATA_ARCHITECTURE.md`, and `TAURI_IPC.md` without editing accepted ADR text.
2. **Deliver config suggestion and weekly orchestration.** Add exact descriptor-match query/IPC, make the guided wizard available after first import, and implement import → duplicate handling → forecast orchestration using returned revision ids. Cover retries, cancellation, dirty planner blocking, and job refresh.
3. **Deliver comparison context and changed-series list.** Add typed context/list contracts and bounded Rust queries; wire generated bindings, frontend IPC helpers, pair empty states, search, pagination, and stable list selection.
4. **Deliver series comparison detail.** Add demand and forecast point derivation, current planning values, accessible old/new/delta tables, null/status labels, and non-comparable horizon edges. Reuse existing formatting and visual language; do not extend `ModelComparison.svelte` because it represents a different concept.
5. **Complete export integration and end-to-end proof.** Ensure comparison/current-run transitions target the existing export, update honest README milestone status, and add an integration path proving saved config → import → forecast → comparison → CSV.

## Tests

Add focused tests with names equivalent to:

- Rust: `import_config_suggestion_requires_one_exact_descriptor_match`, `weekly_context_selects_completed_runs_from_adjacent_revisions`, `weekly_context_does_not_skip_unforecast_revision`, `comparison_aligns_series_by_uuid_and_forecasts_by_week`, `comparison_preserves_zero_missing_and_status_changes`, `comparison_marks_horizon_edges_non_comparable`, `comparison_pages_changed_series_deterministically`.
- Frontend: `weekly_update_preselects_unique_matching_config`, `weekly_update_forecasts_new_revision`, `duplicate_revision_reuses_completed_run`, `duplicate_without_run_starts_forecast`, `comparison_list_opens_series_detail`, `dirty_planner_blocks_weekly_update`, `comparison_empty_state_keeps_current_export_available`.
- Integration: extend the milestone path with two distinct imports and completed runs, baseline changes, an override that does not affect the delta, and successful current-run CSV export.

Run all repository gates from `AGENTS.md`: Rust tests, Python/unittest suites, frontend check/test/build, schema/recovery tests, rustfmt check, and clippy with warnings denied. Regenerate `src/lib/bindings.ts` through the existing binding test after Rust contract changes.

## Explicitly Postponed

No scheduler, watched folder, remembered source-file path, cloud/network behavior, location aggregation, percentage/ranking summaries, comparison export, XLSX, override carry-forward, model-policy changes, sidecar changes, new dependency, or schema migration.
