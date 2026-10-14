"""Behaviour tests for the model task engine through the public run_job seam."""

from __future__ import annotations

import math
import unittest

from tests.engine import helpers as h


class BehaviourTests(unittest.TestCase):
    def setUp(self):
        import tempfile

        self._tmp = tempfile.TemporaryDirectory()
        self.job_dir = h.Path(self._tmp.name)

    def tearDown(self):
        self._tmp.cleanup()

    # -- helpers -----------------------------------------------------------

    def run_job(self, observation_rows, task_rows, **kwargs):
        h.write_inputs(self.job_dir, observation_rows, task_rows, **kwargs)
        code, stdout, stderr = h.run_inproc(self.job_dir, [h.job_request()])
        return code, stdout, stderr

    def status_by_task(self, stdout):
        table = h.read_table(self.job_dir / "task_status.arrow")
        rows = table.to_pylist()
        return {row["task_id"]: row for row in rows}, rows

    def metrics_rows(self):
        return h.read_table(self.job_dir / "origin_metrics.arrow").to_pylist()

    def point_rows(self):
        return h.read_table(self.job_dir / "forecast_points.arrow").to_pylist()

    # -- tests -------------------------------------------------------------

    def test_production_forecast_repeats_last_training_value_for_12_weeks(self):
        pid, lid = h.series_uuid("p1"), h.series_uuid("l1")
        start = h.monday("2026-01-05")
        weeks = h.week_grid(start, 12)
        observations = h.weekly_rows(pid, lid, start, [float(i) for i in range(1, 13)])
        task = h.production_task(h.series_uuid("p1/prod"), pid, lid, weeks)

        code, stdout, _ = self.run_job(observations, [task])
        self.assertEqual(code, 0)
        terminal = stdout.terminal()
        self.assertEqual(terminal["type"], "job.result")

        status, rows = self.status_by_task(stdout)
        self.assertEqual(len(rows), 1)
        self.assertEqual(status[task["task_id"]]["status"], "completed")
        self.assertIsNone(status[task["task_id"]]["failure_code"])

        points = self.point_rows()
        self.assertEqual(len(points), 12)
        self.assertEqual({p["value"] for p in points}, {12.0})
        self.assertEqual(
            [p["week_start"] for p in points],
            h.week_grid(h.add_weeks(start, 12), 12),
        )
        self.assertEqual(len({p["week_start"] for p in points}), 12)
        self.assertEqual(self.metrics_rows(), [])

    def test_negative_values_are_valid_everywhere(self):
        pid, lid = h.series_uuid("p2"), h.series_uuid("l2")
        start = h.monday("2026-01-05")
        values = [float(-i) for i in range(1, 13)] + [-12.0] * 11 + [-6.0]
        observations = h.weekly_rows(pid, lid, start, values)
        weeks = h.week_grid(start, 24)
        task = h.backtest_task(h.series_uuid("p2/b1"), pid, lid, weeks, 1, 12)

        code, stdout, _ = self.run_job(observations, [task])
        self.assertEqual(code, 0)
        self.assertEqual(stdout.terminal()["type"], "job.result")

        points = self.point_rows()
        self.assertEqual(len(points), 12)
        self.assertEqual({p["value"] for p in points}, {-12.0})

        metrics = self.metrics_rows()
        self.assertEqual(len(metrics), 1)
        self.assertAlmostEqual(metrics[0]["wape"], 6.0 / 138.0, places=15)
        self.assertAlmostEqual(metrics[0]["mae"], 0.5, places=15)
        self.assertEqual(metrics[0]["metric_kind"], "wape")
        self.assertAlmostEqual(metrics[0]["metric_value"], 6.0 / 138.0, places=15)
        self.assertEqual(metrics[0]["origin_week_start"], h.monday("2026-03-23"))

    def test_all_zero_actuals_use_mae_fallback_with_null_wape(self):
        pid, lid = h.series_uuid("p3"), h.series_uuid("l3")
        start = h.monday("2026-01-05")
        values = [5.0] * 12 + [0.0] * 12
        observations = h.weekly_rows(pid, lid, start, values)
        weeks = h.week_grid(start, 24)
        task = h.backtest_task(h.series_uuid("p3/b1"), pid, lid, weeks, 1, 12)

        code, stdout, _ = self.run_job(observations, [task])
        self.assertEqual(code, 0)
        metrics = self.metrics_rows()
        self.assertEqual(len(metrics), 1)
        self.assertIsNone(metrics[0]["wape"])
        self.assertEqual(metrics[0]["mae"], 5.0)
        self.assertEqual(metrics[0]["metric_kind"], "mae_fallback")
        self.assertEqual(metrics[0]["metric_value"], 5.0)

    def test_zero_training_values_forecast_zero(self):
        pid, lid = h.series_uuid("p4"), h.series_uuid("l4")
        start = h.monday("2026-01-05")
        observations = h.weekly_rows(pid, lid, start, [0.0] * 12)
        weeks = h.week_grid(start, 12)
        task = h.production_task(h.series_uuid("p4/prod"), pid, lid, weeks)

        code, stdout, _ = self.run_job(observations, [task])
        self.assertEqual(code, 0)
        points = self.point_rows()
        self.assertEqual(len(points), 12)
        self.assertEqual({p["value"] for p in points}, {0.0})
        status, _ = self.status_by_task(stdout)
        self.assertEqual(status[task["task_id"]]["status"], "completed")

    def test_per_series_backtest_metrics_are_independent(self):
        start = h.monday("2026-01-05")
        weeks = h.week_grid(start, 24)
        pid_a, lid_a = h.series_uuid("pA"), h.series_uuid("lA")
        pid_b, lid_b = h.series_uuid("pB"), h.series_uuid("lB")
        observations = h.weekly_rows(pid_a, lid_a, start, [10.0] * 24)
        observations += h.weekly_rows(pid_b, lid_b, start, [4.0] * 23 + [8.0])
        task_a = h.backtest_task(h.series_uuid("pA/b1"), pid_a, lid_a, weeks, 1, 12)
        task_b = h.backtest_task(h.series_uuid("pB/b1"), pid_b, lid_b, weeks, 1, 12)

        code, stdout, _ = self.run_job(observations, [task_a, task_b])
        self.assertEqual(code, 0)
        metrics = {row["task_id"]: row for row in self.metrics_rows()}
        self.assertEqual(set(metrics), {task_a["task_id"], task_b["task_id"]})
        self.assertEqual(metrics[task_a["task_id"]]["wape"], 0.0)
        self.assertEqual(metrics[task_a["task_id"]]["mae"], 0.0)
        self.assertAlmostEqual(metrics[task_b["task_id"]]["wape"], 4.0 / 52.0, places=15)
        self.assertAlmostEqual(metrics[task_b["task_id"]]["mae"], 4.0 / 12.0, places=15)
        self.assertEqual(
            metrics[task_a["task_id"]]["origin_week_start"], h.monday("2026-03-23")
        )
        self.assertEqual(metrics[task_a["task_id"]]["train_size"], 12)

    def test_insufficient_history_task_fails_without_failing_the_job(self):
        pid, lid = h.series_uuid("p5"), h.series_uuid("l5")
        start = h.monday("2026-01-05")
        observations = h.weekly_rows(pid, lid, start, [3.0] * 12)
        short = h.production_task(
            h.series_uuid("p5/short"), pid, lid, h.week_grid(start, 11)
        )
        ok = h.production_task(h.series_uuid("p5/ok"), pid, lid, h.week_grid(start, 12))

        code, stdout, _ = self.run_job(observations, [short, ok])
        self.assertEqual(code, 0)
        self.assertEqual(stdout.terminal()["type"], "job.result")
        status, rows = self.status_by_task(stdout)
        self.assertEqual(len(rows), 2)
        self.assertEqual(status[short["task_id"]]["status"], "failed")
        self.assertEqual(status[short["task_id"]]["failure_code"], "insufficient_history")
        self.assertIsNotNone(status[short["task_id"]]["failure_detail"])
        self.assertEqual(status[ok["task_id"]]["status"], "completed")
        points = self.point_rows()
        self.assertEqual({p["task_id"] for p in points}, {ok["task_id"]})
        self.assertEqual(len(points), 12)
        self.assertEqual(self.metrics_rows(), [])

    def test_unknown_model_is_a_task_failure(self):
        pid, lid = h.series_uuid("p6"), h.series_uuid("l6")
        start = h.monday("2026-01-05")
        observations = h.weekly_rows(pid, lid, start, [2.0] * 12)
        task = h.production_task(
            h.series_uuid("p6/seasonal"),
            pid,
            lid,
            h.week_grid(start, 12),
            model_id="auto_arima",
        )

        code, stdout, _ = self.run_job(observations, [task])
        self.assertEqual(code, 0)
        self.assertEqual(stdout.terminal()["type"], "job.result")
        status, _ = self.status_by_task(stdout)
        self.assertEqual(status[task["task_id"]]["status"], "failed")
        self.assertEqual(status[task["task_id"]]["failure_code"], "unsupported_model")
        self.assertEqual(self.point_rows(), [])

    def test_horizon_must_be_exactly_12(self):
        pid, lid = h.series_uuid("p7"), h.series_uuid("l7")
        start = h.monday("2026-01-05")
        observations = h.weekly_rows(pid, lid, start, [2.0] * 12)
        task = h.production_task(
            h.series_uuid("p7/h11"), pid, lid, h.week_grid(start, 12), horizon_weeks=11
        )

        code, stdout, _ = self.run_job(observations, [task])
        self.assertEqual(code, 0)
        status, _ = self.status_by_task(stdout)
        self.assertEqual(status[task["task_id"]]["failure_code"], "invalid_geometry")

    def test_forecast_start_must_follow_train_end(self):
        pid, lid = h.series_uuid("p8"), h.series_uuid("l8")
        start = h.monday("2026-01-05")
        observations = h.weekly_rows(pid, lid, start, [2.0] * 12)
        weeks = h.week_grid(start, 12)
        task = h.production_task(h.series_uuid("p8/gap"), pid, lid, weeks)
        task["forecast_start_week"] = h.add_weeks(weeks[-1], 2)

        code, stdout, _ = self.run_job(observations, [task])
        self.assertEqual(code, 0)
        status, _ = self.status_by_task(stdout)
        self.assertEqual(status[task["task_id"]]["failure_code"], "invalid_geometry")

    def test_non_monday_week_dates_are_rejected(self):
        pid, lid = h.series_uuid("p9"), h.series_uuid("l9")
        start = h.monday("2026-01-05")
        tuesday = start + h.dt.timedelta(days=1)
        observations = h.weekly_rows(pid, lid, tuesday, [2.0] * 12)
        task = h.production_task(
            h.series_uuid("p9/tue"), pid, lid, h.week_grid(tuesday, 12)
        )

        code, stdout, _ = self.run_job(observations, [task])
        self.assertEqual(code, 0)
        status, _ = self.status_by_task(stdout)
        self.assertEqual(status[task["task_id"]]["failure_code"], "invalid_geometry")

    def test_missing_training_week_fails_task(self):
        pid, lid = h.series_uuid("p10"), h.series_uuid("l10")
        start = h.monday("2026-01-05")
        values = [2.0] * 12
        values[4] = None
        observations = h.weekly_rows(pid, lid, start, values)
        task = h.production_task(h.series_uuid("p10/hole"), pid, lid, h.week_grid(start, 12))

        code, stdout, _ = self.run_job(observations, [task])
        self.assertEqual(code, 0)
        status, _ = self.status_by_task(stdout)
        self.assertEqual(status[task["task_id"]]["failure_code"], "missing_training_data")

    def test_null_quantity_inside_training_window_fails_task(self):
        pid, lid = h.series_uuid("p11"), h.series_uuid("l11")
        start = h.monday("2026-01-05")
        observations = h.weekly_rows(pid, lid, start, [2.0] * 11)
        observations.append(
            {
                "product_id": pid,
                "location_id": lid,
                "week_start": h.add_weeks(start, 11),
                "quantity": None,
                "status": "missing",
            }
        )
        task = h.production_task(h.series_uuid("p11/nul"), pid, lid, h.week_grid(start, 12))

        code, stdout, _ = self.run_job(observations, [task])
        self.assertEqual(code, 0)
        status, _ = self.status_by_task(stdout)
        self.assertEqual(status[task["task_id"]]["failure_code"], "missing_training_data")

    def test_non_finite_training_value_fails_task(self):
        pid, lid = h.series_uuid("p12"), h.series_uuid("l12")
        start = h.monday("2026-01-05")
        values = [2.0] * 11 + [float("nan")]
        observations = h.weekly_rows(pid, lid, start, values)
        task = h.production_task(h.series_uuid("p12/nan"), pid, lid, h.week_grid(start, 12))

        code, stdout, _ = self.run_job(observations, [task])
        self.assertEqual(code, 0)
        status, _ = self.status_by_task(stdout)
        self.assertEqual(status[task["task_id"]]["failure_code"], "non_finite_values")

    def test_duplicate_training_week_fails_task(self):
        pid, lid = h.series_uuid("p13"), h.series_uuid("l13")
        start = h.monday("2026-01-05")
        observations = h.weekly_rows(pid, lid, start, [2.0] * 12)
        observations.append(observations[3])
        task = h.production_task(h.series_uuid("p13/dup"), pid, lid, h.week_grid(start, 12))

        code, stdout, _ = self.run_job(observations, [task])
        self.assertEqual(code, 0)
        status, _ = self.status_by_task(stdout)
        self.assertEqual(status[task["task_id"]]["failure_code"], "duplicate_observation")

    def test_missing_backtest_actuals_fail_only_that_task(self):
        pid, lid = h.series_uuid("p14"), h.series_uuid("l14")
        start = h.monday("2026-01-05")
        observations = h.weekly_rows(pid, lid, start, [2.0] * 12 + [2.0] * 5)
        weeks = h.week_grid(start, 24)
        backtest = h.backtest_task(h.series_uuid("p14/b1"), pid, lid, weeks, 1, 12)
        production = h.production_task(
            h.series_uuid("p14/prod"), pid, lid, h.week_grid(start, 12)
        )

        code, stdout, _ = self.run_job(observations, [backtest, production])
        self.assertEqual(code, 0)
        self.assertEqual(stdout.terminal()["type"], "job.result")
        status, _ = self.status_by_task(stdout)
        self.assertEqual(
            status[backtest["task_id"]]["failure_code"], "missing_backtest_actuals"
        )
        self.assertEqual(status[production["task_id"]]["status"], "completed")
        self.assertEqual(
            {p["task_id"] for p in self.point_rows()}, {production["task_id"]}
        )
        self.assertEqual(self.metrics_rows(), [])

    def test_origin_index_rules_for_kinds(self):
        pid, lid = h.series_uuid("p15"), h.series_uuid("l15")
        start = h.monday("2026-01-05")
        observations = h.weekly_rows(pid, lid, start, [2.0] * 24)
        weeks = h.week_grid(start, 24)
        production_with_origin = h.production_task(
            h.series_uuid("p15/prod-origin"),
            pid,
            lid,
            h.week_grid(start, 12),
            origin_index=1,
        )
        backtest_without_origin = h.backtest_task(
            h.series_uuid("p15/b-null"), pid, lid, weeks, 1, 12
        )
        backtest_without_origin["origin_index"] = None

        code, stdout, _ = self.run_job(
            observations, [production_with_origin, backtest_without_origin]
        )
        self.assertEqual(code, 0)
        status, rows = self.status_by_task(stdout)
        self.assertEqual(len(rows), 2)
        for task_id in (production_with_origin["task_id"], backtest_without_origin["task_id"]):
            self.assertEqual(status[task_id]["status"], "failed")
            self.assertEqual(status[task_id]["failure_code"], "invalid_geometry")

    def test_empty_task_plan_produces_full_schema_empty_outputs(self):
        code, stdout, _ = self.run_job([], [])
        self.assertEqual(code, 0)
        terminal = stdout.terminal()
        self.assertEqual(terminal["type"], "job.result")
        for key, (schema, filename) in h.OUTPUT_SCHEMAS.items():
            path = self.job_dir / filename
            self.assertTrue(path.exists(), filename)
            self.assertEqual(h.read_schema(path), schema)
            table = h.read_table(path)
            self.assertEqual(table.num_rows, 0)
            self.assertEqual(terminal["output_files"][key]["rows"], 0)
            self.assertEqual(terminal["output_files"][key]["path"], filename)
            self.assertTrue(terminal["output_files"][key]["sha256"])

    # -- Milestone 3 models ------------------------------------------------

    def test_seasonal_naive_repeats_the_annual_cycle(self):
        pid, lid = h.series_uuid("m1"), h.series_uuid("m1")
        start = h.monday("2024-01-01")
        values = [float(i % 52) for i in range(104)]
        observations = h.weekly_rows(pid, lid, start, values)
        weeks = h.week_grid(start, 104)
        task = h.production_task(
            h.series_uuid("m1/prod"), pid, lid, weeks, model_id="seasonal_naive_52"
        )

        code, stdout, _ = self.run_job(observations, [task])
        self.assertEqual(code, 0)
        status, _ = self.status_by_task(stdout)
        self.assertEqual(status[task["task_id"]]["status"], "completed")
        points = self.point_rows()
        self.assertEqual(len(points), 12)
        # The 52-week cycle repeats: weeks 52..63 of the training window.
        self.assertEqual([p["value"] for p in points], [float(i % 52) for i in range(52, 64)])

    def test_auto_ets_continues_a_trend_and_reports_wape(self):
        pid, lid = h.series_uuid("m2"), h.series_uuid("m2")
        start = h.monday("2023-01-02")
        values = [10.0 + 2.0 * i for i in range(116)]
        observations = h.weekly_rows(pid, lid, start, values)
        weeks = h.week_grid(start, 116)
        task = h.backtest_task(
            h.series_uuid("m2/b1"), pid, lid, weeks, 1, 104, model_id="auto_ets_52"
        )

        code, stdout, _ = self.run_job(observations, [task])
        self.assertEqual(code, 0)
        status, _ = self.status_by_task(stdout)
        self.assertEqual(status[task["task_id"]]["status"], "completed")
        points = self.point_rows()
        self.assertEqual(len(points), 12)
        self.assertTrue(all(math.isfinite(p["value"]) for p in points))
        # A clean linear trend: ETS should score a low but non-zero WAPE.
        metrics = self.metrics_rows()
        self.assertEqual(len(metrics), 1)
        self.assertEqual(metrics[0]["metric_kind"], "wape")
        self.assertIsNotNone(metrics[0]["wape"])
        self.assertLess(metrics[0]["wape"], 0.25)

    def test_seasonal_models_fail_below_their_history_floor(self):
        pid, lid = h.series_uuid("m3"), h.series_uuid("m3")
        start = h.monday("2024-01-01")
        values = [5.0] * 103
        observations = h.weekly_rows(pid, lid, start, values)
        weeks = h.week_grid(start, 103)
        seasonal = h.production_task(
            h.series_uuid("m3/sn"), pid, lid, weeks, model_id="seasonal_naive_52"
        )
        ets = h.production_task(
            h.series_uuid("m3/ets"), pid, lid, weeks, model_id="auto_ets_52"
        )
        naive = h.production_task(h.series_uuid("m3/naive"), pid, lid, weeks)

        code, stdout, _ = self.run_job(observations, [seasonal, ets, naive])
        self.assertEqual(code, 0)
        status, _ = self.status_by_task(stdout)
        self.assertEqual(status[seasonal["task_id"]]["failure_code"], "insufficient_history")
        self.assertEqual(status[ets["task_id"]]["failure_code"], "insufficient_history")
        self.assertEqual(status[naive["task_id"]]["status"], "completed")
        self.assertEqual(len(self.point_rows()), 12)

    def test_all_zero_actuals_use_mae_fallback_for_every_model(self):
        pid, lid = h.series_uuid("m4"), h.series_uuid("m4")
        start = h.monday("2024-01-01")
        values = [5.0] * 116 + [0.0] * 12
        observations = h.weekly_rows(pid, lid, start, values)
        weeks = h.week_grid(start, 128)
        tasks = [
            h.backtest_task(
                h.series_uuid(f"m4/{model}"),
                pid,
                lid,
                weeks,
                1,
                116,
                model_id=model,
            )
            for model in ("naive", "seasonal_naive_52", "auto_ets_52")
        ]

        code, stdout, _ = self.run_job(observations, tasks)
        self.assertEqual(code, 0)
        status, _ = self.status_by_task(stdout)
        for task in tasks:
            self.assertEqual(status[task["task_id"]]["status"], "completed")
        metrics = {row["task_id"]: row for row in self.metrics_rows()}
        self.assertEqual(len(metrics), 3)
        for task in tasks:
            model = str(task["model_id"])
            self.assertIsNone(metrics[task["task_id"]]["wape"])
            self.assertEqual(metrics[task["task_id"]]["metric_kind"], "mae_fallback")
            self.assertTrue(model in ("naive", "seasonal_naive_52", "auto_ets_52"))

    def test_intermittent_history_forecasts_for_every_model(self):
        pid, lid = h.series_uuid("m6"), h.series_uuid("m6")
        start = h.monday("2024-01-01")
        values = [0.0] * 104
        for index in range(0, 104, 9):
            values[index] = 5.0
        observations = h.weekly_rows(pid, lid, start, values)
        weeks = h.week_grid(start, 104)
        tasks = [
            h.production_task(
                h.series_uuid(f"m6/{model}"), pid, lid, weeks, model_id=model
            )
            for model in ("naive", "seasonal_naive_52", "auto_ets_52")
        ]

        code, stdout, _ = self.run_job(observations, tasks)
        self.assertEqual(code, 0)
        status, _ = self.status_by_task(stdout)
        for task in tasks:
            self.assertEqual(status[task["task_id"]]["status"], "completed")
        points = self.point_rows()
        self.assertEqual(len(points), 36)
        self.assertTrue(all(math.isfinite(point["value"]) for point in points))

    def test_seasonal_naive_beats_naive_on_a_seasonal_backtest(self):
        pid, lid = h.series_uuid("m5"), h.series_uuid("m5")
        start = h.monday("2024-01-01")
        values = [float(i % 52) for i in range(116)]
        observations = h.weekly_rows(pid, lid, start, values)
        weeks = h.week_grid(start, 116)
        naive = h.backtest_task(h.series_uuid("m5/naive"), pid, lid, weeks, 1, 104)
        seasonal = h.backtest_task(
            h.series_uuid("m5/sn"), pid, lid, weeks, 1, 104, model_id="seasonal_naive_52"
        )

        code, stdout, _ = self.run_job(observations, [naive, seasonal])
        self.assertEqual(code, 0)
        metrics = {row["task_id"]: row for row in self.metrics_rows()}
        self.assertEqual(len(metrics), 2)
        self.assertLess(
            metrics[seasonal["task_id"]]["wape"], metrics[naive["task_id"]]["wape"]
        )


if __name__ == "__main__":
    unittest.main()
