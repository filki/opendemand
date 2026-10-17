"""Integration tests for migration 007 (actuals confirmations and bias policy).

Apply 001..007 in filename order, exactly as the Rust migrator does, then
assert the append-only confirmation table, the added review-policy columns,
and that a version-6 database upgrades additively while migration 001 stays
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
]


def apply_migrations(conn: sqlite3.Connection) -> None:
    """Apply 001..007 in the migrator's order."""
    apply_schema(conn)
    for name in MIGRATION_FILES:
        conn.executescript((SCHEMA_DIRECTORY / name).read_text(encoding="utf-8"))


def column_names(conn: sqlite3.Connection, table: str) -> set:
    return {row[1] for row in conn.execute(f"PRAGMA table_info({table})").fetchall()}


class MigrationSevenTest(unittest.TestCase):
    """One temporary file-backed database per test method."""

    def setUp(self) -> None:
        self._tmpdir = tempfile.TemporaryDirectory(prefix="opendemand-m007-")
        self.db_path = os.path.join(self._tmpdir.name, "workspace.sqlite3")
        self.conn = connect(self.db_path)
        apply_migrations(self.conn)
        self.conn.execute(
            "INSERT INTO source_profile (id, name, is_active, created_at) VALUES (1, 'canonical/1', 1, ?)",
            (NOW,),
        )
        self.conn.commit()

    def tearDown(self) -> None:
        self.conn.close()
        self._tmpdir.cleanup()

    def test_confirmation_table_carries_its_documented_columns(self):
        self.assertTrue(
            {"id", "profile_id", "revision_id", "confirmed_through", "created_at"}
            <= column_names(self.conn, "actuals_confirmation")
        )
        self.assertTrue(
            {"bias_horizon", "bias_window", "bias_min", "bias_abs"}
            <= column_names(self.conn, "review_policy")
        )

    def test_confirmations_are_append_only(self):
        # revision_id has a real foreign key; use a minimal revision chain.
        self.conn.execute(
            "INSERT INTO import_revision (id, profile_id, content_hash, mapping_fingerprint, contract_version, source_filename, imported_at, row_count, observation_count, issue_count)"
            " VALUES (1, 1, 'sha256:content', 'sha256:mapping', 'canonical/1', 'source.csv', ?, 1, 1, 0)",
            (NOW,),
        )
        self.conn.execute(
            "INSERT INTO actuals_confirmation (profile_id, revision_id, confirmed_through, created_at)"
            " VALUES (1, 1, '2026-01-05', ?)",
            (NOW,),
        )
        with self.assertRaises(sqlite3.IntegrityError):
            self.conn.execute("UPDATE actuals_confirmation SET confirmed_through = '2026-01-12'")
        with self.assertRaises(sqlite3.IntegrityError):
            self.conn.execute("DELETE FROM actuals_confirmation")

    def test_bias_columns_enforce_their_ranges(self):
        self.conn.execute(
            "INSERT INTO review_policy (profile_id, version, change_abs, change_rel, created_at)"
            " VALUES (1, 1, NULL, NULL, ?)",
            (NOW,),
        )
        with self.assertRaises(sqlite3.IntegrityError):
            self.conn.execute(
                "INSERT INTO review_policy (profile_id, version, bias_horizon, created_at)"
                " VALUES (1, 2, 13, ?)",
                (NOW,),
            )
        with self.assertRaises(sqlite3.IntegrityError):
            self.conn.execute(
                "INSERT INTO review_policy (profile_id, version, bias_abs, created_at)"
                " VALUES (1, 3, 0.0, ?)",
                (NOW,),
            )

    def test_schema_stays_consistent(self):
        assert_integrity_ok(self, self.conn)
        assert_fk_check_empty(self, self.conn)

    def test_a_version_six_database_upgrades_additively(self):
        with tempfile.TemporaryDirectory(prefix="opendemand-m007-old-") as directory:
            old_path = os.path.join(directory, "workspace.sqlite3")
            old = connect(old_path)
            apply_schema(old)
            for name in MIGRATION_FILES[:5]:
                old.executescript((SCHEMA_DIRECTORY / name).read_text(encoding="utf-8"))
            old.execute(
                "INSERT INTO workspace_settings (id, schema_version, timezone, locale, mcp_enabled, created_at, updated_at)"
                " VALUES (1, '6', 'Europe/Warsaw', 'en', 0, ?, ?)",
                (NOW, NOW),
            )
            old.execute(
                "INSERT INTO source_profile (id, name, is_active, created_at) VALUES (1, 'canonical/1', 1, ?)",
                (NOW,),
            )
            old.execute(
                "INSERT INTO review_policy (profile_id, version, change_abs, change_rel, created_at)"
                " VALUES (1, 1, 25.0, 0.25, ?)",
                (NOW,),
            )
            old.commit()
            old.executescript(
                (SCHEMA_DIRECTORY / "007_actuals_confirmations.sql").read_text(encoding="utf-8")
            )
            tables = {
                row[0]
                for row in old.execute(
                    "SELECT name FROM sqlite_master WHERE type = 'table'"
                ).fetchall()
            }
            self.assertIn("actuals_confirmation", tables)
            self.assertTrue(
                {"bias_horizon", "bias_window", "bias_min", "bias_abs"}
                <= column_names(old, "review_policy")
            )
            # The pre-existing policy version survives the column additions.
            row = old.execute(
                "SELECT change_abs, change_rel, bias_horizon FROM review_policy WHERE version = 1"
            ).fetchone()
            self.assertEqual(row, (25.0, 0.25, None))
            assert_integrity_ok(self, old)
            assert_fk_check_empty(self, old)
            old.close()

    def test_migration_seven_lives_next_to_001_which_stays_frozen(self):
        self.assertTrue(
            (SCHEMA_DIRECTORY / "007_actuals_confirmations.sql").exists(),
            "007 must live next to 001",
        )
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
