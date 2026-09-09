# Verified Registry Reference Lifecycle

Governing specs: `996-registry-app-preparation` and
`1258-offline-cache-activation`.

Registry-backed application components use one fixed lifecycle:

1. Sync a validated Registry index.
2. Run the host-owned preparation boundary to retrieve and verify contract,
   artifact, signature, lifecycle, ABI, target, placement, and digest evidence.
3. Commit verified bytes under their digest keys without replacing different
   bytes already stored for a key.
4. Validate, register, and activate the application using only those prepared
   cache entries.
5. Execute using the persisted activation evidence; execution does not fetch or
   select an alternate Registry version.

An exact reference such as `=1.0.1` only accepts `1.0.1`. A missing exact
record never falls back to a nearby version.

## Safe troubleshooting

Public outcomes identify a lifecycle boundary without exposing endpoints,
credentials, cache paths, headers, or artifact bytes:

- `registry_version_range_unsatisfied`: the exact requested version is absent.
- `registry_lifecycle_rejected`: the selected release is not active.
- `registry_contract_digest_mismatch` or
  `registry_artifact_digest_mismatch`: prepared bytes changed or do not match
  their published digest.
- `registry_signature_unverified`: trust evidence did not verify.
- `registry_abi_incompatible` or `registry_target_incompatible`: the host or
  requested placement cannot run the selected release.
- `registry_cache_commit_failed` or `registry_cache_entry_missing`: preparation
  did not produce usable immutable cache evidence.

Resolve these conditions by re-running the host preparation workflow with the
appropriate policy and verified Registry input. Do not edit the manifest to
substitute a local path, copy cache entries between identities, or enable a
network fallback during validation, activation, or execution.

## Validation evidence

The CLI activation record contains selected artifact and connector evidence but
omits host-private paths and configuration values. A failed activation is safe
to share when it includes only its stable code, public capability identity, and
requested version range.

## Cross-host projection conformance

Spec `1258-offline-cache-activation` FR-006 requires the Rust and Web cache
paths to expose equivalent evidence and errors, with native coverage following
the Spec-107 conformance matrix.
`fixtures/cross-host/registry-cache-projections/projection-matrix.json` is the
single canonical, key-sorted redacted projection for each FR-004 failure
category (missing, altered, lifecycle-rejected, signature-invalid,
ABI-incompatible, target-incompatible). It is evidence data only and changes no
cache ownership, network policy, or resolver behavior.

Both hosts assert that fixture:

- Rust: `crates/traverse-cli/src/registry_resolution_diagnostics.rs`
  (`cross_host_projection_matrix_matches_rust_projection`).
- Web:
  `packages/web/TraverseEmbedder/tests/registryCacheCrossHostProjection.test.mjs`.

Each canonical projection carries only the permitted identity fields and no
path, URL, endpoint, header, or credential-shaped value. Native adapters
(`swift`, `kotlin`, `dotnet`) are equivalent for the Spec-107 FR-009 set
(`preparation_success`, `missing_cache`, `yanked_dependency`,
`artifact_digest_mismatch`); `signature-invalid`, `abi-incompatible`, and
`target-incompatible` are recorded as Rust/Web resolution-path projections
outside that native set.
