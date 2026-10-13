# Milestone 1 Sidecar Protocol

Protocol id: `opendemand.sidecar/1`. One UTF-8 JSON object per stdout/stdin line.
Every object includes `protocol`, `type`, and `job_id` (null only in the initial hello).
Stdout is protocol-only. Logs go to stderr. No HTTP listener or database access.
Launch `python -m opendemand_engine --job-dir <directory>` with a dedicated temporary directory.

## Exchange

1. Python sends `hello`: engine version, contract versions, supported models.
2. Rust sends exactly one `job.request`: job id and fixed input filenames.
3. Python sends `job.progress`: phase, completed task count, total task count.
4. Python sends one terminal `job.result`, `job.error`, or `job.cancelled`.
5. Python exits. Rust validates output before the single result transaction.

```json
{"protocol":"opendemand.sidecar/1","type":"hello","job_id":null,"engine_version":"0.1.0","contracts":["opendemand.arrow/1"],"models":["naive"]}
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
- Rust sends `job.cancel` with the current job id; Python acknowledges with `job.cancelled`.
- Cancellation grace: 5 seconds, then Rust kills and reaps the process.
- At most one whole-job retry for `engine_start_failed` or `unexpected_eof`, before accepting output.
- No automatic retry for invalid input, contract violations, timeout, or cancellation.
- Temporary directories are cleaned on every terminal path. Retained source files are never temporary engine files.
- Outputs are written to temporary names, closed, then renamed before `job.result`.

Rust owns the task plan, production eligibility, champion ranking, fallback, and persistence.
Python uses StatsForecast Naive for each planned task, repeats the last training value
for 12 weeks, and computes WAPE/MAE for completed backtests.
Seasonal models are not implemented in Milestone 1.
