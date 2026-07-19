# Native Runtime Artifact Registry

This document describes how a native package (Swift, Kotlin, or .NET)
acquires the production `runtime.wasm` bridge artifact built by
`crates/traverse-native-bridge` (Traverse #756), through the registry
publish/resolve API in `crates/traverse-registry`'s
`native_runtime_artifact` module (Traverse #757).

It implements the distribution contract defined by
[`specs/075-native-runtime-distribution-contract/spec.md`](../specs/075-native-runtime-distribution-contract/spec.md)
and the channel decision recorded in
[ADR-0012](adr/0012-native-runtime-distribution-channel.md): reuse Traverse's
existing registry publish/resolve infrastructure instead of a bespoke
per-package distribution channel, and resolve metadata at package
build/release time rather than at process runtime.

## Publication

A release publishes exactly one `NativeRuntimeArtifactRecord` into a
`NativeRuntimeArtifactIndex`, identified by an immutable
`runtime_version` + certified `bridge_version` + SHA-256 `sha256` digest
tuple (Spec 075 FR-001):

```rust
use traverse_registry::{
    HostCertification, NativeRuntimeArtifactRecord, publish_native_runtime_artifact,
    write_native_runtime_registry,
};

let record = NativeRuntimeArtifactRecord {
    runtime_version: "0.9.0".to_string(),
    bridge_version: "1.1.0".to_string(),
    supported_bridge_range: ">=1.1.0,<2.0.0".to_string(),
    sha256: "<runtime.wasm digest>".to_string(),
    artifact_url: "<content-addressed artifact location>".to_string(),
    host_certifications: vec![
        HostCertification {
            host: "swift".to_string(),
            engine_name: "WasmKit".to_string(),
            engine_version: "0.3.1".to_string(),
            conformance_passed: true,
        },
        // one entry per certified host (kotlin, dotnet), same schema
    ],
};

publish_native_runtime_artifact(&mut index, record)?;
write_native_runtime_registry(workspace_root, &index)?;
```

Publishing rejects a record that reuses an already-published
`runtime_version` (Spec 075 FR-007): a corrected build always publishes
under a new version rather than overwriting one. The index is written
atomically to `runtime/native-runtime-registry.json`, alongside the
`runtime.wasm` and `runtime-release.json` outputs from #756.

## Resolution

Swift, Kotlin, and .NET packages resolve through the same function and the
same field names — the schema carries no host-specific requirement
(Spec 075 FR-008):

```rust
use traverse_registry::{load_native_runtime_registry, resolve_native_runtime_artifact};

let index = load_native_runtime_registry(workspace_root)?;
let record = resolve_native_runtime_artifact(
    &index,
    pinned_runtime_version,
    &fetched_artifact_digest,
    package_supported_bridge_range,
    host_label, // "swift", "kotlin", or "dotnet"
)?;
```

Resolution never performs a network call or depends on an HTTP/CLI sidecar
(Spec 075 FR-009): both the index and the artifact are already local to the
build. It rejects, deterministically and before any bridge instantiation:

| Condition | Error code |
| --- | --- |
| No release published at the pinned `runtime_version` | `ArtifactNotFound` |
| Fetched artifact bytes don't hash to the published `sha256` | `DigestMismatch` (tamper) |
| The release's certified `bridge_version` falls outside the package's declared range | `BridgeVersionMismatch` |
| No passing certification exists for the requesting host | `UncertifiedHost` |

Every published release stays independently resolvable by its exact
`runtime_version` after a newer release publishes (Spec 075 FR-006), so a
package can pin, audit, or roll back without losing access to prior
evidence.

## Scope

This module governs runtime artifact identity and resolution only. It does
not build the artifact (#756), run host-package conformance (#758), or
change the public `embedder-api` surface (Specs 057, 068, 071–073, which
this distribution layer sits beneath without revising).
