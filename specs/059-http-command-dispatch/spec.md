# Feature Specification: HTTP Command Dispatch

**Feature Branch**: `059-http-command-dispatch`
**Created**: 2026-07-06
**Status**: Approved
**Version**: 1.0.0
**Input**: Traverse issue #527, spec **052-app-state-machine**, **033-http-json-api** SSE slice.

## Purpose

Define the HTTP command dispatch endpoint that completes the app state-machine client pattern started by **052** and **033** SSE events.

Clients send **commands**; runtime owns transitions and capability invocation. Clients MUST NOT poll execution status when using this slice.

## Relationship to Other Specs

| Spec | Relationship |
|------|----------------|
| **033-http-json-api** | SSE `/apps/{app_id}/events` delivers state; this spec adds POST `/apps/{app_id}/commands`. |
| **052-app-state-machine** | Commands map to transition `on` values; invalid commands return 422. |
| **057-embeddable-runtime-host** | Embedded apps use embedder `runtime.submit` instead of HTTP; same command names and payload shapes. |

**No conflict with 033 execute/poll**: execute+polling remains for Phase 1 clients until migrated; command dispatch is the Phase 2 client path for state-machine apps.

## Endpoint

`POST /v1/workspaces/{workspace_id}/apps/{app_id}/commands`

Request:

```json
{
  "command": "submit",
  "payload": { "note": "Meeting with design team" }
}
```

Responses:

| Status | Meaning |
|--------|---------|
| **202** | Command accepted; transition started; events on SSE stream |
| **409** | Invalid transition for current state |
| **422** | Unknown command name |
| **404** | App not registered in workspace |

## Functional Requirements

- **FR-001**: Runtime MUST validate command against current state machine state before transition.
- **FR-002**: Accepted commands MUST emit `state_changed` on SSE within 100ms (same stream as 033).
- **FR-003**: When transition includes `invoke`, runtime MUST invoke capability and emit `capability_result` or `error`.
- **FR-004**: Command payload MUST map to `invoke.input_from: command.payload` per 052.
- **FR-005**: Clients MUST NOT require polling `/executions/{id}` when using command + SSE path.
- **FR-006**: Problem Details (RFC 9457) for 409/422 errors.
- **FR-007**: OpenAPI artifact updated in repo with this endpoint.

## Definition of Done (implementation — #527)

- [ ] Endpoint implemented in `traverse-cli serve`
- [ ] Integration test: submit command → SSE state_changed → capability_result
- [ ] reference-apps **#43** unblocked

## Out of Scope

- Embedded embedder transport (057 covers in-process submit)
