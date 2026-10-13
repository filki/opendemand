# ADR-0009: Backup Format and Retention

**Status:** Accepted
**Date:** 2026-10-13
**Extended by:** [ADR-0015](0015-retained-source-file-durability.md) — backup enumerates retained files from its SQLite snapshot and fails closed on missing or hash-invalid files.

## Context

A backup must be verifiable and restorable without the application, must capture the database and the retained source CSVs, and must not silently delete business data. At benchmark scale, the database grows ~0.2-0.3 GB per import revision, so backup retention has a real storage cost.

## Decision Drivers

- Verifiability: a backup that cannot be validated is not a backup.
- Hot backups: creation must work while the application is running (no manual close).
- Predictable storage: retention must be bounded for backup files.
- No automatic deletion of business history: import revisions and forecast runs stay.

## Decision

- A backup consists of:
  - a database copy created with the **SQLite Online Backup API** (safe while the database is open),
  - **copies of the source CSV files** retained for the covered revisions,
  - a **manifest** (JSON) with the file list, SHA-256 hashes, schema version, app version, and timestamp.
- Every backup is **verified after creation**: hashes recomputed and the database copy opened and integrity-checked.
- Retention default: keep the **3 most recent periodic backups** plus the **latest pre-migration backup**; pruning applies only to backup files and only as configured or as an explicit user action.
- Import revisions and forecast runs inside the database are **never auto-deleted** (ADR-0008); storage is managed through visibility, not silent deletion.
- The UI shows a storage report (database, CSV copies, backups, runs).

## Consequences

Positive:

- Restorable without the app: database copy plus manifest plus CSVs.
- Hot backup keeps UX simple; verification catches corruption early.
- Bounded backup storage (roughly <= 4x database size) while business history is preserved.

Negative:

- Backup creation at benchmark scale copies hundreds of MB; scheduling and progress reporting are needed.
- Retention defaults are opinionated; users must be able to see and adjust them.
- Duplication between CSV copies in backups and workspace-retained CSVs must be documented to avoid confusion.

## Alternatives Considered

- **Raw file copy while open**: unsafe; can capture a torn database; rejected.
- **`sqlite3` CLI `.backup`**: correct but adds an external process dependency; the Online Backup API is already available in the Rust driver.
- **No retention policy**: unbounded growth; rejected.
