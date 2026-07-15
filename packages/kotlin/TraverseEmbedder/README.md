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

Runtime-WASM bridging, runtime event subscriptions, release evidence, and the
Compose reference-app integration remain tracked by Traverse #648.
