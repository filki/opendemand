## Summary

<!-- What does this change and why? -->

## Related issue

<!-- Fixes #123, or "none" -->

## Verification

<!-- Which checks did you run? Paste the commands from AGENTS.md § Verification and their results. -->

## Checklist

- [ ] The change advances Milestone 1 (canonical CSV → SQLite → Python Naive forecast → Rust persistence → Svelte chart); deferred features (MCP, XLSX, seasonal models, advanced workspace management) are left out.
- [ ] The approved stack is preserved: Tauri v2 / static SvelteKit, Rust as the sole SQLite owner, the Python sidecar never opens the database.
- [ ] `docs/architecture/schema/001_init.sql` is untouched while unfrozen; after the freeze gate, schema changes arrive as new migrations.
- [ ] Decision changes arrive as a new ADR that supersedes the old one.
- [ ] No secrets, credentials, or local absolute paths were added.
- [ ] Docs (`docs/`, `GLOSSARY.md`) are updated when behavior or vocabulary changed.