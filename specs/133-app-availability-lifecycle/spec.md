# Feature Specification: Server-Owned Application Availability Lifecycle

**Status**: Approved
**Canonical governing ID**: `133-app-availability-lifecycle`
**Extends**: `046-public-cli-app-registration`, `052-app-state-machine`, `033-http-json-api`
**Decision evidence**: Traverse #1324, decisions D1–D10

## Purpose

Define the server-owned availability of a registered workspace application. This
model tells operators and HTTP clients whether `traverse-cli serve` has
materialized a persisted, resolved app declaration and may accept app commands.
It is distinct from the fixed runtime execution lifecycle and the app's
manifest-defined business workflow.

## Scope

This spec governs `traverse-cli serve` startup evidence and a read-only
workspace app-status endpoint. It introduces no capability, event, network
fetch, state mutation, reload operation, or durable availability journal.

## State Model

The complete v1 vocabulary is `loading`, `ready`, and `failed`.

| From | To | Stable reason |
| --- | --- | --- |
| `loading` | `ready` | `persisted_app_state_loaded` |
| `loading` | `failed` | a documented app-state materialization reason |

No other transition is valid. `ready` and `failed` are terminal for one
server-process load attempt. Restarting `serve`, or a future governed reload,
begins a new attempt at `loading`. Command-time capability failures do not
alter app availability.

## Functional Requirements

- **FR-001**: `serve` MUST model runtime execution lifecycle, application
  business workflow, and application availability as distinct owned models.
- **FR-002**: For every registered workspace app, `serve` MUST create one
  load attempt and record every permitted availability transition.
- **FR-003**: `ready` MUST mean that the persisted resolved app declaration
  and state machine are valid and usable, and that declared components were
  admitted into verified local workspace state. It MUST NOT require all WASM
  modules to have been read, compiled, or instantiated in that process.
- **FR-004**: A missing, malformed, incompatible, or unmaterializable
  persisted declaration MUST produce `failed`; it MUST NOT silently omit the
  app or report it as unregistered.
- **FR-005**: Each transition record MUST contain schema version, workspace
  id, app id, load-attempt id, prior state, next state, timestamp, and a
  stable secret-free reason code. Failed evidence MAY contain a redacted
  message, but MUST NOT expose private paths, Registry URLs, artifact bytes,
  configuration values, or secrets.
- **FR-006**: `serve` MUST emit the records as structured startup/server
  diagnostics and retain their ordered sequence in process memory for the
  active process lifetime. It MUST NOT persist or restore lifecycle history.
- **FR-007**: `GET /v1/workspaces/{workspace}/apps/status` MUST return the
  current availability and ordered transition history for every registered
  app visible to the caller. Its authorization policy MUST match protected
  workspace read endpoints; unauthorized callers receive no app evidence.
- **FR-008**: The endpoint MUST distinguish an absent workspace/app from a
  registered failed app. A registered failed app remains observable with its
  failure reason code.
- **FR-009**: A command addressed to a registered failed app MUST return
  `503 app_unavailable` with the stable diagnostic; it MUST NOT return
  `404 app_not_registered`.
- **FR-010**: `serve` startup and command execution MUST NOT fetch, sync,
  resolve mutable Registry references, or mutate workspace state to determine
  availability.

## Compatibility and Non-goals

This is additive to existing runtime and app-state-machine contracts. It does
not add `degraded`, hot reload, disable/suspend/recover operations, app-defined
availability values, a durable status journal, lazy Registry resolution, or
general lazy capability-registry reconstruction. A legacy local registration
that lacks required persisted resolved state requires explicit re-registration;
no source-manifest fallback or automatic migration is permitted.

## Acceptance Scenarios

1. A valid persisted app transitions `loading → ready`; the endpoint and
   startup evidence report the same ordered record.
2. A malformed persisted declaration transitions `loading → failed`; the app
   is visible through status and its command returns `503 app_unavailable`.
3. A missing/tampered capability on first invocation fails only that command
   with a secret-free capability diagnostic; the app remains `ready`.
4. A restart creates a new load-attempt id and new in-memory history without
   restoring the previous attempt's transition records.
5. Unauthorized callers cannot obtain app availability or transition evidence.

## Quality Gates

- **QG-001**: Unit coverage proves the closed transition table and rejection
  of every undeclared transition.
- **QG-002**: HTTP conformance covers ready, failed, `503`, authorization,
  redaction, ordering, and restart semantics on supported host OSes.
- **QG-003**: Tests prove startup and command handling make no network call
  and perform no Registry sync or workspace mutation.
