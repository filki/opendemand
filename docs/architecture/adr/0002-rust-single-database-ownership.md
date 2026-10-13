# ADR-0002: Rust as the Sole Owner of SQLite

**Status:** Accepted
**Date:** 2026-10-13
**Extended by:** [ADR-0012](0012-series-model-evaluation-and-selection.md) — selection policy and per-series evaluation persistence stay in the Rust core; [ADR-0015](0015-retained-source-file-durability.md) — file-before-row retained-file durability and startup reconciliation.

## Context

The workspace database is the single source of truth for imports, forecasts, overrides, and audit history. SQLite allows many readers but only one writer; careless multi-component access leads to busy errors, partial writes, and inconsistent validation. Three other components could touch the database: the SvelteKit frontend (via Tauri plugins), the Python sidecar (ADR-0003), and the MCP adapter (ADR-0010).

## Decision Drivers

- Data integrity: one component serializes writes and owns transactions.
- Centralized validation: import validation, mapping, qualification, and status logic must not be duplicated.
- Predictable failure modes: one place to handle busy timeouts, retries, and recovery.
- Auditability: every write goes through the same code path.

## Decision

The **Rust core** is the sole owner of the workspace SQLite database:

- Only Rust opens the database file (including read-only connections).
- All writes are serialized through a single writer task in Rust.
- The frontend receives data exclusively through typed commands; it never opens the database.
- The Python sidecar never opens the database; it exchanges Arrow IPC files with Rust (ADR-0003).
- The MCP adapter never opens the database; it calls allowlisted read-only tools on the core over local IPC (ADR-0010).
- Connection pragmas (WAL, `synchronous=FULL`, `foreign_keys=ON`, `busy_timeout=5000`) are applied by Rust at connection open (ADR-0008).

## Consequences

Positive:

- Single writer removes write contention by construction.
- Validation and invariants live in one place and are directly testable.
- Every future component can only reach data through an explicit, reviewable interface.

Negative:

- The Rust command surface must cover every UI and MCP need; it becomes a design bottleneck that must be shaped deliberately.
- Read-heavy screens need purpose-built queries rather than ad-hoc SQL from the UI.

## Alternatives Considered

- **tauri-plugin-sql / frontend SQL**: fastest to build, but splits validation, breaks single-writer discipline, and exposes SQL to the UI layer.
- **Multi-process access** (sidecar or MCP opening the database read-only): technically possible with WAL, but spreads schema knowledge and invites accidental writes; rejected to keep one owner.
- **Embedded server process**: unnecessary complexity for a single-user desktop application.
