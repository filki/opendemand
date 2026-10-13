"""Integration-oriented tests for the OpenDemand initial migration (001_init.sql).

These tests create a real temporary file-backed SQLite database, apply the
migration, enable foreign keys/pragmas, and exercise transactions instead of
only parsing DDL. They cover every Revision 3 positive and negative case from
the approved Phase 1 plan and verify the freeze fingerprint once present.

Run from the repository root:

    python3 -m unittest discover -s tests/architecture -v
"""

from __future__ import annotations

import sqlite3
import unittest

from tests.architecture.migration_harness import (
    HORIZON_START,
    HORIZON_WEEKS,
    LOCATION_ID,
    NOW,
    PRODUCT_A,
    PRODUCT_B,
    RUN_ID,
    FullChainDb,
    assert_fk_check_empty,
    assert_integrity_ok,
    compute_schema_sha256,
    read_hash_file,
    table_names,
)

EXPECTED_STATUS_VOCABULARIES = {
    "job": {"queued", "running", "completed", "failed", "cancelled"},
    "run_model": {"completed", "partial", "failed", "not_run"},
    "forecast_run": {"running", "completed", "failed", "cancelled"},
    "forecast_series_status": {
        "ok", "fallback_naive", "insufficient_history", "model_failure", "invalid_data",
    },
    "series_model_evaluation": {"not_eligible", "production_only", "champion_eligible"},
    "series_model_origin": {"completed", "failed"},
}


def assert_raises_db_error(testcase, sql: str, *params) -> None:
    with testcase.assertRaises(sqlite3.Error):
        testcase.conn.execute(sql, params)


def insert_run(conn: sqlite3.Connection, run_id: bytes, status: str = "running") -> None:
    conn.execute(
        "INSERT INTO forecast_run (id, revision_id, created_at, finished_at, status, horizon_weeks,"
        " horizon_start_week, timezone, config_json, app_version, engine_version)"
        " VALUES (?, 1, ?, NULL, ?, 12, ?, 'Europe/Warsaw', '{}', '0.0.0', 'engine/1')",
        (run_id, NOW, status, HORIZON_START),
    )


class FullChainTests(FullChainDb):
    # ------------------------------------------------------------------
    # Positive cases
    # ------------------------------------------------------------------

    def test_full_chain_and_integrity(self):
        c = self.conn
        self.assertEqual(
            c.execute("SELECT COUNT(*) FROM series_model_evaluation").fetchone()[0], 2
        )
        self.assertEqual(
            c.execute("SELECT COUNT(*) FROM series_model_origin").fetchone()[0], 4
        )
        self.assertEqual(
            c.execute("SELECT COUNT(*) FROM forecast_override").fetchone()[0], 1
        )
        self.assertEqual(
            c.execute(
                "SELECT selected_model_id FROM forecast_series_status WHERE run_id = ? AND product_id = ?",
                (RUN_ID, PRODUCT_A),
            ).fetchone()[0],
            "naive",
        )
        assert_fk_check_empty(self, c)
        assert_integrity_ok(self, c)

    def test_two_series_same_model_origin_index_keep_distinct_data(self):
        c = self.conn
        a = c.execute(
            "SELECT origin_week_start, train_size, wape, mae FROM series_model_origin"
            " WHERE product_id = ? AND model_id = 'naive' AND origin_index = 1",
            (PRODUCT_A,),
        ).fetchone()
        b = c.execute(
            "SELECT origin_week_start, train_size, wape, mae FROM series_model_origin"
            " WHERE product_id = ? AND model_id = 'naive' AND origin_index = 1",
            (PRODUCT_B,),
        ).fetchone()
        self.assertEqual(a, ("2026-09-21", 36, 0.10, 1.0))
        self.assertEqual(b, ("2026-08-24", 12, 0.50, 2.0))
        self.assertNotEqual(a[0], b[0])
        self.assertNotEqual(a[1], b[1])
        # Series B origin 2 is a recorded failure, not a metric row.
        failed = c.execute(
            "SELECT status, wape, failure_code FROM series_model_origin"
            " WHERE product_id = ? AND origin_index = 2",
            (PRODUCT_B,),
        ).fetchone()
        self.assertEqual(failed, ("failed", None, "predict_error"))

    def test_naive_multi_origin_champion_and_fallback(self):
        c = self.conn
        champ = c.execute(
            "SELECT eligibility_status, origins_completed, is_champion, selection_rank"
            " FROM series_model_evaluation WHERE product_id = ?",
            (PRODUCT_A,),
        ).fetchone()
        self.assertEqual(champ, ("champion_eligible", 2, 1, 1))
        fallback = c.execute(
            "SELECT eligibility_status, origins_completed, is_champion"
            " FROM series_model_evaluation WHERE product_id = ?",
            (PRODUCT_B,),
        ).fetchone()
        self.assertEqual(fallback, ("production_only", 1, 0))
        self.assertEqual(
            c.execute(
                "SELECT status FROM forecast_series_status WHERE product_id = ?",
                (PRODUCT_B,),
            ).fetchone()[0],
            "fallback_naive",
        )

    def test_all_zero_actuals_store_mae_fallback_with_null_wape(self):
        row = self.conn.execute(
            "SELECT wape, mae, metric_kind, metric_value FROM series_model_origin"
            " WHERE product_id = ? AND origin_index = 2",
            (PRODUCT_A,),
        ).fetchone()
        self.assertEqual(row, (None, 0.75, "mae_fallback", 0.75))

    def test_aggregates_only_selection_survives_backtest_deletion(self):
        c = self.conn
        c.execute(
            "DELETE FROM backtest_forecast WHERE run_id = ? AND product_id = ?",
            (RUN_ID, PRODUCT_A),
        )
        champion = c.execute(
            "SELECT model_id, is_champion FROM series_model_evaluation"
            " WHERE run_id = ? AND product_id = ? AND is_champion = 1",
            (RUN_ID, PRODUCT_A),
        ).fetchone()
        self.assertEqual(champion, ("naive", 1))
        self.assertEqual(
            c.execute(
                "SELECT COUNT(*) FROM series_model_origin WHERE run_id = ? AND product_id = ?",
                (RUN_ID, PRODUCT_A),
            ).fetchone()[0],
            2,
        )
        assert_fk_check_empty(self, c)
        assert_integrity_ok(self, c)

    def test_run_model_is_execution_summary_only(self):
        columns = {
            row[1]
            for row in self.conn.execute("PRAGMA table_info(run_model)").fetchall()
        }
        self.assertIn("status", columns)
        self.assertIn("tasks_planned", columns)
        self.assertIn("tasks_completed", columns)
        self.assertIn("tasks_failed", columns)
        self.assertNotIn("aggregate_metric", columns)
        row = self.conn.execute(
            "SELECT status, tasks_planned, tasks_completed, tasks_failed FROM run_model WHERE run_id = ?",
            (RUN_ID,),
        ).fetchone()
        self.assertEqual(row, ("completed", 6, 6, 0))
        status_sql = self.conn.execute(
            "SELECT sql FROM sqlite_master WHERE name = 'run_model'"
        ).fetchone()[0]
        self.assertIn("status IN ('completed', 'partial', 'failed', 'not_run')", status_sql)

    def test_forecast_series_status_has_no_duplicated_metrics(self):
        columns = {
            row[1]
            for row in self.conn.execute("PRAGMA table_info(forecast_series_status)").fetchall()
        }
        self.assertIn("selected_model_id", columns)
        self.assertNotIn("champion_metric", columns)
        self.assertNotIn("origins_completed", columns)

    def test_legacy_origin_tables_replaced(self):
        names = table_names(self.conn)
        self.assertNotIn("backtest_origin", names)
        origin_pk = [
            row[1]
            for row in sorted(
                self.conn.execute("PRAGMA table_info(series_model_origin)").fetchall(),
                key=lambda row: row[5],
            )
            if row[5] > 0
        ]
        self.assertEqual(
            origin_pk,
            ["run_id", "product_id", "location_id", "model_id", "origin_index"],
        )
        backtest_fks = self.conn.execute("PRAGMA foreign_key_list(backtest_forecast)").fetchall()
        self.assertEqual({fk[2] for fk in backtest_fks}, {"series_model_origin"})
        self.assertEqual(len(backtest_fks), 5)

    def test_status_vocabulary_matches_approved_revision_3(self):
        c = self.conn
        for table, expected in EXPECTED_STATUS_VOCABULARIES.items():
            sql = c.execute(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = ?", (table,)
            ).fetchone()[0]
            marker = "status IN ("
            start = sql.index(marker) + len(marker)
            end = sql.index(")", start)
            actual = {token.strip().strip("'") for token in sql[start:end].split(",")}
            self.assertEqual(actual, expected, "status vocabulary drifted for %s" % table)
        eligibility_sql = c.execute(
            "SELECT sql FROM sqlite_master WHERE name = 'series_model_evaluation'"
        ).fetchone()[0]
        self.assertIn(
            "eligibility_status IN ('not_eligible', 'production_only', 'champion_eligible')",
            eligibility_sql,
        )
        self.assertIn(
            "production_status IN ('not_run', 'completed', 'failed')", eligibility_sql
        )

    # ------------------------------------------------------------------
    # Observation constraints
    # ------------------------------------------------------------------

    def _observation(self, product: bytes, week: str, quantity, status: str):
        self.conn.execute(
            "INSERT INTO observation (revision_id, product_id, location_id, week_start, quantity, status)"
            " VALUES (1, ?, ?, ?, ?, ?)",
            (product, LOCATION_ID, week, quantity, status),
        )

    def test_observation_quantities_accept_valid_combinations(self):
        self._observation(PRODUCT_A, "2026-09-07", None, "partial")
        self._observation(PRODUCT_A, "2026-09-14", None, "missing")
        self._observation(PRODUCT_A, "2026-09-21", 4.5, "known")
        self._observation(PRODUCT_A, "2026-09-28", 0.0, "zero")
        self._observation(PRODUCT_A, "2026-10-05", -2.5, "return")
        assert_fk_check_empty(self, self.conn)

    def test_observation_quantities_reject_invalid_combinations(self):
        for week, quantity, status in [
            ("2026-09-07", 1.0, "partial"),
            ("2026-09-14", 1.0, "missing"),
            ("2026-09-21", None, "known"),
            ("2026-09-28", 1.0, "zero"),
        ]:
            assert_raises_db_error(
                self,
                "INSERT INTO observation (revision_id, product_id, location_id, week_start, quantity, status)"
                " VALUES (1, ?, ?, ?, ?, ?)",
                PRODUCT_A,
                LOCATION_ID,
                week,
                quantity,
                status,
            )

    # ------------------------------------------------------------------
    # Per-series evaluation constraints (negative cases)
    # ------------------------------------------------------------------

    def test_second_champion_for_same_run_series_fails(self):
        c = self.conn
        c.execute(
            "INSERT INTO run_model (run_id, model_id, status, tasks_planned, tasks_completed, tasks_failed)"
            " VALUES (?, 'seasonal_naive_52', 'not_run', 0, 0, 0)",
            (RUN_ID,),
        )
        assert_raises_db_error(
            self,
            "INSERT INTO series_model_evaluation (run_id, product_id, location_id, model_id,"
            " eligibility_status, origins_planned, origins_completed, selection_score, selection_rank,"
            " is_champion, production_status)"
            " VALUES (?, ?, ?, 'seasonal_naive_52', 'champion_eligible', 0, 0, 0.9, 1, 1, 'not_run')",
            RUN_ID,
            PRODUCT_A,
            LOCATION_ID,
        )

    def test_origin_without_series_evaluation_fails_by_fk(self):
        assert_raises_db_error(
            self,
            "INSERT INTO series_model_origin (run_id, product_id, location_id, model_id, origin_index,"
            " origin_week_start, train_size, status)"
            " VALUES (?, ?, ?, 'auto_ets_52', 1, '2026-09-21', 10, 'completed')",
            RUN_ID,
            PRODUCT_A,
            LOCATION_ID,
        )

    def test_backtest_point_without_exact_origin_fails_by_fk(self):
        assert_raises_db_error(
            self,
            "INSERT INTO backtest_forecast (run_id, product_id, location_id, model_id, origin_index, week_start, value)"
            " VALUES (?, ?, ?, 'naive', 3, '2026-10-05', 1.0)",
            RUN_ID,
            PRODUCT_A,
            LOCATION_ID,
        )

    def test_selected_model_not_in_series_evaluation_fails_by_fk(self):
        assert_raises_db_error(
            self,
            "INSERT INTO forecast_series_status (run_id, product_id, location_id, status, selected_model_id)"
            " VALUES (?, ?, ?, 'ok', 'seasonal_naive_52')",
            RUN_ID,
            PRODUCT_A,
            LOCATION_ID,
        )

    def test_cross_series_origin_leakage_fails_by_fk(self):
        # Product B has no seasonal model origins at all; a backtest point must
        # not borrow another series'/model's origin row.
        assert_raises_db_error(
            self,
            "INSERT INTO backtest_forecast (run_id, product_id, location_id, model_id, origin_index, week_start, value)"
            " VALUES (?, ?, ?, 'seasonal_naive_52', 1, '2026-10-05', 1.0)",
            RUN_ID,
            PRODUCT_B,
            LOCATION_ID,
        )


class ImportIdempotencyTests(FullChainDb):
    def test_only_one_active_default_profile(self):
        # The genesis profile is active; a second active profile must be rejected.
        assert_raises_db_error(
            self,
            "INSERT INTO source_profile (name, is_active, created_at) VALUES ('second', 1, ?)",
            NOW,
        )

    def _revision(self, revision_id: int, content: str, fingerprint: str, version: str = "canonical/1"):
        self.conn.execute(
            "INSERT INTO import_revision (id, profile_id, content_hash, mapping_fingerprint, contract_version,"
            " source_filename, imported_at) VALUES (?, 1, ?, ?, ?, 'x.csv', ?)",
            (revision_id, content, fingerprint, version, NOW),
        )

    def test_same_content_and_fingerprint_collides(self):
        # Revision 1 already exists with sha256:content / sha256:mapping / canonical/1.
        assert_raises_db_error(
            self,
            "INSERT INTO import_revision (id, profile_id, content_hash, mapping_fingerprint, contract_version,"
            " source_filename, imported_at) VALUES (99, 1, 'sha256:content', 'sha256:mapping', 'canonical/1', 'x.csv', ?)",
            NOW,
        )

    def test_changed_fingerprint_allows_new_revision(self):
        self._revision(2, "sha256:content", "sha256:mapping-v2")

    def test_changed_contract_version_allows_new_revision(self):
        self._revision(3, "sha256:content", "sha256:mapping", "canonical/2")

    def test_changed_content_allows_new_revision(self):
        self._revision(4, "sha256:content-v2", "sha256:mapping")

    def test_duplicate_global_relative_path_fails(self):
        assert_raises_db_error(
            self,
            "INSERT INTO source_file (revision_id, relative_path, sha256, byte_size)"
            " VALUES (1, 'sources/1/source.csv', 'sha256:other', 10)",
        )

    def test_failed_imports_live_only_in_job(self):
        # A failed attempt is a job row; it never blocks a later committed revision.
        self.conn.execute(
            "INSERT INTO job (kind, status, created_at, error_code) VALUES ('import', 'failed', ?, 'parse_error')",
            (NOW,),
        )
        self.assertEqual(
            self.conn.execute(
                "SELECT COUNT(*) FROM import_revision WHERE content_hash = 'sha256:nonexistent'"
            ).fetchone()[0],
            0,
        )


class RunHorizonTests(FullChainDb):
    def test_horizon_start_week_required(self):
        assert_raises_db_error(
            self,
            "INSERT INTO forecast_run (id, revision_id, created_at, status, horizon_weeks, timezone, config_json, app_version, engine_version)"
            " VALUES (?, 1, ?, 'running', 12, 'Europe/Warsaw', '{}', '0.0.0', 'engine/1')",
            bytes.fromhex("00000000000070008000000000000050"),
            NOW,
        )

    def test_horizon_weeks_must_remain_twelve(self):
        assert_raises_db_error(
            self,
            "INSERT INTO forecast_run (id, revision_id, created_at, status, horizon_weeks, horizon_start_week, timezone, config_json, app_version, engine_version)"
            " VALUES (?, 1, ?, 'running', 8, ?, 'Europe/Warsaw', '{}', '0.0.0', 'engine/1')",
            bytes.fromhex("00000000000070008000000000000051"),
            NOW,
            HORIZON_START,
        )


class ManualOverrideTests(FullChainDb):
    def _override(self, week: str, revision: int, value, product: bytes = PRODUCT_A, run_id: bytes = RUN_ID):
        self.conn.execute(
            "INSERT INTO forecast_override (run_id, product_id, location_id, week_start, revision, value, note, created_at)"
            " VALUES (?, ?, ?, ?, ?, ?, NULL, ?)",
            (run_id, product, LOCATION_ID, week, revision, value, NOW),
        )

    def test_first_override_accepts_value_and_clearing_revision(self):
        self._override(HORIZON_WEEKS[1], 1, 99.0)
        # Clearing revision: NULL is allowed for revision 2.
        self._override(HORIZON_WEEKS[1], 2, None)
        latest = self.conn.execute(
            "SELECT revision, value FROM forecast_override WHERE run_id = ? AND product_id = ? AND week_start = ?"
            " ORDER BY revision DESC LIMIT 1",
            (RUN_ID, PRODUCT_A, HORIZON_WEEKS[1]),
        ).fetchone()
        self.assertEqual(latest, (2, None))

    def test_override_without_baseline_is_allowed(self):
        # Series B has only six persisted forecast points; manual-only week 12 is fine.
        self._override(HORIZON_WEEKS[11], 1, 4.5, product=PRODUCT_B)

    def test_first_revision_null_fails(self):
        assert_raises_db_error(
            self,
            "INSERT INTO forecast_override (run_id, product_id, location_id, week_start, revision, value, note, created_at)"
            " VALUES (?, ?, ?, ?, 1, NULL, NULL, ?)",
            RUN_ID,
            PRODUCT_A,
            LOCATION_ID,
            HORIZON_WEEKS[1],
            NOW,
        )

    def test_skipped_revision_fails(self):
        assert_raises_db_error(
            self,
            "INSERT INTO forecast_override (run_id, product_id, location_id, week_start, revision, value, note, created_at)"
            " VALUES (?, ?, ?, ?, 3, 5.0, NULL, ?)",
            RUN_ID,
            PRODUCT_A,
            LOCATION_ID,
            HORIZON_WEEKS[0],
            NOW,
        )

    def test_out_of_horizon_week_fails(self):
        assert_raises_db_error(
            self,
            "INSERT INTO forecast_override (run_id, product_id, location_id, week_start, revision, value, note, created_at)"
            " VALUES (?, ?, ?, '2027-01-04', 1, 1.0, NULL, ?)",
            RUN_ID,
            PRODUCT_A,
            LOCATION_ID,
            NOW,
        )

    def test_week_before_horizon_fails(self):
        assert_raises_db_error(
            self,
            "INSERT INTO forecast_override (run_id, product_id, location_id, week_start, revision, value, note, created_at)"
            " VALUES (?, ?, ?, '2026-10-05', 1, 1.0, NULL, ?)",
            RUN_ID,
            PRODUCT_A,
            LOCATION_ID,
            NOW,
        )

    def test_non_completed_run_fails(self):
        run_id = bytes.fromhex("00000000000070008000000000000060")
        insert_run(self.conn, run_id, status="running")
        self.conn.execute(
            "INSERT INTO run_model (run_id, model_id, status) VALUES (?, 'naive', 'not_run')",
            (run_id,),
        )
        self.conn.execute(
            "INSERT INTO series_model_evaluation (run_id, product_id, location_id, model_id, eligibility_status)"
            " VALUES (?, ?, ?, 'naive', 'not_eligible')",
            (run_id, PRODUCT_A, LOCATION_ID),
        )
        self.conn.execute(
            "INSERT INTO forecast_series_status (run_id, product_id, location_id, status)"
            " VALUES (?, ?, ?, 'insufficient_history')",
            (run_id, PRODUCT_A, LOCATION_ID),
        )
        assert_raises_db_error(
            self,
            "INSERT INTO forecast_override (run_id, product_id, location_id, week_start, revision, value, note, created_at)"
            " VALUES (?, ?, ?, ?, 1, 1.0, NULL, ?)",
            run_id,
            PRODUCT_A,
            LOCATION_ID,
            HORIZON_WEEKS[0],
            NOW,
        )

    def test_unscoped_series_fails(self):
        run_id = bytes.fromhex("00000000000070008000000000000061")
        insert_run(self.conn, run_id, status="completed")
        assert_raises_db_error(
            self,
            "INSERT INTO forecast_override (run_id, product_id, location_id, week_start, revision, value, note, created_at)"
            " VALUES (?, ?, ?, ?, 1, 1.0, NULL, ?)",
            run_id,
            PRODUCT_A,
            LOCATION_ID,
            HORIZON_WEEKS[0],
            NOW,
        )

    def test_update_is_rejected(self):
        assert_raises_db_error(
            self,
            "UPDATE forecast_override SET value = 99.0 WHERE run_id = ? AND product_id = ? AND week_start = ?",
            RUN_ID,
            PRODUCT_A,
            HORIZON_WEEKS[0],
        )

    def test_delete_is_rejected(self):
        assert_raises_db_error(
            self,
            "DELETE FROM forecast_override WHERE run_id = ? AND product_id = ?",
            RUN_ID,
            PRODUCT_A,
        )


class FreezeFingerprintTests(unittest.TestCase):
    def test_read_hash_file_roundtrip(self):
        info = read_hash_file()
        if info is None:
            self.skipTest("freeze fingerprint not yet written (pre-freeze phase)")
        digest, filename = info
        self.assertEqual(filename, "001_init.sql", "hash file must name the migration")
        self.assertEqual(len(digest), 64, "hash file must contain a SHA-256 digest")
        self.assertEqual(compute_schema_sha256(), digest,
                         "001_init.sql changed after it was frozen")


if __name__ == "__main__":
    unittest.main()
