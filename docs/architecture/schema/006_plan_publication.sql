-- OpenDemand workspace database: immutable plan publications (migration 006).
--
-- Additive only. Migration 001 is frozen and must never be edited; this file
-- adds two new tables, their guardrail triggers, and one nullable column on
-- export_record.
--
-- Gate: the Rust core creates and verifies a pre-migration backup before it
-- applies this migration to an existing workspace (ADR-0008, ADR-0021).

-- ---------------------------------------------------------------------------
-- Plan publications
-- ---------------------------------------------------------------------------

-- One immutable official publication per source profile and history cutoff
-- week (ADR-0029). Publishing pins the exact override revision of every
-- overridden cell; published effective values are derived from those pins and
-- the immutable baselines, never copied.
CREATE TABLE plan_publication (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    profile_id          INTEGER NOT NULL REFERENCES source_profile (id),
    run_id              BLOB    NOT NULL REFERENCES forecast_run (id),
    revision_id         INTEGER NOT NULL REFERENCES import_revision (id),
    cutoff_week         TEXT    NOT NULL CHECK (cutoff_week GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]'),
    horizon_start_week  TEXT    NOT NULL CHECK (horizon_start_week GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]'),
    -- Highest import revision id of the profile visible in the publication
    -- transaction: the hindsight boundary for retrospective evaluation.
    revision_high_water INTEGER NOT NULL REFERENCES import_revision (id),
    timezone            TEXT    NOT NULL,
    policy_context      TEXT    NOT NULL,        -- publication policy label, e.g. 'publication/1'
    published_at        TEXT    NOT NULL,
    UNIQUE (profile_id, cutoff_week)
);

CREATE TRIGGER plan_publication_reject_update
BEFORE UPDATE ON plan_publication
BEGIN
    SELECT RAISE(ABORT, 'plan_publication: UPDATE is rejected; publications are immutable');
END;

CREATE TRIGGER plan_publication_reject_delete
BEFORE DELETE ON plan_publication
BEGIN
    SELECT RAISE(ABORT, 'plan_publication: DELETE is rejected; publications are immutable');
END;

-- The exact latest override revision of every overridden cell of the pinned
-- run at publication time, including NULL clearing revisions. The composite
-- foreign key proves the pin matches a real override revision.
CREATE TABLE publication_override (
    publication_id INTEGER NOT NULL REFERENCES plan_publication (id),
    run_id         BLOB    NOT NULL,
    product_id     BLOB    NOT NULL,
    location_id    BLOB    NOT NULL,
    week_start     TEXT    NOT NULL CHECK (week_start GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]'),
    revision       INTEGER NOT NULL CHECK (revision >= 1),
    PRIMARY KEY (publication_id, product_id, location_id, week_start),
    FOREIGN KEY (run_id, product_id, location_id, week_start, revision)
        REFERENCES forecast_override (run_id, product_id, location_id, week_start, revision)
) WITHOUT ROWID;

CREATE TRIGGER publication_override_reject_update
BEFORE UPDATE ON publication_override
BEGIN
    SELECT RAISE(ABORT, 'publication_override: UPDATE is rejected; publications are immutable');
END;

CREATE TRIGGER publication_override_reject_delete
BEFORE DELETE ON publication_override
BEGIN
    SELECT RAISE(ABORT, 'publication_override: DELETE is rejected; publications are immutable');
END;

-- Publication exports link back to the publication they published; ordinary
-- run exports keep publication_id NULL.
ALTER TABLE export_record ADD COLUMN publication_id INTEGER DEFAULT NULL REFERENCES plan_publication (id);

-- End of migration 006.
