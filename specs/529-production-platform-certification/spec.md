# Feature Specification: Production Platform Certification

**Status**: Approved (2026-09-13)
**Canonical governing ID**: `529-production-platform-certification`
**Version**: 1.0.0
**Input**: Traverse #849; #1369; Decision 38; Decision 83;
`524-production-app-readiness`; Specs
`073-native-embedder-release-baseline`, `074-swift-native-resource-control-certification`,
`075-native-runtime-distribution-contract`, `076-production-swift-wasmi-cabi`,
`080`, `068-public-platform-embedder-packages`, and
`136-native-embedder-publication`.

## Purpose

Define the single, host-neutral conformance decision that classifies the Web,
Linux/Rust, Apple, Android, and Windows/.NET embedded packages as Certified or
Preview for a specific immutable runtime and locked application generation.

## Relationship to Shipped

**Shipped** is Spec 068 / Spec 136 public-package completeness: a stranger can
depend on the host package from the language-default registry (crates.io, npm,
Maven Central, nuget.org, or the GitHub-release XCFramework) and a reference
app can execute without a sidecar.

**Certified** and **Preview** are this spec's classification only. This spec
is not a Shipped gate and MUST NOT be cited to hold a platform at Blocked or
In progress when Spec 068 / Spec 136 is met.

A host that has no Maven, NuGet, or current XCFramework publish, or that lacks
a required matrix result, is **Preview** (`platform_conformance_missing` or
`platform_conformance_failed`). It is never Blocked by this spec. Edge is not
in the Certified target set. Implementing the five-platform runner is a later
ticket; approving this spec does not authorize host-package work.

## Capability Boundary

**Production-platform conformance evaluation** consumes versioned fixtures,
an exact runtime artifact, a locked dependency generation, and recorded host
results. It produces a reproducible classification and safe evidence. It does
not implement host packages, run a registry, prepare cache bytes, or change a
release's declared compatibility.

## Requirements

- **FR-001**: The Certified target set is exactly Web, Linux/Rust, Apple,
  Android, and Windows/.NET. A platform is Certified only when every target in
  that set passes the same versioned conformance suite for the same runtime
  digest, bridge version, lockfile schema, and fixture corpus.
- **FR-002**: The common suite MUST exercise embedded initialization from a
  verified local cache, execution of a locked capability without network use,
  cache-generation activation and explicit rollback, durable safe-trace
  readability after restart, and configured resource-control rejection.
- **FR-003**: Each result MUST bind platform and host-package version, OS and
  architecture, engine/version, runtime and artifact digests, bridge version,
  lockfile digest, suite/fixture version, execution timestamp, and pass/fail
  outcome. Evidence MUST NOT contain raw requests, raw outputs, credentials,
  cache roots, or encryption keys.
- **FR-004**: A host that lacks a required result, has a failing result, uses
  a mismatched artifact/fixture identity, or cannot enforce the required
  resource controls MUST be classified `preview`, never implicitly Certified.
- **FR-005**: Certification evidence is immutable per release candidate.
  Changing the runtime digest, bridge version, host engine, suite version, or
  locked-generation schema requires a new evidence set and classification.
- **FR-006**: A Certified classification MUST fail closed if cache initialization
  or execution attempts runtime network access, including a fallback network
  request after local lookup failure.
- **FR-007**: Activation tests MUST prove that a candidate generation is
  verified before atomic activation, the prior active generation remains
  available for explicit rollback, and a failed activation preserves the
  prior runnable state.
- **FR-008**: Trace-restart tests MUST prove that only safe workspace-authorized
  evidence survives restart, retention/pruning evidence remains inspectable,
  and raw request/output payloads are absent.
- **FR-009**: Resource tests MUST prove bounded rejection for over-limit memory,
  non-terminating or exhausted execution, oversized input/output, and forbidden
  ambient imports using the stable errors governed by the host contracts.
- **FR-010**: Stable classification errors MUST include
  `platform_conformance_missing`, `platform_conformance_failed`,
  `platform_evidence_mismatch`, `platform_runtime_network_forbidden`, and
  `platform_resource_control_missing`.

## Acceptance Scenarios

1. Given one passing evidence set for each target platform and identical
   versioned inputs, when release certification runs, then the release is
   Certified with a complete safe evidence matrix.
2. Given an Android result is absent or failing, when certification runs, then
   the release is Preview with `platform_conformance_missing` or
   `platform_conformance_failed`; other passing platforms do not compensate.
3. Given a host initializes a valid locked bundle offline, when it executes a
   fixture, then the suite records no runtime network access on every target.
4. Given a failed cache-generation activation, when the host is restarted,
   then the prior generation runs and the candidate remains inactive.
5. Given a trace-bearing execution, when the host restarts, then the suite
   reads only authorized safe trace evidence and verifies retention/pruning.
6. Given a changed runtime digest or fixture corpus, when prior evidence is
   offered, then certification rejects it with `platform_evidence_mismatch`.

## Governed Files and Conformance

This successor governs future common-fixture and matrix definitions under
`packages/{web,swift,kotlin,dotnet}/TraverseEmbedder/`, the Rust embedded-host
conformance path, `scripts/ci/embedder_conformance/`, native certification CI,
and release-evidence schemas/tests. Independent validation MUST execute each
acceptance scenario across all five targets and publish only safe matrix data.

## Compatibility and Scope

Existing package/bridge APIs remain unchanged. This is additive release
evidence; an existing host remains usable but is Preview unless it satisfies
this exact suite. Registry tier admission, cache implementation, durable trace
storage, and DataStore migration are governed by separate successors.

## Out of Scope

- Implementing or changing any host package, runtime bridge, or CI runner.
- Registry publisher trust, lock resolution, cache storage, trace persistence,
  DataStore migration, and platform-specific engine selection.
- Declaring a platform Certified from simulated or partial results.
- Using this spec as the website or Spec 068/136 Shipped gate.
- Edge host adapters and cloud / multi-cloud placement.
