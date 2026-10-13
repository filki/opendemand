# ADR-0010: MCP Exposure — Opt-In, Read-Only, Delegated to the Rust Core

**Status:** Accepted
**Date:** 2026-10-13

## Context

MCP clients (for example Claude) may want to read planning data: runs, forecasts, series history, data quality. The Rust core is the sole database owner (ADR-0002). MCP must not become a second writer, an arbitrary-SQL surface, or a path-escape hatch, and it must be invisible unless the user opts in.

## Decision Drivers

- Preserve ADR-0002: no component other than the core touches SQLite.
- Read-only, allowlisted capabilities with no arbitrary SQL or paths.
- Opt-in per workspace with a clear off state.
- Local-only transport; no network listeners.

## Decision

- The MCP server is a **FastMCP adapter process** speaking MCP over stdio to the client. It is **disabled by default** and enabled per workspace in settings.
- The adapter **never opens SQLite**. It connects over a **local IPC channel** (Unix domain socket; Windows named pipe) to the running Rust core, authenticated by a **per-session token**.
- The core exposes a **versioned allowlist of read-only tools** (initial set: `get_workspace_info`, `list_runs`, `get_run_summary`, `search_products`, `get_series_history`, `get_forecast`, `get_data_quality_summary`). Every tool maps to predefined parameterized queries; the adapter cannot send SQL, file paths, or workspace switches.
- The workspace is **fixed when the adapter launches**; there is no workspace parameter on any tool.
- Results are **paginated and size-capped**; every call is written to an **audit log**.
- If the app is not running or MCP is disabled, tools return a clear "unavailable" error; the adapter never falls back to direct database access.

## Consequences

Positive:

- ADR-0002 holds: one database owner; MCP is a presentation layer over core queries.
- No network listener; the only surface is a local socket guarded by a session token.
- The tool set can evolve with versioned contracts without schema knowledge leaking to clients.

Negative:

- MCP requires the desktop app to be running; this must be documented for users.
- The adapter adds a second runtime (Python) next to the sidecar; packaging must account for it.
- Tool allowlists must be maintained as features grow.

## Alternatives Considered

- **Adapter opening the database read-only**: technically feasible with WAL, but violates the single-owner decision and spreads schema knowledge; rejected.
- **MCP server implemented in Rust inside the core**: fewer processes, but loses the FastMCP ecosystem and couples MCP release cadence to the core; possible future revision via a new ADR.
- **No MCP support**: simplest, but blocks a valuable read-only integration; rejected.
