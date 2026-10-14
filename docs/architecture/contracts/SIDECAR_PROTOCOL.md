# Sidecar Protocol

Protocol id: `opendemand.sidecar/1`. One UTF-8 JSON object per stdout/stdin line.
Stdin carries exactly one message, `job.request` (ADR-0016).
Every object includes `protocol`, `type`, and `job_id` (null only in the initial hello).
Stdout is protocol-only. Logs go to stderr. No HTTP listener or database access.
Launch `python -m opendemand_engine --job-dir <directory>` in development, or the
packaged `opendemand-engine --job-dir <directory>` binary in a release build.
The desktop app resolves `OPENDEMAND_ENGINE` first, then the bundled
`engine/opendemand-engine` resource, then the repository `.venv` fallback.

Process model (ADR-0018): the engine runs as one job process. For a job holding
enough AutoETS tasks it starts a bounded pool of worker processes inside that
job; each worker only fits and predicts, and the job process owns their lifetime,
the output files, and the `cancel.flag` checks. `OPENDEMAND_ENGINE_WORKERS` sets
the worker count (default `min(4, cpu_count)`; `1` disables the pool).

## Exchange

1. Python sends `hello`: engine version, contract versions, supported models.
2. Rust sends exactly one `job.request`: job id and fixed input filenames, then closes stdin.
   Python reads it on the main thread and never reads stdin again.
3. Python sends `job.progress`: phase, completed task count, total task count.
4. Python sends one terminal `job.result`, `job.error`, or `job.cancelled`.
5. Python exits. Rust validates output before the single result transaction.

```json
{"protocol":"opendemand.sidecar/1","type":"hello","job_id":null,"engine_version":"0.5.0","contracts":["opendemand.arrow/1"],"models":["naive","seasonal_naive_52","auto_ets_52"]}
{"protocol":"opendemand.sidecar/1","type":"job.request","job_id":"42","contract":"opendemand.arrow/1","input_files":{"observations":"observations.arrow","tasks":"tasks.arrow"}}
{"protocol":"opendemand.sidecar/1","type":"job.progress","job_id":"42","phase":"backtest","completed":1,"total":5}
```

`job.result.output_files` maps `task_status`, `origin_metrics`, and `forecast_points`
to objects containing `path`, `rows`, and lowercase hexadecimal `sha256`.
Paths must equal the fixed filenames from the Arrow contract. No arbitrary paths,
symlinks, parent traversal, or files outside the job directory are accepted.

`job.error` contains `code`, `message`, and `retryable`.
Error codes: `protocol_mismatch`, `contract_violation`, `invalid_input`, `fit_error`,
`predict_error`, `engine_start_failed`, `unexpected_eof`, `timeout`, `cancelled`,
and `internal_error`.
Task-specific prediction failures are output rows, not a whole-job error.

## Lifecycle

- Hello timeout: 5 seconds.
- Whole-job timeout: configurable, 30 minutes by default.
- Progress heartbeat: no more than 30 seconds apart.
- Progress phases: `validate_input`, `backtest`, `production`, `write_output`.
- To cancel, Rust creates the empty file `cancel.flag` in the job directory. Python checks for it
  before reading inputs, between tasks, and around output writing, removes partial outputs, and
  answers `job.cancelled`. Any first stdin message other than `job.request` is `protocol_mismatch`.
- Cancellation grace: 5 seconds, then Rust kills and reaps the process.
- At most one whole-job retry for `engine_start_failed` or `unexpected_eof`, before accepting output.
- No automatic retry for invalid input, contract violations, timeout, or cancellation.
- Temporary directories are cleaned on every terminal path. Retained source files are never temporary engine files.
- Outputs are written to temporary names, closed, then renamed before `job.result`.

Rust owns the task plan, production eligibility, champion ranking, fallback, and persistence.
Python fits the task's planned model through StatsForecast and never selects one. Supported
models: `naive` (Naive), `seasonal_naive_52` (SeasonalNaive, `season_length=52`), and
`auto_ets_52` (AutoETS, `season_length=52`). Training floors (ADR-0006): Naive 12 weeks;
SeasonalNaive and AutoETS 104 weeks. Python computes WAPE/MAE for completed backtests.
