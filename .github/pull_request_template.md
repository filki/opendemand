## Summary

<!-- What does this change and why? -->

## Related issue

<!-- Fixes #123, or "none" -->

## Verification

<!-- Which checks did you run? Paste the commands from AGENTS.md § Verification and their results. -->

## Checklist

- [ ] The change stays within the current milestone scope in `AGENTS.md`; deferred features (MCP, XLSX, seasonal models, flexible raw-file parsing, advanced workspace management) are left out.
- [ ] The approved stack is preserved: Tauri v2 / static SvelteKit, Rust as the sole SQLite owner, the Python sidecar never opens the database.
- [ ] Frozen migration `docs/architecture/schema/001_init.sql` and its fingerprint are untouched; schema changes arrive as new numbered migrations with a verified backup.
- [ ] Decision changes arrive as a new ADR that supersedes the old one.
- [ ] No secrets, credentials, or local absolute paths were added.
- [ ] Docs (`docs/`, `GLOSSARY.md`) are updated when behavior or vocabulary changed.
