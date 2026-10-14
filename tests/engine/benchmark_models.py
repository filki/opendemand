"""Lightweight benchmark for the multi-model engine pipeline.

Usage::

    uv run python tests/engine/benchmark_models.py [series_counts ...]

Defaults to 100 series; pass ``100 1000`` for the documented run. The script
mimics the Rust planner (three models, ADR-0006 floors, 12-week horizon, at most
four origins) and reports elapsed wall clock, the case process peak RSS,
completed and failed model tasks, production coverage, and output row counts.
Each series count runs in a fresh subprocess so peak RSS is not a cumulative
high-water mark across cases. The module name does not match ``test*.py``, so
unittest discovery ignores it.
"""

from __future__ import annotations

import json
import math
import os
import shutil
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))

from tests.engine import helpers as h  # noqa: E402

try:
    import resource
except ImportError:  # pragma: no cover - Windows has no resource module
    resource = None

MODELS = [("naive", 12), ("seasonal_naive_52", 104), ("auto_ets_52", 104)]
HORIZON = 12
MAX_ORIGINS = 4
WEEKS = 156


class TreeRssSampler(threading.Thread):
    """Sample the summed RSS of this process and its descendants.

    ``resource.getrusage(RUSAGE_SELF)`` only covers the case process, so worker
    processes spawned by the engine pool would be invisible. This sampler walks
    ``ps`` output to include them. Summed RSS double-counts shared pages (a
    deliberate upper bound) and is unavailable on Windows.
    """

    def __init__(self, interval: float = 0.2):
        super().__init__(name="rss-sampler", daemon=True)
        self._interval = interval
        self._stop_event = threading.Event()
        self.peak_kb = 0
        self.available = resource is not None and shutil.which("ps") is not None

    def run(self) -> None:  # pragma: no cover - timing thread
        if not self.available:
            return
        root = os.getpid()
        while not self._stop_event.wait(self._interval):
            try:
                output = subprocess.run(
                    ["ps", "-axo", "pid=,ppid=,rss="],
                    capture_output=True,
                    text=True,
                    timeout=5,
                ).stdout
            except Exception:
                continue
            children: dict[int, list[int]] = {}
            rss_by_pid: dict[int, int] = {}
            for line in output.splitlines():
                parts = line.split()
                if len(parts) < 3:
                    continue
                try:
                    pid, ppid, rss = int(parts[0]), int(parts[1]), int(parts[2])
                except ValueError:
                    continue
                children.setdefault(ppid, []).append(pid)
                rss_by_pid[pid] = rss
            total = rss_by_pid.get(root, 0)
            stack = list(children.get(root, []))
            while stack:
                pid = stack.pop()
                total += rss_by_pid.get(pid, 0)
                stack.extend(children.get(pid, []))
            self.peak_kb = max(self.peak_kb, total)

    def stop(self) -> None:
        self._stop_event.set()
        self.join(timeout=5.0)


def plan(pid: str, lid: str, start_week, h_len: int) -> list[dict]:
    tasks: list[dict] = []
    weeks = h.week_grid(start_week, h_len)
    for model_id, floor in MODELS:
        if h_len < floor:
            continue
        origin_count = max(0, min(MAX_ORIGINS, (h_len - floor) // HORIZON))
        tasks.append(
            h.production_task(
                h.series_uuid(f"{pid}/{model_id}/prod"),
                pid,
                lid,
                weeks,
                model_id=model_id,
            )
        )
        for index in range(1, origin_count + 1):
            train_size = h_len - HORIZON * (origin_count - index + 1)
            tasks.append(
                h.backtest_task(
                    h.series_uuid(f"{pid}/{model_id}/b{index}"),
                    pid,
                    lid,
                    weeks,
                    index,
                    train_size,
                    model_id=model_id,
                )
            )
    return tasks


def run_case(series_count: int) -> dict:
    start = h.monday("2023-03-06")
    observations: list[dict] = []
    tasks: list[dict] = []
    for index in range(series_count):
        pid, lid = h.series_uuid(f"bench-p{index}"), h.series_uuid(f"bench-l{index}")
        values = [
            100 + 40 * math.sin(2 * math.pi * (week % 52) / 52) + (week % 7)
            for week in range(WEEKS)
        ]
        observations += h.weekly_rows(pid, lid, start, values)
        tasks += plan(pid, lid, start, WEEKS)

    with tempfile.TemporaryDirectory() as temporary:
        job_dir = Path(temporary)
        h.write_inputs(job_dir, observations, tasks)
        sampler = TreeRssSampler()
        sampler.start()
        started = time.perf_counter()
        code, stdout, _ = h.run_inproc(job_dir, [h.job_request()])
        elapsed = time.perf_counter() - started
        sampler.stop()
        peak_after = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss if resource else 0
        status_rows = h.read_table(job_dir / "task_status.arrow").to_pylist()
        metrics_rows = h.read_table(job_dir / "origin_metrics.arrow").num_rows
        point_rows = h.read_table(job_dir / "forecast_points.arrow").num_rows

    kinds = {task["task_id"]: (task["kind"], task["model_id"]) for task in tasks}
    completed = sum(1 for row in status_rows if row["status"] == "completed")
    failed = len(status_rows) - completed
    production_total = sum(1 for task in tasks if task["kind"] == "production")
    production_completed = sum(
        1
        for row in status_rows
        if row["status"] == "completed" and kinds[row["task_id"]][0] == "production"
    )
    failed_by_model: dict[str, int] = {}
    for row in status_rows:
        if row["status"] != "completed":
            model_id = kinds[row["task_id"]][1]
            failed_by_model[model_id] = failed_by_model.get(model_id, 0) + 1
    # ru_maxrss is a lifetime high-water mark; each case runs in a fresh
    # subprocess, so this value covers the whole case process (interpreter,
    # imports, and the job). Bytes on macOS, kibibytes on Linux.
    divisor = 1024 * 1024 if sys.platform == "darwin" else 1024
    peak_mb = round(peak_after / divisor, 1) if resource else None
    tree_peak_mb = round(sampler.peak_kb / 1024, 1) if sampler.available else None

    from opendemand_engine import engine

    return {
        "series": series_count,
        "windows": WEEKS,
        "tasks": len(tasks),
        "exit_code": code,
        "terminal": stdout.terminal()["type"],
        "completed": completed,
        "failed": failed,
        "failed_by_model": failed_by_model,
        "metrics_rows": metrics_rows,
        "point_rows": point_rows,
        "production_coverage": production_completed / production_total if production_total else 0.0,
        "elapsed_s": round(elapsed, 2),
        "peak_rss_mb": peak_mb,
        "tree_peak_rss_mb": tree_peak_mb,
        "workers": engine.worker_count(),
    }


def main(argv: list[str]) -> int:
    if argv and argv[0] == "--case":
        # Internal mode: run one case in this process and emit a JSON report.
        print(json.dumps(run_case(int(argv[1]))))
        return 0
    counts = [int(value) for value in argv if not value.startswith("--")] or [100]
    for count in counts:
        result = subprocess.run(
            [sys.executable, str(Path(__file__).resolve()), "--case", str(count)],
            capture_output=True,
            text=True,
            check=True,
        )
        report = json.loads(result.stdout.strip().splitlines()[-1])
        print(
            "series={series} tasks={tasks} workers={workers} completed={completed} "
            "failed={failed} coverage={coverage:.3f} points={point_rows} metrics={metrics_rows} "
            "elapsed={elapsed}s peak_rss={peak}MB tree_rss={tree}MB terminal={terminal}".format(
                coverage=report["production_coverage"],
                elapsed=report["elapsed_s"],
                peak=report["peak_rss_mb"],
                tree=report["tree_peak_rss_mb"],
                **report,
            )
        )
        if report["failed_by_model"]:
            print(f"  failed by model: {report['failed_by_model']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
