# ADR-0011: Packaging, Platform Targets, and Performance as Hypothesis

**Status:** Accepted
**Date:** 2026-10-13

## Context

OpenDemand ships a Rust core (Tauri application), a Python forecasting sidecar, and a FastMCP adapter. Distribution targets are Windows x64, macOS Apple Silicon, and Linux x64, with macOS Intel conditional. The performance numbers (10k SKUs ≤ 10 min forecast, import ≤ 120 s, and similar) were set as goals before any measurement and must not be treated as architectural guarantees.

## Decision Drivers

- Predictable installs without a user-managed Python environment.
- Platform coverage that matches the user base; no unsupported builds.
- Honest performance claims: hypotheses until measured.

## Decision

- Package the Python sidecar (and the MCP adapter) with **PyInstaller `onedir`** and Python 3.11 or newer. `onedir` is chosen over `onefile` for faster startup and debuggability.
- Ship targets: **Windows x64, macOS Apple Silicon, Linux x64**. macOS Intel is added only after CI proves compatible dependency wheels (risk R-09).
- **Performance targets are hypotheses, not guarantees** — especially "forecast 10k SKUs × 156 weeks ≤ 10 min at RSS ≤ 4 GB". Benchmark protocol:
  - a synthetic generator produces the 10k × 156-week dataset (1.56M observations);
  - wall-clock time and peak RSS are measured on the reference host (4 CPU, 8 GB RAM, SSD);
  - results are recorded under `docs/benchmarks/` with a pass/fail verdict per target;
  - targets may be revised based on results, with the change recorded.
- The architecture guarantees **progress reporting, cancellation, and bounded memory** regardless of target outcomes. If a target fails, the documented mitigations (fewer backtest windows or models, chunking, streaming I/O, bounded parallelism) must be applicable **without schema changes**.

## Consequences

Positive:

- Performance claims stay honest; benchmarks are repeatable and versioned.
- Platform support is CI-verified instead of aspirational.
- `onedir` simplifies diagnosing sidecar problems in the field.

Negative:

- `onedir` ships more files and a directory layout instead of a single binary.
- macOS Intel users may be unsupported until CI proves otherwise.
- The benchmark harness is additional implementation work (accepted; it also serves as a regression gate).

## Alternatives Considered

- **PyInstaller `onefile`**: single file, but slower cold start and harder debugging; rejected.
- **Nuitka**: viable, but a less familiar toolchain and less predictable wheel behavior across platforms; rejected for MVP.
- **Treating targets as guarantees**: dishonest and untestable; rejected.
