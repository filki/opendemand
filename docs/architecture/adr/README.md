# Architecture Decision Records

This directory records the accepted architecture decisions for OpenDemand. Each record follows a concise MADR-style structure (Status, Date, Context, Decision Drivers, Decision, Consequences, Alternatives).

## Index

| ADR | Title | Status | Date |
|-----|-------|--------|------|
| [0001](0001-desktop-application-stack.md) | Desktop application stack: Tauri v2 with static SvelteKit | Accepted | 2026-10-13 |
| [0002](0002-rust-single-database-ownership.md) | Rust as the sole owner of SQLite | Accepted | 2026-10-13 |
| [0003](0003-python-sidecar-protocol.md) | Python sidecar: per-job process, Arrow IPC, JSONL v1 | Accepted | 2026-10-13 |
| [0004](0004-identity-and-external-mappings.md) | Identity and external code mappings | Accepted | 2026-10-13 |
| [0005](0005-missing-data-semantics.md) | Missing-data semantics and observation statuses | Accepted | 2026-10-13 |
| [0006](0006-forecasting-models-and-backtesting.md) | Forecasting models, eligibility mathematics, and backtesting | Accepted | 2026-10-13 |
| [0007](0007-forecast-value-representation.md) | Forecast value representation: baseline, override, effective | Accepted | 2026-10-13 |
| [0008](0008-sqlite-durability-and-migration-gate.md) | SQLite durability settings and migration gate | Accepted | 2026-10-13 |
| [0009](0009-backup-format-and-retention.md) | Backup format and retention | Accepted | 2026-10-13 |
| [0010](0010-mcp-readonly-exposure.md) | MCP exposure: opt-in, read-only, delegated to the Rust core | Accepted | 2026-10-13 |
| [0011](0011-packaging-and-platform-targets.md) | Packaging, platform targets, and performance as hypothesis | Accepted | 2026-10-13 |
| [0012](0012-series-model-evaluation-and-selection.md) | Per-series model evaluation and selection | Accepted | 2026-10-13 |
| [0013](0013-canonical-import-and-mapping-idempotency.md) | Canonical import and mapping idempotency | Accepted | 2026-10-13 |
| [0014](0014-manual-only-overrides.md) | Manual-only forecast overrides | Accepted | 2026-10-13 |
| [0015](0015-retained-source-file-durability.md) | Retained source-file durability and crash recovery | Accepted | 2026-10-13 |
| [0016](0016-sidecar-cancellation-flag-file.md) | Sidecar cancellation through a job-directory flag file | Accepted | 2026-10-13 |

Schema status: migration 001 is frozen (2026-10-13; SHA-256 recorded in `../schema/001_init.sha256`). All schema changes now use new numbered migrations.

## Rules

- Accepted ADRs are not edited; a changed decision gets a new ADR that supersedes the old one (status changes to Superseded).
- Extending ADRs add an "Extended by" pointer to the new record; the original decision text is not rewritten.
- Numbers that are hypotheses (AutoETS fit minimum, performance targets) are re-verified during implementation; if a verified number differs, a new ADR records the change.
- One decision per record; keep records short and specific.
