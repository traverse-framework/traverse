# Feature Specification: Mediated Runtime Connector Invocation

**Status**: Approved
**Canonical governing ID**: `104-mediated-connector-invocation`
**Version**: 1.1.0
**Amended**: 2026-11-02 (#1719, Decision 122): FR-008.
**Extends**: `039-connector-plugin-architecture`, `098-capability-event-host-abi`, `103-application-connector-binding`
**Input**: #826, #1050, #1052; ADR-0039.

## Purpose

Define the only guest-to-host route for invoking an activated Spec 039 connector
without weakening Traverse's deny-by-default WASM sandbox.

## Requirements

- **FR-001**: The runtime MUST expose a versioned `connector_invoke` host ABI
  with typed, bounded request, response, and error envelopes.
- **FR-002**: The host MUST authorize a call only when the caller declares the
  Spec 039 requirement and an activated application binding resolves it.
- **FR-003**: The host MUST enforce connector scope, placement, path/device/
  network policy, payload and resource limits, cancellation, and output schema.
- **FR-004**: Private configuration, credentials, paths, device identifiers,
  and provider endpoints MUST remain host-owned and inaccessible to guests.
- **FR-005**: The runtime MUST emit deterministic, non-secret trace evidence
  for authorization, selected connector/version, result class, and failure.
- **FR-006**: Undeclared, unbound, unavailable, incompatible, unconfigured,
  unauthorized, bounded-I/O, cancellation, and execution failures MUST return
  stable non-secret errors.
- **FR-007**: Ambient WASI filesystem, network, environment, and device access
  MUST remain denied.
- **FR-008**: The host-private output filter MUST NOT apply to the top-level
  `model_evidence` of a `traverse.model-runtime` response. That value is the
  signed rights record the host verified, so it is public and may hold public
  URLs. The filter MUST still apply to the rest of that response and to every
  other connector's response.

## Acceptance Scenarios

1. An authorized call reaches only its activated compatible binding.
2. Each authorization failure is rejected before connector execution.
3. Oversized/out-of-bounds guest payloads fail without host panic or leakage.
4. Public traces contain no private configuration or credentials.

## Quality Gates

- ABI, authorization, bounds, cancellation, and redaction conformance fixtures pass.
- No guest can bypass declared requirements, bindings, or host policy.
- The spec-alignment gate covers host ABI, runtime, contracts, and ADR-0039.

## Out of Scope

Ambient WASI grants, sidecar bridges, concrete connectors, scheduler/retry, and
generic local-model artifact governance.
