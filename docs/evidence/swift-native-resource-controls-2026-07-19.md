# Swift Native Resource-Control Candidate Screen — 2026-07-19

- Issue: Traverse #762
- Governing specification: `074-swift-native-resource-control-certification` 1.0.0
- Decision: **WAMR 2.4.5 is not eligible for Swift production certification.**

## Scope and method

This is a source-and-platform-support screen, not certification evidence. It
evaluates the first alternative required by #762: WAMR interpreter mode. The
candidate must meet every requirement in Spec 074, including reproducible
physical-iOS and macOS results, before an engine-selection ADR or any #647
implementation may begin.

The screen used WAMR's official 2.4.5 release and its public embedding header:

- Release: <https://github.com/bytecodealliance/wasm-micro-runtime/releases/tag/WAMR-2.4.5>
- Public API: <https://github.com/bytecodealliance/wasm-micro-runtime/blob/WAMR-2.4.5/core/iwasm/include/wasm_export.h>
- Supported-platform statement: <https://github.com/bytecodealliance/wasm-micro-runtime#supported-architectures-and-platforms>
- License: <https://github.com/bytecodealliance/wasm-micro-runtime/blob/WAMR-2.4.5/LICENSE>

## Candidate screen

| Certification concern | Official 2.4.5 evidence | Result |
| --- | --- | --- |
| Memory-growth bound | Public `wasm_runtime_instantiation_args_set_max_memory_pages` configures an instance maximum. | API surface passes the screen; fixture evidence is still required. |
| Deterministic interruption | Public `wasm_runtime_set_instruction_count_limit` terminates execution at the instruction limit; `wasm_runtime_terminate` supports asynchronous termination. | API surface passes the screen; fixture evidence is still required. |
| macOS host | WAMR's official supported-platform list includes macOS. | Eligible for later physical-macOS testing. |
| iOS host | WAMR's official supported-platform list does **not** include iOS. | Fails the platform-support screen. |
| Apple packaging, bridge corpus, and `embedder-api/1.0.0` conformance | No reproducible iOS-device evidence exists for this candidate. | Not run; certification is impossible at this stage. |
| License | Apache-2.0 with LLVM exception. | Requires normal dependency review only if a supported Apple profile becomes available. |

## Result

WAMR's public control APIs are stronger than the rejected WasmKit SPI path,
but public controls alone do not satisfy Spec 074. The official WAMR support
matrix does not claim iOS. Therefore no physical-iOS host result, Apple bundle
evidence, or cross-engine corpus can be truthfully recorded for this profile.

The candidate is rejected before implementation. This does not select an
engine and does not create an ADR. Traverse #647 remains blocked until either
WAMR publishes supported iOS coverage with reproducible device evidence, or a
different engine demonstrates every Spec 074 requirement.

## Required evidence for a future candidate

1. Exact released engine version, license review, and public memory and
   interruption APIs.
2. Physical iOS-device and macOS runs of memory-growth and non-terminating
   fixtures, reporting `bridge_resource_limit` or `bridge_timeout`.
3. No-ambient-import validation, Apple distribution evidence, and complete
   `runtime-wasm-bridge >=1.1.0,<2.0.0` plus `embedder-api/1.0.0` corpus.
4. An approved engine-selection ADR before #647 implementation begins.
