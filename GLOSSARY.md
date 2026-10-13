# Glossary

Vocabulary for issues, tests, ADRs, docs, and UI copy. Definitions here are concise; the authoritative semantics live in `docs/architecture/DATA_ARCHITECTURE.md` and the ADRs under `docs/architecture/adr/`. Use these terms as written — do not drift to synonyms.

- **Backtest origin** — one rolling-origin training cutoff inside a forecast run; a run plans up to four (`origin_index` 1–4), each with its own origin date and train size.
- **Baseline forecast** — immutable model output for one product/location/week cell. Never edited; manual edits are overrides.
- **canonical/1** — the temporary Milestone 1 CSV input contract: `series_key`, `week_start`, `quantity`. A later preprocessing layer may target it; raw planner-file parsing is out of scope for now.
- **Champion** — the model selected per series per run for production forecasts after backtesting; selection rules are in ADR-0006.
- **Content hash** — SHA-256 of the original retained CSV bytes, qualified by the input contract version; part of the import idempotency key.
- **Coverage** — share of series in a run whose outcome is `ok` or `fallback_naive`.
- **Effective forecast** — derived at read time as the latest override ?? baseline. Never stored.
- **fallback_naive** — a run outcome: the series ran Naive because it was not eligible for champion selection (for example too little history).
- **Forecast run** — immutable record of one forecast execution; pins the import revision, horizon, timezone, config, and versions.
- **H (usable history)** — number of consecutive weeks with numeric values ending at the last completed week; `missing` and `partial` weeks break the run (ADR-0005).
- **Import revision** — immutable full-snapshot import; never mutated or auto-deleted.
- **Mapping fingerprint** — identity of the mapping plan an import used; part of the idempotency key together with the content hash.
- **Observation status** — one of `known`, `zero`, `missing`, `return`, `correction`, `stockout`, `partial`; `missing` and `partial` carry NULL quantity and break `H`.
- **Override** — append-only manual edit to a forecast cell; revisions are sequential, NULL clears, the highest revision wins, and overrides are scoped to one run.
- **Series** — one product × location pair; the unit of history, qualification, and forecast status.
- **Series qualification** — per-revision eligibility: `eligible`, `insufficient_history` (`H < 12`), or `invalid_data`.
- **SKU** — the product's canonical display code; product identity is the UUIDv7 id, not the SKU (ADR-0004).
- **week_start** — ISO date (`YYYY-MM-DD`) of the Monday that identifies a calendar week in the workspace timezone.
- **Workspace** — the user-chosen directory holding the SQLite database, retained CSV copies, backups with manifests, and job temp files.
