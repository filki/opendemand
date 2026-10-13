# Contributing to OpenDemand

Thanks for your interest. OpenDemand is pre-release: Milestone 1 (canonical CSV → SQLite → Python Naive forecast → Rust persistence → Svelte chart) is in development, so expect the layout and tooling to change quickly.

## Before you start

- Open or comment on a GitHub issue before large changes so effort is not duplicated. Issues are triaged with five labels — `needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix` — described in `docs/agents/triage-labels.md`.
- Small, focused pull requests are easier to review and merge. One concern per PR.

## Project rules every change must respect

- **Preserve the approved stack** (ADRs 0001–0011 in `docs/architecture/adr/`): Tauri v2 + static SvelteKit frontend, Rust as the sole SQLite owner, the Python sidecar as a separate per-job process that never opens the database.
- **Stay in Milestone 1.** MCP, XLSX export, seasonal models, flexible raw-file parsing, and advanced workspace management land only after the first end-to-end chain works.
- **Schema freeze gate.** `docs/architecture/schema/001_init.sql` is not frozen yet; while it is unfrozen, edit it only through the approved Phase 1 corrections. After the freeze, schema changes arrive as new migrations.
- **Change decisions with new ADRs.** Accepted ADRs are historical records; write a new ADR that supersedes the old one instead of editing it.
- **No secrets or local absolute paths** in code, docs, or configuration.

## Development setup

The build toolchain arrives with the Milestone 1 scaffold. Anticipated prerequisites: Rust stable, the Tauri v2 system dependencies for your platform, a current Node.js LTS, Python 3, and the SQLite CLI.

Test and check commands are listed in `AGENTS.md` § Verification and become concrete once the package manifests exist.

## Pull request checklist

The PR template mirrors this list:

- The change advances Milestone 1 and leaves deferred features out.
- The approved stack is preserved.
- Tests that exist pass locally.
- Schema and ADR rules above are honored.
- Docs (`docs/`, `GLOSSARY.md`) are updated when behavior or vocabulary changed.

## Security

Report vulnerabilities privately as described in `SECURITY.md`; do not open a public issue for them.

## License

By contributing, you agree that your contributions are licensed under the GNU Affero General Public License, version 3 only (`AGPL-3.0-only`) — see `LICENSE`. The change from MIT is not retroactive: contributions made before the transition remain under the terms they were published with.