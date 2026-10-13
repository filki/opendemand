"""Identify which stdin read mechanism stalls sibling threads on Windows.

The release diagnosis showed the engine's main thread starving while a daemon
thread sat blocked in ``sys.stdin.readline()`` on a pipe: with stdin closed
right after the request the full job completes in about two seconds, with
stdin left open the process never reaches its first import. This driver
replays that pattern with tiny snippets over an idle stdin pipe, crossing
three read mechanisms with three kinds of main-thread work:

  readers: textio (``sys.stdin.readline()``), buffer
           (``sys.stdin.buffer.readline()``), raw (``os.read(0, ...)``)
  work:    sleep, ``import pyarrow``, a real Arrow IPC write and read back

``PROCEEDED`` means the main thread ran while the reader stayed blocked;
a ``ModuleNotFoundError`` traceback means the interpreter has no pyarrow
(SKIP, not a stall); silence means that combination stalls the process.
Always exits 0; the verdict lines are the deliverable.
"""

from __future__ import annotations

import queue
import subprocess
import sys
import threading
import time

_TIMEOUT = 15.0

_READERS = {
    "textio": "sys.stdin.readline()",
    "buffer": "sys.stdin.buffer.readline()",
    "raw": "os.read(0, 4096)",
}

_WORKS = {
    "sleep": (
        "import time; time.sleep(0.2); print('PROCEEDED', flush=True)"
    ),
    "import": ("import pyarrow; print('PROCEEDED', flush=True)"),
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

SNIPPETS = {
    f"{reader_name}-{work_name}": (
        "import os, sys, threading;"
        f" threading.Thread(target=lambda: {read_call}, daemon=True).start();"
        f" {work}"
    )
    for reader_name, read_call in _READERS.items()
    for work_name, work in _WORKS.items()
}


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
    deadline = time.monotonic() + _TIMEOUT
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
    if proc.poll() is None:
        proc.kill()
    try:
        proc.wait(timeout=10)
    except subprocess.TimeoutExpired:
        pass

    detail = " | ".join(captured) if captured else "(no output)"
    if any("PROCEEDED" in line for line in captured):
        return f"[{name}] OK    main thread proceeded ({detail})"
    if any("ModuleNotFoundError" in line for line in captured):
        return f"[{name}] SKIP  no pyarrow in this interpreter ({detail})"
    if proc.poll() is not None:
        return f"[{name}] FAIL  child exited without PROCEEDED ({detail})"
    return f"[{name}] HANG  stalls sibling threads ({detail})"


def main() -> int:
    print("=== stdin reader variants ===")
    results = [run_variant(name, snippet) for name, snippet in SNIPPETS.items()]
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
