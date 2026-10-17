-- OpenDemand workspace database: sparse-model registry (migration 009).
--
-- SQLite cannot alter a CHECK constraint, so the model vocabulary is extended
-- by rebuilding the whole run-evidence chain that references it. The tables
-- are dropped in dependency order and recreated from migration 001's exact
-- DDL, then every row is restored from temporary backups inside the same
-- transaction; no foreign key is ever left dangling and migration 001 stays
-- frozen.
--
-- Gate: the Rust core creates and verifies a pre-migration backup before it
-- applies this migration to an existing workspace (ADR-0008, ADR-0021).

CREATE TEMP TABLE backup_backtest_forecast AS SELECT * FROM backtest_forecast;
CREATE TEMP TABLE backup_series_model_origin AS SELECT * FROM series_model_origin;
CREATE TEMP TABLE backup_forecast_series_status AS SELECT * FROM forecast_series_status;
CREATE TEMP TABLE backup_forecast AS SELECT * FROM forecast;
CREATE TEMP TABLE backup_series_model_evaluation AS SELECT * FROM series_model_evaluation;
CREATE TEMP TABLE backup_run_model AS SELECT * FROM run_model;

DROP TABLE backtest_forecast;
DROP TABLE series_model_origin;
DROP TABLE forecast;
DROP TABLE forecast_series_status;
DROP TABLE series_model_evaluation;
DROP TABLE run_model;

CREATE TABLE run_model (
    run_id            BLOB    NOT NULL REFERENCES forecast_run (id),
    model_id          TEXT    NOT NULL CHECK (model_id IN ('naive', 'seasonal_naive_52', 'auto_ets_52', 'croston_sba', 'tsb')),
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

CREATE TABLE forecast_series_status (
    run_id            BLOB    NOT NULL REFERENCES forecast_run (id),
    product_id        BLOB    NOT NULL,
    location_id       BLOB    NOT NULL,
    status            TEXT    NOT NULL CHECK (status IN ('ok', 'fallback_naive', 'insufficient_history', 'model_failure', 'invalid_data')),
    selected_model_id TEXT,
    reason            TEXT,
    imputed_weeks     INTEGER NOT NULL DEFAULT 0,
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

INSERT INTO run_model SELECT * FROM backup_run_model;
INSERT INTO series_model_evaluation SELECT * FROM backup_series_model_evaluation;
INSERT INTO series_model_origin SELECT * FROM backup_series_model_origin;
INSERT INTO forecast_series_status SELECT * FROM backup_forecast_series_status;
INSERT INTO forecast SELECT * FROM backup_forecast;
INSERT INTO backtest_forecast SELECT * FROM backup_backtest_forecast;

DROP TABLE backup_backtest_forecast;
DROP TABLE backup_series_model_origin;
DROP TABLE backup_forecast_series_status;
DROP TABLE backup_forecast;
DROP TABLE backup_series_model_evaluation;
DROP TABLE backup_run_model;

-- End of migration 009.
