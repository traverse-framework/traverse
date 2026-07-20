# Feature Specification: Production Swift wasmi C-ABI Boundary

**Feature Branch**: `codex/issue-776-production-cabi`
**Created**: 2026-07-20
**Status**: Approved
**Version**: 1.0.0
**Input**: ADR-0015, Traverse #776, and the approved production profile.

## Purpose

Define the governed native boundary through which the public Swift package
hosts the runtime-owned bridge with wasmi. This successor specification adds no
public embedder operation and does not certify a release.

## User Scenarios & Testing

### User Story 1 - Load a Verified Bounded Bridge (Priority: P1)

A Swift application loads a bundled runtime only when its identity,
compatibility, and declared limits are valid.

**Independent Test**: A valid fixture creates a host; oversized, tampered,
imported, or incompatible fixtures fail before instantiation.

### User Story 2 - Invoke Runtime-Owned Operations Safely (Priority: P1)

An application invokes an allowlisted bridge operation and receives a bounded,
ordered runtime result without ownership ambiguity.

**Independent Test**: Each bridge 1.1 operation succeeds through one opaque
handle; an insufficient output buffer reports its exact required size.

### User Story 3 - Fail Predictably (Priority: P2)

An application receives stable structured failures for invalid handles, input,
descriptors, traps, and resource exhaustion.

**Independent Test**: Invalid fixtures and exhausted memory/fuel return stable
codes and bounded JSON details without a sidecar or ambient authority.

### Edge Cases

- A caller uses a destroyed, concurrent, or re-entrant handle.
- An output/event exceeds its configured cap.
- A module requests undeclared imports or exceeds fuel/memory limits.

## Requirements

### Functional Requirements

- **FR-001**: The boundary MUST expose only the five production symbols in
  ADR-0015 and one opaque serialized host handle.
- **FR-002**: Creation MUST verify runtime bytes, expected SHA-256 digest,
  artifact size, no ambient imports, bridge 1.1 ABI, and required exports
  before instantiation.
- **FR-003**: Creation MUST require positive caps for artifact, memory, fuel,
  input, output/event, and queued-event resources; no value may mean unlimited.
- **FR-004**: Invocation MUST allow only the governed bridge 1.1 operation
  set and preserve runtime-owned lifecycle, event, and error semantics.
- **FR-005**: Inputs and outputs MUST be caller-owned bounded UTF-8 buffers;
  insufficient output capacity MUST report the exact retry size.
- **FR-006**: The boundary MUST return stable numeric statuses and bounded
  structured UTF-8 JSON errors for all expected failures.
- **FR-007**: Unsafe code MUST be restricted to the audited host file and
  validated pointer conversions; CI MUST reject unreviewed exports or unsafe
  opt-outs.
- **FR-008**: The initial certified-profile target matrix MUST be arm64 iOS,
  arm64 iOS simulator, and arm64 macOS only.

## Success Criteria

- **SC-001**: 100% of rejected fixtures fail before bridge invocation.
- **SC-002**: Every bridge 1.1 operation passes the same bounded host corpus.
- **SC-003**: No production host call requires a sidecar, network, or ambient
  host authority.
- **SC-004**: Release evidence can identify engine version, limits, runtime
  digest, bridge version, host profile, and conformance result for every
  certification candidate.

## Out of Scope

- Release certification, Intel macOS support, reference-app integration, and
  public embedder API changes.
