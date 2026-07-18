# ADR-0010: Define Native Embedder Baseline 1 as Bridge 1.1 or Later

- Status: Accepted
- Date: 2026-07-18

## Context

ADR-0007 established a portable core-Wasm bridge 1.0 for native packages.
ADR-0008 added the compatible-capability lifecycle operations required for a
complete `embedder-api/1.0.0` implementation. The three native loaders now
validate bridge 1.1 exports, while the release contract needs to distinguish
the immutable 1.0 base from the complete public package baseline.

## Decision

Native Embedder Baseline 1 is `embedder-api/1.0.0` with
`runtime-wasm-bridge >=1.1.0,<2.0.0`. Package releases record both that
supported range and the exact bridge/runtime/engine/conformance inputs they
certified. A package must validate the 1.1 required exports as well as the
version range, allowing additive 1.1 patch releases without accepting a new
bridge major.

The core `runtime/runtime.wasm` bridge remains an import-free core-Wasm module.
This decision does not change the separately bounded WASI and Traverse Host ABI
services that Spec 057 applies to capability modules.

## Consequences

- Bridge 1.0 remains a valid immutable base contract but cannot certify a
  complete native `embedder-api/1.0.0` package.
- Native packages can evolve independently within their own semantic versions
  while reporting one auditable compatibility tuple.
- The shared bridge corpus and complete release evidence become required before
  declaring a cross-platform baseline release.
- Specs 071 and 072 remain historical, immutable sources of the ABI behavior;
  Spec 073 defines their release-facing composition.

## Alternatives Considered

- Treat bridge 1.0 as the release baseline: rejected because it omits the
  runtime-owned compatible lifecycle required by the public embedder API.
- Require exactly bridge 1.1.0 forever: rejected because safe 1.1 patch
  releases would be unnecessarily incompatible.
- Revise Specs 071 and 072 in place: rejected because approved governing
  artifacts are immutable and their additive history is useful.
