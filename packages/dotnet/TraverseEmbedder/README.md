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

## Exact-ref model execution (Spec 138, Decision 111)

`ExactModelHost` runs signed exact-ref model packages through the same Rust
code as the native, Swift, and Kotlin hosts. That code is the shared
`traverse-model-host-frame` crate, on `wasmi`, behind two audited P/Invoke
functions in the `traverse-dotnet-host` native library (ADR-0081). No
Spec 138 rule is re-implemented in C#: Ed25519 verification, rights, the
usage policy, package status, derivation, host ceilings, and guest ABI v1
to v3 all come from Rust.

- `new ExactModelHost(pins, trustedPublicKeysHex, modelUsage,
  hostRequiresCommercial, limits)`. `ExactModelHostLimits` defaults to the
  native desktop ceilings: 256 MiB package, 1 GiB memory, 5×10¹⁰ fuel, and
  512 MiB of snapshots.
- `RegisterPackageAsync`, `StageModelInput` / `ReadModelOutput`,
  `ModelRights` / `ModelRightsRecord`, `SetPackageStatus`, and `DropRef`.
- `ExecuteAsync(..., timeoutMs, cancellationToken)`. Cancelling the token
  interrupts the running inference mid-run (`cancelled`).
- `Install(runtimeEmbedder, command)`, or `ModelExecuteAdapter`, routes an
  app command's Spec 137 `model.execute` payload (plus
  `allowed_classifications`) to the host.
- Failures are `ExactModelException`, carrying `Code`, `Reason` and the rights
  `Detail`. If the native library cannot load, every model call fails with
  `model_unavailable` / `engine_unavailable`, and the rest of the embedder
  keeps working.

The tests build the library for the current machine
(`scripts/build_dotnet_host_native.sh`), so `dotnet test` needs a Rust
toolchain. Shipped native assets for `win-x64`, `win-arm64`, and `linux-x64`
come from the publish workflow (#1643).
