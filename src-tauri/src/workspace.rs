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
    preprocess::{self, descriptor_json, ImportConfig, ParsingDescriptor},
    selection,
    types::{
        ActualsConfirmation, ActualsConfirmationEdit, DemandChangePoint, ExceptionInbox,
        ExportResult, ForecastChangePoint, ForecastInput, ForecastPoint, FvaExclusions, FvaPoint,
        FvaReport, FvaSeriesMeasures, HistoryPoint, ImportResult, InboxItem, InboxPolicy,
        InboxPolicyEdit, InputSeries, ModelOriginRow, ObservationRecord, OverrideEdit,
        OverrideRevision, PlanPublication, PlanningPoint, PublicationEdit, PublicationFva,
        PublicationFvaPoints, PublicationSeriesDetail, ReviewActionEdit, RunException, RunOverview,
        RunSummary, SeriesChangeSummary, SeriesComparisonDetail, SeriesDetail, SeriesFvaRow,
        SeriesModelComparison, SeriesNote, SeriesNoteEdit, SeriesReviewInfo, SeriesSummary,
        WeeklyChangeFilter, WeeklyComparisonContext, WorkspaceInfo,
    },
};
use chrono::{Datelike, Duration as ChronoDuration, NaiveDate, Utc, Weekday};
use rust_xlsxwriter::{Format, Workbook};
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

const SCHEMA_VERSION: i64 = 7;
const SCHEMA_VERSION_TEXT: &str = "7";
const CONTRACT_VERSION: &str = "canonical/1";
/// Contract version of revisions produced by the preprocessing layer.
const PREPROCESS_CONTRACT: &str = "preprocess/1";
const DEFAULT_PROFILE_NAME: &str = "canonical/1";
const DEFAULT_LOCATION_CODE: &str = "__default__";
const DEFAULT_TIMEZONE: &str = "UTC";
const DEFAULT_LOCALE: &str = "en";
pub(crate) const DB_FILENAME: &str = "workspace.sqlite3";
const SOURCE_FILENAME: &str = "source.csv";
/// Directory holding verified pre-migration backups inside the workspace.
const BACKUP_DIRECTORY: &str = "backups";
/// How many pre-migration backup pairs (database + manifest) to keep.
const PRE_MIGRATION_BACKUPS_KEPT: usize = 3;
/// Upper bound on the length of a saved configuration name.
const MAX_CONFIG_NAME_CHARS: usize = 200;
/// Upper bound on the length of one planner note (series note or override note).
const MAX_NOTE_CHARS: usize = 1_000;
/// The override reason vocabulary (ADR-0027). Codes describe planner intent,
/// never a detected cause.
const OVERRIDE_REASONS: [&str; 5] = [
    "promotion",
    "customer_information",
    "supply_constraint",
    "data_quality",
    "other",
];
/// Rule/policy context label recorded with every review event; bump it when the
/// acknowledged evidence changes so stored acknowledgements stop matching.
/// `review/2` adds the persistent-bias evidence token (ADR-0030).
const REVIEW_CONTEXT: &str = "review/2";
/// Fixed exception-inbox rule priority (ADR-0028, ADR-0030); row order follows
/// the first applicable rule index.
const INBOX_RULES: [&str; 8] = [
    "model_failure",
    "invalid_data",
    "insufficient_history",
    "imputed_input",
    "production_fallback",
    "material_change",
    "persistent_bias",
    "review_due",
];
const LIST_LIMIT_MAX: u32 = 200;
const EXPORT_HEADERS: [&str; 12] = [
    "run_id",
    "product_sku",
    "product_name",
    "location_code",
    "week_start",
    "horizon",
    "baseline",
    "override",
    "effective",
    "value_source",
    "series_status",
    "model_id",
];

#[derive(Clone, Copy)]
enum ExportFormat {
    Csv,
    Xlsx,
}

/// What an export publishes: the run's live saved plan or the values pinned by
/// one publication (ADR-0029).
#[derive(Clone, Copy)]
enum PlanSelection {
    Run(Uuid),
    Publication(i32),
}

/// Policy label recorded with every publication; bump it when the pinned
/// semantics change.
const PUBLICATION_CONTEXT: &str = "publication/1";
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
           o.value AS override_value, o.note AS override_note,
           COALESCE(o.value, f.value) AS effective,
           COALESCE(o.revision, 0) AS override_revision,
           m.reason_code AS override_reason, m.review_date AS override_review_date,
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
    LEFT JOIN override_meta m ON m.run_id = o.run_id AND m.product_id = o.product_id
        AND m.location_id = o.location_id AND m.week_start = o.week_start
        AND m.revision = o.revision
    ORDER BY s.product_id, s.location_id, w.horizon";

// One target week of a publication (ADR-0030): published values pinned at
// publication time plus the week's identity, for evaluation and bias evidence.
const PUBLICATION_WEEK_ROWS_SQL: &str = "
    WITH RECURSIVE run AS (
        SELECT id, horizon_start_week, horizon_weeks FROM forecast_run WHERE id = ?
    ), weeks(horizon, week_start) AS (
        SELECT 1, horizon_start_week FROM run
        UNION ALL
        SELECT horizon + 1, date(week_start, '+7 days') FROM weeks, run
        WHERE horizon < run.horizon_weeks
    )
    SELECT s.product_id AS product_id, s.location_id AS location_id,
           s.imputed_weeks AS imputed_weeks,
           p.sku AS series_key, l.code AS location_code,
           f.value AS baseline, o.value AS override_value,
           COALESCE(o.value, f.value) AS effective, w.week_start AS week_start,
           s.status AS series_status
    FROM forecast_series_status s
    JOIN run r ON s.run_id = r.id
    JOIN product p ON p.id = s.product_id
    JOIN location l ON l.id = s.location_id
    CROSS JOIN weeks w
    LEFT JOIN forecast f ON f.run_id = s.run_id AND f.product_id = s.product_id
        AND f.location_id = s.location_id AND f.week_start = w.week_start
    LEFT JOIN publication_override po ON po.publication_id = ?
        AND po.product_id = s.product_id AND po.location_id = s.location_id
        AND po.week_start = w.week_start
    LEFT JOIN forecast_override o ON o.run_id = s.run_id AND o.product_id = s.product_id
        AND o.location_id = s.location_id AND o.week_start = w.week_start
        AND o.revision = po.revision
    WHERE w.week_start = ?
    ORDER BY s.product_id, s.location_id";

// The published plan for a pinned run: identical shape to PLANNING_ROWS_SQL,
// but the override join goes through publication_override so the exact pinned
// revision wins even after later edits. A pinned clearing revision yields the
// baseline as the published effective value.
const PUBLICATION_ROWS_SQL: &str = "
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
           o.value AS override_value, o.note AS override_note,
           COALESCE(o.value, f.value) AS effective,
           COALESCE(o.revision, 0) AS override_revision,
           m.reason_code AS override_reason, m.review_date AS override_review_date,
           CASE WHEN o.value IS NOT NULL THEN 'override'
                WHEN f.value IS NOT NULL THEN 'model' ELSE 'missing' END AS value_source,
           s.status AS series_status, s.selected_model_id AS model_id
    FROM scope s
    JOIN product p ON p.id = s.product_id JOIN location l ON l.id = s.location_id
    CROSS JOIN weeks w
    LEFT JOIN forecast f ON f.run_id = s.run_id AND f.product_id = s.product_id
        AND f.location_id = s.location_id AND f.week_start = w.week_start
    LEFT JOIN publication_override po ON po.publication_id = ?
        AND po.product_id = s.product_id AND po.location_id = s.location_id
        AND po.week_start = w.week_start
    LEFT JOIN forecast_override o ON o.run_id = s.run_id AND o.product_id = s.product_id
        AND o.location_id = s.location_id AND o.week_start = w.week_start
        AND o.revision = po.revision
    LEFT JOIN override_meta m ON m.run_id = o.run_id AND m.product_id = o.product_id
        AND m.location_id = o.location_id AND m.week_start = o.week_start
        AND m.revision = o.revision
    ORDER BY s.product_id, s.location_id, w.horizon";

// Changed series across two adjacent revisions (ADR-0022): demand cells compare
// observation rows, forecast changes compare immutable baselines on common
// calendar weeks plus availability gaps inside the other run's horizon. The
// count queries mirror the detail derivation exactly.
const WEEKLY_CHANGED_SERIES_SQL: &str = "
    WITH scope AS (
        SELECT product_id, location_id,
               MAX(CASE WHEN revision_id = ? THEN 1 ELSE 0 END) AS in_current,
               MAX(CASE WHEN revision_id = ? THEN 1 ELSE 0 END) AS in_previous
        FROM series_qualification
        WHERE revision_id IN (?, ?)
        GROUP BY product_id, location_id
    ),
    demand AS (
        SELECT product_id, location_id, COUNT(*) AS changes FROM (
            SELECT c.product_id, c.location_id
            FROM observation c
            LEFT JOIN observation p
                ON p.revision_id = ? AND p.product_id = c.product_id
                AND p.location_id = c.location_id AND p.week_start = c.week_start
            WHERE c.revision_id = ?
                AND (p.week_start IS NULL OR p.quantity IS NOT c.quantity OR p.status IS NOT c.status)
            UNION ALL
            SELECT p.product_id, p.location_id
            FROM observation p
            LEFT JOIN observation c
                ON c.revision_id = ? AND c.product_id = p.product_id
                AND c.location_id = p.location_id AND c.week_start = p.week_start
            WHERE p.revision_id = ? AND c.week_start IS NULL
        ) GROUP BY product_id, location_id
    ),
    forecast_changes AS (
        SELECT product_id, location_id, COUNT(*) AS changes FROM (
            SELECT c.product_id, c.location_id
            FROM forecast c
            JOIN forecast p
                ON p.run_id = ? AND p.product_id = c.product_id
                AND p.location_id = c.location_id AND p.week_start = c.week_start
            WHERE c.run_id = ? AND c.value IS NOT p.value
            UNION ALL
            SELECT c.product_id, c.location_id
            FROM forecast c
            WHERE c.run_id = ?
                AND c.week_start >= ? AND c.week_start <= ?
                AND NOT EXISTS (
                    SELECT 1 FROM forecast p
                    WHERE p.run_id = ? AND p.product_id = c.product_id
                        AND p.location_id = c.location_id AND p.week_start = c.week_start)
            UNION ALL
            SELECT p.product_id, p.location_id
            FROM forecast p
            WHERE p.run_id = ?
                AND p.week_start >= ? AND p.week_start <= ?
                AND NOT EXISTS (
                    SELECT 1 FROM forecast c
                    WHERE c.run_id = ? AND c.product_id = p.product_id
                        AND c.location_id = p.location_id AND c.week_start = p.week_start)
        ) GROUP BY product_id, location_id
    )
    SELECT s.product_id, s.location_id, l.code AS location_code, p.sku AS sku,
           COALESCE(d.changes, 0) AS demand_changes,
           COALESCE(f.changes, 0) AS forecast_changes,
           s.in_current, s.in_previous
    FROM scope s
    JOIN product p ON p.id = s.product_id
    JOIN location l ON l.id = s.location_id
    LEFT JOIN demand d ON d.product_id = s.product_id AND d.location_id = s.location_id
    LEFT JOIN forecast_changes f ON f.product_id = s.product_id AND f.location_id = s.location_id
    WHERE (COALESCE(d.changes, 0) > 0 OR COALESCE(f.changes, 0) > 0
           OR s.in_current = 0 OR s.in_previous = 0)
      AND p.sku LIKE ?
      AND (? = 'all'
           OR (? = 'demand' AND COALESCE(d.changes, 0) > 0)
           OR (? = 'forecast' AND COALESCE(f.changes, 0) > 0)
           OR (? = 'presence' AND (s.in_current = 0 OR s.in_previous = 0)))
    ORDER BY p.sku, s.location_id
    LIMIT ? OFFSET ?";

/// Resolved, validated comparison pair for the weekly read models.
struct ComparisonPair {
    current_revision: i32,
    previous_revision: i32,
    current_run: Vec<u8>,
    previous_run: Vec<u8>,
    current_start: NaiveDate,
    previous_start: NaiveDate,
    current_end: NaiveDate,
    previous_end: NaiveDate,
}

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
        for directory in [
            "staging", "sources", "recovery", "jobs", "exports", "backups",
        ] {
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
    /// files. A workspace from an older build is migrated only after a verified
    /// pre-migration backup (ADR-0008); a non-empty, newer, or unversioned
    /// database is rejected instead of being migrated in place.
    pub async fn open(path: PathBuf) -> AppResult<Self> {
        let database = path.join(DB_FILENAME);
        if !database.exists() {
            return Err(err(
                "workspace_not_found",
                "No workspace database was found at this path; create it first.",
            ));
        }
        let pool = connect(&database, false).await?;
        if let Some(version) = schema_version(&pool).await? {
            if version < SCHEMA_VERSION {
                upgrade_schema(&path, &pool, version).await?;
            }
        }
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

    /// Return the workspace path, schema version, latest revision, locale, and
    /// recovery findings detected when the workspace was opened.
    pub async fn info(&self) -> AppResult<WorkspaceInfo> {
        let settings =
            sqlx::query("SELECT schema_version, locale FROM workspace_settings WHERE id = 1")
                .fetch_one(&self.pool)
                .await?;
        let schema_version: String = settings.get("schema_version");
        let locale: String = settings.get("locale");
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
            locale,
            recovery_issues,
        })
    }

    pub async fn set_locale(&self, locale: &str) -> AppResult<WorkspaceInfo> {
        if !["en", "pl", "es"].contains(&locale) {
            return Err(err("invalid_locale", "Choose English, Polish, or Spanish."));
        }
        let _writer = self.writer.lock().await;
        let mut transaction = self.pool.begin().await?;
        sqlx::query("UPDATE workspace_settings SET locale = ? WHERE id = 1")
            .bind(locale)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        self.info().await
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
        let source_filename = source_filename(source);
        self.commit_parsed(
            &bytes,
            &content_hash,
            &source_filename,
            CommitSpec {
                contract_version: CONTRACT_VERSION,
                descriptor: None,
                retained_name: SOURCE_FILENAME,
            },
            parsed,
            cancel,
        )
        .await
    }

    /// Import a raw planner export through the preprocessing layer.
    ///
    /// The retained bytes stay untouched; the parsing descriptor is part of the
    /// effective mapping plan, so the same bytes under a different descriptor
    /// create a new immutable revision.
    pub async fn import_raw(
        &self,
        source: &Path,
        descriptor: &ParsingDescriptor,
        cancel: &AtomicBool,
    ) -> AppResult<ImportResult> {
        if cancel.load(Ordering::SeqCst) {
            return Err(cancelled_import());
        }
        let extension = preprocess::retained_extension(source)?;
        let bytes = std::fs::read(source)?;
        if bytes.is_empty() {
            return Err(err(
                "empty_file",
                "The import file is empty; expected a header row.",
            ));
        }
        let parsed = preprocess::parse_raw(&bytes, &extension, descriptor, cancel)?;
        let content_hash = hex(&Sha256::digest(&bytes));
        let source_filename = source_filename(source);
        let retained_name = format!("source.{extension}");
        self.commit_parsed(
            &bytes,
            &content_hash,
            &source_filename,
            CommitSpec {
                contract_version: PREPROCESS_CONTRACT,
                descriptor: Some(descriptor),
                retained_name: &retained_name,
            },
            parsed,
            cancel,
        )
        .await
    }

    /// List the saved import configurations of the active profile by name.
    pub async fn list_import_configs(&self) -> AppResult<Vec<ImportConfig>> {
        let profile_id = active_profile_id(&self.pool).await?;
        let rows = sqlx::query(
            "SELECT id, name, descriptor_json, updated_at FROM import_config
             WHERE profile_id = ? ORDER BY name",
        )
        .bind(profile_id)
        .fetch_all(&self.pool)
        .await?;
        let mut configs = Vec::new();
        for row in &rows {
            configs.push(import_config_from_row(row)?);
        }
        Ok(configs)
    }

    /// Save a named descriptor. Saving an existing name updates that row in
    /// place; revisions already committed keep their frozen snapshot.
    pub async fn save_import_config(
        &self,
        name: &str,
        descriptor: &ParsingDescriptor,
    ) -> AppResult<ImportConfig> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > MAX_CONFIG_NAME_CHARS {
            return Err(err(
                "invalid_config",
                format!("The configuration name must be 1 to {MAX_CONFIG_NAME_CHARS} characters."),
            ));
        }
        let serialized = descriptor_json(descriptor).map_err(|_| {
            err(
                "contract_violation",
                "The parsing descriptor could not be serialized.",
            )
        })?;
        let _guard = self.writer.lock().await;
        let profile_id = active_profile_id(&self.pool).await?;
        let timestamp = now_iso();
        sqlx::query(
            "INSERT INTO import_config (profile_id, name, descriptor_json, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?)
             ON CONFLICT (profile_id, name) DO UPDATE SET
                 descriptor_json = excluded.descriptor_json,
                 updated_at = excluded.updated_at",
        )
        .bind(profile_id)
        .bind(name)
        .bind(serialized.as_str())
        .bind(timestamp.as_str())
        .bind(timestamp.as_str())
        .execute(&self.pool)
        .await?;
        let row = sqlx::query(
            "SELECT id, name, descriptor_json, updated_at FROM import_config
             WHERE profile_id = ? AND name = ?",
        )
        .bind(profile_id)
        .bind(name)
        .fetch_one(&self.pool)
        .await?;
        import_config_from_row(&row)
    }

    /// Delete a saved configuration. Revisions keep their own snapshot.
    pub async fn delete_import_config(&self, id: i32) -> AppResult<()> {
        let _guard = self.writer.lock().await;
        let deleted = sqlx::query("DELETE FROM import_config WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        if deleted.rows_affected() == 0 {
            return Err(err(
                "config_not_found",
                "No saved import configuration has this id.",
            ));
        }
        Ok(())
    }

    /// The id of the single saved configuration whose descriptor exactly
    /// matches the latest preprocessed revision's frozen descriptor; `None`
    /// when there is no match or more than one match (ADR-0022).
    pub async fn import_config_suggestion(&self) -> AppResult<Option<i32>> {
        let profile_id = active_profile_id(&self.pool).await?;
        let frozen: Option<String> = sqlx::query_scalar(
            "SELECT descriptor_json FROM import_revision
             WHERE profile_id = ? AND descriptor_json IS NOT NULL
             ORDER BY id DESC LIMIT 1",
        )
        .bind(profile_id)
        .fetch_optional(&self.pool)
        .await?;
        let Some(frozen) = frozen else {
            return Ok(None);
        };
        let ids: Vec<i64> = sqlx::query_scalar(
            "SELECT id FROM import_config WHERE profile_id = ? AND descriptor_json = ? ORDER BY id",
        )
        .bind(profile_id)
        .bind(frozen.as_str())
        .fetch_all(&self.pool)
        .await?;
        if let [id] = ids.as_slice() {
            if (1..=i32::MAX as i64).contains(id) {
                return Ok(Some(*id as i32));
            }
        }
        Ok(None)
    }

    /// Commit a parsed import: map identities, compute the fingerprint, check
    /// idempotency, stage the retained file, and persist one revision in a
    /// single serialized transaction.
    async fn commit_parsed(
        &self,
        bytes: &[u8],
        content_hash: &str,
        source_filename: &str,
        spec: CommitSpec<'_>,
        parsed: ParsedImport,
        cancel: &AtomicBool,
    ) -> AppResult<ImportResult> {
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

        let fingerprint = mapping_fingerprint(
            spec.contract_version,
            &mapped,
            &location_rows,
            spec.descriptor,
        )?;

        let existing_revision: Option<i32> = sqlx::query_scalar(
            "SELECT id FROM import_revision WHERE profile_id = ? AND content_hash = ? AND mapping_fingerprint = ? AND contract_version = ?",
        )
        .bind(profile_id)
        .bind(content_hash)
        .bind(fingerprint.as_str())
        .bind(spec.contract_version)
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

        let staged = stage_file(&self.path, bytes)?;
        if cancel.load(Ordering::SeqCst) {
            let _ = std::fs::remove_file(&staged);
            return Err(cancelled_import());
        }
        if transaction_cancelled(&staged, cancel) {
            return Err(cancelled_import());
        }

        let timestamp = now_iso();
        let descriptor_snapshot = match spec.descriptor {
            Some(descriptor) => Some(descriptor_json(descriptor).map_err(|_| {
                err(
                    "contract_violation",
                    "The parsing descriptor could not be serialized.",
                )
            })?),
            None => None,
        };
        let mut transaction = self.pool.begin().await?;
        let inserted = sqlx::query(
            "INSERT INTO import_revision (profile_id, content_hash, mapping_fingerprint, contract_version, source_filename, imported_at, row_count, observation_count, issue_count, descriptor_json)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(profile_id)
        .bind(content_hash)
        .bind(fingerprint.as_str())
        .bind(spec.contract_version)
        .bind(source_filename)
        .bind(timestamp.as_str())
        .bind(row_count)
        .bind(observation_count)
        .bind(issue_count)
        .bind(descriptor_snapshot)
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
        let target = revision_directory.join(spec.retained_name);
        if target.exists() {
            return Err(err(
                "source_conflict",
                "A retained source file already exists for this revision.",
            ));
        }
        std::fs::rename(&staged, &target)?;
        fsync_dir(&revision_directory);
        fsync_dir(&sources);

        let relative_path = format!("sources/{revision_id}/{}", spec.retained_name);
        sqlx::query("INSERT INTO source_file (revision_id, relative_path, sha256, byte_size) VALUES (?, ?, ?, ?)")
            .bind(revision_id)
            .bind(relative_path.as_str())
            .bind(content_hash)
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
        let profile_id: i64 =
            sqlx::query_scalar("SELECT profile_id FROM import_revision WHERE id = ?")
                .bind(revision_id)
                .fetch_one(&self.pool)
                .await?;

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
            imputation_method: None,
            imputed_weeks: None,
            notes: load_series_notes(&self.pool, profile_id, &product_bytes, &location_bytes)
                .await?,
            review: None,
        };

        if let Some(run) = run_id {
            let run_uuid = parse_uuid(run, "run_id")?;
            let run_bytes = run_uuid.as_bytes().to_vec();
            let run_row = sqlx::query(
                "SELECT revision_id, horizon_start_week, status, imputation_method FROM forecast_run WHERE id = ?",
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
            detail.imputation_method = run_row.get::<Option<String>, _>("imputation_method");

            let status_row = sqlx::query(
                "SELECT status, selected_model_id, reason, imputed_weeks FROM forecast_series_status WHERE run_id = ? AND product_id = ? AND location_id = ?",
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
                detail.imputed_weeks = Some(row.get::<i64, _>("imputed_weeks") as u32);
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
                    PlanSource::Run(&run_bytes),
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

            let mut connection = self.pool.acquire().await?;
            detail.review = Some(
                review_state(
                    &mut connection,
                    profile_id,
                    &run_bytes,
                    &product_bytes,
                    &location_bytes,
                )
                .await?,
            );
        }
        Ok(detail)
    }

    /// Model comparison evidence for one series in one run: stored evaluation
    /// rows and per-origin metrics plus pooled diagnostics computed from the
    /// retained held-out backtest forecasts against the pinned revision actuals.
    pub async fn series_models(
        &self,
        run_id: &str,
        product_id: &str,
        location_id: &str,
    ) -> AppResult<Vec<SeriesModelComparison>> {
        let run = parse_uuid(run_id, "run_id")?;
        let product = parse_uuid(product_id, "product_id")?;
        let location = parse_uuid(location_id, "location_id")?;
        let run_bytes = run.as_bytes().to_vec();
        let product_bytes = product.as_bytes().to_vec();
        let location_bytes = location.as_bytes().to_vec();

        sqlx::query("SELECT id FROM forecast_run WHERE id = ?")
            .bind(run_bytes.as_slice())
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| {
                err(
                    "unknown_run",
                    "The forecast run does not exist in this workspace.",
                )
            })?;
        let status_row = sqlx::query(
            "SELECT selected_model_id FROM forecast_series_status
             WHERE run_id = ? AND product_id = ? AND location_id = ?",
        )
        .bind(run_bytes.as_slice())
        .bind(product_bytes.as_slice())
        .bind(location_bytes.as_slice())
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| {
            err(
                "unknown_series",
                "The selected series is not part of this forecast run.",
            )
        })?;
        let selected_model: Option<String> = status_row.get("selected_model_id");

        let evaluations = sqlx::query(
            "SELECT model_id, eligibility_status, origins_planned, origins_completed,
                    selection_score, selection_rank, is_champion, production_status,
                    failure_code, failure_detail
             FROM series_model_evaluation
             WHERE run_id = ? AND product_id = ? AND location_id = ?",
        )
        .bind(run_bytes.as_slice())
        .bind(product_bytes.as_slice())
        .bind(location_bytes.as_slice())
        .fetch_all(&self.pool)
        .await?;
        let origin_rows = sqlx::query(
            "SELECT model_id, origin_index, origin_week_start, train_size, status,
                    wape, mae, metric_kind, failure_code
             FROM series_model_origin
             WHERE run_id = ? AND product_id = ? AND location_id = ?
             ORDER BY model_id, origin_index",
        )
        .bind(run_bytes.as_slice())
        .bind(product_bytes.as_slice())
        .bind(location_bytes.as_slice())
        .fetch_all(&self.pool)
        .await?;
        let sum_rows = sqlx::query(
            "SELECT bf.model_id AS model_id, bf.origin_index AS origin_index,
                    SUM(ABS(bf.value - o.quantity)) AS error,
                    SUM(ABS(o.quantity)) AS magnitude,
                    SUM(bf.value - o.quantity) AS signed,
                    COUNT(*) AS points
             FROM backtest_forecast bf
             JOIN forecast_run r ON r.id = bf.run_id
             JOIN observation o ON o.revision_id = r.revision_id
                 AND o.product_id = bf.product_id AND o.location_id = bf.location_id
                 AND o.week_start = bf.week_start
             WHERE bf.run_id = ? AND bf.product_id = ? AND bf.location_id = ?
                 AND o.quantity IS NOT NULL
             GROUP BY bf.model_id, bf.origin_index",
        )
        .bind(run_bytes.as_slice())
        .bind(product_bytes.as_slice())
        .bind(location_bytes.as_slice())
        .fetch_all(&self.pool)
        .await?;

        let mut sums: HashMap<(String, i32), selection::OriginSums> = HashMap::new();
        for row in &sum_rows {
            sums.insert(
                (
                    row.get("model_id"),
                    row.get::<i64, _>("origin_index") as i32,
                ),
                selection::OriginSums {
                    points: row.get::<i64, _>("points") as u32,
                    absolute_error: row.get("error"),
                    actual_magnitude: row.get("magnitude"),
                    signed_error: row.get("signed"),
                },
            );
        }
        let mut origins_by_model: HashMap<String, Vec<ModelOriginRow>> = HashMap::new();
        for row in &origin_rows {
            origins_by_model
                .entry(row.get("model_id"))
                .or_default()
                .push(ModelOriginRow {
                    origin_index: row.get::<i64, _>("origin_index") as u32,
                    origin_week_start: row.get("origin_week_start"),
                    train_size: row.get::<i64, _>("train_size") as u32,
                    status: row.get("status"),
                    wape: row.get("wape"),
                    mae: row.get("mae"),
                    metric_kind: row.get("metric_kind"),
                    failure_code: row.get("failure_code"),
                });
        }

        struct Evaluation {
            model_id: String,
            eligibility_status: String,
            origins_planned: u32,
            origins_completed: u32,
            selection_score: Option<f64>,
            selection_rank: Option<u32>,
            is_champion: bool,
            production_status: String,
            failure_code: Option<String>,
            failure_detail: Option<String>,
        }
        let mut entries: Vec<Evaluation> = evaluations
            .iter()
            .map(|row| Evaluation {
                model_id: row.get("model_id"),
                eligibility_status: row.get("eligibility_status"),
                origins_planned: row.get::<i64, _>("origins_planned") as u32,
                origins_completed: row.get::<i64, _>("origins_completed") as u32,
                selection_score: row.get("selection_score"),
                selection_rank: row
                    .get::<Option<i64>, _>("selection_rank")
                    .map(|v| v as u32),
                is_champion: row.get::<i64, _>("is_champion") == 1,
                production_status: row.get("production_status"),
                failure_code: row.get("failure_code"),
                failure_detail: row.get("failure_detail"),
            })
            .collect();
        entries.sort_by_key(|entry| {
            selection::MODELS
                .iter()
                .position(|model| *model == entry.model_id)
                .unwrap_or(usize::MAX)
        });

        let completed_origins =
            |model_id: &str| -> AppResult<Vec<(NaiveDate, selection::OriginSums)>> {
                let mut completed = Vec::new();
                for origin in origins_by_model.get(model_id).into_iter().flatten() {
                    if origin.status != "completed" {
                        continue;
                    }
                    let Some(value) = sums.get(&(model_id.to_owned(), origin.origin_index as i32))
                    else {
                        continue;
                    };
                    let week = NaiveDate::parse_from_str(&origin.origin_week_start, "%Y-%m-%d")
                        .map_err(|_| err("database_error", "Stored origin week is invalid."))?;
                    completed.push((week, *value));
                }
                Ok(completed)
            };
        let mut evidences = Vec::with_capacity(entries.len());
        for entry in &entries {
            evidences.push(selection::ModelEvidence {
                model_id: entry.model_id.clone(),
                production_eligible: entry.eligibility_status != "not_eligible",
                production_completed: entry.production_status == "completed",
                completed: completed_origins(&entry.model_id)?,
            });
        }
        let ranked = selection::rank(&evidences);

        let mut comparisons = Vec::with_capacity(entries.len());
        for entry in entries {
            let completed = completed_origins(&entry.model_id)?;
            let diagnostics = selection::diagnostics(&completed);
            let comparable_metric = ranked
                .iter()
                .find(|score| score.model_id == entry.model_id)
                .map(|score| score.metric_kind.to_owned());
            comparisons.push(SeriesModelComparison {
                selected: selected_model.as_deref() == Some(entry.model_id.as_str()),
                model_id: entry.model_id.clone(),
                eligibility_status: entry.eligibility_status,
                origins_planned: entry.origins_planned,
                origins_completed: entry.origins_completed,
                selection_score: entry.selection_score,
                selection_rank: entry.selection_rank,
                is_champion: entry.is_champion,
                production_status: entry.production_status,
                failure_code: entry.failure_code,
                failure_detail: entry.failure_detail,
                comparable_metric,
                wape: diagnostics.wape,
                mae: diagnostics.mae,
                bias: diagnostics.bias,
                origins: origins_by_model
                    .get(&entry.model_id)
                    .cloned()
                    .unwrap_or_default(),
            });
        }
        Ok(comparisons)
    }

    /// Append one cell revision, including a NULL clearing revision, after
    /// validating the run scope and the revision the planner actually edited.
    ///
    /// Every new revision carries a required reason code and optional note and
    /// review date; the pairing commits in one transaction (ADR-0027).
    pub async fn append_override(&self, edit: OverrideEdit) -> AppResult<OverrideRevision> {
        let run = parse_uuid(&edit.run_id, "run_id")?;
        let product = parse_uuid(&edit.product_id, "product_id")?;
        let location = parse_uuid(&edit.location_id, "location_id")?;
        let reason_code = validate_override_reason(&edit.reason_code)?;
        let note = validate_optional_note(edit.note.as_deref())?;
        if reason_code == "other" && note.is_none() {
            return Err(err(
                "note_required",
                "A note is required when the reason is 'other'.",
            ));
        }
        let review_date = validate_optional_review_date(edit.review_date.as_deref())?;
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
        let timestamp = now_iso();
        sqlx::query(
            "INSERT INTO forecast_override
             (run_id, product_id, location_id, week_start, revision, value, note, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(run.as_bytes().as_slice())
        .bind(product.as_bytes().as_slice())
        .bind(location.as_bytes().as_slice())
        .bind(&edit.week_start)
        .bind(i64::from(revision))
        .bind(value)
        .bind(note.as_deref())
        .bind(timestamp.as_str())
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "INSERT INTO override_meta
             (run_id, product_id, location_id, week_start, revision, reason_code, review_date, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(run.as_bytes().as_slice())
        .bind(product.as_bytes().as_slice())
        .bind(location.as_bytes().as_slice())
        .bind(&edit.week_start)
        .bind(i64::from(revision))
        .bind(reason_code)
        .bind(review_date.as_deref())
        .bind(timestamp.as_str())
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(OverrideRevision { revision })
    }

    /// Add one append-only planner note for a series (ADR-0027). Notes are
    /// profile-scoped and survive across runs; they never change coverage,
    /// plan values, or stored history.
    pub async fn series_note_add(&self, edit: SeriesNoteEdit) -> AppResult<SeriesNote> {
        let run = parse_uuid(&edit.run_id, "run_id")?;
        let product = parse_uuid(&edit.product_id, "product_id")?;
        let location = parse_uuid(&edit.location_id, "location_id")?;
        let note = validate_note(&edit.note)?;
        let _writer = self.writer.lock().await;
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let profile_id = decision_scope(
            &mut transaction,
            &run,
            product.as_bytes().as_slice(),
            location.as_bytes().as_slice(),
        )
        .await?;
        let timestamp = now_iso();
        let inserted = sqlx::query(
            "INSERT INTO series_note (profile_id, product_id, location_id, note, context_run_id, created_at)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(profile_id)
        .bind(product.as_bytes().as_slice())
        .bind(location.as_bytes().as_slice())
        .bind(note.as_str())
        .bind(run.as_bytes().as_slice())
        .bind(timestamp.as_str())
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        let id = inserted.last_insert_rowid();
        Ok(SeriesNote {
            id: i32::try_from(id)
                .map_err(|_| err("contract_violation", "The note id is unsupported."))?,
            note,
            context_run_id: Some(run.to_string()),
            created_at: timestamp,
        })
    }

    /// Record one append-only review event for a series and run (ADR-0027) and
    /// return the derived review state. `reviewed` acknowledges the run's
    /// current evidence; `reopened` clears the acknowledgement.
    pub async fn series_review_set(&self, edit: ReviewActionEdit) -> AppResult<SeriesReviewInfo> {
        let run = parse_uuid(&edit.run_id, "run_id")?;
        let product = parse_uuid(&edit.product_id, "product_id")?;
        let location = parse_uuid(&edit.location_id, "location_id")?;
        let action = match edit.action.as_str() {
            "reviewed" => "reviewed",
            "reopened" => "reopened",
            _ => return Err(err("invalid_action", "Choose mark reviewed or reopen.")),
        };
        let _writer = self.writer.lock().await;
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let profile_id = decision_scope(
            &mut transaction,
            &run,
            product.as_bytes().as_slice(),
            location.as_bytes().as_slice(),
        )
        .await?;
        let fingerprint = evidence_fingerprint(
            &mut transaction,
            run.as_bytes().as_slice(),
            product.as_bytes().as_slice(),
            location.as_bytes().as_slice(),
        )
        .await?;
        let timestamp = now_iso();
        sqlx::query(
            "INSERT INTO series_review
             (profile_id, product_id, location_id, run_id, action, evidence_context, evidence_fingerprint, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(profile_id)
        .bind(product.as_bytes().as_slice())
        .bind(location.as_bytes().as_slice())
        .bind(run.as_bytes().as_slice())
        .bind(action)
        .bind(REVIEW_CONTEXT)
        .bind(fingerprint.as_str())
        .bind(timestamp.as_str())
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(match action {
            "reviewed" => SeriesReviewInfo {
                status: "reviewed".into(),
                reviewed_at: Some(timestamp),
            },
            _ => SeriesReviewInfo {
                status: "unreviewed".into(),
                reviewed_at: None,
            },
        })
    }

    /// Current profile-scoped exception-inbox policy (ADR-0028).
    pub async fn review_policy(&self) -> AppResult<InboxPolicy> {
        let profile_id = active_profile_id(&self.pool).await?;
        let mut connection = self.pool.acquire().await?;
        review_policy_row(&mut connection, profile_id).await
    }

    /// Append one inbox-policy version (ADR-0028, ADR-0030): each rule group is
    /// all-set or all-null. The material-change group is the absolute and
    /// relative thresholds; the persistent-bias group is the horizon step, the
    /// mature-week window, the minimum week count, and the bias threshold.
    pub async fn save_review_policy(&self, edit: InboxPolicyEdit) -> AppResult<InboxPolicy> {
        let change_abs = parse_threshold(edit.change_abs.as_deref(), "absolute")?;
        let change_rel = parse_threshold(edit.change_rel.as_deref(), "relative")?;
        if change_abs.is_some() != change_rel.is_some() {
            return Err(err(
                "invalid_policy",
                "Set both change thresholds to enable the change rule, or leave both empty to disable it.",
            ));
        }
        let bias_horizon = parse_bias_horizon(edit.bias_horizon.as_deref())?;
        let bias_window = parse_bias_count(edit.bias_window.as_deref(), "window")?;
        let bias_min = parse_bias_count(edit.bias_min.as_deref(), "minimum count")?;
        let bias_abs = parse_threshold(edit.bias_abs.as_deref(), "bias")?;
        let bias_set = [
            bias_horizon.is_some(),
            bias_window.is_some(),
            bias_min.is_some(),
            bias_abs.is_some(),
        ];
        if bias_set.iter().any(|set| *set) && !bias_set.iter().all(|set| *set) {
            return Err(err(
                "invalid_policy",
                "Set the horizon, window, minimum count, and bias threshold together, or leave all four empty.",
            ));
        }
        if let (Some(window), Some(minimum)) = (bias_window, bias_min) {
            if minimum > window {
                return Err(err(
                    "invalid_policy",
                    "The minimum count cannot exceed the window.",
                ));
            }
        }
        let profile_id = active_profile_id(&self.pool).await?;
        let _writer = self.writer.lock().await;
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let current: Option<i64> =
            sqlx::query_scalar("SELECT MAX(version) FROM review_policy WHERE profile_id = ?")
                .bind(profile_id)
                .fetch_one(&mut *transaction)
                .await?;
        let version = current.unwrap_or(0) + 1;
        sqlx::query(
            "INSERT INTO review_policy
             (profile_id, version, change_abs, change_rel, bias_horizon, bias_window, bias_min, bias_abs, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(profile_id)
        .bind(version)
        .bind(change_abs)
        .bind(change_rel)
        .bind(bias_horizon.map(i64::from))
        .bind(bias_window.map(i64::from))
        .bind(bias_min.map(i64::from))
        .bind(bias_abs)
        .bind(now_iso().as_str())
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        let version = i32::try_from(version)
            .map_err(|_| err("contract_violation", "The policy version is unsupported."))?;
        Ok(InboxPolicy {
            change_abs,
            change_rel,
            bias_horizon,
            bias_window,
            bias_min,
            bias_abs,
            version: Some(version),
        })
    }

    /// Derived exception inbox for one completed run (ADR-0028): one row per
    /// exception series with all applicable rule codes in the fixed priority
    /// order. Membership, fingerprints, change aggregates, and review state
    /// are computed at read time; nothing is stored. Search and the review
    /// filter apply before pagination in one read snapshot.
    pub async fn exception_inbox(
        &self,
        run_id: &str,
        search: &str,
        filter: &str,
        offset: u32,
        limit: u32,
    ) -> AppResult<ExceptionInbox> {
        if !matches!(filter, "all" | "unreviewed" | "reviewed") {
            return Err(err(
                "invalid_filter",
                "Choose all, unreviewed, or reviewed rows.",
            ));
        }
        let limit = limit.clamp(1, LIST_LIMIT_MAX);
        let run = parse_uuid(run_id, "run_id")?;
        let run_bytes = run.as_bytes().to_vec();
        let mut connection = self.pool.acquire().await?;
        let run_row = sqlx::query(
            "SELECT r.status AS status, r.revision_id AS revision_id, i.profile_id AS profile_id
             FROM forecast_run r JOIN import_revision i ON i.id = r.revision_id WHERE r.id = ?",
        )
        .bind(run_bytes.as_slice())
        .fetch_optional(&mut *connection)
        .await?
        .ok_or_else(|| {
            err(
                "unknown_run",
                "The forecast run does not exist in this workspace.",
            )
        })?;
        if run_row.get::<String, _>("status") != "completed" {
            return Err(err(
                "run_not_completed",
                "Only completed forecast runs can be reviewed.",
            ));
        }
        let revision_id = i32::try_from(run_row.get::<i64, _>("revision_id"))
            .map_err(|_| err("contract_violation", "The run revision is unsupported."))?;
        let profile_id: i64 = run_row.get("profile_id");
        let policy = review_policy_row(&mut connection, profile_id).await?;
        let predecessor = predecessor_run(&mut connection, profile_id, revision_id).await?;
        let today = Utc::now().date_naive().to_string();

        let status_rows = sqlx::query(
            "SELECT s.product_id AS product_id, s.location_id AS location_id,
                    s.status AS outcome, s.reason AS reason, s.imputed_weeks AS imputed_weeks,
                    p.sku AS series_key, l.code AS location_code
             FROM forecast_series_status s
             JOIN product p ON p.id = s.product_id
             JOIN location l ON l.id = s.location_id
             WHERE s.run_id = ?
             ORDER BY p.sku, l.code, s.product_id, s.location_id",
        )
        .bind(run_bytes.as_slice())
        .fetch_all(&mut *connection)
        .await?;

        let mut changes: HashMap<SeriesKey, (f64, f64)> = HashMap::new();
        if let Some((predecessor_bytes, _)) = &predecessor {
            let rows = sqlx::query(
                "SELECT c.product_id AS product_id, c.location_id AS location_id,
                        SUM(ABS(c.value - p.value)) AS change_abs, SUM(ABS(p.value)) AS prev_abs
                 FROM forecast c
                 JOIN forecast p ON p.run_id = ? AND p.product_id = c.product_id
                     AND p.location_id = c.location_id AND p.week_start = c.week_start
                 WHERE c.run_id = ?
                 GROUP BY c.product_id, c.location_id",
            )
            .bind(predecessor_bytes.as_slice())
            .bind(run_bytes.as_slice())
            .fetch_all(&mut *connection)
            .await?;
            for row in &rows {
                changes.insert(
                    (row.get("product_id"), row.get("location_id")),
                    (
                        row.get::<f64, _>("change_abs"),
                        row.get::<f64, _>("prev_abs"),
                    ),
                );
            }
        }

        let mut reviews: HashMap<SeriesKey, (String, String, String)> = HashMap::new();
        let rows = sqlx::query(
            "SELECT product_id, location_id, action, evidence_fingerprint, created_at
             FROM series_review WHERE profile_id = ? AND run_id = ? ORDER BY id",
        )
        .bind(profile_id)
        .bind(run_bytes.as_slice())
        .fetch_all(&mut *connection)
        .await?;
        for row in &rows {
            reviews.insert(
                (row.get("product_id"), row.get("location_id")),
                (
                    row.get("action"),
                    row.get("evidence_fingerprint"),
                    row.get("created_at"),
                ),
            );
        }

        let mut override_maps: HashMap<SeriesKey, Vec<(String, i64)>> = HashMap::new();
        let rows = sqlx::query(
            "SELECT product_id, location_id, week_start, MAX(revision) AS revision
             FROM forecast_override WHERE run_id = ?
             GROUP BY product_id, location_id, week_start
             ORDER BY product_id, location_id, week_start",
        )
        .bind(run_bytes.as_slice())
        .fetch_all(&mut *connection)
        .await?;
        for row in &rows {
            override_maps
                .entry((row.get("product_id"), row.get("location_id")))
                .or_default()
                .push((row.get("week_start"), row.get("revision")));
        }

        let mut notes: HashMap<SeriesKey, (i64, String)> = HashMap::new();
        let rows = sqlx::query(
            "SELECT product_id, location_id, COUNT(*) AS note_count, MAX(created_at) AS last_note_at
             FROM series_note WHERE profile_id = ? GROUP BY product_id, location_id",
        )
        .bind(profile_id)
        .fetch_all(&mut *connection)
        .await?;
        for row in &rows {
            notes.insert(
                (row.get("product_id"), row.get("location_id")),
                (row.get("note_count"), row.get("last_note_at")),
            );
        }

        let mut due: HashMap<SeriesKey, String> = HashMap::new();
        let rows = sqlx::query(
            "SELECT o.product_id AS product_id, o.location_id AS location_id,
                    MIN(m.review_date) AS due_review_date
             FROM forecast_override o
             JOIN override_meta m ON m.run_id = o.run_id AND m.product_id = o.product_id
                 AND m.location_id = o.location_id AND m.week_start = o.week_start
                 AND m.revision = o.revision
             WHERE o.run_id = ? AND o.value IS NOT NULL AND m.review_date IS NOT NULL
               AND m.review_date <= ?
               AND o.revision = (SELECT MAX(x.revision) FROM forecast_override x
                   WHERE x.run_id = o.run_id AND x.product_id = o.product_id
                     AND x.location_id = o.location_id AND x.week_start = o.week_start)
             GROUP BY o.product_id, o.location_id",
        )
        .bind(run_bytes.as_slice())
        .bind(today.as_str())
        .fetch_all(&mut *connection)
        .await?;
        for row in &rows {
            due.insert(
                (row.get("product_id"), row.get("location_id")),
                row.get("due_review_date"),
            );
        }

        let policy_enabled = policy.change_abs.is_some() && policy.change_rel.is_some();
        let bias_map = persistent_bias_map(&mut connection, profile_id).await?;
        let mut items: Vec<(usize, InboxItem)> = Vec::new();
        for row in &status_rows {
            let product: Vec<u8> = row.get("product_id");
            let location: Vec<u8> = row.get("location_id");
            let key: SeriesKey = (product.clone(), location.clone());
            let outcome: String = row.get("outcome");
            let reason: Option<String> = row.get("reason");
            let imputed_weeks: i64 = row.get("imputed_weeks");
            let mut rules: Vec<&'static str> = Vec::new();
            match outcome.as_str() {
                "model_failure" => rules.push("model_failure"),
                "invalid_data" => rules.push("invalid_data"),
                "insufficient_history" => rules.push("insufficient_history"),
                _ => {}
            }
            if imputed_weeks > 0 {
                rules.push("imputed_input");
            }
            if outcome == "fallback_naive" || (outcome == "ok" && reason.is_some()) {
                rules.push("production_fallback");
            }
            let mut change_abs = None;
            let mut change_rel = None;
            if policy_enabled {
                if let Some((abs, prev_abs)) = changes.get(&key) {
                    if *abs > 0.0 && *abs >= policy.change_abs.unwrap_or(f64::MAX) {
                        let rel = if *prev_abs > 0.0 {
                            Some(*abs / *prev_abs)
                        } else {
                            None
                        };
                        let fires = match rel {
                            Some(rel) => rel >= policy.change_rel.unwrap_or(f64::MAX),
                            None => true,
                        };
                        if fires {
                            rules.push("material_change");
                            change_abs = Some(*abs);
                            change_rel = rel;
                        }
                    }
                }
            }
            let bias = bias_map.get(&key).copied();
            if bias.is_some() {
                rules.push("persistent_bias");
            }
            let due_review_date = due.get(&key).cloned();
            if due_review_date.is_some() {
                rules.push("review_due");
            }
            if rules.is_empty() {
                continue;
            }
            let empty: Vec<(String, i64)> = Vec::new();
            let override_map = override_maps.get(&key).unwrap_or(&empty);
            let (review_status, reviewed_at) = match reviews.get(&key) {
                Some((action, fingerprint, created_at)) if action == "reviewed" => {
                    let current = fingerprint_from_parts(
                        &run_bytes,
                        &product,
                        &location,
                        Some((outcome.as_str(), reason.as_deref())),
                        override_map,
                        bias,
                    );
                    if &current == fingerprint {
                        ("reviewed", Some(created_at.clone()))
                    } else {
                        ("unreviewed", None)
                    }
                }
                _ => ("unreviewed", None),
            };
            let note_meta = notes.get(&key);
            let priority = INBOX_RULES
                .iter()
                .position(|rule| *rule == rules[0])
                .unwrap_or(INBOX_RULES.len());
            items.push((
                priority,
                InboxItem {
                    product_id: uuid_from_bytes(&product)?.to_string(),
                    location_id: uuid_from_bytes(&location)?.to_string(),
                    series_key: row.get("series_key"),
                    location_code: row.get("location_code"),
                    outcome,
                    reason,
                    rules: rules.into_iter().map(str::to_string).collect(),
                    change_abs,
                    change_rel,
                    due_review_date,
                    bias_value: bias.map(|(value, _, _)| value),
                    bias_weeks: bias.map(|(_, weeks, _)| weeks),
                    bias_horizon: bias.map(|(_, _, horizon)| horizon),
                    review_status: review_status.to_string(),
                    reviewed_at,
                    note_count: note_meta.map(|(count, _)| *count as u32).unwrap_or(0),
                    last_note_at: note_meta.map(|(_, last)| last.clone()),
                },
            ));
        }

        let needle = search.trim().to_lowercase();
        let mut filtered: Vec<(usize, InboxItem)> = items
            .into_iter()
            .filter(|(_, item)| {
                if !needle.is_empty() {
                    let series = item.series_key.to_lowercase();
                    let location = item.location_code.to_lowercase();
                    if !series.contains(&needle) && !location.contains(&needle) {
                        return false;
                    }
                }
                match filter {
                    "unreviewed" => item.review_status == "unreviewed",
                    "reviewed" => item.review_status == "reviewed",
                    _ => true,
                }
            })
            .collect();
        filtered.sort_by(|(left_priority, left), (right_priority, right)| {
            left_priority
                .cmp(right_priority)
                .then_with(|| left.series_key.cmp(&right.series_key))
                .then_with(|| left.location_code.cmp(&right.location_code))
                .then_with(|| left.product_id.cmp(&right.product_id))
                .then_with(|| left.location_id.cmp(&right.location_id))
        });
        let total = u32::try_from(filtered.len())
            .map_err(|_| err("contract_violation", "The inbox is too large to report."))?;
        let rows: Vec<InboxItem> = filtered
            .into_iter()
            .skip(offset as usize)
            .take(limit as usize)
            .map(|(_, item)| item)
            .collect();
        let material_change_available = predecessor.is_some();
        Ok(ExceptionInbox {
            run_id: run.to_string(),
            policy,
            predecessor_run_id: predecessor.map(|(_, id)| id),
            material_change_available,
            total,
            rows,
        })
    }

    /// Publish one completed run as the immutable official plan for its history
    /// cutoff (ADR-0029). One publication per profile and cutoff week; the
    /// exact latest override revision of every overridden cell is pinned in the
    /// same transaction, together with the highest import revision id visible
    /// to the publication (the hindsight boundary for later evaluation).
    pub async fn publish_plan(&self, edit: PublicationEdit) -> AppResult<PlanPublication> {
        let run = parse_uuid(&edit.run_id, "run_id")?;
        let _writer = self.writer.lock().await;
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let run_row = sqlx::query(
            "SELECT r.status AS status, r.revision_id AS revision_id,
                    r.horizon_start_week AS horizon_start_week, r.timezone AS timezone,
                    i.profile_id AS profile_id
             FROM forecast_run r JOIN import_revision i ON i.id = r.revision_id
             WHERE r.id = ?",
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
        if run_row.get::<String, _>("status") != "completed" {
            return Err(err(
                "run_not_completed",
                "Only completed forecast runs can be published.",
            ));
        }
        let revision_id = i32::try_from(run_row.get::<i64, _>("revision_id"))
            .map_err(|_| err("contract_violation", "The run revision is unsupported."))?;
        let profile_id: i64 = run_row.get("profile_id");
        let timezone: String = run_row.get("timezone");
        let horizon_start_week: String = run_row.get("horizon_start_week");
        let horizon_start = NaiveDate::parse_from_str(&horizon_start_week, "%Y-%m-%d")
            .map_err(|_| err("contract_violation", "The run horizon start is invalid."))?;
        let cutoff_week = (horizon_start - ChronoDuration::weeks(1))
            .format("%Y-%m-%d")
            .to_string();
        let stored_cutoff: Option<String> =
            sqlx::query_scalar("SELECT MAX(week_start) FROM observation WHERE revision_id = ?")
                .bind(revision_id)
                .fetch_one(&mut *transaction)
                .await?;
        if stored_cutoff.as_deref() != Some(cutoff_week.as_str()) {
            return Err(err(
                "contract_violation",
                "The run horizon does not match its pinned history cutoff.",
            ));
        }
        // Only finished weeks may be published: a source that reaches into the
        // unfinished current week (or beyond) must wait for a completed import.
        let today = Utc::now().date_naive();
        let current_monday =
            today - ChronoDuration::days(i64::from(today.weekday().num_days_from_monday()));
        let last_finished_week = current_monday - ChronoDuration::weeks(1);
        if horizon_start > last_finished_week + ChronoDuration::weeks(1) {
            return Err(err(
                "future_history",
                "The imported history reaches into an unfinished week. Import a completed week before publishing.",
            ));
        }
        let existing: Option<i64> = sqlx::query_scalar(
            "SELECT id FROM plan_publication WHERE profile_id = ? AND cutoff_week = ?",
        )
        .bind(profile_id)
        .bind(cutoff_week.as_str())
        .fetch_optional(&mut *transaction)
        .await?;
        if existing.is_some() {
            return Err(err(
                "already_published",
                "This history cutoff already has an official publication. Publish a newer cutoff instead.",
            ));
        }
        let revision_high_water: Option<i64> =
            sqlx::query_scalar("SELECT MAX(id) FROM import_revision WHERE profile_id = ?")
                .bind(profile_id)
                .fetch_one(&mut *transaction)
                .await?;
        let revision_high_water = revision_high_water.ok_or_else(|| {
            err(
                "contract_violation",
                "The profile has no import revisions to publish.",
            )
        })?;
        let published_at = now_iso();
        let publication_id = sqlx::query(
            "INSERT INTO plan_publication
             (profile_id, run_id, revision_id, cutoff_week, horizon_start_week, revision_high_water, timezone, policy_context, published_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(profile_id)
        .bind(run.as_bytes().as_slice())
        .bind(revision_id)
        .bind(cutoff_week.as_str())
        .bind(horizon_start_week.as_str())
        .bind(revision_high_water)
        .bind(timezone.as_str())
        .bind(PUBLICATION_CONTEXT)
        .bind(published_at.as_str())
        .execute(&mut *transaction)
        .await?
        .last_insert_rowid();
        let cells = sqlx::query(
            "INSERT INTO publication_override (publication_id, run_id, product_id, location_id, week_start, revision)
             SELECT ?, o.run_id, o.product_id, o.location_id, o.week_start, o.revision
             FROM forecast_override o
             WHERE o.run_id = ? AND o.revision = (
                 SELECT MAX(x.revision) FROM forecast_override x
                 WHERE x.run_id = o.run_id AND x.product_id = o.product_id
                     AND x.location_id = o.location_id AND x.week_start = o.week_start)",
        )
        .bind(publication_id)
        .bind(run.as_bytes().as_slice())
        .execute(&mut *transaction)
        .await?
        .rows_affected();
        let override_series: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM (
                SELECT DISTINCT product_id, location_id FROM publication_override WHERE publication_id = ?
             )",
        )
        .bind(publication_id)
        .fetch_one(&mut *transaction)
        .await?;
        let coverage = run_coverage(&mut transaction, run.as_bytes()).await?;
        transaction.commit().await?;
        Ok(PlanPublication {
            id: i32::try_from(publication_id)
                .map_err(|_| err("contract_violation", "The publication id is unsupported."))?,
            run_id: run.to_string(),
            revision_id,
            cutoff_week,
            horizon_start_week,
            revision_high_water: i32::try_from(revision_high_water).map_err(|_| {
                err(
                    "contract_violation",
                    "The revision high-water mark is unsupported.",
                )
            })?,
            published_at,
            override_cells: u32::try_from(cells)
                .map_err(|_| err("contract_violation", "The override count is unsupported."))?,
            override_series: u32::try_from(override_series)
                .map_err(|_| err("contract_violation", "The override count is unsupported."))?,
            series_count: coverage.series_count,
            covered_series: coverage.covered_series,
        })
    }

    /// Publications of the active profile, newest first.
    pub async fn publications_list(&self) -> AppResult<Vec<PlanPublication>> {
        let profile_id = active_profile_id(&self.pool).await?;
        let rows = sqlx::query(
            "SELECT pub.id, pub.run_id, pub.revision_id, pub.cutoff_week, pub.horizon_start_week,
                    pub.revision_high_water, pub.published_at,
                    (SELECT COUNT(*) FROM publication_override po WHERE po.publication_id = pub.id) AS override_cells,
                    (SELECT COUNT(*) FROM (
                        SELECT DISTINCT product_id, location_id FROM publication_override po
                        WHERE po.publication_id = pub.id)) AS override_series,
                    (SELECT COUNT(*) FROM forecast_series_status s WHERE s.run_id = pub.run_id) AS series_count,
                    (SELECT COUNT(*) FROM forecast_series_status s WHERE s.run_id = pub.run_id
                        AND s.status IN ('ok', 'fallback_naive')) AS covered_series
             FROM plan_publication pub WHERE pub.profile_id = ?
             ORDER BY pub.published_at DESC, pub.id DESC LIMIT 200",
        )
        .bind(profile_id)
        .fetch_all(&self.pool)
        .await?;
        let mut publications = Vec::with_capacity(rows.len());
        for row in &rows {
            let run_bytes: Vec<u8> = row.get("run_id");
            publications.push(PlanPublication {
                id: i32::try_from(row.get::<i64, _>("id"))
                    .map_err(|_| err("contract_violation", "The publication id is unsupported."))?,
                run_id: uuid_from_bytes(&run_bytes)?.to_string(),
                revision_id: i32::try_from(row.get::<i64, _>("revision_id"))
                    .map_err(|_| err("contract_violation", "The run revision is unsupported."))?,
                cutoff_week: row.get("cutoff_week"),
                horizon_start_week: row.get("horizon_start_week"),
                revision_high_water: i32::try_from(row.get::<i64, _>("revision_high_water"))
                    .map_err(|_| {
                        err(
                            "contract_violation",
                            "The revision high-water mark is unsupported.",
                        )
                    })?,
                published_at: row.get("published_at"),
                override_cells: u32::try_from(row.get::<i64, _>("override_cells"))
                    .map_err(|_| err("contract_violation", "The override count is unsupported."))?,
                override_series: u32::try_from(row.get::<i64, _>("override_series"))
                    .map_err(|_| err("contract_violation", "The override count is unsupported."))?,
                series_count: u32::try_from(row.get::<i64, _>("series_count"))
                    .map_err(|_| err("contract_violation", "The series count is unsupported."))?,
                covered_series: u32::try_from(row.get::<i64, _>("covered_series"))
                    .map_err(|_| err("contract_violation", "The series count is unsupported."))?,
            });
        }
        Ok(publications)
    }

    /// The published plan for one series: pinned override revisions from the
    /// publication plus immutable baselines (ADR-0029).
    pub async fn publication_series_detail(
        &self,
        publication_id: i32,
        product_id: &str,
        location_id: &str,
    ) -> AppResult<PublicationSeriesDetail> {
        let product = parse_uuid(product_id, "product_id")?;
        let location = parse_uuid(location_id, "location_id")?;
        let mut connection = self.pool.acquire().await?;
        let publication = sqlx::query("SELECT run_id FROM plan_publication WHERE id = ?")
            .bind(i64::from(publication_id))
            .fetch_optional(&mut *connection)
            .await?
            .ok_or_else(|| {
                err(
                    "unknown_publication",
                    "The publication does not exist in this workspace.",
                )
            })?;
        let run_bytes: Vec<u8> = publication.get("run_id");
        let rows = load_planning_rows(
            &mut connection,
            PlanSource::Publication {
                run: &run_bytes,
                publication_id: i64::from(publication_id),
            },
            Some(product.as_bytes().as_slice()),
            Some(location.as_bytes().as_slice()),
            (&[], &[]),
            1,
        )
        .await?;
        let Some(first) = rows.first() else {
            return Err(err(
                "unknown_series",
                "The selected series is not part of this publication.",
            ));
        };
        Ok(PublicationSeriesDetail {
            publication_id,
            product_id: uuid_from_bytes(&first.get::<Vec<u8>, _>("product_id"))?.to_string(),
            location_id: uuid_from_bytes(&first.get::<Vec<u8>, _>("location_id"))?.to_string(),
            series_key: first.get("product_sku"),
            location_code: first.get("location_code"),
            outcome: first.get("series_status"),
            reason: None,
            planning: rows.iter().map(planning_point).collect(),
        })
    }

    /// Append one actuals confirmation (ADR-0030): the named revision is
    /// complete through a finished week. Corrections and retractions append a
    /// new row; evaluations always pin the exact confirmation they used.
    pub async fn actuals_confirmation_set(
        &self,
        edit: ActualsConfirmationEdit,
    ) -> AppResult<ActualsConfirmation> {
        ensure_revision(&self.pool, edit.revision_id).await?;
        let week = parse_confirmed_week(&edit.confirmed_through)?;
        let today = Utc::now().date_naive();
        let current_monday =
            today - ChronoDuration::days(i64::from(today.weekday().num_days_from_monday()));
        let last_finished_week = current_monday - ChronoDuration::weeks(1);
        if week > last_finished_week {
            return Err(err(
                "confirmation_not_finished",
                "Confirm completeness only through a finished week.",
            ));
        }
        let profile_id: i64 =
            sqlx::query_scalar("SELECT profile_id FROM import_revision WHERE id = ?")
                .bind(edit.revision_id)
                .fetch_one(&self.pool)
                .await?;
        let _writer = self.writer.lock().await;
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let timestamp = now_iso();
        let id = sqlx::query(
            "INSERT INTO actuals_confirmation (profile_id, revision_id, confirmed_through, created_at)
             VALUES (?, ?, ?, ?)",
        )
        .bind(profile_id)
        .bind(edit.revision_id)
        .bind(week.format("%Y-%m-%d").to_string())
        .bind(timestamp.as_str())
        .execute(&mut *transaction)
        .await?
        .last_insert_rowid();
        transaction.commit().await?;
        Ok(ActualsConfirmation {
            id: i32::try_from(id)
                .map_err(|_| err("contract_violation", "The confirmation id is unsupported."))?,
            revision_id: edit.revision_id,
            confirmed_through: week.format("%Y-%m-%d").to_string(),
            created_at: timestamp,
        })
    }

    /// The current (latest) actuals confirmation of the active profile.
    pub async fn actuals_confirmation_latest(&self) -> AppResult<Option<ActualsConfirmation>> {
        let profile_id = active_profile_id(&self.pool).await?;
        let mut connection = self.pool.acquire().await?;
        latest_confirmation(&mut connection, profile_id).await
    }

    /// Derived retrospective planner FVA across the profile's publications at
    /// one horizon step (ADR-0030). Nothing is stored; the latest confirmation
    /// pins the actuals revision, and every exclusion is reported explicitly.
    pub async fn fva_report(
        &self,
        horizon: u32,
        search: &str,
        offset: u32,
        limit: u32,
    ) -> AppResult<FvaReport> {
        if !(1..=12).contains(&horizon) {
            return Err(err(
                "invalid_horizon",
                "Choose a horizon step from 1 to 12.",
            ));
        }
        let limit = limit.clamp(1, LIST_LIMIT_MAX);
        let profile_id = active_profile_id(&self.pool).await?;
        let mut connection = self.pool.acquire().await?;
        let confirmation = latest_confirmation(&mut connection, profile_id).await?;
        let publications = sqlx::query(
            "SELECT id, run_id, revision_id, cutoff_week, horizon_start_week,
                    revision_high_water, published_at
             FROM plan_publication WHERE profile_id = ?
             ORDER BY published_at DESC, id DESC LIMIT 200",
        )
        .bind(profile_id)
        .fetch_all(&mut *connection)
        .await?;
        let mut publication_rows = Vec::new();
        let mut series_acc: HashMap<SeriesKey, SeriesFvaAccumulator> = HashMap::new();
        for row in &publications {
            let publication_id: i64 = row.get("id");
            let run_bytes: Vec<u8> = row.get("run_id");
            let revision_id = i32::try_from(row.get::<i64, _>("revision_id"))
                .map_err(|_| err("contract_violation", "The run revision is unsupported."))?;
            let high_water: i64 = row.get("revision_high_water");
            let horizon_start_week: String = row.get("horizon_start_week");
            let published_at: String = row.get("published_at");
            let mut excluded = FvaExclusions::default();
            let mut scored = 0u32;
            let mut adjusted = 0u32;
            let mut imputed = 0u32;
            let mut improved = 0u32;
            let mut harmful = 0u32;
            let mut unchanged = 0u32;
            if let Some(confirmation) = &confirmation {
                // Publications whose high-water mark reaches the confirmation
                // revision exclude every visible actual as hindsight; the
                // per-point verdict does that explicitly, never by skipping.
                let points = publication_fva_points(
                    &mut connection,
                    profile_id,
                    publication_id,
                    &run_bytes,
                    high_water,
                    &horizon_start_week,
                    &published_at,
                    confirmation,
                    horizon,
                )
                .await?;
                for point in &points {
                    match point.excluded {
                        Some(code) => excluded.bump(code),
                        None => {
                            scored += 1;
                            let baseline = point.baseline.unwrap_or_default();
                            let effective = point.effective.unwrap_or_default();
                            let actual = point.actual.unwrap_or_default();
                            if effective != baseline {
                                adjusted += 1;
                            }
                            if point.imputed_input {
                                imputed += 1;
                            }
                            match (effective - actual)
                                .abs()
                                .partial_cmp(&(baseline - actual).abs())
                            {
                                Some(std::cmp::Ordering::Less) => improved += 1,
                                Some(std::cmp::Ordering::Greater) => harmful += 1,
                                _ => unchanged += 1,
                            }
                            series_acc
                                .entry((point.product.clone(), point.location.clone()))
                                .or_insert_with(|| SeriesFvaAccumulator {
                                    series_key: point.series_key.clone(),
                                    location_code: point.location_code.clone(),
                                    ..SeriesFvaAccumulator::default()
                                })
                                .push_point(baseline, effective, actual, point.imputed_input);
                        }
                    }
                }
            }
            publication_rows.push(PublicationFva {
                publication_id: i32::try_from(publication_id)
                    .map_err(|_| err("contract_violation", "The publication id is unsupported."))?,
                run_id: uuid_from_bytes(&run_bytes)?.to_string(),
                revision_id,
                cutoff_week: row.get("cutoff_week"),
                horizon,
                scored,
                adjusted,
                imputed_input: imputed,
                improved,
                harmful,
                unchanged,
                excluded,
                low_sample: scored > 0 && scored < 5,
            });
        }
        let needle = search.trim().to_lowercase();
        let mut rows: Vec<SeriesFvaRow> = series_acc
            .into_iter()
            .map(|((product, location), accumulator)| {
                let measures = accumulator.measures();
                Ok(SeriesFvaRow {
                    product_id: uuid_from_bytes(&product)?.to_string(),
                    location_id: uuid_from_bytes(&location)?.to_string(),
                    series_key: accumulator.series_key,
                    location_code: accumulator.location_code,
                    scored: measures.points,
                    adjusted: accumulator.adjusted,
                    measures,
                    imputed_input: accumulator.imputed_input,
                })
            })
            .collect::<AppResult<Vec<_>>>()?;
        if !needle.is_empty() {
            rows.retain(|row| {
                row.series_key.to_lowercase().contains(&needle)
                    || row.location_code.to_lowercase().contains(&needle)
            });
        }
        rows.sort_by(|left, right| {
            left.series_key
                .cmp(&right.series_key)
                .then_with(|| left.location_code.cmp(&right.location_code))
                .then_with(|| left.product_id.cmp(&right.product_id))
                .then_with(|| left.location_id.cmp(&right.location_id))
        });
        let total = u32::try_from(rows.len())
            .map_err(|_| err("contract_violation", "The report is too large."))?;
        let rows = rows
            .into_iter()
            .skip(offset as usize)
            .take(limit as usize)
            .collect();
        Ok(FvaReport {
            horizon,
            confirmation,
            publications: publication_rows,
            total,
            rows,
        })
    }

    /// Every target point of one publication at one horizon: scored values and
    /// explicit exclusions, for the per-publication detail view (ADR-0030).
    pub async fn fva_publication_series(
        &self,
        publication_id: i32,
        horizon: u32,
        search: &str,
        offset: u32,
        limit: u32,
    ) -> AppResult<PublicationFvaPoints> {
        if !(1..=12).contains(&horizon) {
            return Err(err(
                "invalid_horizon",
                "Choose a horizon step from 1 to 12.",
            ));
        }
        let limit = limit.clamp(1, LIST_LIMIT_MAX);
        let (profile_id, run_bytes, high_water, horizon_start_week, published_at, confirmation) = {
            let mut connection = self.pool.acquire().await?;
            let row = sqlx::query(
                "SELECT run_id, revision_high_water, horizon_start_week, published_at, profile_id
                 FROM plan_publication WHERE id = ?",
            )
            .bind(i64::from(publication_id))
            .fetch_optional(&mut *connection)
            .await?
            .ok_or_else(|| {
                err(
                    "unknown_publication",
                    "The publication does not exist in this workspace.",
                )
            })?;
            let profile_id: i64 = row.get("profile_id");
            let confirmation = latest_confirmation(&mut connection, profile_id).await?;
            (
                profile_id,
                row.get::<Vec<u8>, _>("run_id"),
                row.get::<i64, _>("revision_high_water"),
                row.get::<String, _>("horizon_start_week"),
                row.get::<String, _>("published_at"),
                confirmation,
            )
        };
        let mut rows = Vec::new();
        if let Some(confirmation) = confirmation {
            let mut connection = self.pool.acquire().await?;
            let points = publication_fva_points(
                &mut connection,
                profile_id,
                i64::from(publication_id),
                &run_bytes,
                high_water,
                &horizon_start_week,
                &published_at,
                &confirmation,
                horizon,
            )
            .await?;
            for point in points {
                rows.push(FvaPoint {
                    product_id: uuid_from_bytes(&point.product)?.to_string(),
                    location_id: uuid_from_bytes(&point.location)?.to_string(),
                    series_key: point.series_key,
                    location_code: point.location_code,
                    week_start: point.week,
                    actual: point.actual,
                    baseline: point.baseline,
                    effective: point.effective,
                    baseline_error: match (point.baseline, point.actual) {
                        (Some(baseline), Some(actual)) => Some(baseline - actual),
                        _ => None,
                    },
                    effective_error: match (point.effective, point.actual) {
                        (Some(effective), Some(actual)) => Some(effective - actual),
                        _ => None,
                    },
                    adjustment: match (point.effective, point.baseline) {
                        (Some(effective), Some(baseline)) => Some(effective - baseline),
                        _ => None,
                    },
                    imputed_input: point.imputed_input,
                    excluded: point.excluded.map(str::to_string),
                });
            }
        }
        let needle = search.trim().to_lowercase();
        if !needle.is_empty() {
            rows.retain(|row| {
                row.series_key.to_lowercase().contains(&needle)
                    || row.location_code.to_lowercase().contains(&needle)
            });
        }
        let total = u32::try_from(rows.len())
            .map_err(|_| err("contract_violation", "The report is too large."))?;
        let rows = rows
            .into_iter()
            .skip(offset as usize)
            .take(limit as usize)
            .collect();
        Ok(PublicationFvaPoints {
            publication_id,
            horizon,
            total,
            rows,
        })
    }

    /// Export a consistent full-run plan in bounded batches. The workspace copy
    /// gives export_record a durable relative path; the selected file is a copy.
    /// Neither publication may overwrite existing data.
    pub async fn export_csv(&self, run_id: &str, target: &Path) -> AppResult<ExportResult> {
        let run = parse_uuid(run_id, "run_id")?;
        self.export_plan(PlanSelection::Run(run), target, ExportFormat::Csv)
            .await
    }

    pub async fn export_xlsx(&self, run_id: &str, target: &Path) -> AppResult<ExportResult> {
        let run = parse_uuid(run_id, "run_id")?;
        self.export_plan(PlanSelection::Run(run), target, ExportFormat::Xlsx)
            .await
    }

    /// Export the immutable published values of one publication (ADR-0029); the
    /// export record links back to the publication.
    pub async fn export_publication_csv(
        &self,
        publication_id: i32,
        target: &Path,
    ) -> AppResult<ExportResult> {
        self.export_plan(
            PlanSelection::Publication(publication_id),
            target,
            ExportFormat::Csv,
        )
        .await
    }

    pub async fn export_publication_xlsx(
        &self,
        publication_id: i32,
        target: &Path,
    ) -> AppResult<ExportResult> {
        self.export_plan(
            PlanSelection::Publication(publication_id),
            target,
            ExportFormat::Xlsx,
        )
        .await
    }

    async fn export_plan(
        &self,
        selection: PlanSelection,
        target: &Path,
        format: ExportFormat,
    ) -> AppResult<ExportResult> {
        let extension = match format {
            ExportFormat::Csv => "csv",
            ExportFormat::Xlsx => "xlsx",
        };
        if !target.is_absolute()
            || !target
                .extension()
                .is_some_and(|value| value.eq_ignore_ascii_case(extension))
        {
            return Err(err(
                "invalid_export_path",
                format!("Choose an absolute path ending in .{extension}."),
            ));
        }
        let parent = target
            .parent()
            .ok_or_else(|| err("invalid_export_path", "Choose an export file destination."))?
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
            // occupying the sole writer connection while a large export is built.
            let mut reader = self.read_connection().await?;
            let mut snapshot = reader.begin().await?;
            let (run, publication) = match selection {
                PlanSelection::Run(run) => (run, None),
                PlanSelection::Publication(publication_id) => {
                    let row = sqlx::query(
                        "SELECT run_id, published_at, cutoff_week, revision_high_water, policy_context
                         FROM plan_publication WHERE id = ?",
                    )
                    .bind(i64::from(publication_id))
                    .fetch_optional(&mut *snapshot)
                    .await?
                    .ok_or_else(|| {
                        err(
                            "unknown_publication",
                            "The publication does not exist in this workspace.",
                        )
                    })?;
                    let run_bytes: Vec<u8> = row.get("run_id");
                    (
                        uuid_from_bytes(&run_bytes)?,
                        Some(PublicationStamp {
                            id: i64::from(publication_id),
                            published_at: row.get("published_at"),
                            cutoff_week: row.get("cutoff_week"),
                            revision_high_water: row.get("revision_high_water"),
                        }),
                    )
                }
            };
            let status: Option<String> = sqlx::query_scalar("SELECT status FROM forecast_run WHERE id = ?")
                .bind(run.as_bytes().as_slice()).fetch_optional(&mut *snapshot).await?;
            match status.as_deref() {
                None => return Err(err("unknown_run", "The forecast run does not exist in this workspace.")),
                Some("completed") => {},
                _ => return Err(err("run_not_completed", "Only completed forecast runs can be exported.")),
            }
            let coverage = run_coverage(&mut snapshot, run.as_bytes()).await?;

            let source = match &publication {
                Some(stamp) => PlanSource::Publication {
                    run: run.as_bytes(),
                    publication_id: stamp.id,
                },
                None => PlanSource::Run(run.as_bytes()),
            };
            let mut temporary = tempfile::Builder::new().prefix(".opendemand-export-").tempfile_in(&parent)?;
            let row_count = match format {
                ExportFormat::Csv => write_csv_plan(&mut snapshot, source, &run, &mut temporary).await?,
                ExportFormat::Xlsx => write_xlsx_plan(&mut snapshot, source, &run, &mut temporary, &coverage, publication.as_ref()).await?,
            };
            let coverage = serde_json::to_string(&coverage)?;
            snapshot.commit().await?;
            reader.close().await?;
            temporary.as_file().sync_all()?;

            let exports = self.path.join("exports");
            std::fs::create_dir_all(&exports)?;
            let relative_path = format!("exports/{}.{extension}", Uuid::now_v7());
            let archive_path = self.path.join(&relative_path);
            let archive = tempfile::Builder::new().prefix(".opendemand-export-").tempfile_in(&exports)?;
            std::fs::copy(temporary.path(), archive.path())?;
            archive.as_file().sync_all()?;
            let _writer = self.writer.lock().await;
            let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
            let export_id = sqlx::query(
                "INSERT INTO export_record (run_id, format, relative_path, created_at, row_count, coverage_json, publication_id)
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
            ).bind(run.as_bytes().as_slice()).bind(extension).bind(&relative_path).bind(now_iso()).bind(row_count).bind(coverage)
                .bind(publication.as_ref().map(|stamp| stamp.id))
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

    /// Read aggregate outcomes and a bounded exception page from one snapshot.
    pub async fn run_overview(
        &self,
        run_id: &str,
        offset: u32,
        limit: u32,
    ) -> AppResult<RunOverview> {
        let run = parse_uuid(run_id, "run_id")?;
        let mut reader = self.read_connection().await?;
        let mut transaction = reader.begin().await?;
        let status: Option<String> =
            sqlx::query_scalar("SELECT status FROM forecast_run WHERE id = ?")
                .bind(run.as_bytes().as_slice())
                .fetch_optional(&mut *transaction)
                .await?;
        match status.as_deref() {
            None => {
                return Err(err(
                    "unknown_run",
                    "The forecast run does not exist in this workspace.",
                ))
            }
            Some("completed") => {}
            _ => return Err(err("run_not_completed", "Choose a completed forecast run.")),
        }
        let mut coverage = run_coverage(&mut transaction, run.as_bytes()).await?;
        for outcome in [
            "ok",
            "fallback_naive",
            "insufficient_history",
            "model_failure",
            "invalid_data",
        ] {
            coverage.outcomes.entry(outcome.into()).or_insert(0);
        }
        let exception_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM forecast_series_status
             WHERE run_id = ? AND (status != 'ok' OR reason IS NOT NULL)",
        )
        .bind(run.as_bytes().as_slice())
        .fetch_one(&mut *transaction)
        .await?;
        let rows = sqlx::query(
            "SELECT s.product_id, s.location_id, p.sku, l.code, s.status, s.reason
             FROM forecast_series_status s
             JOIN product p ON p.id = s.product_id JOIN location l ON l.id = s.location_id
             WHERE s.run_id = ? AND (s.status != 'ok' OR s.reason IS NOT NULL)
             ORDER BY CASE s.status WHEN 'model_failure' THEN 0 WHEN 'invalid_data' THEN 1
                 WHEN 'insufficient_history' THEN 2 WHEN 'fallback_naive' THEN 3 ELSE 4 END,
                 p.sku, l.code, s.product_id, s.location_id
             LIMIT ? OFFSET ?",
        )
        .bind(run.as_bytes().as_slice())
        .bind(i64::from(limit.clamp(1, LIST_LIMIT_MAX)))
        .bind(i64::from(offset))
        .fetch_all(&mut *transaction)
        .await?;
        let mut exceptions = Vec::with_capacity(rows.len());
        for row in rows {
            exceptions.push(RunException {
                product_id: uuid_from_bytes(&row.get::<Vec<u8>, _>("product_id"))?.to_string(),
                location_id: uuid_from_bytes(&row.get::<Vec<u8>, _>("location_id"))?.to_string(),
                series_key: row.get("sku"),
                location_code: row.get("code"),
                outcome: row.get("status"),
                reason: row.get("reason"),
            });
        }
        transaction.commit().await?;
        reader.close().await?;
        Ok(RunOverview {
            series_count: coverage.series_count,
            covered_series: coverage.covered_series,
            coverage: coverage.coverage,
            outcomes: coverage.outcomes,
            exception_count: exception_count as u32,
            exceptions,
        })
    }

    async fn read_connection(&self) -> AppResult<SqliteConnection> {
        Ok(SqliteConnection::connect_with(
            &SqliteConnectOptions::new()
                .filename(self.path.join(DB_FILENAME))
                .read_only(true)
                .foreign_keys(true)
                .journal_mode(SqliteJournalMode::Wal)
                .synchronous(SqliteSynchronous::Full)
                .busy_timeout(Duration::from_millis(5_000)),
        )
        .await?)
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
            runs.push(run_summary_row(row)?);
        }
        Ok(runs)
    }

    /// The default weekly comparison pair (ADR-0022): the newest completed run
    /// of the latest revision and of the immediately preceding revision of the
    /// active profile. An unforecast previous revision is never skipped.
    pub async fn weekly_comparison_context(&self) -> AppResult<WeeklyComparisonContext> {
        let profile_id = active_profile_id(&self.pool).await?;
        let latest: Option<i32> =
            sqlx::query_scalar("SELECT MAX(id) FROM import_revision WHERE profile_id = ?")
                .bind(profile_id)
                .fetch_one(&self.pool)
                .await?;
        let Some(current_revision) = latest else {
            return Ok(WeeklyComparisonContext {
                current: None,
                previous: None,
            });
        };
        let Some(current) = self.newest_completed_run(current_revision).await? else {
            return Ok(WeeklyComparisonContext {
                current: None,
                previous: None,
            });
        };
        let previous_revision: Option<i32> = sqlx::query_scalar(
            "SELECT MAX(id) FROM import_revision WHERE profile_id = ? AND id < ?",
        )
        .bind(profile_id)
        .bind(current_revision)
        .fetch_one(&self.pool)
        .await?;
        let previous = match previous_revision {
            Some(revision) => self.newest_completed_run(revision).await?,
            None => None,
        };
        Ok(WeeklyComparisonContext {
            current: Some(current),
            previous,
        })
    }

    async fn newest_completed_run(&self, revision_id: i32) -> AppResult<Option<RunSummary>> {
        let row = sqlx::query(
            "SELECT id, revision_id, status, created_at, horizon_start_week, engine_version
             FROM forecast_run WHERE revision_id = ? AND status = 'completed'
             ORDER BY created_at DESC, id DESC LIMIT 1",
        )
        .bind(revision_id)
        .fetch_optional(&self.pool)
        .await?;
        match row {
            Some(row) => Ok(Some(run_summary_row(&row)?)),
            None => Ok(None),
        }
    }

    /// List only the series that changed between two adjacent revisions, in
    /// stable SKU/location order. The pair is re-validated on every call.
    pub async fn weekly_comparison_series_list(
        &self,
        current_run_id: &str,
        previous_run_id: &str,
        search: &str,
        filter: WeeklyChangeFilter,
        offset: u32,
        limit: u32,
    ) -> AppResult<Vec<SeriesChangeSummary>> {
        let pair = self
            .comparison_pair(current_run_id, previous_run_id)
            .await?;
        let limit = limit.clamp(1, LIST_LIMIT_MAX) as i64;
        let offset = offset as i64;
        let pattern = format!("%{search}%");
        let filter = match filter {
            WeeklyChangeFilter::All => "all",
            WeeklyChangeFilter::Demand => "demand",
            WeeklyChangeFilter::Forecast => "forecast",
            WeeklyChangeFilter::Presence => "presence",
        };
        let rows = sqlx::query(WEEKLY_CHANGED_SERIES_SQL)
            .bind(pair.current_revision)
            .bind(pair.previous_revision)
            .bind(pair.current_revision)
            .bind(pair.previous_revision)
            .bind(pair.previous_revision)
            .bind(pair.current_revision)
            .bind(pair.current_revision)
            .bind(pair.previous_revision)
            .bind(pair.previous_run.as_slice())
            .bind(pair.current_run.as_slice())
            .bind(pair.current_run.as_slice())
            .bind(pair.previous_start.to_string())
            .bind(pair.previous_end.to_string())
            .bind(pair.previous_run.as_slice())
            .bind(pair.previous_run.as_slice())
            .bind(pair.current_start.to_string())
            .bind(pair.current_end.to_string())
            .bind(pair.current_run.as_slice())
            .bind(pattern.as_str())
            .bind(filter)
            .bind(filter)
            .bind(filter)
            .bind(filter)
            .bind(limit)
            .bind(offset)
            .fetch_all(&self.pool)
            .await?;

        let mut changed = Vec::with_capacity(rows.len());
        for row in &rows {
            let product: Vec<u8> = row.get("product_id");
            let location: Vec<u8> = row.get("location_id");
            let presence = match (
                row.get::<i64, _>("in_current"),
                row.get::<i64, _>("in_previous"),
            ) {
                (1, 1) => "both",
                (1, _) => "current_only",
                _ => "previous_only",
            };
            changed.push(SeriesChangeSummary {
                product_id: uuid_from_bytes(&product)?.to_string(),
                location_id: uuid_from_bytes(&location)?.to_string(),
                location_code: row.get("location_code"),
                series_key: row.get("sku"),
                demand_change_count: row.get::<i64, _>("demand_changes") as u32,
                forecast_change_count: row.get::<i64, _>("forecast_changes") as u32,
                presence: presence.to_string(),
            });
        }
        Ok(changed)
    }

    /// Weekly demand and baseline forecast comparison for one series, plus the
    /// current run's effective plan (ADR-0022).
    pub async fn weekly_comparison_series_detail(
        &self,
        current_run_id: &str,
        previous_run_id: &str,
        product_id: &str,
        location_id: &str,
    ) -> AppResult<SeriesComparisonDetail> {
        let pair = self
            .comparison_pair(current_run_id, previous_run_id)
            .await?;
        let product = parse_uuid(product_id, "product_id")?;
        let location = parse_uuid(location_id, "location_id")?;
        let product_bytes = product.as_bytes().to_vec();
        let location_bytes = location.as_bytes().to_vec();

        let sku: Option<String> = sqlx::query_scalar("SELECT sku FROM product WHERE id = ?")
            .bind(product_bytes.as_slice())
            .fetch_optional(&self.pool)
            .await?;
        let location_code: Option<String> =
            sqlx::query_scalar("SELECT code FROM location WHERE id = ?")
                .bind(location_bytes.as_slice())
                .fetch_optional(&self.pool)
                .await?;
        let (Some(series_key), Some(location_code)) = (sku, location_code) else {
            return Err(err(
                "unknown_series",
                "The selected series is not part of the compared runs.",
            ));
        };

        let in_current: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM series_qualification
             WHERE revision_id = ? AND product_id = ? AND location_id = ?)",
        )
        .bind(pair.current_revision)
        .bind(product_bytes.as_slice())
        .bind(location_bytes.as_slice())
        .fetch_one(&self.pool)
        .await?;
        let in_previous: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM series_qualification
             WHERE revision_id = ? AND product_id = ? AND location_id = ?)",
        )
        .bind(pair.previous_revision)
        .bind(product_bytes.as_slice())
        .bind(location_bytes.as_slice())
        .fetch_one(&self.pool)
        .await?;
        if !in_current && !in_previous {
            return Err(err(
                "unknown_series",
                "The selected series is not part of the compared runs.",
            ));
        }
        let presence = match (in_current, in_previous) {
            (true, true) => "both",
            (true, false) => "current_only",
            _ => "previous_only",
        };

        let current_outcome: Option<String> = sqlx::query_scalar(
            "SELECT status FROM forecast_series_status
             WHERE run_id = ? AND product_id = ? AND location_id = ?",
        )
        .bind(pair.current_run.as_slice())
        .bind(product_bytes.as_slice())
        .bind(location_bytes.as_slice())
        .fetch_optional(&self.pool)
        .await?;
        let previous_outcome: Option<String> = sqlx::query_scalar(
            "SELECT status FROM forecast_series_status
             WHERE run_id = ? AND product_id = ? AND location_id = ?",
        )
        .bind(pair.previous_run.as_slice())
        .bind(product_bytes.as_slice())
        .bind(location_bytes.as_slice())
        .fetch_optional(&self.pool)
        .await?;

        let current_obs = load_observations(
            &self.pool,
            pair.current_revision,
            &product_bytes,
            &location_bytes,
        )
        .await?;
        let previous_obs = load_observations(
            &self.pool,
            pair.previous_revision,
            &product_bytes,
            &location_bytes,
        )
        .await?;
        let mut weeks: BTreeSet<NaiveDate> = BTreeSet::new();
        weeks.extend(current_obs.keys());
        weeks.extend(previous_obs.keys());
        let mut demand_changes = Vec::new();
        for week in weeks {
            let current = current_obs.get(&week);
            let previous = previous_obs.get(&week);
            let week_start = week.format("%Y-%m-%d").to_string();
            match (current, previous) {
                (
                    Some((current_quantity, current_status)),
                    Some((previous_quantity, previous_status)),
                ) => {
                    if current_quantity != previous_quantity || current_status != previous_status {
                        demand_changes.push(DemandChangePoint {
                            week_start,
                            previous_quantity: *previous_quantity,
                            previous_status: Some(previous_status.clone()),
                            current_quantity: *current_quantity,
                            current_status: Some(current_status.clone()),
                            change_kind: "changed".to_string(),
                        });
                    }
                }
                (Some((current_quantity, current_status)), None) => {
                    demand_changes.push(DemandChangePoint {
                        week_start,
                        previous_quantity: None,
                        previous_status: None,
                        current_quantity: *current_quantity,
                        current_status: Some(current_status.clone()),
                        change_kind: "added".to_string(),
                    });
                }
                (None, Some((previous_quantity, previous_status))) => {
                    demand_changes.push(DemandChangePoint {
                        week_start,
                        previous_quantity: *previous_quantity,
                        previous_status: Some(previous_status.clone()),
                        current_quantity: None,
                        current_status: None,
                        change_kind: "removed".to_string(),
                    });
                }
                (None, None) => {}
            }
        }

        let current_points = load_forecast_rows(
            &self.pool,
            &pair.current_run,
            &product_bytes,
            &location_bytes,
        )
        .await?;
        let previous_points = load_forecast_rows(
            &self.pool,
            &pair.previous_run,
            &product_bytes,
            &location_bytes,
        )
        .await?;
        let mut forecast_weeks: BTreeSet<NaiveDate> = BTreeSet::new();
        forecast_weeks.extend(current_points.keys());
        forecast_weeks.extend(previous_points.keys());
        let mut forecast_changes = Vec::new();
        for week in forecast_weeks {
            let current_value = current_points.get(&week).copied();
            let previous_value = previous_points.get(&week).copied();
            let (comparability, is_change) = match (current_value, previous_value) {
                (Some(current), Some(previous)) => ("common", current != previous),
                (Some(_), None) => (
                    "current_only",
                    week >= pair.previous_start && week <= pair.previous_end,
                ),
                (None, Some(_)) => (
                    "previous_only",
                    week >= pair.current_start && week <= pair.current_end,
                ),
                (None, None) => continue,
            };
            let delta = match (current_value, previous_value) {
                (Some(current), Some(previous)) => Some(current - previous),
                _ => None,
            };
            forecast_changes.push(ForecastChangePoint {
                week_start: week.format("%Y-%m-%d").to_string(),
                previous_value,
                current_value,
                delta,
                comparability: comparability.to_string(),
                is_change,
            });
        }

        let planning = {
            let mut connection = self.pool.acquire().await?;
            load_planning_rows(
                &mut connection,
                PlanSource::Run(&pair.current_run),
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

        Ok(SeriesComparisonDetail {
            product_id: product.to_string(),
            location_id: location.to_string(),
            location_code,
            series_key,
            presence: presence.to_string(),
            current_outcome,
            previous_outcome,
            demand_changes,
            forecast_changes,
            planning,
        })
    }

    /// Validate a caller-supplied comparison pair and resolve its revisions,
    /// run ids, and horizon windows.
    async fn comparison_pair(
        &self,
        current_run_id: &str,
        previous_run_id: &str,
    ) -> AppResult<ComparisonPair> {
        let current = parse_uuid(current_run_id, "current_run_id")?;
        let previous = parse_uuid(previous_run_id, "previous_run_id")?;
        let current_row = self.run_row(current.as_bytes()).await?;
        let previous_row = self.run_row(previous.as_bytes()).await?;

        let current_revision: i32 = current_row.get("revision_id");
        let previous_revision: i32 = previous_row.get("revision_id");
        if previous_revision >= current_revision {
            return Err(err(
                "invalid_comparison_pair",
                "The current run must pin a newer revision than the previous run.",
            ));
        }
        let current_profile: i64 =
            sqlx::query_scalar("SELECT profile_id FROM import_revision WHERE id = ?")
                .bind(current_revision)
                .fetch_one(&self.pool)
                .await?;
        let previous_profile: i64 =
            sqlx::query_scalar("SELECT profile_id FROM import_revision WHERE id = ?")
                .bind(previous_revision)
                .fetch_one(&self.pool)
                .await?;
        if current_profile != previous_profile {
            return Err(err(
                "invalid_comparison_pair",
                "The compared runs pin revisions from different source profiles.",
            ));
        }
        let adjacent: Option<i32> = sqlx::query_scalar(
            "SELECT MAX(id) FROM import_revision WHERE profile_id = ? AND id < ?",
        )
        .bind(current_profile)
        .bind(current_revision)
        .fetch_one(&self.pool)
        .await?;
        if adjacent != Some(previous_revision) {
            return Err(err(
                "not_adjacent_revisions",
                "The compared runs do not pin adjacent import revisions.",
            ));
        }

        let current_start = parse_week(&current_row.get::<String, _>("horizon_start_week"))?;
        let previous_start = parse_week(&previous_row.get::<String, _>("horizon_start_week"))?;
        let current_end = horizon_end(current_start, current_row.get("horizon_weeks"))?;
        let previous_end = horizon_end(previous_start, previous_row.get("horizon_weeks"))?;
        Ok(ComparisonPair {
            current_revision,
            previous_revision,
            current_run: current.as_bytes().to_vec(),
            previous_run: previous.as_bytes().to_vec(),
            current_start,
            previous_start,
            current_end,
            previous_end,
        })
    }

    async fn run_row(&self, run: &[u8]) -> AppResult<SqliteRow> {
        let row = sqlx::query(
            "SELECT revision_id, status, horizon_start_week, horizon_weeks
             FROM forecast_run WHERE id = ?",
        )
        .bind(run)
        .fetch_optional(&self.pool)
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
                "Only completed forecast runs can be compared.",
            ));
        }
        Ok(row)
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
                imputed_indices: Vec::new(),
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

#[derive(serde::Serialize)]
struct RunCoverage {
    series_count: u32,
    covered_series: u32,
    coverage: f64,
    outcomes: BTreeMap<String, u32>,
}

async fn write_csv_plan(
    connection: &mut SqliteConnection,
    source: PlanSource<'_>,
    run_label: &Uuid,
    temporary: &mut tempfile::NamedTempFile,
) -> AppResult<i64> {
    // Keep the existing Excel-friendly CSV bytes unchanged.
    temporary.write_all(b"\xef\xbb\xbf")?;
    let mut writer = csv::WriterBuilder::new()
        .terminator(csv::Terminator::CRLF)
        .from_writer(temporary);
    writer
        .write_record(EXPORT_HEADERS)
        .map_err(|_| err("export_failed", "Could not write the CSV header."))?;
    let mut after = (Vec::<u8>::new(), Vec::<u8>::new());
    let mut row_count = 0i64;
    loop {
        let rows =
            load_planning_rows(connection, source, None, None, (&after.0, &after.1), 32).await?;
        let Some(last) = rows.last() else {
            break;
        };
        after = (last.get("product_id"), last.get("location_id"));
        for row in &rows {
            let point = planning_point(row);
            writer
                .write_record([
                    run_label.to_string(),
                    row.get::<String, _>("product_sku"),
                    row.get("product_name"),
                    row.get("location_code"),
                    point.week_start,
                    point.horizon.to_string(),
                    point
                        .baseline
                        .map(|value| value.to_string())
                        .unwrap_or_default(),
                    point
                        .override_value
                        .map(|value| value.to_string())
                        .unwrap_or_default(),
                    point
                        .effective
                        .map(|value| value.to_string())
                        .unwrap_or_default(),
                    point.value_source,
                    row.get("series_status"),
                    row.get::<Option<String>, _>("model_id").unwrap_or_default(),
                ])
                .map_err(|_| {
                    err(
                        "export_failed",
                        "Could not write the CSV rows. Check the destination and retry.",
                    )
                })?;
        }
        row_count += rows.len() as i64;
    }
    writer.flush()?;
    Ok(row_count)
}

/// Identity lines a publication export adds to the XLSX Summary sheet; the
/// export never masquerades as a plain run export (ADR-0029).
struct PublicationStamp {
    id: i64,
    published_at: String,
    cutoff_week: String,
    revision_high_water: i64,
}

async fn write_xlsx_plan(
    connection: &mut SqliteConnection,
    source: PlanSource<'_>,
    run: &Uuid,
    temporary: &mut tempfile::NamedTempFile,
    coverage: &RunCoverage,
    publication: Option<&PublicationStamp>,
) -> AppResult<i64> {
    let mut workbook = Workbook::new();
    let date_format = Format::new().set_num_format("yyyy-mm-dd");
    let mut row_count = 0u32;
    {
        // Flush completed rows to temporary files instead of retaining the plan.
        let sheet = workbook.add_worksheet_with_constant_memory();
        sheet.set_name("Plan")?;
        for (column, header) in EXPORT_HEADERS.iter().enumerate() {
            sheet.write_string(0, column as u16, *header)?;
        }
        let mut after = (Vec::<u8>::new(), Vec::<u8>::new());
        loop {
            let rows = load_planning_rows(connection, source, None, None, (&after.0, &after.1), 32)
                .await?;
            let Some(last) = rows.last() else {
                break;
            };
            after = (last.get("product_id"), last.get("location_id"));
            for row in &rows {
                row_count += 1;
                let point = planning_point(row);
                for (column, text) in [
                    run.to_string(),
                    row.get("product_sku"),
                    row.get("product_name"),
                    row.get("location_code"),
                ]
                .iter()
                .enumerate()
                {
                    // Explicit strings preserve leading zeroes and cannot become formulas.
                    sheet.write_string(row_count, column as u16, text)?;
                }
                let week = NaiveDate::parse_from_str(&point.week_start, "%Y-%m-%d")
                    .map_err(|_| err("export_failed", "The plan contains an invalid week date."))?;
                sheet.write_datetime_with_format(row_count, 4, week, &date_format)?;
                sheet.write_number(row_count, 5, point.horizon)?;
                for (column, value) in [
                    (6, point.baseline),
                    (7, point.override_value),
                    (8, point.effective),
                ] {
                    if let Some(value) = value {
                        sheet.write_number(row_count, column, value)?;
                    }
                }
                sheet.write_string(row_count, 9, &point.value_source)?;
                sheet.write_string(row_count, 10, row.get::<String, _>("series_status"))?;
                if let Some(model) = row.get::<Option<String>, _>("model_id") {
                    sheet.write_string(row_count, 11, model)?;
                }
            }
        }
    }
    let metadata = sqlx::query(
        "SELECT revision_id, horizon_start_week, horizon_weeks, engine_version FROM forecast_run WHERE id = ?",
    ).bind(run.as_bytes().as_slice()).fetch_one(&mut *connection).await?;
    let champions: BTreeMap<String, i64> = sqlx::query_as::<_, (String, i64)>(
        "SELECT model_id, COUNT(*) FROM series_model_evaluation WHERE run_id = ? AND is_champion = 1 GROUP BY model_id",
    ).bind(run.as_bytes().as_slice()).fetch_all(&mut *connection).await?.into_iter().collect();
    let summary = workbook.add_worksheet();
    summary.set_name("Summary")?;
    summary.write_string(0, 0, "Metric")?;
    summary.write_string(0, 1, "Value")?;
    for (row, label, value) in [
        (1, "run_id", run.to_string()),
        (3, "horizon_start_week", metadata.get("horizon_start_week")),
        (5, "engine_version", metadata.get("engine_version")),
    ] {
        summary.write_string(row, 0, label)?;
        summary.write_string(row, 1, value)?;
    }
    for (row, label, value) in [
        (
            2,
            "revision_id",
            metadata.get::<i64, _>("revision_id") as f64,
        ),
        (
            4,
            "horizon_weeks",
            metadata.get::<i64, _>("horizon_weeks") as f64,
        ),
        (6, "series_count", f64::from(coverage.series_count)),
        (7, "covered_series", f64::from(coverage.covered_series)),
        (8, "coverage", coverage.coverage),
    ] {
        summary.write_string(row, 0, label)?;
        summary.write_number(row, 1, value)?;
    }
    for (index, status) in [
        "ok",
        "fallback_naive",
        "insufficient_history",
        "model_failure",
        "invalid_data",
    ]
    .iter()
    .enumerate()
    {
        let row = index as u32 + 9;
        summary.write_string(row, 0, format!("outcome.{status}"))?;
        summary.write_number(row, 1, coverage.outcomes.get(*status).copied().unwrap_or(0))?;
    }
    for (index, model) in ["naive", "seasonal_naive_52", "auto_ets_52"]
        .iter()
        .enumerate()
    {
        let row = index as u32 + 14;
        summary.write_string(row, 0, format!("statistical_champion.{model}"))?;
        summary.write_number(row, 1, champions.get(*model).copied().unwrap_or(0) as f64)?;
    }
    summary.write_string(17, 0, "coverage_definition")?;
    summary.write_string(
        17,
        1,
        "Share of series with ok or fallback_naive outcomes; not forecast accuracy.",
    )?;
    if let Some(stamp) = publication {
        summary.write_string(18, 0, "publication_id")?;
        summary.write_number(18, 1, stamp.id as f64)?;
        summary.write_string(19, 0, "published_at")?;
        summary.write_string(19, 1, &stamp.published_at)?;
        summary.write_string(20, 0, "publication_cutoff_week")?;
        summary.write_string(20, 1, &stamp.cutoff_week)?;
        summary.write_string(21, 0, "publication_revision_high_water")?;
        summary.write_number(21, 1, stamp.revision_high_water as f64)?;
        summary.write_string(22, 0, "publication_note")?;
        summary.write_string(
            22,
            1,
            "Values are pinned at publication time; later run edits do not change them (ADR-0029).",
        )?;
    }
    workbook.save_to_writer(temporary.as_file_mut())?;
    Ok(i64::from(row_count))
}

async fn run_coverage(connection: &mut SqliteConnection, run: &[u8]) -> AppResult<RunCoverage> {
    let outcomes: BTreeMap<String, u32> = sqlx::query_as::<_, (String, i64)>(
        "SELECT status, COUNT(*) FROM forecast_series_status WHERE run_id = ? GROUP BY status",
    )
    .bind(run)
    .fetch_all(connection)
    .await?
    .into_iter()
    .map(|(status, count)| (status, count as u32))
    .collect();
    let series_count = outcomes.values().sum::<u32>();
    let covered_series = outcomes.get("ok").copied().unwrap_or(0)
        + outcomes.get("fallback_naive").copied().unwrap_or(0);
    Ok(RunCoverage {
        series_count,
        covered_series,
        coverage: f64::from(covered_series) / f64::from(series_count.max(1)),
        outcomes,
    })
}

/// Where a batch of plan rows comes from: the run's live saved plan or the
/// values pinned by one publication (ADR-0029).
#[derive(Clone, Copy)]
enum PlanSource<'a> {
    Run(&'a [u8]),
    Publication { run: &'a [u8], publication_id: i64 },
}

async fn load_planning_rows(
    connection: &mut SqliteConnection,
    source: PlanSource<'_>,
    product: Option<&[u8]>,
    location: Option<&[u8]>,
    after: (&[u8], &[u8]),
    limit: i64,
) -> AppResult<Vec<SqliteRow>> {
    let rows = match source {
        PlanSource::Run(run) => {
            sqlx::query(PLANNING_ROWS_SQL)
                .bind(run)
                .bind(product)
                .bind(product)
                .bind(location)
                .bind(location)
                .bind(after.0)
                .bind(after.1)
                .bind(limit)
                .fetch_all(&mut *connection)
                .await?
        }
        PlanSource::Publication {
            run,
            publication_id,
        } => {
            sqlx::query(PUBLICATION_ROWS_SQL)
                .bind(run)
                .bind(product)
                .bind(product)
                .bind(location)
                .bind(location)
                .bind(after.0)
                .bind(after.1)
                .bind(limit)
                .bind(publication_id)
                .fetch_all(&mut *connection)
                .await?
        }
    };
    Ok(rows)
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
        override_reason: row.get("override_reason"),
        override_note: row.get("override_note"),
        override_review_date: row.get("override_review_date"),
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

/// The stored schema version, or `None` when the database is not a versioned
/// workspace (no settings table or no settings row).
async fn schema_version(pool: &SqlitePool) -> AppResult<Option<i64>> {
    let version: Option<String> =
        match sqlx::query_scalar("SELECT schema_version FROM workspace_settings WHERE id = 1")
            .fetch_optional(pool)
            .await
        {
            Ok(value) => value,
            Err(_) => return Ok(None),
        };
    Ok(version.and_then(|value| value.parse().ok()))
}

/// Migrate an older workspace to this build: verified pre-migration backup
/// first, then the pending migrations, then the settings version bump. A
/// workspace is never migrated without the backup (ADR-0008).
async fn upgrade_schema(root: &Path, pool: &SqlitePool, from: i64) -> AppResult<()> {
    let backup = backup_before_migration(root, pool, from, SCHEMA_VERSION).await?;
    MIGRATOR.run(pool).await.map_err(|error| {
        err(
            "migration_failed",
            format!(
                "The workspace schema upgrade from version {from} failed: {error}. The verified backup is kept at {}.",
                backup.display()
            ),
        )
    })?;
    sqlx::query("UPDATE workspace_settings SET schema_version = ?, updated_at = ? WHERE id = 1")
        .bind(SCHEMA_VERSION_TEXT)
        .bind(now_iso())
        .execute(pool)
        .await?;
    Ok(())
}

/// Create and verify the pre-migration backup pair (database plus manifest).
///
/// The snapshot uses `VACUUM INTO` under the single-writer invariant: no job
/// runs while a workspace is being opened. Verification opens the copy
/// read-only, runs `integrity_check`, and confirms the schema version, so a
/// backup that cannot be validated aborts the migration.
async fn backup_before_migration(
    root: &Path,
    pool: &SqlitePool,
    from: i64,
    to: i64,
) -> AppResult<PathBuf> {
    let directory = root.join(BACKUP_DIRECTORY);
    std::fs::create_dir_all(&directory).map_err(|error| {
        err(
            "backup_failed",
            format!(
                "The pre-migration backup directory could not be prepared ({error}). The workspace was not migrated."
            ),
        )
    })?;
    let stamp = Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let file_name = format!("pre-migration-v{from}-to-v{to}-{stamp}.sqlite3");
    let target = directory.join(&file_name);
    // The destination is a bound expression, so the statement stays static.
    sqlx::query("VACUUM INTO ?")
        .bind(target.display().to_string())
        .execute(pool)
        .await
        .map_err(|error| {
            err(
                "backup_failed",
                format!("The pre-migration backup could not be created ({error}). The workspace was not migrated."),
            )
        })?;

    verify_backup_file(&target, from).await?;

    let bytes = std::fs::read(&target)?;
    let sha256 = hex(&Sha256::digest(&bytes));
    let manifest = serde_json::json!({
        "scope": "database",
        "file": file_name,
        "sha256": sha256,
        "byte_size": bytes.len(),
        "schema_version_before": from.to_string(),
        "schema_version_after": to.to_string(),
        "app_version": env!("CARGO_PKG_VERSION"),
        "created_at": now_iso(),
    });
    std::fs::write(
        directory.join(format!("{file_name}.manifest.json")),
        serde_json::to_string_pretty(&manifest)?,
    )?;
    fsync_dir(&directory);
    prune_pre_migration_backups(&directory);
    Ok(target)
}

/// Open a backup read-only and prove it is intact and at the expected version.
async fn verify_backup_file(target: &Path, expected_version: i64) -> AppResult<()> {
    let options = SqliteConnectOptions::new()
        .filename(target)
        .read_only(true)
        .busy_timeout(Duration::from_millis(5_000));
    let copy = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .map_err(|error| {
            err(
                "backup_failed",
                format!("The pre-migration backup could not be opened for verification ({error}). The workspace was not migrated."),
            )
        })?;
    let integrity: String = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_one(&copy)
        .await
        .map_err(|error| {
            err(
                "backup_failed",
                format!("The pre-migration backup failed its integrity check ({error}). The workspace was not migrated."),
            )
        })?;
    let version: Option<String> =
        sqlx::query_scalar("SELECT schema_version FROM workspace_settings WHERE id = 1")
            .fetch_optional(&copy)
            .await
            .map_err(|error| {
                err(
                    "backup_failed",
                    format!("The pre-migration backup could not be read back ({error}). The workspace was not migrated."),
                )
            })?;
    copy.close().await;
    if integrity != "ok" {
        return Err(err(
            "backup_failed",
            "The pre-migration backup failed its integrity check. The workspace was not migrated.",
        ));
    }
    if version.as_deref() != Some(expected_version.to_string().as_str()) {
        return Err(err(
            "backup_failed",
            format!("The pre-migration backup is not at schema version {expected_version}. The workspace was not migrated."),
        ));
    }
    Ok(())
}

/// Keep only the newest pre-migration backup pairs; pruning covers backup
/// files only and never touches workspace data (ADR-0009).
fn prune_pre_migration_backups(directory: &Path) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.starts_with("pre-migration-") && name.ends_with(".sqlite3"))
        .collect();
    names.sort();
    if names.len() <= PRE_MIGRATION_BACKUPS_KEPT {
        return;
    }
    for name in &names[..names.len() - PRE_MIGRATION_BACKUPS_KEPT] {
        let _ = std::fs::remove_file(directory.join(name));
        let _ = std::fs::remove_file(directory.join(format!("{name}.manifest.json")));
    }
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

    for migration in MIGRATOR.iter() {
        let stored: Option<Vec<u8>> =
            sqlx::query_scalar("SELECT checksum FROM _sqlx_migrations WHERE version = ?")
                .bind(migration.version)
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
                format!(
                    "The workspace is missing its migration {:03} record. Restore from a verified backup before opening.",
                    migration.version
                ),
            )
        })?;
        if stored != migration.checksum.to_vec() {
            return Err(err(
                "needs_verified_backup",
                "The workspace schema fingerprint does not match this application build. Restore from a verified backup before opening.",
            ));
        }
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

pub(crate) struct ParsedImport {
    pub(crate) row_count: usize,
    pub(crate) cutoff: NaiveDate,
    pub(crate) series_keys: BTreeSet<String>,
    /// Included, aggregated numeric cells keyed by series then week.
    pub(crate) series: BTreeMap<String, BTreeMap<NaiveDate, (f64, &'static str)>>,
    pub(crate) issues: Vec<ParsedIssue>,
}

pub(crate) struct ParsedIssue {
    pub(crate) severity: &'static str,
    pub(crate) code: &'static str,
    pub(crate) detail: String,
    pub(crate) row_ref: Option<String>,
}

/// How one parsed import is committed: the contract version that labels it, the
/// descriptor that participates in the fingerprint (if any), and the retained
/// file name.
struct CommitSpec<'a> {
    contract_version: &'a str,
    descriptor: Option<&'a ParsingDescriptor>,
    retained_name: &'a str,
}

fn source_filename(source: &Path) -> String {
    source
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| SOURCE_FILENAME.to_string())
}

async fn active_profile_id(pool: &SqlitePool) -> AppResult<i64> {
    sqlx::query_scalar("SELECT id FROM source_profile WHERE is_active = 1")
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| {
            err(
                "no_active_profile",
                "The workspace has no active source profile.",
            )
        })
}

fn import_config_from_row(row: &SqliteRow) -> AppResult<ImportConfig> {
    let id: i64 = row.get("id");
    if !(1..=i32::MAX as i64).contains(&id) {
        return Err(err(
            "contract_violation",
            "A saved configuration has an unsupported id.",
        ));
    }
    let name: String = row.get("name");
    let json: String = row.get("descriptor_json");
    let descriptor: ParsingDescriptor = serde_json::from_str(&json).map_err(|_| {
        err(
            "contract_violation",
            format!("Saved configuration '{name}' is unreadable."),
        )
    })?;
    Ok(ImportConfig {
        id: id as i32,
        name,
        descriptor,
        updated_at: row.get("updated_at"),
    })
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

pub(crate) fn classify(quantity: f64) -> &'static str {
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
    descriptor: Option<&ParsingDescriptor>,
) -> AppResult<String> {
    let mut lines: BTreeSet<String> = BTreeSet::new();
    lines.insert(format!("contract={contract}"));
    if let Some(descriptor) = descriptor {
        let json = descriptor_json(descriptor).map_err(|_| {
            err(
                "contract_violation",
                "The parsing descriptor could not be serialized.",
            )
        })?;
        lines.insert(format!(
            "descriptor_sha256={}",
            hex(&Sha256::digest(json.as_bytes()))
        ));
    }
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
    Ok(hex(&Sha256::digest(joined.as_bytes())))
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

// ---------------------------------------------------------------------------
// Planner decision memory (ADR-0027)
// ---------------------------------------------------------------------------

/// Raw `(product_id, location_id)` bytes; the series identity in bulk lookups.
type SeriesKey = (Vec<u8>, Vec<u8>);

async fn load_series_notes(
    pool: &SqlitePool,
    profile_id: i64,
    product: &[u8],
    location: &[u8],
) -> AppResult<Vec<SeriesNote>> {
    let rows = sqlx::query(
        "SELECT id, note, context_run_id, created_at FROM series_note
         WHERE profile_id = ? AND product_id = ? AND location_id = ?
         ORDER BY id DESC",
    )
    .bind(profile_id)
    .bind(product)
    .bind(location)
    .fetch_all(pool)
    .await?;
    let mut notes = Vec::with_capacity(rows.len());
    for row in &rows {
        let id: i64 = row.get("id");
        let id = i32::try_from(id)
            .map_err(|_| err("contract_violation", "A stored note id is unsupported."))?;
        notes.push(SeriesNote {
            id,
            note: row.get("note"),
            context_run_id: match row.get::<Option<Vec<u8>>, _>("context_run_id") {
                Some(bytes) => Some(uuid_from_bytes(&bytes)?.to_string()),
                None => None,
            },
            created_at: row.get("created_at"),
        });
    }
    Ok(notes)
}

/// Resolve the write scope for one decision-memory command: the run must exist
/// and be completed, and the series must be part of it. Returns the profile id
/// the note or review event is stored under.
async fn decision_scope(
    transaction: &mut SqliteConnection,
    run: &Uuid,
    product: &[u8],
    location: &[u8],
) -> AppResult<i64> {
    let row = sqlx::query(
        "SELECT r.status AS status, i.profile_id AS profile_id FROM forecast_run r
         JOIN import_revision i ON i.id = r.revision_id WHERE r.id = ?",
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
            "Only completed forecast runs can be reviewed.",
        ));
    }
    let scoped: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM forecast_series_status
         WHERE run_id = ? AND product_id = ? AND location_id = ?)",
    )
    .bind(run.as_bytes().as_slice())
    .bind(product)
    .bind(location)
    .fetch_one(&mut *transaction)
    .await?;
    if !scoped {
        return Err(err(
            "unknown_series",
            "The selected series is not part of this forecast run.",
        ));
    }
    Ok(row.get("profile_id"))
}

/// Deterministic fingerprint over the evidence one review acknowledgement
/// covers (ADR-0027): the run's stored outcome and reason for the series plus
/// the cell override revision map (`week:revision`, sorted). Later override
/// revisions change the map and reopen the series at read time. The leading
/// version marker is the rule context; bump it when the covered evidence
/// changes.
fn fingerprint_from_parts(
    run: &[u8],
    product: &[u8],
    location: &[u8],
    status: Option<(&str, Option<&str>)>,
    overrides: &[(String, i64)],
    bias: Option<(f64, u32, u32)>,
) -> String {
    let mut lines = vec![
        format!("{REVIEW_CONTEXT}|v1"),
        hex(run),
        hex(product),
        hex(location),
        match status {
            Some((outcome, reason)) => format!("{outcome}|{}", reason.unwrap_or_default()),
            None => "no-status".to_string(),
        },
        match bias {
            Some((value, weeks, horizon)) => format!("bias:{horizon}:{value:.4}:{weeks}"),
            None => "bias:none".to_string(),
        },
    ];
    for (week, revision) in overrides {
        lines.push(format!("{week}:{revision}"));
    }
    hex(&Sha256::digest(lines.join("\n").as_bytes()))
}

async fn evidence_fingerprint(
    connection: &mut SqliteConnection,
    run: &[u8],
    product: &[u8],
    location: &[u8],
) -> AppResult<String> {
    let status = sqlx::query(
        "SELECT status, reason FROM forecast_series_status
         WHERE run_id = ? AND product_id = ? AND location_id = ?",
    )
    .bind(run)
    .bind(product)
    .bind(location)
    .fetch_optional(&mut *connection)
    .await?;
    let status = status.as_ref().map(|row| {
        (
            row.get::<String, _>("status"),
            row.get::<Option<String>, _>("reason"),
        )
    });
    let overrides = sqlx::query(
        "SELECT week_start, MAX(revision) AS revision FROM forecast_override
         WHERE run_id = ? AND product_id = ? AND location_id = ?
         GROUP BY week_start ORDER BY week_start",
    )
    .bind(run)
    .bind(product)
    .bind(location)
    .fetch_all(&mut *connection)
    .await?;
    let overrides: Vec<(String, i64)> = overrides
        .iter()
        .map(|row| (row.get("week_start"), row.get("revision")))
        .collect();
    let profile_id: Option<i64> = sqlx::query_scalar(
        "SELECT i.profile_id FROM forecast_run r JOIN import_revision i ON i.id = r.revision_id
         WHERE r.id = ?",
    )
    .bind(run)
    .fetch_optional(&mut *connection)
    .await?;
    let bias = match profile_id {
        Some(profile_id) => {
            persistent_bias_for_series(connection, profile_id, product, location).await?
        }
        None => None,
    };
    Ok(fingerprint_from_parts(
        run,
        product,
        location,
        status
            .as_ref()
            .map(|(outcome, reason)| (outcome.as_str(), reason.as_deref())),
        &overrides,
        bias,
    ))
}

/// Derived review state for one series in one run. A stored acknowledgement
/// whose fingerprint no longer matches the run's current evidence reads as
/// `unreviewed`; the stored event is never rewritten (ADR-0027).
async fn review_state(
    connection: &mut SqliteConnection,
    profile_id: i64,
    run: &[u8],
    product: &[u8],
    location: &[u8],
) -> AppResult<SeriesReviewInfo> {
    let latest = sqlx::query(
        "SELECT action, evidence_fingerprint, created_at FROM series_review
         WHERE profile_id = ? AND product_id = ? AND location_id = ? AND run_id = ?
         ORDER BY id DESC LIMIT 1",
    )
    .bind(profile_id)
    .bind(product)
    .bind(location)
    .bind(run)
    .fetch_optional(&mut *connection)
    .await?;
    let unreviewed = SeriesReviewInfo {
        status: "unreviewed".into(),
        reviewed_at: None,
    };
    let Some(latest) = latest else {
        return Ok(unreviewed);
    };
    let action: String = latest.get("action");
    if action != "reviewed" {
        return Ok(unreviewed);
    }
    let fingerprint: String = latest.get("evidence_fingerprint");
    let current = evidence_fingerprint(connection, run, product, location).await?;
    if current != fingerprint {
        return Ok(unreviewed);
    }
    Ok(SeriesReviewInfo {
        status: "reviewed".into(),
        reviewed_at: Some(latest.get("created_at")),
    })
}

fn validate_note(text: &str) -> AppResult<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(err("invalid_note", "Write a note before saving it."));
    }
    if trimmed.chars().count() > MAX_NOTE_CHARS {
        return Err(err(
            "invalid_note",
            format!("Notes are limited to {MAX_NOTE_CHARS} characters."),
        ));
    }
    if trimmed
        .chars()
        .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        return Err(err(
            "invalid_note",
            "The note contains unsupported control characters.",
        ));
    }
    Ok(trimmed.to_string())
}

fn validate_optional_note(text: Option<&str>) -> AppResult<Option<String>> {
    match text.map(str::trim).filter(|text| !text.is_empty()) {
        Some(text) => validate_note(text).map(Some),
        None => Ok(None),
    }
}

fn validate_override_reason(code: &str) -> AppResult<&'static str> {
    let code = code.trim();
    OVERRIDE_REASONS
        .iter()
        .find(|reason| **reason == code)
        .copied()
        .ok_or_else(|| {
            err(
                "invalid_reason",
                "Choose one of the documented override reasons.",
            )
        })
}

fn validate_optional_review_date(text: Option<&str>) -> AppResult<Option<String>> {
    match text.map(str::trim).filter(|text| !text.is_empty()) {
        Some(text) => {
            let valid = NaiveDate::parse_from_str(text, "%Y-%m-%d")
                .map(|date| date.to_string() == text)
                .unwrap_or(false);
            if !valid {
                return Err(err(
                    "invalid_review_date",
                    "The review date must be an ISO date (YYYY-MM-DD).",
                ));
            }
            Ok(Some(text.to_string()))
        }
        None => Ok(None),
    }
}

fn parse_threshold(text: Option<&str>, label: &str) -> AppResult<Option<f64>> {
    match text.map(str::trim).filter(|text| !text.is_empty()) {
        Some(text) => {
            let value = text
                .parse::<f64>()
                .ok()
                .filter(|value| value.is_finite() && *value > 0.0)
                .ok_or_else(|| {
                    err(
                        "invalid_policy",
                        format!("The {label} threshold must be a positive number."),
                    )
                })?;
            Ok(Some(value))
        }
        None => Ok(None),
    }
}

fn parse_bias_horizon(text: Option<&str>) -> AppResult<Option<u32>> {
    match text.map(str::trim).filter(|text| !text.is_empty()) {
        Some(text) => {
            let value = text
                .parse::<u32>()
                .ok()
                .filter(|value| (1..=12).contains(value))
                .ok_or_else(|| {
                    err(
                        "invalid_policy",
                        "The bias horizon must be a whole number from 1 to 12.",
                    )
                })?;
            Ok(Some(value))
        }
        None => Ok(None),
    }
}

fn parse_bias_count(text: Option<&str>, label: &str) -> AppResult<Option<u32>> {
    match text.map(str::trim).filter(|text| !text.is_empty()) {
        Some(text) => {
            let value = text
                .parse::<u32>()
                .ok()
                .filter(|value| (2..=52).contains(value))
                .ok_or_else(|| {
                    err(
                        "invalid_policy",
                        format!("The bias {label} must be a whole number from 2 to 52."),
                    )
                })?;
            Ok(Some(value))
        }
        None => Ok(None),
    }
}

fn parse_confirmed_week(text: &str) -> AppResult<NaiveDate> {
    NaiveDate::parse_from_str(text, "%Y-%m-%d")
        .ok()
        .filter(|week| week.weekday() == Weekday::Mon && week.to_string() == text)
        .ok_or_else(|| err("invalid_week", "Choose a Monday in YYYY-MM-DD format."))
}

/// Horizon week for a 1-based step: `start + (horizon - 1)` weeks.
fn add_weeks_str(start: &str, weeks: u32) -> AppResult<Option<String>> {
    match NaiveDate::parse_from_str(start, "%Y-%m-%d") {
        Ok(date) => Ok(Some(
            (date + ChronoDuration::weeks(i64::from(weeks) - 1))
                .format("%Y-%m-%d")
                .to_string(),
        )),
        Err(_) => Ok(None),
    }
}

fn confirmation_from_row(row: &SqliteRow) -> AppResult<ActualsConfirmation> {
    Ok(ActualsConfirmation {
        id: i32::try_from(row.get::<i64, _>("id"))
            .map_err(|_| err("contract_violation", "The confirmation id is unsupported."))?,
        revision_id: i32::try_from(row.get::<i64, _>("revision_id"))
            .map_err(|_| err("contract_violation", "The revision id is unsupported."))?,
        confirmed_through: row.get("confirmed_through"),
        created_at: row.get("created_at"),
    })
}

async fn latest_confirmation(
    connection: &mut SqliteConnection,
    profile_id: i64,
) -> AppResult<Option<ActualsConfirmation>> {
    let row = sqlx::query(
        "SELECT id, revision_id, confirmed_through, created_at FROM actuals_confirmation
         WHERE profile_id = ? ORDER BY id DESC LIMIT 1",
    )
    .bind(profile_id)
    .fetch_optional(&mut *connection)
    .await?;
    row.as_ref().map(confirmation_from_row).transpose()
}

// ---------------------------------------------------------------------------
// Retrospective evaluation and persistent-bias evidence (ADR-0030)
// ---------------------------------------------------------------------------

impl FvaExclusions {
    fn bump(&mut self, code: &str) {
        match code {
            "future" => self.future += 1,
            "late_published" => self.late_published += 1,
            "hindsight" => self.hindsight += 1,
            "absent" => self.absent += 1,
            "missing_or_partial" => self.missing_or_partial += 1,
            "stockout" => self.stockout += 1,
            "no_forecast" => self.no_forecast += 1,
            _ => {}
        }
    }
}

/// Publication windows examined for persistent-bias evidence: the most recent
/// published cutoffs of the profile, newest first.
async fn recent_bias_publications(
    connection: &mut SqliteConnection,
    profile_id: i64,
    window: u32,
) -> AppResult<Vec<SqliteRow>> {
    Ok(sqlx::query(
        "SELECT id, run_id, revision_high_water, horizon_start_week, published_at
         FROM plan_publication WHERE profile_id = ?
         ORDER BY published_at DESC, id DESC LIMIT ?",
    )
    .bind(profile_id)
    .bind(i64::from(window))
    .fetch_all(&mut *connection)
    .await?)
}

/// One mature target point of one publication for a single series: the pinned
/// published values and the confirmed actual, or `None` when not scoreable.
async fn published_week_values_for_series(
    connection: &mut SqliteConnection,
    run: &[u8],
    publication_id: i64,
    week: &str,
    product: &[u8],
    location: &[u8],
) -> AppResult<Option<(Option<f64>, Option<f64>)>> {
    let row = sqlx::query(
        "SELECT f.value AS baseline, COALESCE(o.value, f.value) AS effective
         FROM forecast_series_status s
         LEFT JOIN forecast f ON f.run_id = s.run_id AND f.product_id = s.product_id
             AND f.location_id = s.location_id AND f.week_start = ?
         LEFT JOIN publication_override po ON po.publication_id = ?
             AND po.product_id = s.product_id AND po.location_id = s.location_id
             AND po.week_start = ?
         LEFT JOIN forecast_override o ON o.run_id = s.run_id AND o.product_id = s.product_id
             AND o.location_id = s.location_id AND o.week_start = ? AND o.revision = po.revision
         WHERE s.run_id = ? AND s.product_id = ? AND s.location_id = ?",
    )
    .bind(week)
    .bind(publication_id)
    .bind(week)
    .bind(week)
    .bind(run)
    .bind(product)
    .bind(location)
    .fetch_optional(&mut *connection)
    .await?;
    Ok(row.map(|row| (row.get("baseline"), row.get("effective"))))
}

/// Numeric actual of one target week in a specific revision, with its status;
/// `None` when the row is absent or has no numeric quantity.
async fn actual_value_for_series(
    connection: &mut SqliteConnection,
    revision_id: i32,
    week: &str,
    product: &[u8],
    location: &[u8],
) -> AppResult<Option<(f64, String)>> {
    let row = sqlx::query(
        "SELECT quantity, status FROM observation
         WHERE revision_id = ? AND product_id = ? AND location_id = ? AND week_start = ?",
    )
    .bind(revision_id)
    .bind(product)
    .bind(location)
    .bind(week)
    .fetch_optional(&mut *connection)
    .await?;
    Ok(row.and_then(|row| {
        row.get::<Option<f64>, _>("quantity")
            .map(|quantity| (quantity, row.get("status")))
    }))
}

/// True when the target actual existed in any profile revision at or below the
/// high-water mark: the planner could have seen it, so it must not be scored.
async fn actual_visible_before(
    connection: &mut SqliteConnection,
    profile_id: i64,
    high_water: i64,
    week: &str,
    product: &[u8],
    location: &[u8],
) -> AppResult<bool> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM observation o JOIN import_revision i ON i.id = o.revision_id
         WHERE i.profile_id = ? AND i.id <= ? AND o.product_id = ? AND o.location_id = ?
           AND o.week_start = ?)",
    )
    .bind(profile_id)
    .bind(high_water)
    .bind(product)
    .bind(location)
    .bind(week)
    .fetch_one(&mut *connection)
    .await?)
}

/// Persistent-bias evidence for one series (ADR-0030): every mature target
/// point in the configured window must share one nonzero error sign and the
/// pooled bias magnitude must exceed the threshold, or the rule stays silent.
async fn persistent_bias_for_series(
    connection: &mut SqliteConnection,
    profile_id: i64,
    product: &[u8],
    location: &[u8],
) -> AppResult<Option<(f64, u32, u32)>> {
    let policy = review_policy_row(connection, profile_id).await?;
    let (Some(horizon), Some(window), Some(minimum), Some(threshold)) = (
        policy.bias_horizon,
        policy.bias_window,
        policy.bias_min,
        policy.bias_abs,
    ) else {
        return Ok(None);
    };
    let Some(confirmation) = latest_confirmation(connection, profile_id).await? else {
        return Ok(None);
    };
    let publications = recent_bias_publications(connection, profile_id, window).await?;
    let mut sum_error = 0.0;
    let mut sum_abs_actual = 0.0;
    let mut count = 0u32;
    let mut sign: Option<bool> = None;
    let mut seen_weeks: HashSet<String> = HashSet::new();
    for row in &publications {
        let high_water: i64 = row.get("revision_high_water");
        if i64::from(confirmation.revision_id) <= high_water {
            continue;
        }
        let horizon_start_week: String = row.get("horizon_start_week");
        let Some(target) = add_weeks_str(&horizon_start_week, horizon)? else {
            continue;
        };
        if target.as_str() > confirmation.confirmed_through.as_str() {
            continue;
        }
        let published_at: String = row.get("published_at");
        if published_at.get(..10).unwrap_or("9999-12-31") >= target.as_str() {
            continue;
        }
        if !seen_weeks.insert(target.clone()) {
            continue;
        }
        let run_bytes: Vec<u8> = row.get("run_id");
        let publication_id: i64 = row.get("id");
        let Some((baseline, effective)) = published_week_values_for_series(
            connection,
            &run_bytes,
            publication_id,
            &target,
            product,
            location,
        )
        .await?
        else {
            continue;
        };
        let (Some(_baseline), Some(effective)) = (baseline, effective) else {
            continue;
        };
        if actual_visible_before(
            connection, profile_id, high_water, &target, product, location,
        )
        .await?
        {
            continue;
        }
        let Some((actual, status)) = actual_value_for_series(
            connection,
            confirmation.revision_id,
            &target,
            product,
            location,
        )
        .await?
        else {
            continue;
        };
        if !matches!(status.as_str(), "known" | "zero" | "return" | "correction") {
            continue;
        }
        let error = effective - actual;
        if error == 0.0 {
            return Ok(None);
        }
        let positive = error > 0.0;
        match sign {
            None => sign = Some(positive),
            Some(previous) if previous != positive => return Ok(None),
            _ => {}
        }
        sum_error += error;
        sum_abs_actual += actual.abs();
        count += 1;
    }
    if count < minimum || sum_abs_actual <= 0.0 {
        return Ok(None);
    }
    let bias = sum_error / sum_abs_actual;
    if bias.abs() < threshold {
        return Ok(None);
    }
    Ok(Some((bias, count, horizon)))
}

/// Bulk persistent-bias evidence for every series of the profile, computed
/// with bounded queries: three bulk reads per mature publication window.
async fn persistent_bias_map(
    connection: &mut SqliteConnection,
    profile_id: i64,
) -> AppResult<HashMap<SeriesKey, (f64, u32, u32)>> {
    let policy = review_policy_row(connection, profile_id).await?;
    let (Some(horizon), Some(window), Some(minimum), Some(threshold)) = (
        policy.bias_horizon,
        policy.bias_window,
        policy.bias_min,
        policy.bias_abs,
    ) else {
        return Ok(HashMap::new());
    };
    let Some(confirmation) = latest_confirmation(connection, profile_id).await? else {
        return Ok(HashMap::new());
    };
    let publications = recent_bias_publications(connection, profile_id, window).await?;
    #[derive(Default)]
    struct BiasAcc {
        sum_error: f64,
        sum_abs_actual: f64,
        count: u32,
        sign: Option<bool>,
        broken: bool,
    }
    let mut acc: HashMap<SeriesKey, BiasAcc> = HashMap::new();
    let mut seen_weeks: HashSet<(SeriesKey, String)> = HashSet::new();
    for row in &publications {
        let high_water: i64 = row.get("revision_high_water");
        if i64::from(confirmation.revision_id) <= high_water {
            continue;
        }
        let horizon_start_week: String = row.get("horizon_start_week");
        let Some(target) = add_weeks_str(&horizon_start_week, horizon)? else {
            continue;
        };
        let published_at: String = row.get("published_at");
        if published_at.get(..10).unwrap_or("9999-12-31") >= target.as_str() {
            continue;
        }
        if target.as_str() > confirmation.confirmed_through.as_str() {
            continue;
        }
        let run_bytes: Vec<u8> = row.get("run_id");
        let publication_id: i64 = row.get("id");
        let week_rows = sqlx::query(PUBLICATION_WEEK_ROWS_SQL)
            .bind(run_bytes.as_slice())
            .bind(publication_id)
            .bind(target.as_str())
            .fetch_all(&mut *connection)
            .await?;
        let actual_rows = sqlx::query(
            "SELECT product_id, location_id, quantity, status FROM observation
             WHERE revision_id = ? AND week_start = ?",
        )
        .bind(confirmation.revision_id)
        .bind(target.as_str())
        .fetch_all(&mut *connection)
        .await?;
        let mut actuals: HashMap<SeriesKey, (Option<f64>, String)> = HashMap::new();
        for row in &actual_rows {
            actuals.insert(
                (row.get("product_id"), row.get("location_id")),
                (row.get("quantity"), row.get("status")),
            );
        }
        let visible_rows = sqlx::query(
            "SELECT o.product_id AS product_id, o.location_id AS location_id
             FROM observation o JOIN import_revision i ON i.id = o.revision_id
             WHERE i.profile_id = ? AND i.id <= ? AND o.week_start = ?
             GROUP BY o.product_id, o.location_id",
        )
        .bind(profile_id)
        .bind(high_water)
        .bind(target.as_str())
        .fetch_all(&mut *connection)
        .await?;
        let mut hindsight: HashSet<SeriesKey> = HashSet::new();
        for row in &visible_rows {
            hindsight.insert((row.get("product_id"), row.get("location_id")));
        }
        for row in &week_rows {
            let key: SeriesKey = (row.get("product_id"), row.get("location_id"));
            let baseline: Option<f64> = row.get("baseline");
            let effective: Option<f64> = row.get("effective");
            if baseline.is_none() || effective.is_none() || hindsight.contains(&key) {
                continue;
            }
            let Some((quantity, status)) = actuals.get(&key) else {
                continue;
            };
            let Some(actual) = quantity else {
                continue;
            };
            if !matches!(status.as_str(), "known" | "zero" | "return" | "correction") {
                continue;
            }
            if !seen_weeks.insert((key.clone(), target.clone())) {
                continue;
            }
            let entry = acc.entry(key).or_default();
            if entry.broken {
                continue;
            }
            let error = effective.unwrap_or_default() - actual;
            if error == 0.0 {
                entry.broken = true;
                continue;
            }
            let positive = error > 0.0;
            match entry.sign {
                None => entry.sign = Some(positive),
                Some(previous) if previous != positive => {
                    entry.broken = true;
                    continue;
                }
                _ => {}
            }
            entry.sum_error += error;
            entry.sum_abs_actual += actual.abs();
            entry.count += 1;
        }
    }
    let mut evidence = HashMap::new();
    for (key, entry) in acc {
        if entry.broken || entry.count < minimum || entry.sum_abs_actual <= 0.0 {
            continue;
        }
        let bias = entry.sum_error / entry.sum_abs_actual;
        if bias.abs() < threshold {
            continue;
        }
        evidence.insert(key, (bias, entry.count, horizon));
    }
    Ok(evidence)
}

/// Retrospective accumulator for one series across eligible publications, in
/// the series' own units.
#[derive(Default)]
struct SeriesFvaAccumulator {
    series_key: String,
    location_code: String,
    points: u32,
    adjusted: u32,
    imputed_input: bool,
    sum_abs_actual: f64,
    sum_abs_baseline_error: f64,
    sum_abs_effective_error: f64,
    sum_baseline_bias: f64,
    sum_effective_bias: f64,
}

impl SeriesFvaAccumulator {
    fn push_point(&mut self, baseline: f64, effective: f64, actual: f64, imputed: bool) {
        self.points += 1;
        if effective != baseline {
            self.adjusted += 1;
        }
        self.imputed_input |= imputed;
        let baseline_error = baseline - actual;
        let effective_error = effective - actual;
        self.sum_abs_actual += actual.abs();
        self.sum_abs_baseline_error += baseline_error.abs();
        self.sum_abs_effective_error += effective_error.abs();
        self.sum_baseline_bias += baseline_error;
        self.sum_effective_bias += effective_error;
    }

    fn measures(&self) -> FvaSeriesMeasures {
        let points = self.points;
        let denominator = self.sum_abs_actual;
        let (baseline_wape, effective_wape, baseline_bias, effective_bias) = if denominator > 0.0 {
            (
                Some(self.sum_abs_baseline_error / denominator),
                Some(self.sum_abs_effective_error / denominator),
                Some(self.sum_baseline_bias / denominator),
                Some(self.sum_effective_bias / denominator),
            )
        } else {
            (None, None, None, None)
        };
        let baseline_mae = (points > 0).then(|| self.sum_abs_baseline_error / f64::from(points));
        let effective_mae = (points > 0).then(|| self.sum_abs_effective_error / f64::from(points));
        FvaSeriesMeasures {
            points,
            baseline_wape,
            effective_wape,
            wape_improvement_pp: match (baseline_wape, effective_wape) {
                (Some(baseline), Some(effective)) => Some((baseline - effective) * 100.0),
                _ => None,
            },
            baseline_mae,
            effective_mae,
            mae_improvement: match (baseline_mae, effective_mae) {
                (Some(baseline), Some(effective)) => Some(baseline - effective),
                _ => None,
            },
            baseline_bias,
            effective_bias,
        }
    }
}

/// One target point of one publication with its scoring verdict.
struct PublicationPointAcc {
    product: Vec<u8>,
    location: Vec<u8>,
    series_key: String,
    location_code: String,
    imputed_input: bool,
    actual: Option<f64>,
    baseline: Option<f64>,
    effective: Option<f64>,
    excluded: Option<&'static str>,
    week: String,
}

/// Score or exclude every series of one publication at one horizon step
/// (ADR-0030): single explicit verdict per point, newest actuals revision
/// pinned by the confirmation, hindsight rejected against the high-water mark.
#[allow(clippy::too_many_arguments)]
async fn publication_fva_points(
    connection: &mut SqliteConnection,
    profile_id: i64,
    publication_id: i64,
    run: &[u8],
    high_water: i64,
    horizon_start_week: &str,
    published_at: &str,
    confirmation: &ActualsConfirmation,
    horizon: u32,
) -> AppResult<Vec<PublicationPointAcc>> {
    let Some(target) = add_weeks_str(horizon_start_week, horizon)? else {
        return Ok(Vec::new());
    };
    let rows = sqlx::query(PUBLICATION_WEEK_ROWS_SQL)
        .bind(run)
        .bind(publication_id)
        .bind(target.as_str())
        .fetch_all(&mut *connection)
        .await?;
    let late = published_at.get(..10).unwrap_or("9999-12-31") >= target.as_str();
    let future = target.as_str() > confirmation.confirmed_through.as_str();
    let mut actuals: HashMap<SeriesKey, (Option<f64>, String)> = HashMap::new();
    let mut hindsight: HashSet<SeriesKey> = HashSet::new();
    if !late && !future {
        let actual_rows = sqlx::query(
            "SELECT product_id, location_id, quantity, status FROM observation
             WHERE revision_id = ? AND week_start = ?",
        )
        .bind(confirmation.revision_id)
        .bind(target.as_str())
        .fetch_all(&mut *connection)
        .await?;
        for row in &actual_rows {
            actuals.insert(
                (row.get("product_id"), row.get("location_id")),
                (row.get("quantity"), row.get("status")),
            );
        }
        let visible_rows = sqlx::query(
            "SELECT o.product_id AS product_id, o.location_id AS location_id
             FROM observation o JOIN import_revision i ON i.id = o.revision_id
             WHERE i.profile_id = ? AND i.id <= ? AND o.week_start = ?
             GROUP BY o.product_id, o.location_id",
        )
        .bind(profile_id)
        .bind(high_water)
        .bind(target.as_str())
        .fetch_all(&mut *connection)
        .await?;
        for row in &visible_rows {
            hindsight.insert((row.get("product_id"), row.get("location_id")));
        }
    }
    let mut points = Vec::with_capacity(rows.len());
    for row in &rows {
        let product: Vec<u8> = row.get("product_id");
        let location: Vec<u8> = row.get("location_id");
        let key: SeriesKey = (product.clone(), location.clone());
        let baseline: Option<f64> = row.get("baseline");
        let effective: Option<f64> = row.get("effective");
        let excluded = if late {
            Some("late_published")
        } else if future {
            Some("future")
        } else if baseline.is_none() || effective.is_none() {
            Some("no_forecast")
        } else {
            match actuals.get(&key) {
                None => Some("absent"),
                Some((quantity, status)) => {
                    if quantity.is_none() {
                        Some("missing_or_partial")
                    } else if status == "stockout" {
                        Some("stockout")
                    } else if hindsight.contains(&key) {
                        Some("hindsight")
                    } else {
                        None
                    }
                }
            }
        };
        points.push(PublicationPointAcc {
            product,
            location,
            series_key: row.get("series_key"),
            location_code: row.get("location_code"),
            imputed_input: row.get::<i64, _>("imputed_weeks") > 0,
            actual: actuals.get(&key).and_then(|(quantity, _)| *quantity),
            baseline,
            effective,
            excluded,
            week: row.get("week_start"),
        });
    }
    Ok(points)
}

async fn review_policy_row(
    connection: &mut SqliteConnection,
    profile_id: i64,
) -> AppResult<InboxPolicy> {
    let row = sqlx::query(
        "SELECT version, change_abs, change_rel, bias_horizon, bias_window, bias_min, bias_abs
         FROM review_policy WHERE profile_id = ? ORDER BY version DESC LIMIT 1",
    )
    .bind(profile_id)
    .fetch_optional(&mut *connection)
    .await?;
    Ok(match row {
        Some(row) => InboxPolicy {
            change_abs: row.get("change_abs"),
            change_rel: row.get("change_rel"),
            bias_horizon: row
                .get::<Option<i64>, _>("bias_horizon")
                .map(|value| value as u32),
            bias_window: row
                .get::<Option<i64>, _>("bias_window")
                .map(|value| value as u32),
            bias_min: row
                .get::<Option<i64>, _>("bias_min")
                .map(|value| value as u32),
            bias_abs: row.get("bias_abs"),
            version: Some(
                i32::try_from(row.get::<i64, _>("version"))
                    .map_err(|_| err("contract_violation", "The policy version is unsupported."))?,
            ),
        },
        None => InboxPolicy {
            change_abs: None,
            change_rel: None,
            bias_horizon: None,
            bias_window: None,
            bias_min: None,
            bias_abs: None,
            version: None,
        },
    })
}

/// The newest completed run of the immediately preceding import revision of
/// one profile (ADR-0022 adjacency). Never skips an unforecast revision: when
/// that revision has no completed run, no comparison evidence exists.
async fn predecessor_run(
    connection: &mut SqliteConnection,
    profile_id: i64,
    revision_id: i32,
) -> AppResult<Option<(Vec<u8>, String)>> {
    let previous_revision: Option<i64> =
        sqlx::query_scalar("SELECT MAX(id) FROM import_revision WHERE profile_id = ? AND id < ?")
            .bind(profile_id)
            .bind(revision_id)
            .fetch_one(&mut *connection)
            .await?;
    let Some(previous_revision) = previous_revision else {
        return Ok(None);
    };
    let run = sqlx::query(
        "SELECT id FROM forecast_run WHERE revision_id = ? AND status = 'completed'
         ORDER BY created_at DESC, id DESC LIMIT 1",
    )
    .bind(previous_revision)
    .fetch_optional(&mut *connection)
    .await?;
    let Some(row) = run else {
        return Ok(None);
    };
    let bytes: Vec<u8> = row.get("id");
    let id = uuid_from_bytes(&bytes)?.to_string();
    Ok(Some((bytes, id)))
}

fn run_summary_row(row: &SqliteRow) -> AppResult<RunSummary> {
    let id: Vec<u8> = row.get("id");
    Ok(RunSummary {
        id: uuid_from_bytes(&id)?.to_string(),
        revision_id: row.get("revision_id"),
        status: row.get("status"),
        created_at: row.get("created_at"),
        horizon_start_week: row.get("horizon_start_week"),
        engine_version: row.get("engine_version"),
    })
}

fn parse_week(value: &str) -> AppResult<NaiveDate> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|_| err("invalid_data", "A stored observation date is invalid."))
}

fn horizon_end(start: NaiveDate, weeks: i64) -> AppResult<NaiveDate> {
    if weeks < 1 {
        return Err(err(
            "invalid_horizon",
            "The run has an invalid planning horizon.",
        ));
    }
    Ok(start + ChronoDuration::weeks(weeks - 1))
}

async fn load_forecast_rows(
    pool: &SqlitePool,
    run: &[u8],
    product: &[u8],
    location: &[u8],
) -> AppResult<BTreeMap<NaiveDate, f64>> {
    let rows = sqlx::query(
        "SELECT week_start, value FROM forecast WHERE run_id = ? AND product_id = ? AND location_id = ? ORDER BY week_start",
    )
    .bind(run)
    .bind(product)
    .bind(location)
    .fetch_all(pool)
    .await?;
    let mut points = BTreeMap::new();
    for row in &rows {
        let week: String = row.get("week_start");
        points.insert(parse_week(&week)?, row.get("value"));
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

pub(crate) fn cancelled_import() -> AppError {
    err(
        "import_cancelled",
        "The import was cancelled before any revision was committed.",
    )
}

fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}
