# TraverseEmbedder for Kotlin/Android

This public Android library is the Spec-068 foundation for `embedder-api/1.0.0`.
It exposes validated bundle and lifecycle types plus an in-memory deterministic
harness for conformance tests. It never launches `traverse-cli serve` or uses
server-discovery files in production.

Runtime-WASM bridging, runtime event subscriptions, release evidence, and the
Compose reference-app integration remain tracked by Traverse #648.
