"""Walk the packaged sidecar's stdio boundaries one probe at a time.

Release run 38449569286 showed the frozen Windows sidecar going silent right
after ``hello sent``; the orchestrator hit its 600 s deadline with no phase
line. The engine logs nothing between hello and the first phase, so this
driver replays the same pipe conversation with hard timeouts and reports
exactly which boundary answers:

  hello     the engine starts and emits hello over stdout
  request   a malformed request is answered with job.error/protocol_mismatch
            (stdin delivery plus request parsing; runs before any heavy import)
  imports   a valid request against an empty job directory is answered with
            job.error/invalid_input (the deferred pyarrow/tasks imports ran)
  forecast  a valid request with real demo Arrow inputs returns job.result

Run it against the frozen binary and, for comparison, the source interpreter:

  uv run python packaging/diagnose_sidecar_stdio.py --engine <binary path>
  uv run python packaging/diagnose_sidecar_stdio.py --engine-module

The script always exits 0; the printed verdict is the deliverable.
"""

from __future__ import annotations

import argparse
import json
import os
import queue
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path
from typing import Callable

REPO_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(REPO_ROOT))

from tests.engine import helpers as h  # noqa: E402

PYTHON_DIR = REPO_ROOT / "python"
HELLO_TIMEOUT = 15.0
REQUEST_TIMEOUT = 45.0
IMPORTS_TIMEOUT = 120.0
FORECAST_TIMEOUT = 240.0


class ProbeTimeout(Exception):
    """No envelope arrived within the probe deadline."""


class EngineDriver:
    """Spawn an engine command over pipes; mirror helpers.EngineProcess."""

    def __init__(self, command: list[str], job_dir: Path, env_extra: dict[str, str]):
        env = dict(os.environ)
        env.update(env_extra)
        self.proc = subprocess.Popen(
            command,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            bufsize=1,
            cwd=str(job_dir),
            env=env,
        )
        self._stdout_queue: queue.Queue = queue.Queue()
        self._stderr_lines: list[str] = []
        self._threads = [
            threading.Thread(target=self._pump_stdout, daemon=True),
            threading.Thread(target=self._pump_stderr, daemon=True),
        ]
        for thread in self._threads:
            thread.start()

    def _pump_stdout(self) -> None:
        assert self.proc.stdout is not None
        for line in self.proc.stdout:
            self._stdout_queue.put(line)
        self._stdout_queue.put(None)

    def _pump_stderr(self) -> None:
        assert self.proc.stderr is not None
        for line in self.proc.stderr:
            self._stderr_lines.append(line)

    def send(self, line: str) -> None:
        assert self.proc.stdin is not None
        self.proc.stdin.write(line + "\n")
        self.proc.stdin.flush()

    def close_stdin(self) -> None:
        if self.proc.stdin is not None and not self.proc.stdin.closed:
            self.proc.stdin.close()

    def read_envelope(self, timeout: float) -> dict | None:
        try:
            item = self._stdout_queue.get(timeout=timeout)
        except queue.Empty as exc:
            raise ProbeTimeout() from exc
        if item is None:
            return None
        return json.loads(item)

    def stderr_text(self) -> str:
        return "".join(self._stderr_lines)

    def stop(self) -> None:
        if self.proc.poll() is None:
            self.proc.kill()
        try:
            self.proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            pass
        for thread in self._threads:
            thread.join(timeout=2.0)


def _command(mode: str, engine_path: str, job_dir: Path) -> tuple[list[str], dict[str, str]]:
    env_extra = {"PYTHONUNBUFFERED": "1", "OPENBLAS_NUM_THREADS": "1"}
    if mode == "binary":
        return [engine_path, "--job-dir", str(job_dir)], env_extra
    existing = os.environ.get("PYTHONPATH")
    env_extra["PYTHONPATH"] = (
        str(PYTHON_DIR) if not existing else str(PYTHON_DIR) + os.pathsep + existing
    )
    return [sys.executable, "-m", "opendemand_engine", "--job-dir", str(job_dir)], env_extra


def _dump(driver: EngineDriver) -> str:
    text = driver.stderr_text().strip()
    if not text:
        return "    engine stderr: (empty)"
    return "\n".join(f"    engine stderr| {line}" for line in text.splitlines())


def _ok(name: str, started: float, envelope: dict, driver: EngineDriver) -> str:
    driver.stop()
    elapsed = time.monotonic() - started
    code = f"/{envelope['code']}" if envelope.get("code") else ""
    return f"[{name}] OK    {elapsed:6.1f}s  answered {envelope['type']}{code}\n{_dump(driver)}"


def _hang(name: str, started: float, expect: tuple[str, str | None], timeout: float, driver: EngineDriver) -> str:
    driver.stop()
    elapsed = time.monotonic() - started
    return (
        f"[{name}] HANG  {elapsed:6.1f}s  no {expect[0]}"
        f"{'/' + expect[1] if expect[1] else ''} within {timeout:.0f}s — this boundary stalled\n{_dump(driver)}"
    )


def _fail(name: str, started: float, reason: str, driver: EngineDriver) -> str:
    driver.stop()
    elapsed = time.monotonic() - started
    return f"[{name}] FAIL  {elapsed:6.1f}s  {reason}\n{_dump(driver)}"


def probe(
    name: str,
    mode: str,
    engine_path: str,
    setup: Callable[[Path], None],
    send_lines: list[str],
    expect: tuple[str, str | None],
    timeout: float,
    close_stdin: bool,
) -> str:
    started = time.monotonic()
    with tempfile.TemporaryDirectory(prefix=f"opendemand-probe-{name}-") as tmp:
        job_dir = Path(tmp)
        setup(job_dir)
        command, env_extra = _command(mode, engine_path, job_dir)
        driver = EngineDriver(command, job_dir, env_extra)
        try:
            hello = driver.read_envelope(HELLO_TIMEOUT)
            if hello is None or hello.get("type") != "hello":
                return _fail(name, started, f"expected hello, got {hello!r}", driver)
            for line in send_lines:
                driver.send(line)
            if close_stdin:
                driver.close_stdin()
            deadline = time.monotonic() + timeout
            while True:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    return _hang(name, started, expect, timeout, driver)
                envelope = driver.read_envelope(remaining)
                if envelope is None:
                    return _fail(name, started, f"engine exited before {expect[0]}", driver)
                if envelope.get("type") == expect[0] and (
                    expect[1] is None or envelope.get("code") == expect[1]
                ):
                    return _ok(name, started, envelope, driver)
        except ProbeTimeout:
            return _hang(name, started, expect, timeout, driver)
        except Exception as error:  # the point is to report the boundary, not crash
            return _fail(name, started, repr(error), driver)


def main() -> int:
    parser = argparse.ArgumentParser(description="Probe sidecar stdio boundaries.")
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument("--engine", help="path to the packaged sidecar binary")
    group.add_argument("--engine-module", action="store_true", help="drive the source interpreter")
    arguments = parser.parse_args()

    mode = "binary" if arguments.engine else "module"
    engine_path = str(Path(arguments.engine).resolve()) if arguments.engine else ""

    series = h.load_demo_series()
    tasks = h.plan_demo_tasks(series)

    def empty(job_dir: Path) -> None:
        return None

    def demo_inputs(job_dir: Path) -> None:
        h.write_inputs(job_dir, h.demo_observation_rows(series), tasks)

    print(f"=== sidecar stdio diagnosis ({mode}) ===")
    print(f"engine: {engine_path or 'python -m opendemand_engine'}")
    results = [
        probe(
            "request",
            mode,
            engine_path,
            empty,
            [h.job_request("probe-a", protocol="opendemand.sidecar/WRONG")],
            ("job.error", "protocol_mismatch"),
            REQUEST_TIMEOUT,
            True,
        ),
        probe(
            "imports",
            mode,
            engine_path,
            empty,
            [h.job_request("probe-c")],
            ("job.error", "invalid_input"),
            IMPORTS_TIMEOUT,
            True,
        ),
        probe(
            "forecast",
            mode,
            engine_path,
            demo_inputs,
            [h.job_request("probe-d")],
            ("job.result", None),
            FORECAST_TIMEOUT,
            False,
        ),
    ]
    for result in results:
        print(result)
        print()
    verdicts: dict[str, str] = {}
    for result in results:
        head = result.splitlines()[0].split()
        verdicts[head[0].strip("[]")] = head[1]
    hung = [name for name, verdict in verdicts.items() if verdict == "HANG"]
    if hung:
        print(f"DIAGNOSIS RESULT [{mode}]: STALLED at {', '.join(hung)} — see the boundaries above")
    else:
        print(f"DIAGNOSIS RESULT [{mode}]: every boundary answered — the pipe path is healthy")
    return 0


if __name__ == "__main__":
    sys.exit(main())
