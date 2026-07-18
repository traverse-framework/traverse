# TraverseEmbedder for .NET/WinUI

This public .NET library is the Spec-068 package foundation for
`embedder-api/1.0.0`. It contains the stable bundle, submission, lifecycle,
and compatible-capability boundary plus `InMemoryTraverseEmbedder` for
deterministic conformance tests. Its `Subscribe` operation returns ordered
runtime-shaped harness events. Compatible-capability start, stop, and kill
operations return stable instance identifiers and lifecycle results. It never
depends on `traverse-cli serve` or server-discovery files in production.

Release tooling constructs `TraverseReleaseEvidence` with the semantic package
version, runtime-WASM digest, conformance version, and supported Windows host
versions. `Validate` rejects incomplete evidence before publication so a
downstream binary can be traced to its exact package and runtime pairing.

`WasmtimeRuntimeBridge` pins Wasmtime .NET 44.0.0, verifies the runtime artifact
before compilation, rejects all ambient imports, and validates the complete
`runtime-wasm-bridge/1.1.0` memory, function-signature, ABI, and compatible
lifecycle export surface without enabling WASI. The host applies a 32 MiB
runtime-memory ceiling, 10,000,000 fuel, and a 30-second epoch deadline to each
bridge call by default.

`WasmtimeBridgeClient` serializes UTF-8 JSON calls, copies runtime-owned output
before the next mutation, bounds descriptors, and releases each caller-owned
input and descriptor allocation exactly once.

`RuntimeTraverseEmbedder` maps the raw boundary into stable public submission,
event, and compatible-lifecycle result records while preserving runtime-owned
identifiers, ordering, and statuses.

Request marshalling, event subscriptions, evidence publication, shared
conformance, and WinUI reference-app integration remain tracked by Traverse #649.
