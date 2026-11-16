# Feature Specification: Cross-Host Governed ONNX Runtime CPU Provider

**Status**: Approved (2026-11-16)
**Canonical governing ID**: `1774-onnxruntime-cpu-runner-family`
**Version**: 1.0.0
**Extends**: `138-governed-exact-model-execution`, `1259-portable-authority-contracts`, and `075-native-runtime-distribution-contract`.
**Decision evidence**: Decision 134; ADR-0084; Traverse #1859 and #1866.

## Purpose and boundary

Define `onnxruntime-cpu` as one provider-neutral, CPU-only, governed exact-model capability across every supported Traverse host. Applications name only `onnxruntime-cpu`; they MUST NOT name ONNX Runtime, a target profile, browser, Node, or a native artifact. Target selection remains host-owned placement policy.

This specification is not Callweave or Perch product scope. Perch may be an external evaluation fixture only. The first release is not permitted to be browser-only.

## Requirements

- **FR-001**: A signed, canonical runner-family manifest MUST bind a family identifier and release to every supported target profile: browser, Node, Rust/native, Swift, Kotlin/Android, and .NET. Each entry MUST include exact artifact digest, source revision, reproducible build flags, license/provenance, CPU-only execution declaration, fixed memory-profile identifier, and conformance-report digest.
- **FR-002**: A signed model package MUST bind a compatible runner-family release and fixed memory profile. A host MUST verify the family signature, family revocation state, matching target artifact digest, and profile before registration. It MUST fail closed rather than substitute a runner, model, target, or profile.
- **FR-003**: Runner artifacts MUST be activated from the signed host release or host-owned verified cache before registration. Execution MUST be offline: it MUST NOT fetch model bytes, code, a runner, or a replacement artifact.
- **FR-004**: The public operation remains the Spec 138 provider-neutral model operation. Application manifests and capability code MUST NOT import or expose ONNX Runtime APIs. Evidence MUST identify the selected internal profile, runner-family digest, target-artifact digest, fixed memory profile, elapsed time, and redacted resource outcome.
- **FR-005**: Every profile MUST enforce its signed fixed memory ceiling, package/input/output byte ceilings, deadline, cancellation, and deterministic teardown. A profile that cannot enforce its memory profile MUST reject registration as `model_incompatible`; an evidence-only limit is forbidden.
- **FR-006**: Browser execution MUST use a terminable Worker; Node execution MUST use a terminable worker-thread boundary. Swift, Kotlin/Android, .NET, and Rust/native MUST use the Traverse-owned audited native runner boundary and platform-appropriate deterministic cancellation/teardown. The implementation primitive is not application-visible.
- **FR-007**: Cancellation, deadline, crash, initialization failure, memory breach, artifact mismatch, cache miss, and revocation MUST return stable normalized Spec 138 outcomes and leave no live execution. Traces MUST redact raw tensor, audio, model, and credential bytes.
- **FR-008**: Every profile MUST pass one shared conformance suite. Statuses, reasons, schemas, selected-profile evidence, and non-floating output bytes MUST be byte-identical. Floating output is permitted only within one model-signed shared tolerance; a profile MUST NOT define a looser tolerance.
- **FR-009**: The initial family MUST expose CPU execution only. WebGPU, WebNN, Core ML, NNAPI, CUDA, and other accelerator paths require a separately approved profile, provenance record, limit model, and conformance suite.
- **FR-010**: The generic fixture suite MUST contain legally clear small and large graph fixtures, including at least one non-Callweave fixture. Perch graph bytes MUST remain outside the repository, registry, and releases; any Perch evaluation retrieval MUST be ephemeral, digest-verified, and labelled non-production.
- **FR-011**: A release claiming `onnxruntime-cpu` support MUST publish the signed runner-family manifest, each target artifact integrity record, license/provenance notices, and machine-readable conformance report. It MUST include all profiles named in FR-001; partial-host release is prohibited.

## Acceptance scenarios

1. The same signed package is registered on every supported host; each host selects its internal profile, verifies the same runner-family release, and returns the same normalized successful result.
2. A changed family field, wrong target digest, revoked family, unknown profile, or offline cache miss fails before inference with a stable redacted outcome.
3. A package selecting a memory profile unsupported by the current target is rejected before execution; a memory breach, cancellation, deadline, or crash leaves no worker, process, or native execution live.
4. Browser Worker, Node worker-thread, and every native bridge pass the same malformed-input, byte-limit, timeout, cancellation, crash, tamper, revocation, and redaction vectors.
5. A floating tensor differs within the model-signed tolerance; all non-floating outputs and policy/evidence fields remain exact. A wider profile-local tolerance fails conformance.
6. An application imports an ORT API or attempts execution-time runtime fetch; contract validation fails.

## Compatibility and non-goals

This is additive. Existing `wasm-cpu`, `tract-onnx`, and accelerator variants remain unchanged. Unsupported future hosts are not silently emulated or downgraded. This specification does not approve a Perch model, any Callweave workflow/UI, accelerator execution, a native application-only fallback, or network model delivery.

## Validation

Implementation PRs MUST cite this spec and prove FR-001 through FR-011 through #1868–#1875: runner-family canonicalization/signing, browser, Node, Rust/native, Swift, Kotlin/Android, .NET, and cross-host conformance. The release PR MUST reject absent profiles, missing integrity/provenance, or a partial matrix.
