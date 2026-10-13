"""Identify which stdin read mechanism stalls sibling threads on Windows.

The release diagnosis showed the engine's main thread starving while a daemon
thread sat blocked in ``sys.stdin.readline()`` on a pipe: with stdin closed
right after the request the full job completes in about two seconds, with
stdin left open the process never reaches its first import. This driver
replays that pattern with tiny snippets, one read mechanism at a time, over
an idle stdin pipe:

  textio-sleep   sys.stdin.readline()         + main thread sleeps and prints
  textio-import  sys.stdin.readline()         + main thread imports pyarrow
  buffer-sleep   sys.stdin.buffer.readline()  + main thread sleeps and prints
  raw-sleep      os.read(0, 4096)             + main thread sleeps and prints

``PROCEEDED`` means the main thread ran while the reader stayed blocked;
silence means that mechanism stalls the whole process. Always exits 0; the
verdict lines are the deliverable.
"""

from __future__ import annotations

import queue
import subprocess
import sys
import threading
import time

SNIPPETS = {
    "textio-sleep": (
        "import sys, threading, time;"
        " threading.Thread(target=lambda: sys.stdin.readline(), daemon=True).start();"
        " time.sleep(0.2); print('PROCEEDED', flush=True)"
    ),
    "textio-import": (
        "import sys, threading;"
        " threading.Thread(target=lambda: sys.stdin.readline(), daemon=True).start();"
        " import pyarrow; print('PROCEEDED', flush=True)"
    ),
    "buffer-sleep": (
        "import sys, threading, time;"
        " threading.Thread(target=lambda: sys.stdin.buffer.readline(), daemon=True).start();"
        " time.sleep(0.2); print('PROCEEDED', flush=True)"
    ),
    "raw-sleep": (
        "import os, threading, time;"
        " threading.Thread(target=lambda: os.read(0, 4096), daemon=True).start();"
        " time.sleep(0.2); print('PROCEEDED', flush=True)"
    ),
}

TIMEOUT = 15.0


def run_variant(name: str, snippet: str) -> str:
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
    deadline = time.monotonic() + TIMEOUT
    captured: list[str] = []
    while time.monotonic() < deadline:
        try:
            item = lines.get(timeout=max(0.5, deadline - time.monotonic()))
        except queue.Empty:
            break
        if item is None:
            break
        captured.append(item.rstrip())
        if "PROCEEDED" in item:
            break
    hung = not any("PROCEEDED" in line for line in captured)
    if proc.poll() is None:
        proc.kill()
    try:
        proc.wait(timeout=10)
    except subprocess.TimeoutExpired:
        pass
    detail = " | ".join(captured) if captured else "(no output)"
    verdict = "HANG" if hung else "OK  "
    meaning = "stalls sibling threads" if hung else "main thread proceeded"
    return f"[{name}] {verdict} {meaning} ({detail})"


def main() -> int:
    print("=== stdin reader variants ===")
    results = [run_variant(name, snippet) for name, snippet in SNIPPETS.items()]
    for result in results:
        print(result)
    stalled = [result.split()[0].strip("[]") for result in results if result.split()[1] == "HANG"]
    if stalled:
        print(f"READER VARIANTS RESULT: STALLING — {', '.join(stalled)}")
    else:
        print("READER VARIANTS RESULT: every mechanism lets the main thread proceed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
