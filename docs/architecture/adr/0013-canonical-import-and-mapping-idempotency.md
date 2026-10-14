# ADR-0013: Canonical Import and Mapping Idempotency

**Status:** Accepted
**Date:** 2026-10-13
**Extended by:** [ADR-0019](0019-raw-source-preprocessing-layer.md) — a raw-source preprocessing layer converts planner exports into `canonical/1` — and [ADR-0020](0020-parsing-descriptor-in-the-mapping-fingerprint.md) — the effective mapping plan now includes a versioned parsing descriptor.

## Context

Imports must be idempotent, attributable, and crash-safe, and the importer must never guess identities from names. The Phase 1 review replaced the content-only idempotency key, introduced the `canonical/1` temporary input contract, and required the effective mapping plan to be part of the fingerprint. The semantics of the retained bytes must be qualified explicitly instead of guessed at hash time.

## Decision Drivers

- No silent mis-attribution: identity comes from explicit mappings only.
- Idempotency must account for mappings, not only content.
- The hash must be computable without reinterpreting the file: hash the raw retained bytes.
- One global forecast cutoff, independent of the wall clock.

## Decision

- Temporary input contract `canonical/1`: columns `series_key`, `week_start` (ISO Monday), `quantity` (finite, non-null). Zero is explicit `zero`, negative values are `return`, positive values are `known`; identical duplicate cells collapse and conflicting cells are excluded with an import issue. A later preprocessing layer may convert arbitrary planner files into `canonical/1` or a later version without changing the engine contract.
- A new `series_key` creates an exact identity mapping (`product_code_map`) and product; no fuzzy or name-based matching exists. All `canonical/1` rows land on the workspace's fixed technical default location, preserving the product/location architecture.
- `content_hash` is the SHA-256 of the raw retained bytes. No canonicalization guessing happens before hashing; `contract_version` qualifies the parsing semantics of those bytes.
- `mapping_fingerprint` is computed deterministically from the effective mapping plan, including identity mappings created by that import.
- `import_revision` represents committed snapshots only and stores `content_hash`, `mapping_fingerprint`, and `contract_version`. Uniqueness is `(profile_id, content_hash, mapping_fingerprint, contract_version)`: same content plus the same fingerprint is a no-op returning the existing revision; same content plus a changed fingerprint creates a new immutable revision. Failed attempts live in `job` and never block a retry.
- `forecast_run.horizon_start_week` is the Monday after the maximum imported week in the pinned revision, never the wall clock. Rust aligns all series to that global cutoff; weeks absent for a series are treated as missing for trailing-history qualification.

## Consequences

Positive:

- Re-importing the same file with the same mappings is a provable no-op.
- Mapping changes are visible as new immutable revisions instead of silent overwrites.
- The cutoff is deterministic and reproducible from the revision's data.

Negative:

- Fingerprint computation must enumerate the effective mapping plan consistently in one place.
- A changed mapping fingerprint duplicates retained files and observations (accepted: immutable snapshots).

## Alternatives Considered

- **`UNIQUE(profile_id, content_hash)`** (previous design): cannot distinguish re-imports under changed mappings; rejected.
- **Hashing canonicalized content**: requires guessing semantics before parsing; rejected in favor of raw bytes plus `contract_version`.
- **Wall-clock horizon start**: not reproducible from data and shifts with import timing; rejected.
- **Fuzzy/name matching for new `series_key`s**: silently wrong attribution; rejected (ADR-0004 stands).
