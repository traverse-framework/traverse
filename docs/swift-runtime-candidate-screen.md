# Swift Runtime Candidate Screen

**Date**: 2026-07-19  
**Issue**: Traverse #766  
**Governing specs**: `074-swift-native-resource-control-certification`,
`073-native-embedder-release-baseline`

## Decision

No evidence-ready Swift runtime candidate is currently available for Native
Embedder Baseline 1. #647 remains blocked. A successor ADR and physical iOS /
macOS certification run must not begin until an engine exposes both required
controls as documented, supported public APIs.

## Screened candidates

| Candidate | Version screened | License | Apple distribution and public controls | Result |
| --- | --- | --- | --- | --- |
| [WasmKit](https://github.com/swiftwasm/WasmKit) | 0.3.1 | MIT | Official Swift package with iOS 12+ and macOS support. `Store.resourceLimiter` is `@_spi(Fuzzing)` and no public fuel, epoch, deadline, or interruption API exists. | Rejected: fails Spec 074 FR-001 and FR-004. |
| [WAMR](https://github.com/bytecodealliance/wasm-micro-runtime) interpreter | 2.4.5 | Apache-2.0 | The completed #762 screen found no official iOS support suitable for the required physical-device certification. | Rejected: fails the supported Apple host evidence requirement. |
| [Wasmtime](https://github.com/bytecodealliance/wasmtime) | 44.x | Apache-2.0 | It provides resource controls, but is an optimizing JIT runtime. Its Apple support needs executable-memory/JIT entitlement handling and has no documented certified iOS Swift-package profile. | Rejected: JIT/entitlement-dependent profile and no iOS certification evidence. |
| [Wasmer](https://github.com/wasmerio/wasmer) | 6.x | MIT | The upstream project does not publish a Swift package; its runtime profiles are not documented as an iOS interpreter with supported interruption controls. | Rejected: no supported Swift/iOS profile. |

## Evidence and reproduction plan

The rejection evidence is the linked upstream public project documentation and
the repository records in ADR-0011, Spec 074, and #762. Re-open this screen
only when an upstream release documents its exact version, license, iOS and
macOS package support, public memory-growth limiter, and public interruption
mechanism.

For a future candidate, run on a physical iOS device and macOS with:

1. a core-Wasm module whose `memory.grow` exceeds the configured bound;
2. a non-terminating core-Wasm module under the configured execution budget;
3. the digest-addressed bridge corpus and `embedder-api/1.0.0` conformance.

Both negative fixtures must return a stable `bridge_resource_limit` or
`bridge_timeout` result and must leave no untrusted execution alive.

## Next step

Do not create a successor engine-selection ADR or start certification. Track
an upstream release that satisfies every prerequisite, then repeat this screen
with device evidence and a renewed license/security review.
