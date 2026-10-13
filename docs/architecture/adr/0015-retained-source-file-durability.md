# ADR-0015: Retained Source-File Durability and Crash Recovery

**Status:** Accepted
**Date:** 2026-10-13

## Context

Import revisions reference retained source CSV copies, and ADR-0009 backups copy those files. A crash between writing a file and committing SQLite could previously leave a committed revision referencing a file that was never made durable, or leave unexplained leftovers. The review required a recoverable file-before-row protocol with explicit startup reconciliation.

## Decision Drivers

- No committed revision may reference a file that is not durable.
- Crashes may leave orphans; recovery must report or quarantine them, never silently delete business data.
- Backup must fail closed when a retained file is missing or hash-invalid.

## Decision

File-before-row import protocol:

1. Parse/hash the canonical input and build the complete mapping plan.
2. Check idempotency before retaining another file.
3. Copy to a workspace staging file while computing SHA-256; flush and fsync it.
4. Begin the serialized SQLite transaction, allocate the revision id, and insert the proposed rows.
5. Atomically rename the staged file into `sources/<revision_id>/<safe_filename>` (canonical imports: `sources/<revision_id>/source.csv`) and fsync the relevant directories.
6. Insert the globally unique relative path in `source_file`, then commit SQLite.
7. A crash before commit may leave an orphan file but cannot leave a committed revision referencing a file that was never made durable.

Startup reconciliation (run on workspace open) detects stale staging files, orphan final files, missing committed files, and hash mismatches. Recoverable artifacts move to a recovery/quarantine area and are reported as recovery issues; nothing is silently deleted. `source_file.relative_path` is globally unique. Backup enumerates retained files from its SQLite snapshot and fails closed when a file is missing or hash-invalid.

## Consequences

Positive:

- The durability ordering makes "committed implies durable" true for retained files.
- Crashes are recoverable and visible instead of silent.
- Backup verification covers the retained-file set as well as the database.

Negative:

- Extra fsyncs on the import path (bounded; one staged file per revision).
- Reconciliation runs at startup and needs its own tests (staging stop, rename stop, commit stop, hash mismatch).

## Alternatives Considered

- **Commit first, copy later**: can commit a revision whose file never lands; rejected.
- **Deleting orphan files automatically**: destroys potentially recoverable business data; rejected.
- **Content-addressed file names**: deduplicates identical bytes but complicates revision-scoped retention and backup enumeration; deferred, not needed for MVP.
