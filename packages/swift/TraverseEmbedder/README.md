# TraverseEmbedder for Swift

`TraverseEmbedder` is the public iOS/macOS Swift Package foundation for
`embedder-api/1.1.0` (Spec 068). It exposes bundle validation, lifecycle,
submission, and compatible-capability operations plus `InMemoryTraverseEmbedder`
for deterministic conformance tests. Its `subscribe(after:)` operation returns
the ordered runtime-shaped events recorded by the harness. Compatible-capability
start, stop, and kill operations return stable instance identifiers and lifecycle
results. It never starts `traverse-cli serve` or uses server-discovery files.

`WasmiHostBridgeClient` is the production runtime loader. It links
`TraverseSwiftHost`, a prebuilt XCFramework wrapping the `wasmi` interpreter
behind a narrow C ABI, verifies `runtime/runtime.wasm` against its declared
SHA-256 digest before instantiation, and enforces host-owned `TraverseHostLimits`
(artifact size, linear-memory ceiling, fuel per invocation, and input/output/event
bounds) on every call. Exceeding any limit fails the call with a stable
`bridge_resource_limit` error rather than allowing continued unbounded execution.
This is the certification path defined by Spec 074 (Swift Native
Resource-Control Certification) and governed by ADR-0014 (wasmi Apple runtime
profile) and ADR-0015 (production Swift wasmi C ABI).

`RuntimeTraverseEmbedder` maps that boundary into stable public Swift
submission, event, and compatible-lifecycle result types without synthesizing
runtime identifiers, ordering, or statuses. Its default initializer constructs
a `WasmiHostBridgeClient`.

WasmKit is **not supported** and is not a package dependency. The production
product and default resolve path use only `TraverseSwiftHost` / wasmi.

Release tooling constructs `TraverseReleaseEvidence` with the semantic package
version, runtime-WASM digest, conformance version, and supported iOS/macOS host
versions. The initializer rejects incomplete evidence before publication so a
downstream binary can be traced to the exact package and runtime pairing.

## Bundle compatibility

Applications provide a bundle root URL, the runtime-WASM SHA-256 digest used
for release traceability, and the bundle's embedder API version. The API version
defaults to the package's `TraverseEmbedder.apiVersion`. Initialization rejects
a bundle declaring a different version with `incompatibleBundle`; it does not
start a sidecar or attempt a network fallback.

The package pins `TraverseSwiftHost` to a specific released XCFramework
checksum (currently swift-host-v1.1.0) for the production `wasmi` bridge. The
accompanying `dependency-review.json` records that engine selection.

`RuntimeTraverseEmbedder.submit(TraverseAppCommand)` submits Spec 139
`app_command` envelopes; the state machine runs in `runtime.wasm` and the
embedder never re-implements transitions. Register host authorities per
manifest command with `registerHostConnectorAdapter(command:adapter:)` (Spec 140
WIT semantics; `HostConnectorRegistration.remove()` only removes the adapter it
registered). When the runtime stages a host-connector wait, the adapter runs and
its result is submitted as a `host_connector_result` terminal; a command with no
adapter completes as `failed` / `target_incompatible`, and an adapter error
completes as `failed` / `execution_failed` without native detail. The host half
of the dual deadline is registered on the `TraverseTimer` port (default
`SystemTraverseTimer`) and submits `deadline_fired`. `shutdown()` cancels
adapters and timers and drops any late completion. The first correlated
terminal wins in the runtime.

### Audio input adapter (Spec 140)

`AppleAudioInputAdapters` implements the `traverse.audio-input` host adapter for
Apple hosts. `requestPermission` and `captureAudio` are `HostConnectorAdapter`s to
register for the manifest commands routed to `audio.permission.request` and
`audio.capture`; `cancel(correlationID:)` is WIT `cancel`. Permission and capture
are behind `AudioPermissionDriver` / `AudioCaptureDriver`; the defaults,
`AVFoundationAudioPermissionDriver` and `AVFoundationAudioCaptureDriver`
(`AVAudioEngine`, mono 16-bit PCM WAV), require the host app to declare
`NSMicrophoneUsageDescription`. The capture driver never prompts: it fails
`policy_denied` unless permission is already granted. Captured bytes are staged
through `ArtifactStagingStore.stageArtifact`, and only the opaque `artifact_ref`
or a non-secret `permission_state` is returned. Failures are `failed` results
carrying `error_code` (`policy_denied`, `unavailable`, `input_limit_exceeded`) or
a `cancelled` result; native error text is dropped.

The package follows semantic versioning. Additive, backward-compatible API
changes use minor releases; breaking public API or error-semantic changes use a
new major version. Call `shutdown()` to clear the active bundle, submission
sequence, and recorded events before cancellation or replacement.
