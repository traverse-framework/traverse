# Feature Specification: Production Artifact Execution Routing

**Spec ID**: 064
**Status**: Draft
**Created**: 2026-07-12
**Input**: User-approved blocker-resolution decisions for issue #583.

## Context

Traverse resolves registered capabilities through `LocalExecutor`, while the
WASM executor exposes a separate capability-executor boundary. The shipped
server must route governed registered artifacts through one portable production
boundary rather than a demonstration executor.

## Requirements

- **FR-001**: The runtime MUST provide an `ArtifactRouter` that implements the
  local execution boundary and converts resolved capabilities into executor
  inputs.
- **FR-002**: The router MUST dispatch WASM artifacts to `WasmExecutor` and
  native artifacts only to explicitly host-registered native handlers.
- **FR-003**: The router MUST NOT load arbitrary native binaries, dynamic
  libraries, or shell commands from artifact paths.
- **FR-004**: `traverse-cli serve` MUST use the production artifact router by
  default.
- **FR-005**: The demonstration executor MUST be available only through an
  explicit `--example` mode and MUST NOT be the default execution path.
- **FR-006**: Fuel or epoch exhaustion MUST map to `Timeout`; memory/table
  exhaustion to `ResourceExhausted`; malformed artifact, checksum, ABI, or
  host-import violations to `ConstraintViolated`; invalid input/output to
  `InvalidInput`; and guest traps or host failures to `ExecutionFailed`.
- **FR-007**: The router MUST emit trace evidence naming the selected artifact
  type and stable failure classification without exposing host-internal errors.

## Out of Scope

- Dynamic native plugin loading.
- Remote execution placement.
- Changing WASM resource-limit defaults defined by spec 060.

## Verification

- End-to-end tests MUST register and execute a WASM artifact through the
  production server path.
- Tests MUST prove native handlers require explicit host registration and that
  unsupported artifact types fail with the documented stable classification.
