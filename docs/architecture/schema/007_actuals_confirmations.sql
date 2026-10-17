-- OpenDemand workspace database: actuals confirmations and persistent-bias
-- policy (migration 007).
--
-- Additive only. Migration 001 is frozen and must never be edited; this file
-- adds one append-only table, its guardrail triggers, and four nullable
-- columns on review_policy.
--
-- Gate: the Rust core creates and verifies a pre-migration backup before it
-- applies this migration to an existing workspace (ADR-0008, ADR-0021).

-- Append-only user confirmations that one actuals revision is complete through
-- a specific finished week (ADR-0030). Retrospective evaluation pins the exact
-- confirmation; corrections or retractions append a new row.
CREATE TABLE actuals_confirmation (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    profile_id        INTEGER NOT NULL REFERENCES source_profile (id),
    revision_id       INTEGER NOT NULL REFERENCES import_revision (id),
    confirmed_through TEXT    NOT NULL CHECK (confirmed_through GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]'),
    created_at        TEXT    NOT NULL
);

CREATE INDEX idx_actuals_confirmation_profile
    ON actuals_confirmation (profile_id, revision_id, id);

CREATE TRIGGER actuals_confirmation_reject_update
BEFORE UPDATE ON actuals_confirmation
BEGIN
    SELECT RAISE(ABORT, 'actuals_confirmation: UPDATE is rejected; append a new confirmation instead');
END;

CREATE TRIGGER actuals_confirmation_reject_delete
BEFORE DELETE ON actuals_confirmation
BEGIN
    SELECT RAISE(ABORT, 'actuals_confirmation: DELETE is rejected');
END;

-- Persistent-bias rule fields of the append-only review policy (ADR-0030).
-- Per-column self-checks only: SQLite ADD COLUMN CHECK constraints cannot
-- reference other columns, so the rule group (horizon, window, minimum count,
-- threshold) is validated as an all-set-or-all-null tuple in Rust.
ALTER TABLE review_policy ADD COLUMN bias_horizon INTEGER CHECK (bias_horizon IS NULL OR (bias_horizon >= 1 AND bias_horizon <= 12));
ALTER TABLE review_policy ADD COLUMN bias_window INTEGER CHECK (bias_window IS NULL OR (bias_window >= 2 AND bias_window <= 52));
ALTER TABLE review_policy ADD COLUMN bias_min INTEGER CHECK (bias_min IS NULL OR (bias_min >= 2 AND bias_min <= 52));
ALTER TABLE review_policy ADD COLUMN bias_abs REAL CHECK (bias_abs IS NULL OR bias_abs > 0);

-- End of migration 007.
