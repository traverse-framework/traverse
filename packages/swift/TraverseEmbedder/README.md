# TraverseEmbedder for Swift

`TraverseEmbedder` is the public iOS/macOS Swift Package foundation for
`embedder-api/1.0.0` (Spec 068). It exposes bundle validation, lifecycle,
submission, and compatible-capability operations plus `InMemoryTraverseEmbedder`
for deterministic conformance tests. It never starts `traverse-cli serve` or
uses server-discovery files.

The production runtime-WASM bridge, event subscription stream, package release
evidence, and app-reference integration are tracked by Traverse #647.
