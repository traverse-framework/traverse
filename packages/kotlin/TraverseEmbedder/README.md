# TraverseEmbedder for Kotlin/Android

This public Android library is the Spec-068 foundation for `embedder-api/1.0.0`.
It exposes validated bundle and lifecycle types plus an in-memory deterministic
harness for conformance tests. Its `subscribe(afterSequence)` operation
returns ordered runtime-shaped harness events. Compatible-capability start,
stop, and kill operations return stable instance identifiers and lifecycle
results. `TraverseReleaseEvidence` records the package version, runtime-WASM
digest, conformance version, and supported Android host versions for a release.
It never launches `traverse-cli serve` or uses server-discovery files in
production.

`ChicoryRuntimeBridge` is the production runtime loader. It pins Chicory 1.7.5,
verifies `runtime/runtime.wasm` against its declared SHA-256 digest and a 32 MiB
default artifact limit before parsing, rejects ambient imports, and validates
the complete `runtime-wasm-bridge/1.1.0` memory, function-signature, and ABI
surface without linking Chicory WASI. Runtime JSON marshalling, serialized
event draining, typed mapping, and shared conformance are provided by the
package; Compose reference-app integration remains tracked by Traverse #648.

`ChicoryBridgeClient` now implements the serialized UTF-8 JSON ABI boundary,
copies runtime-owned outputs before the next mutation, releases every caller
allocation exactly once, bounds output descriptors, and exposes ordered
single-event draining. Each call has a configurable instruction/deadline budget,
and modules declaring memory above the configured host ceiling fail before
instantiation. Compose integration remains tracked by #648.

`RuntimeTraverseEmbedder` maps that boundary into stable public Kotlin
submission, event, and compatible-lifecycle result types while preserving
runtime-owned identifiers and statuses.
