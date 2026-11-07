# traverse-embedder

Public Traverse platform embedder SDK for Rust hosts — the Linux GTK and CLI
delivery of spec `068-public-platform-embedder-packages`, implementing every
[`embedder-api/1.0.0`](../../specs/057-embeddable-runtime-host/embedder-api-1.0.0.json)
operation against an application-owned bundle. Production execution requires
no `traverse-cli serve` sidecar and no `.traverse/server.json` discovery.

## Supported platforms

Linux and any host with a Rust toolchain (the crate is pure Rust over the
natively linked Traverse runtime; macOS and Windows work for development).
The `platform` value in `EmbedderConfig` gates compatible-mode capabilities
against their manifest `platforms[]` allowlist.

## Operations

| `embedder-api/1.0.0` | Rust surface |
| --- | --- |
| `runtime.init` | `BundleEmbedder::init(EmbedderConfig)` |
| `runtime.shutdown` | `TraverseEmbedderApi::shutdown` |
| `runtime.submit` | `TraverseEmbedderApi::submit` (workflow or capability id) |
| `runtime.subscribe` | `TraverseEmbedderApi::subscribe` (ordered, replayed) |
| `compatible.start` | `TraverseEmbedderApi::start_compatible` |
| `compatible.stop` | `TraverseEmbedderApi::stop_compatible` |
| `compatible.kill` | `TraverseEmbedderApi::kill_compatible` |

Compatible capabilities follow the same lifecycle rules as `runtime.wasm`,
from `traverse_contracts::CompatibleLifecycle` (Decision 129).
`start_compatible` takes the input as raw JSON text, rejects input Spec 1722
can't represent, and runs one instance at a time. The started
`state_changed` event's `input` is the canonical input.

```rust,no_run
use serde_json::json;
use traverse_embedder::{BundleEmbedder, EmbedderConfig, TraverseEmbedderApi};

let mut embedder = BundleEmbedder::init(EmbedderConfig::new("app/app.manifest.json"))
    .expect("bundle should initialize");
embedder.subscribe(Box::new(|event| println!("{event}")));
embedder.submit("my-app.process", &json!({ "note": "hello" }));
embedder.shutdown();
```

## Optional host-owned local state

Durable state is opt-in and remains owned by the embedding host. The host
chooses the root, creates the adapter, fixes the public/private
classification, and injects it after initialization. Traverse does not derive
a root, create one by default, or expose the store to capabilities.

```rust,no_run
use serde_json::json;
use traverse_embedder::{BundleEmbedder, EmbedderConfig, HostDataStore};
use traverse_runtime::data_store::{
    LocalDataClassification, LocalFileDataStore, StateRecord,
};

let mut embedder = BundleEmbedder::init(EmbedderConfig::new("app/app.manifest.json"))?;
let local_store = LocalFileDataStore::new("/host-selected/app-state")?;
embedder.inject_data_store(HostDataStore::new(
    local_store,
    LocalDataClassification::Private,
));
embedder.data_store_write(StateRecord {
    key: "last-opened".into(),
    value: json!("document-42"),
    lamport_clock: 1,
    writer_id: "my-host".into(),
})?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

Only explicit host `read`, `write`, and `delete` operations are available.
Their safe error codes include `data_store_not_configured`, `store_locked`,
`integrity_check_failed`, `durability_commit_failed`, and `storage_io_failed`.
DataStore telemetry reports only operation, outcome, and classification — never
the root, keys, or values.

## Bundle input shape

`init` consumes the `app.manifest.json` bundle defined by spec
`044-application-bundle-manifest`: component manifests, capability contracts,
digest-verified WASM artifacts, and workflow definitions, all
application-owned and shipped with the host binary.

## Runtime-WASM compatibility

The Traverse runtime is linked natively into this crate at the same
workspace version; the Rust package ships no separate runtime-WASM artifact.
Bundle WASM components must target the Traverse host ABI supported by the
linked runtime. Incompatible bundles (unsupported `schema_version`, digest
mismatch, invalid manifests) are rejected deterministically at `init` with a
stable error code — never by falling back to a sidecar (spec 068 NFR-001).

## Shutdown and cancellation

`shutdown` force-terminates every running compatible instance (emitting
`state_changed` events with state `killed`), then rejects all further
operations with `runtime_stopped`. It is idempotent.

## Error mapping

Boundary failures use stable `EmbedderErrorCode` values
(`bundle_load_failed`, `unsupported_bundle_schema`, `runtime_stopped`,
`target_not_found`, `compatible_lifecycle_required`,
`capability_not_compatible`, `platform_not_supported`,
`compatible_instance_active`, `compatible_instance_not_active`,
`input_not_canonicalizable`, `invalid_input`, …). The compatible lifecycle
codes are `runtime.wasm`'s own (Spec 072, Decisions 128 and 129).
Runtime execution failures surface inside `error` events with the runtime's
stable snake_case codes (`execution_failed`, `capability_not_found`, …).
Secrets never appear in errors, events, or evidence (spec 068 NFR-004).

## Test double

`EmbedderTestDouble` implements the same `TraverseEmbedderApi` boundary with
scripted results, identical event envelopes, deterministic identifiers, and
the same compatible lifecycle — for host tests without WASM or file I/O
(spec 068 FR-006).

## Mediated connectors (Spec 104 / Spec 1719)

Native hosts activate `traverse.model-runtime` with
`traverse_runtime::activate_exact_model_runtime`. Browser/Node use
`activateMediatedConnector` + `submitAsync`. Minimal examples:
[host-connector-command-dispatch.md](../../docs/host-connector-command-dispatch.md).

## Upgrade policy

- Embedder API `1.0.0`; a new IDL version requires a new conformance suite
  revision and a release stating the new version in its evidence.
- Supported bundle schema versions: `1.0.0`.
- Semantic versioning; the workspace currently versions in lockstep.

## Release evidence

`release_evidence()` returns JSON recording the package name/version, the
linked runtime version and linkage, embedder API + conformance versions,
supported bundle schemas, and the digest of every bundled WASM component
(spec 068 FR-008, NFR-002).

## Conformance

`tests/conformance.rs` executes the shared spec 057 corpus: `init-shutdown`,
`wasm-capability-submit`, `compatible-lifecycle`, `platform-guard`, and
`determinism`. Certification metadata:

```json
{ "traverse_embedder_api": "1.0.0", "conformance_passed": true }
```
