# TraverseEmbedder for .NET/WinUI

This public .NET library is the Spec-068 package foundation for
`embedder-api/1.1.0`. It contains the stable bundle, submission, lifecycle,
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

`RuntimeTraverseEmbedder.Submit(TraverseAppCommand)` submits Spec 139
`app_command` envelopes (`kind`, `command`, `payload`, optional `session_id`).
The state machine runs in `runtime.wasm`; the embedder never re-implements
transitions. Host authorities are registered per manifest command with
`RegisterHostConnectorAdapter` (Spec 140 WIT semantics; the returned
`IDisposable` removes the registration). When the runtime stages a
host-connector wait the adapter runs and its result is submitted as a
`host_connector_result` terminal; a command with no adapter completes as
`failed` / `target_incompatible`. The host half of the dual deadline is
registered on the `ITraverseTimer` port (default `SystemTraverseTimer`) and
submits `deadline_fired`. `Shutdown` cancels adapters and timers and drops any
late completion. The first correlated terminal wins in the runtime.

A command the runtime rejects (no transition, or issued during a wait) returns a
`TraverseSubmissionResult` with `Status` `rejected` and the runtime's `Error`, like web.
`Subscribe()` delivers Spec 139 app lifecycle events (`state_changed`, `host_connector_*`,
`capability_*`, `error`) as `TraverseRuntimeEvent`s with `EventType`, `SessionId`, and
`Output` (the event `data` as JSON), numbered in arrival order; unknown runtime types
surface as `error`. Legacy bridge events keep their original shape.

Request marshalling, event subscriptions, evidence publication, shared
conformance, and WinUI reference-app integration remain tracked by Traverse #649.
