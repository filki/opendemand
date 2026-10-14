# ADR-0021: Import configuration persistence and the pre-migration backup mechanism

**Status:** Accepted
**Date:** 2026-10-14
**Related:** [ADR-0008](0008-sqlite-durability-and-migration-gate.md), [ADR-0009](0009-backup-format-and-retention.md), [ADR-0019](0019-raw-source-preprocessing-layer.md)

## Context

Saved import configurations (ADR-0019) need durable storage, and migration 001 is frozen, so they arrive as migration 002. ADR-0008 requires a **verified backup** before any migration runs, but no backup mechanism existed: ADR-0009 names the SQLite Online Backup API, which the SQLx 0.9 driver does not expose, and SQLx rejects dynamic SQL strings for a `VACUUM INTO '<path>'` workaround.

## Decision Drivers

- No migration without a verified backup (ADR-0008); a failed backup must leave the workspace untouched.
- Frozen artifacts stay frozen: 001 is never edited; every change is an additive migration.
- Configurations must never rewrite history; a revision keeps the interpretation it was committed with.
- No new dependency for the backup mechanism.

## Decision

- **Migration 002** adds `import_config` (`profile_id`, unique `name`, `descriptor_json`, timestamps; no revisions reference a configuration row) and a nullable `import_revision.descriptor_json` that freezes the descriptor a preprocessed revision was committed with. Canonical imports keep `NULL`.
- **Saving** a configuration with an existing name updates that row in place; **deleting** one removes only the row. Neither touches any revision, because revisions carry their own frozen snapshot.
- **The migration gate**: `open` on an older schema creates a pre-migration backup first, with `VACUUM INTO ?` (a bound expression keeps the statement static). The snapshot is created under the single-writer invariant — no job runs while a workspace is opening — then verified: opened **read-only**, `PRAGMA integrity_check` must return `ok`, and the recorded `schema_version` inside the copy must equal the pre-upgrade version. A manifest (scope `database`, file name, SHA-256, byte size, versions before/after, app version, timestamp) is written next to it and the directory is fsynced. Only a verified pair unlocks the migration; otherwise `open` fails with `backup_failed` and the workspace stays at its old version.
- **Retention**: the newest three pre-migration pairs are kept; pruning covers backup files only.
- **Version bump**: the schema version is updated after the migrator runs. A crash between the two steps is self-healing: the next open re-runs the (now no-op) migration and bumps the version.
- **Verification**: schema verification checks the recorded checksum of every embedded migration, not only 001.

## Consequences

Positive:

- The ADR-0008 gate exists and is testable; a workspace cannot be migrated without a readable, intact, version-matching copy.
- A preprocessed revision remains explainable after its configuration is edited or deleted.

Negative:

- The pre-migration pair covers the database only, not the retained source copies; a schema migration does not modify source files, so restoring the database copy is sufficient for reversibility. Full ADR-0009 backups (database plus retained sources plus manifest) remain future work, together with periodic backups and a storage report.
- This mechanism deviates from ADR-0009's wording (Online Backup API); the deviation is recorded here and reviewed when full backups are implemented.

## Alternatives Considered

- **`sqlite3` CLI `.backup`**: already rejected in ADR-0009; shelling out contradicts the single-owner core.
- **Online Backup API through another SQLite dependency**: rejected; a new dependency is not justified for a snapshot taken while no writer exists.
- **Storing configurations as loose files**: rejected; outside the single-owner database (ADR-0002) and outside backup enumeration.
