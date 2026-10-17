"""Integration tests for migration 009 (sparse-model registry rebuild).

Apply 001..009 in filename order, exactly as the Rust migrator does, then
assert the extended model vocabulary, that existing run_model rows survive the
table rebuild, that foreign keys stay intact, and that migration 001 stays
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
    "009_sparse_models.sql",
]


def apply_migrations(conn: sqlite3.Connection) -> None:
    apply_schema(conn)
    for name in MIGRATION_FILES:
        conn.executescript((SCHEMA_DIRECTORY / name).read_text(encoding="utf-8"))


class MigrationNineTest(unittest.TestCase):
    def setUp(self) -> None:
        self._tmpdir = tempfile.TemporaryDirectory(prefix="opendemand-m009-")
        self.db_path = os.path.join(self._tmpdir.name, "workspace.sqlite3")
        self.conn = connect(self.db_path)
        apply_migrations(self.conn)

    def tearDown(self) -> None:
        self.conn.close()
        self._tmpdir.cleanup()

    def _seed_run(self) -> bytes:
        run_id = bytes.fromhex("1112131415161718191a1b1c1d1e1f20")
        self.conn.execute(
            "INSERT INTO source_profile (id, name, is_active, created_at) VALUES (1, 'canonical/1', 1, ?)",
            (NOW,),
        )
        self.conn.execute(
            "INSERT INTO import_revision (id, profile_id, content_hash, mapping_fingerprint, contract_version, source_filename, imported_at, row_count, observation_count, issue_count)"
            " VALUES (1, 1, 'sha256:content', 'sha256:mapping', 'canonical/1', 'source.csv', ?, 1, 1, 0)",
            (NOW,),
        )
        self.conn.execute(
            "INSERT INTO forecast_run (id, revision_id, created_at, finished_at, status, horizon_weeks, horizon_start_week, timezone, config_json, app_version, engine_version)"
            " VALUES (?, 1, ?, ?, 'completed', 12, '2026-03-02', 'UTC', '{}', 'test', 'test')",
            (run_id, NOW, NOW),
        )
        return run_id

    def test_sparse_model_ids_are_accepted(self):
        run_id = self._seed_run()
        for model in ("croston_sba", "tsb"):
            self.conn.execute(
                "INSERT INTO run_model (run_id, model_id, status) VALUES (?, ?, 'not_run')",
                (run_id, model),
            )
        with self.assertRaises(sqlite3.IntegrityError):
            self.conn.execute(
                "INSERT INTO run_model (run_id, model_id, status) VALUES (?, 'croston', 'not_run')",
                (run_id,),
            )

    def test_rebuild_preserves_existing_rows_and_foreign_keys(self):
        # Build a version-8 database with a real completed run, then upgrade.
        with tempfile.TemporaryDirectory(prefix="opendemand-m009-old-") as directory:
            old_path = os.path.join(directory, "workspace.sqlite3")
            old = connect(old_path)
            apply_schema(old)
            for name in MIGRATION_FILES[:7]:
                old.executescript((SCHEMA_DIRECTORY / name).read_text(encoding="utf-8"))
            old.execute(
                "INSERT INTO workspace_settings (id, schema_version, timezone, locale, mcp_enabled, created_at, updated_at)"
                " VALUES (1, '8', 'Europe/Warsaw', 'en', 0, ?, ?)",
                (NOW, NOW),
            )
            old.execute(
                "INSERT INTO source_profile (id, name, is_active, created_at) VALUES (1, 'canonical/1', 1, ?)",
                (NOW,),
            )
            run_id = bytes.fromhex("2122232425262728292a2b2c2d2e2f30")
            product_id = bytes.fromhex("3132333435363738393a3b3c3d3e3f40")
            location_id = bytes.fromhex("4142434445464748494a4b4c4d4e4f50")
            old.execute(
                "INSERT INTO import_revision (id, profile_id, content_hash, mapping_fingerprint, contract_version, source_filename, imported_at, row_count, observation_count, issue_count)"
                " VALUES (1, 1, 'sha256:content', 'sha256:mapping', 'canonical/1', 'source.csv', ?, 1, 1, 0)",
                (NOW,),
            )
            old.execute(
                "INSERT INTO forecast_run (id, revision_id, created_at, finished_at, status, horizon_weeks, horizon_start_week, timezone, config_json, app_version, engine_version)"
                " VALUES (?, 1, ?, ?, 'completed', 12, '2026-03-02', 'UTC', '{}', 'test', 'test')",
                (run_id, NOW, NOW),
            )
            old.execute(
                "INSERT INTO run_model (run_id, model_id, status, tasks_planned, tasks_completed, tasks_failed)"
                " VALUES (?, 'naive', 'completed', 3, 3, 0)",
                (run_id,),
            )
            old.execute(
                "INSERT INTO series_model_evaluation (run_id, product_id, location_id, model_id, eligibility_status, origins_planned, origins_completed, is_champion, production_status)"
                " VALUES (?, ?, ?, 'naive', 'champion_eligible', 2, 2, 1, 'completed')",
                (run_id, product_id, location_id),
            )
            old.commit()
            old.executescript(
                (SCHEMA_DIRECTORY / "009_sparse_models.sql").read_text(encoding="utf-8")
            )
            row = old.execute(
                "SELECT status, tasks_completed FROM run_model WHERE run_id = ? AND model_id = 'naive'",
                (run_id,),
            ).fetchone()
            self.assertEqual(row, ("completed", 3))
            child = old.execute(
                "SELECT is_champion FROM series_model_evaluation WHERE run_id = ?",
                (run_id,),
            ).fetchone()
            self.assertEqual(child, (1,))
            old.execute(
                "INSERT INTO run_model (run_id, model_id, status) VALUES (?, 'croston_sba', 'not_run')",
                (run_id,),
            )
            assert_integrity_ok(self, old)
            assert_fk_check_empty(self, old)
            old.close()

    def test_schema_stays_consistent(self):
        assert_integrity_ok(self, self.conn)
        assert_fk_check_empty(self, self.conn)

    def test_migration_nine_lives_next_to_001_which_stays_frozen(self):
        self.assertTrue((SCHEMA_DIRECTORY / "009_sparse_models.sql").exists())
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
