"""Shared harness for the frozen-migration and source-recovery test modules.

Creates real temporary file-backed SQLite databases, applies the migration
under test, enables the workspace pragmas, and provides the full integration
fixture chain required by the Phase 1 freeze gate.

These tests validate the SQL migration and the file-first protocol logic. They
do not claim that the Rust implementation exists yet; the recovery harness is a
Python model of the documented protocol whose semantics the Rust core must
reproduce.
"""

from __future__ import annotations

import hashlib
import os
import sqlite3
import tempfile
import unittest
from pathlib import Path

SCHEMA_PATH = (
    Path(__file__).resolve().parents[2]
    / "docs"
    / "architecture"
    / "schema"
    / "001_init.sql"
)
HASH_PATH = SCHEMA_PATH.with_suffix(".sha256")

NOW = "2026-10-13T12:00:00Z"

PRODUCT_A = bytes.fromhex("00000000000070008000000000000001")
PRODUCT_B = bytes.fromhex("00000000000070008000000000000002")
LOCATION_ID = bytes.fromhex("00000000000070008000000000000003")
RUN_ID = bytes.fromhex("00000000000070008000000000000004")

HORIZON_START = "2026-10-12"
# Twelve weekly Mondays starting at HORIZON_START.
HORIZON_WEEKS = ["2026-10-12", "2026-10-19", "2026-10-26", "2026-11-02",
                 "2026-11-09", "2026-11-16", "2026-11-23", "2026-11-30",
                 "2026-12-07", "2026-12-14", "2026-12-21", "2026-12-28"]


def read_schema() -> str:
    return SCHEMA_PATH.read_text(encoding="utf-8")


def read_hash_file():
    """Return (sha256, filename) from the freeze fingerprint, or None."""
    if not HASH_PATH.exists():
        return None
    tokens = HASH_PATH.read_text(encoding="utf-8").split()
    if len(tokens) < 2:
        raise AssertionError("malformed hash file: %s" % HASH_PATH)
    return tokens[0].lower(), tokens[1].strip("*")


def compute_schema_sha256() -> str:
    return hashlib.sha256(SCHEMA_PATH.read_bytes()).hexdigest()


def connect(path: str) -> sqlite3.Connection:
    """Open a file-backed connection with the documented workspace pragmas."""
    conn = sqlite3.connect(path)
    conn.isolation_level = None
    conn.execute("PRAGMA journal_mode=WAL")
    conn.execute("PRAGMA synchronous=FULL")
    conn.execute("PRAGMA foreign_keys=ON")
    conn.execute("PRAGMA busy_timeout=5000")
    return conn


def apply_schema(conn: sqlite3.Connection) -> None:
    conn.executescript(read_schema())


def assert_fk_check_empty(testcase: unittest.TestCase, conn: sqlite3.Connection) -> None:
    rows = conn.execute("PRAGMA foreign_key_check").fetchall()
    testcase.assertEqual(rows, [], "foreign_key_check must be empty, got %r" % (rows,))


def assert_integrity_ok(testcase: unittest.TestCase, conn: sqlite3.Connection) -> None:
    rows = conn.execute("PRAGMA integrity_check").fetchall()
    testcase.assertEqual(rows, [("ok",)], "integrity_check must be ok, got %r" % (rows,))


def table_names(conn: sqlite3.Connection) -> set:
    rows = conn.execute(
        "SELECT name FROM sqlite_master WHERE type = 'table'"
    ).fetchall()
    return {row[0] for row in rows}


class FullChainDb(unittest.TestCase):
    """Base class: one temporary file-backed database per test method."""

    def setUp(self) -> None:
        self._tmpdir = tempfile.TemporaryDirectory(prefix="opendemand-test-")
        self.db_path = os.path.join(self._tmpdir.name, "workspace.sqlite3")
        self.conn = connect(self.db_path)
        apply_schema(self.conn)
        FullChainFixture(self.conn).build()  # commits the chain
        self.conn.execute("SAVEPOINT test_case")

    def tearDown(self) -> None:
        try:
            self.conn.execute("ROLLBACK TO test_case")
            self.conn.execute("RELEASE test_case")
        finally:
            self.conn.close()
            self._tmpdir.cleanup()


class FullChainFixture:
    """Builds the complete integration chain used by the positive cases.

    profile -> canonical mapping -> import revision -> observations ->
    qualification -> forecast run -> run model -> two series/model evaluations
    -> per-series origins -> backtest points -> selected result -> forecast ->
    manual override.
    """

    def __init__(self, conn: sqlite3.Connection):
        self.conn = conn

    def build(self) -> None:
        c = self.conn
        c.execute(
            "INSERT INTO source_profile (id, name, is_active, created_at) VALUES (1, 'default', 1, ?)",
            (NOW,),
        )
        c.execute(
            "INSERT INTO product (id, sku, name, created_at) VALUES (?, 'SKU-A', 'Product A', ?)",
            (PRODUCT_A, NOW),
        )
        c.execute(
            "INSERT INTO product (id, sku, name, created_at) VALUES (?, 'SKU-B', 'Product B', ?)",
            (PRODUCT_B, NOW),
        )
        c.execute(
            "INSERT INTO location (id, code, name, created_at) VALUES (?, 'default', 'Default', ?)",
            (LOCATION_ID, NOW),
        )
        c.execute(
            "INSERT INTO product_code_map (profile_id, source_code, product_id, created_at) VALUES (1, 'A-001', ?, ?)",
            (PRODUCT_A, NOW),
        )
        c.execute(
            "INSERT INTO product_code_map (profile_id, source_code, product_id, created_at) VALUES (1, 'B-001', ?, ?)",
            (PRODUCT_B, NOW),
        )
        c.execute(
            "INSERT INTO location_code_map (profile_id, source_code, location_id, created_at) VALUES (1, 'default', ?, ?)",
            (LOCATION_ID, NOW),
        )
        c.execute(
            "INSERT INTO import_revision (id, profile_id, content_hash, mapping_fingerprint, contract_version, source_filename, imported_at, row_count, observation_count, issue_count)"
            " VALUES (1, 1, 'sha256:content', 'sha256:mapping', 'canonical/1', 'source.csv', ?, 6, 6, 0)",
            (NOW,),
        )
        c.execute(
            "INSERT INTO source_file (id, revision_id, relative_path, sha256, byte_size)"
            " VALUES (1, 1, 'sources/1/source.csv', 'sha256:file', 128)"
        )
        self._observations()
        self._qualification()
        self._run()
        self._evaluation()
        self._origins()
        self._backtest_forecasts()
        self._selection()
        c.commit()

    def _observations(self) -> None:
        rows = []
        for product in (PRODUCT_A, PRODUCT_B):
            rows += [
                (1, product, LOCATION_ID, "2026-08-17", 4.0, "known"),
                (1, product, LOCATION_ID, "2026-08-24", 0.0, "zero"),
                (1, product, LOCATION_ID, "2026-08-31", None, "missing"),
            ]
        self.conn.executemany(
            "INSERT INTO observation (revision_id, product_id, location_id, week_start, quantity, status)"
            " VALUES (?, ?, ?, ?, ?, ?)",
            rows,
        )

    def _qualification(self) -> None:
        self.conn.executemany(
            "INSERT INTO series_qualification (revision_id, product_id, location_id, h_length, status, reason, computed_at)"
            " VALUES (1, ?, ?, ?, 'eligible', NULL, ?)",
            [(PRODUCT_A, LOCATION_ID, 40, NOW), (PRODUCT_B, LOCATION_ID, 15, NOW)],
        )

    def _run(self) -> None:
        c = self.conn
        c.execute(
            "INSERT INTO forecast_run (id, revision_id, created_at, finished_at, status, horizon_weeks, horizon_start_week, timezone, config_json, app_version, engine_version)"
            " VALUES (?, 1, ?, ?, 'completed', 12, ?, 'Europe/Warsaw', '{\"models\":[\"naive\"]}', '0.0.0', 'engine/1')",
            (RUN_ID, NOW, NOW, HORIZON_START),
        )
        c.execute(
            "INSERT INTO run_model (run_id, model_id, status, tasks_planned, tasks_completed, tasks_failed)"
            " VALUES (?, 'naive', 'completed', 6, 6, 0)",
            (RUN_ID,),
        )

    def _evaluation(self) -> None:
        rows = [
            # series A: champion, two completed origins, distinct score
            (RUN_ID, PRODUCT_A, LOCATION_ID, "naive", "champion_eligible", 2, 2, 0.10, 1, 1, "completed", None, None),
            # series B: production-only fallback (one completed origin, one failed)
            (RUN_ID, PRODUCT_B, LOCATION_ID, "naive", "production_only", 2, 1, 0.50, 2, 0, "completed", None, None),
        ]
        self.conn.executemany(
            "INSERT INTO series_model_evaluation (run_id, product_id, location_id, model_id, eligibility_status, origins_planned, origins_completed, selection_score, selection_rank, is_champion, production_status, failure_code, failure_detail)"
            " VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            rows,
        )

    def _origins(self) -> None:
        rows = [
            # series A, origin 1 vs series B, origin 1: same model and origin_index,
            # different dates, train sizes, and metrics.
            (RUN_ID, PRODUCT_A, LOCATION_ID, "naive", 1, "2026-09-21", 36, "completed", 0.10, 1.0, "wape", 0.10, None, None),
            (RUN_ID, PRODUCT_B, LOCATION_ID, "naive", 1, "2026-08-24", 12, "completed", 0.50, 2.0, "wape", 0.50, None, None),
            # series A, origin 2 (completed; all-zero actuals -> MAE fallback, WAPE NULL)
            (RUN_ID, PRODUCT_A, LOCATION_ID, "naive", 2, "2026-09-28", 40, "completed", None, 0.75, "mae_fallback", 0.75, None, None),
            # series B, origin 2 failed
            (RUN_ID, PRODUCT_B, LOCATION_ID, "naive", 2, "2026-08-31", 16, "failed", None, None, None, None, "predict_error", "sidecar reported predict_error"),
        ]
        self.conn.executemany(
            "INSERT INTO series_model_origin (run_id, product_id, location_id, model_id, origin_index, origin_week_start, train_size, status, wape, mae, metric_kind, metric_value, failure_code, failure_detail)"
            " VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            rows,
        )

    def _backtest_forecasts(self) -> None:
        rows = [
            (RUN_ID, PRODUCT_A, LOCATION_ID, "naive", 1, "2026-09-28", 5.0),
            (RUN_ID, PRODUCT_B, LOCATION_ID, "naive", 1, "2026-08-31", 3.0),
            (RUN_ID, PRODUCT_A, LOCATION_ID, "naive", 2, "2026-10-05", 6.0),
        ]
        self.conn.executemany(
            "INSERT INTO backtest_forecast (run_id, product_id, location_id, model_id, origin_index, week_start, value)"
            " VALUES (?, ?, ?, ?, ?, ?, ?)",
            rows,
        )

    def _selection(self) -> None:
        c = self.conn
        c.executemany(
            "INSERT INTO forecast_series_status (run_id, product_id, location_id, status, selected_model_id, reason)"
            " VALUES (?, ?, ?, ?, 'naive', NULL)",
            [
                (RUN_ID, PRODUCT_A, LOCATION_ID, "ok"),
                (RUN_ID, PRODUCT_B, LOCATION_ID, "fallback_naive"),
            ],
        )
        c.executemany(
            "INSERT INTO forecast (run_id, product_id, location_id, week_start, value, selected_model_id)"
            " VALUES (?, ?, ?, ?, ?, 'naive')",
            [
                (RUN_ID, PRODUCT_A, LOCATION_ID, week, 10.0)
                for week in HORIZON_WEEKS
            ]
            + [
                (RUN_ID, PRODUCT_B, LOCATION_ID, week, 7.0)
                for week in HORIZON_WEEKS[:6]
            ],
        )
        c.execute(
            "INSERT INTO forecast_override (run_id, product_id, location_id, week_start, revision, value, note, created_at)"
            " VALUES (?, ?, ?, ?, 1, 12.5, 'first override', ?)",
            (RUN_ID, PRODUCT_A, LOCATION_ID, HORIZON_WEEKS[0], NOW),
        )
