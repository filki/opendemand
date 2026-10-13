"""Arrow IPC contract violations must fail the whole job closed."""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

import pyarrow as pa

from tests.engine import helpers as h


class ArrowContractTests(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.job_dir = Path(self._tmp.name)
        self.start = h.monday("2026-01-05")
        self.pid, self.lid = h.series_uuid("ct-p"), h.series_uuid("ct-l")
        self.weeks = h.week_grid(self.start, 12)
        self.observations = h.weekly_rows(self.pid, self.lid, self.start, [1.0] * 12)
        self.task = h.production_task(h.series_uuid("ct/prod"), self.pid, self.lid, self.weeks)

    def tearDown(self):
        self._tmp.cleanup()

    def run_job(self, observations_schema=None, tasks_schema=None):
        h.write_inputs(
            self.job_dir,
            self.observations,
            [self.task],
            observations_schema=observations_schema or h.OBSERVATIONS_SCHEMA,
            tasks_schema=tasks_schema or h.TASKS_SCHEMA,
        )
        return h.run_inproc(self.job_dir, [h.job_request()])

    def assert_contract_violation(self, stdout):
        terminal = stdout.terminal()
        self.assertEqual(terminal["type"], "job.error")
        self.assertEqual(terminal["code"], "contract_violation")
        self.assertEqual(h.output_files_present(self.job_dir), [])

    def test_wrong_column_type_fails_closed(self):
        schema = h.replace_field(
            h.OBSERVATIONS_SCHEMA, "week_start", pa.field("week_start", pa.utf8(), nullable=False)
        )
        observations = [
            {**row, "week_start": row["week_start"].isoformat()} for row in self.observations
        ]
        h.write_rows(self.job_dir / "observations.arrow", schema, observations)
        h.write_rows(self.job_dir / "tasks.arrow", h.TASKS_SCHEMA, [self.task])
        code, stdout, _ = h.run_inproc(self.job_dir, [h.job_request()])
        self.assertEqual(code, 1)
        self.assert_contract_violation(stdout)

    def test_wrong_nullability_fails_closed(self):
        schema = h.replace_field(
            h.OBSERVATIONS_SCHEMA, "quantity", pa.field("quantity", pa.float64(), nullable=False)
        )
        _, stdout, _ = self.run_job(observations_schema=schema)
        self.assert_contract_violation(stdout)

    def test_column_order_mismatch_fails_closed(self):
        fields = list(h.OBSERVATIONS_SCHEMA)
        fields[3], fields[4] = fields[4], fields[3]
        schema = pa.schema(fields, metadata=h.OBSERVATIONS_SCHEMA.metadata)
        _, stdout, _ = self.run_job(observations_schema=schema)
        self.assert_contract_violation(stdout)

    def test_missing_contract_metadata_fails_closed(self):
        schema = h.without_metadata(h.OBSERVATIONS_SCHEMA)
        _, stdout, _ = self.run_job(observations_schema=schema)
        self.assert_contract_violation(stdout)

    def test_wrong_contract_metadata_value_fails_closed(self):
        schema = h.OBSERVATIONS_SCHEMA.with_metadata(
            {
                b"opendemand.contract": b"opendemand.arrow/2",
                b"opendemand.table": b"observations",
            }
        )
        _, stdout, _ = self.run_job(observations_schema=schema)
        self.assert_contract_violation(stdout)

    def test_wrong_table_metadata_value_fails_closed(self):
        schema = h.OBSERVATIONS_SCHEMA.with_metadata(
            {
                b"opendemand.contract": b"opendemand.arrow/1",
                b"opendemand.table": b"observation",
            }
        )
        _, stdout, _ = self.run_job(observations_schema=schema)
        self.assert_contract_violation(stdout)

    def test_extra_metadata_key_is_tolerated(self):
        schema = h.OBSERVATIONS_SCHEMA.with_metadata(
            {
                b"opendemand.contract": b"opendemand.arrow/1",
                b"opendemand.table": b"observations",
                b"opendemand.created_by": b"test",
            }
        )
        code, stdout, _ = self.run_job(observations_schema=schema)
        self.assertEqual(code, 0)
        self.assertEqual(stdout.terminal()["type"], "job.result")

    def test_ipc_stream_format_fails_closed(self):
        h.write_ipc_stream(self.job_dir / "observations.arrow", h.OBSERVATIONS_SCHEMA, self.observations)
        h.write_rows(self.job_dir / "tasks.arrow", h.TASKS_SCHEMA, [self.task])
        code, stdout, _ = h.run_inproc(self.job_dir, [h.job_request()])
        self.assertEqual(code, 1)
        self.assert_contract_violation(stdout)

    def test_corrupted_magic_header_fails_closed(self):
        path = self.job_dir / "observations.arrow"
        h.write_rows(path, h.OBSERVATIONS_SCHEMA, self.observations)
        h.write_rows(self.job_dir / "tasks.arrow", h.TASKS_SCHEMA, [self.task])
        raw = bytearray(path.read_bytes())
        raw[0:6] = b"XARROW"
        path.write_bytes(bytes(raw))
        code, stdout, _ = h.run_inproc(self.job_dir, [h.job_request()])
        self.assertEqual(code, 1)
        self.assert_contract_violation(stdout)

    def test_tasks_schema_violation_fails_closed(self):
        schema = h.replace_field(
            h.TASKS_SCHEMA, "origin_index", pa.field("origin_index", pa.float64(), nullable=True)
        )
        _, stdout, _ = self.run_job(tasks_schema=schema)
        self.assert_contract_violation(stdout)


if __name__ == "__main__":
    unittest.main()
