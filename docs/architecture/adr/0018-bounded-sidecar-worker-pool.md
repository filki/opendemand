# ADR-0018: Bounded In-Sidecar Worker Pool for Model Fitting

**Status:** Accepted
**Date:** 2026-10-14
**Extends:** [ADR-0003](0003-python-sidecar-protocol.md) — the "one process per job" rule gains a bounded, short-lived worker pool *inside* the job process. [ADR-0016](0016-sidecar-cancellation-flag-file.md) cancellation semantics are unchanged. Relates to [ADR-0011](0011-packaging-and-platform-targets.md), which lists bounded parallelism as an approved mitigation.

## Context

The Milestone 3 benchmark measured the sidecar as sequential: 1,000 series took about 525 seconds and 10,000 series were extrapolated to roughly 88 minutes. Profiling on the reference workload shows why:

- 96% of engine wall time is AutoETS(season_length=52) fitting. Naive and SeasonalNaive are microseconds per task, input parsing and output writing are negligible.
- 90% of an AutoETS fit is spent in one compiled function, `statsforecast._lib.ets.optimize` (Nelder-Mead over the ETS candidate models).
- That compiled kernel does not release the CPython GIL: a four-thread fit measured 0.98x, no faster than sequential. Only separate processes can use more than one core.

The model set, floors, horizon, backtest geometry, metrics, and selection policy are fixed by ADR-0006, ADR-0012, and ADR-0017 and must not change. The only lever is executing the independent per-task fits concurrently.

## Decision Drivers

- Use the available cores without changing forecasting semantics or the Rust/Python ownership split.
- Keep the wire contract (one job, one Arrow input/output set, one JSONL conversation) unchanged.
- Keep cancellation, progress, timeouts, and failure handling working exactly as before.
- Stay runnable inside the PyInstaller `onedir` sidecar on Windows, macOS, and Linux.
- No new dependencies.

## Decision

- The sidecar stays **one process per job**. That job process may start a **bounded pool of worker processes** for the duration of the job.
- The pool uses the **spawn** start method on every platform. Spawn avoids forking the heartbeat thread and is the Windows and PyInstaller default, so the development and frozen paths behave the same.
- The worker count is bounded by `OPENDEMAND_ENGINE_WORKERS` (a positive integer) and defaults to `min(4, cpu_count)`. `1` disables the pool.
- The pool is used only when a job holds at least 24 tasks of models whose fit is expensive (currently `auto_ets_52`). Below that the fixed process start-up cost outweighs the parallel speed-up, so the job runs sequentially.
- Workers only fit and predict. They never open SQLite, never read or write the `cancel.flag`, and never write the output files. Each worker reads `observations.arrow` once and builds its own lookup index; the parent skips that index when it delegates to workers.
- Results are assigned back to tasks **by position**, so the pooled output is byte-identical to the sequential output and independent of worker count and completion order.
- Cancellation stays cooperative through `cancel.flag`: the job process checks it before every dispatch and every result, then shuts the pool down, waiting only for the tasks already in flight.
- The job timeout path in the Rust core now creates `cancel.flag` and applies the same five-second grace as user cancellation before it kills the job process, so the pool shuts down cleanly instead of being left behind.
- `multiprocessing.freeze_support()` runs first in the development and PyInstaller entry points so the frozen binary can re-launch itself as a worker.

## Consequences

Positive:

- On the reference host the pooled path is about 3.5x faster on the 100- and 1,000-series cases, with identical forecast output.
- Memory is bounded: the worker count is capped and the observation index is built per worker, not transferred over the pipe.
- No dependency, schema, protocol, or selection-policy change.

Negative:

- Each worker re-imports StatsForecast, costing about 230 MB of RSS and one to two seconds of start-up. The bound keeps four workers inside an 8 GB machine.
- The sidecar is no longer a single OS process while a job runs; the parent must own worker lifetime on every exit path.
- The PyInstaller bundle must call `freeze_support()`.

## Alternatives Considered

- **Threads**: measured at 0.98x on four threads because the compiled ETS kernel holds the GIL. Rejected.
- **A pool of sidecar processes driven by Rust**: each process would need its own task shard, its own Arrow inputs, and a merge of several output sets, changing validation, determinism, and cancellation. Rejected as more machinery for the same result.
- **joblib or loky**: adds a dependency for behaviour the standard library already provides. Rejected.
- **Batched StatsForecast panel fitting**: reduces Python overhead but not the per-series compiled fitting cost, and needs a DataFrame reshape of every task window. Rejected as the smaller win with more code.
