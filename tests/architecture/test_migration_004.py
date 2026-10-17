"""Integration tests for migration 004 (planner decision memory).

Apply 001..004 in filename order, exactly as the Rust migrator does, then assert
the additive tables, their guardrail triggers, append-only behavior, and that a
version-3 database upgrades additively while migration 001 stays untouched.

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
]


def apply_migrations(conn: sqlite3.Connection) -> None:
    """Apply 001..004 in the migrator's order."""
    apply_schema(conn)
    for name in MIGRATION_FILES:
        conn.executescript((SCHEMA_DIRECTORY / name).read_text(encoding="utf-8"))


def column_names(conn: sqlite3.Connection, table: str) -> set:
    return {row[1] for row in conn.execute(f"PRAGMA table_info({table})").fetchall()}


class MigrationFourTest(unittest.TestCase):
    """One temporary file-backed database per test method."""

    def setUp(self) -> None:
        self._tmpdir = tempfile.TemporaryDirectory(prefix="opendemand-m004-")
        self.db_path = os.path.join(self._tmpdir.name, "workspace.sqlite3")
        self.conn = connect(self.db_path)
        apply_migrations(self.conn)
        FullChainFixture(self.conn).build()

    def tearDown(self) -> None:
        self.conn.close()
        self._tmpdir.cleanup()

    def _insert_override_meta(self, revision: int, reason: str, review_date=None):
        return self.conn.execute(
            "INSERT INTO override_meta (run_id, product_id, location_id, week_start, revision, reason_code, review_date, created_at)"
            " VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            (
                RUN_ID,
                PRODUCT_A,
                LOCATION_ID,
                HORIZON_WEEKS[0],
                revision,
                reason,
                review_date,
                NOW,
            ),
        )

    def test_decision_tables_carry_their_documented_columns(self):
        self.assertTrue(
            {"id", "profile_id", "product_id", "location_id", "note", "context_run_id", "created_at"}
            <= column_names(self.conn, "series_note")
        )
        self.assertTrue(
            {
                "id",
                "profile_id",
                "product_id",
                "location_id",
                "run_id",
                "action",
                "evidence_context",
                "evidence_fingerprint",
                "created_at",
            }
            <= column_names(self.conn, "series_review")
        )
        self.assertTrue(
            {"run_id", "product_id", "location_id", "week_start", "revision",
             "reason_code", "review_date", "created_at"}
            <= column_names(self.conn, "override_meta")
        )

    def test_override_meta_requires_the_matching_override_revision(self):
        with self.assertRaises(sqlite3.IntegrityError):
            self._insert_override_meta(2, "promotion")

    def test_override_meta_other_reason_requires_a_note(self):
        # The fixture override carries the note 'first override', so this one
        # passes; a revision without a note must fail.
        self.conn.execute(
            "INSERT INTO forecast_override (run_id, product_id, location_id, week_start, revision, value, note, created_at)"
            " VALUES (?, ?, ?, ?, 2, 3.5, NULL, ?)",
            (RUN_ID, PRODUCT_A, LOCATION_ID, HORIZON_WEEKS[0], NOW),
        )
        with self.assertRaises(sqlite3.IntegrityError):
            self._insert_override_meta(2, "other")
        self._insert_override_meta(1, "other")  # revision 1 has a note

    def test_override_meta_is_immutable_and_append_only(self):
        self._insert_override_meta(1, "promotion", "2026-06-01")
        with self.assertRaises(sqlite3.IntegrityError):
            self.conn.execute("UPDATE override_meta SET reason_code = 'data_quality'")
        with self.assertRaises(sqlite3.IntegrityError):
            self.conn.execute("DELETE FROM override_meta")

    def test_override_meta_enforces_the_reason_vocabulary(self):
        with self.assertRaises(sqlite3.IntegrityError):
            self._insert_override_meta(1, "guessed_cause")

    def test_series_notes_are_append_only(self):
        self.conn.execute(
            "INSERT INTO series_note (profile_id, product_id, location_id, note, context_run_id, created_at)"
            " VALUES (1, ?, ?, 'Watch week 12.', ?, ?)",
            (PRODUCT_A, LOCATION_ID, RUN_ID, NOW),
        )
        with self.assertRaises(sqlite3.IntegrityError):
            self.conn.execute("UPDATE series_note SET note = 'rewritten'")
        with self.assertRaises(sqlite3.IntegrityError):
            self.conn.execute("DELETE FROM series_note")
        with self.assertRaises(sqlite3.IntegrityError):
            self.conn.execute(
                "INSERT INTO series_note (profile_id, product_id, location_id, note, created_at)"
                " VALUES (1, ?, ?, '   ', ?)",
                (PRODUCT_A, LOCATION_ID, NOW),
            )

    def test_series_reviews_require_a_run_and_an_append_only_event(self):
        self.conn.execute(
            "INSERT INTO series_review (profile_id, product_id, location_id, run_id, action, evidence_context, evidence_fingerprint, created_at)"
            " VALUES (1, ?, ?, ?, 'reviewed', 'review/1', ?, ?)",
            (PRODUCT_A, LOCATION_ID, RUN_ID, "a" * 64, NOW),
        )
        with self.assertRaises(sqlite3.IntegrityError):
            self.conn.execute(
                "INSERT INTO series_review (profile_id, product_id, location_id, run_id, action, evidence_context, evidence_fingerprint, created_at)"
                " VALUES (1, ?, ?, ?, 'approved', 'review/1', 'b', ?)",
                (PRODUCT_A, LOCATION_ID, RUN_ID, NOW),
            )
        with self.assertRaises(sqlite3.IntegrityError):
            self.conn.execute(
                "INSERT INTO series_review (profile_id, product_id, location_id, run_id, action, evidence_context, evidence_fingerprint, created_at)"
                " VALUES (1, ?, ?, ?, 'reviewed', 'review/1', 'c', ?)",
                (PRODUCT_A, LOCATION_ID, b"\x01" * 16, NOW),
            )
        with self.assertRaises(sqlite3.IntegrityError):
            self.conn.execute("UPDATE series_review SET action = 'reopened'")
        with self.assertRaises(sqlite3.IntegrityError):
            self.conn.execute("DELETE FROM series_review")

    def test_schema_stays_consistent(self):
        assert_integrity_ok(self, self.conn)
        assert_fk_check_empty(self, self.conn)

    def test_a_version_three_database_upgrades_additively(self):
        """Reconstruct a version-3 database and prove 004 adds only new tables."""
        with tempfile.TemporaryDirectory(prefix="opendemand-m004-old-") as directory:
            old_path = os.path.join(directory, "workspace.sqlite3")
            old = connect(old_path)
            apply_schema(old)
            for name in MIGRATION_FILES[:2]:
                old.executescript((SCHEMA_DIRECTORY / name).read_text(encoding="utf-8"))
            old.execute(
                "INSERT INTO workspace_settings (id, schema_version, timezone, locale, mcp_enabled, created_at, updated_at)"
                " VALUES (1, '3', 'Europe/Warsaw', 'en', 0, ?, ?)",
                (NOW, NOW),
            )
            old.execute(
                "INSERT INTO source_profile (id, name, is_active, created_at) VALUES (1, 'canonical/1', 1, ?)",
                (NOW,),
            )
            old.execute(
                "INSERT INTO product (id, sku, name, created_at) VALUES (?, 'SKU-A', 'Product A', ?)",
                (PRODUCT_A, NOW),
            )
            old.commit()
            before = column_names(old, "forecast_run")
            self.assertNotIn("series_note", column_names(old, "product"))
            old.executescript(
                (SCHEMA_DIRECTORY / "004_review_decisions.sql").read_text(encoding="utf-8")
            )
            after = {
                row[0]
                for row in old.execute(
                    "SELECT name FROM sqlite_master WHERE type = 'table'"
                ).fetchall()
            }
            self.assertTrue({"series_note", "series_review", "override_meta"} <= after)
            self.assertEqual(before, column_names(old, "forecast_run"))
            count = old.execute("SELECT COUNT(*) FROM product").fetchone()[0]
            self.assertEqual(count, 1)
            assert_integrity_ok(self, old)
            assert_fk_check_empty(self, old)
            old.close()

    def test_migration_four_lives_next_to_001_which_stays_frozen(self):
        self.assertTrue(
            (SCHEMA_DIRECTORY / "004_review_decisions.sql").exists(),
            "004 must live next to 001",
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
