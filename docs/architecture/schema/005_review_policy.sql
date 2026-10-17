-- OpenDemand workspace database: exception-inbox review policy
-- (migration 005).
--
-- Additive only. Migration 001 is frozen and must never be edited; this file
-- adds one append-only table and its guardrail triggers.
--
-- Gate: the Rust core creates and verifies a pre-migration backup before it
-- applies this migration to an existing workspace (ADR-0008, ADR-0021).

-- Versioned, profile-scoped user thresholds for the derived exception inbox
-- (ADR-0028). A row with both thresholds NULL means the material-change rule
-- is disabled; rows are append-only and the highest version is current.
CREATE TABLE review_policy (
    profile_id INTEGER NOT NULL REFERENCES source_profile (id),
    version    INTEGER NOT NULL CHECK (version >= 1),
    change_abs REAL    CHECK (change_abs IS NULL OR change_abs > 0),
    change_rel REAL    CHECK (change_rel IS NULL OR change_rel > 0),
    created_at TEXT    NOT NULL,
    PRIMARY KEY (profile_id, version),
    CHECK ((change_abs IS NULL) = (change_rel IS NULL))
) WITHOUT ROWID;

CREATE TRIGGER review_policy_reject_update
BEFORE UPDATE ON review_policy
BEGIN
    SELECT RAISE(ABORT, 'review_policy: UPDATE is rejected; append a new version instead');
END;

CREATE TRIGGER review_policy_reject_delete
BEFORE DELETE ON review_policy
BEGIN
    SELECT RAISE(ABORT, 'review_policy: DELETE is rejected');
END;

-- End of migration 005.
