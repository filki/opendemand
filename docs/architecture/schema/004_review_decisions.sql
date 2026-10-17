-- OpenDemand workspace database: planner decision memory and review events
-- (migration 004).
--
-- Additive only. Migration 001 is frozen and must never be edited; this file
-- changes the schema exclusively through three new tables and their append-only
-- guardrail triggers.
--
-- Gate: the Rust core creates and verifies a pre-migration backup before it
-- applies this migration to an existing workspace (ADR-0008, ADR-0021).

-- ---------------------------------------------------------------------------
-- Planner notes and review events
-- ---------------------------------------------------------------------------

-- Append-only series notes: planner intent that survives across forecast runs
-- for the same product x location inside one source profile. Notes never change
-- coverage, plan values, or stored history.
CREATE TABLE series_note (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    profile_id     INTEGER NOT NULL REFERENCES source_profile (id),
    product_id     BLOB    NOT NULL,
    location_id    BLOB    NOT NULL,
    note           TEXT    NOT NULL CHECK (length(trim(note)) > 0),
    context_run_id BLOB,                          -- run open when the note was written, if any
    created_at     TEXT    NOT NULL
);

CREATE INDEX idx_series_note_series
    ON series_note (profile_id, product_id, location_id, id);

-- Append-only review events. An acknowledgement names the run it reviewed and
-- the deterministic evidence fingerprint it was based on; the current state is
-- derived at read time by comparing the latest event's fingerprint with the
-- run's current evidence. New evidence never rewrites an event.
CREATE TABLE series_review (
    id                   INTEGER PRIMARY KEY AUTOINCREMENT,
    profile_id           INTEGER NOT NULL REFERENCES source_profile (id),
    product_id           BLOB    NOT NULL,
    location_id          BLOB    NOT NULL,
    run_id               BLOB    NOT NULL REFERENCES forecast_run (id),
    action               TEXT    NOT NULL CHECK (action IN ('reviewed', 'reopened')),
    evidence_context     TEXT    NOT NULL,        -- rule/policy context label, e.g. 'review/1'
    evidence_fingerprint TEXT    NOT NULL,        -- sha256 hex over the acknowledged evidence
    created_at           TEXT    NOT NULL
);

CREATE INDEX idx_series_review_series
    ON series_review (profile_id, product_id, location_id, run_id, id);

CREATE TRIGGER series_note_reject_update
BEFORE UPDATE ON series_note
BEGIN
    SELECT RAISE(ABORT, 'series_note: UPDATE is rejected; add a new note instead');
END;

CREATE TRIGGER series_note_reject_delete
BEFORE DELETE ON series_note
BEGIN
    SELECT RAISE(ABORT, 'series_note: DELETE is rejected');
END;

CREATE TRIGGER series_review_reject_update
BEFORE UPDATE ON series_review
BEGIN
    SELECT RAISE(ABORT, 'series_review: UPDATE is rejected; append a new event instead');
END;

CREATE TRIGGER series_review_reject_delete
BEFORE DELETE ON series_review
BEGIN
    SELECT RAISE(ABORT, 'series_review: DELETE is rejected');
END;

-- ---------------------------------------------------------------------------
-- Override decision metadata
-- ---------------------------------------------------------------------------

-- Immutable companion metadata for one exact forecast_override revision: the
-- planner's reason and an optional review date. The free-text note lives in the
-- existing forecast_override.note column on the same revision. Historical
-- overrides without a metadata row stay readable as legacy decisions.
CREATE TABLE override_meta (
    run_id      BLOB    NOT NULL,
    product_id  BLOB    NOT NULL,
    location_id BLOB    NOT NULL,
    week_start  TEXT    NOT NULL CHECK (week_start GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]'),
    revision    INTEGER NOT NULL CHECK (revision >= 1),
    reason_code TEXT    NOT NULL CHECK (reason_code IN
        ('promotion', 'customer_information', 'supply_constraint', 'data_quality', 'other')),
    review_date TEXT    CHECK (review_date IS NULL OR review_date GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]'),
    created_at  TEXT    NOT NULL,
    PRIMARY KEY (run_id, product_id, location_id, week_start, revision),
    FOREIGN KEY (run_id, product_id, location_id, week_start, revision)
        REFERENCES forecast_override (run_id, product_id, location_id, week_start, revision)
) WITHOUT ROWID;

-- The metadata must name an existing override revision, and the 'other' reason
-- requires a recorded note on that same revision.
CREATE TRIGGER override_meta_guardrails
BEFORE INSERT ON override_meta
BEGIN
    SELECT RAISE(ABORT, 'override_meta: the matching override revision does not exist')
    WHERE NOT EXISTS (
        SELECT 1 FROM forecast_override
        WHERE run_id = NEW.run_id AND product_id = NEW.product_id
          AND location_id = NEW.location_id AND week_start = NEW.week_start
          AND revision = NEW.revision
    );
    SELECT RAISE(ABORT, 'override_meta: reason other requires a note')
    WHERE NEW.reason_code = 'other' AND NOT EXISTS (
        SELECT 1 FROM forecast_override
        WHERE run_id = NEW.run_id AND product_id = NEW.product_id
          AND location_id = NEW.location_id AND week_start = NEW.week_start
          AND revision = NEW.revision AND note IS NOT NULL AND length(trim(note)) > 0
    );
END;

CREATE TRIGGER override_meta_reject_update
BEFORE UPDATE ON override_meta
BEGIN
    SELECT RAISE(ABORT, 'override_meta: UPDATE is rejected; bookkeeping is immutable');
END;

CREATE TRIGGER override_meta_reject_delete
BEFORE DELETE ON override_meta
BEGIN
    SELECT RAISE(ABORT, 'override_meta: DELETE is rejected');
END;

-- End of migration 004.
