# ADR-0004: Identity and External Code Mappings

**Status:** Accepted
**Date:** 2026-10-13
**Extended by:** [ADR-0013](0013-canonical-import-and-mapping-idempotency.md) — exact identity mappings for `canonical/1`, fixed technical default location, and the effective mapping fingerprint.

## Context

Source files identify products and locations by external codes. Codes change between sources and over time, can be re-used, and names are neither unique nor stable. Internal identity must survive re-imports, source switches, and renames, and every attribution must be explainable.

## Decision Drivers

- Stable identity across imports, renames, and source changes.
- No silent mis-attribution: matching must be explicit and reviewable.
- Portability: identifiers must work inside a portable workspace directory.
- Time correctness: weekly bucketing must be deterministic in the workspace timezone.

## Decision

- Internal identifiers for products and locations are **UUIDv7**, stored as `BLOB(16)`; they are generated on first mapping and never reused.
- External codes map to internal ids through **explicit mapping tables**, scoped per source profile (`product_code_map`, `location_code_map`). Name-based or fuzzy matching is never used.
- Unmapped codes are recorded as **import issues** (warning) and their rows are excluded from observations; the import report surfaces them for resolution.
- A week is **Monday through Sunday**, identified by `week_start` (the ISO date of the Monday in workspace time). All source dates are interpreted in the workspace's confirmed **IANA timezone**, snapshotted per workspace and per forecast run.

## Consequences

Positive:

- Identity survives re-imports and source changes; renames never break history.
- Every attribution is explainable through the mapping tables; mis-attribution becomes visible, not silent.
- Week bucketing is deterministic and reproducible; runs record the timezone they used.

Negative:

- First-time setup requires explicit mapping work; the UX must make bulk mapping efficient.
- Mapping maintenance is ongoing operational effort when sources add codes.

## Alternatives Considered

- **Natural keys** (source code as primary key): breaks when codes change or collide across sources.
- **Fuzzy or name-based auto-matching**: fast but silently wrong; rejected.
- **Global mappings** (not per profile): collides when two sources use the same code for different products.
