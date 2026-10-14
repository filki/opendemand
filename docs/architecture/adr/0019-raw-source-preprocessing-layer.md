# ADR-0019: Raw-source preprocessing layer

**Status:** Accepted
**Date:** 2026-10-14
**Extends:** [ADR-0013](0013-canonical-import-and-mapping-idempotency.md)

## Context

Real planner exports (SAP, Subiekt, Comarch, Excel workbooks) do not match the `canonical/1` CSV, so the user had to hand-build the canonical file before any import. ADR-0013 anticipated "a later preprocessing layer" that converts arbitrary planner files into `canonical/1` without changing the engine contract; `AGENTS.md` deferred flexible raw-file parsing until it was separately approved. This is that approval.

## Decision Drivers

- Keep `canonical/1`, `parse_canonical`, and the forecast pipeline frozen.
- Rust owns validation and file handling (ADR-0002); the sidecar must stay untouched.
- Never invent business semantics: conversions and allocations are rule inventions, not mechanics.
- Explicit mappings only (ADR-0004); no fuzzy or name-based identity.
- Every committed revision must stay explainable from retained evidence.

## Decision

- Preprocessing is a Rust-core module (`preprocess.rs`); its output targets the frozen `canonical/1` observation shape. The sidecar and the forecast path are unchanged.
- One **parsing descriptor** (versioned, documented in `contracts/TAURI_IPC.md`) describes how one file's bytes map to observations: format, encoding, delimiter or sheet, header row, key columns and separator, date column and accepted formats, quantity column, decimal and thousands separators, and the aggregation rule.
- Supported sources: CSV/TXT (UTF-8 with or without BOM, UTF-16 with BOM, Windows-1250/1252, ISO-8859-2) and `.xlsx` workbooks. The approved readers are `encoding_rs` and `calamine`.
- Row semantics: document rows are summed into Monday–Sunday weeks keyed by the composed series key; the weekly net is classified `zero`/`return`/`known` by sign (ADR-0005). Weeks without rows stay **missing**; they are never zero-filled.
- Rows that cannot be interpreted (empty key part, key separator inside a part, unparsable date or number) are excluded and reported as capped `import_issue` rows with a summary; file-level problems are blocking and fail the job without a revision.
- The untouched source bytes are retained and hashed; preprocessed revisions carry `contract_version = 'preprocess/1'`.
- No unit conversion, no month-to-week allocation, no sign derivation from indicator columns, no product/location split (a composite key is one product identity), no name-based matching. The single active profile and the technical default location stand.

## Consequences

Positive:

- Real exports import without a hand-built CSV and without touching the engine.
- Raw bytes plus the frozen descriptor keep every revision explainable.
- Validation stays in one place: blocking findings, warnings, and issue codes are shared by the preview and the import job.

Negative:

- A composite key cannot later be split into product × location without a contract change.
- XLSX previews and imports load the whole worksheet into memory; very large workbooks are bounded by available memory.
- `.xls`, `.xlsb`, and `.ods` are not supported.

## Alternatives Considered

- **Preprocess in Python**: rejected; file ownership and validation belong to the Rust core (ADR-0002), and the sidecar must never read workspace files.
- **Convert files outside the application**: rejected; it keeps the burden that motivated this ADR.
- **Canonicalize bytes before hashing**: rejected again for the same reason as in ADR-0013: hashing stays over the original retained bytes.
