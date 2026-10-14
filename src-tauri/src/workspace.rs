//! Workspace ownership for OpenDemand.
//!
//! The `Workspace` type owns the workspace directory and is the single point
//! that opens SQLite. It applies the documented connection pragmas
//! (WAL, synchronous=FULL, foreign_keys=ON, busy_timeout=5000), serializes all
//! writer work through one pooled connection plus a `tokio::sync::Mutex`, runs
//! the canonical `canonical/1` CSV import with the file-before-row durability
//! protocol (ADR-0015), reconciles retained files on open, and serves the read
//! queries the UI and the forecasting core consume.

use crate::{
    error::{AppError, AppResult},
    types::{
        ExportResult, ForecastInput, ForecastPoint, HistoryPoint, ImportResult, InputSeries,
        ObservationRecord, OverrideEdit, OverrideRevision, PlanningPoint, RunSummary, SeriesDetail,
        SeriesSummary, WorkspaceInfo,
    },
};
use chrono::{Datelike, Duration as ChronoDuration, NaiveDate, Utc, Weekday};
use sha2::{Digest, Sha256};
use sqlx::{
    sqlite::{
        SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteRow, SqliteSynchronous,
    },
    Connection, Row, SqliteConnection, SqlitePool,
};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    fs::File,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use uuid::Uuid;

/// The frozen initial migration, embedded at compile time from the authoritative
/// schema directory. Migration 001 must never be edited; the checksum recorded by
/// SQLx is what `open` verifies against.
static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../docs/architecture/schema");

const SCHEMA_VERSION: i64 = 1;
const SCHEMA_VERSION_TEXT: &str = "1";
const CONTRACT_VERSION: &str = "canonical/1";
const DEFAULT_PROFILE_NAME: &str = "canonical/1";
const DEFAULT_LOCATION_CODE: &str = "__default__";
const DEFAULT_TIMEZONE: &str = "UTC";
const DEFAULT_LOCALE: &str = "en";
const DB_FILENAME: &str = "workspace.sqlite3";
const SOURCE_FILENAME: &str = "source.csv";
const LIST_LIMIT_MAX: u32 = 200;
/// Upper bound on the number of dense weekly points materialised for a single
/// series, protecting memory against a corrupted or absurd date span.
const MAX_HISTORY_WEEKS: i64 = 5_200;
const MIN_ELIGIBLE_HISTORY: u32 = 12;

// One definition of the effective plan for both the weekly editor and export.
// The latest revision is joined even when NULL: a clear must not revive an
// older non-null override. Scope comes from outcomes, not baseline rows.
const PLANNING_ROWS_SQL: &str = "
    WITH RECURSIVE run AS (
        SELECT id, horizon_start_week, horizon_weeks FROM forecast_run WHERE id = ?
    ), weeks(horizon, week_start) AS (
        SELECT 1, horizon_start_week FROM run
        UNION ALL
        SELECT horizon + 1, date(week_start, '+7 days') FROM weeks, run
        WHERE horizon < run.horizon_weeks
    ), scope AS (
        SELECT s.* FROM forecast_series_status s JOIN run r ON s.run_id = r.id
        WHERE (? IS NULL OR s.product_id = ?) AND (? IS NULL OR s.location_id = ?)
            AND (s.product_id, s.location_id) > (?, ?)
        ORDER BY s.product_id, s.location_id LIMIT ?
    )
    SELECT s.product_id, s.location_id, p.sku AS product_sku, p.name AS product_name,
           l.code AS location_code, w.week_start, w.horizon, f.value AS baseline,
           o.value AS override_value, COALESCE(o.value, f.value) AS effective,
           COALESCE(o.revision, 0) AS override_revision,
           CASE WHEN o.value IS NOT NULL THEN 'override'
                WHEN f.value IS NOT NULL THEN 'model' ELSE 'missing' END AS value_source,
           s.status AS series_status, s.selected_model_id AS model_id
    FROM scope s
    JOIN product p ON p.id = s.product_id JOIN location l ON l.id = s.location_id
    CROSS JOIN weeks w
    LEFT JOIN forecast f ON f.run_id = s.run_id AND f.product_id = s.product_id
        AND f.location_id = s.location_id AND f.week_start = w.week_start
    LEFT JOIN forecast_override o ON o.run_id = s.run_id AND o.product_id = s.product_id
        AND o.location_id = s.location_id AND o.week_start = w.week_start
        AND o.revision = (SELECT MAX(revision) FROM forecast_override
            WHERE run_id = s.run_id AND product_id = s.product_id
                AND location_id = s.location_id AND week_start = w.week_start)
    ORDER BY s.product_id, s.location_id, w.horizon";

/// A workspace directory plus the exclusive database connection it owns.
///
/// The struct is `Clone`: cloning shares the pooled connection and the writer
/// mutex so that every handle serializes through the same SQLite writer.
#[derive(Clone)]
pub struct Workspace {
    pub path: PathBuf,
    pub pool: sqlx::SqlitePool,
    pub writer: Arc<tokio::sync::Mutex<()>>,
    recovery_issues: Arc<std::sync::Mutex<Vec<String>>>,
}

impl Workspace {
    /// Create a brand-new workspace at `path`.
    ///
    /// The directory must not already contain a workspace database; if it exists
    /// it must be empty. Existing data is never overwritten.
    pub async fn create(path: PathBuf) -> AppResult<Self> {
        let database = path.join(DB_FILENAME);
        if database.exists() {
            return Err(err(
                "workspace_exists",
                "A workspace database already exists here; open it instead of creating over it.",
            ));
        }
        if path.exists() {
            if !path.is_dir() {
                return Err(err(
                    "workspace_not_directory",
                    "The workspace path exists and is not a directory.",
                ));
            }
            let mut entries = std::fs::read_dir(&path)?;
            if entries.next().is_some() {
                return Err(err(
                    "workspace_not_empty",
                    "The workspace directory is not empty; choose an empty directory or open the existing workspace.",
                ));
            }
        } else {
            std::fs::create_dir_all(&path)?;
        }
        for directory in ["staging", "sources", "recovery", "jobs", "exports"] {
            std::fs::create_dir_all(path.join(directory))?;
        }

        let pool = connect(&database, true).await?;
        MIGRATOR.run(&pool).await.map_err(|error| {
            err(
                "migration_failed",
                format!("The workspace schema could not be applied: {error}"),
            )
        })?;

        let timestamp = now_iso();
        let location_id = Uuid::now_v7();
        let mut transaction = pool.begin().await?;
        sqlx::query(
            "INSERT INTO workspace_settings (id, schema_version, timezone, locale, mcp_enabled, created_at, updated_at)
             VALUES (1, ?, ?, ?, 0, ?, ?)",
        )
        .bind(SCHEMA_VERSION_TEXT)
        .bind(DEFAULT_TIMEZONE)
        .bind(DEFAULT_LOCALE)
        .bind(timestamp.as_str())
        .bind(timestamp.as_str())
        .execute(&mut *transaction)
        .await?;
        sqlx::query("INSERT INTO source_profile (name, is_active, created_at) VALUES (?, 1, ?)")
            .bind(DEFAULT_PROFILE_NAME)
            .bind(timestamp.as_str())
            .execute(&mut *transaction)
            .await?;
        let profile_id: i64 = sqlx::query_scalar("SELECT id FROM source_profile WHERE name = ?")
            .bind(DEFAULT_PROFILE_NAME)
            .fetch_one(&mut *transaction)
            .await?;
        sqlx::query(
            "INSERT INTO location (id, code, name, created_at) VALUES (?, ?, 'Default', ?)",
        )
        .bind(location_id.as_bytes().as_slice())
        .bind(DEFAULT_LOCATION_CODE)
        .bind(timestamp.as_str())
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "INSERT INTO location_code_map (profile_id, source_code, location_id, created_at) VALUES (?, ?, ?, ?)",
        )
        .bind(profile_id)
        .bind(DEFAULT_LOCATION_CODE)
        .bind(location_id.as_bytes().as_slice())
        .bind(timestamp.as_str())
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;

        Ok(Self {
            path,
            pool,
            writer: Arc::new(tokio::sync::Mutex::new(())),
            recovery_issues: Arc::new(std::sync::Mutex::new(Vec::new())),
        })
    }

    /// Open an existing workspace, verifying the schema and reconciling retained
    /// files. A non-empty, newer, or unversioned database is rejected instead of
    /// being migrated in place.
    pub async fn open(path: PathBuf) -> AppResult<Self> {
        let database = path.join(DB_FILENAME);
        if !database.exists() {
            return Err(err(
                "workspace_not_found",
                "No workspace database was found at this path; create it first.",
            ));
        }
        let pool = connect(&database, false).await?;
        verify_schema(&pool).await?;
        mark_interrupted_jobs(&pool).await?;
        let issues = reconcile(&pool, &path).await?;
        Ok(Self {
            path,
            pool,
            writer: Arc::new(tokio::sync::Mutex::new(())),
            recovery_issues: Arc::new(std::sync::Mutex::new(issues)),
        })
    }

    /// Return the workspace path, schema version, latest revision, and recovery
    /// findings detected when the workspace was opened.
    pub async fn info(&self) -> AppResult<WorkspaceInfo> {
        let schema_version: String =
            sqlx::query_scalar("SELECT schema_version FROM workspace_settings WHERE id = 1")
                .fetch_one(&self.pool)
                .await?;
        let latest_revision_id: Option<i32> =
            sqlx::query_scalar("SELECT MAX(id) FROM import_revision")
                .fetch_one(&self.pool)
                .await?;
        let recovery_issues = self
            .recovery_issues
            .lock()
            .map(|issues| issues.clone())
            .unwrap_or_default();
        Ok(WorkspaceInfo {
            path: self.path.display().to_string(),
            schema_version,
            latest_revision_id,
            recovery_issues,
        })
    }

    /// Import a canonical `canonical/1` CSV as an immutable revision.
    ///
    /// The raw bytes are hashed, parsed, deduplicated, mapped to exact identities,
    /// and committed with the file-before-row protocol. Re-importing the same
    /// bytes under the same effective mapping plan is a no-op returning the
    /// existing revision.
    pub async fn import_csv(&self, source: &Path, cancel: &AtomicBool) -> AppResult<ImportResult> {
        if cancel.load(Ordering::SeqCst) {
            return Err(cancelled_import());
        }
        let bytes = std::fs::read(source)?;
        if bytes.is_empty() {
            return Err(err(
                "invalid_header",
                "The import file is empty; expected a header row.",
            ));
        }
        let parsed = parse_canonical(&bytes, cancel)?;
        let content_hash = hex(&Sha256::digest(&bytes));
        let source_filename = source
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| SOURCE_FILENAME.to_string());

        let _guard = self.writer.lock().await;
        if cancel.load(Ordering::SeqCst) {
            return Err(cancelled_import());
        }

        let profile_id: i64 =
            sqlx::query_scalar("SELECT id FROM source_profile WHERE is_active = 1")
                .fetch_optional(&self.pool)
                .await?
                .ok_or_else(|| {
                    err(
                        "no_active_profile",
                        "The workspace has no active source profile.",
                    )
                })?;
        let location_bytes: Vec<u8> = sqlx::query_scalar(
            "SELECT location_id FROM location_code_map WHERE profile_id = ? AND source_code = ?",
        )
        .bind(profile_id)
        .bind(DEFAULT_LOCATION_CODE)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| {
            err(
                "missing_default_location",
                "The workspace is missing its technical default location.",
            )
        })?;
        let location_id = uuid_from_bytes(&location_bytes)?;

        let mapped_rows = sqlx::query(
            "SELECT source_code, product_id FROM product_code_map WHERE profile_id = ?",
        )
        .bind(profile_id)
        .fetch_all(&self.pool)
        .await?;
        let mut mapped: BTreeMap<String, Uuid> = BTreeMap::new();
        for row in &mapped_rows {
            let code: String = row.get("source_code");
            let product: Vec<u8> = row.get("product_id");
            mapped.insert(code, uuid_from_bytes(&product)?);
        }
        let location_rows = sqlx::query(
            "SELECT source_code, location_id FROM location_code_map WHERE profile_id = ?",
        )
        .bind(profile_id)
        .fetch_all(&self.pool)
        .await?;

        // Resolve an exact identity mapping for every distinct series key. New
        // keys create a product and a profile-scoped mapping; no fuzzy matching.
        let mut new_products: Vec<(String, Uuid, bool)> = Vec::new();
        for key in &parsed.series_keys {
            if mapped.contains_key(key) {
                continue;
            }
            let existing: Option<Vec<u8>> =
                sqlx::query_scalar("SELECT id FROM product WHERE sku = ?")
                    .bind(key)
                    .fetch_optional(&self.pool)
                    .await?;
            let (id, create_row) = match existing {
                Some(record) => (uuid_from_bytes(&record)?, false),
                None => (Uuid::now_v7(), true),
            };
            mapped.insert(key.clone(), id);
            new_products.push((key.clone(), id, create_row));
        }

        let fingerprint = mapping_fingerprint(CONTRACT_VERSION, &mapped, &location_rows);

        let existing_revision: Option<i32> = sqlx::query_scalar(
            "SELECT id FROM import_revision WHERE profile_id = ? AND content_hash = ? AND mapping_fingerprint = ? AND contract_version = ?",
        )
        .bind(profile_id)
        .bind(content_hash.as_str())
        .bind(fingerprint.as_str())
        .bind(CONTRACT_VERSION)
        .fetch_optional(&self.pool)
        .await?;
        if let Some(revision_id) = existing_revision {
            return self.existing_import_result(revision_id).await;
        }

        let cutoff = parsed.cutoff;
        let series_count = parsed.series_keys.len() as u32;
        let observation_count = parsed
            .series
            .values()
            .map(|weeks| weeks.len())
            .sum::<usize>() as u32;
        let observation_count = observation_count as i64;
        let issue_count = parsed.issues.len() as i64;
        let row_count = parsed.row_count as i64;

        let staged = stage_file(&self.path, &bytes)?;
        if cancel.load(Ordering::SeqCst) {
            let _ = std::fs::remove_file(&staged);
            return Err(cancelled_import());
        }
        if transaction_cancelled(&staged, cancel) {
            return Err(cancelled_import());
        }

        let timestamp = now_iso();
        let mut transaction = self.pool.begin().await?;
        let inserted = sqlx::query(
            "INSERT INTO import_revision (profile_id, content_hash, mapping_fingerprint, contract_version, source_filename, imported_at, row_count, observation_count, issue_count)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(profile_id)
        .bind(content_hash.as_str())
        .bind(fingerprint.as_str())
        .bind(CONTRACT_VERSION)
        .bind(source_filename.as_str())
        .bind(timestamp.as_str())
        .bind(row_count)
        .bind(observation_count)
        .bind(issue_count)
        .execute(&mut *transaction)
        .await?;
        let revision_id = inserted.last_insert_rowid();
        if revision_id < 1 || revision_id > i32::MAX as i64 {
            return Err(err(
                "revision_overflow",
                "The workspace has more import revisions than this build supports.",
            ));
        }
        let revision_id = revision_id as i32;

        for (key, id, create_row) in &new_products {
            if *create_row {
                sqlx::query("INSERT INTO product (id, sku, name, is_active, created_at) VALUES (?, ?, '', 1, ?)")
                    .bind(id.as_bytes().as_slice())
                    .bind(key)
                    .bind(timestamp.as_str())
                    .execute(&mut *transaction)
                    .await?;
            }
            sqlx::query("INSERT INTO product_code_map (profile_id, source_code, product_id, created_at) VALUES (?, ?, ?, ?)")
                .bind(profile_id)
                .bind(key)
                .bind(id.as_bytes().as_slice())
                .bind(timestamp.as_str())
                .execute(&mut *transaction)
                .await?;
        }

        for (key, weeks) in &parsed.series {
            let product_id = mapped[key];
            for (week, (quantity, status)) in weeks {
                sqlx::query(
                    "INSERT INTO observation (revision_id, product_id, location_id, week_start, quantity, status) VALUES (?, ?, ?, ?, ?, ?)",
                )
                .bind(revision_id)
                .bind(product_id.as_bytes().as_slice())
                .bind(location_id.as_bytes().as_slice())
                .bind(week.format("%Y-%m-%d").to_string())
                .bind(*quantity)
                .bind(*status)
                .execute(&mut *transaction)
                .await?;
            }
        }

        for key in &parsed.series_keys {
            let product_id = mapped[key];
            let (h_length, status, reason): (u32, &str, Option<&str>) = match parsed.series.get(key)
            {
                None => (
                    0,
                    "invalid_data",
                    Some("No usable data was available for this series."),
                ),
                Some(weeks) => {
                    let h = trailing_history(weeks, cutoff);
                    if h >= MIN_ELIGIBLE_HISTORY {
                        (h, "eligible", None)
                    } else {
                        (h, "insufficient_history", None)
                    }
                }
            };
            sqlx::query(
                "INSERT INTO series_qualification (revision_id, product_id, location_id, h_length, status, reason, computed_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(revision_id)
            .bind(product_id.as_bytes().as_slice())
            .bind(location_id.as_bytes().as_slice())
            .bind(h_length as i64)
            .bind(status)
            .bind(reason)
            .bind(timestamp.as_str())
            .execute(&mut *transaction)
            .await?;
        }

        for issue in &parsed.issues {
            sqlx::query(
                "INSERT INTO import_issue (revision_id, severity, code, detail, row_ref) VALUES (?, ?, ?, ?, ?)",
            )
            .bind(revision_id)
            .bind(issue.severity)
            .bind(issue.code)
            .bind(issue.detail.as_str())
            .bind(issue.row_ref.as_deref())
            .execute(&mut *transaction)
            .await?;
        }

        // File-before-row: make the retained copy durable and rename it into place
        // before the `source_file` row and the SQLite commit.
        let sources = self.path.join("sources");
        let revision_directory = sources.join(revision_id.to_string());
        std::fs::create_dir_all(&revision_directory)?;
        let target = revision_directory.join(SOURCE_FILENAME);
        if target.exists() {
            return Err(err(
                "source_conflict",
                "A retained source file already exists for this revision.",
            ));
        }
        std::fs::rename(&staged, &target)?;
        fsync_dir(&revision_directory);
        fsync_dir(&sources);

        let relative_path = format!("sources/{revision_id}/{SOURCE_FILENAME}");
        sqlx::query("INSERT INTO source_file (revision_id, relative_path, sha256, byte_size) VALUES (?, ?, ?, ?)")
            .bind(revision_id)
            .bind(relative_path.as_str())
            .bind(content_hash.as_str())
            .bind(bytes.len() as i64)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        let _ = sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
            .execute(&self.pool)
            .await;

        Ok(ImportResult {
            revision_id,
            is_duplicate: false,
            series_count,
            observation_count: observation_count as u32,
            issue_count: issue_count as u32,
            last_week: cutoff.format("%Y-%m-%d").to_string(),
        })
    }

    async fn existing_import_result(&self, revision_id: i32) -> AppResult<ImportResult> {
        let observation_count: i64 =
            sqlx::query_scalar("SELECT observation_count FROM import_revision WHERE id = ?")
                .bind(revision_id)
                .fetch_one(&self.pool)
                .await?;
        let issue_count: i64 =
            sqlx::query_scalar("SELECT issue_count FROM import_revision WHERE id = ?")
                .bind(revision_id)
                .fetch_one(&self.pool)
                .await?;
        let series_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM series_qualification WHERE revision_id = ?")
                .bind(revision_id)
                .fetch_one(&self.pool)
                .await?;
        let last_week: Option<String> =
            sqlx::query_scalar("SELECT MAX(week_start) FROM observation WHERE revision_id = ?")
                .bind(revision_id)
                .fetch_optional(&self.pool)
                .await?;
        Ok(ImportResult {
            revision_id,
            is_duplicate: true,
            series_count: series_count as u32,
            observation_count: observation_count as u32,
            issue_count: issue_count as u32,
            last_week: last_week.unwrap_or_default(),
        })
    }

    /// List the series qualified in a revision, filtered by an SKU search term.
    pub async fn list_series(
        &self,
        revision_id: i32,
        search: &str,
        offset: u32,
        limit: u32,
    ) -> AppResult<Vec<SeriesSummary>> {
        ensure_revision(&self.pool, revision_id).await?;
        let limit = limit.clamp(1, LIST_LIMIT_MAX) as i64;
        let offset = offset as i64;
        let pattern = format!("%{search}%");
        let rows = sqlx::query(
            "SELECT q.product_id AS product_id, q.location_id AS location_id, l.code AS location_code, p.sku AS sku, q.h_length AS h_length, q.status AS status
             FROM series_qualification q JOIN product p ON p.id = q.product_id
             JOIN location l ON l.id = q.location_id
             WHERE q.revision_id = ? AND p.sku LIKE ?
             ORDER BY p.sku, q.location_id
             LIMIT ? OFFSET ?",
        )
        .bind(revision_id)
        .bind(pattern.as_str())
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;

        let mut series = Vec::with_capacity(rows.len());
        for row in &rows {
            let product: Vec<u8> = row.get("product_id");
            let location: Vec<u8> = row.get("location_id");
            series.push(SeriesSummary {
                product_id: uuid_from_bytes(&product)?.to_string(),
                location_id: uuid_from_bytes(&location)?.to_string(),
                location_code: row.get("location_code"),
                series_key: row.get("sku"),
                h_length: row.get::<i64, _>("h_length") as u32,
                qualification_status: row.get("status"),
            });
        }
        Ok(series)
    }

    /// Return the dense history (gaps as `missing`) and, when a run is supplied,
    /// the persisted forecast for one series in one revision.
    pub async fn series_detail(
        &self,
        revision_id: i32,
        product_id: &str,
        location_id: &str,
        run_id: Option<&str>,
    ) -> AppResult<SeriesDetail> {
        ensure_revision(&self.pool, revision_id).await?;
        let product = parse_uuid(product_id, "product_id")?;
        let location = parse_uuid(location_id, "location_id")?;
        let product_bytes = product.as_bytes().to_vec();
        let location_bytes = location.as_bytes().to_vec();

        let series = sqlx::query(
            "SELECT p.sku, l.code AS location_code FROM series_qualification q
             JOIN product p ON p.id = q.product_id JOIN location l ON l.id = q.location_id
             WHERE q.revision_id = ? AND q.product_id = ? AND q.location_id = ?",
        )
        .bind(revision_id)
        .bind(product_bytes.as_slice())
        .bind(location_bytes.as_slice())
        .fetch_optional(&self.pool)
        .await?;
        let series = series.ok_or_else(|| {
            err(
                "unknown_series",
                "The selected series is not part of this import revision.",
            )
        })?;

        let observations =
            load_observations(&self.pool, revision_id, &product_bytes, &location_bytes).await?;
        let cutoff = revision_cutoff(&self.pool, revision_id).await?;
        let history = build_history(&observations, cutoff)?;

        let mut detail = SeriesDetail {
            product_id: product.to_string(),
            location_id: location.to_string(),
            location_code: series.get("location_code"),
            series_key: series.get("sku"),
            history,
            forecast: Vec::new(),
            planning: Vec::new(),
            can_override: false,
            run_id: None,
            outcome: None,
            reason: None,
            horizon_start_week: None,
            selected_model_id: None,
            selection_score: None,
        };

        if let Some(run) = run_id {
            let run_uuid = parse_uuid(run, "run_id")?;
            let run_bytes = run_uuid.as_bytes().to_vec();
            let run_row = sqlx::query(
                "SELECT revision_id, horizon_start_week, status FROM forecast_run WHERE id = ?",
            )
            .bind(run_bytes.as_slice())
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| {
                err(
                    "unknown_run",
                    "The forecast run does not exist in this workspace.",
                )
            })?;
            let run_revision: i32 = run_row.get("revision_id");
            if run_revision != revision_id {
                return Err(err(
                    "run_revision_mismatch",
                    "The forecast run was produced from a different import revision.",
                ));
            }
            detail.run_id = Some(run_uuid.to_string());
            detail.horizon_start_week = Some(run_row.get("horizon_start_week"));

            let status_row = sqlx::query(
                "SELECT status, selected_model_id, reason FROM forecast_series_status WHERE run_id = ? AND product_id = ? AND location_id = ?",
            )
            .bind(run_bytes.as_slice())
            .bind(product_bytes.as_slice())
            .bind(location_bytes.as_slice())
            .fetch_optional(&self.pool)
            .await?;
            if let Some(row) = status_row {
                detail.can_override = run_row.get::<String, _>("status") == "completed";
                detail.outcome = Some(row.get("status"));
                detail.reason = row.get::<Option<String>, _>("reason");
                detail.selected_model_id = row.get::<Option<String>, _>("selected_model_id");
            }

            let points = sqlx::query(
                "SELECT week_start, value FROM forecast WHERE run_id = ? AND product_id = ? AND location_id = ? ORDER BY week_start",
            )
            .bind(run_bytes.as_slice())
            .bind(product_bytes.as_slice())
            .bind(location_bytes.as_slice())
            .fetch_all(&self.pool)
            .await?;
            detail.forecast = points
                .iter()
                .map(|row| ForecastPoint {
                    week_start: row.get("week_start"),
                    value: row.get("value"),
                })
                .collect();

            detail.planning = {
                let mut connection = self.pool.acquire().await?;
                load_planning_rows(
                    &mut connection,
                    &run_bytes,
                    Some(&product_bytes),
                    Some(&location_bytes),
                    (&[], &[]),
                    1,
                )
                .await?
                .iter()
                .map(planning_point)
                .collect()
            };

            if let Some(model) = detail.selected_model_id.clone() {
                let score = sqlx::query_scalar::<_, Option<f64>>(
                    "SELECT selection_score FROM series_model_evaluation WHERE run_id = ? AND product_id = ? AND location_id = ? AND model_id = ?",
                )
                .bind(run_bytes.as_slice())
                .bind(product_bytes.as_slice())
                .bind(location_bytes.as_slice())
                .bind(model.as_str())
                .fetch_optional(&self.pool)
                .await?
                .flatten();
                detail.selection_score = score;
            }
        }
        Ok(detail)
    }

    /// Append one cell revision, including a NULL clearing revision, after
    /// validating the run scope and the revision the planner actually edited.
    pub async fn append_override(&self, edit: OverrideEdit) -> AppResult<OverrideRevision> {
        let run = parse_uuid(&edit.run_id, "run_id")?;
        let product = parse_uuid(&edit.product_id, "product_id")?;
        let location = parse_uuid(&edit.location_id, "location_id")?;
        let week = NaiveDate::parse_from_str(&edit.week_start, "%Y-%m-%d")
            .ok()
            .filter(|week| week.weekday() == Weekday::Mon && week.to_string() == edit.week_start)
            .ok_or_else(|| err("invalid_week", "Choose a Monday in YYYY-MM-DD format."))?;
        let value = edit
            .value
            .as_deref()
            .map(|text| {
                text.trim()
                    .parse::<f64>()
                    .ok()
                    .filter(|value| value.is_finite())
                    .ok_or_else(|| err("invalid_value", "Enter a finite numeric forecast value."))
            })
            .transpose()?;

        let _writer = self.writer.lock().await;
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let row = sqlx::query(
            "SELECT status, horizon_start_week, horizon_weeks FROM forecast_run WHERE id = ?",
        )
        .bind(run.as_bytes().as_slice())
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or_else(|| {
            err(
                "unknown_run",
                "The forecast run does not exist in this workspace.",
            )
        })?;
        if row.get::<String, _>("status") != "completed" {
            return Err(err(
                "run_not_completed",
                "Only completed forecast runs can be adjusted.",
            ));
        }
        let start =
            NaiveDate::parse_from_str(&row.get::<String, _>("horizon_start_week"), "%Y-%m-%d")
                .map_err(|_| {
                    err(
                        "invalid_horizon",
                        "The run has an invalid planning horizon.",
                    )
                })?;
        let weeks: i64 = row.get("horizon_weeks");
        if week < start || week >= start + ChronoDuration::weeks(weeks) {
            return Err(err(
                "outside_horizon",
                "Choose a week inside this run's planning horizon.",
            ));
        }
        let scoped: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM forecast_series_status
             WHERE run_id = ? AND product_id = ? AND location_id = ?)",
        )
        .bind(run.as_bytes().as_slice())
        .bind(product.as_bytes().as_slice())
        .bind(location.as_bytes().as_slice())
        .fetch_one(&mut *transaction)
        .await?;
        if !scoped {
            return Err(err(
                "unknown_series",
                "The selected series is not part of this forecast run.",
            ));
        }
        let current: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(revision), 0) FROM forecast_override
             WHERE run_id = ? AND product_id = ? AND location_id = ? AND week_start = ?",
        )
        .bind(run.as_bytes().as_slice())
        .bind(product.as_bytes().as_slice())
        .bind(location.as_bytes().as_slice())
        .bind(&edit.week_start)
        .fetch_one(&mut *transaction)
        .await?;
        if current != i64::from(edit.expected_revision) {
            return Err(err(
                "override_conflict",
                "This week changed since it was loaded. Refresh the plan, review your input, and save again.",
            ));
        }
        if current == 0 && value.is_none() {
            return Err(err("no_override", "This week has no override to clear."));
        }
        let revision = edit.expected_revision.checked_add(1).ok_or_else(|| {
            err(
                "override_conflict",
                "The cell revision limit has been reached.",
            )
        })?;
        sqlx::query(
            "INSERT INTO forecast_override
             (run_id, product_id, location_id, week_start, revision, value, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(run.as_bytes().as_slice())
        .bind(product.as_bytes().as_slice())
        .bind(location.as_bytes().as_slice())
        .bind(&edit.week_start)
        .bind(i64::from(revision))
        .bind(value)
        .bind(now_iso())
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(OverrideRevision { revision })
    }

    /// Export a consistent full-run plan in bounded batches. The workspace copy
    /// gives export_record a durable relative path; the selected file is a copy.
    /// Neither publication may overwrite existing data.
    pub async fn export_csv(&self, run_id: &str, target: &Path) -> AppResult<ExportResult> {
        let run = parse_uuid(run_id, "run_id")?;
        if !target.is_absolute()
            || !target
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("csv"))
        {
            return Err(err(
                "invalid_export_path",
                "Choose an absolute path ending in .csv.",
            ));
        }
        let parent = target
            .parent()
            .ok_or_else(|| err("invalid_export_path", "Choose a CSV file destination."))?
            .canonicalize()?;
        let root = self.path.canonicalize()?;
        if parent.starts_with(&root) && parent != root.join("exports") {
            return Err(err(
                "invalid_export_path",
                "Save outside the workspace data folders, or in its exports folder.",
            ));
        }
        if target.symlink_metadata().is_ok() {
            return Err(err("export_exists", "A file already exists here. Choose a new filename; exports never overwrite existing files."));
        }

        let job_id = {
            let _writer = self.writer.lock().await;
            sqlx::query("INSERT INTO job (kind, status, created_at, started_at) VALUES ('export', 'running', ?, ?)")
                .bind(now_iso()).bind(now_iso()).execute(&self.pool).await?.last_insert_rowid()
        };
        let result = async {
            // A read-only WAL snapshot keeps every batch consistent without
            // occupying the sole writer connection while a large CSV is built.
            let mut reader = SqliteConnection::connect_with(
                &SqliteConnectOptions::new().filename(self.path.join(DB_FILENAME))
                    .read_only(true).foreign_keys(true).journal_mode(SqliteJournalMode::Wal)
                    .synchronous(SqliteSynchronous::Full).busy_timeout(Duration::from_millis(5_000)),
            ).await?;
            let mut snapshot = reader.begin().await?;
            let status: Option<String> = sqlx::query_scalar("SELECT status FROM forecast_run WHERE id = ?")
                .bind(run.as_bytes().as_slice()).fetch_optional(&mut *snapshot).await?;
            match status.as_deref() {
                None => return Err(err("unknown_run", "The forecast run does not exist in this workspace.")),
                Some("completed") => {},
                _ => return Err(err("run_not_completed", "Only completed forecast runs can be exported.")),
            }
            let outcomes: BTreeMap<String, i64> = sqlx::query_as::<_, (String, i64)>(
                "SELECT status, COUNT(*) FROM forecast_series_status WHERE run_id = ? GROUP BY status",
            ).bind(run.as_bytes().as_slice()).fetch_all(&mut *snapshot).await?.into_iter().collect();
            let series_count: i64 = outcomes.values().sum();
            let covered_series = outcomes.get("ok").copied().unwrap_or(0)
                + outcomes.get("fallback_naive").copied().unwrap_or(0);
            let coverage = serde_json::json!({
                "series_count": series_count, "covered_series": covered_series,
                "coverage": covered_series as f64 / series_count.max(1) as f64, "outcomes": outcomes,
            }).to_string();

            let mut temporary = tempfile::Builder::new().prefix(".opendemand-export-").tempfile_in(&parent)?;
            // UTF-8 BOM and CRLF make names and records portable to desktop Excel.
            temporary.write_all(b"\xef\xbb\xbf")?;
            let mut row_count = 0i64;
            {
                let mut writer = csv::WriterBuilder::new()
                    .terminator(csv::Terminator::CRLF).from_writer(&mut temporary);
                writer.write_record([
                    "run_id", "product_sku", "product_name", "location_code", "week_start", "horizon",
                    "baseline", "override", "effective", "value_source", "series_status", "model_id",
                ]).map_err(|_| err("export_failed", "Could not write the CSV header."))?;
                let mut after = (Vec::<u8>::new(), Vec::<u8>::new());
                loop {
                    // Page the run inventory by its primary key before expanding
                    // each series into weeks, never re-scan a growing OFFSET.
                    let rows = load_planning_rows(&mut snapshot, run.as_bytes(), None, None, (&after.0, &after.1), 32).await?;
                    let Some(last) = rows.last() else { break; };
                    after = (last.get("product_id"), last.get("location_id"));
                    for row in &rows {
                        let point = planning_point(row);
                        writer.write_record([
                            run.to_string(), row.get::<String, _>("product_sku"), row.get("product_name"),
                            row.get("location_code"), point.week_start, point.horizon.to_string(),
                            point.baseline.map(|value| value.to_string()).unwrap_or_default(),
                            point.override_value.map(|value| value.to_string()).unwrap_or_default(),
                            point.effective.map(|value| value.to_string()).unwrap_or_default(),
                            point.value_source, row.get("series_status"), row.get::<Option<String>, _>("model_id").unwrap_or_default(),
                        ]).map_err(|_| err("export_failed", "Could not write the CSV rows. Check the destination and retry."))?;
                    }
                    row_count += rows.len() as i64;
                }
                writer.flush()?;
            }
            snapshot.commit().await?;
            reader.close().await?;
            temporary.as_file().sync_all()?;

            let exports = self.path.join("exports");
            std::fs::create_dir_all(&exports)?;
            let relative_path = format!("exports/{}.csv", Uuid::now_v7());
            let archive_path = self.path.join(&relative_path);
            let archive = tempfile::Builder::new().prefix(".opendemand-export-").tempfile_in(&exports)?;
            std::fs::copy(temporary.path(), archive.path())?;
            archive.as_file().sync_all()?;
            let _writer = self.writer.lock().await;
            let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
            let export_id = sqlx::query(
                "INSERT INTO export_record (run_id, format, relative_path, created_at, row_count, coverage_json)
                 VALUES (?, 'csv', ?, ?, ?, ?)",
            ).bind(run.as_bytes().as_slice()).bind(&relative_path).bind(now_iso()).bind(row_count).bind(coverage)
                .execute(&mut *transaction).await?.last_insert_rowid();
            sqlx::query("UPDATE job SET status = 'completed', finished_at = ? WHERE id = ?")
                .bind(now_iso()).bind(job_id).execute(&mut *transaction).await?;

            archive.persist_noclobber(&archive_path).map_err(|error| AppError::from(error.error))?;
            if let Err(error) = temporary.persist_noclobber(target) {
                let _ = std::fs::remove_file(&archive_path);
                return Err(if error.error.kind() == std::io::ErrorKind::AlreadyExists {
                    err("export_exists", "A file appeared at this destination. Choose a new filename.")
                } else { AppError::from(error.error) });
            }
            fsync_dir(&exports);
            fsync_dir(&parent);
            if let Err(error) = transaction.commit().await {
                let _ = std::fs::remove_file(target);
                let _ = std::fs::remove_file(&archive_path);
                return Err(AppError::from(error));
            }
            Ok(ExportResult { export_id: export_id.to_string(), path: target.display().to_string(), row_count: row_count as u32 })
        }.await;
        if let Err(error) = &result {
            let _writer = self.writer.lock().await;
            if let Err(database_error) = sqlx::query(
                "UPDATE job SET status = 'failed', finished_at = ?, error_code = ?, error_detail = ? WHERE id = ?",
            ).bind(now_iso()).bind(&error.code).bind(&error.message).bind(job_id).execute(&self.pool).await {
                eprintln!("Could not record the failed export job: {database_error}");
            }
        }
        result
    }

    /// List recent persisted forecast runs for a revision.
    pub async fn runs_list(&self, revision_id: i32) -> AppResult<Vec<RunSummary>> {
        ensure_revision(&self.pool, revision_id).await?;
        self.query_runs(Some(revision_id)).await
    }

    pub async fn runs_list_all(&self) -> AppResult<Vec<RunSummary>> {
        self.query_runs(None).await
    }

    async fn query_runs(&self, revision_id: Option<i32>) -> AppResult<Vec<RunSummary>> {
        let rows = sqlx::query(
            "SELECT id, revision_id, status, created_at, horizon_start_week, engine_version
             FROM forecast_run WHERE (? IS NULL OR revision_id = ?) ORDER BY created_at DESC, id DESC",
        )
        .bind(revision_id)
        .bind(revision_id)
        .fetch_all(&self.pool)
        .await?;
        let mut runs = Vec::with_capacity(rows.len());
        for row in &rows {
            let id: Vec<u8> = row.get("id");
            runs.push(RunSummary {
                id: uuid_from_bytes(&id)?.to_string(),
                revision_id: row.get("revision_id"),
                status: row.get("status"),
                created_at: row.get("created_at"),
                horizon_start_week: row.get("horizon_start_week"),
                engine_version: row.get("engine_version"),
            });
        }
        Ok(runs)
    }

    /// Build the sidecar-facing forecast input for a revision: the global cutoff,
    /// the workspace timezone, and every series with its dense weekly history.
    pub async fn forecast_input(&self, revision_id: i32) -> AppResult<ForecastInput> {
        ensure_revision(&self.pool, revision_id).await?;
        let cutoff = revision_cutoff(&self.pool, revision_id).await?;
        let timezone: String =
            sqlx::query_scalar("SELECT timezone FROM workspace_settings WHERE id = 1")
                .fetch_one(&self.pool)
                .await?;
        let rows = sqlx::query(
            "SELECT q.product_id AS product_id, q.location_id AS location_id, p.sku AS sku, q.h_length AS h_length, q.status AS status, q.reason AS reason
             FROM series_qualification q JOIN product p ON p.id = q.product_id
             WHERE q.revision_id = ? ORDER BY p.sku, q.location_id",
        )
        .bind(revision_id)
        .fetch_all(&self.pool)
        .await?;

        let mut series = Vec::with_capacity(rows.len());
        for row in &rows {
            let product: Vec<u8> = row.get("product_id");
            let location: Vec<u8> = row.get("location_id");
            let product_id = uuid_from_bytes(&product)?;
            let location_id = uuid_from_bytes(&location)?;
            let observations =
                load_observations(&self.pool, revision_id, &product, &location).await?;
            let dense = build_observations(&observations, cutoff)?;
            series.push(InputSeries {
                product_id,
                location_id,
                series_key: row.get("sku"),
                h_length: row.get::<i64, _>("h_length") as u32,
                qualification_status: row.get("status"),
                reason: row.get::<Option<String>, _>("reason"),
                observations: dense,
            });
        }
        Ok(ForecastInput {
            revision_id,
            cutoff,
            timezone,
            series,
        })
    }
}

// ---------------------------------------------------------------------------
// Connection and schema verification
// ---------------------------------------------------------------------------

async fn load_planning_rows(
    connection: &mut SqliteConnection,
    run: &[u8],
    product: Option<&[u8]>,
    location: Option<&[u8]>,
    after: (&[u8], &[u8]),
    limit: i64,
) -> AppResult<Vec<SqliteRow>> {
    Ok(sqlx::query(PLANNING_ROWS_SQL)
        .bind(run)
        .bind(product)
        .bind(product)
        .bind(location)
        .bind(location)
        .bind(after.0)
        .bind(after.1)
        .bind(limit)
        .fetch_all(connection)
        .await?)
}

fn planning_point(row: &SqliteRow) -> PlanningPoint {
    PlanningPoint {
        week_start: row.get("week_start"),
        horizon: row.get::<i64, _>("horizon") as u32,
        baseline: row.get("baseline"),
        override_value: row.get("override_value"),
        effective: row.get("effective"),
        value_source: row.get("value_source"),
        override_revision: row.get::<i64, _>("override_revision") as u32,
    }
}

async fn connect(database: &Path, create: bool) -> AppResult<SqlitePool> {
    let options = SqliteConnectOptions::new()
        .filename(database)
        .create_if_missing(create)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Full)
        .foreign_keys(true)
        .busy_timeout(Duration::from_millis(5_000));
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_millis(5_000))
        .connect_with(options)
        .await?;
    Ok(pool)
}

async fn verify_schema(pool: &SqlitePool) -> AppResult<()> {
    let version: Option<String> =
        sqlx::query_scalar("SELECT schema_version FROM workspace_settings WHERE id = 1")
            .fetch_optional(pool)
            .await
            .map_err(|_| {
                err(
                    "unsupported_schema",
                    "This database is not a recognised OpenDemand workspace (unversioned). Restore from a verified backup before opening.",
                )
            })?;
    let version = version.ok_or_else(|| {
        err(
            "unsupported_schema",
            "This workspace has no settings row; it is not a versioned OpenDemand database.",
        )
    })?;
    let parsed: i64 = version.parse().map_err(|_| {
        err(
            "unsupported_schema",
            format!("Unrecognised workspace schema version '{version}'."),
        )
    })?;
    if parsed != SCHEMA_VERSION {
        return Err(err(
            "unsupported_schema",
            format!(
                "Workspace schema version {version} is not supported by this build (expected {SCHEMA_VERSION_TEXT}). Update the application or restore a verified backup."
            ),
        ));
    }

    let stored: Option<Vec<u8>> =
        sqlx::query_scalar("SELECT checksum FROM _sqlx_migrations WHERE version = 1")
            .fetch_optional(pool)
            .await
            .map_err(|_| {
                err(
                    "needs_verified_backup",
                    "The workspace migration record is missing or unreadable. Restore from a verified backup before opening.",
                )
            })?;
    let stored = stored.ok_or_else(|| {
        err(
            "needs_verified_backup",
            "The workspace is missing its migration 001 record. Restore from a verified backup before opening.",
        )
    })?;
    let expected = MIGRATOR
        .iter()
        .find(|migration| migration.version == 1)
        .map(|migration| migration.checksum.to_vec())
        .ok_or_else(|| err("unsupported_schema", "This build embeds no migration 001."))?;
    if stored != expected {
        return Err(err(
            "needs_verified_backup",
            "The workspace schema fingerprint does not match this application build. Restore from a verified backup before opening.",
        ));
    }
    Ok(())
}

async fn mark_interrupted_jobs(pool: &SqlitePool) -> AppResult<()> {
    sqlx::query(
        "UPDATE job SET status = 'failed', finished_at = ?, error_code = 'interrupted', error_detail = ?
         WHERE status IN ('queued', 'running')",
    )
    .bind(now_iso())
    .bind("The job was interrupted when the application stopped. Nothing partial was committed.")
    .execute(pool)
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Startup reconciliation
// ---------------------------------------------------------------------------

async fn reconcile(pool: &SqlitePool, root: &Path) -> AppResult<Vec<String>> {
    let mut issues: Vec<String> = Vec::new();
    let staging = root.join("staging");
    let sources = root.join("sources");
    let recovery = root.join("recovery");
    std::fs::create_dir_all(&recovery)?;
    std::fs::create_dir_all(&staging)?;
    std::fs::create_dir_all(&sources)?;
    std::fs::create_dir_all(root.join("exports"))?;

    let rows = sqlx::query("SELECT relative_path, sha256 FROM source_file")
        .fetch_all(pool)
        .await?;
    let mut committed: HashMap<String, String> = HashMap::new();
    for row in &rows {
        committed.insert(row.get("relative_path"), row.get("sha256"));
    }

    if let Ok(entries) = std::fs::read_dir(&staging) {
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if !(kind.is_file() || kind.is_symlink()) {
                continue;
            }
            let path = entry.path();
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| "staging-entry".to_string());
            issues.push(format!("stale_staging_file: staging/{name}"));
            let _ = move_to_recovery(&path, &recovery);
        }
    }

    let mut seen: HashSet<String> = HashSet::new();
    for path in walk_files(&sources) {
        let Ok(relative) = path.strip_prefix(root) else {
            continue;
        };
        let relative = relative.to_string_lossy().replace('\\', "/");
        seen.insert(relative.clone());
        match committed.get(&relative) {
            None => {
                issues.push(format!("orphan_final_file: {relative}"));
                let _ = move_to_recovery(&path, &recovery);
            }
            Some(expected) => match std::fs::read(&path) {
                Ok(bytes) => {
                    if &hex(&Sha256::digest(&bytes)) != expected {
                        issues.push(format!("hash_mismatch: {relative}"));
                    }
                }
                Err(_) => issues.push(format!("missing_committed_file: {relative}")),
            },
        }
    }
    for relative in committed.keys() {
        if !seen.contains(relative) {
            issues.push(format!("missing_committed_file: {relative}"));
        }
    }
    Ok(issues)
}

fn walk_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((directory, depth)) = stack.pop() {
        if depth > 12 {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_symlink() {
                continue;
            }
            let path = entry.path();
            if kind.is_dir() {
                stack.push((path, depth + 1));
            } else if kind.is_file() {
                files.push(path);
            }
        }
    }
    files
}

fn move_to_recovery(source: &Path, recovery: &Path) -> std::io::Result<()> {
    let name = source
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "recovered".to_string());
    let target = recovery.join(format!("{}-{name}", Uuid::now_v7()));
    std::fs::rename(source, target)
}

// ---------------------------------------------------------------------------
// Canonical CSV parsing
// ---------------------------------------------------------------------------

struct ParsedImport {
    row_count: usize,
    cutoff: NaiveDate,
    series_keys: BTreeSet<String>,
    /// Included, deduplicated numeric cells keyed by series then week.
    series: BTreeMap<String, BTreeMap<NaiveDate, (f64, &'static str)>>,
    issues: Vec<ParsedIssue>,
}

struct ParsedIssue {
    severity: &'static str,
    code: &'static str,
    detail: String,
    row_ref: Option<String>,
}

fn parse_canonical(bytes: &[u8], cancel: &AtomicBool) -> AppResult<ParsedImport> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| err("invalid_encoding", "The import file must be UTF-8 encoded."))?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);

    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(false)
        .from_reader(text.as_bytes());
    let mut records = reader.records();

    let header = records
        .next()
        .ok_or_else(|| {
            err(
                "invalid_header",
                "The import file is empty; expected a header row.",
            )
        })?
        .map_err(|_| {
            err(
                "invalid_header",
                "The import file header could not be parsed.",
            )
        })?;
    let fields: Vec<&str> = header.iter().collect();
    if fields != ["series_key", "week_start", "quantity"] {
        return Err(err(
            "invalid_header",
            "Expected the exact header 'series_key,week_start,quantity'.",
        ));
    }

    let mut cells: Vec<(String, NaiveDate, f64)> = Vec::new();
    for (index, record) in records.enumerate() {
        if cancel.load(Ordering::SeqCst) {
            return Err(cancelled_import());
        }
        let line = index + 2;
        let record = record.map_err(|_| {
            err(
                "parse_error",
                format!("Row {line} could not be parsed as CSV."),
            )
        })?;
        if record.len() != 3 {
            return Err(err(
                "invalid_row",
                format!("Row {line} must have exactly three columns."),
            ));
        }
        let series_key = record.get(0).unwrap_or("");
        if series_key.is_empty() {
            return Err(err(
                "invalid_row",
                format!("Row {line} has an empty series_key."),
            ));
        }
        let raw_week = record.get(1).unwrap_or("");
        let week = NaiveDate::parse_from_str(raw_week, "%Y-%m-%d").map_err(|_| {
            err(
                "invalid_date",
                format!("Row {line} has an invalid ISO date."),
            )
        })?;
        if week.weekday() != Weekday::Mon || week.format("%Y-%m-%d").to_string() != raw_week {
            return Err(err(
                "invalid_date",
                format!("Row {line} must be the Monday of its week in YYYY-MM-DD form."),
            ));
        }
        let quantity: f64 = record.get(2).unwrap_or("").parse().map_err(|_| {
            err(
                "invalid_quantity",
                format!("Row {line} has a non-numeric quantity."),
            )
        })?;
        if !quantity.is_finite() {
            return Err(err(
                "invalid_quantity",
                format!("Row {line} has a non-finite quantity."),
            ));
        }
        cells.push((series_key.to_string(), week, quantity));
    }

    if cells.is_empty() {
        return Err(err(
            "empty_import",
            "The import file contains no data rows.",
        ));
    }

    let row_count = cells.len();
    let mut cutoff = cells[0].1;
    let mut series_keys: BTreeSet<String> = BTreeSet::new();
    let mut grouped: BTreeMap<(String, NaiveDate), Vec<f64>> = BTreeMap::new();
    for (key, week, quantity) in &cells {
        series_keys.insert(key.clone());
        if *week > cutoff {
            cutoff = *week;
        }
        grouped
            .entry((key.clone(), *week))
            .or_default()
            .push(*quantity);
    }
    drop(cells);

    let mut series: BTreeMap<String, BTreeMap<NaiveDate, (f64, &'static str)>> = BTreeMap::new();
    let mut issues: Vec<ParsedIssue> = Vec::new();
    for ((key, week), quantities) in grouped {
        let first = quantities[0];
        let identical = quantities.iter().all(|quantity| *quantity == first);
        if !identical {
            issues.push(ParsedIssue {
                severity: "error",
                code: "duplicate_conflict",
                detail: format!(
                    "Conflicting quantities for series {key} at {week}; the cell was excluded."
                ),
                row_ref: Some(format!("{key}|{week}")),
            });
            continue;
        }
        if quantities.len() > 1 {
            issues.push(ParsedIssue {
                severity: "info",
                code: "duplicate_collapsed",
                detail: format!(
                    "Identical duplicate rows for series {key} at {week} were collapsed."
                ),
                row_ref: Some(format!("{key}|{week}")),
            });
        }
        series
            .entry(key)
            .or_default()
            .insert(week, (first, classify(first)));
    }

    Ok(ParsedImport {
        row_count,
        cutoff,
        series_keys,
        series,
        issues,
    })
}

fn classify(quantity: f64) -> &'static str {
    if quantity == 0.0 {
        "zero"
    } else if quantity < 0.0 {
        "return"
    } else {
        "known"
    }
}

fn trailing_history(weeks: &BTreeMap<NaiveDate, (f64, &'static str)>, cutoff: NaiveDate) -> u32 {
    let mut history = 0u32;
    let mut week = cutoff;
    while weeks.contains_key(&week) {
        history += 1;
        week -= ChronoDuration::weeks(1);
    }
    history
}

fn mapping_fingerprint(
    contract: &str,
    products: &BTreeMap<String, Uuid>,
    locations: &[sqlx::sqlite::SqliteRow],
) -> String {
    let mut lines: BTreeSet<String> = BTreeSet::new();
    lines.insert(format!("contract={contract}"));
    for row in locations {
        let code: String = row.get("source_code");
        let location: Vec<u8> = row.get("location_id");
        if let Ok(id) = uuid_from_bytes(&location) {
            lines.insert(format!("location\t{code}\t{id}"));
        }
    }
    for (code, id) in products {
        lines.insert(format!("product\t{code}\t{id}"));
    }
    let joined = lines.into_iter().collect::<Vec<_>>().join("\n");
    hex(&Sha256::digest(joined.as_bytes()))
}

// ---------------------------------------------------------------------------
// Filesystem durability helpers
// ---------------------------------------------------------------------------

fn stage_file(root: &Path, bytes: &[u8]) -> AppResult<PathBuf> {
    let staging = root.join("staging");
    std::fs::create_dir_all(&staging)?;
    let staged = staging.join(format!("{}.part", Uuid::now_v7().simple()));
    let mut file = File::create(&staged)?;
    file.write_all(bytes)?;
    file.flush()?;
    file.sync_all()?;
    drop(file);
    fsync_dir(&staging);
    Ok(staged)
}

fn fsync_dir(path: &Path) {
    if let Ok(directory) = File::open(path) {
        let _ = directory.sync_all();
    }
}

fn transaction_cancelled(staged: &Path, cancel: &AtomicBool) -> bool {
    if cancel.load(Ordering::SeqCst) {
        let _ = std::fs::remove_file(staged);
        true
    } else {
        false
    }
}

// ---------------------------------------------------------------------------
// Query and formatting helpers
// ---------------------------------------------------------------------------

async fn ensure_revision(pool: &SqlitePool, revision_id: i32) -> AppResult<()> {
    let exists: Option<i64> = sqlx::query_scalar("SELECT id FROM import_revision WHERE id = ?")
        .bind(revision_id)
        .fetch_optional(pool)
        .await?;
    if exists.is_none() {
        return Err(err(
            "unknown_revision",
            format!("Import revision {revision_id} does not exist in this workspace."),
        ));
    }
    Ok(())
}

async fn revision_cutoff(pool: &SqlitePool, revision_id: i32) -> AppResult<NaiveDate> {
    let maximum: Option<String> =
        sqlx::query_scalar("SELECT MAX(week_start) FROM observation WHERE revision_id = ?")
            .bind(revision_id)
            .fetch_optional(pool)
            .await?;
    let maximum = maximum.ok_or_else(|| {
        err(
            "empty_revision",
            "The import revision contains no observations.",
        )
    })?;
    NaiveDate::parse_from_str(&maximum, "%Y-%m-%d")
        .map_err(|_| err("invalid_data", "A stored observation date is invalid."))
}

async fn load_observations(
    pool: &SqlitePool,
    revision_id: i32,
    product: &[u8],
    location: &[u8],
) -> AppResult<BTreeMap<NaiveDate, (Option<f64>, String)>> {
    let rows = sqlx::query(
        "SELECT week_start, quantity, status FROM observation WHERE revision_id = ? AND product_id = ? AND location_id = ? ORDER BY week_start",
    )
    .bind(revision_id)
    .bind(product)
    .bind(location)
    .fetch_all(pool)
    .await?;
    let mut points = BTreeMap::new();
    for row in &rows {
        let week: String = row.get("week_start");
        let week = NaiveDate::parse_from_str(&week, "%Y-%m-%d")
            .map_err(|_| err("invalid_data", "A stored observation date is invalid."))?;
        points.insert(
            week,
            (row.get::<Option<f64>, _>("quantity"), row.get("status")),
        );
    }
    Ok(points)
}

fn build_history(
    points: &BTreeMap<NaiveDate, (Option<f64>, String)>,
    cutoff: NaiveDate,
) -> AppResult<Vec<HistoryPoint>> {
    let Some((earliest, _)) = points.iter().next() else {
        return Ok(Vec::new());
    };
    check_span(*earliest, cutoff)?;
    let mut history = Vec::new();
    let mut week = *earliest;
    while week <= cutoff {
        let entry = points.get(&week);
        history.push(HistoryPoint {
            week_start: week.format("%Y-%m-%d").to_string(),
            quantity: entry.and_then(|(quantity, _)| *quantity),
            status: entry
                .map(|(_, status)| status.clone())
                .unwrap_or_else(|| "missing".to_string()),
        });
        week += ChronoDuration::weeks(1);
    }
    Ok(history)
}

fn build_observations(
    points: &BTreeMap<NaiveDate, (Option<f64>, String)>,
    cutoff: NaiveDate,
) -> AppResult<Vec<ObservationRecord>> {
    let Some((earliest, _)) = points.iter().next() else {
        return Ok(Vec::new());
    };
    check_span(*earliest, cutoff)?;
    let mut observations = Vec::new();
    let mut week = *earliest;
    while week <= cutoff {
        match points.get(&week) {
            Some((quantity, status)) => observations.push(ObservationRecord {
                week_start: week,
                quantity: *quantity,
                status: status.clone(),
            }),
            None => observations.push(ObservationRecord {
                week_start: week,
                quantity: None,
                status: "missing".to_string(),
            }),
        }
        week += ChronoDuration::weeks(1);
    }
    Ok(observations)
}

fn check_span(earliest: NaiveDate, cutoff: NaiveDate) -> AppResult<()> {
    let weeks = (cutoff - earliest).num_days() / 7 + 1;
    if weeks > MAX_HISTORY_WEEKS {
        return Err(err(
            "history_span_too_large",
            "The series history spans more weeks than this build supports.",
        ));
    }
    Ok(())
}

fn parse_uuid(value: &str, field: &str) -> AppResult<Uuid> {
    Uuid::parse_str(value).map_err(|_| {
        err(
            "invalid_input",
            format!("{field} must be a lowercase hyphenated UUID string."),
        )
    })
}

fn uuid_from_bytes(bytes: &[u8]) -> AppResult<Uuid> {
    Uuid::from_slice(bytes)
        .map_err(|_| err("invalid_data", "A stored identifier is not a valid UUID."))
}

fn hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push_str(&format!("{byte:02x}"));
    }
    output
}

fn err(code: impl Into<String>, message: impl Into<String>) -> AppError {
    AppError::new(code, message)
}

fn cancelled_import() -> AppError {
    err(
        "import_cancelled",
        "The import was cancelled before any revision was committed.",
    )
}

fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}
