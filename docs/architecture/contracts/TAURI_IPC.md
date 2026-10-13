# Milestone 1 Tauri IPC

The Rust `tauri-specta` command collection generates `src/lib/bindings.ts`.
Generated bindings, not hand-maintained frontend interfaces, are the source of truth.
The UI never opens SQLite or runs the Python engine directly.

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
- `series_detail(revision_id: number, product_id: string, location_id: string, run_id: string | null)`: return history and persisted forecast points. Run and revision must match.
- `runs_list(revision_id: number)`: return recent persisted runs for restoring the UI after reload.

UUIDs are lowercase hyphenated strings at this boundary and `BLOB(16)` inside SQLite.
Revision ids are JSON numbers; Rust rejects values outside the supported integer range.
Jobs use string ids to correlate Tauri, SQLite, and sidecar messages.

## Results

- `WorkspaceInfo`: `path`, `schema_version`, `latest_revision_id`, `recovery_issues`.
- `ImportResult`: `revision_id`, `is_duplicate`, `series_count`, `observation_count`, `issue_count`, `last_week`.
- `SeriesSummary`: `product_id`, `location_id`, `series_key`, `h_length`, `qualification_status`.
- `HistoryPoint`: `week_start`, nullable `quantity`, `status`.
- `ForecastPoint`: `week_start`, finite `value`.
- `SeriesDetail`: identifiers, `series_key`, `history`, `forecast`, nullable `run_id`, `outcome`, `reason`, and `horizon_start_week`.
- `ForecastResult`: `run_id`, `revision_id`, `status`, `series_count`, `forecast_count`, `coverage`.
- `JobInfo`: `id`, `kind`, `status`, `phase`, `progress` (fraction 0..1), nullable `error`, `import_result`, and `forecast_result`.
- `AppError`: stable `code` and user-readable `message`; filesystem paths and raw process stderr are not exposed as error details.

Commands return a typed Result. Tauri event `job-progress` carries `JobInfo`.
Events are advisory; `job_status`, `jobs_list`, and `runs_list` restore authoritative state.
Import parsing and forecasting run outside the UI thread. One import and one forecast may run concurrently; writes serialize through the Rust-owned connection.
Workspace switching is rejected while a job is active.

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
