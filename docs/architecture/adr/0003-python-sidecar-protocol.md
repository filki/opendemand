# ADR-0003: Python Sidecar — Per-Job Process, Arrow IPC, JSONL v1

**Status:** Accepted
**Date:** 2026-10-13
**Extended by:** [ADR-0012](0012-series-model-evaluation-and-selection.md) — Rust plans tasks; Python computes metrics and forecasts only.

## Context

Forecasting uses StatsForecast and Polars, which are Python libraries. The Rust core must invoke them without a network service, transfer large weekly series efficiently (up to ~1.56M observations at benchmark scale), isolate crashes, and stay predictable on a single-user desktop. Exactly one forecast job may run at a time.

## Decision Drivers

- No local HTTP server: no ports, no firewall prompts, no local attack surface.
- Efficient bulk transfer: Arrow IPC instead of JSON for data; JSON only for control.
- Crash isolation: a failed sidecar must not take down the app or corrupt the database.
- Determinism: a versioned, testable protocol between Rust and Python.
- Simplicity: one job at a time; no queueing daemon.

## Decision

- Run the sidecar as **one process per job** (spawned by the Rust core, terminated when the job ends).
- Transfer data as **Arrow IPC files** in a per-job temp directory managed by the core.
- Transfer control as **versioned JSON Lines** (protocol id `opendemand.sidecar/1`) over stdin/stdout: one JSON object per line, envelope `{"protocol","id","type","payload"}`.
- Message types: `hello` (sidecar announces engine and protocol versions), `job.request` (core: run id, input/output paths, models, backtest config, horizon, timezone), `job.progress` (sidecar: phase, series done/total), `job.result` (sidecar: output path, counts, durations), `job.error` (code, message, detail), `job.cancel` (core).
- Handshake rules: protocol version mismatch or unknown message type fails the job with a user-correctable error; the core never interprets sidecar output files before schema validation.
- Cancellation and timeouts: `job.cancel` first, then process termination after a grace period; a wall-clock timeout kills the process. The core cleans the job temp dir on every exit path.
- The sidecar never opens the SQLite database (ADR-0002) and never writes outside its job temp directory.

## Consequences

Positive:

- No network surface; nothing to firewall; works in restricted environments.
- Arrow IPC keeps large transfers fast and typed; JSON stays small and debuggable.
- Crash or hang in Python cannot corrupt the database; results are persisted only after validation, in a single transaction.
- The protocol is versioned, so Rust and Python releases cannot drift silently.

Negative:

- Process startup cost per job (seconds) and per-job memory overhead.
- Two-language maintenance: the protocol contract needs tests on both sides.
- One-job-at-a-time limits throughput (accepted for a single-user desktop).

## Alternatives Considered

- **Local HTTP or gRPC server**: adds ports, lifecycle management, and a local attack surface; rejected.
- **Long-lived worker process**: saves startup time but adds state, restart, and version-skew complexity; rejected for the MVP.
- **Rust-native forecasting** (no Python): avoids the sidecar entirely, but abandons StatsForecast, which is the chosen modeling stack.
