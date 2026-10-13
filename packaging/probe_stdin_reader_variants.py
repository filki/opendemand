"""Identify which stdin read mechanism stalls sibling threads on Windows.

The release diagnosis showed the engine's main thread starving while a daemon
thread sat blocked in ``sys.stdin.readline()`` on a pipe: with stdin closed
right after the request the full job completes in about two seconds, with
stdin left open the process never reaches its first import. This driver
replays that pattern with tiny snippets over an idle stdin pipe, crossing
read mechanisms with kinds of main-thread work:

  readers: none (control), textio (``sys.stdin.readline()``), buffer
           (``sys.stdin.buffer.readline()``), raw (``os.read(0, ...)``)
  work:    sleep, ``import pyarrow``, a real Arrow IPC write and read back

The control without a reader measures the machine's cold pyarrow import; the
reader variants get an adaptive timeout derived from it. ``PROCEEDED`` means
the main thread ran while the reader stayed blocked; a ``ModuleNotFoundError``
traceback means the interpreter has no pyarrow (SKIP, not a stall); a timed
out, still-running child means that combination stalls the process. Always
exits 0; the verdict lines are the deliverable.
"""

from __future__ import annotations

import queue
import subprocess
import sys
import threading
import time

_SLEEP_TIMEOUT = 15.0
_CONTROL_TIMEOUT = 180.0

_READERS = {
    "textio": "sys.stdin.readline()",
    "buffer": "sys.stdin.buffer.readline()",
    "raw": "os.read(0, 4096)",
}

_WORKS = {
    "sleep": "import time; time.sleep(0.2); print('PROCEEDED', flush=True)",
    "import": "import pyarrow; print('PROCEEDED', flush=True)",
    "arrowwork": (
        "import os, tempfile, pyarrow as pa;"
        " table = pa.table({'a': [1.0, 2.0]});"
        " path = os.path.join(tempfile.gettempdir(), 'opendemand-variant.arrow');"
        " handle = open(path, 'wb');"
        " writer = pa.ipc.new_file(handle, table.schema);"
        " writer.write_table(table);"
        " writer.close(); handle.close();"
        " pa.ipc.open_file(path).read_all();"
        " print('PROCEEDED', flush=True)"
    ),
}

SNIPPETS: dict[str, str] = {}
for _work_name, _work in _WORKS.items():
    if _work_name != "sleep":
        SNIPPETS[f"none-{_work_name}"] = f"import os, sys; {_work}"
for _reader_name, _read_call in _READERS.items():
    for _work_name, _work in _WORKS.items():
        SNIPPETS[f"{_reader_name}-{_work_name}"] = (
            "import os, sys, threading;"
            f" threading.Thread(target=lambda: {_read_call}, daemon=True).start();"
            f" {_work}"
        )


def run_variant(name: str, snippet: str, timeout: float) -> str:
    started = time.monotonic()
    proc = subprocess.Popen(
        [sys.executable, "-c", snippet],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
    )
    lines: queue.Queue = queue.Queue()

    def pump() -> None:
        assert proc.stdout is not None
        for line in proc.stdout:
            lines.put(line)
        lines.put(None)

    threading.Thread(target=pump, daemon=True).start()
    captured: list[str] = []
    hung = False
    deadline = started + timeout
    while time.monotonic() < deadline:
        try:
            item = lines.get(timeout=max(0.5, deadline - time.monotonic()))
        except queue.Empty:
            hung = True
            break
        if item is None:
            break
        captured.append(item.rstrip())
        if "PROCEEDED" in item:
            break
    still_running = proc.poll() is None
    if still_running:
        proc.kill()
    try:
        proc.wait(timeout=10)
    except subprocess.TimeoutExpired:
        pass
    elapsed = time.monotonic() - started
    detail = " | ".join(captured) if captured else "(no output)"
    if any("PROCEEDED" in line for line in captured):
        return f"[{name}] OK    {elapsed:6.1f}s  main thread proceeded ({detail})"
    if any("ModuleNotFoundError" in line for line in captured):
        return f"[{name}] SKIP  {elapsed:6.1f}s  no pyarrow in this interpreter ({detail})"
    if hung and still_running:
        return f"[{name}] HANG  {elapsed:6.1f}s  stalls sibling threads ({detail})"
    return (
        f"[{name}] FAIL  {elapsed:6.1f}s  child exited with {proc.returncode}"
        f" without PROCEEDED ({detail})"
    )


def main() -> int:
    print("=== stdin reader variants ===")
    results = [
        run_variant(name, SNIPPETS[name], _SLEEP_TIMEOUT)
        for name in ("textio-sleep", "buffer-sleep", "raw-sleep")
    ]
    control = run_variant("none-import", SNIPPETS["none-import"], _CONTROL_TIMEOUT)
    results.append(control)
    parts = control.split()
    if parts[1] != "OK":
        print(f"(cold import control did not complete cleanly: {parts[1]}; skipping reader work variants)")
        for result in results:
            print(result)
        print("READER VARIANTS RESULT: baseline unusable — see the control above")
        return 0
    baseline = float(parts[2].rstrip("s"))
    adaptive = max(60.0, min(120.0, 3.0 * baseline + 30.0))
    print(f"(cold import baseline {baseline:.1f}s; adaptive timeout {adaptive:.0f}s)")
    for name in (
        "none-arrowwork",
        "textio-import",
        "textio-arrowwork",
        "buffer-import",
        "buffer-arrowwork",
        "raw-import",
        "raw-arrowwork",
    ):
        results.append(run_variant(name, SNIPPETS[name], adaptive))
    for result in results:
        print(result)
    stalled = [
        result.split()[0].strip("[]") for result in results if result.split()[1] == "HANG"
    ]
    if stalled:
        print(f"READER VARIANTS RESULT: STALLING — {', '.join(stalled)}")
    else:
        print("READER VARIANTS RESULT: every tested combination let the main thread proceed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
