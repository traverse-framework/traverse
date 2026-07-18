# TraverseEmbedder for Swift

`TraverseEmbedder` is the public iOS/macOS Swift Package foundation for
`embedder-api/1.0.0` (Spec 068). It exposes bundle validation, lifecycle,
submission, and compatible-capability operations plus `InMemoryTraverseEmbedder`
for deterministic conformance tests. Its `subscribe(after:)` operation returns
the ordered runtime-shaped events recorded by the harness. Compatible-capability
start, stop, and kill operations return stable instance identifiers and lifecycle
results. It never starts `traverse-cli serve` or uses server-discovery files.

`WasmKitRuntimeBridge` is the production runtime loader. It resolves only the
core WasmKit product, verifies `runtime/runtime.wasm` against its declared
SHA-256 digest and a 32 MiB default artifact limit before parsing, rejects all
ambient imports, and validates the memory, function signatures, and ABI version
required by `runtime-wasm-bridge/1.1.0`, including compatible lifecycle exports.
It never links WasmKitWASI. JSON
marshalling is provided by `WasmKitBridgeClient`, which serializes calls,
copies runtime-owned output before the next mutation, bounds descriptors, and
releases every caller allocation exactly once.

`RuntimeTraverseEmbedder` maps that boundary into stable public Swift
submission, event, and compatible-lifecycle result types without synthesizing
runtime identifiers, ordering, or statuses.
Evidence publication and app-reference integration remain tracked by Traverse
#647.

Release tooling constructs `TraverseReleaseEvidence` with the semantic package
version, runtime-WASM digest, conformance version, and supported iOS/macOS host
versions. The initializer rejects incomplete evidence before publication so a
downstream binary can be traced to the exact package and runtime pairing.

## Bundle compatibility

Applications provide a bundle root URL, the runtime-WASM SHA-256 digest used
for release traceability, and the bundle's embedder API version. The API version
defaults to the package's `TraverseEmbedder.apiVersion`. Initialization rejects
a bundle declaring a different version with `incompatibleBundle`; it does not
start a sidecar or attempt a network fallback.

The package pins WasmKit 0.2.2 and swift-system 1.5.0 exactly. The accompanying
`dependency-review.json` records why this is the newest reviewed combination
compatible with the package's Swift 6.0 toolchain and discloses the engine's
remaining resource-control limitations.

The package follows semantic versioning. Additive, backward-compatible API
changes use minor releases; breaking public API or error-semantic changes use a
new major version. Call `shutdown()` to clear the active bundle, submission
sequence, and recorded events before cancellation or replacement.
