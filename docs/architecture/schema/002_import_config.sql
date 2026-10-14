-- OpenDemand workspace database: saved import configurations and the frozen
-- parsing descriptor per revision (migration 002).
--
-- Additive only. Migration 001 is frozen and must never be edited; this file
-- changes the schema exclusively through a new table and one nullable column.
-- Rationale: see docs/architecture/adr/ (import preprocessing boundary) and the
-- import-preprocessing plan.
--
-- Gate: the Rust core creates and verifies a pre-migration backup before it
-- applies this migration to an existing workspace (ADR-0008).

-- ---------------------------------------------------------------------------
-- Guided import preprocessing
-- ---------------------------------------------------------------------------

-- Named, profile-scoped parsing descriptors the guided import can reuse.
-- Configurations are intentionally separate from `source_profile`: a profile
-- owns identity mappings, a configuration describes one file shape.
CREATE TABLE import_config (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    profile_id      INTEGER NOT NULL REFERENCES source_profile (id),
    name            TEXT    NOT NULL,
    descriptor_json TEXT    NOT NULL,
    created_at      TEXT    NOT NULL,
    updated_at      TEXT    NOT NULL,
    -- One name per profile; saving an existing name updates that row.
    UNIQUE (profile_id, name)
);

-- The parsing descriptor a preprocessed revision was created with, frozen at
-- commit time so the interpretation of the retained bytes stays explainable
-- after a saved configuration is edited. NULL for direct canonical/1 imports.
ALTER TABLE import_revision ADD COLUMN descriptor_json TEXT;

-- End of migration 002.
