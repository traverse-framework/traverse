# ADR-0011: Distribute the Native Runtime Artifact Through Traverse's Existing Registry Infrastructure

- Status: Proposed
- Date: 2026-07-18

## Context

Traverse #750 and #755 need a way for Swift, Kotlin, and .NET packages to
acquire one canonical, digest-pinned `runtime.wasm` build without an
HTTP/CLI sidecar and without three independently drifting per-package
publish pipelines. Traverse already operates publish/resolve infrastructure
for capability, event, and workflow records, including semver-range
resolution (Spec 051's `traverse-registry`, currently `crates/traverse-registry`
and moving to `traverse-framework/registry`). No equivalent mechanism yet
exists for a cross-language binary runtime artifact.

## Decision

Reuse the registry's existing publish/resolve model for a new
"runtime artifact" record kind carrying identity (`runtime_version`,
certified `bridge_version`, SHA-256 digest), supported bridge range, and
per-host certification evidence, rather than building a bespoke
native-artifact distribution channel.

Native packages resolve this record at package build/release time, not at
process runtime, preserving the no-sidecar rule already required by Specs
057, 068, and 071, and fetch the referenced content-addressed artifact bytes
for embedding into the package's own release bundle. Implementation
(Traverse #757) lands wherever Spec 051's registry crate is authoritative
when it is built; this ADR does not change that migration's timeline or
scope.

## Consequences

- One publish path and one semver/compatibility resolver serve capability,
  event, workflow, and now runtime-artifact records — no parallel
  distribution stack to design, review, or maintain.
- Registry resolution happens at package build/release time, so runtime
  instantiation itself never depends on network or registry availability,
  preserving the sidecar-free acceptance criteria in Specs 057, 068, and 071.
- Couples the runtime-artifact distribution contract's implementation
  timeline to Spec 051's registry-extraction migration; Traverse #757 must
  target whichever repository is authoritative for `traverse-registry` when
  it is implemented.
- Physical byte storage for the artifact is left to whatever
  content-addressed asset mechanism the registry already uses or expects for
  other published artifacts, avoiding a new storage integration.

## Alternatives Considered

- **Bespoke per-package artifact channel**: each of the Swift, Kotlin, and
  .NET packages independently vendors or downloads `runtime.wasm` from a
  dedicated release page — rejected because it triples the publish/verify
  logic and risks the three packages drifting to different runtime
  identities.
- **Runtime-time HTTP fetch from a registry endpoint**: resolve and fetch the
  artifact when the bridge initializes — rejected because it reintroduces
  the sidecar/network dependency Specs 057, 068, and 071 explicitly forbid
  for production native hosts.
- **Publish `runtime.wasm` through this repository's crates.io pipeline**:
  reuse `048-semver-publishing-pipeline` — rejected because crates.io
  distributes Rust source crates, not a cross-language consumable Wasm
  binary with per-host certification metadata; it does not fit that
  pipeline's shape.

## Approval Note

This ADR accompanies draft spec `074-native-runtime-distribution-contract`
and is Proposed, not Accepted, pending the same explicit human approval
required by Traverse #755's Definition of Done. It should move to Accepted
alongside that spec's approval, with a corresponding decision-log entry
added at that time.
