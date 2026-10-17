# Planner Review, FVA, And Replenishment

## Goal And Accepted Boundaries

Make OpenDemand useful for recurring professional planning through three features: an exception inbox with decision memory; retrospective planner Forecast Value Added (FVA); and explainable replenishment with optional intermittent-demand models.

- General-purpose positioning, not an industrial-distributor-only product. V1 remains weekly, local-first, single-planner, and one real location per workspace. Existing imports use the technical default location; do not claim multi-location ingestion exists.
- Keep import, forecast, comparison, publication, and replenishment explicit. No automatic orchestration or Pipelines implementation (ADR-0026).
- Notes carry across runs for the same series. Review completion does not. Overrides remain run-scoped, manual, and append-only; review dates are reminders, never automatic expiry.
- FVA requires explicit publication. One immutable official publication per source profile and history cutoff week. Other runs remain exploratory and editable. No retroactive publication dates or historical performance claims.
- FVA compares the published effective plan with its model baseline, not model-versus-Naive value added. Choose a pinned actuals revision and explicitly confirm actuals completeness.
- Default FVA trend horizon is h=2. With completed source weeks and publication before target-week start, a normal Monday review is too late for h=1. Never weaken the leakage rule or silently combine horizons to hide unavailable results.
- Replenishment uses a published plan plus a pinned inventory snapshot. Inputs come from local CSV/XLSX files, not ERP APIs. Safety stock is user-entered. Customer orders, reservations, backlog, and forecast-consumption logic are excluded and disclosed.
- Require Monday opening-stock snapshots, whole-week lead/review periods, and a protection period fully covered by the existing 12-week forecast. No daily allocation, forecast extension, transfers, production/BOM planning, perishability, automatic orders, service-level guarantees, or monetary risk scores.
- Croston-SBA and TSB are opt-in additions. Existing runs and the default model set stay unchanged. Missing observations never become zero automatically.

## Read First And Reuse

- `AGENTS.md`, `GLOSSARY.md`, `README.md`, `docs/architecture/{DATA_ARCHITECTURE,SYSTEM_ARCHITECTURE}.md`, and `docs/architecture/contracts/{TAURI_IPC,ARROW_SCHEMAS,SIDECAR_PROTOCOL}.md`.
- ADRs 0004-0008, 0012, 0014, 0017, 0019-0023, 0025-0026. Add successor/extension ADRs; do not rewrite accepted decision text.
- `src-tauri/src/workspace.rs`: `PLANNING_ROWS_SQL`, `run_overview`, `append_override`, revision/comparison queries, export lifecycle, and verified migration backups. `forecast_override.note` already exists but is unused.
- `src-tauri/src/{forecast,selection,engine,imputation,types,desktop}.rs`; `python/opendemand_engine/{modeling,tasks,metrics,__init__}.py`.
- `src/lib/components/{RunOverview,ForecastEditor,SeriesDetailPanel,WeeklyComparison}.svelte`, `src/lib/frontend/{planner,weekly,ipc,i18n}.ts`, and `src/routes/+page.svelte`. Reuse query scoping, dirty-draft guards, pagination, and en/pl/es dictionaries.
- Schema is version 3. Never edit migrations 001-003 or their checksums. Forecasts and actuals are immutable; effective values and comparisons are derived. Override timestamps have one-second precision and are not an exact publication checkpoint.
- Pinned StatsForecast 2.1.1 includes `CrostonSBA` and `TSB(alpha_d, alpha_p)`. Reuse these, not custom algorithms or a new dependency. `run_model.model_id` has a fixed SQL CHECK that must be extended safely before storing new models.

## Ordered Delivery

Implement vertical slices in this order. All three features are approved for planning; do not interpret this plan as authority to commit, publish releases, or change GitHub issues.

Migration numbers as implemented (superseding the "004-007" reservation): 004 planner decision memory (slice 1), 005 review policy (slice 2), 006 plan publication + actuals confirmations (slices 3-4), 007 inventory inputs (slice 5, extended in slice 6), 008 sparse-model registry rebuild (slice 7). Recheck the next free number before each new migration.

### 1. Persist Decision Memory

**Boundary:** migration 004, Rust workspace/types/IPC, existing planner editor, new review panel, generated bindings.

- Add append-only series notes and review events, scoped by source profile, product ID, location ID, and optional context run. Use UUID identities, never display labels. Notes remain visible across runs within that profile. Review state has only `unreviewed` and `reviewed`; reopening appends an event. No assignments or task engine.
- Each review event acknowledges a specific run and evidence context. Record context references/rule version and a deterministic evidence fingerprint, not copies of exception outcomes or comparison values. New runs, new relevant evidence, and later override edits invalidate the acknowledgment at read time. A reviewed flag never hides new evidence.
- Extend new override writes with required reason code, optional note, and optional review date; write the existing note column and add immutable companion metadata keyed to the exact override revision. Initial codes: `promotion`, `customer_information`, `supply_constraint`, `data_quality`, `other`; `other` requires a note. Codes express planner intent, not detected causes.
- Preserve existing value/clear semantics and optimistic concurrency. Clearing is still an explicit NULL revision. Historical overrides without metadata show `Legacy: reason not recorded`; never invent a reason or rewrite them. A due review date only creates a reminder while its override revision is current in the selected run.
- Notes are plain text with bounded length, no HTML, and no business text in logs. Keep note-only actions separate from value edits. Review/notes never change coverage or forecast outcomes.

**Acceptance:** reopen preserves notes and review events; a new run is unreviewed; later edits reopen relevant review; old notes remain discoverable; clear never revives an older override; conflicting saves fail atomically; existing overrides work after migration.

### 2. Deliver The Exception Inbox

**Boundary:** Rust derived queries plus a `Review` panel reached from the existing run overview. Preserve existing overview counts and weekly-comparison behavior.

- Derive one paginated row per series for the selected completed run, with all applicable rule codes and evidence links. Persist only user actions and versioned user thresholds, never exception membership, changed-cell counts, forecast deltas, or calculated priority.
- Fixed priority: forecast failures/invalid data; insufficient or imputed input; production fallback; configured material changes; configured persistent bias; due review reminders. Stable SKU/location/ID tie-breaks. Review status is a filter, not suppression of evidence.
- Start with existing outcome/reason evidence and missing-versus-zero semantics. For change rules, compare the selected run with the newest completed run of its immediately preceding import revision. Missing predecessor evidence makes that rule unavailable; never skip a revision or trigger forecasting.
- Material-change rules require user-set absolute and relative thresholds. Compute the sum of absolute baseline deltas over common calendar weeks, excluding horizon edges; require positive change and both thresholds. If the previous baseline magnitude is zero, use the absolute threshold and disclose the unavailable percentage. Do not compare units across SKUs for priority.
- Threshold-based rules stay disabled until configured. Persist policy versions per profile. Bias rules arrive with slice 4; no invented early bias score.
- Include reason chips, prior notes, review/reopen controls, and navigation to the existing series editor. Apply search/filter/order before pagination in Rust, using one read snapshot. Preserve drafts and reject stale workspace/run responses.

**Acceptance:** pagination/search cannot omit matching rows beyond the first page; one series can show multiple rules; rule changes reopen review; first-run/missing-predecessor states are explicit; keyboard use, locale parity, and narrow-window layouts work.

### 3. Publish An Exact Plan

**Boundary:** migration 005, Rust publication commands, planner publication control, immutable publication view/export path.

- Add publication headers with source profile, run, cutoff week, timezone, publication time, policy version, and the highest import revision ID visible for that profile in the publication transaction. This high-water mark distinguishes actuals present before publication without relying on second-resolution timestamps. Enforce uniqueness of `(profile_id, cutoff_week)` in SQLite. Publish only completed runs, with no unsaved edits, in the serialized writer transaction; reject duplicate publication and backdating.
- Pin the exact latest override revision for every overridden cell, including NULL clears, in publication checkpoint rows. Baselines already remain immutable in the pinned run. No timestamp reconstruction and no duplicate stored effective values: derive the published effective plan from these references.
- Capture a coherent snapshot even if an override save races publication: exactly one serial order wins. Later run edits do not change the publication. Do not repurpose an ordinary CSV/XLSX export as publication.
- Show publication coverage and require acknowledgment of series with no usable forecast. Keep missing values missing. Prevent source history from extending into unfinished/future weeks at publication; historical-cutoff runs may publish now, but past/current target weeks remain ineligible for FVA.
- The publication view and its export use pinned values and clear publication identification. Add a small publication-export link to the existing export record in migration 005. Existing ordinary run exports keep their existing live-saved-run semantics and layout. Reuse the safe export lifecycle; do not silently substitute the publication in existing commands.

**Acceptance:** exact revision checkpoint despite same-second edits; post-publication edit/clear cannot alter published values; duplicate publication and transaction failures leave no partial rows; concurrent save behavior is deterministic; old runs remain usable without fabricated publications.

### 4. Score Retrospective Planner FVA

**Boundary:** actuals-confirmation rows in migration 005; derived Rust reports; `Performance` panel; optional bias inbox rule. No persisted accuracy aggregates.

- Add append-only user confirmations of `actuals complete through` for a specific actuals revision. Validate a Monday no later than the last finished week in the workspace timezone. Corrections/retractions append a new confirmation. Reports pin the exact confirmation ID as well as revision ID.
- Default actuals selection to the newest revision of the publication's profile, but require explicit completeness confirmation. Allow selecting older revisions. Changing actuals or confirmation creates a different report view, not a rewritten publication.
- Join by product/location/calendar week. A scoreable point must be from an import later than the publication's profile revision high-water mark, have a publication strictly before target-week start in its timezone, be within the confirmed completed range, and have baseline, published effective value, and numeric actual. Reject hindsight: exclude any target actual present in any profile revision at or below that mark, even if later removed. Report future, late-published, absent, missing/partial, stockout-marked, and missing-forecast exclusions separately. Read actuals only from stored observations; never fill missing targets for scoring. Disclose the baseline's persisted imputation method/count per series. Exact imputed training weeks are not persisted, so do not invent per-week provenance.
- Report per series and horizon step across selected publications, plus a per-publication detail view. Default trend horizon is h=2; h=1..12 are selectable and stay separate. Explain why the standard Monday publication has no eligible h=1 result. Later publication can also disqualify h=2; show unavailable evidence rather than selecting a different horizon silently. Do not pool overlapping horizons or produce a cross-SKU accuracy score. Show eligible/scored/adjusted counts and low-sample warnings; no significance claim.
- Reuse Rust metric sums/conventions: WAPE=`sum(abs(A-F))/sum(abs(A))`, MAE=`sum(abs(A-F))/n`, bias=`sum(F-A)/sum(abs(A))`; positive bias means overforecasting. FVA is baseline WAPE minus effective WAPE, displayed in percentage points; positive means improvement. For all-zero actuals, WAPE/bias/FVA-pp are unavailable; show MAE improvement in quantity units, never relabel it as percentage FVA.
- Compare both forecasts on exactly the same scored cells. Show all comparable cells and adjusted-only evidence separately so unchanged cells do not conceal the adjustment effect. Numeric negative returns follow existing net-quantity metric semantics and carry a visible caveat, not clipping.
- Persistent-bias inbox rules require explicit horizon, target-count window, minimum count (at least two), and a positive bias-magnitude threshold. Use distinct mature target weeks at one horizon; require every error in the selected window to share a nonzero sign, and `abs(pooled_bias)` to exceed the threshold. A zero-error or opposite-sign point breaks persistence; do not silently drop it. Report overforecast/underforecast direction. Zero denominator/insufficient evidence means unavailable. Evidence includes publication IDs, actuals revision, confirmation, and policy version.

**Acceptance:** baseline `[10,20]`, adjusted `[12,18]`, actuals `[12,18]` yield WAPE 13.333...% versus 0% and +13.333... pp FVA. Cover harmful/unchanged adjustments, all-zero actuals, partial maturity, later corrections/removals, returns, imputed-input disclosure, series churn, and hindsight leakage. Test Sunday 23:59/Monday 00:00 local and DST boundaries, h=1 exclusion/h=2 eligibility for normal Monday publication, and later h=2 exclusion. Test persistent overforecast, underforecast, sign reversal, and exact-zero errors. No old exploratory run is shown as prospectively validated.

### 5. Import Complete Inventory Snapshots

**Boundary:** migration 006, new `inventory/1` local contract, Rust import/validation, existing parsing helpers and guided-import patterns. Do not alter `canonical/1` or silently activate its reserved unit field.

- Import one atomic inventory batch with three logical tables: opening stock; open dated inbound receipts; per-SKU policy. Support three CSV files or one XLSX with three sheets. Preview/mapping is explicit; an empty receipts table is valid only with confirmation that there are no open inbound receipts for the included scope.
- Header pins profile, Monday as-of date, default location, source/mapping fingerprints, contract version, and retained raw files. Confirm that sales and stock describe the same real location. Reject unknown SKU mappings; do not create inventory-only products or guess names.
- Stock rows contain explicit finite nonnegative unrestricted physical stock. Policy rows require unit code, whole-week lead time `L >= 0`, review interval `R >= 1`, finite safety stock/MOQ `>= 0`, and pack size `> 0`. Quantities in stock and receipts must use that declared unit; require confirmation that forecast quantities use the same unit. No UoM conversion or currency.
- Scope is the SKUs explicitly included in stock/policy rows; all open inbound for that scope must be included. Missing a SKU is not zero stock. Receipts use stable source row IDs, quantity, and Monday availability date; only remaining quantities not already reflected in opening stock are admissible, including same-Monday receipts. Reject duplicate/conflicting keys. Retain overdue open receipts with a preview warning and block recommendations for affected SKUs until a corrected snapshot supplies a reliable future date; never assume receipt occurred. Future receipts beyond the forecast are retained and disclosed, not counted inside the protection period.
- A changed batch creates a new immutable snapshot; identical content+mapping is a no-op. Policy changes require a new snapshot, never mutate a prior recommendation's inputs. Reuse raw-file retention, validation, checksums, and file-before-row durability for the entire batch; do not call the sales importer or overload its revisions/jobs without explicit schema support.

**Acceptance:** atomic rollback on any invalid file; explicit zero versus absent rows; unit mismatch; date errors; missing policy; duplicate receipts; overdue-receipt warning and affected-SKU blocking; idempotency; mapping changes; retained-file recovery; and workspace reopen. Preserve sales revisions and existing import behavior.

### 6. Explain And Export Replenishment

**Boundary:** Rust derived policy, new typed commands and `Replenishment` panel, separate XLSX export; inventory-export provenance in migration 006.

- Select one publication and inventory snapshot from the same profile/location. The snapshot Monday is the calculation anchor, within the published horizon. Require all `P=L+R` weekly forecast cells from that anchor. No wall-clock roll-forward or extrapolation; label historical inputs and their as-of dates.
- Base demand is the published effective plan. The P-week window includes the anchor Monday and excludes anchor+P weeks. For each SKU, compute `raw=max(0, demand_over_P + safety_stock - opening_stock - inbound_due_in_P)`. If raw is zero, order zero; otherwise `order=pack*ceil(max(raw,MOQ)/pack)`. Apply finite-number validation and tested rounding boundaries. One source/lead-time policy per SKU; no supplier optimization.
- Return a weekly stock projection: receipts arrive at the beginning of their availability week; forecast demand consumes stock during that week; the suggested order arrives at anchor+L weeks. Show pre-arrival shortages separately: a later order cannot repair an earlier service failure. Negative projected balances denote cumulative uncovered forecast demand, not known customer backlog or a promised lost-sales model.
- Block a SKU's suggestion for missing inputs, mismatched units, unavailable/negative forecast cells, overdue inbound, or insufficient horizon. Preserve the reason and inputs; never replace blocked quantity with zero. Model fallback/imputed training raises warnings, not invented demand corrections.
- Show every named arithmetic term, rounding effect, receipt timing, input IDs, and policy version. State that history is net sales, stockouts can suppress observed sales, and no demand uncensoring, order/backlog netting, or service probability is provided.
- Base scenario uses multiplier 1. Optional low/high demand multipliers must be user-entered (`0 <= low <= 1 <= high`); scale demand only, not stock or safety stock. Label these sensitivity scenarios, never prediction intervals. Do not persist calculated recommendations.
- Export a separate workbook with `Recommendations`, `Projection`, and `Summary`, retaining existing forecast XLSX `Plan`/`Summary` unchanged. Pin publication, inventory snapshot, scenario, calculation version, and export ID in provenance; reuse no-overwrite publication/retained-copy behavior and `export_record(format='xlsx')` with an explicit companion record distinguishing the export kind.

**Acceptance:** raw 13, MOQ 15, pack 4 yields 16; zero need yields zero despite MOQ; inbound after P does not reduce the order; late receipts expose earlier shortages; horizon >12 blocks; missing is not zero; negative forecasts block rather than clip. Reopening/exporting pinned inputs reproduces the same recommendation, including after later run edits.

### 7. Add Opt-In Intermittent Models

**Boundary:** migration 007, Rust forecast planning/selection/validation, Python dispatch/advertisement, model-set IPC, model labels and export summary.

- Extend the `run_model` CHECK through a tested table-rebuild migration. Preserve every dependent FK, row, index, and trigger. SQLite cannot change this CHECK through `ALTER COLUMN`; do not assume `PRAGMA foreign_keys=OFF` inside a migration transaction works. First prove the create/copy/drop/rename mechanism on a populated in-memory fixture, including whether deferred FK checks work with the current SQLx transaction. Validate commit, reopen, and an empty `PRAGMA foreign_key_check` on realistic v3-v6 fixtures. If the existing migrator cannot perform a safe atomic rebuild, block this slice and resolve the migration mechanism explicitly; never bypass constraints or silently switch to non-atomic DDL.
- Add `croston_sba` and `tsb` via pinned StatsForecast classes. Initial policy: training floor 12 numeric weeks; TSB `alpha_d=0.1`, `alpha_p=0.1`, frozen in versioned run configuration, not fitted against held-out targets. These are explicit baseline policy choices, not claimed optimal parameters. Smoke-test pinned-library behavior before accepting them; a library incompatibility blocks this slice rather than prompting a custom implementation.
- Default remains the existing three-model set. The opt-in set adds both candidates per run; freeze model set, floors, parameters, and selection-policy version in `config_json`. Old runs never change.
- Sparse-model task windows must be complete, nonnegative, and contain no imputed training points. Rust excludes imputed windows using its in-memory mask; Python validates the supplied numeric, nonnegative, complete window. Do not claim Python can recover imputation provenance from the unchanged Arrow input. Apply eligibility per production/backtest task. Unsupported series retain existing-model behavior with a specific eligibility explanation; do not invalidate the entire run or convert gaps/returns to zero.
- Keep comparable-origin scoring, at least two completed origins, zero-actual MAE fallback, and Naive fallback. Deterministic tie order: Naive, SeasonalNaive, Croston-SBA, TSB, AutoETS; the relative order of existing models is unchanged. Test thresholds around 12/24/36 weeks and common-origin loss.
- Keep Arrow/JSONL versions unchanged: model IDs are already data fields. Advertise supported IDs, validate them in Rust, update hard-coded summary registries, and preserve cancellation, timeout, and bounded-pool behavior. No probability intervals or service-level claims.

**Acceptance:** all-zero, single-spike, long gaps of explicit zeros, constant demand, obsolescence, negative returns, true missing weeks, imputed histories, task failure, opt-out parity, sequential/pool output parity, and real-sidecar persistence/reopen. Record runtime and comparative results; do not assert improved accuracy without evidence.

## Architecture And Rollout Rules

- Rust remains sole database/filesystem/policy owner. Python only fits/predicts. Svelte renders generated IPC; regenerate `src/lib/bindings.ts`, never edit it manually.
- Use existing modules where practical. A focused replenishment policy module is justified by its own tested calculation boundary; do not introduce generic workflow, rule-engine, connector, report, or optimizer frameworks. No new dependency is planned.
- Reserve migrations 004-007 in this delivery order, rechecking numbers before implementation. Each new migration uses the verified-backup gate, a schema-version bump, checksum checks, and populated-workspace upgrade tests. Never rewrite a migration after it ships; rollback is a compatible binary or verified restore, not destructive down-migrations.
- Add focused ADRs for review/publication semantics, retrospective evaluation, inventory contracts/replenishment, and sparse-model policy. Update IPC/data/glossary/current-scope documents as each slice ships. Preserve historical ADR text and ordinary forecast export contracts.
- Keep persisted user decisions, publications, inventory inputs, confirmations, and export provenance. Derive inbox membership, FVA, effective plan values, and replenishment. No automatic deletion of business records.
- Test each slice independently. The inbox ships before bias evidence exists; FVA appears only for publications; replenishment requires published plans and complete inventory inputs. Models are an optional final extension, not a prerequisite for stock arithmetic.

## Validation And Stop Conditions

- Extend `src-tauri/tests/{planner,weekly,milestone,preprocess}.rs`, frontend planner/i18n tests, engine contract/behavior/pool tests, and architecture migration/recovery tests. Add focused test files only where an existing suite is not a clear home.
- Migration tests cover every supported starting version, integrity and foreign-key checks, backup failure/read-only recovery, checksums, old forecasts/overrides/exports, trigger preservation, and interruption safety. Inventory ingestion tests cover both database and retained files.
- Use injected clocks for publication/completeness/week-boundary tests. Keep the existing historical quickstart files truthful: they demonstrate forecasts, not prospective FVA. Add a separate synthetic multi-revision scenario for review, publication, mature actuals, inventory, and sparse models.
- Manual check: import, forecast, review/note/override, publish, edit exploratory run, import later actuals and confirm completeness, inspect unchanged publication/FVA, import inventory, inspect scenarios, export/reopen. Confirm keyboard access, en/pl/es, narrow layouts, dirty guards, and actual Excel/LibreOffice opening where available.
- Run `cargo test --manifest-path src-tauri/Cargo.toml`, the existing headless/no-default-features gate, `uv run python -m unittest discover`, `npm run check`, `npm test`, `npm run build`, `python3 -m unittest discover -s tests/architecture -v`, `cargo fmt --manifest-path src-tauri/Cargo.toml --check`, and `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`.
- Planning used read-only inspection; tests/builds have not been run. Current unrelated `session-ses_ec1a.md` is untracked and must remain untouched.
- Stop when the seven slices meet their acceptance checks. No automatic purchasing, additional model families, generalized multi-location data migration, ERP synchronization, Pipelines, MCP, telemetry, release publication, or unrelated cleanup.
