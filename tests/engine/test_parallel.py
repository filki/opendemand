"""Parallel execution tests for the bounded sidecar worker pool.

These tests drive the public ``run_job`` seam with a job large enough to pool
(the threshold is lowered for the test), then check that the pooled path is
byte-identical to the sequential path, deterministic across worker counts,
cancellable under load, and free of leftover child processes.
"""

from __future__ import annotations

import datetime as dt
import hashlib
import io
import math
import multiprocessing
import os
import tempfile
import threading
import time
import unittest
from pathlib import Path
from unittest import mock

from tests.engine import helpers as h

MODELS = [("naive", 12), ("seasonal_naive_52", 104), ("auto_ets_52", 104)]
OUTPUT_FILES = ("task_status", "origin_metrics", "forecast_points")


def _plan(pid: str, lid: str, start: dt.date, weeks: int) -> list[dict]:
    grid = h.week_grid(start, weeks)
    tasks: list[dict] = []
    for model_id, floor in MODELS:
        if weeks < floor:
            continue
        origins = max(0, min(4, (weeks - floor) // 12))
        tasks.append(
            h.production_task(
                h.series_uuid(f"{pid}/{model_id}/prod"), pid, lid, grid, model_id=model_id
            )
        )
        for index in range(1, origins + 1):
            tasks.append(
                h.backtest_task(
                    h.series_uuid(f"{pid}/{model_id}/b{index}"),
                    pid,
                    lid,
                    grid,
                    index,
                    weeks - 12 * (origins - index + 1),
                    model_id=model_id,
                )
            )
    return tasks


def _build(series_count: int, weeks: int = 116, duplicate_series: int | None = None):
    start = h.monday("2023-03-06")
    observations: list[dict] = []
    tasks: list[dict] = []
    for index in range(series_count):
        pid, lid = h.series_uuid(f"par-p{index}"), h.series_uuid(f"par-l{index}")
        values = [
            100 + 40 * math.sin(2 * math.pi * (week % 52) / 52) + (week % 7)
            for week in range(weeks)
        ]
        rows = h.weekly_rows(pid, lid, start, values)
        if duplicate_series == index:
            rows.append(dict(rows[0]))  # a duplicate weekly observation
        observations += rows
        tasks += _plan(pid, lid, start, weeks)
    return observations, tasks


def _digests(job_dir: Path) -> dict[str, str]:
    return {
        name: hashlib.sha256((job_dir / f"{name}.arrow").read_bytes()).hexdigest()
        for name in OUTPUT_FILES
    }


class ParallelExecutionTests(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.job_dir = Path(self._tmp.name)

    def tearDown(self):
        self._tmp.cleanup()

    def _run(self, workers: int, threshold: int = 1, series: int = 8, **build_kwargs):
        for name in OUTPUT_FILES:
            (self.job_dir / f"{name}.arrow").unlink(missing_ok=True)
        observations, tasks = _build(series, **build_kwargs)
        h.write_inputs(self.job_dir, observations, tasks)
        with (
            mock.patch.dict(os.environ, {"OPENDEMAND_ENGINE_WORKERS": str(workers)}),
            mock.patch("opendemand_engine.engine.PARALLEL_MIN_EXPENSIVE_TASKS", threshold),
        ):
            return h.run_inproc(self.job_dir, [h.job_request()])

    def test_pooled_output_matches_sequential_output_byte_for_byte(self):
        _, stdout_seq, _ = self._run(workers=1)
        sequential = _digests(self.job_dir)
        self.assertEqual(stdout_seq.terminal()["type"], "job.result")

        _, stdout_pool, stderr = self._run(workers=2)
        pooled = _digests(self.job_dir)

        self.assertEqual(stdout_pool.terminal()["type"], "job.result")
        self.assertIn("executing with 2 worker processes", stderr)
        self.assertEqual(sequential, pooled)

    def test_output_is_deterministic_across_worker_counts(self):
        digests = []
        for workers in (1, 2, 3):
            _, stdout, _ = self._run(workers=workers)
            self.assertEqual(stdout.terminal()["type"], "job.result")
            digests.append(_digests(self.job_dir))
        self.assertEqual(digests[0], digests[1])
        self.assertEqual(digests[1], digests[2])

    def test_partial_failures_match_between_paths(self):
        # One series carries a duplicate observation: its tasks fail while every
        # other task completes. The failure must be identical on both paths.
        _, stdout_seq, _ = self._run(workers=1, duplicate_series=3)
        sequential = _digests(self.job_dir)
        self.assertEqual(stdout_seq.terminal()["type"], "job.result")

        _, stdout_pool, _ = self._run(workers=2, duplicate_series=3)
        pooled = _digests(self.job_dir)

        self.assertEqual(stdout_pool.terminal()["type"], "job.result")
        self.assertEqual(sequential, pooled)
        statuses = h.read_table(self.job_dir / "task_status.arrow").to_pylist()
        self.assertTrue(any(row["status"] == "failed" for row in statuses))
        self.assertTrue(any(row["status"] == "completed" for row in statuses))

    def test_cancel_during_a_pooled_run_yields_job_cancelled_without_outputs(self):
        observations, tasks = _build(8)
        h.write_inputs(self.job_dir, observations, tasks)

        def cancel_once_started(stdout: h.FakeStdout, started: threading.Event):
            deadline = time.monotonic() + 10.0
            while time.monotonic() < deadline:
                if any(e.get("type") == "job.progress" for e in stdout.envelopes()):
                    h.request_cancel(self.job_dir)
                    started.set()
                    return
                time.sleep(0.005)

        stdin = h.FakeStdin([h.job_request("job-cancel")])
        stdin.eof()
        stdout = h.FakeStdout()
        started = threading.Event()
        watcher = threading.Thread(target=cancel_once_started, args=(stdout, started))
        watcher.start()
        try:
            from opendemand_engine import engine

            with (
                mock.patch.dict(os.environ, {"OPENDEMAND_ENGINE_WORKERS": "2"}),
                mock.patch("opendemand_engine.engine.PARALLEL_MIN_EXPENSIVE_TASKS", 1),
            ):
                code = engine.run_job(
                    self.job_dir, stdin, stdout, io.StringIO(), heartbeat_interval=15.0
                )
        finally:
            watcher.join(timeout=5.0)

        self.assertTrue(started.is_set(), "the pooled run never produced progress")
        self.assertEqual(code, 0)
        terminal = stdout.terminal()
        self.assertEqual(terminal["type"], "job.cancelled")
        self.assertEqual(terminal["job_id"], "job-cancel")
        self.assertEqual(h.output_files_present(self.job_dir), [])

    def test_pooled_run_leaves_no_child_processes(self):
        self._run(workers=2)
        # The pool is shut down inside run_job; no spawn worker may outlive it.
        self.assertEqual(multiprocessing.active_children(), [])

    def test_worker_failure_fails_the_job_without_outputs(self):
        with mock.patch("opendemand_engine.engine.ProcessPoolExecutor") as executor:
            pool = executor.return_value
            pool.submit.return_value.result.side_effect = RuntimeError("worker failed")
            code, stdout, _ = self._run(workers=2)

        self.assertEqual(code, 1)
        terminal = stdout.terminal()
        self.assertEqual(terminal["type"], "job.error")
        self.assertEqual(terminal["code"], "internal_error")
        self.assertEqual(h.output_files_present(self.job_dir), [])
        pool.shutdown.assert_called_once_with(wait=True, cancel_futures=True)

    def test_small_job_stays_in_process_when_workers_are_disabled(self):
        _, stdout, stderr = self._run(workers=1)
        self.assertEqual(stdout.terminal()["type"], "job.result")
        self.assertNotIn("executing with", stderr)
        self.assertEqual(multiprocessing.active_children(), [])

    def test_worker_count_is_bounded_and_overridable(self):
        from opendemand_engine import engine

        overrides = (("2", 2), ("32", 32), ("0", 1), ("-1", 1), ("", 1), ("nonsense", 1))
        for value, expected in overrides:
            with self.subTest(value=value), mock.patch.dict(
                os.environ, {"OPENDEMAND_ENGINE_WORKERS": value}
            ):
                self.assertEqual(engine.worker_count(), expected)
        with mock.patch.dict(os.environ, {}, clear=True):
            for cpus, expected in ((None, 1), (0, 1), (1, 1), (2, 2), (8, engine.MAX_WORKERS)):
                with self.subTest(cpus=cpus), mock.patch(
                    "opendemand_engine.engine.os.cpu_count", return_value=cpus
                ):
                    self.assertEqual(engine.worker_count(), expected)


if __name__ == "__main__":
    raise SystemExit(unittest.main())
