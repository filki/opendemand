"""Arrow IPC file schemas, strict readers, and atomic output writers.

The single source of truth for the wire contract is
``docs/architecture/contracts/ARROW_SCHEMAS.md``: IPC *file* format, exact
field order, types and nullability, and the ``opendemand.contract`` /
``opendemand.table`` metadata pair on every schema.
"""

from __future__ import annotations

import hashlib
import os
from pathlib import Path

import pyarrow as pa

from . import (
    ARROW_CONTRACT_ID,
    FORECAST_POINTS_FILE,
    OBSERVATIONS_FILE,
    ORIGIN_METRICS_FILE,
    TASK_STATUS_FILE,
    TASKS_FILE,
)
from .errors import CONTRACT_VIOLATION, INVALID_INPUT, EngineError

_IPC_MAGIC = b"ARROW1"

OUTPUT_FILES = {
    "task_status": TASK_STATUS_FILE,
    "origin_metrics": ORIGIN_METRICS_FILE,
    "forecast_points": FORECAST_POINTS_FILE,
}


def _table_metadata(table_name: str) -> dict[bytes, bytes]:
    return {
        b"opendemand.contract": ARROW_CONTRACT_ID.encode("utf-8"),
        b"opendemand.table": table_name.encode("utf-8"),
    }


OBSERVATIONS_SCHEMA = pa.schema(
    [
        pa.field("product_id", pa.utf8(), nullable=False),
        pa.field("location_id", pa.utf8(), nullable=False),
        pa.field("week_start", pa.date32(), nullable=False),
        pa.field("quantity", pa.float64(), nullable=True),
        pa.field("status", pa.utf8(), nullable=False),
    ],
    metadata=_table_metadata("observations"),
)

TASKS_SCHEMA = pa.schema(
    [
        pa.field("task_id", pa.utf8(), nullable=False),
        pa.field("product_id", pa.utf8(), nullable=False),
        pa.field("location_id", pa.utf8(), nullable=False),
        pa.field("model_id", pa.utf8(), nullable=False),
        pa.field("kind", pa.utf8(), nullable=False),
        pa.field("origin_index", pa.int32(), nullable=True),
        pa.field("train_start_week", pa.date32(), nullable=False),
        pa.field("train_end_week", pa.date32(), nullable=False),
        pa.field("forecast_start_week", pa.date32(), nullable=False),
        pa.field("train_size", pa.int32(), nullable=False),
        pa.field("horizon_weeks", pa.int32(), nullable=False),
    ],
    metadata=_table_metadata("tasks"),
)

TASK_STATUS_SCHEMA = pa.schema(
    [
        pa.field("task_id", pa.utf8(), nullable=False),
        pa.field("status", pa.utf8(), nullable=False),
        pa.field("failure_code", pa.utf8(), nullable=True),
        pa.field("failure_detail", pa.utf8(), nullable=True),
    ],
    metadata=_table_metadata("task_status"),
)

ORIGIN_METRICS_SCHEMA = pa.schema(
    [
        pa.field("task_id", pa.utf8(), nullable=False),
        pa.field("origin_week_start", pa.date32(), nullable=False),
        pa.field("train_size", pa.int32(), nullable=False),
        pa.field("wape", pa.float64(), nullable=True),
        pa.field("mae", pa.float64(), nullable=False),
        pa.field("metric_kind", pa.utf8(), nullable=False),
        pa.field("metric_value", pa.float64(), nullable=False),
    ],
    metadata=_table_metadata("origin_metrics"),
)

FORECAST_POINTS_SCHEMA = pa.schema(
    [
        pa.field("task_id", pa.utf8(), nullable=False),
        pa.field("week_start", pa.date32(), nullable=False),
        pa.field("value", pa.float64(), nullable=False),
    ],
    metadata=_table_metadata("forecast_points"),
)

OUTPUT_SCHEMAS = {
    "task_status": TASK_STATUS_SCHEMA,
    "origin_metrics": ORIGIN_METRICS_SCHEMA,
    "forecast_points": FORECAST_POINTS_SCHEMA,
}


# ---------------------------------------------------------------------------
# Reading inputs
# ---------------------------------------------------------------------------


def _require_safe_input_file(job_dir: Path, name: str) -> Path:
    path = job_dir / name
    if path.is_symlink():
        raise EngineError(INVALID_INPUT, f"input file must not be a symlink: {name}")
    if not path.is_file():
        raise EngineError(INVALID_INPUT, f"input file is missing or not a regular file: {name}")
    try:
        inside = path.resolve().parent == job_dir.resolve()
    except OSError:
        inside = False
    if not inside:
        raise EngineError(INVALID_INPUT, f"input file escapes the job directory: {name}")
    return path


def _require_ipc_file_format(path: Path, name: str) -> None:
    try:
        with open(path, "rb") as handle:
            head = handle.read(len(_IPC_MAGIC))
            handle.seek(-len(_IPC_MAGIC), os.SEEK_END)
            tail = handle.read(len(_IPC_MAGIC))
    except OSError as exc:
        raise EngineError(CONTRACT_VIOLATION, f"input file cannot be read: {name}") from exc
    if head != _IPC_MAGIC or tail != _IPC_MAGIC:
        raise EngineError(
            CONTRACT_VIOLATION, f"input file is not an Arrow IPC file: {name}"
        )


def read_input_table(job_dir: Path, name: str, schema: pa.Schema, table_name: str) -> pa.Table:
    """Strictly validate and read one input Arrow IPC file."""
    path = _require_safe_input_file(job_dir, name)
    _require_ipc_file_format(path, name)
    try:
        reader = pa.ipc.open_file(str(path))
    except Exception as exc:
        raise EngineError(
            CONTRACT_VIOLATION, f"input file is not a readable Arrow IPC file: {name}"
        ) from exc

    actual_schema = reader.schema
    if not actual_schema.equals(schema, check_metadata=False):
        raise EngineError(CONTRACT_VIOLATION, f"input schema mismatch: {name}")
    actual_metadata = actual_schema.metadata or {}
    for key, value in _table_metadata(table_name).items():
        if actual_metadata.get(key) != value:
            raise EngineError(CONTRACT_VIOLATION, f"input schema metadata mismatch: {name}")

    try:
        table = reader.read_all()
        table.validate(full=True)
    except Exception as exc:
        raise EngineError(CONTRACT_VIOLATION, f"input table failed validation: {name}") from exc
    return table


# ---------------------------------------------------------------------------
# Writing outputs
# ---------------------------------------------------------------------------


def _sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _write_ipc_file(path: Path, schema: pa.Schema, table: pa.Table) -> None:
    """Write an IPC file, flush and fsync it, and close it before returning."""
    handle = open(path, "wb")
    try:
        with pa.ipc.new_file(handle, schema) as writer:
            writer.write_table(table)
        handle.flush()
        os.fsync(handle.fileno())
    finally:
        handle.close()


def write_output_tables(
    job_dir: Path, rows_by_table: dict[str, list[dict]]
) -> tuple[dict[str, dict], list[Path]]:
    """Write the three fixed output files via temp names and atomic renames.

    Returns ``(manifest, written_paths)`` where the manifest maps the fixed
    keys to ``{"path", "rows", "sha256"}``. Temp files are removed on failure.
    """
    manifest: dict[str, dict] = {}
    written: list[Path] = []
    pending: list[Path] = []
    try:
        for key, filename in OUTPUT_FILES.items():
            schema = OUTPUT_SCHEMAS[key]
            table = pa.Table.from_pylist(rows_by_table[key], schema=schema)
            final_path = job_dir / filename
            temp_path = job_dir / f".{filename}.{os.getpid()}.tmp"
            pending.append(temp_path)
            _write_ipc_file(temp_path, schema, table)
            os.replace(temp_path, final_path)
            pending.remove(temp_path)
            written.append(final_path)
            manifest[key] = {
                "path": filename,
                "rows": table.num_rows,
                "sha256": _sha256_file(final_path),
            }
    except Exception:
        for temp_path in pending:
            try:
                temp_path.unlink()
            except OSError:
                pass
        raise
    return manifest, written


def remove_output_files(paths: list[Path]) -> None:
    for path in paths:
        try:
            path.unlink()
        except OSError:
            pass
