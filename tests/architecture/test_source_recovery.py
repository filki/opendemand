"""Tests for the retained-source file-first crash-recovery protocol.

These tests exercise a Python harness that models the documented protocol
(stage -> fsync -> rename -> insert path -> commit) on a real temporary
workspace with a real file-backed SQLite database. They verify startup
reconciliation, retry idempotency after a lost response, and backup
fail-closed behavior. The harness is a test model of the protocol; it does not
claim that the Rust implementation exists yet.
"""

from __future__ import annotations

import hashlib
import os
import sqlite3
import tempfile
import unittest
import uuid
from dataclasses import dataclass, field
from pathlib import Path

from tests.architecture.migration_harness import NOW, connect, read_schema

PROFILE_ID = 1


def fsync_dir(path: Path) -> None:
    if hasattr(os, "O_DIRECTORY") and hasattr(os, "O_RDONLY"):
        fd = os.open(str(path), os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(fd)
        finally:
            os.close(fd)


class BackupError(RuntimeError):
    """Raised by the backup enumerator when a retained file fails closed."""


@dataclass
class ScanResult:
    findings: list = field(default_factory=list)

    def codes(self):
        return sorted({code for code, _ in self.findings})

    def count(self, code: str) -> int:
        return sum(1 for finding_code, _ in self.findings if finding_code == code)


class RecoveryHarness:
    """Models the file-first protocol on top of a real workspace directory."""

    def __init__(self, workspace: Path, conn: sqlite3.Connection, profile_id: int = PROFILE_ID):
        self.ws = workspace
        self.conn = conn
        self.profile_id = profile_id
        self.staging_dir = self.ws / "staging"
        self.sources_dir = self.ws / "sources"
        self.recovery_dir = self.ws / "recovery"
        self.staging_dir.mkdir(parents=True, exist_ok=True)
        self.sources_dir.mkdir(parents=True, exist_ok=True)

    # -- protocol steps -----------------------------------------------------

    @staticmethod
    def content_hash(content: bytes) -> str:
        return hashlib.sha256(content).hexdigest()

    def lookup_committed(self, content_hash: str, mapping_fingerprint: str) -> int | None:
        row = self.conn.execute(
            "SELECT id FROM import_revision"
            " WHERE profile_id = ? AND content_hash = ? AND mapping_fingerprint = ?"
            "   AND contract_version = 'canonical/1'",
            (self.profile_id, content_hash, mapping_fingerprint),
        ).fetchone()
        return None if row is None else row[0]

    def stage(self, content: bytes) -> tuple[Path, str]:
        """Step 3: copy to a staging file while hashing; flush and fsync."""
        token = uuid.uuid4().hex
        path = self.staging_dir / ("%s.part" % token)
        with open(path, "wb") as handle:
            handle.write(content)
            handle.flush()
            os.fsync(handle.fileno())
        fsync_dir(self.staging_dir)
        return path, self.content_hash(content)

    def rename_final(self, staged: Path, revision_id: int, filename: str) -> Path:
        """Step 5: atomically rename into sources/<revision_id>/<safe_filename>."""
        target_dir = self.sources_dir / str(revision_id)
        target_dir.mkdir(parents=True, exist_ok=True)
        target = target_dir / filename
        os.rename(staged, target)
        fsync_dir(target_dir)
        fsync_dir(self.sources_dir)
        return target

    def commit_revision(self, content_hash: str, fingerprint: str, filename: str, target: Path) -> int:
        """Steps 4+6: revision row, durable file, path row, one commit."""
        cursor = self.conn.execute(
            "INSERT INTO import_revision (profile_id, content_hash, mapping_fingerprint, contract_version,"
            " source_filename, imported_at, row_count, observation_count, issue_count)"
            " VALUES (?, ?, ?, 'canonical/1', ?, ?, 1, 1, 0)",
            (self.profile_id, content_hash, fingerprint, filename, NOW),
        )
        revision_id = cursor.lastrowid
        relative_path = "sources/%d/%s" % (revision_id, filename)
        self.conn.execute(
            "INSERT INTO source_file (revision_id, relative_path, sha256, byte_size)"
            " VALUES (?, ?, ?, ?)",
            (revision_id, relative_path, content_hash, target.stat().st_size),
        )
        self.conn.commit()
        return revision_id

    def run_import(self, content: bytes, fingerprint: str, filename: str):
        """Full protocol with the step-2 idempotency check."""
        content_hash = self.content_hash(content)
        existing = self.lookup_committed(content_hash, fingerprint)
        if existing is not None:
            return "noop", existing, None
        staged, digest = self.stage(content)
        next_id = (self.conn.execute("SELECT COALESCE(MAX(id), 0) + 1 FROM import_revision").fetchone()[0])
        target = self.rename_final(staged, next_id, filename)
        revision_id = self.commit_revision(digest, fingerprint, filename, target)
        assert revision_id == next_id, "revision id allocation must match the staged directory"
        return "committed", revision_id, target

    # -- recovery and reconciliation ---------------------------------------

    def committed_files(self) -> list:
        return [
            (row[0], row[1], row[2], row[3])
            for row in self.conn.execute(
                "SELECT revision_id, relative_path, sha256, byte_size FROM source_file ORDER BY id"
            ).fetchall()
        ]

    def scan(self) -> ScanResult:
        result = ScanResult()
        committed = {
            relative_path: (revision_id, sha256)
            for revision_id, relative_path, sha256, _ in self.committed_files()
        }
        seen = set()
        for staged in sorted(self.staging_dir.glob("*.part")):
            digest = staged.stem
            if not digest.isalnum() or len(digest) != 32:
                result.findings.append(("hash_mismatch", str(staged)))
            result.findings.append(("stale_staging_file", str(staged)))
            seen.add(str(staged))
        for final in sorted(self.sources_dir.rglob("*")):
            if not final.is_file():
                continue
            key = str(final.relative_to(self.ws))
            seen.add(key)
            if key not in committed:
                result.findings.append(("orphan_final_file", key))
                continue
            _, expected = committed[key]
            actual = hashlib.sha256(final.read_bytes()).hexdigest()
            if actual != expected:
                result.findings.append(("hash_mismatch", key))
        for relative_path in committed:
            if relative_path not in seen:
                result.findings.append(("missing_committed_file", relative_path))
        return result

    def reconcile(self) -> list:
        """Move recoverable artifacts to the recovery area; report them."""
        result = self.scan()
        self.recovery_dir.mkdir(parents=True, exist_ok=True)
        moved = []
        for code, path in result.findings:
            if code in ("stale_staging_file", "orphan_final_file"):
                source = Path(path)
                if not source.is_absolute():
                    source = self.ws / source
                target = self.recovery_dir / source.name
                if target.exists():
                    target = self.recovery_dir / ("%s-%s" % (uuid.uuid4().hex[:8], source.name))
                os.rename(source, target)
                moved.append((code, path, str(target)))
        return moved

    def backup_enumerate(self) -> list:
        """Backup enumerates retained files from its snapshot and fails closed."""
        for _, relative_path, expected, _ in self.committed_files():
            path = self.ws / relative_path
            if not path.exists():
                raise BackupError("missing retained source file: %s" % relative_path)
            actual = hashlib.sha256(path.read_bytes()).hexdigest()
            if actual != expected:
                raise BackupError("hash-invalid retained source file: %s" % relative_path)
        return [relative_path for _, relative_path, _, _ in self.committed_files()]


class RecoveryHarnessCase(unittest.TestCase):
    def setUp(self) -> None:
        self._tmpdir = tempfile.TemporaryDirectory(prefix="opendemand-recovery-")
        self.ws = Path(self._tmpdir.name)
        self.db_path = str(self.ws / "workspace.sqlite3")
        self.conn = connect(self.db_path)
        self.conn.executescript(read_schema())
        self.conn.execute("PRAGMA foreign_keys = OFF")  # recovery inspects raw rows
        self.conn.execute(
            "INSERT INTO source_profile (id, name, is_active, created_at) VALUES (1, 'default', 1, ?)",
            (NOW,),
        )
        self.conn.commit()
        self.harness = RecoveryHarness(self.ws, self.conn)

    def tearDown(self) -> None:
        self.conn.close()
        self._tmpdir.cleanup()


class RecoveryProtocolTests(RecoveryHarnessCase):
    CONTENT = b"series_key,week_start,quantity\nA-001,2026-01-05,3\n"
    FINGERPRINT = "sha256:mapping-v1"

    def test_crash_after_staging_is_detected_and_quarantined(self):
        staged, digest = self.harness.stage(self.CONTENT)
        result = self.harness.scan()
        self.assertEqual(result.count("stale_staging_file"), 1)
        self.assertEqual(result.count("orphan_final_file"), 0)
        moved = self.harness.reconcile()
        self.assertEqual(len(moved), 1)
        self.assertEqual(moved[0][0], "stale_staging_file")
        self.assertTrue(Path(moved[0][2]).exists())
        self.assertFalse(staged.exists())
        self.assertEqual(self.harness.committed_files(), [])

    def test_crash_after_rename_before_commit_leaves_orphan_final_file(self):
        staged, digest = self.harness.stage(self.CONTENT)
        target = self.harness.rename_final(staged, 1, "source-a.csv")
        result = self.harness.scan()
        self.assertEqual(result.codes(), ["orphan_final_file"])
        moved = self.harness.reconcile()
        self.assertEqual(moved[0][0], "orphan_final_file")
        self.assertTrue(Path(moved[0][2]).exists())
        self.assertFalse(target.exists())
        self.assertEqual(len(self.harness.committed_files()), 0)

    def test_crash_after_commit_is_clean(self):
        outcome, revision_id, target = self.harness.run_import(
            self.CONTENT, self.FINGERPRINT, "source-a.csv"
        )
        self.assertEqual(outcome, "committed")
        self.assertTrue(target.exists())
        result = self.harness.scan()
        self.assertEqual(result.findings, [], "a clean commit must reconcile with no findings")
        self.assertEqual(self.harness.backup_enumerate(), ["sources/1/source-a.csv"])

    def test_retry_after_commit_before_response_is_noop(self):
        outcome, revision_id, _ = self.harness.run_import(
            self.CONTENT, self.FINGERPRINT, "source-a.csv"
        )
        self.assertEqual(outcome, "committed")
        outcome2, revision2, target2 = self.harness.run_import(
            self.CONTENT, self.FINGERPRINT, "source-a.csv"
        )
        self.assertEqual(outcome2, "noop")
        self.assertEqual(revision_id, revision2)
        self.assertIsNone(target2)
        self.assertEqual(self.conn.execute("SELECT COUNT(*) FROM import_revision").fetchone()[0], 1)
        self.assertEqual(self.conn.execute("SELECT COUNT(*) FROM source_file").fetchone()[0], 1)
        self.assertEqual(self.harness.scan().findings, [])

    def test_missing_committed_file_is_detected_and_backup_fails_closed(self):
        self.harness.run_import(self.CONTENT, self.FINGERPRINT, "source-a.csv")
        (self.ws / "sources" / "1" / "source-a.csv").unlink()
        result = self.harness.scan()
        self.assertEqual(result.codes(), ["missing_committed_file"])
        with self.assertRaises(BackupError):
            self.harness.backup_enumerate()

    def test_corrupted_committed_file_is_detected_and_backup_fails_closed(self):
        self.harness.run_import(self.CONTENT, self.FINGERPRINT, "source-a.csv")
        (self.ws / "sources" / "1" / "source-a.csv").write_bytes(b"tampered")
        result = self.harness.scan()
        self.assertEqual(result.codes(), ["hash_mismatch"])
        with self.assertRaises(BackupError):
            self.harness.backup_enumerate()

    def test_corrupted_staging_file_is_detected(self):
        staged, digest = self.harness.stage(self.CONTENT)
        staged.write_bytes(b"corrupted")
        result = self.harness.scan()
        self.assertEqual(result.codes(), ["stale_staging_file"])

    def test_recovery_moves_artifacts_without_touching_business_rows(self):
        staged, digest = self.harness.stage(self.CONTENT)
        moved = self.harness.reconcile()
        self.assertEqual(len(moved), 1)
        self.assertEqual(len(self.harness.committed_files()), 0)
        self.assertEqual(
            self.conn.execute("SELECT COUNT(*) FROM import_revision").fetchone()[0], 0
        )
        self.assertEqual(
            self.conn.execute("SELECT COUNT(*) FROM job").fetchone()[0], 0
        )


if __name__ == "__main__":
    unittest.main()
