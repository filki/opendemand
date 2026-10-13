# Domain Docs

How the engineering skills should consume this repo's domain documentation when exploring the codebase.

## Layout: single-context

```
/
├── GLOSSARY.md
├── docs/architecture/adr/        ← accepted decisions (MADR-style, README index)
│   ├── README.md
│   └── 0001-desktop-application-stack.md …
└── docs/architecture/            ← DATA_ARCHITECTURE.md, SYSTEM_ARCHITECTURE.md, schema/
```

## Before exploring, read these

- **`GLOSSARY.md`** at the repo root — the project vocabulary.
- **`docs/architecture/adr/README.md`** — the ADR index; then read the ADRs that touch the area you're about to work in.
- **`docs/architecture/DATA_ARCHITECTURE.md`** for data semantics and **`docs/architecture/SYSTEM_ARCHITECTURE.md`** for component structure when the task touches them.

Accepted ADRs are historical: a changed decision is recorded in a new ADR that supersedes the old one, and the old record keeps its text.

## Use the glossary's vocabulary

When your output names a domain concept (in an issue title, a refactor proposal, a hypothesis, a test name), use the term as defined in `GLOSSARY.md`. Don't drift to synonyms the glossary explicitly avoids.

If the concept you need isn't in the glossary yet, that's a signal: either you're inventing language the project doesn't use (reconsider) or there's a real gap (note it for `/domain-modeling`).

## Flag ADR conflicts

If your output contradicts an existing ADR, surface it explicitly rather than silently overriding:

> _Contradicts ADR-0007 (forecast value representation), but worth reopening because…_