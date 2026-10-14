# Security Policy

## Status

OpenDemand is pre-release and no version is supported for production use. See `README.md` for implemented functionality and verification gaps. Do not rely on any build for real business data yet.

## Reporting a vulnerability

Report privately through GitHub's private vulnerability reporting on `filki/opendemand` (Security → Advisories → "Report a vulnerability"). Please do not open a public issue for a security problem.

Include as much of this as you can:

- the affected component and the version or commit you ran;
- what you observed and what you expected;
- reproduction steps or a minimal proof of concept;
- the impact you think it has.

This is a volunteer project: there is no bug bounty, and acknowledgment or fixes happen on a best-effort basis.

## Design context for reports

OpenDemand is local-first by design. All business data stays on the user's machine, and no component opens a network listener. The optional MCP adapter is opt-in, read-only, token-guarded, and deferred beyond the current milestone.

Areas worth probing include: CSV import parsing (untrusted input), the sidecar protocol boundary, SQLite handling, override validation, CSV export paths and publication, and anything that could leak workspace data or execute code from crafted input.
