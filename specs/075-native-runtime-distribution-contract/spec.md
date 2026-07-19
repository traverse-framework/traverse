# Feature Specification: Native Runtime Distribution Contract

**Feature Branch**: `075-native-runtime-distribution-contract`
**Created**: 2026-07-18
**Status**: Approved
**Version**: 1.0.0
**Input**: Decision 30, ADR-0012, and Traverse #755, parent #750, Specs 057,
068, 071, 072, 073, and ADR-0007.

## Purpose

Define the versioned distribution contract for the production native-runtime
artifact (`runtime.wasm`): how one governed build becomes an identified,
digest-pinned, host-certified release that every public embedder package and
reference app acquires, resolves, and upgrades, with no host-specific API
surface.

This spec sits beneath the ABI and compatibility contracts already approved
in Specs 071, 072, and 073 and above their release mechanics. It does not
change bridge semantics or the Native Embedder Baseline. It defines how the
one canonical build travels from a single build step to Swift, Kotlin, and
.NET consumers. Implementation is tracked separately by Traverse #756
(production build), #757 (registry-facing publication and resolution), and
#758 (cross-host conformance and release-smoke coverage), all blocked on this
spec.

## User Scenarios & Testing

### User Story 1 - Resolve a Certified Runtime Artifact for a Host Package (Priority: P1)

A native package build resolves exactly one runtime artifact release that
satisfies its declared bridge compatibility range, verifies its digest, and
either accepts a fully certified artifact or fails deterministically before
instantiation.

**Why this priority**: Without deterministic resolution and rejection, a
package could silently ship an unverified, incompatible, or uncertified
runtime — the exact failure mode Specs 071 and 073 already forbid at the ABI
layer, but that distribution/acquisition itself must also close.

**Independent Test**: Resolve against fixture metadata records covering a
matching case and mismatched-digest, incompatible-bridge, and
uncertified-host cases.

**Acceptance Scenarios**:

1. **Given** a published artifact whose digest matches its metadata record,
   **When** a package resolves it, **Then** resolution succeeds and returns
   that exact artifact reference.
2. **Given** a metadata record whose digest does not match the fetched
   bytes, **When** a package resolves it, **Then** resolution fails with a
   tamper error before any bridge instantiation.
3. **Given** a package requiring `runtime-wasm-bridge >=1.1.0,<2.0.0`,
   **When** it resolves an artifact certified only at bridge 1.0.0, **Then**
   resolution fails with `bridge_version_mismatch`.
4. **Given** an artifact with no certification evidence for the requesting
   host profile, **When** a package resolves it, **Then** resolution fails
   with a deterministic uncertified-host error rather than falling back
   silently.

---

### User Story 2 - Upgrade a Package to a Newer Runtime Release (Priority: P2)

A package maintainer moves to a newer runtime release without changing
package-facing API, by pointing distribution resolution at a new immutable
artifact identity while the previous release remains independently
resolvable for rollback and audit.

**Why this priority**: Releases must be independently addressable and
immutable, or rollback, audit, and staged multi-host rollout all become
unreliable.

**Independent Test**: Publish two artifact releases with different
`runtime_version` values and resolve each independently after the second is
published.

**Acceptance Scenarios**:

1. **Given** two published artifact releases with different
   `runtime_version` values, **When** either is resolved by exact identity,
   **Then** both remain independently resolvable.
2. **Given** a package upgrade to a new compatible release, **When** a
   reviewer inspects the previous release's evidence and digest, **Then**
   both remain unchanged and resolvable for rollback or audit.
3. **Given** an attempted republish that reuses an existing `runtime_version`
   with different bytes, **When** publication is attempted, **Then** it is
   rejected.

---

### User Story 3 - Audit Cross-Host Distribution Without Host-Specific Extensions (Priority: P3)

A release reviewer or host implementer confirms that Swift, Kotlin, and .NET
consume the identical distribution metadata schema, with no host-specific
required fields, so the embedder-api surface stays uniform.

**Why this priority**: A host-specific metadata field would let one platform
silently diverge from the others' compatibility and evidence guarantees,
undermining Spec 073's cross-platform baseline.

**Independent Test**: Resolve the same metadata record through all three
declared host profiles and compare the fields consumed.

**Acceptance Scenarios**:

1. **Given** the same metadata record, **When** each of the three host
   profiles resolves it, **Then** identity, digest, compatibility range, and
   certification evidence are read through the same field names.
2. **Given** a proposed host-specific metadata field, **When** it is reviewed
   against this contract, **Then** it is rejected as out-of-contract unless
   added for all hosts.

### Edge Cases

- A metadata record references a digest that was valid at publish time, but
  the artifact storage now serves corrupted or truncated bytes (an
  integrity failure at fetch time, not just at publish time).
- Two releases are certified for overlapping bridge ranges but different
  engine floors (for example, Decision 27's WasmKit 0.3.1 bump) — resolution
  must honor the requesting host's declared floor, not simply the latest
  release.
- A host resolves an artifact identity that exists in metadata but was never
  fully uploaded (a partially failed publish).

## Requirements

### Functional Requirements

- **FR-001**: Every published runtime artifact release MUST be identified by
  an immutable tuple of `runtime_version` (semver), the exact certified
  `bridge_version`, and a SHA-256 digest of the artifact bytes. Once
  published, an identity's digest MUST NOT change.
- **FR-002**: A release's distribution bundle MUST retain the immutable
  bridge-manifest layout required by Spec 071 FR-001 (`runtime.wasm`, bridge
  manifest(s), digest manifest) plus a machine-readable release-evidence
  record satisfying Spec 073 FR-004.
- **FR-003**: Resolution MUST verify a fetched artifact's SHA-256 digest
  against its published metadata before instantiation; a mismatch MUST fail
  deterministically as a tamper error with no fallback.
- **FR-004**: Resolution MUST reject an artifact whose certified
  `bridge_version` falls outside a package's declared supported range (per
  Spec 073's Native Embedder Baseline) with `bridge_version_mismatch`, before
  execution.
- **FR-005**: Resolution MUST reject an artifact that lacks certification
  evidence for the requesting host profile (package and engine) with a
  deterministic uncertified-host error rather than silently accepting an
  uncertified artifact.
- **FR-006**: Each published release MUST remain independently resolvable by
  its exact `runtime_version` identity after a newer release is published, so
  packages can pin, audit, or roll back without losing access to prior
  evidence.
- **FR-007**: A publish operation that reuses an existing `runtime_version`
  with different artifact bytes MUST be rejected; a corrected build MUST
  publish under a new version.
- **FR-008**: The distribution metadata schema MUST be host-agnostic: Swift,
  Kotlin, and .NET consumers MUST resolve identity, digest, compatibility
  range, and certification evidence through the same field names, with no
  host-specific required fields.
- **FR-009**: Distribution resolution and publication MUST NOT introduce a
  network or HTTP sidecar dependency at runtime; verification and rejection
  MUST occur using locally resolvable metadata and bundled artifacts,
  consistent with the no-sidecar requirement in Specs 057, 068, and 071.

### Key Entities

- **Runtime Artifact Release**: An immutable, digest-identified
  `runtime.wasm` build plus its bridge manifest(s) and release evidence,
  published exactly once per `runtime_version`.
- **Distribution Metadata Record**: The registry-facing record binding a
  release's identity, digest, supported bridge range, and per-host
  certification evidence, resolved by native packages (implemented by
  Traverse #757).
- **Host Certification Evidence**: The per-package-profile (Swift/WasmKit,
  Kotlin/Chicory, .NET/Wasmtime) conformance and resource-control results a
  release must carry before a host profile may resolve it (Spec 073 FR-004,
  FR-006).

## Success Criteria

### Measurable Outcomes

- **SC-001**: Every native package resolves a runtime artifact using only the
  fields defined by this contract, with zero host-specific required fields,
  verified by spec-alignment review across all three host packages.
- **SC-002**: 100% of tamper (digest mismatch), incompatible-bridge, and
  uncertified-host resolution attempts fail deterministically before bridge
  instantiation, with no sidecar fallback.
- **SC-003**: Every published release remains independently resolvable by
  identity after at least one newer release is published.
- **SC-004**: A republish attempt that reuses an existing `runtime_version`
  with changed bytes is rejected in 100% of tested cases.

## Assumptions

- Specs 057, 068, 071, 072, and 073 remain immutable governing artifacts;
  this spec adds a distribution layer beneath them without revising their
  ABI or compatibility semantics.
- The distribution metadata mechanism reuses Traverse's existing registry
  publish/resolve infrastructure (Spec 051's `traverse-registry`, moving to
  `traverse-framework/registry`) rather than introducing a new distribution
  channel; see ADR-0012.
- The artifact bytes may be hosted by any content-addressed storage the
  registry metadata references; this spec constrains identity and
  verification, not the physical storage choice.

## Out of Scope

- The production build implementation of `runtime.wasm` (Traverse #756).
- The concrete registry publish/resolve implementation (Traverse #757).
- End-to-end host certification test coverage (Traverse #758).
- Any new public `embedder-api` version or bridge ABI change.
- Physical artifact storage/hosting technology selection beyond
  "content-addressed, digest-verifiable."

## Implementation Tickets

- Traverse #755 — this distribution-contract specification.
- Traverse #756 — digest-pinned production bridge artifact build.
- Traverse #757 — registry-facing metadata publication and resolution.
- Traverse #758 — real-artifact conformance and release-smoke coverage
  across hosts.
