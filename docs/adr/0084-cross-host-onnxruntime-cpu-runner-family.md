# ADR-0084: Govern ONNX Runtime as One Cross-Host CPU Provider

- Status: Accepted
- Date: 2026-11-16
- Governing spec: `1774-onnxruntime-cpu-runner-family`
- Related: Decision 134; ADR-0074; Spec 138; #1859; #1866
- Approval: Enrico Piovesan, recorded in #1866

## Context

Large ONNX graphs can exceed the current `tract-onnx` compatibility or resource envelope. A downstream application exposed the gap, but Traverse is a portable platform and must not turn that application or a browser runtime into product scope.

Browser ORT Web/WASM and native ONNX Runtime have different binary formats, isolation primitives, and resource controls. Requiring one browser artifact on every host is not portable; allowing each application or SDK to adopt ORT directly would fragment trust, limits, evidence, and releases.

## Decision

1. Traverse exposes one provider-neutral `onnxruntime-cpu` capability across browser, Node, Rust/native, Swift, Kotlin/Android, and .NET.
2. A signed runner-family manifest releases the exact target artifacts as one coherent provenance, revocation, and conformance unit. Applications never select the target artifact.
3. Browser and Node use terminable host isolation appropriate to their platforms. Native hosts use one Traverse-owned audited native runner boundary. All profiles implement the same public contract.
4. Runner activation is verified before registration and inference is offline. Every profile enforces a signed fixed memory profile and deterministic cancellation/deadline cleanup.
5. The initial release is CPU-only and requires the complete host matrix. Accelerator profiles and product-model approvals remain separate decisions.

## Consequences

- A browser-only ORT release is prohibited.
- Every target artifact, build recipe, license/provenance record, and conformance result is release-gated.
- Host-specific implementation detail is isolated behind placement and evidence, preserving application portability.
- The implementation cost is higher than a web-only integration, but supported-host behavior remains honest and testable.

## Alternatives considered

- **Browser-only ORT Web release:** rejected because it makes one downstream workaround a platform feature and leaves native products behind.
- **Direct ORT integration in each SDK:** rejected because verification, limits, cancellation, and provenance would drift.
- **One Emscripten/ORT-WASM artifact on native hosts:** rejected because its JavaScript runtime and Worker assumptions are not a native execution boundary.
- **Per-model bundled runtimes:** rejected because security, licensing, and revocation updates require repackaging every model.
