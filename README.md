# OpenDemand

Local-first, SKU-level demand forecasting for the desktop. One user, one local workspace: the SQLite database, retained source and export CSV copies, and backups live in a directory you choose, and no component of the application listens on the network.

> **Status: Milestone 2 planner features are implemented in development.** The chain canonical CSV → SQLite → Python Naive forecast → Rust persistence → Svelte chart runs locally through `npm run tauri dev`. Run-scoped append-only overrides, baseline/effective comparison, and full-run CSV export are covered by automated tests and browser-driven checks against a real Rust workspace. The native save-dialog flow for CSV export was manually verified on macOS on 2026-10-14 (open, choose folder, save, cancel); Windows and Linux remain unverified. Packaging is experimental; backups, seasonal models, and MCP are not implemented yet. Nothing here is ready for production use.

## Application

- **Desktop shell**: Tauri v2 with a static SvelteKit frontend (Svelte 5 runes, TanStack Query) and `tauri-specta` typed IPC.
- **Core**: Rust; sole owner of the workspace SQLite database (WAL, one serialized writer, one transaction per persist).
- **Forecasting**: Python sidecar, one process per job, Arrow IPC files and versioned JSONL; Naive first, seasonal models later.
- **Optional MCP adapter**: read-only, opt-in, deferred beyond the current milestone.

Target platforms are Windows x64, macOS Apple Silicon, and Linux x64.

## Milestone 1 input: the temporary `canonical/1` CSV

Milestone 1 accepts one deliberately simple CSV contract, versioned `canonical/1`, with exactly three columns:

| Column | Meaning |
|--------|---------|
| `series_key` | The series identifier; a new key creates its exact identity mapping (no fuzzy or name matching) |
| `week_start` | ISO Monday date of the week (`YYYY-MM-DD`) |
| `quantity` | Numeric weekly quantity |

It is temporary on purpose. Parsing arbitrary planner files is out of scope for Milestone 1, so a later preprocessing layer can convert real source files into `canonical/1` (or a later version) without changing the engine. Later versions add richer semantics: missing weeks, multiple locations, and source profiles. Milestone 1 assigns imported series to a technical default location so the product/location architecture stays intact.

## Repository layout

- `docs/architecture/` — accepted decisions (`adr/`), data semantics, system structure, the frozen initial schema (`schema/001_init.sql` plus its SHA-256), and the implemented IPC/Arrow contracts (`contracts/`).
- `docs/agents/` — engineering-skill configuration: issue tracker, triage labels, domain docs.
- `src-tauri/` — Rust core and Tauri desktop shell (SQLite owner, job orchestration, selection, persistence).
- `src/` — SvelteKit frontend with generated typed IPC bindings.
- `python/opendemand_engine/` — Python forecasting sidecar (one process per job, Arrow IPC + JSONL).
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
