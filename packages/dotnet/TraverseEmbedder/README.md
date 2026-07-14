# TraverseEmbedder for .NET/WinUI

This public .NET library is the Spec-068 package foundation for
`embedder-api/1.0.0`. It contains the stable bundle, submission, lifecycle,
and compatible-capability boundary plus `InMemoryTraverseEmbedder` for
deterministic conformance tests. Its `Subscribe` operation returns ordered
runtime-shaped harness events. It never depends on `traverse-cli serve` or
server-discovery files in production.

The runtime-WASM bridge, event subscriptions, release evidence, and WinUI
reference-app integration remain tracked by Traverse #649.
