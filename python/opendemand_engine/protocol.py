"""JSONL sidecar protocol plumbing: thread-safe stdout writer and stdin inbox.

Stdlib-only by design so that the hello envelope can be emitted before any
heavy import (PyArrow, StatsForecast) happens.
"""

from __future__ import annotations

import json
import threading

from . import PROTOCOL_ID


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


class InboxReader(threading.Thread):
    """Reads stdin lines, classifies them, and records cancel/protocol state.

    The first classified message is surfaced through :meth:`wait_initial` as
    ``("request", obj)``, ``("error", obj_or_none)`` or ``("eof", None)``.
    After a request has been seen the reader keeps consuming lines and:
    - sets ``cancel_event`` for a valid ``job.cancel`` of the current job,
    - records ``protocol_error`` for malformed or uncorrelated messages,
    - silently ignores stdin EOF (no more cancels can arrive).
    """

    def __init__(self, stream):
        super().__init__(name="opendemand-inbox", daemon=True)
        self._stream = stream
        self._first_ready = threading.Event()
        self._first_kind: str | None = None
        self._first_payload: dict | None = None
        self._request: dict | None = None
        self._pending_cancel_job_id: str | None = None
        self.cancel_event = threading.Event()
        self.protocol_error: str | None = None

    # -- public API --------------------------------------------------------

    @property
    def request(self) -> dict | None:
        return self._request

    def wait_initial(self) -> tuple[str, dict | None]:
        self._first_ready.wait()
        return self._first_kind or "eof", self._first_payload

    # -- thread body -------------------------------------------------------

    def run(self) -> None:  # pragma: no cover - thin loop over _handle
        while True:
            line = self._readline()
            if line == "":
                if self._first_kind is None:
                    self._finish_first("eof", None)
                return
            self._handle(line)

    def _readline(self) -> str:
        try:
            return self._stream.readline()
        except Exception:
            return ""

    def _handle(self, line: str) -> None:
        text = line.strip()
        if not text:
            return
        try:
            obj = json.loads(text)
        except json.JSONDecodeError:
            self._record_error("malformed JSON on stdin", None)
            return
        if not isinstance(obj, dict):
            self._record_error("stdin message is not a JSON object", None)
            return

        message_type = obj.get("type")
        if message_type == "job.request":
            # Protocol id is validated by the engine so the error can be
            # correlated with this request's job id.
            if self._request is not None or self._first_kind not in (None, "request"):
                self._record_error("duplicate job.request", None)
                return
            self._request = obj
            self._finish_first("request", obj)
            pending = self._pending_cancel_job_id
            if pending is not None and pending == self._usable_job_id(obj):
                self.cancel_event.set()
            return

        if obj.get("protocol") != PROTOCOL_ID:
            self._record_error("message protocol id mismatch", obj)
            return

        if message_type == "job.cancel":
            job_id = self._usable_job_id(obj)
            if self._request is None:
                if job_id is not None:
                    self._pending_cancel_job_id = job_id
                return
            if job_id is not None and job_id == self._usable_job_id(self._request):
                self.cancel_event.set()
            else:
                self._record_error("job.cancel job_id does not match the job", obj)
            return

        self._record_error("unsupported stdin message type", obj)

    # -- classification helpers ---------------------------------------------

    @staticmethod
    def _usable_job_id(obj: dict | None) -> str | None:
        if not isinstance(obj, dict):
            return None
        job_id = obj.get("job_id")
        if isinstance(job_id, str) and job_id:
            return job_id
        return None

    def _finish_first(self, kind: str, payload: dict | None) -> None:
        if self._first_kind is not None:
            return
        self._first_kind = kind
        self._first_payload = payload
        self._first_ready.set()

    def _record_error(self, message: str, obj: dict | None) -> None:
        if self.protocol_error is None:
            self.protocol_error = message
        if self._first_kind is None:
            self._finish_first("error", obj)
