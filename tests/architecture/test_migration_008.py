"""Integration tests for migration 008 (inventory snapshot inputs).

Apply 001..008 in filename order, exactly as the Rust migrator does, then
assert the additive tables, immutability triggers, the export-record column,
and that a version-7 database upgrades additively while migration 001 stays
untouched.

Run from the repository root:

    python3 -m unittest discover -s tests/architecture -v
"""

from __future__ import annotations

import hashlib
import os
import sqlite3
import tempfile
import unittest
from pathlib import Path

from tests.architecture.migration_harness import (
    NOW,
    apply_schema,
    assert_fk_check_empty,
    assert_integrity_ok,
    connect,
    read_hash_file,
)

SCHEMA_DIRECTORY = (
    Path(__file__).resolve().parents[2] / "docs" / "architecture" / "schema"
)
MIGRATION_FILES = [
    "002_import_config.sql",
    "003_forecast_imputation.sql",
    "004_review_decisions.sql",
    "005_review_policy.sql",
    "006_plan_publication.sql",
    "007_actuals_confirmations.sql",
    "008_inventory.sql",
]


def apply_migrations(conn: sqlite3.Connection) -> None:
    apply_schema(conn)
    for name in MIGRATION_FILES:
        conn.executescript((SCHEMA_DIRECTORY / name).read_text(encoding="utf-8"))


def column_names(conn: sqlite3.Connection, table: str) -> set:
    return {row[1] for row in conn.execute(f"PRAGMA table_info({table})").fetchall()}


class MigrationEightTest(unittest.TestCase):
    def setUp(self) -> None:
        self._tmpdir = tempfile.TemporaryDirectory(prefix="opendemand-m008-")
        self.db_path = os.path.join(self._tmpdir.name, "workspace.sqlite3")
        self.conn = connect(self.db_path)
        apply_migrations(self.conn)
        self.conn.execute(
            "INSERT INTO source_profile (id, name, is_active, created_at) VALUES (1, 'canonical/1', 1, ?)",
            (NOW,),
        )
        self.conn.execute(
            "INSERT INTO location (id, code, name, created_at) VALUES (x'0102030405060708090a0b0c0d0e0f10', '__default__', 'Default', ?)",
            (NOW,),
        )
        self.conn.execute(
            "INSERT INTO product (id, sku, name, created_at) VALUES (x'1112131415161718191a1b1c1d1e1f20', 'SKU-A', 'Product A', ?)",
            (NOW,),
        )
        self.conn.commit()

    def tearDown(self) -> None:
        self.conn.close()
        self._tmpdir.cleanup()

    def _snapshot(self) -> int:
        cursor = self.conn.execute(
            "INSERT INTO inventory_snapshot (profile_id, location_id, as_of_week, contract_version, content_hash, unit_confirmed, receipts_empty_confirmed, imported_at)"
            " VALUES (1, x'0102030405060708090a0b0c0d0e0f10', '2026-01-05', 'inventory/1', 'hash-a', 1, 1, ?)",
            (NOW,),
        )
        return cursor.lastrowid

    def test_tables_carry_their_documented_columns(self):
        for table, expected in {
            "inventory_snapshot": {
                "id",
                "profile_id",
                "location_id",
                "as_of_week",
                "contract_version",
                "content_hash",
                "unit_confirmed",
                "receipts_empty_confirmed",
                "imported_at",
            },
            "inventory_stock": {"snapshot_id", "product_id", "quantity"},
            "inventory_receipt": {
                "snapshot_id",
                "receipt_key",
                "product_id",
                "quantity",
                "available_week",
                "state",
            },
            "inventory_policy": {
                "snapshot_id",
                "product_id",
                "unit",
                "lead_weeks",
                "review_weeks",
                "safety_stock",
                "moq",
                "pack_size",
            },
            "inventory_source_file": {
                "snapshot_id",
                "role",
                "relative_path",
                "sha256",
                "byte_size",
            },
        }.items():
            self.assertTrue(expected <= column_names(self.conn, table), table)
        self.assertIn("inventory_snapshot_id", column_names(self.conn, "export_record"))

    def test_rows_are_immutable_and_constrained(self):
        snapshot_id = self._snapshot()
        self.conn.execute(
            "INSERT INTO inventory_stock (snapshot_id, product_id, quantity)"
            " VALUES (?, x'1112131415161718191a1b1c1d1e1f20', 3.0)",
            (snapshot_id,),
        )
        self.conn.execute(
            "INSERT INTO inventory_receipt (snapshot_id, receipt_key, product_id, quantity, available_week, state)"
            " VALUES (?, 'R1', x'1112131415161718191a1b1c1d1e1f20', 5.0, '2026-01-12', 'open')",
            (snapshot_id,),
        )
        for statement in [
            "UPDATE inventory_snapshot SET as_of_week = '2026-01-12'",
            "DELETE FROM inventory_snapshot",
            "UPDATE inventory_stock SET quantity = 99",
            "DELETE FROM inventory_stock",
            "UPDATE inventory_receipt SET state = 'overdue'",
            "DELETE FROM inventory_receipt",
        ]:
            with self.assertRaises(sqlite3.IntegrityError, msg=statement):
                self.conn.execute(statement)
        with self.assertRaises(sqlite3.IntegrityError):
            self.conn.execute(
                "INSERT INTO inventory_stock (snapshot_id, product_id, quantity)"
                " VALUES (?, x'1112131415161718191a1b1c1d1e1f20', -1.0)",
                (snapshot_id,),
            )
        with self.assertRaises(sqlite3.IntegrityError):
            self.conn.execute(
                "INSERT INTO inventory_receipt (snapshot_id, receipt_key, product_id, quantity, available_week, state)"
                " VALUES (?, 'R2', x'1112131415161718191a1b1c1d1e1f20', 1.0, '2026-01-12', 'late')",
                (snapshot_id,),
            )

    def test_one_snapshot_per_content_and_contract(self):
        self._snapshot()
        with self.assertRaises(sqlite3.IntegrityError):
            self._snapshot()

    def test_schema_stays_consistent(self):
        assert_integrity_ok(self, self.conn)
        assert_fk_check_empty(self, self.conn)

    def test_a_version_seven_database_upgrades_additively(self):
        with tempfile.TemporaryDirectory(prefix="opendemand-m008-old-") as directory:
            old_path = os.path.join(directory, "workspace.sqlite3")
            old = connect(old_path)
            apply_schema(old)
            for name in MIGRATION_FILES[:6]:
                old.executescript((SCHEMA_DIRECTORY / name).read_text(encoding="utf-8"))
            old.execute(
                "INSERT INTO workspace_settings (id, schema_version, timezone, locale, mcp_enabled, created_at, updated_at)"
                " VALUES (1, '7', 'Europe/Warsaw', 'en', 0, ?, ?)",
                (NOW, NOW),
            )
            old.execute(
                "INSERT INTO source_profile (id, name, is_active, created_at) VALUES (1, 'canonical/1', 1, ?)",
                (NOW,),
            )
            old.commit()
            old.executescript(
                (SCHEMA_DIRECTORY / "008_inventory.sql").read_text(encoding="utf-8")
            )
            tables = {
                row[0]
                for row in old.execute(
                    "SELECT name FROM sqlite_master WHERE type = 'table'"
                ).fetchall()
            }
            self.assertTrue(
                {
                    "inventory_snapshot",
                    "inventory_stock",
                    "inventory_receipt",
                    "inventory_policy",
                    "inventory_source_file",
                }
                <= tables
            )
            self.assertIn("inventory_snapshot_id", column_names(old, "export_record"))
            assert_integrity_ok(self, old)
            assert_fk_check_empty(self, old)
            old.close()

    def test_migration_eight_lives_next_to_001_which_stays_frozen(self):
        self.assertTrue((SCHEMA_DIRECTORY / "008_inventory.sql").exists())
        fingerprint = read_hash_file()
        self.assertIsNotNone(fingerprint)
        recorded_sha, recorded_name = fingerprint
        one_sha = hashlib.sha256(
            (SCHEMA_DIRECTORY / "001_init.sql").read_bytes()
        ).hexdigest()
        self.assertEqual(one_sha, recorded_sha)
        self.assertEqual(recorded_name, "001_init.sql")


if __name__ == "__main__":
    unittest.main()
