# Feature Specification: App State Machine

**Feature Branch**: `052-app-state-machine`
**Created**: 2026-07-05
**Status**: Approved
**Input**: GitHub issue #525, following the app bundle manifest model from `044-application-bundle-manifest`.

## Purpose

Traverse application manifests MAY declare a runtime-owned `state_machine` block. Clients render state and send commands; they do not own duplicated business state-machine logic.

This spec governs the manifest schema and validation slice. HTTP command dispatch, state subscriptions, session listing, and conditional output-based transitions are governed by follow-up Project tickets.

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

- HTTP state subscription endpoints.
- HTTP command dispatch endpoints.
- Multiple app sessions and session listing.
- Conditional transitions based on capability output values.
- Durable execution of state-machine sessions.
