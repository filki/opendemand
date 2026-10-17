"""Integration tests for migration 006 (immutable plan publications).

Apply 001..006 in filename order, exactly as the Rust migrator does, then
assert the additive tables, their immutability triggers, the override pin
foreign key, and that a version-5 database upgrades additively while
migration 001 stays untouched.

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
    HORIZON_WEEKS,
    LOCATION_ID,
    NOW,
    PRODUCT_A,
    RUN_ID,
    FullChainFixture,
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
]


def apply_migrations(conn: sqlite3.Connection) -> None:
    """Apply 001..006 in the migrator's order."""
    apply_schema(conn)
    for name in MIGRATION_FILES:
        conn.executescript((SCHEMA_DIRECTORY / name).read_text(encoding="utf-8"))


def column_names(conn: sqlite3.Connection, table: str) -> set:
    return {row[1] for row in conn.execute(f"PRAGMA table_info({table})").fetchall()}


class MigrationSixTest(unittest.TestCase):
    """One temporary file-backed database per test method."""

    def setUp(self) -> None:
        self._tmpdir = tempfile.TemporaryDirectory(prefix="opendemand-m006-")
        self.db_path = os.path.join(self._tmpdir.name, "workspace.sqlite3")
        self.conn = connect(self.db_path)
        apply_migrations(self.conn)
        FullChainFixture(self.conn).build()

    def tearDown(self) -> None:
        self.conn.close()
        self._tmpdir.cleanup()

    def _publish(self) -> int:
        cursor = self.conn.execute(
            "INSERT INTO plan_publication (profile_id, run_id, revision_id, cutoff_week, horizon_start_week, revision_high_water, timezone, policy_context, published_at)"
            " VALUES (1, ?, 1, '2026-02-23', ?, 1, 'UTC', 'publication/1', ?)",
            (RUN_ID, HORIZON_WEEKS[0], NOW),
        )
        return cursor.lastrowid

    def test_publication_tables_carry_their_documented_columns(self):
        self.assertTrue(
            {
                "id",
                "profile_id",
                "run_id",
                "revision_id",
                "cutoff_week",
                "horizon_start_week",
                "revision_high_water",
                "timezone",
                "policy_context",
                "published_at",
            }
            <= column_names(self.conn, "plan_publication")
        )
        self.assertTrue(
            {"publication_id", "run_id", "product_id", "location_id", "week_start", "revision"}
            <= column_names(self.conn, "publication_override")
        )
        self.assertIn("publication_id", column_names(self.conn, "export_record"))

    def test_one_publication_per_profile_and_cutoff(self):
        self._publish()
        with self.assertRaises(sqlite3.IntegrityError):
            self._publish()

    def test_publications_are_immutable(self):
        publication_id = self._publish()
        with self.assertRaises(sqlite3.IntegrityError):
            self.conn.execute("UPDATE plan_publication SET cutoff_week = '2026-01-05'")
        with self.assertRaises(sqlite3.IntegrityError):
            self.conn.execute("DELETE FROM plan_publication")
        # Pinned override revisions: valid pins insert; a bogus revision fails.
        self.conn.execute(
            "INSERT INTO publication_override (publication_id, run_id, product_id, location_id, week_start, revision)"
            " VALUES (?, ?, ?, ?, ?, 1)",
            (publication_id, RUN_ID, PRODUCT_A, LOCATION_ID, HORIZON_WEEKS[0]),
        )
        with self.assertRaises(sqlite3.IntegrityError):
            self.conn.execute(
                "INSERT INTO publication_override (publication_id, run_id, product_id, location_id, week_start, revision)"
                " VALUES (?, ?, ?, ?, ?, 2)",
                (publication_id, RUN_ID, PRODUCT_A, LOCATION_ID, HORIZON_WEEKS[1]),
            )
        with self.assertRaises(sqlite3.IntegrityError):
            self.conn.execute("UPDATE publication_override SET revision = 2")
        with self.assertRaises(sqlite3.IntegrityError):
            self.conn.execute("DELETE FROM publication_override")

    def test_schema_stays_consistent(self):
        assert_integrity_ok(self, self.conn)
        assert_fk_check_empty(self, self.conn)

    def test_a_version_five_database_upgrades_additively(self):
        with tempfile.TemporaryDirectory(prefix="opendemand-m006-old-") as directory:
            old_path = os.path.join(directory, "workspace.sqlite3")
            old = connect(old_path)
            apply_schema(old)
            for name in MIGRATION_FILES[:4]:
                old.executescript((SCHEMA_DIRECTORY / name).read_text(encoding="utf-8"))
            old.execute(
                "INSERT INTO workspace_settings (id, schema_version, timezone, locale, mcp_enabled, created_at, updated_at)"
                " VALUES (1, '5', 'Europe/Warsaw', 'en', 0, ?, ?)",
                (NOW, NOW),
            )
            old.execute(
                "INSERT INTO source_profile (id, name, is_active, created_at) VALUES (1, 'canonical/1', 1, ?)",
                (NOW,),
            )
            old.commit()
            old.executescript(
                (SCHEMA_DIRECTORY / "006_plan_publication.sql").read_text(encoding="utf-8")
            )
            tables = {
                row[0]
                for row in old.execute(
                    "SELECT name FROM sqlite_master WHERE type = 'table'"
                ).fetchall()
            }
            self.assertTrue({"plan_publication", "publication_override"} <= tables)
            self.assertIn("publication_id", column_names(old, "export_record"))
            count = old.execute("SELECT COUNT(*) FROM source_profile").fetchone()[0]
            self.assertEqual(count, 1)
            assert_integrity_ok(self, old)
            assert_fk_check_empty(self, old)
            old.close()

    def test_migration_six_lives_next_to_001_which_stays_frozen(self):
        self.assertTrue(
            (SCHEMA_DIRECTORY / "006_plan_publication.sql").exists(),
            "006 must live next to 001",
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
