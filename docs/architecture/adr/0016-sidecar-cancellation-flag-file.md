# ADR-0016: Sidecar Cancellation Through a Job-Directory Flag File

**Status:** Accepted
**Date:** 2026-10-13
**Supersedes in part:** [ADR-0003](0003-python-sidecar-protocol.md) — only the `job.cancel` stdin message and the open stdin channel after `job.request`. Everything else in ADR-0003 still applies.

## Context

ADR-0003 sends `job.cancel` over stdin. To receive it, the engine kept a background thread blocked in a stdin read while the main thread imported PyArrow, NumPy, and StatsForecast and ran the forecast.

The first Windows release build hung after `hello` for the full 600-second gate. Diagnosis runs on `windows-latest` showed the following:

- PyInstaller, tokio, and pipe delivery are not the cause. The request arrives, and the imports alone take about 0.3–0.5 seconds.
- A thread blocked on stdin plus heavy C-extension work on the main thread hangs the process. This happens with every read mechanism tested: `TextIOWrapper.readline`, `sys.stdin.buffer.readline`, and raw `os.read`. The same thread with a sleeping main thread does not hang.
- Reading the request on the main thread and starting the reader after the imports only moved the hang to the first lazy NumPy/StatsForecast import in `backtest`.
- With stdin closed and no reader thread, the full job finishes in 1.5–8 seconds.

No change to the read layer avoids the hang. The engine must not block on stdin while it works.

## Decision Drivers

- The packaged engine must run on Windows, macOS, and Linux with identical behaviour.
- Cancellation must stay cooperative, with the existing 5-second grace and kill fallback.
- The rest of the protocol (hello, one request, progress, terminal envelopes, Arrow files) stays unchanged.

## Decision

- Stdin carries exactly one message, `job.request`. The engine reads it on the main thread before any heavy import and never reads stdin again.
- The core closes the engine's stdin immediately after it writes and flushes `job.request`.
- To cancel, the core creates the empty file `cancel.flag` in the job directory. The engine checks for that regular file at its existing interruption points: before reading inputs, between tasks, and before and after writing outputs. When the file is present, the engine removes partial outputs and sends `job.cancelled`.
- The core keeps the 5-second cancellation grace, then kills and reaps the process.
- `job.cancel` is removed from the message set. Any first stdin message other than `job.request` is a `protocol_mismatch`.
- The protocol id stays `opendemand.sidecar/1`. No release shipped the stdin cancel channel, and the engine and the core are always bundled together.

## Consequences

Positive:

- No thread blocks on stdin, so the Windows hang cannot occur.
- Cancellation uses the job directory, which the core already owns and cleans on every terminal path.
- Engine cancellation can be tested without a live stdin pipe.

Negative:

- The engine detects cancellation only at its interruption points, the same as before. A long single task still runs until the next point or the kill fallback.
- `cancel.flag` becomes a reserved name in the job directory.

## Alternatives Considered

- **Use another stdin read mechanism** (`buffer`, raw `os.read`): tested on Windows. All of them hang with C-extension work. Rejected.
- **Start the reader thread only after the imports**: moves the hang to the first lazy import inside the tasks. Rejected.
- **Use a local socket or named pipe for control**: adds a listener and platform-specific code. ADR-0003 rejects network surfaces. Rejected.
- **Kill the process without cooperative cancellation**: loses clean output removal and the `job.cancelled` acknowledgement. Kill remains only as the fallback after the grace period.
