# ADR-0025: Native XLSX Plan Export

- **Status:** Accepted
- **Date:** 2026-10-15
- **Extends:** ADR-0007 (baseline, override, effective, and Summary coverage)

## Context

The approved planner improvements need an Excel handoff without manual CSV conversion. Rust already owns full-run CSV export, safe publication, and audit records. The frozen schema already permits `export_record.format = 'xlsx'`. GitHub #26 proposes one worksheet; ADR-0007 requires a Summary sheet as well.

## Decision Drivers

- Preserve saved-plan semantics, missing values, and no-overwrite publication.
- Keep filesystem and database operations in Rust.
- Reuse the current data query, export lifecycle, and coverage aggregate.
- Avoid retaining a large worksheet in memory or adding a Python export path.

## Decision

Use pinned `rust_xlsxwriter` 0.99.1 with `chrono` and `constant_memory` features. It has compatible MIT/Apache-2.0 licensing, uses the existing `zip` dependency family, and needs no system spreadsheet installation. Existing `calamine` reads output in regression tests.

Add `export_xlsx` beside `export_csv`. Both use one read-only snapshot, bounded keyset reads, synced temporary files, no-clobber publication, a retained workspace copy, and one export-record/job transaction. CSV bytes remain unchanged. No migration is added.

The workbook has two sheets, preserving ADR-0007 instead of adopting #26's one-sheet simplification:

- **Plan:** the same 12 columns and full-run horizon rows as CSV. Dates and numbers are typed; missing values are blank; names and identifiers are literal strings, never formulas. The worksheet uses constant-memory mode.
- **Summary:** static run/revision/horizon/engine identity, coverage, all outcome counts, and statistical champion counts. Coverage means the share of `ok` or `fallback_naive` series, not forecast accuracy. Statistical champions can differ from selected production fallback models.

The run overview reads the same coverage aggregate. Neither it nor weekly comparison filters add stored state. Worksheet-limit/write failures return `export_failed` and publish no successful export.

## Consequences

- One writer dependency and two concrete format paths share the existing durability lifecycle.
- Summary keeps the accepted decision and makes a detached workbook identifiable.
- Charts, formulas, styling, multi-run workbooks, progress, and cancellation remain outside this slice.
- Export retention and the hard-kill orphan risk R-22 apply to both formats; constant-memory mode uses unnamed OS-managed temp files, so it adds no new named artifacts.
- Automated workbook readback does not replace manual Excel/LibreOffice and native-dialog checks.

## Alternatives

- CSV only requires manual conversion and does not provide a native workbook handoff.
- A Python exporter breaks the chosen ownership boundary and adds another output pipeline.
- Hand-written OOXML/ZIP duplicates a mature file-format implementation.
- A Plan-only workbook removes a Summary requirement from an accepted ADR for a small saving; it is not selected.
