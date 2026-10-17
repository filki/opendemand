# Data Architecture — OpenDemand

Scope: the logical data model, semantics, and lifecycle rules for the workspace database. Rationale lives in the ADRs; the physical schema is `schema/001_init.sql`. All technical text is English; user-facing copy is localized separately (`en`, `pl`, `es`).

## 1. Principles

- **One writer**: only the Rust core opens the database; all writes serialize through it (ADR-0002).
- **Immutability**: import revisions and forecast runs are never mutated or auto-deleted; overrides are append-only (ADR-0007).
- **Honest data**: missing is unknown, zero is explicit, and every deviation carries a status (ADR-0005).
- **Explicit identity**: UUIDv7 ids for products and locations; external codes map explicitly (ADR-0004).
- **Derived over duplicated**: the effective forecast and all coverage statistics are derived at read time, never stored.
- **Deterministic math**: eligibility, selection scores, ranks, and champion selection are per series and reproducible from stored rows alone (ADR-0006, ADR-0012).
- **Recoverable durability**: retained source files are made durable before the SQLite commit that references them; startup reconciliation reports recoverable leftovers and never silently deletes them (ADR-0015).

## 2. Entity Catalog

### Workspace and configuration

| Entity | Purpose | Key fields | Notes |
|--------|---------|------------|-------|
| `workspace_settings` | Workspace configuration, single row | `timezone` (IANA), `locale` (en/pl/es), `mcp_enabled`, `schema_version` | Timezone is confirmed at workspace creation and snapshotted into every run |
| `source_profile` | Source definition; at most one active profile | `name`, `is_active`, `notes` | `uq_source_profile_active` enforces a single active profile; switching profiles does not touch existing revisions |
| `job` | Job audit trail | `kind` (import/forecast/export/backup/migration), `status`, timestamps, error | Failed import attempts live here, never in `import_revision`; diagnostic record; never stores business payloads |

### Import and staging

| Entity | Purpose | Key fields | Notes |
|--------|---------|------------|-------|
| `import_revision` | Committed immutable import snapshot | `profile_id`, `content_hash`, `mapping_fingerprint`, `contract_version`, `source_filename`, `imported_at`, counts | `content_hash` = SHA-256 of the raw retained bytes; `UNIQUE(profile_id, content_hash, mapping_fingerprint, contract_version)` is the idempotency key (ADR-0013) |
| `source_file` | Retained CSV copy | `revision_id`, `relative_path`, `sha256`, `byte_size` | `relative_path` is globally unique (`sources/<revision_id>/<safe_filename>`; canonical imports use `source.csv`); files live in the workspace directory, not in the database |
| `import_issue` | Data-quality diagnostics | `revision_id`, `severity`, `code`, `detail`, `row_ref` | Unmapped codes, duplicates, parse problems |
| `observation` | Weekly cell per revision | `revision_id`, `product_id`, `location_id`, `week_start`, `quantity`, `status` | Primary key includes `revision_id`: full snapshots coexist |
| `series_qualification` | Eligibility per series per revision | `h_length`, `status`, `reason` | `H` is stored here; computed at import |

### Master data and mappings

| Entity | Purpose | Key fields | Notes |
|--------|---------|------------|-------|
| `product` | Product master | `id` BLOB(16) UUIDv7, `sku` unique, `name`, `is_active` | SKU is the canonical display code, not the identity |
| `location` | Location master | `id` BLOB(16) UUIDv7, `code` unique, `name` | |
| `product_code_map` | External code → product | `(profile_id, source_code)` → `product_id` | Profile-scoped; no name matching (ADR-0004) |
| `location_code_map` | External code → location | `(profile_id, source_code)` → `location_id` | |

### Forecast runs and outcomes

| Entity | Purpose | Key fields | Notes |
|--------|---------|------------|-------|
| `forecast_run` | Immutable run record | `id` UUIDv7, `revision_id`, `status`, `horizon_weeks` = 12, `horizon_start_week`, `timezone`, `config_json`, versions | Pins the revision it consumed; `horizon_start_week` is the Monday after the maximum imported week in that revision, never the wall clock (ADR-0013) |
| `run_model` | Run-level execution summary | `(run_id, model_id)`, `status` (completed/partial/failed/not_run), `tasks_planned`, `tasks_completed`, `tasks_failed`, failure code/detail | Execution summary only; no champion, selection metric, or per-series data (ADR-0012) |
| `series_model_evaluation` | Per-series model eligibility and selection | `(run_id, product_id, location_id, model_id)`, `eligibility_status` (not_eligible/production_only/champion_eligible), `origins_planned`, `origins_completed`, `selection_score`, `selection_rank`, `is_champion`, `production_status` (not_run/completed/failed), failure code/detail | At most one champion per run/series (partial unique index); computed by Rust (ADR-0012) |
| `series_model_origin` | Per-series model origin result | `(run_id, product_id, location_id, model_id, origin_index)` 1–4, `origin_week_start`, `train_size`, `status` (completed/failed), `wape`, `mae`, `metric_kind` (wape/mae_fallback), `metric_value`, failure code/detail | Origin dates, train sizes, and metrics vary per series; always retained, even in aggregates-only mode |
| `backtest_forecast` | Per-point backtest values | `(run_id, product_id, location_id, model_id, origin_index, week_start)` → `value` | FK to the full series_model_origin key; review data; aggregates-only mode may omit these rows but never series_model_origin |
| `forecast_series_status` | Run-level series outcome | `status` (ok/fallback_naive/insufficient_history/model_failure/invalid_data), `selected_model_id`, `reason` | `selected_model_id` references the series_model_evaluation row; champion metric and origin counts are queried from that row, never duplicated here |
| `forecast` | Baseline forecast values | `(run_id, product, location, week)` → `value`, `selected_model_id` | Model output only; never edited; `selected_model_id` references the matching series_model_evaluation row |
| `forecast_override` | Manual edits, append-only | `(run_id, product, location, week, revision)` → `value` (NULL = cleared), `note`, `created_at` | Highest revision is effective; no auto-carry between runs; manual-only edits are allowed with guardrails (ADR-0014) |
| `export_record` | Export audit | `run_id`, `format`, `relative_path`, `row_count`, `coverage_json` | |

**Derived, never stored**: effective forecast (latest override ?? baseline), coverage statistics, series lists, storage report numbers, unmapped-code reports.

## 3. Identity and Mappings

- UUIDv7 ids are generated on first mapping of an external code and stored as `BLOB(16)`; they never change and are never reused.
- Mapping tables are profile-scoped; the same source code in two profiles may map to different products.
- Import resolves every row through the mapping tables. Unmapped codes produce `import_issue` rows (warning) and are excluded; no name-based fallback exists.
- The temporary input contract `canonical/1` (`series_key`, `week_start`, `quantity`) creates an exact identity mapping (`series_key` → new product) on first sight; no fuzzy or name-based matching exists. All `canonical/1` rows land on the workspace's fixed technical default location so the product/location architecture stays intact (ADR-0013).
- The effective mapping fingerprint is computed deterministically from the mapping plan, including identity mappings created by that import; it is part of the import idempotency key (ADR-0013).
- SKU and location code are display and search keys only.

## 4. Time Semantics

- A week is Monday–Sunday, identified by `week_start`: the ISO date (`YYYY-MM-DD`) of its Monday in the workspace timezone.
- The workspace timezone (IANA) is confirmed at setup and snapshotted into each forecast run; changing it later does not rewrite history.
- Weeks are calendar weeks in local time, not 168-hour buckets; DST transitions do not shift week boundaries.
- `H` (usable history) is the length of the trailing run of consecutive weeks with numeric values, ending at the last completed week. `missing` and `partial` weeks break the run (ADR-0005).
- The newest week may be `partial` until the source marks it complete; it never counts toward `H`.
- Forecast runs use a global canonical cutoff: `horizon_start_week` is the Monday after the maximum imported week in the pinned revision, never the wall clock. All series align to this cutoff, and weeks absent for a series are treated as missing for trailing-history qualification (ADR-0013).
- Forecast horizons are the 12 weeks starting at the run's persisted `horizon_start_week`; `horizon` in exports is the 1-based index within that span.

## 5. Missing Data and Observation Statuses

| Status | Numeric value | Counts toward `H` | Meaning |
|--------|---------------|-------------------|---------|
| `known` | yes | yes | Normal reported value |
| `zero` | yes (0) | yes | Source explicitly reports zero demand |
| `missing` | no (NULL) | no (breaks run) | No source value; unknown |
| `return` | yes (negative net) | yes | Return/credit reducing net demand |
| `correction` | yes | yes | Source-corrected value |
| `stockout` | yes | yes | Demand truncated by stockout; flagged only in MVP |
| `partial` | no (NULL) | no (breaks run) | Incomplete newest week |

Modeling input is the net weekly quantity from numeric statuses; NaN values are never persisted, and `missing` is never exported as zero. SQL enforces the status/value nullability and `zero = 0`; the Rust core additionally rejects nonnumeric and nonfinite quantities before persistence. The `canonical/1` importer derives `known`, `zero`, or `return` from the numeric quantity; `missing`/`partial` semantics remain for future sources (ADR-0005, ADR-0013).

## 6. Import Protocol (file-before-row, idempotent)

Input contract is `canonical/1`: `series_key`, `week_start` (ISO Monday), `quantity` (finite, non-null). The importer never guesses: `content_hash` is the SHA-256 of the raw retained bytes, and `contract_version` qualifies the parsing semantics of those bytes — `canonical/1` for a direct canonical CSV, `preprocess/1` for a revision produced by the preprocessing layer (ADR-0019). The preprocessing layer converts arbitrary planner files into the `canonical/1` observation shape without changing the engine contract, and its parsing descriptor joins the effective mapping plan in the fingerprint (ADR-0020; canonical CSV details in `contracts/TAURI_IPC.md`).

1. Validate the input: a canonical CSV checks UTF-8 and the exact header; a raw export is decoded per its descriptor, its header row and column bindings are resolved, document rows are summed into Monday weeks, and a blocking problem fails the job. Compute the raw-bytes SHA-256 and build the complete mapping plan (exact identity mappings for new `series_key`s; fixed technical default location; descriptor digest for preprocessed imports). Identical duplicate rows are collapsed (info issue); conflicting duplicates are excluded (error issue).
2. Check idempotency before retaining another file: same `(profile_id, content_hash, mapping_fingerprint, contract_version)` is a no-op returning the existing committed revision.
3. Copy to a workspace staging file while computing SHA-256; flush and fsync the staged file.
4. Begin the serialized SQLite transaction, allocate the revision id, and insert the proposed rows.
5. Atomically rename the staged file to `sources/<revision_id>/<safe_filename>` and fsync the relevant directories.
6. Insert the globally unique relative path in `source_file`, then commit SQLite.
7. After commit, run a WAL checkpoint and return the import summary (counts, issues, qualification breakdown).

Crash safety: a crash before commit may leave an orphan file but can never leave a committed revision referencing a file that was not made durable. Startup reconciliation detects stale staging files, orphan final files, missing committed files, and hash mismatches; recoverable artifacts move to a recovery/quarantine area and are reported, never silently deleted (ADR-0015).

Failed attempts live in `job`; `import_revision` represents committed snapshots only and never blocks a retry. Same content plus a changed mapping fingerprint creates a new immutable revision. A preprocessed revision also stores the frozen parsing descriptor (`descriptor_json`, migration 002), so editing or deleting a saved configuration cannot change how a committed revision is explained (ADR-0021).

Retention: revisions and their CSV copies are never auto-deleted; storage is managed through visibility and explicit user action only.

## 7. Qualification and Eligibility

- Qualification statuses (stored per revision): `eligible`, `insufficient_history` (`H < 12`), `invalid_data`.
- Forecast eligibility, backtest windows, champion rules, outcome vocabularies, and the threshold table are defined in **ADR-0006**; the canonical window count is `k = max(0, min(4, floor((H - f) / 12)))`; champion needs `c >= 2` completed origins.
- Per-series model evaluation is stored at `(run_id, product_id, location_id, model_id)` granularity with `eligibility_status` (`not_eligible | production_only | champion_eligible`), planned/completed origin counts, selection score/rank, `is_champion`, production status (`not_run | completed | failed`), and failure code/detail. Origins are stored per series/model at `(…, origin_index)` with origin date, train size, `completed | failed`, WAPE, MAE, and the selected metric kind/value (`wape` or `mae_fallback`) (ADR-0012).
- Rust computes eligibility, ranking, champion selection, and fallback; Python executes Rust-planned tasks and returns metrics and forecasts only.
- Run outcomes (`forecast_series_status.status`): `ok`, `fallback_naive`, `insufficient_history`, `model_failure`, `invalid_data`; missing forecasts are blank cells with the outcome as the reason.

## 8. Forecast Value Layers

- `baseline` (stored, immutable): model output per cell.
- `override` (append-only): revisions per cell; NULL clears; highest revision wins; scoped to one run. Manual-only overrides may target any series in a completed run's scope even without a baseline; the week must lie inside the persisted 12-week horizon; the first revision must be non-null; revisions are sequential and append-only; UPDATE and DELETE are rejected (ADR-0014).
- `effective` (derived): latest override ?? baseline, and `missing` when neither exists (cleared manual-only cell); computed in queries and exports.

## 9. Retention and Storage Estimate

Assumptions: benchmark scale 10,000 SKUs × 156 weeks ≈ 1.56M observations, one location. Estimates are first-order and must be re-measured during implementation (risk R-11).

| Component | Size estimate | Growth rule |
|-----------|---------------|-------------|
| Database rows per full snapshot | ~160–230 MB (~100–150 B/observation incl. indexes) | One per import revision |
| Retained CSV copy | ~45–80 MB | One per import revision |
| **Import revision total** | **~0.2–0.3 GB** | monthly ≈ 2.5–3.5 GB/yr; weekly ≈ 10–15 GB/yr; daily ≈ 50–80 GB/yr |
| Backup pool | ≤ ~4× database size | 3 most recent periodic backups + latest pre-migration backup |
| Production forecasts per run | ~15–25 MB | One per run; runs never auto-deleted |
| Per-point backtest results per run | ~150–220 MB | Aggregates-only mode (<5 MB/run) may omit backtest_forecast points only; series_model_evaluation and series_model_origin are always retained so selection stays reproducible (ADR-0012) |

Rules: import revisions, runs, and overrides are never auto-deleted. Backup files follow the retention policy (ADR-0009). The UI shows a storage report (database, CSV copies, backups, runs).

## 10. Export Contract

CSV uses RFC 4180, UTF-8 with BOM, CRLF records, and a `.` decimal separator. XLSX uses the same columns in its `Plan` sheet (ADR-0025):

`run_id`, `product_sku`, `product_name`, `location_code`, `week_start`, `horizon`, `baseline`, `override`, `effective`, `value_source` (`model` | `override` | `missing`), `series_status`, `model_id`.

- One row per series × week for all series in the run scope (eligible and ineligible), so coverage is visible.
- Missing forecasts: `effective` is blank and `series_status` carries the reason; never zero-filled.
- Each successful CSV or XLSX export retains a workspace copy under `exports/` and an `export_record` with row count and coverage metadata. Export reads one saved run snapshot; existing files are never overwritten. The existing format constraint already permits XLSX; no migration is needed.
- Coverage = share of series with outcome `ok` or `fallback_naive`; reported in the XLSX Summary sheet with outcome counts and champion distribution.
- XLSX dates and quantities are typed cells. Missing values remain blank, including after the latest override revision clears a manual-only value. Product identifiers are literal text, never formulas. Summary champion counts use `series_model_evaluation.is_champion`, not the selected production fallback model.
- The run overview uses the same derived coverage aggregate, includes all five outcome counts, and pages exceptions separately from its full-run totals. It is not a portfolio accuracy score and adds no stored state.

## 11. Weekly Planning Comparison (derived)

The weekly workflow (ADR-0022) adds no entities. Everything is derived at read time from immutable revisions, runs, baselines, and overrides:

- Pair selection: newest completed run of the latest revision versus newest completed run of the immediately preceding revision of the same profile; an unforecast previous revision yields "comparison unavailable" and is never skipped.
- Series align by `product_id × location_id` UUIDs; cells align by `week_start`. Horizon indices are never used for alignment.
- Demand changes compare observation cells of the two revisions (`added`, `removed`, `changed`); `missing` stays distinct from zero, `NULL` is distinguished from every value, and no percentage or aggregate is computed.
- Forecast deltas compare immutable baselines of the two runs: a common week changes only when values differ; a week inside the other run's horizon without a value counts as an availability change (no numeric delta); a week outside the other horizon is a non-comparable edge and counts as no change.
- The current run's effective plan (latest override ?? baseline) is displayed separately; overrides are never carried into a delta.
- The wizard's configuration suggestion is derived by exact string equality between a saved configuration's descriptor JSON and the latest preprocessed revision's frozen `descriptor_json`; zero or multiple matches suggest nothing.
- The changed-series list can filter demand changes, forecast changes, or series presence changes. Filters combine with search before pagination and keep the original SKU/location ordering; no comparison policy or persisted state changes.

## Related Documents

- ADRs: `adr/README.md` (0001–0016; semantics in 0004–0007 and 0012–0015)
- Contracts: `contracts/` (TAURI_IPC, SIDECAR_PROTOCOL, ARROW_SCHEMAS)
- System structure: `SYSTEM_ARCHITECTURE.md`
- Schema: `schema/001_init.sql` (frozen; SHA-256 in `schema/001_init.sha256`)
