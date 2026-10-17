# Risk Register — OpenDemand

Scope: risks to the approved architecture and implemented workflows, with the mitigation and the stage where it must be implemented or verified.

- Impact and likelihood are qualitative: H = high, M = medium, L = low.
- Phase: Impl = core implementation (Rust core, import, database); Release = forecasting, packaging, and release preparation; Ongoing = continuous monitoring or verification.

| ID | Risk | Impact | Likelihood | Mitigation | Phase |
|----|------|--------|------------|------------|-------|
| R-01 | Sidecar crash or hang during a forecast job | H | M | Per-job process isolation; JSONL heartbeat and wall-clock timeout; kill the process, clean the job temp dir, allow one bounded retry; no partial database state because results are persisted in a single final transaction | Impl |
| R-02 | Arrow IPC schema drift between Rust and Python | H | M | Versioned contract metadata (`opendemand.contract`, `opendemand.table`); both sides validate schema before use; contract tests in CI; refuse mismatched contracts with a clear error | Impl |
| R-03 | SQLite writer contention (busy errors) | M | M | Single serialized writer in Rust; WAL mode; busy_timeout 5000; keep write transactions short and batch imports into one transaction | Impl |
| R-04 | Timezone or DST errors in weekly bucketing | H | M | IANA timezone snapshot per workspace; weeks are local Monday–Sunday; boundary tests around DST transitions; never bucket by UTC | Impl |
| R-05 | Partial, duplicate, or mis-encoded source data | H | H | Explicit observation statuses; import issues log; raw-bytes SHA-256 plus mapping-fingerprint idempotency; encoding detection with a hard failure and actionable message | Impl |
| R-06 | AutoETS fit minimum unknown in statsforecast | M | M | Verify the library's exact minimum during implementation; raise `f_seasonal` above 104 if required; record the verified value in ADR-0006 | Impl |
| R-07 | Backup failure before a migration | H | L | Verified backup gate; on failure open the workspace read-only with clear recovery instructions; retry path; never migrate without a verified backup | Impl |
| R-08 | Migration fails midway | M | L | Transactional DDL where possible; pre-migration backup as the restore path; documented rollback procedure | Impl |
| R-09 | Missing wheels for a target platform (macOS Intel, Linux glibc) | M | M | CI matrix for all supported platforms; macOS Intel conditional until CI proves compatible wheels; pin dependency versions | Release |
| R-10 | Forecast performance hypothesis unmet (10k SKUs ≤ 10 min) | M | M | Benchmark early with the synthetic 10k × 156 dataset; documented mitigations (fewer backtest windows or models, chunking, streaming); targets may be revised; progress and cancellation always available | Release |
| R-11 | Unbounded storage growth (immutable revisions, retained runs, backups) | M | H | Storage estimates in DATA_ARCHITECTURE; storage report in the UI; user-driven pruning of backup files only; aggregates-only mode may omit per-point backtests but always keeps per-series evaluation/origin rows; revisions and runs are never auto-deleted | Release |
| R-12 | MCP exposure or misuse | H | L | Off by default; opt-in per workspace; read-only allowlisted tools; no arbitrary SQL or paths; local IPC with a per-session token; workspace fixed at launch; pagination and size caps; audit log of MCP calls | Release |
| R-13 | Installer exceeds the 600 MB budget | M | M | PyInstaller excludes and onedir trimming; measure in CI; review dependency set before release | Release |
| R-14 | Wrong external code mappings silently mis-attribute data | H | M | Explicit mapping UX; never auto-match by name; import validation report lists unmapped and conflicting codes; mapping changes are visible in the import history | Impl |
| R-15 | Degenerate series break models (all-zero, near-constant, very short) | M | M | Qualification rules; explicit outcome statuses; fallback chain ending in Naive; never persist NaN values | Impl |
| R-16 | Write amplification from `synchronous=FULL` slows large imports | M | M | Single-transaction batched imports; measure import time against the 120 s target; durability is not traded away | Impl |
| R-17 | Canonical import contract drift (`canonical/1` semantics vs. parser) | H | M | `contract_version` stored per revision and qualified per sidecar request; parsers reject unknown versions; canonical fixtures in tests; the preprocessing layer targets the frozen contract without changing the engine | Impl |
| R-18 | Mapping fingerprint drift (same content resolving differently, or an inconsistent fingerprint computation) | M | M | Fingerprint computed deterministically from the effective mapping plan, including identity mappings created by the import; stored in `import_revision`; part of the idempotency key; covered by migration tests (ADR-0013) | Impl |
| R-19 | Orphan or stale retained files after a crash | M | M | File-before-row protocol with staged fsync and atomic rename; startup reconciliation detects staging leftovers and orphan finals, quarantines and reports them; never silent deletion (ADR-0015) | Impl |
| R-20 | Missing or hash-invalid retained file for a committed revision | H | M | Globally unique retained paths; startup reconciliation detects missing committed files and hash mismatches; backup enumerates retained files from its SQLite snapshot and fails closed (ADR-0015) | Impl |
| R-21 | Manual-only override misuse (out-of-horizon, non-sequential, unguarded clear) | M | M | SQLite triggers enforce completed run, scoped series, persisted horizon, first non-null revision, and sequential append-only revisions; Rust repeats the validations; UPDATE/DELETE rejected (ADR-0014) | Impl |
| R-22 | Orphan CSV/XLSX exports or hidden temporary files after a hard kill | M | L | Synced temporary files and no-overwrite publication prevent partial successful exports and replacement of existing data. Files publish before the export transaction commits; a crash can leave orphan files without an export record. XLSX constant-memory row files are unnamed and OS-managed. Export artifact reconciliation is not implemented yet; never delete retained export history automatically | Ongoing |

Notes:

- R-06 and R-10 are the two hypotheses that can change documented numbers (`f_seasonal`, performance targets); both must be re-verified and the documents updated if the outcome differs.
- Storage estimates in R-11 are first-order estimates with stated assumptions; they must be re-measured on a real workspace during implementation.
- R-17 through R-21 are covered by migration 001 artifacts (fingerprint fields, unique indexes, override triggers) and the architecture integration tests.
