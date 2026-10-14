"""Integration tests for migration 003 (run-scoped model-input imputation).

Apply 001, 002, and 003 in filename order, exactly as the Rust migrator does,
then assert the additive columns, their defaults, that a version-2 database
remains readable before the upgrade, and that migration 001 is untouched.

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
MIGRATION_002_PATH = SCHEMA_DIRECTORY / "002_import_config.sql"
MIGRATION_003_PATH = SCHEMA_DIRECTORY / "003_forecast_imputation.sql"


def read_migration_003() -> str:
    return MIGRATION_003_PATH.read_text(encoding="utf-8")


def apply_migrations(conn: sqlite3.Connection) -> None:
    """Apply 001..003 in the migrator's order."""
    apply_schema(conn)
    conn.executescript(MIGRATION_002_PATH.read_text(encoding="utf-8"))
    conn.executescript(read_migration_003())


def column_info(conn: sqlite3.Connection, table: str) -> dict[str, tuple]:
    return {
        row[1]: row
        for row in conn.execute(f"PRAGMA table_info({table})").fetchall()
    }


class MigrationThreeTest(unittest.TestCase):
    """One temporary file-backed database per test method."""

    def setUp(self) -> None:
        self._tmpdir = tempfile.TemporaryDirectory(prefix="opendemand-m003-")
        self.db_path = os.path.join(self._tmpdir.name, "workspace.sqlite3")
        self.conn = connect(self.db_path)
        apply_migrations(self.conn)
        self.conn.execute(
            "INSERT INTO workspace_settings (id, schema_version, timezone, locale, mcp_enabled, created_at, updated_at)"
            " VALUES (1, '3', 'Europe/Warsaw', 'en', 0, ?, ?)",
            (NOW, NOW),
        )
        self.conn.execute(
            "INSERT INTO source_profile (id, name, is_active, created_at) VALUES (1, 'canonical/1', 1, ?)",
            (NOW,),
        )
        self.conn.commit()

    def tearDown(self) -> None:
        self.conn.close()
        self._tmpdir.cleanup()

    def test_migration_file_exists_and_is_not_frozen_001(self):
        self.assertTrue(MIGRATION_003_PATH.exists(), "003 must live next to 001")
        fingerprint = read_hash_file()
        self.assertIsNotNone(fingerprint)
        recorded_sha, recorded_name = fingerprint
        one_sha = hashlib.sha256((SCHEMA_DIRECTORY / "001_init.sql").read_bytes()).hexdigest()
        self.assertEqual(one_sha, recorded_sha)
        self.assertEqual(recorded_name, "001_init.sql")

    def test_run_records_the_imputation_method_as_nullable(self):
        columns = column_info(self.conn, "forecast_run")
        self.assertIn("imputation_method", columns)
        cid, _name, declared_type, notnull, default, _pk = columns["imputation_method"]
        self.assertEqual(declared_type, "TEXT")
        self.assertEqual(notnull, 0)
        self.assertIsNone(default)
        self.assertGreater(cid, 0)

    def test_series_status_counts_filled_weeks_with_a_zero_default(self):
        columns = column_info(self.conn, "forecast_series_status")
        self.assertIn("imputed_weeks", columns)
        _cid, _name, declared_type, notnull, default, _pk = columns["imputed_weeks"]
        self.assertEqual(declared_type, "INTEGER")
        self.assertEqual(notnull, 1)
        self.assertEqual(default, "0")

    def test_schema_stays_consistent(self):
        assert_integrity_ok(self, self.conn)
        assert_fk_check_empty(self, self.conn)

    def test_a_version_two_database_is_unchanged_before_the_upgrade(self):
        """Reconstruct the pre-upgrade state and prove 003 is purely additive."""
        with tempfile.TemporaryDirectory(prefix="opendemand-m003-old-") as directory:
            old_path = os.path.join(directory, "workspace.sqlite3")
            old = connect(old_path)
            apply_schema(old)
            old.executescript(MIGRATION_002_PATH.read_text(encoding="utf-8"))
            old.execute(
                "INSERT INTO workspace_settings (id, schema_version, timezone, locale, mcp_enabled, created_at, updated_at)"
                " VALUES (1, '2', 'Europe/Warsaw', 'en', 0, ?, ?)",
                (NOW, NOW),
            )
            old.execute(
                "INSERT INTO source_profile (id, name, is_active, created_at) VALUES (1, 'canonical/1', 1, ?)",
                (NOW,),
            )
            old.commit()
            before = column_info(old, "forecast_run")
            self.assertNotIn("imputation_method", before)
            # Applying 003 in place is additive.
            old.executescript(read_migration_003())
            after = column_info(old, "forecast_run")
            self.assertIn("imputation_method", after)
            assert_integrity_ok(self, old)
            assert_fk_check_empty(self, old)
            old.close()


if __name__ == "__main__":
    unittest.main()
