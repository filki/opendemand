"""End-to-end tests through the real CLI subprocess and Arrow files on disk."""

from __future__ import annotations

import hashlib
import tempfile
import unittest
from pathlib import Path

from tests.engine import helpers as h

PHASES = {"validate_input", "backtest", "production", "write_output"}


class CliSubprocessTests(unittest.TestCase):
    def test_demo_fixture_roundtrip(self):
        series = h.load_demo_series()
        tasks = h.plan_demo_tasks(series)

        with tempfile.TemporaryDirectory() as tmp:
            job_dir = Path(tmp)
            h.write_inputs(job_dir, h.demo_observation_rows(series), tasks)
            proc = h.EngineProcess(job_dir)
            hello = proc.read_envelope(timeout=60)
            self.assertEqual(hello["type"], "hello")
            proc.send(h.job_request("job-demo"))
            terminal = proc.read_until_terminal(timeout=300)
            self.assertEqual(terminal["type"], "job.result")
            self.assertEqual(terminal["job_id"], "job-demo")
            rc = proc.finish()
            self.assertEqual(rc, 0)
            self.assertEqual(proc.drain_stdout(), [])

            # every stdout line was protocol JSON with the right protocol id
            for envelope in proc.envelopes:
                self.assertEqual(envelope["protocol"], h.PROTOCOL_ID)

            status_rows = h.read_table(job_dir / "task_status.arrow").to_pylist()
            self.assertEqual(len(status_rows), len(tasks))
            by_task = {row["task_id"]: row for row in status_rows}
            completed = [row for row in status_rows if row["status"] == "completed"]
            failed = [row for row in status_rows if row["status"] == "failed"]
            self.assertEqual(len(completed), 6)
            self.assertEqual(len(failed), 1)
            self.assertEqual(failed[0]["failure_code"], "insufficient_history")
            self.assertEqual(
                failed[0]["task_id"], h.series_uuid("DEMAND-06/production")
            )

            metric_rows = h.read_table(job_dir / "origin_metrics.arrow").to_pylist()
            self.assertEqual(len(metric_rows), 4)
            metrics_by_task = {row["task_id"]: row for row in metric_rows}
            first = metrics_by_task[h.series_uuid("DEMAND-60/backtest/1")]
            self.assertEqual(first["wape"], 0.0)
            self.assertEqual(first["mae"], 0.0)
            self.assertEqual(first["metric_kind"], "wape")
            self.assertEqual(first["train_size"], 12)
            last = metrics_by_task[h.series_uuid("DEMAND-60/backtest/4")]
            self.assertAlmostEqual(last["wape"], 10.0 / 130.0, places=12)
            self.assertAlmostEqual(last["mae"], 10.0 / 12.0, places=12)
            self.assertEqual(last["train_size"], 48)

            points = h.read_table(job_dir / "forecast_points.arrow").to_pylist()
            self.assertEqual(len(points), 6 * 12)
            prod_60 = [
                p for p in points if p["task_id"] == h.series_uuid("DEMAND-60/production")
            ]
            prod_20 = [
                p for p in points if p["task_id"] == h.series_uuid("DEMAND-20/production")
            ]
            self.assertEqual(len(prod_60), 12)
            self.assertEqual({p["value"] for p in prod_60}, {20.0})
            self.assertEqual(len(prod_20), 12)
            self.assertEqual({p["value"] for p in prod_20}, {5.0})

            for key, (_, filename) in h.OUTPUT_SCHEMAS.items():
                entry = terminal["output_files"][key]
                self.assertEqual(entry["path"], filename)
                path = job_dir / filename
                self.assertEqual(entry["sha256"], hashlib.sha256(path.read_bytes()).hexdigest())
                self.assertEqual(entry["rows"], h.read_table(path).num_rows)
                self.assertEqual(h.read_schema(path).metadata, h.meta(key))

            progresses = [e for e in proc.envelopes if e["type"] == "job.progress"]
            self.assertGreater(len(progresses), 0)
            self.assertEqual(progresses[0]["phase"], "validate_input")
            self.assertEqual(progresses[-1]["phase"], "write_output")
            self.assertEqual(progresses[-1]["completed"], len(tasks))
            seen_phases = {e["phase"] for e in progresses}
            self.assertLessEqual(seen_phases, PHASES)
            completed_counts = [e["completed"] for e in progresses]
            self.assertEqual(completed_counts, sorted(completed_counts))
            for envelope in progresses:
                self.assertEqual(envelope["total"], len(tasks))
                self.assertEqual(envelope["job_id"], "job-demo")

    def test_cancellation_returns_job_cancelled_and_no_outputs(self):
        start = h.monday("2026-01-05")
        pid, lid = h.series_uuid("cancel-p"), h.series_uuid("cancel-l")
        observations = h.weekly_rows(pid, lid, start, [1.0] * 12)
        task = h.production_task(h.series_uuid("cancel/prod"), pid, lid, h.week_grid(start, 12))

        with tempfile.TemporaryDirectory() as tmp:
            job_dir = Path(tmp)
            h.write_inputs(job_dir, observations, [task])
            proc = h.EngineProcess(job_dir)
            hello = proc.read_envelope(timeout=60)
            self.assertEqual(hello["type"], "hello")
            proc.send(h.job_request("job-cancel"))
            proc.send(h.cancel_message("job-cancel"))
            terminal = proc.read_until_terminal(timeout=120)
            self.assertEqual(terminal["type"], "job.cancelled")
            self.assertEqual(terminal["job_id"], "job-cancel")
            rc = proc.finish()
            self.assertEqual(rc, 0)
            self.assertEqual(h.output_files_present(job_dir), [])

    def test_cli_rejects_missing_job_dir(self):
        missing = Path(tempfile.gettempdir()) / "opendemand-engine-missing-dir"
        proc = h.EngineProcess(missing, cwd=h.REPO_ROOT)
        rc = proc.finish(timeout=60)
        self.assertNotEqual(rc, 0)
        self.assertEqual(proc.drain_stdout(), [])
        self.assertIn("opendemand-engine", proc.stderr_text)

    def test_stderr_never_contains_business_values(self):
        start = h.monday("2026-01-05")
        pid, lid = h.series_uuid("secret-p"), h.series_uuid("secret-l")
        sentinel = 12345.6789
        observations = h.weekly_rows(pid, lid, start, [sentinel] * 12)
        task = h.production_task(h.series_uuid("secret/prod"), pid, lid, h.week_grid(start, 12))

        with tempfile.TemporaryDirectory() as tmp:
            job_dir = Path(tmp)
            h.write_inputs(job_dir, observations, [task])
            proc = h.EngineProcess(job_dir)
            proc.read_envelope(timeout=60)
            proc.send(h.job_request("job-secret"))
            terminal = proc.read_until_terminal(timeout=120)
            self.assertEqual(terminal["type"], "job.result")
            proc.finish()
            stderr_text = proc.stderr_text
            for needle in (pid, lid, "12345.678", "12345", str(sentinel)):
                self.assertNotIn(needle, stderr_text)


if __name__ == "__main__":
    unittest.main()
