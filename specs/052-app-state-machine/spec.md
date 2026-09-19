# Feature Specification: App State Machine

**Feature Branch**: `052-app-state-machine`
**Created**: 2026-07-05
**Status**: Approved
**Version**: 1.1.0
**Input**: GitHub issue #525, following the app bundle manifest model from `044-application-bundle-manifest`.

**Amendment (2026-09-18, version 1.0.0 -> 1.1.0, approved 2026-09-18)**: Decision 96 /
Spec `139-embedder-app-state-machine-execution` / ADR-0075 add mutually exclusive
`invoke.host_connector`, host-connector completion event names, and static
validation that invoke waits declare required unhappy routes. Embedder and
`serve` execution of the machine is governed by Spec 139; this spec remains the
manifest schema home.

## Purpose

Traverse application manifests MAY declare a runtime-owned `state_machine` block. Clients render state and send commands; they do not own duplicated business state-machine logic.

This spec governs the manifest schema and validation slice. HTTP command dispatch, embedder command dispatch, saga waits, state subscriptions, session listing, and conditional output-based transitions are governed by successor specs (`059`, `139`, and related).

## Requirements

- **FR-001**: Application manifests MAY include `state_machine`.
- **FR-002**: `state_machine.initial_state` MUST name one declared state.
- **FR-003**: `state_machine.states[]` MUST contain unique non-empty `id` values.
- **FR-004**: Every transition `to` target MUST name a declared state.
- **FR-005**: Every state MUST be reachable from `initial_state`.
- **FR-006**: `invoke.capability_id`, when present, MUST reference a capability provided by a component declared in the same app manifest.
- **FR-007**: `invoke.input_from` MUST be explicit; the first supported value is `command.payload`.
- **FR-008**: `with_last_payload` MUST default to `false` when omitted.
- **FR-009**: `traverse-cli app validate --json` MUST include the validated state-machine summary on success.
- **FR-010**: Invalid state machines MUST fail validation before app registration.
- **FR-011**: A state MUST NOT declare both `invoke.capability_id` and `invoke.host_connector`.
- **FR-012**: `invoke.host_connector`, when present, MUST name a Spec 137–routable app command (for example `capture_audio`) bound by the application’s activated connector bindings (Spec 103).
- **FR-013**: Capability invoke waits MUST declare transitions for `capability_succeeded` and `capability_failed` (and timeout/cancel when applicable under Spec 139).
- **FR-014**: Host-connector invoke waits MUST declare transitions for `host_connector_succeeded`, `host_connector_failed`, `host_connector_timeout`, and `host_connector_cancelled` when the operation is cancellable.
- **FR-015**: `app validate` MUST reject invoke waits that omit the required unhappy routes in FR-013/FR-014.

## Schema Shape

```json
{
  "state_machine": {
    "initial_state": "idle",
    "states": [
      {
        "id": "idle",
        "transitions": [
          { "on": "submit", "to": "processing" }
        ]
      },
      {
        "id": "processing",
        "invoke": {
          "capability_id": "traverse-starter.process",
          "input_from": "command.payload"
        },
        "transitions": [
          { "on": "capability_succeeded", "to": "results" },
          { "on": "capability_failed", "to": "error" }
        ]
      }
    ]
  }
}
```

## Out of Scope

- HTTP state subscription endpoints (see Spec 033 successors).
- HTTP / embedder command *execution* mechanics (see Specs 059 and 139).
- Multiple app sessions and session listing.
- Conditional transitions based on capability output values.
- Durable execution of state-machine sessions (explicit follow-up; Spec 139 FR-016 keeps sessions process-local).
