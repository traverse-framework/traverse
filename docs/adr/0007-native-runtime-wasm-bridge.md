# ADR-0007: Host One Canonical Runtime-WASM Bridge in Native Packages

- Status: Accepted
- Date: 2026-07-15

## Context

Specs 057 and 068 require Swift, Kotlin/Android, and .NET packages to run the
same runtime-owned semantics without `traverse-cli serve`. The available
language-native engines do not yet share production-ready WebAssembly
Component Model support, while all support core WebAssembly modules and linear
memory. The repository therefore needs a portable ABI and a governed engine
policy before any native package can be production-complete.

## Decision

Traverse publishes one core WebAssembly orchestrator artifact implementing
`runtime-wasm-bridge/1.0.0`. Its public exports use scalar WebAssembly values,
one exported memory, caller-allocated input bytes, and an eight-byte
little-endian `(pointer, length)` output descriptor. UTF-8 JSON is the v1 wire
format. The runtime owns all workflow, lifecycle, event, error, cancellation,
and resource-limit semantics; native adapters only marshal, verify, schedule,
and expose idiomatic APIs.

The initial host matrix is:

| Package | Engine | License | Selection rationale |
| --- | --- | --- | --- |
| Swift/iOS/macOS | WasmKit | MIT (runtime modules) | Swift Package, interpreter-safe on iOS, documented iOS 12+ support |
| Kotlin/Android | Chicory | Apache-2.0 | JVM-native interpreter with no JNI/native distribution matrix |
| .NET/WinUI | Wasmtime .NET | Apache-2.0 | Bytecode Alliance package with supported Windows native assets |

Each Traverse package release pins an exact reviewed engine version and records
its version, license, runtime-WASM digest, supported host versions, and
conformance result. Automated dependency alerts and upstream security notices
trigger review; a critical unfixed engine advisory blocks release. Engines
receive no ambient filesystem, network, clock, environment, or process access.
Only explicit bridge imports listed by the artifact manifest may be linked.

The bridge uses an engine-neutral pull operation for events. A package may turn
that ordered stream into a callback, async sequence, Flow, or observable, but
it cannot reorder, synthesize, or reinterpret events.

## Consequences

- One runtime artifact and conformance corpus govern all native packages.
- The v1 bridge works on current core-Wasm engines without waiting for uniform
  Component Model support.
- JSON and copy-based marshalling trade peak throughput for portability and a
  reviewable ownership boundary; a future ABI version may add canonical ABI
  bindings after all supported hosts can certify them.
- Platform packages carry different engine dependencies, so release evidence
  and cross-engine fixture execution are mandatory.

## Alternatives Considered

- **WebAssembly Component Model/WIT for v1**: rejected for now because support
  is incomplete in the selected Swift host and not uniform across all three.
- **One native C runtime for every platform**: rejected because it adds a large
  architecture/OS binary matrix and complicates mobile distribution.
- **Platform-native runtime implementations**: rejected because they duplicate
  runtime-owned semantics and weaken conformance.
- **Development sidecar**: rejected by Specs 057 and 068 for production use.
