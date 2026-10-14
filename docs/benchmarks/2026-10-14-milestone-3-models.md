# Benchmark: Milestone 3 multi-model forecasting

Date: 2026-10-14. Host: Apple M1 Pro (10 cores), 32 GB RAM, macOS 26.6, SSD.
Engine: repository `.venv` (Python 3.12, statsforecast 2.1.1, polars 2.0.0, pyarrow 26.0.0), one sidecar process, tasks executed sequentially.

Method: `uv run python tests/engine/benchmark_models.py 100 1000`. The script mimics the Rust planner exactly: three models, ADR-0006 floors, 12-week horizon, up to four origins; 156 weeks of synthetic 52-periodic history per series; the real engine entry point runs in-process over the Arrow job contract. Each series count runs in a fresh subprocess, so the peak RSS column covers the whole case process (interpreter, imports, and the job) and is not a cumulative high-water mark across cases.

| Series | Tasks | Completed | Failed | Production coverage | Points | Origin metric rows | Elapsed | Peak RSS (case process) |
|-------:|------:|----------:|-------:|--------------------:|-------:|-------------------:|--------:|---------------:|
| 100 | 1,500 | 1,500 | 0 | 1.000 | 18,000 | 1,200 | 55.8 s | 245.2 MB |
| 1,000 | 15,000 | 15,000 | 0 | 1.000 | 180,000 | 12,000 | 524.9 s | 425.7 MB |

Notes:

- Cost is linear in series count and dominated by AutoETS fits: the 1,000-series run spends most wall time in `AutoETS(season_length=52)` on roughly 150-week windows (5,000 ETS fits; Naive and SeasonalNaive are negligible).
- Memory stays bounded: the interpreter and imports alone account for about 226 MB (a 10-series case measures the same), and peak growth beyond that baseline is a few hundred MB at 1,000 series. Per-task arrays are freed between fits and no full observation copy grows with task count.
- The SYSTEM_ARCHITECTURE hypothesis target (10k SKUs x 156 weeks inside 10 minutes, RSS <= 4 GB) is not met by the sequential three-model path: extrapolating the measured ~0.53 s per series puts 10k series near 88 minutes. The target was flagged as a hypothesis, and the documented mitigations (bounded parallelism across a small pool of sidecar processes, fewer origins per model, or batched StatsForecast execution) were deliberately not built in this milestone to keep the change small. Memory is not the blocker; wall clock is.
- This benchmark does not weaken validation: every task was validated through the real engine pipeline before rows were counted.
