# Cross-host registry-cache error/evidence projection matrix

Repository-controlled evidence for spec `1258-offline-cache-activation` FR-006
("Rust and Web cache paths MUST expose equivalent evidence and errors; native
coverage follows the Spec-107 conformance matrix") and FR-004 (missing, altered,
lifecycle-rejected, signature-invalid, ABI-incompatible, and target-incompatible
entries fail closed with stable secret-free errors).

`projection-matrix.json` is the single canonical, recursively key-sorted
redacted projection for each failure category. It is evidence data only: it adds
no cache ownership, network policy, or resolver behavior.

## Consumers

| Host | Test |
| --- | --- |
| Rust (`traverse-embedder`) | `crates/traverse-embedder/tests/registry_cache_cross_host_projection.rs` |
| Web (`packages/web/TraverseEmbedder`) | `packages/web/TraverseEmbedder/tests/registryCacheCrossHostProjection.test.mjs` |

Each host builds the redacted evidence object for every category from its own
types and asserts, after recursive key-sorting, that it equals
`categories[].redacted_evidence` in this fixture. Both tests also assert:

- only `allowed_evidence_fields` keys appear;
- no value contains a path separator, `://`, an `authorization`/`bearer` token,
  a `secret`/`password`, or a `token=` query fragment;
- the `missing` and `altered` codes equal the production
  `RegistryCacheErrorCode` values shipped by that host.

Because both hosts compare against one shared canonical projection, an
inequality on either side is a cross-host conformance failure.

## Native matrix (spec 107 FR-009)

Spec 107 FR-009 scopes required native (`swift`, `kotlin`, `dotnet`) equivalence
to `preparation_success`, `missing_cache`, `yanked_dependency`, and
`artifact_digest_mismatch`. `native_matrix.coverage` records, per category,
whether it is in that set and the observable native code:

- `missing`, `altered` — in the set; native adapters emit
  `registry_cache_entry_missing` / `registry_artifact_digest_mismatch`.
- `lifecycle-rejected` — partial; native equivalence covers the yanked case
  (`registry_dependency_yanked`). Inactive/draft rejection
  (`registry_lifecycle_rejected`) is a Rust/Web resolution-path projection.
- `signature-invalid`, `abi-incompatible`, `target-incompatible` — outside the
  spec 107 FR-009 native set; recorded as `rust_web_projection_only`.
