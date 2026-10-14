# Tauri IPC Contract

The Rust `tauri-specta` command collection generates `src/lib/bindings.ts`.
Generated bindings, not hand-maintained frontend interfaces, are the source of truth.
The UI never opens SQLite or runs the Python engine directly. This document covers the command surface implemented through Milestone 2.

## Commands

- `workspace_create(path: string | null)`: create an empty workspace. A null path uses the application data directory.
- `workspace_open(path: string)`: open an existing workspace and reconcile retained files.
- `workspace_info()`: return the active workspace, latest revision, and recovery findings.
- `import_start(path: string)`: queue a canonical CSV import and return its initial `JobInfo` including the job id.
- `forecast_start(revision_id: number)`: queue a Naive run pinned to an immutable revision and return its initial `JobInfo`.
- `job_status(job_id: string)`: return progress, terminal error, and import/forecast result.
- `job_cancel(job_id: string)`: request cancellation; never cancel a committed transaction retroactively.
- `jobs_list()`: return recent jobs, including terminal jobs recovered from SQLite.
- `series_list(revision_id: number, search: string, offset: number, limit: number)`: return revision-scoped series. Limit is 1..200; offset is nonnegative.
- `series_detail(revision_id: number, product_id: string, location_id: string, run_id: string | null)`: return history, persisted baseline forecasts, planning points, and override capability. Run and revision must match.
- `runs_list(revision_id: number | null)`: return persisted runs for a revision; `null` returns run history across all revisions.
- `override_append(edit: OverrideEdit)`: append a run-scoped value or clearing revision and return `OverrideRevision`.
- `export_csv(run_id: string, path: string)`: publish the saved full-run plan to a new CSV file and return `ExportResult`.

UUIDs are lowercase hyphenated strings at this boundary and `BLOB(16)` inside SQLite.
Revision ids are JSON numbers; Rust rejects values outside the supported integer range.
Jobs use string ids to correlate Tauri, SQLite, and sidecar messages.

## Results

- `WorkspaceInfo`: `path`, `schema_version`, `latest_revision_id`, `recovery_issues`.
- `ImportResult`: `revision_id`, `is_duplicate`, `series_count`, `observation_count`, `issue_count`, `last_week`.
- `SeriesSummary`: `product_id`, `location_id`, `location_code`, `series_key`, `h_length`, `qualification_status`.
- `HistoryPoint`: `week_start`, nullable `quantity`, `status`.
- `ForecastPoint`: `week_start`, finite `value`.
- `SeriesDetail`: `product_id`, `location_id`, `location_code`, `series_key`, `history`, `forecast`, `planning`, `can_override`, and nullable `run_id`, `outcome`, `reason`, `horizon_start_week`, `selected_model_id`, and `selection_score`.
- `PlanningPoint`: `week_start`, 1-based `horizon`, nullable `baseline`, `override_value`, and `effective`, `value_source` (`model | override | missing`), and `override_revision` (0 only when no revision exists).
- `RunSummary`: `id`, `revision_id`, `status`, `created_at`, `horizon_start_week`, `engine_version`.
- `OverrideRevision`: the newly appended `revision` number.
- `ExportResult`: `export_id` (string form of the database identifier), selected `path`, and `row_count`.
- `ForecastResult`: `run_id`, `revision_id`, `status`, `series_count`, `forecast_count`, `coverage`.
- `JobInfo`: `id`, `kind`, `status`, `phase`, `progress` (fraction 0..1), nullable `error`, `import_result`, and `forecast_result`.
- `AppError`: stable `code` and user-readable `message`; filesystem paths and raw process stderr are not exposed as error details.

Commands return a typed Result. Tauri event `job-progress` carries `JobInfo`.
Events are advisory; `job_status`, `jobs_list`, and `runs_list` restore authoritative state.
Import parsing and forecasting run outside the UI thread. One import and one forecast may run concurrently; writes serialize through the Rust-owned connection.
Workspace switching is rejected while an import or forecast job is active. The UI also blocks switching while edits are pending or CSV export is in progress.

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

Rust trims and parses `value` to a finite number. Blank strings, NaN, and infinity are invalid; they never become an accidental clear. Validation includes completed run status, series scope, and horizon membership. Errors include `unknown_run`, `run_not_completed`, `invalid_week`, `invalid_value`, `outside_horizon`, `unknown_series`, `override_conflict`, and `no_override`.

Every accepted write appends the next revision, including a clear. Baseline forecasts remain immutable, and overrides never carry into another run. A stale `expected_revision` fails with `override_conflict` rather than replacing a newer edit. `can_override` is true only for a completed run and a series in its scope; a model baseline is not required, so manual-only values are permitted (ADR-0014).

The effective forecast uses the latest override revision's value when non-null, otherwise the baseline; it never searches backward past a clear. If neither value exists it is missing, not zero. A clear retains its revision number for the next concurrency check. The UI preserves invalid and stale drafts, requires explicit refresh/rebase for conflicts, and pins the edited run when background imports or forecasts complete.

## Full-Run CSV Export

Export reads the saved plan for every series in the selected completed run and all 12 horizon weeks from one read-only WAL snapshot, using bounded keyset batches. Unsaved UI drafts are not exported. Columns are defined in `../DATA_ARCHITECTURE.md`; missing values are blank. Files use UTF-8 with a BOM, CRLF records, and a `.` decimal separator.

The selected path must be absolute with a case-insensitive `.csv` extension, have an existing parent directory, and not already exist. Destinations inside the workspace are rejected except directly in its `exports/` directory. Validation errors include `invalid_export_path`, `export_exists`, `unknown_run`, and `run_not_completed`; CSV write failures can report `export_failed` or a common I/O error.

Rust writes and syncs temporary files before no-overwrite publication. A successful export includes the selected CSV, a retained workspace copy at `exports/<uuid>.csv`, an `export_record`, and a completed export job. The command returns only after publication and the database transaction succeed; CSV export cancellation is not implemented.

A hard kill between publication and database commit can leave a complete orphan CSV/archive or hidden temporary file without an `export_record`. Startup reconciliation does not reclaim these export artifacts yet (risk R-22). Native save-dialog interaction and opening the output in Excel remain unverified; browser checks used a simulated dialog with real Rust workspace methods.

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
encoding are deterministic, and include auto-created exact mappings.
The committed idempotency key is profile, content hash, effective mapping fingerprint,
and `canonical/1`. See ADR-0013 and ADR-0015.
