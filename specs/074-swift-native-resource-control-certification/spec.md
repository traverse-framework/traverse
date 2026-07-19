# Feature Specification: Swift Native Resource-Control Certification

**Feature Branch**: `codex/issue-761-swift-resource-profile`
**Created**: 2026-07-18
**Status**: Approved
**Version**: 1.0.0
**Input**: Decision 29, ADR-0011, and Traverse #761.

## Purpose

Define the certification conditions for the Swift/iOS/macOS native embedder to
join Native Embedder Baseline 1. This successor specification preserves the
immutable bridge and release-baseline history in Specs 071, 072, and 073 while
correcting the unsupported assumption that a WasmKit version increase alone
provides production resource controls.

## User Scenarios & Testing

### User Story 1 - Select a Safe Swift Runtime Profile (Priority: P1)

A downstream Apple application developer can rely on a package release only
when its selected execution engine enforces documented memory and execution
limits on supported Apple devices.

**Why this priority**: An unbounded or non-interruptible untrusted module is
not a production-safe native runtime.

**Independent Test**: Run a memory-growth fixture and a non-terminating
fixture through the package on iOS device and macOS; both fail predictably
without allowing continued unbounded execution.

**Acceptance Scenarios**:

1. **Given** a module that requests memory beyond the declared limit, **When**
   the package instantiates or grows it, **Then** it fails with the stable
   resource-limit error and does not exceed the configured bound.
2. **Given** a module that does not terminate within the declared execution
   budget, **When** the package invokes it, **Then** it is deterministically
   interrupted and reports the stable timeout or resource-limit error.

### User Story 2 - Audit the Apple Engine Choice (Priority: P2)

A release reviewer can prove that the engine controls used by a Swift package
are supported public APIs rather than internal or unstable hooks.

**Why this priority**: A version pin does not establish that a host can safely
enforce its declared limits.

**Independent Test**: Review the package source and evidence record and verify
the engine version, API support status, device results, and no-SPI policy.

**Acceptance Scenarios**:

1. **Given** release evidence that names an internal or SPI control,
   **When** certification is reviewed, **Then** the release is rejected.
2. **Given** complete evidence for a supported profile, **When** it is
   reviewed, **Then** it identifies the exact engine, public control APIs,
   Apple host matrix, and conformance results.

### Edge Cases

- An engine has a public memory limiter but no supported way to interrupt a
  running module.
- A timeout returns control to the application while the engine continues the
  untrusted computation in another thread.
- A candidate runtime works on macOS but has no reproducible physical-iOS
  device evidence.

## Requirements

### Functional Requirements

- **FR-001**: A Swift package MUST NOT certify Native Embedder Baseline 1
  unless its runtime profile enforces both bounded memory growth and
  deterministic execution interruption through documented, supported public
  engine APIs.
- **FR-002**: Certification MUST prove a module exceeding the configured
  memory limit fails deterministically without exceeding that limit.
- **FR-003**: Certification MUST prove a non-terminating module is stopped
  within the configured execution budget and reports a stable
  `bridge_timeout` or `bridge_resource_limit` error.
- **FR-004**: A profile MUST NOT depend on `@_spi`, undocumented APIs, a
  watchdog that leaves the untrusted execution alive, or an equivalent
  unsupported escape hatch.
- **FR-005**: Certification evidence MUST include the exact engine version and
  license, the public APIs relied upon, configured limits, iOS-device and
  macOS results, bridge corpus result, and `embedder-api/1.0.0` conformance
  result.
- **FR-006**: An engine that satisfies only one required control remains
  ineligible for production certification. The package MAY remain a
  development or test harness, but MUST NOT claim a cross-platform baseline.
- **FR-007**: Replacing the Swift engine requires an approved ADR, renewed
  license/security review, bundle-distribution evidence, and the complete
  cross-engine corpus before #647 implementation begins.
- **FR-008**: WasmKit 0.3.1 is not by itself a certified production profile:
  its `Store.resourceLimiter` SPI and lack of supported interruption controls
  do not meet FR-001.

### Key Entities

- **Swift Runtime Profile**: The exact engine, public controls, host matrix,
  and evidence used by the Apple native embedder.
- **Resource-Control Evidence**: Reproducible proof of memory and execution
  bounds on both supported Apple host classes.
- **Certified Profile**: A Swift runtime profile satisfying every requirement
  in this specification and the bridge/API contracts it composes.

## Success Criteria

### Measurable Outcomes

- **SC-001**: Every certified profile passes both memory-exhaustion and
  non-termination fixtures on physical iOS and macOS with a stable error.
- **SC-002**: Every certified release record identifies 100% of its engine
  controls as supported public APIs.
- **SC-003**: No package that lacks either control is described as a
  cross-platform Native Embedder Baseline release.
- **SC-004**: A reviewer can reproduce each evidence result from the recorded
  engine version, host matrix, limits, and fixture commands.

## Assumptions

- Specs 071, 072, and 073 remain immutable sources of bridge and release
  compatibility history.
- Kotlin and .NET native profiles can progress independently but do not make a
  release cross-platform while the Swift profile remains uncertified.
- #762 evaluates supported options before an engine replacement is proposed.

## Out of Scope

- Choosing or implementing an alternative Swift engine.
- Relaxing the core-Wasm no-ambient-import boundary in Spec 071.
- Changing the public `embedder-api/1.0.0` operation set.
- Releasing #647 before the certification evidence exists.

## Implementation Tickets

- Traverse #761 — this governing specification and ADR.
- Traverse #762 — supported-engine evaluation spike.
- Traverse #647 — Apple package implementation after a certified profile.
- Traverse #750 and #758 — artifact and cross-host certification delivery.
