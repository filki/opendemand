"""Integration tests for migration 002 (import configurations and the frozen
parsing descriptor).

Apply 001 and 002 in filename order, exactly as the Rust migrator does, then
assert the additive schema, its constraints, that a version-1 database remains
readable before the upgrade, and that migration 001 is untouched.

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
    LOCATION_ID,
    PRODUCT_A,
    apply_schema,
    assert_fk_check_empty,
    assert_integrity_ok,
    connect,
    read_hash_file,
    table_names,
)

SCHEMA_DIRECTORY = (
    Path(__file__).resolve().parents[2] / "docs" / "architecture" / "schema"
)
MIGRATION_002_PATH = SCHEMA_DIRECTORY / "002_import_config.sql"


def read_migration_002() -> str:
    return MIGRATION_002_PATH.read_text(encoding="utf-8")


def apply_migrations(conn: sqlite3.Connection) -> None:
    """Apply 001 and 002 in the migrator's order."""
    apply_schema(conn)
    conn.executescript(read_migration_002())


class MigrationTwoTest(unittest.TestCase):
    """One temporary file-backed database per test method."""

    def setUp(self) -> None:
        self._tmpdir = tempfile.TemporaryDirectory(prefix="opendemand-m002-")
        self.db_path = os.path.join(self._tmpdir.name, "workspace.sqlite3")
        self.conn = connect(self.db_path)
        apply_migrations(self.conn)
        self.conn.execute(
            "INSERT INTO workspace_settings (id, schema_version, timezone, locale, mcp_enabled, created_at, updated_at)"
            " VALUES (1, '2', 'Europe/Warsaw', 'en', 0, ?, ?)",
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
        self.assertTrue(MIGRATION_002_PATH.exists(), "002 must live next to 001")
        # The frozen 001 fingerprint must not cover the new file.
        fingerprint = read_hash_file()
        self.assertIsNotNone(fingerprint)
        recorded_sha, recorded_name = fingerprint
        one_sha = hashlib.sha256((SCHEMA_DIRECTORY / "001_init.sql").read_bytes()).hexdigest()
        self.assertEqual(one_sha, recorded_sha)
        self.assertEqual(recorded_name, "001_init.sql")

    def test_additive_schema_is_present_and_intact(self):
        names = table_names(self.conn)
        self.assertIn("import_config", names)
        columns = {
            row[1]
            for row in self.conn.execute("PRAGMA table_info(import_revision)").fetchall()
        }
        self.assertIn("descriptor_json", columns)
        assert_integrity_ok(self, self.conn)
        assert_fk_check_empty(self, self.conn)

    def test_canonical_revisions_keep_a_null_descriptor(self):
        self.conn.execute(
            "INSERT INTO import_revision (id, profile_id, content_hash, mapping_fingerprint,"
            " contract_version, source_filename, imported_at, row_count, observation_count, issue_count)"
            " VALUES (1, 1, 'sha256:content', 'sha256:mapping', 'canonical/1', 'source.csv', ?, 1, 1, 0)",
            (NOW,),
        )
        row = self.conn.execute(
            "SELECT descriptor_json FROM import_revision WHERE id = 1"
        ).fetchone()
        self.assertIsNone(row[0])
        assert_fk_check_empty(self, self.conn)

    def test_preprocessed_revisions_carry_a_frozen_descriptor(self):
        self.conn.execute(
            "INSERT INTO import_revision (id, profile_id, content_hash, mapping_fingerprint,"
            " contract_version, source_filename, imported_at, row_count, observation_count,"
            " issue_count, descriptor_json)"
            " VALUES (1, 1, 'sha256:content', 'sha256:mapping', 'preprocess/1', 'subiekt.csv',"
            " ?, 101, 24, 6, '{\"descriptor_version\":1,\"delimiter\":\";\"}')",
            (NOW,),
        )
        row = self.conn.execute(
            "SELECT contract_version, descriptor_json FROM import_revision WHERE id = 1"
        ).fetchone()
        self.assertEqual(row[0], "preprocess/1")
        self.assertIn('"descriptor_version":1', row[1])

    def test_configuration_names_are_unique_per_profile(self):
        self.conn.execute(
            "INSERT INTO import_config (profile_id, name, descriptor_json, created_at, updated_at)"
            " VALUES (1, 'Subiekt monthly', '{}', ?, ?)",
            (NOW, NOW),
        )
        with self.assertRaises(sqlite3.IntegrityError):
            self.conn.execute(
                "INSERT INTO import_config (profile_id, name, descriptor_json, created_at, updated_at)"
                " VALUES (1, 'Subiekt monthly', '{}', ?, ?)",
                (NOW, NOW),
            )

    def test_configuration_requires_a_known_profile(self):
        with self.assertRaises(sqlite3.IntegrityError):
            self.conn.execute(
                "INSERT INTO import_config (profile_id, name, descriptor_json, created_at, updated_at)"
                " VALUES (99, 'orphan', '{}', ?, ?)",
                (NOW, NOW),
            )

    def test_updating_a_configuration_never_touches_revisions(self):
        self.conn.execute(
            "INSERT INTO import_revision (id, profile_id, content_hash, mapping_fingerprint,"
            " contract_version, source_filename, imported_at, row_count, observation_count,"
            " issue_count, descriptor_json)"
            " VALUES (1, 1, 'sha256:content', 'sha256:mapping', 'preprocess/1', 'subiekt.csv',"
            " ?, 1, 1, 0, '{\"delimiter\":\";\"}')",
            (NOW,),
        )
        self.conn.execute(
            "INSERT INTO import_config (id, profile_id, name, descriptor_json, created_at, updated_at)"
            " VALUES (1, 1, 'Subiekt monthly', '{\"delimiter\":\";\"}', ?, ?)",
            (NOW, NOW),
        )
        self.conn.execute(
            "UPDATE import_config SET descriptor_json = '{\"delimiter\":\",\"}', updated_at = ? WHERE id = 1",
            (NOW,),
        )
        row = self.conn.execute(
            "SELECT descriptor_json FROM import_revision WHERE id = 1"
        ).fetchone()
        self.assertEqual(row[0], '{"delimiter":";"}')
        assert_fk_check_empty(self, self.conn)

    def test_a_version_one_database_is_unchanged_before_the_upgrade(self):
        """Reconstruct the pre-upgrade state and prove 002 is purely additive."""
        with tempfile.TemporaryDirectory(prefix="opendemand-m002-old-") as directory:
            old_path = os.path.join(directory, "workspace.sqlite3")
            old = connect(old_path)
            apply_schema(old)
            old.execute(
                "INSERT INTO workspace_settings (id, schema_version, timezone, locale, mcp_enabled, created_at, updated_at)"
                " VALUES (1, '1', 'Europe/Warsaw', 'en', 0, ?, ?)",
                (NOW, NOW),
            )
            old.execute(
                "INSERT INTO source_profile (id, name, is_active, created_at) VALUES (1, 'canonical/1', 1, ?)",
                (NOW,),
            )
            old.commit()
            assert_integrity_ok(self, old)
            before = table_names(old)
            self.assertNotIn("import_config", before)
            # Applying 002 in place is additive.
            old.executescript(read_migration_002())
            after = table_names(old)
            self.assertIn("import_config", after)
            assert_integrity_ok(self, old)
            assert_fk_check_empty(self, old)
            old.close()


if __name__ == "__main__":
    unittest.main()
