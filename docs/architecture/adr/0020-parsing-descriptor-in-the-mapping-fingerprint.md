# ADR-0020: Parsing configuration in the mapping fingerprint

**Status:** Accepted
**Date:** 2026-10-14
**Extends:** [ADR-0013](0013-canonical-import-and-mapping-idempotency.md)

## Context

ADR-0013 makes `(profile_id, content_hash, mapping_fingerprint, contract_version)` the import idempotency key, where `mapping_fingerprint` covers the **effective mapping plan**. With ADR-0019, the interpretation of raw bytes is no longer just product/location code mappings: the same bytes read with a different column binding, encoding, delimiter, header row, date formats, separators, or aggregation rule produce different observations. Hashing only the code mappings would silently dedupe a re-import that the user expects to be a new revision.

## Decision Drivers

- Same bytes under a different interpretation must never collapse into the existing revision.
- Canonical imports must keep their existing fingerprints byte-identical, so no committed canonical revision is invalidated by this change.
- The fingerprint must be reproducible and reviewable from stored evidence.

## Decision

- The **effective mapping plan** now means: contract version, the exact product and location code mappings, plus, when the import is preprocessed, one deterministic descriptor digest.
- The descriptor enters the fingerprint as a single line `descriptor_sha256=<hex>`, the SHA-256 of the descriptor's canonical JSON. Canonical (non-preprocessed) imports add no line, so their fingerprints are unchanged.
- Canonical serialization: field order follows the struct declaration, no unordered maps, no locale-dependent formatting; the descriptor carries `descriptor_version` and a change to the serialization itself requires bumping it. A golden-JSON test pins the format.
- Non-parsing metadata (a configuration name, timestamps, ids) never enters the descriptor and therefore never affects the fingerprint.
- Outcomes: same bytes + same descriptor + same mappings = no-op; any changed parsing field = a new immutable revision; failed attempts stay in `job`. Preprocessed revisions are labelled `contract_version = 'preprocess/1'` (ADR-0019).

## Consequences

Positive:

- Idempotency stays honest for raw sources: "the same file imported the same way" means exactly that.
- Editing a saved configuration cannot rewrite history; a re-import with the edited configuration is a new revision, and the old one keeps its frozen snapshot (ADR-0021).

Negative:

- Any future field reordering or formatting change in the descriptor requires a `descriptor_version` bump, or fingerprints would change implicitly.

## Alternatives Considered

- **Keep the fingerprint as code mappings only**: rejected; it silently dedupes a changed interpretation.
- **Hash the descriptor with a generic JSON canonicalizer**: rejected; an extra dependency for a shape we already control, whereas declaration order plus a golden test is sufficient.
