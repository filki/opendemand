"""JSONL sidecar protocol plumbing: thread-safe stdout writer and request reader.

Stdlib-only by design so that the hello envelope can be emitted before any
heavy import (PyArrow, StatsForecast) happens.

Stdin carries exactly one message, ``job.request``, and is read only on the
calling thread before any work starts. The engine never reads stdin again:
on Windows a thread parked in a blocking stdin read starves the rest of the
process while C extensions load or run (ADR-0016). Cancellation arrives as
the ``cancel.flag`` file in the job directory instead.
"""

from __future__ import annotations

import json
import threading


class EnvelopeWriter:
    """Serialises one UTF-8 JSON object per line to stdout under a lock."""

    def __init__(self, stream):
        self._stream = stream
        self._lock = threading.Lock()

    def send(self, envelope: dict) -> None:
        line = json.dumps(envelope, separators=(",", ":"), ensure_ascii=False) + "\n"
        with self._lock:
            self._stream.write(line)
            self._stream.flush()


def read_request(stream) -> tuple[str, dict | None]:
    """Read and classify the first stdin message on the calling thread.

    Returns ``("request", obj)`` for a ``job.request`` object, ``("error",
    obj_or_none)`` for malformed JSON, a non-object, or any other message
    type, and ``("eof", None)`` when stdin closes first. Blank lines are
    skipped. The protocol id is validated by the engine so the error can be
    correlated with the request's job id. Nothing after the first message is
    read.
    """
    while True:
        try:
            line = stream.readline()
        except Exception:
            line = ""
        if line == "":
            return "eof", None
        text = line.strip()
        if not text:
            continue
        try:
            obj = json.loads(text)
        except json.JSONDecodeError:
            return "error", None
        if not isinstance(obj, dict):
            return "error", None
        if obj.get("type") == "job.request":
            return "request", obj
        return "error", obj
