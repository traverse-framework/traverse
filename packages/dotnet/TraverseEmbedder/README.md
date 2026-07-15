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

The runtime-WASM bridge, event subscriptions, evidence publication, and WinUI
reference-app integration remain tracked by Traverse #649.
