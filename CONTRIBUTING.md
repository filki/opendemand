# Contributing to OpenDemand

OpenDemand is pre-release. The import/forecast chain, the Milestone 2 planner, raw-source preprocessing, and the Milestone 6 weekly workflow are implemented in development; see `README.md` for current functionality and verification gaps.

## Before you start

- Open or comment on a GitHub issue before large changes so effort is not duplicated. Issues are triaged with five labels — `needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix` — described in `docs/agents/triage-labels.md`.
- Small, focused pull requests are easier to review and merge. One concern per PR.

## Trying the development preview

Early testers are welcome; starter tasks are labeled `good first issue` and `help wanted`.

The published installers are **not code-signed**. Windows SmartScreen can warn about an unknown publisher, and macOS Gatekeeper blocks the first launch until you allow it (right-click the app, then Open, or use System Settings → Privacy & Security). Those warnings are expected in this preview — they are not a sign of a corrupt download. Get installers from the [Releases](https://github.com/filki/opendemand/releases) page and report findings with the bug report template.

## Project rules every change must respect

- **Preserve the approved stack** (accepted ADRs in `docs/architecture/adr/`): Tauri v2 + static SvelteKit frontend, Rust as the sole SQLite owner, the Python sidecar as a separate per-job process that never opens the database.
- **Stay within the current milestone.** The scope in `AGENTS.md` covers the weekly planning workflow, run comparison, run-scoped overrides, and full-run CSV export. MCP, XLSX export, scheduled imports, periodic backups and restore, and the true monthly forecasting mode remain deferred until separately approved.
- **Schema freeze.** `docs/architecture/schema/001_init.sql` is frozen; its SHA-256 is recorded in `001_init.sha256`. Never edit it. Schema changes arrive as new numbered migrations with a verified backup.
- **Change decisions with new ADRs.** Accepted ADRs are historical records; write a new ADR that supersedes the old one instead of editing it.
- **No secrets or local absolute paths** in code, docs, or configuration.

## Development setup

Prerequisites: stable Rust, the Tauri v2 system dependencies for your platform, Node 22, and `uv`. Python 3.12 is managed by `uv`.

```bash
uv sync
npm ci
npm run tauri dev
```

Run the checks in `AGENTS.md` before submitting a PR. Major dependency upgrades are reviewed and tested individually against the current application; passing checks on an older base are not sufficient.

## Pull request checklist

The PR template mirrors this list:

- The change stays within the current milestone scope in `AGENTS.md` and leaves deferred features out.
- The approved stack is preserved.
- Tests that exist pass locally.
- Schema and ADR rules above are honored.
- Docs (`docs/`, `GLOSSARY.md`) are updated when behavior or vocabulary changed.

## Security

Report vulnerabilities privately as described in `SECURITY.md`; do not open a public issue for them.

## License

By contributing, you agree that your contributions are licensed under the GNU Affero General Public License, version 3 only (`AGPL-3.0-only`) — see `LICENSE`. The change from MIT is not retroactive: contributions made before the transition remain under the terms they were published with.
