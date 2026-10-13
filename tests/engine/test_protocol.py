"""Protocol envelope, correlation, path-safety and cancellation tests."""

from __future__ import annotations

import hashlib
import json
import os
import tempfile
import unittest
from pathlib import Path

import opendemand_engine

from tests.engine import helpers as h


class ProtocolTests(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.job_dir = Path(self._tmp.name)
        self.start = h.monday("2026-01-05")
        self.pid, self.lid = h.series_uuid("prot-p"), h.series_uuid("prot-l")
        self.weeks = h.week_grid(self.start, 12)
        self.observations = h.weekly_rows(self.pid, self.lid, self.start, [1.0] * 12)
        self.task = h.production_task(h.series_uuid("prot/prod"), self.pid, self.lid, self.weeks)

    def tearDown(self):
        self._tmp.cleanup()

    def tiny_job(self, task_rows=None, observation_rows=None):
        h.write_inputs(
            self.job_dir,
            self.observations if observation_rows is None else observation_rows,
            [self.task] if task_rows is None else task_rows,
        )

    def assert_no_outputs(self):
        self.assertEqual(h.output_files_present(self.job_dir), [])

    def assert_protocol_only(self, envelopes):
        for envelope in envelopes:
            self.assertEqual(envelope["protocol"], h.PROTOCOL_ID)
            self.assertIn("type", envelope)

    # -- tests -------------------------------------------------------------

    def test_hello_is_the_first_line_and_exact(self):
        self.tiny_job()
        code, stdout, _ = h.run_inproc(self.job_dir, [h.job_request()])
        self.assertEqual(code, 0)
        envelopes = stdout.envelopes()
        self.assertEqual(
            envelopes[0],
            {
                "protocol": h.PROTOCOL_ID,
                "type": "hello",
                "job_id": None,
                "engine_version": opendemand_engine.__version__,
                "contracts": [h.CONTRACT_ID],
                "models": ["naive"],
            },
        )
        self.assert_protocol_only(envelopes)

    def test_happy_path_result_manifest_matches_files_on_disk(self):
        self.tiny_job()
        code, stdout, _ = h.run_inproc(self.job_dir, [h.job_request()])
        self.assertEqual(code, 0)
        terminal = stdout.terminal()
        self.assertEqual(terminal["type"], "job.result")
        self.assertEqual(terminal["job_id"], "job-1")
        self.assertEqual(
            sorted(terminal["output_files"]),
            ["forecast_points", "origin_metrics", "task_status"],
        )
        for key, (_, filename) in h.OUTPUT_SCHEMAS.items():
            entry = terminal["output_files"][key]
            self.assertEqual(entry["path"], filename)
            path = self.job_dir / filename
            digest = hashlib.sha256(path.read_bytes()).hexdigest()
            self.assertEqual(entry["sha256"], digest)
            self.assertRegex(entry["sha256"], r"^[0-9a-f]{64}$")
            self.assertEqual(entry["rows"], h.read_table(path).num_rows)
        leftovers = sorted(p.name for p in self.job_dir.iterdir() if ".tmp" in p.name)
        self.assertEqual(leftovers, [])

    def test_wrong_protocol_id_is_protocol_mismatch(self):
        self.tiny_job()
        code, stdout, _ = h.run_inproc(
            self.job_dir, [h.job_request(protocol="opendemand.sidecar/2")]
        )
        self.assertEqual(code, 1)
        terminal = stdout.terminal()
        self.assertEqual(terminal["type"], "job.error")
        self.assertEqual(terminal["code"], "protocol_mismatch")
        self.assertEqual(terminal["job_id"], "job-1")
        self.assertIs(terminal["retryable"], False)
        self.assertTrue(terminal["message"])
        self.assert_no_outputs()

    def test_unknown_type_before_request_is_protocol_mismatch(self):
        self.tiny_job()
        message = json.dumps(
            {"protocol": h.PROTOCOL_ID, "type": "job.ping", "job_id": "job-9"}
        )
        code, stdout, _ = h.run_inproc(self.job_dir, [message])
        self.assertEqual(code, 1)
        terminal = stdout.terminal()
        self.assertEqual(terminal["type"], "job.error")
        self.assertEqual(terminal["code"], "protocol_mismatch")
        self.assertEqual(terminal["job_id"], "job-9")
        self.assert_no_outputs()

    def test_duplicate_request_fails_closed(self):
        self.tiny_job()
        code, stdout, _ = h.run_inproc(
            self.job_dir, [h.job_request(), h.job_request()]
        )
        self.assertEqual(code, 1)
        terminal = stdout.terminal()
        self.assertEqual(terminal["type"], "job.error")
        self.assertEqual(terminal["code"], "protocol_mismatch")
        self.assert_no_outputs()

    def test_unsupported_contract_is_contract_violation(self):
        self.tiny_job()
        code, stdout, _ = h.run_inproc(
            self.job_dir, [h.job_request(contract="opendemand.arrow/2")]
        )
        self.assertEqual(code, 1)
        terminal = stdout.terminal()
        self.assertEqual(terminal["type"], "job.error")
        self.assertEqual(terminal["code"], "contract_violation")
        self.assert_no_outputs()

    def test_input_files_must_have_exactly_the_fixed_keys(self):
        self.tiny_job()
        code, stdout, _ = h.run_inproc(
            self.job_dir, [h.job_request(input_files={"observations": "observations.arrow"})]
        )
        self.assertEqual(code, 1)
        terminal = stdout.terminal()
        self.assertEqual(terminal["type"], "job.error")
        self.assertEqual(terminal["code"], "contract_violation")
        self.assert_no_outputs()

    def test_input_file_name_must_be_fixed(self):
        self.tiny_job()
        code, stdout, _ = h.run_inproc(
            self.job_dir,
            [h.job_request(input_files={"observations": "obs.arrow", "tasks": "tasks.arrow"})],
        )
        self.assertEqual(code, 1)
        self.assertEqual(stdout.terminal()["code"], "contract_violation")
        self.assert_no_outputs()

    def test_input_file_traversal_is_rejected(self):
        self.tiny_job()
        code, stdout, _ = h.run_inproc(
            self.job_dir,
            [
                h.job_request(
                    input_files={"observations": "../observations.arrow", "tasks": "tasks.arrow"}
                )
            ],
        )
        self.assertEqual(code, 1)
        self.assertEqual(stdout.terminal()["code"], "contract_violation")
        self.assert_no_outputs()

    def test_input_file_absolute_path_is_rejected(self):
        self.tiny_job()
        code, stdout, _ = h.run_inproc(
            self.job_dir,
            [
                h.job_request(
                    input_files={
                        "observations": "/tmp/observations.arrow",
                        "tasks": "tasks.arrow",
                    }
                )
            ],
        )
        self.assertEqual(code, 1)
        self.assertEqual(stdout.terminal()["code"], "contract_violation")
        self.assert_no_outputs()

    def test_symlinked_input_file_is_rejected(self):
        outside = Path(self._tmp.name) / "outside"
        outside.mkdir()
        h.write_rows(outside / "observations.arrow", h.OBSERVATIONS_SCHEMA, self.observations)
        h.write_rows(self.job_dir / "tasks.arrow", h.TASKS_SCHEMA, [self.task])
        os.symlink(outside / "observations.arrow", self.job_dir / "observations.arrow")

        code, stdout, _ = h.run_inproc(self.job_dir, [h.job_request()])
        self.assertEqual(code, 1)
        terminal = stdout.terminal()
        self.assertEqual(terminal["type"], "job.error")
        self.assertEqual(terminal["code"], "invalid_input")
        self.assert_no_outputs()

    def test_missing_input_file_is_invalid_input(self):
        h.write_rows(self.job_dir / "tasks.arrow", h.TASKS_SCHEMA, [self.task])
        code, stdout, _ = h.run_inproc(self.job_dir, [h.job_request()])
        self.assertEqual(code, 1)
        terminal = stdout.terminal()
        self.assertEqual(terminal["type"], "job.error")
        self.assertEqual(terminal["code"], "invalid_input")
        self.assert_no_outputs()

    def test_cancel_with_wrong_job_id_is_protocol_mismatch(self):
        self.tiny_job()
        code, stdout, _ = h.run_inproc(
            self.job_dir, [h.job_request("job-1"), h.cancel_message("job-other")]
        )
        self.assertEqual(code, 1)
        terminal = stdout.terminal()
        self.assertEqual(terminal["type"], "job.error")
        self.assertEqual(terminal["code"], "protocol_mismatch")
        self.assertEqual(terminal["job_id"], "job-1")
        self.assert_no_outputs()

    def test_stdin_eof_after_request_still_completes(self):
        self.tiny_job()
        code, stdout, _ = h.run_inproc(self.job_dir, [h.job_request()], eof=True)
        self.assertEqual(code, 0)
        self.assertEqual(stdout.terminal()["type"], "job.result")

    def test_cancel_before_any_task_yields_job_cancelled_without_outputs(self):
        self.tiny_job()
        code, stdout, _ = h.run_inproc(
            self.job_dir, [h.job_request("job-7"), h.cancel_message("job-7")]
        )
        self.assertEqual(code, 0)
        terminal = stdout.terminal()
        self.assertEqual(terminal["type"], "job.cancelled")
        self.assertEqual(terminal["job_id"], "job-7")
        self.assert_no_outputs()
        self.assertNotIn("job.result", [e["type"] for e in stdout.envelopes()])


class HeartbeatTests(unittest.TestCase):
    def test_heartbeat_reemits_last_progress(self):
        import io
        import time

        from opendemand_engine.engine import Heartbeat, ProgressTracker
        from opendemand_engine.protocol import EnvelopeWriter

        buffer = io.StringIO()
        tracker = ProgressTracker(EnvelopeWriter(buffer), "job-hb")
        tracker.update("backtest", 1, 3)
        heartbeat = Heartbeat(0.05, tracker.flush)
        heartbeat.start()
        try:
            time.sleep(0.2)
        finally:
            heartbeat.stop()

        progress = [
            json.loads(line)
            for line in buffer.getvalue().splitlines()
            if line.strip()
        ]
        self.assertGreaterEqual(len(progress), 2)
        for envelope in progress:
            self.assertEqual(envelope["type"], "job.progress")
            self.assertEqual(envelope["job_id"], "job-hb")
            self.assertEqual(envelope["completed"], 1)
            self.assertEqual(envelope["total"], 3)


if __name__ == "__main__":
    unittest.main()
