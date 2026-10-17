# Five Quick Wins For First Real Planners

## Goal And Priority

The user selected **First real planners**: help a new tester produce a usable plan, hand it to Excel, and repeat the weekly review. Optimize for reuse of the current workflow, not feature count or production-readiness claims.

| Rank | Win | Planner benefit | Reuse boundary |
|---|---|---|---|
| 1 | A complete sample workflow | Reach a first forecast and export without architecture documentation | Existing guided import, automatic forecast, comparison, CSV export; GitHub #20 |
| 2 | Persistent run overview and exceptions | See whether a completed run produced usable forecasts, including after restart | Persisted run outcomes and the aggregate already used by CSV export |
| 3 | Focused weekly-change filters | Separate forecast changes, demand corrections, and series arrivals/removals | Existing read-time comparison and change counts; add bounded read-only filtering |
| 4 | Usable language selection and localized comparison | Polish and Spanish testers can read the changes they must review | Existing locale column and en/pl/es dictionaries; GitHub #22 |
| 5 | Native XLSX handoff | Open a saved full-run plan in Excel without manual CSV conversion | Existing safe CSV export lifecycle; GitHub #26 |

Ranks express expected value relative to change surface, not measured adoption results. XLSX has the broadest change surface in this package and comes after the smaller improvements. The implementation order below also accounts for shared files.

## Current Facts And Constraints

- Read `AGENTS.md`, `GLOSSARY.md`, `README.md`, `docs/architecture/contracts/TAURI_IPC.md`, and the cited ADRs before implementation.
- Milestone 6 already includes saved import configurations, automatic forecasting after a confirmed weekly import, changed-series comparison, manual run-scoped overrides, and full-run CSV export. Do not rebuild them.
- Migration 002 and its verified pre-migration database backup gate are already implemented. Full retained-file backups and guided restore remain GitHub #25 and are not quick wins.
- The weekly list already contains changed series only. The proposed filters refine that list; they do not add another redundant "changed only" mode.
- Per-series WAPE, MAE, bias, comparable selection scores, and model comparison already exist. This package does not invent a portfolio accuracy score.
- Plain-language per-series explanations already exist in `ModelComparison.svelte` and `SeriesDetailPanel.svelte`. Do not duplicate them. The missing surface is the durable full-run overview.
- Rust stays the sole owner of SQLite and filesystem access. The frontend uses generated typed IPC. Python model policy, canonical/1, and existing override semantics stay unchanged.
- Comparison stays derived at read time. No schema migration or persisted comparison state is permitted.
- Preserve the current working-tree edits, including the 0.5.3 version changes, workspace picker, desktop shell, and page changes. Re-read these files before integrating with them.

## Explicit Non-Goals

No monthly forecasting, monthly-to-weekly allocation, new models, threshold-based risk scoring, pooled or averaged cross-series WAPE, automatic ordering, multi-user support, cloud behavior, MCP, watched-folder imports, periodic backup/restore, installer signing, or release publication. Do not create or close GitHub issues, commit, push, or publish as part of implementation without separate authorization.

## Resolved Scope Choices

- Keep XLSX **Plan + Summary**, as required by accepted ADR-0007. The layout question received no answer, so preserve the repository decision rather than silently remove a sheet. GitHub #26 asks for one sheet; record this difference in the implementation handoff, not as an unapproved issue edit.
- XLSX is the only new dependency: a compatible, pinned `rust_xlsxwriter` release, with only required features. `calamine` already reads workbooks and can verify outputs in tests. Do not use Python or hand-written OOXML for export.
- The only new UI component needed is a small `RunOverview.svelte`. Reuse the existing planner selection, dictionary, query, button, and status-chip patterns; do not add a settings framework, dashboard framework, or new frontend dependency.
- New controls and overview text use en/pl/es keys. Localization is limited to import and weekly-review surfaces, not a promise that every application screen or persisted backend diagnostic is translated.

## Ordered Implementation Tickets

Implement each ticket as a complete vertical slice. The order below minimizes repeated changes to shared files; it is not the benefit ranking above.

### 1. Deliver A Real Sample Workflow (#20)

**Files:** `examples/quickstart/first-import.csv`, `examples/quickstart/weekly-update.csv`, existing `README.md`, `CONTRIBUTING.md`, `site/index.html`, `site/pl/index.html`, and `src-tauri/tests/weekly.rs`.

- Add two small synthetic files with `series_key,week_start,quantity`, usable through Guided import by mapping those columns. Snapshot A has three products with 48 consecutive Monday weeks ending 2026-10-05, quantities 10/20/30. Snapshot B retains that history, adds 2026-10-12 for all three, changes the second product's final quantity from 20 to 25, and corrects one older observation of the first product from 10 to 12.
- Fixed historical dates remain valid: the current core uses the revision's maximum week as its cutoff, not today's date. All series have enough history for evaluated Naive champions, but not seasonal models. Explain that explicitly; do not imply this file demonstrates a three-model contest. Do not use a 16-week first-run file, which would flag every series as a fallback.
- Add a short **First Forecast** section to README, not another documentation tree. Give concrete clicks: create a separate sample workspace, Guided import A, confirm mapping and save its configuration, import, press **Generate forecast** for this initial non-weekly import, inspect a series, and export CSV.
- Add an optional **Next Weekly Update** continuation: import B through **Weekly update**, reuse the saved configuration, let the existing workflow forecast automatically, compare, save one override, and export. Show expected results: three usable series, 36 export data rows, the second product's baseline changes from 20 to 25 on common horizon weeks, historical correction does not become an override, and horizon edges are not forecast changes.
- Link this section and the example downloads from the existing contribution page and both catalog pages. Do not change release asset URLs or introduce a bundled demo loader.

**Acceptance/tests:** extend the real-sidecar weekly integration path to consume these exact files; assert completed runs, expected comparisons, saved override/effective values, a 36-row CSV, and duplicate-B no-op/reuse. A fresh tester can follow the guide without Python, developer commands, or architecture docs. Retain CSV completion as an independent outcome before XLSX exists.

### 2. Make Language Selection Work And Localize Comparison (#22)

**Files:** `src-tauri/src/{workspace,desktop,types}.rs`, `src/lib/frontend/{ipc,i18n,weekly}.ts`, `src/lib/components/{WorkspacePanel,WeeklyComparison,WeeklyChangeDetail}.svelte`, `src/routes/+page.svelte`, generated `src/lib/bindings.ts`, and relevant workspace/frontend tests.

- Add `workspace_set_locale(locale) -> WorkspaceInfo`, validating the existing supported values en/pl/es. Update only `workspace_settings.locale` through the serialized Rust writer and a transaction. Invalid values fail with a stable documented `invalid_locale` error; failed writes leave the previous locale intact. No migration or new setting is needed.
- Add one labelled language selector to the existing workspace panel. Update/refetch the workspace query after success; show an error and retain the old choice on failure. Persist the choice across reopen. Language changes must not discard planner drafts, change the selected run, or rewrite parsing descriptors, revisions, timezone, or numeric data.
- Pass the resolved locale into both comparison components. Move their copy, accessibility labels, presence/status labels, horizon notes, and interpolated messages into the existing dictionaries. Localize the comparison-specific paths of `weekly.ts`; leave unrelated shared `format.ts` consumers unchanged.
- Explain the limited translation scope near the selector. Canonical field names, identifiers, and persisted diagnostic text remain unchanged. Unknown legacy locale values still resolve to English through the existing resolver.

**Acceptance/tests:** verify persistence and invalid-input handling in Rust; update `tests/frontend/{i18n,weekly,workspace}.test.ts` for dictionary parity, interpolation, locale propagation, and no state loss. Manually switch languages and reopen a workspace; confirm import and comparison follow the same selection. Use the terminology of `site/pl/index.html`.

### 3. Add A Persistent Run Overview With Actionable Exceptions

**Files:** `src-tauri/src/{workspace,types,desktop}.rs`, `src/lib/frontend/ipc.ts`, new `src/lib/components/RunOverview.svelte`, `src/routes/+page.svelte`, generated bindings, `src-tauri/tests/{planner,milestone}.rs`, and frontend planner tests.

- Factor the existing export outcome histogram into a small shared Rust helper, inside `workspace.rs`, that can use the caller's read snapshot. Reuse it for CSV metadata, this overview, and the later XLSX Summary. Do not move it into a new service/module.
- Add read-only `run_overview(run_id, offset, limit) -> RunOverview`: total series, covered series, coverage, all five outcome counts including zero counts, exception total, and a paginated exception list. Coverage is `(ok + fallback_naive) / total`, matching the current export; an empty run reports zero and the UI says **No series**. This is coverage, not forecast accuracy or confidence.
- An exception is `status != 'ok' OR reason IS NOT NULL`. This includes the important case where a champion failed in production but another model produced an `ok` forecast. Each entry contains product/location IDs, SKU/location labels, outcome, and the existing reason. Order by `model_failure`, `invalid_data`, `insufficient_history`, `fallback_naive`, then `ok` with reason; use SKU/location/identity tie-breaks. Default page size 20; clamp to the existing maximum 200.
- Counts and exception totals cover the complete selected run, not the current series page. Read the response from one consistent read-only snapshot. Preserve `unknown_run` and `run_not_completed` behavior. Do not fetch every series detail, aggregate WAPE/bias, add model-task charts, or create a risk threshold.
- Show the strip below the selected-run controls and above the existing detail view. Clicking an exception opens the existing series detail for that same run using product/location UUIDs and the existing dirty-edit guard. Query by workspace path, run ID, and exception offset; reset pagination on workspace/run changes and reject stale responses. Loading/error/retry must not block the existing planner or export.

**Acceptance/tests:** use existing mixed `tests/fixtures/demo.csv` to prove one `ok`, one `fallback_naive`, one `insufficient_history`, coverage 2/3, and the expected exceptions. Add fixtures/assertions for production fallback with `ok` plus reason, runs whose forecasts are all missing, zero series, pagination beyond one page, historical-run selection, and reopen/reload. Assert parity with CSV `coverage_json`; verify exception navigation cannot discard unsaved edits. No state is persisted for the overview.

### 4. Focus Weekly Review By Change Type

**Files:** `WEEKLY_CHANGED_SERIES_SQL` and `Workspace::weekly_comparison_series_list` in `src-tauri/src/workspace.rs`, the matching command in `desktop.rs`, a typed filter in `types.rs`, generated bindings, `src/lib/frontend/ipc.ts`, `WeeklyComparison.svelte`, dictionaries, `src-tauri/tests/weekly.rs`, and `tests/frontend/weekly.test.ts`.

- Add one typed `WeeklyChangeFilter`: `all`, `demand`, `forecast`, `presence`. `all` preserves today's result; `demand` requires `demand_change_count > 0`; `forecast` requires `forecast_change_count > 0`; `presence` requires `current_only` or `previous_only`. Filters are intentionally overlapping, not partitions.
- Pass the chosen filter through the existing list command. Apply it in Rust SQL together with search **before** `LIMIT/OFFSET`; retain existing SKU/location ordering and the current bounded-page response. Do not sort/filter just the browser's current page, fetch all pages, add magnitude ranking, or claim economic importance from changed-cell counts.
- Add one translated selector above the changed-series list. Reset offset and selection when search, filter, or comparison pair changes; select a row only from the new result. Use a distinct **No changes match this filter** state, not **Nothing changed**. Preserve first-run/missing-previous-run states, baseline-only comparison, and all null/zero/horizon-edge semantics.

**Acceptance/tests:** combine search, each filter, overlapping changes, added/removed series, and more than one page. Put a matching series beyond the original first page to prove server-side filtering. Confirm a horizon-edge-only difference never passes `forecast`, the default `all` is unchanged, and selection never displays an old filtered-out row.

### 5. Export A Native Excel Workbook (#26, ADR-0007)

**Files:** `src-tauri/Cargo.{toml,lock}`, `src-tauri/src/{workspace,desktop}.rs`, `src/lib/frontend/ipc.ts`, existing export controls in `src/routes/+page.svelte`, generated bindings, `src-tauri/tests/planner.rs`, and the weekly integration test.

- Add `export_xlsx(run_id, path) -> ExportResult` and one matching save dialog/button. Keep `export_csv` and its output unchanged. Both formats target the selected completed run, exclude unsaved drafts, and use the existing exporting/dirty/saving guards.
- Share the existing snapshot/batched-query/publication lifecycle between the two real formats, using a small private format enum or equivalent explicit branching, not a generic exporter framework. Reuse `PLANNING_ROWS_SQL`, `load_planning_rows`, and `planning_point`; do not recalculate effective forecasts in the frontend or Python.
- **Plan** sheet: exactly the 12 CSV columns in their current order, one data row per series/horizon week. Write `week_start` as real Excel dates with ISO display, quantities and horizon as numbers, and identity/status fields as literal strings. Preserve zero versus empty values, latest clear semantics, diacritics, leading-zero SKUs, and formula-like text without formulas. No charts, report styling, or alternative planner layout.
- **Summary** sheet: selected run/revision identity, horizon and engine version, total/covered series, coverage, all outcome counts, and statistical champion counts from that run's `series_model_evaluation.is_champion` rows. Read these from the same snapshot as Plan. Label champion distribution separately from the model actually used after production fallback. Use static values, not spreadsheet formulas.
- Path rules mirror CSV with `.xlsx`: absolute path, existing parent, no overwrite, workspace-internal destination restrictions, retained `exports/<uuid>.xlsx`, `export_record(format='xlsx')`, completed export job, fsync and no-clobber publication. The frozen schema already permits xlsx; do not migrate. Reuse established error codes; map workbook write/worksheet-limit failures to `export_failed`, with no successful record or partial published files.
- Preserve cleanup and race behavior on write/publish/DB failure. The existing hard-kill orphan risk R-22 remains documented, not silently solved or worsened. Preserve batched database reads and choose the writer's supported memory-efficient mode where appropriate; batching alone is not proof of bounded workbook memory.

**Acceptance/tests:** read the workbook with existing `calamine`; assert Plan/CSV cell parity, 36 sample rows, typed dates/numbers, blank versus zero, escaped/formula-like names, diacritics, latest override/clear, and Summary/count parity. Repeat existing CSV path, no-overwrite/race, retained-copy, batching, unknown-run, failed-publication, and DB rollback tests for XLSX. Both files must remain usable in one workflow; dialog cancellation creates no artifacts. Manually open output in Excel and LibreOffice when available; explicitly report any unavailable application rather than claiming compatibility was verified.

## Contracts, Documentation, And Rollout

1. Before each code ticket, re-read relevant existing edits; do not replace the dirty `desktop.rs`, `+page.svelte`, workspace picker, manifest, or lockfile wholesale. Execute shared-core tickets sequentially, not with multiple writers editing those files.
2. Update `TAURI_IPC.md` for locale mutation, run overview, comparison filter, and XLSX. Update `DATA_ARCHITECTURE.md` only for derived-read/output descriptions; never change frozen SQL. Add the next unused ADR for the XLSX writer/layout implementation, preserving ADR-0007's Summary requirement and all accepted historical text.
3. Regenerate `src/lib/bindings.ts` with `cargo test --manifest-path src-tauri/Cargo.toml export_generated_frontend_bindings`; do not edit generated command/types manually. Review binding diffs after each IPC slice.
4. Update honest README/catalog status and current deferred-feature lists only as each capability is implemented. Do not describe this package as production-ready, change version/release links, or close issues automatically.
5. No migration or backfill is needed. Existing schema-v3 workspaces gain derived reads and optional XLSX; language is the only new persisted user choice and uses an existing column. Existing CSV files, runs, revisions, and overrides stay valid. Reverting the feature code does not remove user data or export artifacts.

## Verification And Stop Conditions

Run focused tests after each slice, then the repository gates before completion:

```sh
cargo test --locked --manifest-path src-tauri/Cargo.toml
cargo test --locked --manifest-path src-tauri/Cargo.toml --no-default-features
uv run python -m unittest discover
npm run check
npm test
npm run build
python3 -m unittest discover -s tests/architecture -v
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo clippy --locked --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
```

Use RTK for supported output-heavy commands if installed. Review generated bindings and confirm migration 001/hash are unchanged. Planning performed read-only code/GitHub inspection; these tests have **not** been run during planning.

Final manual scenario: separate sample workspace -> A import/forecast -> overview -> B weekly update -> filter comparison -> select a changed series -> save one override -> CSV and XLSX export -> reopen and confirm the run overview and language choice. Also check keyboard access, visible labels, narrow-window layouts, locale changes, and an exception click while a draft is dirty.

Stop when these five observable wins and their gates are complete. Do not add extra settings, analytics, model policies, scheduling, integrations, or cleanup. The adoption hypothesis is validated separately by observing real planners complete this scenario on their own exports, without collecting business data or adding telemetry.
