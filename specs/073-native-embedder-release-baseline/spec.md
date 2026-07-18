# Feature Specification: Native Embedder Release Baseline

**Feature Branch**: `516-native-bridge-baseline`
**Created**: 2026-07-18
**Status**: Approved
**Version**: 1.0.0
**Input**: Decision 28, ADR-0010, and Traverse #752.

## Purpose

Define the release-facing compatibility baseline for a production native
embedder. This specification reconciles the public `embedder-api/1.0.0`
surface in Specs 057 and 068 with the immutable runtime-WASM bridge 1.0 base
in Spec 071 and its compatible-capability lifecycle extension in Spec 072.

Native Embedder Baseline 1 is the compatibility tuple:

```text
embedder-api/1.0.0 + runtime-wasm-bridge >=1.1.0,<2.0.0
```

It does not replace or revise Specs 057, 068, 071, or 072.

## User Scenarios & Testing

### User Story 1 - Select a Complete Native Bundle (Priority: P1)

A downstream application developer selects a native package and an
application-owned bundle knowing whether the package can implement every
`embedder-api/1.0.0` operation without a sidecar.

**Why this priority**: A bundle that lacks runtime-owned compatible lifecycle
semantics cannot meet the public embedder contract.

**Independent Test**: Initialize a complete native package with bridge 1.0,
bridge 1.1, and bridge 2.0 fixtures; only the bridge 1.1 fixture succeeds.

**Acceptance Scenarios**:

1. **Given** a bundle declaring bridge 1.0, **When** a package claims complete
   `embedder-api/1.0.0` conformance, **Then** initialization fails with
   `bridge_version_mismatch` before execution or fallback.
2. **Given** a bundle declaring bridge 1.1.x and all required exports,
   **When** the package initializes it, **Then** it accepts the bundle and
   exposes all public lifecycle operations.
3. **Given** a bundle declaring bridge major version 2, **When** the package
   initializes it, **Then** it fails deterministically with
   `bridge_version_mismatch`.

### User Story 2 - Audit a Native Package Release (Priority: P2)

A release reviewer can connect a published native package to the exact runtime
artifact, bridge compatibility range, engine profile, and conformance result.

**Why this priority**: Package semantic versions alone cannot prove that a
downstream binary hosts the intended runtime or resource-control profile.

**Independent Test**: Inspect a release-evidence record and verify every
required identity, compatibility, host, engine, resource-control, and
conformance field without consulting private build state.

**Acceptance Scenarios**:

1. **Given** a proposed package release, **When** its evidence lacks an exact
   runtime digest, certified bridge version, or engine version, **Then** it is
   not eligible to claim Native Embedder Baseline 1.
2. **Given** complete evidence, **When** a reviewer compares it with the
   bundled runtime, **Then** the runtime digest and certified bridge version
   match exactly.

### User Story 3 - Preserve the Runtime and Capability Boundaries (Priority: P3)

A native-host implementer can distinguish the isolated core-Wasm bridge module
from separately sandboxed capability modules.

**Why this priority**: Treating the bridge's no-import rule as a ban on the
capability host would incorrectly weaken the WASI and Traverse Host ABI model
required by Spec 057.

**Independent Test**: Run the bridge module without ambient imports and run a
declared capability only through the bounded capability-host profile.

**Acceptance Scenarios**:

1. **Given** `runtime/runtime.wasm`, **When** a package validates the bridge,
   **Then** it rejects undeclared imports before instantiation.
2. **Given** a bundled capability module, **When** it executes through the
   embedded runtime, **Then** its allowed WASI and Traverse Host ABI services
   remain bounded by Spec 057 rather than becoming ambient bridge imports.

### Edge Cases

- A bridge reports a compatible ABI integer but omits a mandatory 1.1 export.
- A 1.1 patch release adds optional exports but preserves the required 1.1
  semantics.
- A release has a correct runtime digest but was certified with an unreviewed
  engine or undisclosed resource-control limitation.

## Requirements

### Functional Requirements

- **FR-001**: A package claiming complete `embedder-api/1.0.0` conformance
  MUST require `runtime-wasm-bridge >=1.1.0,<2.0.0` and MUST reject bridge 1.0
  or a different bridge major with `bridge_version_mismatch` before execution,
  network access, or sidecar fallback.
- **FR-002**: The bridge ABI integer MUST encode semantic versions as
  `major * 10000 + minor * 100 + patch`. A complete package MUST accept the
  inclusive integer range `10100..19999` only after validating every mandatory
  bridge 1.1 export and signature.
- **FR-003**: A complete native bundle MUST retain the immutable bridge 1.0
  base manifest required by Spec 071, include the selected 1.x extension
  manifest, and declare its exact selected bridge version in bundle metadata.
- **FR-004**: Release evidence MUST contain package semantic version, source
  release identifier, runtime SHA-256 digest, supported bridge range, exact
  certified bridge version, embedder API and conformance versions, engine
  name/version/license, host platform and architecture, configured resource
  limits and disclosed unsupported controls, and the results of both bridge
  and embedder conformance.
- **FR-005**: A package MAY adapt ordered runtime events into an idiomatic
  language mechanism, but MUST preserve all runtime-owned lifecycle, output,
  error, and ordering semantics required by Specs 057 and 068.
- **FR-006**: The same digest-addressed bridge conformance corpus MUST execute
  through each declared Swift, Kotlin, and .NET engine profile before a
  cross-platform baseline release is certified.
- **FR-007**: `runtime/runtime.wasm` MUST remain a no-ambient-import core-Wasm
  bridge under Spec 071. Capability WASI and Traverse Host ABI services remain
  a separate bounded capability-host responsibility under Spec 057.
- **FR-008**: The approved-spec registry path guard MUST remain enabled so a
  release cannot claim this or any other approved spec whose declared path is
  absent from the repository.

### Key Entities

- **Native Embedder Baseline**: The release compatibility tuple relating the
  public embedder API to a supported bridge ABI range.
- **Release Evidence**: The auditable record binding a package release to its
  runtime artifact, host engine, limits, and conformance results.
- **Bridge Extension Manifest**: The immutable ABI manifest that adds a
  compatible 1.x capability to the bridge 1.0 base.

## Success Criteria

### Measurable Outcomes

- **SC-001**: Every package claiming complete native embedder conformance
  accepts a valid 1.1.x fixture and rejects 1.0 and 2.0 fixtures before any
  execution begins.
- **SC-002**: A reviewer can determine the exact runtime, bridge, engine, and
  conformance inputs for 100% of baseline-certified package releases from
  their evidence record.
- **SC-003**: The shared bridge corpus produces the same required transcript
  across all three declared native engine profiles.
- **SC-004**: The repository check reports any approved registry path missing
  from disk in the same validation run.

## Assumptions

- Specs 057, 068, 071, and 072 remain immutable governing artifacts.
- Native package versions remain independently semver-versioned; this baseline
  does not require lockstep package publication.
- A cross-platform baseline release is withheld until every declared engine
  profile, including Swift resource controls, meets the recorded evidence and
  conformance requirements.

## Out of Scope

- A new public `embedder-api` version.
- Replacing the core-Wasm bridge with the WebAssembly Component Model.
- Platform-owned workflow, compatible-lifecycle, or output semantics.
- Production artifact implementation, public-event API parity, or Swift engine
  upgrade work, which remain respectively tracked by #750, #751, and #647.

## Implementation Tickets

- Traverse #752 — baseline specification, decision record, and reconciliation.
- Traverse #750 — production bridge artifact and consumer release evidence.
- Traverse #751 — native public event/output parity.
- Traverse #647 — Swift production resource controls.
