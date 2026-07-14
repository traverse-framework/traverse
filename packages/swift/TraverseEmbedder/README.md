# TraverseEmbedder for Swift

`TraverseEmbedder` is the public iOS/macOS Swift Package foundation for
`embedder-api/1.0.0` (Spec 068). It exposes bundle validation, lifecycle,
submission, and compatible-capability operations plus `InMemoryTraverseEmbedder`
for deterministic conformance tests. Its `subscribe(after:)` operation returns
the ordered runtime-shaped events recorded by the harness. Compatible-capability
start, stop, and kill operations return stable instance identifiers and lifecycle
results. It never starts `traverse-cli serve` or uses server-discovery files.

The production runtime-WASM bridge, event subscription stream, package release
evidence, and app-reference integration are tracked by Traverse #647.

## Bundle compatibility

Applications provide a bundle root URL, the runtime-WASM SHA-256 digest used
for release traceability, and the bundle's embedder API version. The API version
defaults to the package's `TraverseEmbedder.apiVersion`. Initialization rejects
a bundle declaring a different version with `incompatibleBundle`; it does not
start a sidecar or attempt a network fallback.

The package follows semantic versioning. Additive, backward-compatible API
changes use minor releases; breaking public API or error-semantic changes use a
new major version. Call `shutdown()` to clear the active bundle, submission
sequence, and recorded events before cancellation or replacement.
