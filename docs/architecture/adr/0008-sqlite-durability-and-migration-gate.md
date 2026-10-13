# ADR-0008: SQLite Durability Settings and Migration Gate

**Status:** Accepted
**Date:** 2026-10-13
**Extended by:** [ADR-0015](0015-retained-source-file-durability.md) — retained files are made durable before the SQLite commit that references them.

## Context

The workspace database is the only copy of planning data. The application runs on a single-user desktop where power loss and forced termination are realistic; import jobs write up to ~1.56M rows, and schema migrations are the highest-risk operation because they can destroy data when they fail.

## Decision Drivers

- Durability over raw write speed: losing committed data is unacceptable.
- One writer, predictable locking behavior.
- Migrations must be recoverable; no migration without a verified backup.
- No half-migrated workspace: a failed migration must leave a usable state.

## Decision

- Per-connection pragmas applied by the Rust core at open time: `journal_mode=WAL`, `synchronous=FULL`, `foreign_keys=ON`, `busy_timeout=5000`.
- All writes are serialized through a single writer task (ADR-0002); long imports are one transaction, so WAL checkpoints happen after commit, not during parsing.
- SQLx migrations run **only after a verified backup** (ADR-0009). If the backup cannot be created or verified, the workspace opens **read-only** with a clear recovery message; no migration is attempted.
- A failed migration triggers the documented restore procedure from the pre-migration backup.
- After acceptance, `001_init.sql` is immutable; every schema change is a new additive numbered migration.

## Consequences

Positive:

- `synchronous=FULL` with WAL survives power loss of committed transactions; the cost is bounded extra fsyncs, acceptable at desktop write volumes.
- The backup gate makes destructive migrations effectively recoverable.
- Read-only mode is a safe, visible failure state instead of a silent one.

Negative:

- Import throughput is lower than with `synchronous=NORMAL`; the 120 s import target must be validated under FULL (risk R-16).
- Users can be blocked by a failed backup; messaging and a retry path are mandatory.
- Migrations carry operational weight (backup first, restore path documented).

## Alternatives Considered

- **`synchronous=NORMAL`**: faster, but committed transactions can be lost on power failure; rejected for a data-of-record database.
- **DELETE journal mode**: simpler files, slower and worse concurrency; rejected.
- **Auto-migrate without backup**: fastest developer experience, unacceptable data risk; rejected.
