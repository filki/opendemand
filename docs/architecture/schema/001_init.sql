-- OpenDemand workspace database: initial schema (migration 001).
--
-- Conventions:
--   * UUIDv7 identifiers are stored as BLOB(16).
--   * week_start is the ISO date (YYYY-MM-DD) of the Monday of the week,
--     in the workspace IANA timezone.
--   * Timestamps are ISO-8601 UTC text.
--   * Connection pragmas (journal_mode=WAL, synchronous=FULL, foreign_keys=ON,
--     busy_timeout=5000) are applied by the Rust core at connection open and are
--     intentionally not part of this migration.
--   * This file is executed as migration 001 by SQLx. After acceptance it is
--     immutable; later changes are additive migrations (002, 003, ...).
--
-- Freeze status: FROZEN 2026-10-13. The SHA-256 fingerprint in
-- docs/architecture/schema/001_init.sha256 covers this file after the Phase 1
-- gates passed (integration tests, empty foreign_key_check, integrity_check ok).
-- Do not edit this migration; change the schema only by adding 002_* migrations.
--
-- Rationale: see docs/architecture/adr/ and docs/architecture/DATA_ARCHITECTURE.md.

-- ---------------------------------------------------------------------------
-- Workspace configuration
-- ---------------------------------------------------------------------------

CREATE TABLE workspace_settings (
    id             INTEGER PRIMARY KEY CHECK (id = 1),
    schema_version TEXT    NOT NULL,
    timezone       TEXT    NOT NULL,                -- IANA name, e.g. Europe/Warsaw
    locale         TEXT    NOT NULL DEFAULT 'en' CHECK (locale IN ('en', 'pl', 'es')),
    mcp_enabled    INTEGER NOT NULL DEFAULT 0 CHECK (mcp_enabled IN (0, 1)),
    created_at     TEXT    NOT NULL,
    updated_at     TEXT    NOT NULL
);

CREATE TABLE source_profile (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    name       TEXT    NOT NULL UNIQUE,
    is_active  INTEGER NOT NULL DEFAULT 0 CHECK (is_active IN (0, 1)),
    created_at TEXT    NOT NULL,
    notes      TEXT
);

-- The workspace keeps at most one active default import profile.
CREATE UNIQUE INDEX uq_source_profile_active ON source_profile (is_active) WHERE is_active = 1;

CREATE TABLE job (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    kind         TEXT NOT NULL CHECK (kind IN ('import', 'forecast', 'export', 'backup', 'migration')),
    status       TEXT NOT NULL CHECK (status IN ('queued', 'running', 'completed', 'failed', 'cancelled')),
    created_at   TEXT NOT NULL,
    started_at   TEXT,
    finished_at  TEXT,
    error_code   TEXT,
    error_detail TEXT
);

CREATE INDEX idx_job_status ON job (status, created_at);

-- ---------------------------------------------------------------------------
-- Import revisions and staging
-- ---------------------------------------------------------------------------

-- Committed snapshots only. Failed import attempts exist solely as `job` rows
-- (kind='import', status='failed') and never block a retry.
CREATE TABLE import_revision (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    profile_id          INTEGER NOT NULL REFERENCES source_profile (id),
    content_hash        TEXT    NOT NULL,           -- SHA-256 of the canonicalized source content
    mapping_fingerprint TEXT    NOT NULL,           -- SHA-256 of the effective mapping plan
    contract_version    TEXT    NOT NULL,           -- input contract, e.g. 'canonical/1'
    source_filename     TEXT    NOT NULL,
    imported_at         TEXT    NOT NULL,
    row_count           INTEGER NOT NULL DEFAULT 0 CHECK (row_count >= 0),
    observation_count   INTEGER NOT NULL DEFAULT 0 CHECK (observation_count >= 0),
    issue_count         INTEGER NOT NULL DEFAULT 0 CHECK (issue_count >= 0),
    -- Idempotency: same content + same mapping fingerprint + same contract
    -- version resolves to the existing revision; a changed fingerprint creates
    -- a new immutable revision.
    UNIQUE (profile_id, content_hash, mapping_fingerprint, contract_version)
);

CREATE TABLE source_file (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    revision_id   INTEGER NOT NULL REFERENCES import_revision (id),
    relative_path TEXT    NOT NULL UNIQUE,          -- globally unique path inside the workspace directory
    sha256        TEXT    NOT NULL,
    byte_size     INTEGER NOT NULL CHECK (byte_size >= 0)
);

CREATE TABLE import_issue (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    revision_id INTEGER NOT NULL REFERENCES import_revision (id),
    severity    TEXT    NOT NULL CHECK (severity IN ('info', 'warning', 'error')),
    code        TEXT    NOT NULL,                   -- e.g. unmapped_code, duplicate_conflict, parse_error
    detail      TEXT    NOT NULL,
    row_ref     TEXT
);

CREATE INDEX idx_import_issue_revision ON import_issue (revision_id, severity);

-- ---------------------------------------------------------------------------
-- Master data and explicit mappings
-- ---------------------------------------------------------------------------

CREATE TABLE product (
    id         BLOB    NOT NULL PRIMARY KEY CHECK (length(id) = 16),  -- UUIDv7
    sku        TEXT    NOT NULL UNIQUE,             -- canonical display code
    name       TEXT    NOT NULL DEFAULT '',
    is_active  INTEGER NOT NULL DEFAULT 1 CHECK (is_active IN (0, 1)),
    created_at TEXT    NOT NULL
);

CREATE TABLE location (
    id         BLOB NOT NULL PRIMARY KEY CHECK (length(id) = 16),     -- UUIDv7
    code       TEXT NOT NULL UNIQUE,
    name       TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL
);

CREATE TABLE product_code_map (
    profile_id  INTEGER NOT NULL REFERENCES source_profile (id),
    source_code TEXT    NOT NULL,
    product_id  BLOB    NOT NULL REFERENCES product (id),
    created_at  TEXT    NOT NULL,
    PRIMARY KEY (profile_id, source_code)
) WITHOUT ROWID;

CREATE INDEX idx_product_code_map_product ON product_code_map (product_id);

CREATE TABLE location_code_map (
    profile_id  INTEGER NOT NULL REFERENCES source_profile (id),
    source_code TEXT    NOT NULL,
    location_id BLOB    NOT NULL REFERENCES location (id),
    created_at  TEXT    NOT NULL,
    PRIMARY KEY (profile_id, source_code)
) WITHOUT ROWID;

CREATE INDEX idx_location_code_map_location ON location_code_map (location_id);

-- ---------------------------------------------------------------------------
-- Observations (revision-scoped, immutable snapshots)
-- ---------------------------------------------------------------------------

-- Observation status vs. quantity:
--   * 'missing' and 'partial' require quantity IS NULL;
--   * 'zero' requires quantity = 0;
--   * every other status requires a numeric quantity (may be negative for returns).
CREATE TABLE observation (
    revision_id INTEGER NOT NULL REFERENCES import_revision (id),
    product_id  BLOB    NOT NULL REFERENCES product (id),
    location_id BLOB    NOT NULL REFERENCES location (id),
    week_start  TEXT    NOT NULL CHECK (week_start GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]'),
    quantity    REAL,
    status      TEXT    NOT NULL CHECK (status IN ('known', 'zero', 'missing', 'return', 'correction', 'stockout', 'partial')),
    CHECK (
        ((status IN ('missing', 'partial')) = (quantity IS NULL))
        AND (status <> 'zero' OR quantity = 0)
    ),
    PRIMARY KEY (revision_id, product_id, location_id, week_start)
) WITHOUT ROWID;

-- Series history lookup across revisions (newest revision first is applied in queries).
CREATE INDEX idx_observation_series ON observation (product_id, location_id, week_start, revision_id);

CREATE TABLE series_qualification (
    revision_id INTEGER NOT NULL REFERENCES import_revision (id),
    product_id  BLOB    NOT NULL REFERENCES product (id),
    location_id BLOB    NOT NULL REFERENCES location (id),
    h_length    INTEGER NOT NULL CHECK (h_length >= 0),  -- trailing run of numeric weeks (ADR-0005)
    status      TEXT    NOT NULL CHECK (status IN ('eligible', 'insufficient_history', 'invalid_data')),
    reason      TEXT,
    computed_at TEXT    NOT NULL,
    PRIMARY KEY (revision_id, product_id, location_id)
) WITHOUT ROWID;

CREATE INDEX idx_series_qualification_status ON series_qualification (revision_id, status);

-- ---------------------------------------------------------------------------
-- Forecast runs, models, per-series evaluation
-- ---------------------------------------------------------------------------

CREATE TABLE forecast_run (
    id                  BLOB    NOT NULL PRIMARY KEY CHECK (length(id) = 16),  -- UUIDv7
    revision_id         INTEGER NOT NULL REFERENCES import_revision (id),      -- pinned input revision
    created_at          TEXT    NOT NULL,
    finished_at         TEXT,
    status              TEXT    NOT NULL CHECK (status IN ('running', 'completed', 'failed', 'cancelled')),
    horizon_weeks       INTEGER NOT NULL DEFAULT 12 CHECK (horizon_weeks = 12),
    horizon_start_week  TEXT    NOT NULL CHECK (horizon_start_week GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]'),  -- Monday after the max imported week in the pinned revision
    timezone            TEXT    NOT NULL,                 -- timezone snapshot used by the run
    config_json         TEXT    NOT NULL,                 -- models, windows, floors, engine config
    app_version         TEXT    NOT NULL,
    engine_version      TEXT    NOT NULL
);

CREATE INDEX idx_forecast_run_status ON forecast_run (status, created_at);

-- Run-level execution summary only. Champion selection, ranking, and metrics
-- live per series in series_model_evaluation / series_model_origin.
CREATE TABLE run_model (
    run_id            BLOB    NOT NULL REFERENCES forecast_run (id),
    model_id          TEXT    NOT NULL CHECK (model_id IN ('naive', 'seasonal_naive_52', 'auto_ets_52')),
    status            TEXT    NOT NULL CHECK (status IN ('completed', 'partial', 'failed', 'not_run')),
    tasks_planned     INTEGER NOT NULL DEFAULT 0 CHECK (tasks_planned >= 0),
    tasks_completed   INTEGER NOT NULL DEFAULT 0 CHECK (tasks_completed >= 0),
    tasks_failed      INTEGER NOT NULL DEFAULT 0 CHECK (tasks_failed >= 0),
    failure_code      TEXT,                          -- fit_error | predict_error | timeout | invalid_input | ...
    failure_detail    TEXT,
    PRIMARY KEY (run_id, model_id)
) WITHOUT ROWID;

CREATE TABLE series_model_evaluation (
    run_id            BLOB    NOT NULL,
    product_id        BLOB    NOT NULL,
    location_id       BLOB    NOT NULL,
    model_id          TEXT    NOT NULL,
    eligibility_status TEXT   NOT NULL CHECK (eligibility_status IN ('not_eligible', 'production_only', 'champion_eligible')),
    origins_planned   INTEGER NOT NULL DEFAULT 0 CHECK (origins_planned >= 0),
    origins_completed INTEGER NOT NULL DEFAULT 0 CHECK (origins_completed >= 0),
    selection_score   REAL,                          -- nullable; computed by Rust from completed origins
    selection_rank    INTEGER CHECK (selection_rank IS NULL OR selection_rank >= 1),
    is_champion       INTEGER NOT NULL DEFAULT 0 CHECK (is_champion IN (0, 1)),
    production_status TEXT    NOT NULL DEFAULT 'not_run' CHECK (production_status IN ('not_run', 'completed', 'failed')),
    failure_code      TEXT,
    failure_detail    TEXT,
    PRIMARY KEY (run_id, product_id, location_id, model_id),
    FOREIGN KEY (run_id, model_id) REFERENCES run_model (run_id, model_id)
) WITHOUT ROWID;

CREATE INDEX idx_series_model_evaluation_series
    ON series_model_evaluation (run_id, product_id, location_id);
CREATE INDEX idx_series_model_evaluation_eligibility
    ON series_model_evaluation (run_id, eligibility_status);
-- At most one champion per run/series.
CREATE UNIQUE INDEX uq_series_model_evaluation_champion
    ON series_model_evaluation (run_id, product_id, location_id) WHERE is_champion = 1;

-- Per-series, per-model rolling origins. Origin dates and train sizes vary by
-- series, so they cannot live at run level.
CREATE TABLE series_model_origin (
    run_id            BLOB    NOT NULL,
    product_id        BLOB    NOT NULL,
    location_id       BLOB    NOT NULL,
    model_id          TEXT    NOT NULL,
    origin_index      INTEGER NOT NULL CHECK (origin_index BETWEEN 1 AND 4),
    origin_week_start TEXT    NOT NULL CHECK (origin_week_start GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]'),
    train_size        INTEGER NOT NULL CHECK (train_size >= 0),
    status            TEXT    NOT NULL CHECK (status IN ('completed', 'failed')),
    wape              REAL,                          -- nullable; MAE fallback when actuals are all zero
    mae               REAL,
    metric_kind       TEXT,                          -- wape | mae_fallback | ...
    metric_value      REAL,
    failure_code      TEXT,
    failure_detail    TEXT,
    PRIMARY KEY (run_id, product_id, location_id, model_id, origin_index),
    FOREIGN KEY (run_id, product_id, location_id, model_id)
        REFERENCES series_model_evaluation (run_id, product_id, location_id, model_id)
) WITHOUT ROWID;

CREATE INDEX idx_series_model_origin_run ON series_model_origin (run_id);

CREATE TABLE backtest_forecast (
    run_id       BLOB    NOT NULL,
    product_id   BLOB    NOT NULL,
    location_id  BLOB    NOT NULL,
    model_id     TEXT    NOT NULL,
    origin_index INTEGER NOT NULL,
    week_start   TEXT    NOT NULL CHECK (week_start GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]'),
    value        REAL    NOT NULL,
    PRIMARY KEY (run_id, product_id, location_id, model_id, origin_index, week_start),
    FOREIGN KEY (run_id, product_id, location_id, model_id, origin_index)
        REFERENCES series_model_origin (run_id, product_id, location_id, model_id, origin_index)
) WITHOUT ROWID;

CREATE INDEX idx_backtest_forecast_series ON backtest_forecast (run_id, product_id, location_id);

-- ---------------------------------------------------------------------------
-- Forecast outcomes and values
-- ---------------------------------------------------------------------------

-- Champion metrics and origin counters are not duplicated here; query them from
-- series_model_evaluation. selected_model_id must reference the matching
-- per-series evaluation row.
CREATE TABLE forecast_series_status (
    run_id            BLOB    NOT NULL REFERENCES forecast_run (id),
    product_id        BLOB    NOT NULL,
    location_id       BLOB    NOT NULL,
    status            TEXT    NOT NULL CHECK (status IN ('ok', 'fallback_naive', 'insufficient_history', 'model_failure', 'invalid_data')),
    selected_model_id TEXT,
    reason            TEXT,
    PRIMARY KEY (run_id, product_id, location_id),
    FOREIGN KEY (run_id, product_id, location_id, selected_model_id)
        REFERENCES series_model_evaluation (run_id, product_id, location_id, model_id)
) WITHOUT ROWID;

CREATE TABLE forecast (
    run_id            BLOB    NOT NULL,
    product_id        BLOB    NOT NULL,
    location_id       BLOB    NOT NULL,
    week_start        TEXT    NOT NULL CHECK (week_start GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]'),
    value             REAL    NOT NULL,
    selected_model_id TEXT    NOT NULL,
    PRIMARY KEY (run_id, product_id, location_id, week_start),
    FOREIGN KEY (run_id, product_id, location_id) REFERENCES forecast_series_status (run_id, product_id, location_id),
    FOREIGN KEY (run_id, product_id, location_id, selected_model_id)
        REFERENCES series_model_evaluation (run_id, product_id, location_id, model_id)
) WITHOUT ROWID;

-- Append-only override revisions: never UPDATEd or DELETEd (triggers below); a
-- NULL value clears the override (effective falls back to the baseline, or to
-- missing when no baseline exists). Highest revision wins.
CREATE TABLE forecast_override (
    run_id      BLOB    NOT NULL,
    product_id  BLOB    NOT NULL,
    location_id BLOB    NOT NULL,
    week_start  TEXT    NOT NULL CHECK (week_start GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]'),
    revision    INTEGER NOT NULL CHECK (revision >= 1),
    value       REAL,                               -- NULL = cleared
    note        TEXT,
    created_at  TEXT    NOT NULL,
    PRIMARY KEY (run_id, product_id, location_id, week_start, revision),
    FOREIGN KEY (run_id, product_id, location_id) REFERENCES forecast_series_status (run_id, product_id, location_id)
) WITHOUT ROWID;

-- Manual-override guardrails (repeat the validations in Rust):
--   * the run must be 'completed';
--   * the week must be inside the persisted horizon;
--   * the first revision for a cell must be non-null;
--   * revisions must be sequential and append-only.
CREATE TRIGGER forecast_override_guardrails
BEFORE INSERT ON forecast_override
BEGIN
    SELECT RAISE(ABORT, 'override: run must be completed')
    WHERE (SELECT status FROM forecast_run WHERE id = NEW.run_id) IS NOT 'completed';
    SELECT RAISE(ABORT, 'override: target series is not part of the run')
    WHERE NOT EXISTS (
        SELECT 1 FROM forecast_series_status
        WHERE run_id = NEW.run_id AND product_id = NEW.product_id AND location_id = NEW.location_id
    );
    SELECT RAISE(ABORT, 'override: week outside the persisted horizon')
    WHERE NEW.week_start < (SELECT horizon_start_week FROM forecast_run WHERE id = NEW.run_id)
       OR NEW.week_start >= date((SELECT horizon_start_week FROM forecast_run WHERE id = NEW.run_id), '+83 days');
    SELECT RAISE(ABORT, 'override: first revision must be non-null')
    WHERE NEW.revision = 1 AND NEW.value IS NULL;
    SELECT RAISE(ABORT, 'override: revisions must be sequential')
    WHERE NEW.revision > 1 AND NOT EXISTS (
        SELECT 1 FROM forecast_override
        WHERE run_id = NEW.run_id AND product_id = NEW.product_id AND location_id = NEW.location_id
          AND week_start = NEW.week_start AND revision = NEW.revision - 1
    );
END;

CREATE TRIGGER forecast_override_reject_update
BEFORE UPDATE ON forecast_override
BEGIN
    SELECT RAISE(ABORT, 'override: UPDATE is rejected; insert a new revision instead');
END;

CREATE TRIGGER forecast_override_reject_delete
BEFORE DELETE ON forecast_override
BEGIN
    SELECT RAISE(ABORT, 'override: DELETE is rejected; insert a clearing revision instead');
END;

CREATE TABLE export_record (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id        BLOB    NOT NULL REFERENCES forecast_run (id),
    format        TEXT    NOT NULL CHECK (format IN ('csv', 'xlsx')),
    relative_path TEXT    NOT NULL,
    created_at    TEXT    NOT NULL,
    row_count     INTEGER NOT NULL CHECK (row_count >= 0),
    coverage_json TEXT
);

-- End of migration 001.
