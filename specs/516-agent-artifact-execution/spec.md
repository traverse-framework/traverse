# Feature Specification: Production Agent Artifact Execution

**Feature Branch**: `516-agent-artifact-execution`  
**Created**: 2026-07-21  
**Status**: Approved  
**Input**: Approved Project 1 decisions for governed agent artifacts.

## Purpose

Make `agent execute` a truthful production entry point for every shipped
governed agent package. It must execute the verified package artifact rather
than substitute capability-specific host behavior.

This successor specification complements immutable Spec 064; it does not amend
or weaken that specification.

## User Scenarios & Testing

### User Story 1 - Execute a shipped agent package (Priority: P1)

As a package consumer, I can execute any shipped governed agent package through
`agent execute` and receive that package's valid JSON result.

**Why this priority**: Package verification is meaningful only when the
verified artifact is the artifact that executes.

**Independent Test**: Execute each shipped package with its canonical runtime
request through the CLI and verify a completed result produced by the package.

**Acceptance Scenarios**:

1. **Given** a shipped package with a valid artifact and request, **When** it
   is run through `agent execute`, **Then** the production artifact is executed
   and the CLI returns its JSON result.
2. **Given** a package artifact that cannot produce valid JSON output, **When**
   it is executed, **Then** the CLI returns the stable invalid-output failure.

### User Story 2 - Preserve declared isolation (Priority: P1)

As an operator, I can trust a package declaring the reviewed portability
exception `host_api_access: exception_required` to receive only the narrowly
declared output capability and no other host access.

**Independent Test**: Validate and execute the rebuilt doc-approval package;
its artifact imports only `wasi_snapshot_preview1::fd_write` for stdout and
its constraint declaration explicitly allows that limited capability.

**Acceptance Scenarios**:

1. **Given** a package declaring `exception_required`, **When** its artifact imports
   any host capability other than `wasi_snapshot_preview1::fd_write`, **Then**
   validation rejects it before execution.
2. **Given** a package declaring `exception_required` and importing only `fd_write`,
   **When** it is executed, **Then** it completes with no filesystem,
   environment, network, clock, or other host access.

### User Story 3 - Adopt corrected artifacts deliberately (Priority: P2)

As a package consumer, I can identify the corrected executable artifacts and
adopt them without ambiguous replacement of earlier package versions.

**Independent Test**: Inspect the rebuilt package metadata and verify a minor
version increment with the prior package version still identifiable.

## Edge Cases

- A placeholder or no-output artifact MUST not be treated as a successful
  shipped executable package.
- An artifact with an undeclared host import MUST fail before invocation.
- A malformed artifact, invalid ABI, or invalid JSON output MUST preserve the
  stable failure classifications defined by Spec 064.
- A package missing its canonical request fixture MUST fail its shipping
  validation rather than be omitted from coverage.

## Requirements

### Functional Requirements

- **FR-001**: `agent execute` MUST route every package through the production
  artifact execution boundary defined by Spec 064.
- **FR-002**: Shipped agent packages MUST contain real executable artifacts
  that write exactly one valid JSON result for their canonical request.
- **FR-003**: A package declaring `host_api_access: exception_required` MUST
  cite the stdout-only portability exception. It MAY import only
  `wasi_snapshot_preview1::fd_write`, solely to write its one JSON result to
  stdout. It MUST NOT receive filesystem, environment, network, clock, or any
  other WASI or host capability.
- **FR-004**: The rebuilt `doc-approval.analyze` package MUST declare the
  reviewed portability exception and execute using only that constrained output
  import.
- **FR-005**: Every replacement shipped artifact/package MUST receive a minor
  version increment; prior versions remain distinguishable migration history.
- **FR-006**: CI MUST execute every shipped agent package through `agent
  execute` using a canonical request fixture.
- **FR-007**: CI MUST fail when a shipped package has a placeholder artifact,
  invalid ABI/import profile, invalid JSON output, or no canonical request.
- **FR-008**: Existing stable artifact execution failure classifications MUST
  remain compatible with Spec 064.

### Key Entities

- **Shipped agent package**: A governed package distributed with Traverse and
  validated by the package execution suite.
- **Canonical request fixture**: The stable request used to prove one package's
  production execution behavior.
- **Artifact compatibility version**: The minor-incremented package version
  identifying a corrected executable artifact.

## Success Criteria

- **SC-001**: 100% of shipped agent packages execute successfully through the
  production CLI path in CI.
- **SC-002**: 100% of packages declaring the stdout-only portability exception pass ABI validation
  allowing only `wasi_snapshot_preview1::fd_write` before execution.
- **SC-003**: Every corrected package metadata record has a minor version
  increment and a canonical execution fixture.
- **SC-004**: No capability-specific demonstration executor is reachable from
  the default `agent execute` path.

## Assumptions

- Spec 064 remains the governing production-router boundary and error taxonomy.
- Package contracts remain semver-compatible; this slice changes artifact
  executability and package versions, not their business contracts.
- A separate implementation ticket will deliver the artifact rebuilds and CLI
  routing after this specification is formally approved.

## Out of Scope

- Adding a demo-only execution mode.
- Granting filesystem, environment, network, clock, or other WASI/host
  capabilities to shipped packages.
- Changing capability contract behavior or business rules.
- Remote placement or dynamic native plugin loading.
