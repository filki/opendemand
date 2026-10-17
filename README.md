# OpenDemand

Local-first, SKU-level demand forecasting for the desktop. One user, one local workspace: the SQLite database, retained sources, CSV/XLSX exports, and pre-migration backups live in a directory you choose, and no component of the application listens on the network.

> **Status: Milestone 6 (weekly planning workflow) is implemented in development.** The chain canonical CSV → SQLite → Python sidecar (Naive, SeasonalNaive, AutoETS) → Rust champion selection → Svelte chart runs locally through `npm run tauri dev`. Milestone 3 added per-series comparable-origin selection (ADR-0017), production fallback, model comparison in the series view, run-scoped append-only overrides, and full-run CSV export; all are covered by automated tests against a real Rust workspace, including an end-to-end test that runs the packaged sidecar. Milestone 4 makes the AutoETS fit path about 3.7x faster with a bounded in-sidecar worker pool (ADR-0018), with byte-identical forecast output and the same cancellation, timeout, and failure behaviour. The measured 100/1,000/5,000/10,000-series results are in `docs/benchmarks/2026-10-14-milestone-4-performance.md`; the 10,000-series case runs in about 24 minutes, so the 10-minute scale target is documented as unmet and remains a hypothesis. A guided import now converts raw SAP, Subiekt, Comarch, and Excel (`xlsx`) exports into the same canonical observations through a versioned parsing descriptor (ADR-0019, ADR-0020), with named configurations saved in migration 002 behind a verified pre-migration backup (ADR-0021). Milestone 6 adds the weekly planning workflow (ADR-0022): the wizard preselects the one saved configuration whose descriptor exactly matches the latest revision, a committed import forecasts the returned revision automatically, duplicate imports reuse that revision's newest completed run instead of forecasting again, and the comparison pairs the newest completed run with the immediately previous revision's run — SKU × location demand-cell changes and baseline forecast deltas aligned by calendar week, with horizon edges shown but never counted as changes. The comparison is derived at read time, needs no migration, and is covered by tests that run the real sidecar. The multi-worker engine path is run by CI on Linux and Windows (and by the macOS job) and was verified with the PyInstaller sidecar on macOS on 2026-10-14; the bundled multi-worker path on Windows and Linux stays unverified. `model_failure` series and aggregates-only storage are not exercised. Periodic backups, MCP, and XLSX export are not implemented yet. Nothing here is ready for production use.

> **Current workflow (ADR-0025/0026):** The one-click weekly flow in the milestone history above is historical. Import, on-demand forecast, and run comparison are separate actions; forecast generation asks whether to estimate missing weeks for that run only, without changing imported history. Weekly orchestration is deferred to Pipelines. XLSX export is implemented under ADR-0025; its save-dialog and manual Excel/LibreOffice checks remain unverified.

## Application

The approved planner improvements add a persistent run overview with coverage and exceptions, demand/forecast/presence filters for weekly comparison, workspace language selection for import and weekly review (English, Polish, Spanish), and native XLSX export with `Plan` and `Summary` sheets (ADR-0025). No migration or forecasting-policy change is required. Coverage is not forecast accuracy. Native XLSX save-dialog and Excel/LibreOffice application checks remain unverified; automated tests read the workbook back with `calamine`.

- **Desktop shell**: Tauri v2 with a static SvelteKit frontend (Svelte 5 runes, TanStack Query) and `tauri-specta` typed IPC.
- **Core**: Rust; sole owner of the workspace SQLite database (WAL, one serialized writer, one transaction per persist).
- **Forecasting**: Python sidecar, one job process per job with a bounded worker pool inside it (ADR-0018), Arrow IPC files and versioned JSONL; Naive, SeasonalNaive(52), and AutoETS(52), with Rust as the only selector.
- **Optional MCP adapter**: read-only, opt-in, deferred beyond the current milestone.

Target platforms are Windows x64, macOS Apple Silicon, and Linux x64.

## Milestone 1 input: the temporary `canonical/1` CSV

Milestone 1 accepts one deliberately simple CSV contract, versioned `canonical/1`, with exactly three columns:

| Column | Meaning |
|--------|---------|
| `series_key` | The series identifier; a new key creates its exact identity mapping (no fuzzy or name matching) |
| `week_start` | ISO Monday date of the week (`YYYY-MM-DD`) |
| `quantity` | Numeric weekly quantity |

It is temporary on purpose. Parsing arbitrary planner files stays out of the canonical importer itself; the guided import preprocessing layer converts real source files into `canonical/1` without changing the engine (ADR-0019). Later versions add richer semantics: missing weeks, multiple locations, and source profiles. Milestone 1 assigns imported series to a technical default location so the product/location architecture stays intact.

### Guided import: raw planner exports

The wizard behind **Guided import…** reads a CSV, TXT, or `.xlsx` export, proposes the encoding, delimiter, and header row (or the worksheet), and lets you map key, date, and quantity columns against a live parsed preview. Document rows are summed into Monday weeks, dates must match one of the accepted formats, and anything questionable is reported as blocking or warning findings before you press Import. Raw bytes are retained untouched, the same file imported the same way is a no-op, and any changed mapping field creates a new immutable revision. Named configurations can be saved and reused on next month's export; editing one never rewrites history. Details: `docs/architecture/contracts/TAURI_IPC.md`, ADR-0019/0020/0021.

### First forecast

Two synthetic files in [`examples/quickstart/`](examples/quickstart/) give a complete workflow without real data:

- [`first-import.csv`](examples/quickstart/first-import.csv) — three products (`SKU-A` 10/week, `SKU-B` 20/week, `SKU-C` 30/week), with 48 consecutive Monday weeks ending 2026-10-05.
- [`weekly-update.csv`](examples/quickstart/weekly-update.csv) — the same history plus the week 2026-10-12, with `SKU-B` at 25 in that final week, and one older `SKU-A` week corrected from 10 to 12.

The files carry the same three columns as `canonical/1`, so the guided import binds `series_key`, `week_start`, and `quantity` directly.

1. **Create a separate sample workspace** for these files, so your real workspace stays untouched.
2. Choose **Guided import…** and open `first-import.csv`. Confirm the mapping (`series_key`, `week_start`, `quantity`) and save it as a named configuration, for example "Quickstart sample".
3. Press **Import**. The revision is stored.
4. Press **Generate forecast** and choose whether to estimate missing weeks for this forecast. The imported history stays unchanged.
5. Inspect a series, then press **Export CSV**: 3 series × 12 horizon weeks = 36 data rows.

#### Compare a later import

1. Choose **Import CSV** and open `weekly-update.csv`. Use **Guided import** instead for raw planner exports that need column mapping.
2. After the import finishes, press **Generate forecast** separately. Choose whether to estimate missing weeks for this forecast.
3. Press **Compare runs** to inspect the latest import against the previous import. Expect three usable series. On the common horizon weeks the `SKU-B` baseline changes from 20 to 25. The corrected older `SKU-A` week shows as a demand change and does not become an override. The horizon-edge weeks (2026-10-12 in the previous run and 2027-01-04 in the new run only) are shown but count as no change.
4. Save one override and export again. The CSV carries the effective plan, with the override in the `override` and `effective` columns.

Forty-eight weeks is enough history for an evaluated Naive champion, so every series gets an `ok` outcome; it stays below the 104-week floor of SeasonalNaive and AutoETS, so this sample demonstrates the workflow, not a three-model contest. The fixed historical Mondays keep the walkthrough reproducible — the core uses the newest revision week as its cutoff, not today's date. The walkthrough uses CSV, but **Export XLSX** saves the same full-run plan as native date/number cells, with a Summary sheet that identifies the run and its coverage. Changing the workspace language updates the import and run-comparison copy; other screens and stored diagnostic text can remain English.

## Repository layout

- `docs/architecture/` — accepted decisions (`adr/`), data semantics, system structure, the frozen initial schema (`schema/001_init.sql` plus its SHA-256), and the implemented IPC/Arrow contracts (`contracts/`).
- `docs/agents/` — engineering-skill configuration: issue tracker, triage labels, domain docs.
- `src-tauri/` — Rust core and Tauri desktop shell (SQLite owner, job orchestration, selection, persistence).
- `src/` — SvelteKit frontend with generated typed IPC bindings.
- `examples/quickstart/` — the two synthetic files used by the **First forecast** walkthrough.
- `python/opendemand_engine/` — Python forecasting sidecar (one job process per job, Arrow IPC + JSONL, bounded worker pool).
- `tests/` — architecture/schema integration tests, engine tests, frontend tests, and fixtures.
- `GLOSSARY.md` — the project vocabulary.
- `.kilo/command/` — local agent commands (`/tdd`, `/review`, `/to-tickets`, `/caveman`, `/find-skills`).

## Development

Prerequisites: Node 22, stable Rust, and [uv](https://docs.astral.sh/uv/) (Python 3.12 is fetched automatically).

```bash
uv sync                # Python sidecar environment (.venv)
npm install            # frontend dependencies
npm run tauri dev      # run the desktop application
```

Verification:

```bash
cargo test --manifest-path src-tauri/Cargo.toml   # Rust core, typed IPC, real sidecar integration
uv run python -m unittest discover                # schema/recovery + Python engine suites
npm run check && npm test && npm run build        # frontend types, tests, static build
```

## Releases and packaging

The desktop bundle ships the Python sidecar as a standalone PyInstaller onedir directory, and the app resolves it from its own resources, so a packaged build runs without a Python installation.

Local packaging (macOS, Linux, or Windows):

```bash
npm run build:desktop  # freezes the Python sidecar, then bundles the app
```

Bundled resources are enabled only by `src-tauri/tauri.bundle.conf.json`, applied by this command and the Release workflow. Development and tests do not require generated packaging files.

Release automation runs only through **Actions > Release > Run workflow** (`workflow_dispatch`). It builds Windows x64, macOS Apple Silicon, and Linux x64 bundles and attaches them to a draft GitHub release. Pushing or publishing a tag does not start another build. Builds are currently unsigned; macOS shows a Gatekeeper warning until notarization lands.

CI gates packaging, every platform checks the bundled sidecar through Rust persistence, and successful runs align the draft with the built commit.

The static download catalog in `site/` is deployed to GitHub Pages (`https://filki.github.io/opendemand/`) by the Catalog workflow; it links only to published release assets.

## Contributing, security, license

- Contributions: see `CONTRIBUTING.md`.
- Security reports: see `SECURITY.md`.
- License: GNU Affero General Public License, version 3 only (`AGPL-3.0-only`) — see `LICENSE`. The change from MIT is not retroactive: code committed and builds published before the transition keep the MIT terms they were released under.
- Third-party assets: the IBM Plex fonts bundled in `site/assets/fonts/` remain under the SIL Open Font License — see `site/assets/fonts/OFL.txt`.

Copyright (C) 2026 OpenDemand contributors.

This program is free software: you can redistribute it and/or modify it under the terms of the GNU Affero General Public License, version 3, as published by the Free Software Foundation.

This program is distributed in the hope that it will be useful, but WITHOUT ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License along with this program. If not, see <https://www.gnu.org/licenses/>.
