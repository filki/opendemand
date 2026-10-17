-- OpenDemand workspace database: inventory snapshot inputs (migration 008).
--
-- Additive only. Migration 001 is frozen and must never be edited; this file
-- adds the inventory/1 input tables, their guardrail triggers, and one
-- nullable column on export_record.
--
-- Gate: the Rust core creates and verifies a pre-migration backup before it
-- applies this migration to an existing workspace (ADR-0008, ADR-0021).

-- One immutable imported inventory batch (ADR-0031): an opening-stock
-- snapshot as of a Monday, its receipts and policies, retained raw files, and
-- the confirmations the import required. Identical content is a no-op.
CREATE TABLE inventory_snapshot (
    id                      INTEGER PRIMARY KEY AUTOINCREMENT,
    profile_id              INTEGER NOT NULL REFERENCES source_profile (id),
    location_id             BLOB    NOT NULL REFERENCES location (id),
    as_of_week              TEXT    NOT NULL CHECK (as_of_week GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]'),
    contract_version        TEXT    NOT NULL,
    content_hash            TEXT    NOT NULL,
    unit_confirmed          INTEGER NOT NULL CHECK (unit_confirmed IN (0, 1)),
    receipts_empty_confirmed INTEGER NOT NULL CHECK (receipts_empty_confirmed IN (0, 1)),
    imported_at             TEXT    NOT NULL,
    UNIQUE (profile_id, content_hash, contract_version)
);

-- Explicit unrestricted physical stock per scoped product; absent is NOT zero.
CREATE TABLE inventory_stock (
    snapshot_id INTEGER NOT NULL REFERENCES inventory_snapshot (id),
    product_id  BLOB    NOT NULL REFERENCES product (id),
    quantity    REAL    NOT NULL CHECK (quantity >= 0),
    PRIMARY KEY (snapshot_id, product_id)
) WITHOUT ROWID;

-- Open dated inbound receipts; overdue rows are retained with their state so
-- recommendations can block until a corrected snapshot arrives.
CREATE TABLE inventory_receipt (
    snapshot_id    INTEGER NOT NULL REFERENCES inventory_snapshot (id),
    receipt_key    TEXT    NOT NULL,
    product_id     BLOB    NOT NULL REFERENCES product (id),
    quantity       REAL    NOT NULL CHECK (quantity > 0),
    available_week TEXT    NOT NULL CHECK (available_week GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]'),
    state          TEXT    NOT NULL CHECK (state IN ('open', 'overdue')),
    PRIMARY KEY (snapshot_id, receipt_key)
) WITHOUT ROWID;

-- Whole-week policy per scoped product; one unit per SKU, no conversions.
CREATE TABLE inventory_policy (
    snapshot_id  INTEGER NOT NULL REFERENCES inventory_snapshot (id),
    product_id   BLOB    NOT NULL REFERENCES product (id),
    unit         TEXT    NOT NULL CHECK (length(trim(unit)) > 0),
    lead_weeks   INTEGER NOT NULL CHECK (lead_weeks >= 0),
    review_weeks INTEGER NOT NULL CHECK (review_weeks >= 1),
    safety_stock REAL    NOT NULL CHECK (safety_stock >= 0),
    moq          REAL    NOT NULL CHECK (moq >= 0),
    pack_size    REAL    NOT NULL CHECK (pack_size > 0),
    PRIMARY KEY (snapshot_id, product_id)
) WITHOUT ROWID;

-- Retained raw files of one inventory batch (stock, receipts, policy, or one
-- workbook), with hashes, under the workspace sources area.
CREATE TABLE inventory_source_file (
    snapshot_id   INTEGER NOT NULL REFERENCES inventory_snapshot (id),
    role          TEXT    NOT NULL CHECK (role IN ('stock', 'receipts', 'policy', 'workbook')),
    relative_path TEXT    NOT NULL,
    sha256        TEXT    NOT NULL,
    byte_size     INTEGER NOT NULL,
    PRIMARY KEY (snapshot_id, role)
) WITHOUT ROWID;

CREATE TRIGGER inventory_snapshot_reject_update
BEFORE UPDATE ON inventory_snapshot
BEGIN
    SELECT RAISE(ABORT, 'inventory_snapshot: UPDATE is rejected; import a new snapshot instead');
END;

CREATE TRIGGER inventory_snapshot_reject_delete
BEFORE DELETE ON inventory_snapshot
BEGIN
    SELECT RAISE(ABORT, 'inventory_snapshot: DELETE is rejected');
END;

CREATE TRIGGER inventory_stock_reject_update
BEFORE UPDATE ON inventory_stock
BEGIN
    SELECT RAISE(ABORT, 'inventory_stock: UPDATE is rejected');
END;

CREATE TRIGGER inventory_stock_reject_delete
BEFORE DELETE ON inventory_stock
BEGIN
    SELECT RAISE(ABORT, 'inventory_stock: DELETE is rejected');
END;

CREATE TRIGGER inventory_receipt_reject_update
BEFORE UPDATE ON inventory_receipt
BEGIN
    SELECT RAISE(ABORT, 'inventory_receipt: UPDATE is rejected');
END;

CREATE TRIGGER inventory_receipt_reject_delete
BEFORE DELETE ON inventory_receipt
BEGIN
    SELECT RAISE(ABORT, 'inventory_receipt: DELETE is rejected');
END;

CREATE TRIGGER inventory_policy_reject_update
BEFORE UPDATE ON inventory_policy
BEGIN
    SELECT RAISE(ABORT, 'inventory_policy: UPDATE is rejected');
END;

CREATE TRIGGER inventory_policy_reject_delete
BEFORE DELETE ON inventory_policy
BEGIN
    SELECT RAISE(ABORT, 'inventory_policy: DELETE is rejected');
END;

CREATE TRIGGER inventory_source_file_reject_update
BEFORE UPDATE ON inventory_source_file
BEGIN
    SELECT RAISE(ABORT, 'inventory_source_file: UPDATE is rejected');
END;

CREATE TRIGGER inventory_source_file_reject_delete
BEFORE DELETE ON inventory_source_file
BEGIN
    SELECT RAISE(ABORT, 'inventory_source_file: DELETE is rejected');
END;

-- Replenishment exports link back to their exact inventory snapshot; ordinary
-- and publication-only exports keep this NULL.
ALTER TABLE export_record ADD COLUMN inventory_snapshot_id INTEGER DEFAULT NULL REFERENCES inventory_snapshot (id);

-- End of migration 008.
