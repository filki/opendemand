# Tauri IPC Contract

The Rust `tauri-specta` command collection generates `src/lib/bindings.ts`.
Generated bindings, not hand-maintained frontend interfaces, are the source of truth.
The UI never opens SQLite or runs the Python engine directly. This document covers the implemented command surface.

## Commands

- `workspace_create(path: string | null)`: create an empty workspace. A null path uses the application data directory. An explicit path must be an empty directory without a workspace database.
- `workspace_open(path: string)`: open an existing workspace and reconcile retained files. A successful open records the path in the recent-workspaces list.
- `workspace_close()`: close the active workspace and return to the home flow. Refused with `workspace_busy` while a job runs.
- `workspace_info()`: return the active workspace, latest revision, locale, and recovery findings.
- `workspace_set_locale(locale: string)`: persist `en`, `pl`, or `es` in the existing workspace settings and return `WorkspaceInfo`. Unsupported values fail with `invalid_locale`; no import descriptor, timezone, or run changes. Import and weekly-review copy follow this choice; other screens and stored diagnostics can remain English.
- `workspaces_list()`: return the home screen's workspace list — recent workspaces plus the default application-data workspace while it exists — each with `exists` reflecting its database file.
- `workspaces_forget(path: string)`: remove one recent-workspaces entry. The default application-data workspace reappears while its database exists.
- `workspace_rename(path: string, name: string)`: rename a workspace by moving its directory and re-pointing the registry entry; returns the updated `WorkspaceRef`. Error codes: `invalid_workspace_name`, `workspace_name_taken`, `workspace_not_found`, `workspace_default`, `workspace_busy` (open workspace or active job), `io_error`.
- `workspace_delete(path: string, confirm_name: string)`: permanently delete the whole workspace directory (database, sources, exports, backups). `confirm_name` must equal the current workspace name exactly; the expected name is derived from the path. Error codes: `confirm_name_mismatch`, `workspace_not_found`, `workspace_default`, `workspace_busy` (open workspace or active job), `io_error`.
- `import_start(path: string, descriptor: ParsingDescriptor | null)`: queue an import and return its initial `JobInfo` including the job id. A null descriptor imports a canonical CSV; a descriptor selects the raw-source preprocessing path.
- `import_preview(path: string, descriptor: ParsingDescriptor | null)`: run the read-only preview pass and return `ImportPreview`. A null descriptor runs detection only; nothing is written and no revision is created.
- `import_configs_list()`: return the saved import configurations of the active profile, ordered by name.
- `import_config_save(name: string, descriptor: ParsingDescriptor)`: save a named configuration; an existing name is updated in place.
- `import_config_delete(id: number)`: delete a saved configuration; revisions keep their frozen descriptor.
- `import_config_suggestion()`: return the id of the single saved configuration whose descriptor exactly matches the latest preprocessed revision's frozen descriptor; `null` when there is no match or more than one match.
- `weekly_comparison_context()`: return the default comparison pair (newest completed run of the latest revision and of the immediately preceding revision). Null members mark an unavailable comparison.
- `weekly_comparison_series_list(current_run_id: string, previous_run_id: string, search: string, filter: WeeklyChangeFilter, offset: number, limit: number)`: return only changed series of the validated pair, in stable SKU/location order. `filter` is `all`, `demand`, `forecast`, or `presence`; filters and search apply before pagination. Demand and forecast filters can overlap; presence selects added/removed series. Rust rejects runs that are not completed or not adjacent revisions of one profile.
- `weekly_comparison_series_detail(current_run_id: string, previous_run_id: string, product_id: string, location_id: string)`: return the weekly demand and baseline forecast comparison plus the current run's effective plan for one series.
- `forecast_start(revision_id: number, imputation: string | null)`: queue a run pinned to an immutable revision and return its initial `JobInfo`. `imputation` is `last`, `last_seasonal_52`, `mean_4`, or `null`; it fills missing weeks in the run's model input only (ADR-0023) and neither the stored history nor the exports change.
- `job_status(job_id: string)`: return progress, terminal error, and import/forecast result.
- `job_cancel(job_id: string)`: request cancellation; never cancel a committed transaction retroactively.
- `jobs_list()`: return recent jobs, including terminal jobs recovered from SQLite.
- `series_list(revision_id: number, search: string, offset: number, limit: number)`: return revision-scoped series. Limit is 1..200; offset is nonnegative.
- `series_detail(revision_id: number, product_id: string, location_id: string, run_id: string | null)`: return history, persisted baseline forecasts, planning points, and override capability. Run and revision must match.
- `runs_list(revision_id: number | null)`: return persisted runs for a revision; `null` returns run history across all revisions.
- `run_overview(run_id: string, offset: number, limit: number)`: read full-run coverage/outcome counts and a paginated exception list from one read-only snapshot. Limit is clamped to 1..200; the UI requests 20. Unknown and incomplete runs fail with `unknown_run` and `run_not_completed`.
- `override_append(edit: OverrideEdit)`: append a run-scoped value or clearing revision and return `OverrideRevision`.
- `series_note_add(edit: SeriesNoteEdit)`: append one profile-scoped planner note for a series and return the stored `SeriesNote`. The run provides the profile scope and must be completed; the series must be part of it.
- `series_review_set(edit: ReviewActionEdit)`: append one review or reopen event for a series and run and return the derived `SeriesReviewInfo`. The evidence fingerprint is computed server-side at append time.
- `review_policy()`: return the active profile's current inbox policy (latest version, or all-null when never configured).
- `review_policy_set(edit: InboxPolicyEdit)`: append one policy version and return it. Both thresholds enable the material-change rule; both empty disable it; a partial pair fails with `invalid_policy` and appends nothing.
- `exception_inbox(run_id: string, search: string, filter: string, offset: number, limit: number)`: derive the ranked exception inbox for one completed run (ADR-0028). `filter` is `all`, `unreviewed`, or `reviewed`; unknown values fail with `invalid_filter`. Limit clamps to 1..200; search and filter apply before pagination in one read snapshot. Unknown and incomplete runs fail with `unknown_run` and `run_not_completed`.
- `export_csv(run_id: string, path: string)`: publish the saved full-run plan to a new CSV file and return `ExportResult`.
- `export_xlsx(run_id: string, path: string)`: publish the same saved plan as a new XLSX workbook with `Plan` and `Summary` sheets; return `ExportResult`.

UUIDs are lowercase hyphenated strings at this boundary and `BLOB(16)` inside SQLite.
Revision ids are JSON numbers; Rust rejects values outside the supported integer range.
Jobs use string ids to correlate Tauri, SQLite, and sidecar messages.

## Results

- `WorkspaceInfo`: `path`, `schema_version`, `latest_revision_id`, `locale` (`en`, `pl`, `es`), `recovery_issues`.
- `ImportResult`: `revision_id`, `is_duplicate`, `series_count`, `observation_count`, `issue_count`, `last_week`.
- `SeriesSummary`: `product_id`, `location_id`, `location_code`, `series_key`, `h_length`, `qualification_status`.
- `HistoryPoint`: `week_start`, nullable `quantity`, `status`.
- `ForecastPoint`: `week_start`, finite `value`.
- `SeriesDetail`: `product_id`, `location_id`, `location_code`, `series_key`, `history`, `forecast`, `planning`, `can_override`, nullable `run_id`, `outcome`, `reason`, `horizon_start_week`, `selected_model_id`, and `selection_score`, the run's nullable `imputation_method` and `imputed_weeks`, profile-scoped `notes` (newest first, independent of the run), and the derived `review` state for the selected run (null without a run).
- `PlanningPoint`: `week_start`, 1-based `horizon`, nullable `baseline`, `override_value`, and `effective`, `value_source` (`model | override | missing`), `override_revision` (0 only when no revision exists), and the latest revision's nullable `override_reason`, `override_note`, and `override_review_date`. A non-zero `override_revision` with a null `override_reason` is a legacy override (ADR-0027).
- `SeriesNote`: `id`, `note`, nullable `context_run_id` (the run open when the note was written), `created_at`.
- `SeriesReviewInfo`: derived `status` (`reviewed | unreviewed`) and nullable `reviewed_at`. A stored acknowledgement whose evidence fingerprint no longer matches reads as `unreviewed`.
- `InboxPolicy`: nullable `change_abs`, `change_rel`, and `version`. Both thresholds are null together (rule disabled) or set (numeric threshold in the series' own units, and a fraction like 0.25). `version` is null when never configured.
- `InboxItem`: series identity (`product_id`, `location_id`, `series_key`, `location_code`), `outcome`, nullable `reason`, `rules` (codes in fixed priority order), nullable `change_abs`/`change_rel` (material change evidence; the relative value is null when the previous baseline magnitude is zero), nullable `due_review_date`, `review_status` (`reviewed | unreviewed`), nullable `reviewed_at`, `note_count`, and nullable `last_note_at`.
- `ExceptionInbox`: `run_id`, `policy`, nullable `predecessor_run_id` (newest completed run of the immediately preceding revision), `material_change_available`, `total` (matching rows across the run), and paginated `rows`.
- `RunSummary`: `id`, `revision_id`, `status`, `created_at`, `horizon_start_week`, `engine_version`.
- `RunOverview`: `series_count`, `covered_series`, `coverage`, `outcomes` (all five outcome keys, including zero counts), `exception_count`, and `exceptions`. Coverage is the fraction with `ok` or `fallback_naive`, not forecast accuracy; zero series yields zero coverage. Totals are independent of pagination.
- `RunException`: `product_id`, `location_id`, `series_key`, `location_code`, `outcome`, nullable `reason`. Exceptions have non-`ok` status or a recorded reason, including `ok` after production fallback. Order is model failure, invalid data, insufficient history, Naive fallback, then `ok` with a reason, followed by SKU/location/identity tie-breaks.
- `OverrideRevision`: the newly appended `revision` number.
- `ParsingDescriptor`: the versioned description of how one source file becomes observations. All fields are nullable for partial detection results: `descriptor_version`, `format` (`csv`/`xlsx`), `encoding`, `delimiter`, `header_row`, `sheet`, `key_columns`, `key_separator`, `date_column`, `date_formats`, `quantity_column`, `unit_column` (reserved, unused), `decimal_separator`, `thousands_separator`, `aggregation`.
- `ImportPreview`: `detected` (descriptor with detection results merged in), `headers`, `sample_rows`, `parsed_rows` (composed `series_key`, Monday `week_start`, parsed `quantity`, `status`), `findings`, `can_parse`, `truncated`, and `sheets` (workbook sheet names, empty for text exports).
- `PreviewFinding`: `severity` (`blocking`/`warning`), stable `code`, `detail`, optional `row_ref` (physical line or sheet row).
- `ImportConfig`: `id`, `name`, `descriptor`, `updated_at`.
- `WeeklyComparisonContext`: nullable `current` and `previous` `RunSummary` rows; null `previous` means no comparison exists for the current pair.
- `SeriesChangeSummary`: `product_id`, `location_id`, `location_code`, `series_key`, `demand_change_count`, `forecast_change_count`, and `presence` (`both | current_only | previous_only`).
- `DemandChangePoint`: `week_start`, nullable previous/current `quantity` and `status`, and `change_kind` (`added | removed | changed`).
- `ForecastChangePoint`: `week_start`, nullable previous/current baseline and `delta`, `comparability` (`common | previous_only | current_only`), and `is_change` (false for horizon edges and equal values).
- `SeriesComparisonDetail`: series identity, `presence`, nullable run outcomes, `demand_changes`, `forecast_changes`, and the current run's `planning` points.
- `ExportResult`: `export_id` (string form of the database identifier), selected `path`, and `row_count`.
- `ForecastResult`: `run_id`, `revision_id`, `status`, `series_count`, `forecast_count`, `coverage`.
- `JobInfo`: `id`, `kind`, `status`, `phase`, `progress` (fraction 0..1), nullable `error`, `import_result`, and `forecast_result`.
- `AppError`: stable `code` and user-readable `message`; filesystem paths and raw process stderr are not exposed as error details.

Commands return a typed Result. Tauri event `job-progress` carries `JobInfo`.
Events are advisory; `job_status`, `jobs_list`, and `runs_list` restore authoritative state.
Import parsing and forecasting run outside the UI thread. One import and one forecast may run concurrently; writes serialize through the Rust-owned connection.
Workspace switching is rejected while an import or forecast job is active. The UI also blocks switching while edits are pending or an export is in progress.

## Run-Scoped Overrides

`OverrideEdit` contains:

| Field | Type | Meaning |
|-------|------|---------|
| `run_id` | string | UUID of the completed forecast run |
| `product_id` | string | Product UUID in the run scope |
| `location_id` | string | Location UUID in the run scope |
| `week_start` | string | ISO Monday in the persisted 12-week horizon |
| `value` | string or null | Raw numeric input; `null` explicitly clears an active override |
| `expected_revision` | number | Last observed override revision; 0 for a cell with no revision |
| `reason_code` | string | Required planner reason: `promotion`, `customer_information`, `supply_constraint`, `data_quality`, or `other` (ADR-0027) |
| `note` | string or null | Optional free-text note (max 1000 characters); required with `other`; stored on the same revision |
| `review_date` | string or null | Optional reminder date (`YYYY-MM-DD`); a reminder only, never automatic expiry |

Rust trims and parses `value` to a finite number. Blank strings, NaN, and infinity are invalid; they never become an accidental clear. Validation includes completed run status, series scope, and horizon membership. Errors include `unknown_run`, `run_not_completed`, `invalid_week`, `invalid_value`, `outside_horizon`, `unknown_series`, `override_conflict`, `no_override`, `invalid_reason`, `note_required`, `invalid_note`, and `invalid_review_date`.

Every accepted write appends the next revision, including a clear, and pairs it with an immutable `override_meta` row in the same transaction. A due review date surfaces as a reminder while its revision stays the latest; it never clears or expires an override. Historical overrides without metadata stay readable as legacy (`reason not recorded`); they are never backfilled.

Every accepted write appends the next revision, including a clear. Baseline forecasts remain immutable, and overrides never carry into another run. A stale `expected_revision` fails with `override_conflict` rather than replacing a newer edit. `can_override` is true only for a completed run and a series in its scope; a model baseline is not required, so manual-only values are permitted (ADR-0014).

The effective forecast uses the latest override revision's value when non-null, otherwise the baseline; it never searches backward past a clear. If neither value exists it is missing, not zero. A clear retains its revision number for the next concurrency check. The UI preserves invalid and stale drafts, requires explicit refresh/rebase for conflicts, and pins the edited run when background imports or forecasts complete.

## Decision memory

`SeriesNoteEdit` carries `run_id`, `product_id`, `location_id`, and `note`; `ReviewActionEdit` carries the same identity plus `action` (`reviewed` or `reopened`). Both require a completed run and a series in its scope; errors include `unknown_run`, `run_not_completed`, `unknown_series`, `invalid_note`, and `invalid_action`. Notes are profile-scoped and survive across runs; review state belongs to one run and reopens automatically when its evidence changes (ADR-0027). Decision memory never changes coverage, outcomes, or exports.

## Exception inbox

The inbox (ADR-0028) derives one row per exception series of the selected completed run with all applicable rule codes in the fixed priority order: `model_failure`, `invalid_data`, `insufficient_history`, `imputed_input`, `production_fallback`, `material_change`, `review_due`. Material-change evidence compares the run with the newest completed run of the immediately preceding revision: the sum of absolute baseline deltas over common calendar weeks and its ratio to the previous magnitude; when that magnitude is zero the ratio is null and the absolute threshold alone decides. Thresholds are per profile, append-only versions; the rule stays off until both are set. Review state is a filter, never suppression: reviewed rows stay listed and reopen automatically when evidence changes. No monetary impact, no cross-series score, and no stored inbox state.

## Full-Run CSV And XLSX Export

Export reads the saved plan for every series in the selected completed run and all 12 horizon weeks from one read-only WAL snapshot, using bounded keyset batches. Unsaved UI drafts are not exported. Columns are defined in `../DATA_ARCHITECTURE.md`; missing values are blank. Files use UTF-8 with a BOM, CRLF records, and a `.` decimal separator.

XLSX `Plan` has the same columns and rows. Dates and quantities are typed cells; missing values remain blank. Identity strings remain literal text, including leading zeroes and formula-like names. `Summary` contains run/revision/horizon/engine identity, coverage, outcome counts, and statistical champion counts. Champion counts are distinct from the model used after production fallback. Values are static, with no spreadsheet formulas (ADR-0025).

The selected path must be absolute with a case-insensitive `.csv` or `.xlsx` extension matching the command, have an existing parent directory, and not already exist. Destinations inside the workspace are rejected except directly in its `exports/` directory. Validation errors include `invalid_export_path`, `export_exists`, `unknown_run`, and `run_not_completed`; write failures can report `export_failed` or a common I/O error. Workbook write and worksheet-limit errors report `export_failed`.

Rust writes and syncs temporary files before no-overwrite publication. A successful export includes the selected file, a retained workspace copy at `exports/<uuid>.<csv|xlsx>`, an `export_record`, and a completed export job. The command returns only after publication and the database transaction succeed; export cancellation is not implemented. XLSX uses constant-memory worksheet output and bounded database reads.

A hard kill between publication and database commit can leave a complete orphan export/archive or hidden temporary file without an `export_record`. Startup reconciliation does not reclaim these export artifacts yet (risk R-22). The CSV native save-dialog flow was manually verified on macOS on 2026-10-14. The new XLSX native dialog and opening output in Excel/LibreOffice still require manual application checks; automated workbook checks use `calamine`.

## Weekly comparison

`weekly_comparison_context` pairs the newest completed run of the latest revision with the newest completed run of the immediately preceding revision of the same source profile. An unforecast previous revision yields a null `previous` and is never skipped; when the latest revision has no completed run, both members are null.

List and detail commands re-validate the supplied pair: both runs must exist, be completed, and pin adjacent revisions of one profile. Series align by `product_id × location_id` and calendar `week_start`. Forecast deltas compare immutable baselines only; a week inside the other run's horizon without a value counts as an availability change with no numeric delta, and a week outside it is a non-comparable edge (`is_change` false). Demand changes preserve `missing` versus zero and distinguish `NULL` from every value; nothing is zero-filled and no percentage is computed. Comparison output is derived at read time; no comparison rows are stored (ADR-0022).

## Raw-source preprocessing

The guided import converts a raw planner export into the `canonical/1` observation shape without changing the parser, the engine, or the sidecar (ADR-0019). Supported inputs are CSV and TXT (UTF-8 with or without BOM, UTF-16 with BOM, Windows-1250/1252, ISO-8859-2) and `.xlsx` workbooks.

Detection proposes an encoding, a delimiter, and the header row (the first physical line with at least two non-empty fields, within the first 20 rows); a workbook also reports its sheet names. Detection is never silent: the preview decodes a head window (64 KiB for text exports) and the wizard shows the result. Native workbook date and number cells keep their type and never pass through separator or format guessing.

Interpretation rules: the composed key joins the bound key columns in header order with the key separator (a key part containing the separator is excluded); dates must match one of the accepted formats (or be native workbook dates); document rows are summed into Monday–Sunday weeks in the workspace timezone; the weekly net is classified `zero`/`return`/`known` by sign. Weeks without rows stay `missing`; nothing is zero-filled.

Blocking codes (no revision is created; the job fails): `unsupported_format`, `invalid_encoding`, `empty_file`, `header_row_not_found`, `sheet_not_found`, `unreadable_xlsx`, `missing_key_column`, `missing_date_column`, `missing_quantity_column`, `invalid_config`, `empty_import`.
Warning and info codes (rows are excluded or flagged; stored as `import_issue` rows capped at 50 per code plus a summary): `parse_error`, `empty_key_row`, `key_separator_in_part`, `invalid_date_row`, `invalid_number_row`, `duplicate_rows_summed` (info), `encoding_assumed` (preview only).

Idempotency: preprocessed revisions carry `contract_version = 'preprocess/1'` and the descriptor digest participates in the mapping fingerprint (ADR-0020). The same bytes with the same descriptor resolve to the existing revision; any changed parsing field creates a new immutable revision. The untouched source bytes are retained as `sources/<revision_id>/source.<csv|txt|xlsx>`; the `descriptor_json` snapshot on the revision stays frozen even if the saved configuration is edited or deleted (ADR-0021).

## Canonical CSV

UTF-8, comma-separated, with exactly the header `series_key,week_start,quantity`.
Keys are nonempty, exact, case-sensitive identities. Dates are ISO Monday dates.
Quantities are non-null, finite floating-point numbers. Zero is explicit zero;
negative values are returns, positive values are known demand.
Identical duplicate cells collapse; conflicting cells are excluded with an import issue.
Empty files and invalid rows fail validation before any revision is committed.
The default location is technical, not a guess about planner source data.
The run cutoff is the maximum imported week, not the host's current date.

Retained source hashes use original file bytes. Mapping fingerprint ordering and
encoding are deterministic, and include auto-created exact mappings and, for
preprocessed imports, the descriptor digest. The committed idempotency key is
profile, content hash, effective mapping fingerprint, and the contract version
(`canonical/1` for direct canonical imports, `preprocess/1` for the
preprocessing layer). See ADR-0013, ADR-0015, ADR-0019, and ADR-0020.
