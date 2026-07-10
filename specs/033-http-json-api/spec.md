# Feature Specification: HTTP+JSON Runtime API

**Feature Branch**: `033-http-json-api`  
**Created**: 2026-04-19  
**Amended**: 2026-05-27  
**Status**: Approved  
**Version**: 1.1.0  
**Input**: Approved product decisions for the app-consumable Traverse HTTP+JSON runtime API.

## Purpose

This spec defines the app-consumable HTTP+JSON API for Traverse v0.

The API exists so applications such as `youaskm3`, browser clients, local agents, and non-Rust tools can consume Traverse without shelling out to human-readable CLI commands. It is the first stable external runtime surface for app integration.

This spec is coordinated with:

- `034-programmatic-registration`
- `035-multi-agent-isolation`
- `029-integrated-observability`
- `030-security-identity-model`
- `040-contractual-enforcement-gate`

## Scope

In scope:

- `traverse-cli serve`
- local discovery through `.traverse/server.json`
- `/healthz`
- synchronous and asynchronous execution
- execution status fetch
- public trace fetch
- API versioning and response links
- JSON request/response envelopes
- RFC 9457 Problem Details errors
- CORS policy for local and production bindings
- OpenAPI artifact and CI validation

Out of scope:

- WebSocket transport
- Server-Sent Events transport
- federation
- TLS termination
- complete observability export shape (governed by `029-integrated-observability`)
- programmatic registration semantics (governed by `034-programmatic-registration`)
- workspace authorization semantics (governed by `035-multi-agent-isolation`)

## User Scenarios and Testing

### User Story 1 - Execute Through HTTP From Any App (Priority: P1)

As an application developer, I want to execute Traverse capabilities through a stable HTTP+JSON API so that my app can consume Traverse without parsing CLI output.

**Independent Test**: Start `traverse-cli serve`, call `POST /v1/workspaces/local-default/execute`, and verify a stable execution envelope with `execution_id`, `status`, `output`, `api_version`, and `links.trace`.

**Acceptance Scenarios**:

1. **Given** the server is running in dev-loopback mode, **When** a valid synchronous execution request is submitted, **Then** the API returns `200` with an execution envelope.
2. **Given** the server receives `Prefer: respond-async` or `mode: "async"`, **When** the request is accepted, **Then** the API returns `202` with an `execution_id`, `links.status`, `links.trace`, and optional `links.subscription`.
3. **Given** a completed async execution, **When** the client fetches `GET /v1/workspaces/{workspace_id}/executions/{execution_id}`, **Then** the API returns the execution status envelope.

### User Story 2 - Integrate From Browser Apps (Priority: P1)

As a browser app developer, I want Traverse to expose explicit CORS behavior so that local and production browser clients can call the runtime API safely.

**Independent Test**: Start the server in local dev mode, send a preflight from a loopback origin, and verify it succeeds. Start the server with a production/non-loopback binding and verify only exact configured origins are accepted.

**Acceptance Scenarios**:

1. **Given** dev-loopback mode and no `--allow-origin`, **When** a browser request comes from `http://localhost:*` or `http://127.0.0.1:*`, **Then** the API allows the origin.
2. **Given** a production or non-loopback binding, **When** CORS is configured, **Then** only exact configured origins are allowed.
3. **Given** a production or non-loopback binding, **When** no matching origin is configured, **Then** the API rejects the CORS request.

### User Story 3 - Connect Mobile Clients Without Filesystem Discovery (Priority: P1)

As a mobile app developer, I want a copyable Traverse provisioning URL so that iOS, Android, and other sandboxed clients can configure the runtime endpoint without reading `.traverse/server.json`.

**Independent Test**: Start `traverse-cli serve`, read stderr, and verify it prints a `traverse://connect?...` URL containing `base_url`, `workspace_default`, and `auth_mode`. Start with `--qr` and verify the terminal output includes an ASCII QR rendering for the same URL.

**Acceptance Scenarios**:

1. **Given** the server starts successfully, **When** it prints startup metadata, **Then** it includes a `traverse://connect` URL with percent-encoded `base_url`, `workspace_default`, and `auth_mode` query parameters.
2. **Given** `--qr` is supplied, **When** the server starts successfully, **Then** it renders an ASCII QR code for the same `traverse://connect` URL.
3. **Given** desktop clients still read `.traverse/server.json`, **When** mobile URL provisioning is added, **Then** existing discovery fields remain backward compatible.

### User Story 4 - Discover Local Server Without Guesswork (Priority: P1)

As a local app or agent, I want a repo-local discovery file so that I can find the Traverse server even if the default port is unavailable.

**Independent Test**: Start `traverse-cli serve`, read `.traverse/server.json`, call the declared `health_url`, and verify `status: "ok"` before using `base_url`.

**Acceptance Scenarios**:

1. **Given** the server starts successfully, **When** it writes `.traverse/server.json`, **Then** the file includes `base_url`, `health_url`, `workspace_default`, `pid`, `started_at`, and auth metadata.
2. **Given** `.traverse/server.json` exists, **When** a client wants to use it, **Then** the client MUST verify `GET /healthz` before trusting the file.
3. **Given** dev-loopback mode mints a local token, **When** the token is written to `.traverse/server.json`, **Then** the file MUST be owner-read/write only (`0600`) on Unix-like systems.

### User Story 5 - Receive Machine-Readable Errors (Priority: P1)

As an agent, I want all API errors to use stable JSON errors so that I can classify failures without regexing strings.

**Independent Test**: Submit malformed JSON, invalid contract input, unauthorized requests, and registry conflicts; verify each response uses `application/problem+json` with `traverse_code`.

## Required Endpoints

### Health

`GET /healthz`

Returns a JSON health envelope:

```json
{
  "status": "ok",
  "version": "0.2.0",
  "api_version": "v1",
  "workspace_default": "local-default",
  "auth_mode": "dev-loopback"
}
```

Allowed `auth_mode` values:

- `dev-loopback`
- `dev-any`
- `bearer-required` (token-authenticated production/non-loopback mode)

### Bearer Token Authentication

In `bearer-required` mode the server authenticates callers with **signed JWT
bearer tokens**. A JWT is a *signed assertion*, not a transport encoding: the
server MUST verify the signature before trusting any claim it carries.

- The server MUST verify the token signature over the `header.payload` signing
  input against a configured verification key before deriving any identity.
- The server MUST enforce an explicit `alg` allow-list. The only accepted
  algorithm is `EdDSA` (Ed25519). `alg: none` and any other algorithm MUST be
  rejected with `401` and a distinct `traverse_code` of `token_alg_not_allowed`,
  so an attacker cannot strip verification.
- Privilege claims (`traverse_admin`, `roles`, `role`) MUST only be honored for a
  signature-verified token. An unverified token MUST NOT yield an administrative
  identity or access to the privileged system workspace.
- The server MUST validate `exp` and `nbf` when present, rejecting expired
  (`token_expired`) and not-yet-valid (`token_not_yet_valid`) tokens.
- When `bearer-required` mode is active and no verification key is configured,
  the server MUST **fail closed**: every bearer token is rejected with `401`
  (`jwt_verification_unavailable`). It MUST NOT fall back to trusting unverified
  token payloads.
- The opaque "bearer token equals subject id" convenience is permitted only in
  the `dev-loopback` and `dev-any` modes and MUST NOT yield an administrative
  identity on a network-facing (`bearer-required`) listener.

Identity attribution derived elsewhere in the runtime (for example, trace and
audit labeling from a caller token) is not an authorization decision and MUST
NOT gate access; access control is enforced at this HTTP boundary.

### Execute

`POST /v1/workspaces/{workspace_id}/execute`

If `workspace_id` is omitted through a local dev convenience route, the server MAY use `local-default` only in dev-loopback mode. Production/non-loopback mode MUST require an explicit `workspace_id`.

Synchronous success response:

```json
{
  "api_version": "v1",
  "execution_id": "exec_01HX...",
  "status": "succeeded",
  "output": {},
  "links": {
    "self": "/v1/workspaces/local-default/executions/exec_01HX...",
    "trace": "/v1/workspaces/local-default/traces/exec_01HX..."
  }
}
```

Asynchronous accepted response:

```json
{
  "api_version": "v1",
  "execution_id": "exec_01HX...",
  "status": "accepted",
  "links": {
    "status": "/v1/workspaces/local-default/executions/exec_01HX...",
    "trace": "/v1/workspaces/local-default/traces/exec_01HX...",
    "subscription": "/v1/workspaces/local-default/executions/exec_01HX.../events"
  }
}
```

The API MUST support async through either:

- `Prefer: respond-async`
- request body field `mode: "async"`

### Execution Status

`GET /v1/workspaces/{workspace_id}/executions/{execution_id}`

Response:

```json
{
  "api_version": "v1",
  "execution_id": "exec_01HX...",
  "status": "running",
  "created_at": "2026-05-27T12:00:00Z",
  "updated_at": "2026-05-27T12:00:01Z",
  "links": {
    "self": "/v1/workspaces/local-default/executions/exec_01HX...",
    "trace": "/v1/workspaces/local-default/traces/exec_01HX...",
    "subscription": "/v1/workspaces/local-default/executions/exec_01HX.../events"
  }
}
```

Allowed status values:

- `accepted`
- `running`
- `succeeded`
- `failed`
- `cancelled`

### Trace Fetch

`GET /v1/workspaces/{workspace_id}/traces/{execution_id}`

The API returns a stable public trace envelope, not the raw internal trace artifact and not direct OpenTelemetry export:

```json
{
  "api_version": "v1",
  "execution_id": "exec_01HX...",
  "trace": {
    "spans": [],
    "events": []
  },
  "links": {
    "execution": "/v1/workspaces/local-default/executions/exec_01HX..."
  }
}
```

OpenTelemetry export is governed by `029-integrated-observability`.

### App State Event Stream

`GET /v1/workspaces/{workspace_id}/apps/{app_id}/events`

The API returns a Server-Sent Events stream for app-scoped runtime state updates. Browser clients can consume it with native `EventSource`; mobile and desktop clients can consume it as an HTTP text stream.

Required response headers:

```text
Content-Type: text/event-stream
Cache-Control: no-cache
```

If `Last-Event-ID` is supplied, the server MUST replay app events after the matching event id when those events are still available in runtime memory. If no app events are available, the server MUST return a `heartbeat` event so clients can validate the stream shape.

Implemented event types for this slice:

- `state_changed`
- `capability_invoked`
- `capability_result`
- `error`
- `heartbeat`

Example event:

```text
id: exec_01HX...:capability_result
event: capability_result
data: {"workspace_id":"local-default","app_id":"traverse-starter","session_id":"sess_01HX...","execution_id":"exec_01HX...","state":"results","previous_state":"processing","output":{}}
```

Live command-driven broadcast is completed by the command dispatch slice. This endpoint establishes the app-scoped `text/event-stream` contract, replay behavior, and runtime event payload shape.

### App Session Listing

`GET /v1/workspaces/{workspace_id}/apps/{app_id}/sessions`

The API returns app state-machine sessions derived from retained app runtime events. This endpoint is for authority surfaces that need a list view, such as reviewer, approver, or supervisor queues.

Supported query parameters:

- `state`: optional current-state filter.
- `limit`: optional page size, default `50`, valid range `1..200`.
- `cursor`: optional opaque session cursor from a previous response.
- `order`: optional `created_asc` or `created_desc`, default `created_desc`.

Response:

```json
{
  "api_version": "v1",
  "app_id": "traverse-starter",
  "sessions": [
    {
      "session_id": "sess_01HX...",
      "current_state": "pending_review",
      "created_at": "unix:1",
      "updated_at": "unix:2",
      "context": {
        "document_type": "invoice",
        "confidence_score": 0.72
      }
    }
  ],
  "total": 1,
  "next_cursor": null
}
```

The `context` object MUST include only output fields listed in the app manifest state machine's `list_context_fields` array. Missing fields are omitted. Fields are projected from runtime-owned capability output; clients render this data but do not compute it.

## Server Startup and Discovery

- `traverse-cli serve` MUST default to `127.0.0.1:8787`.
- `--bind` MUST allow overriding the bind address.
- Non-loopback bindings MUST require auth/production mode unless `--auth dev-any` is explicitly supplied for local LAN development.
- `--auth dev-any` MUST bind to `0.0.0.0` by default, MUST allow loopback and RFC 1918 private IPv4 callers without bearer auth, and MUST reject public callers with `403`.
- `--auth dev-any` MUST print a startup warning that the mode accepts LAN connections and is not for production.
- If the default port is unavailable, the server MAY fail clearly or select another local port, but if it selects another port it MUST write the selected address to `.traverse/server.json`.
- Startup MUST print JSON startup information to stdout or stderr in a machine-readable form.
- `.traverse/server.json` MUST be repo-local for v0 and MUST be ignored by git.
- `traverse-cli serve` MUST print the `traverse://connect` URL on startup.
- `traverse-cli serve --qr` MUST render an ASCII QR code for the `traverse://connect` URL on startup.

Discovery file required fields:

```json
{
  "base_url": "http://127.0.0.1:8787",
  "bind_address": "127.0.0.1:8787",
  "health_url": "http://127.0.0.1:8787/healthz",
  "workspace_default": "local-default",
  "pid": 12345,
  "started_at": "2026-05-27T12:00:00Z",
  "auth_mode": "dev-loopback",
  "mobile_connect_url": "traverse://connect?base_url=http%3A%2F%2F127.0.0.1%3A8787&workspace_default=local-default&auth_mode=dev-loopback",
  "token": "local-dev-token"
}
```

When `token` is present, the file MUST be owner-read/write only (`0600`) on Unix-like systems.
Mobile or sandboxed clients MAY use `mobile_connect_url` instead of reading `.traverse/server.json`.
`traverse://connect` URLs MUST carry `base_url`, `workspace_default`, and `auth_mode`; they MUST NOT include local tokens.

## Errors

All API errors MUST use RFC 9457 Problem Details with `Content-Type: application/problem+json`.

Required fields:

```json
{
  "type": "https://traverse.dev/problems/validation-failed",
  "title": "Validation failed",
  "status": 422,
  "detail": "The request body violated the runtime request schema.",
  "instance": "/v1/workspaces/local-default/execute",
  "traverse_code": "validation_failed"
}
```

Status rules:

- Malformed JSON: `400`
- Unknown fields in core API envelopes or governed payloads: `400` or `422`, depending on whether parsing or domain validation found the error
- Contract/schema validation failure: `422`
- Immutable registry conflict: `409`
- Idempotency key reuse with a different body: `409`
- Missing/invalid credentials: `401`
- Valid credentials without permission: `403`
- Missing resource: `404`
- Unsupported media type: `415`
- Payload too large: `413`

## Idempotency

Mutation endpoints SHOULD support optional `Idempotency-Key`.

Rules:

- Reusing the same key with the same request body returns the original result.
- Reusing the same key with a different request body returns `409 Conflict` with Problem Details.
- The server MUST store a request hash with the key.
- The default retention window is 24 hours.
- Retention MUST be configurable with a documented minimum.

## Versioning and Compatibility

- URL paths MUST use path-based versioning (`/v1/...`).
- JSON response envelopes MUST include `api_version`.
- Approved `/v1` APIs MUST NOT introduce breaking changes within `/v1`.
- Additive response fields are allowed.
- Clients MUST ignore unknown response fields.
- Request bodies MUST reject unknown fields in core API envelopes and governed payloads.
- Response envelopes SHOULD include stable `links` for next actions.

## CORS

- Dev-loopback mode MAY allow common loopback browser origins by default:
  - `http://localhost:*`
  - `http://127.0.0.1:*`
- Production/non-loopback mode MUST require exact configured origins.
- Wildcard CORS MUST NOT be allowed by default.
- `--allow-origin <origin>` MUST configure additional allowed origins.

## OpenAPI

- The HTTP API spec MUST include `specs/033-http-json-api/openapi.yaml`.
- CI MUST validate that the OpenAPI artifact is structurally valid.
- The OpenAPI document is the machine-readable contract for app consumers.
- The prose spec wins if prose and OpenAPI conflict until the conflict is corrected.

## Functional Requirements

- **FR-001**: `traverse-cli serve` MUST start the HTTP+JSON runtime API server.
- **FR-002**: `traverse-cli serve` MUST default to `127.0.0.1:8787`.
- **FR-003**: The API MUST expose `GET /healthz`.
- **FR-004**: The API MUST expose `POST /v1/workspaces/{workspace_id}/execute`.
- **FR-005**: The API MUST expose `GET /v1/workspaces/{workspace_id}/executions/{execution_id}`.
- **FR-006**: The API MUST expose `GET /v1/workspaces/{workspace_id}/traces/{execution_id}`.
- **FR-007**: The API MUST expose `GET /v1/workspaces/{workspace_id}/apps/{app_id}/events` as an app-scoped Server-Sent Events stream.
- **FR-008**: The API MUST expose `GET /v1/workspaces/{workspace_id}/apps/{app_id}/sessions` as an app-scoped session listing endpoint.
- **FR-009**: The API MUST support synchronous and asynchronous execution.
- **FR-010**: Synchronous success MUST return a stable execution envelope.
- **FR-011**: Asynchronous acceptance MUST return `202` with `execution_id`, status URL, trace URL, and optional subscription URL.
- **FR-012**: Trace fetch MUST return a public trace envelope.
- **FR-013**: App event streams MUST use `text/event-stream`, stable event ids, named event types, and `Last-Event-ID` replay when retained events exist.
- **FR-014**: App session listing MUST support `state`, `limit`, `cursor`, and `order` query parameters.
- **FR-015**: App session listing MUST project `context` only from manifest-declared `list_context_fields`.
- **FR-016**: All errors MUST use RFC 9457 Problem Details with `traverse_code`.
- **FR-017**: Validation failures MUST use `422`.
- **FR-018**: Immutable registry conflicts MUST use `409`.
- **FR-019**: Unauthenticated access MUST use `401`; unauthorized access MUST use `403`.
- **FR-020**: Mutation endpoints SHOULD support optional `Idempotency-Key`.
- **FR-021**: The server MUST write `.traverse/server.json` in dev-loopback mode.
- **FR-022**: Discovery files containing a token MUST be owner-read/write only (`0600`) on Unix-like systems.
- **FR-023**: The server MUST expose coarse `auth_mode` in health output.
- **FR-024**: Local dev MAY use `local-default` when `workspace_id` is omitted; production MUST require explicit workspace.
- **FR-025**: The API MUST reject unknown request fields in core API envelopes and governed payloads.
- **FR-026**: The API MUST include `api_version` in response envelopes.
- **FR-027**: The API MUST include stable links for next actions.
- **FR-028**: CORS MUST follow the dev-loopback and production rules above.
- **FR-029**: CI MUST validate `specs/033-http-json-api/openapi.yaml`.
- **FR-030**: The server MUST expose mobile URL provisioning through a `traverse://connect` URL that carries `base_url`, `workspace_default`, and `auth_mode`.
- **FR-031**: The server MUST render an ASCII QR code for the mobile provisioning URL when `--qr` is supplied.
- **FR-032**: `--auth dev-any` MUST support real-device LAN development by accepting RFC 1918 private IPv4 callers and rejecting public callers.
- **FR-033**: In `bearer-required` mode the server MUST verify the JWT signature against a configured `Ed25519` verification key before trusting any claim, and MUST reject unverifiable tokens with `401`.
- **FR-034**: The server MUST enforce a JWT `alg` allow-list of `EdDSA` only and MUST reject `alg: none` and every other algorithm with `401` (`token_alg_not_allowed`).
- **FR-035**: Administrative privilege (and system-workspace access) MUST only be granted from a signature-verified token; an unverified token MUST NOT yield `is_admin`.
- **FR-036**: In `bearer-required` mode with no verification key configured, the server MUST fail closed and reject all bearer tokens (`jwt_verification_unavailable`).
- **FR-037**: The server MUST validate JWT `exp` and `nbf` when present, rejecting expired (`token_expired`) and not-yet-valid (`token_not_yet_valid`) tokens.

## Quality Gates

- **QG-001**: Every endpoint in this spec MUST appear in `openapi.yaml`.
- **QG-002**: Every endpoint MUST include at least one example request or response in the spec or OpenAPI artifact.
- **QG-003**: All endpoint tests MUST assert required fields rather than exact full JSON equality, because additive response fields are allowed.
- **QG-004**: No implementation PR may merge against proposed specs before spec approval.
- **QG-005**: Draft implementation branches may exist while specs are proposed, but implementation may not merge until the governing spec is approved.

## Success Criteria

- **SC-001**: A local app can discover the Traverse server through `.traverse/server.json` and verify `/healthz`.
- **SC-002**: A browser app can call the local API from a loopback origin without wildcard CORS.
- **SC-003**: A non-loopback binding requires Bearer auth and exact CORS origin configuration.
- **SC-004**: A client can execute synchronously and receive an execution envelope.
- **SC-005**: A client can request async execution, poll execution status, and fetch a public trace envelope.
- **SC-006**: A browser client can subscribe to an app-scoped SSE endpoint and receive named runtime state events or a heartbeat.
- **SC-007**: An authority UI can list app sessions filtered by state with manifest-governed context fields.
- **SC-008**: Invalid requests produce RFC 9457 Problem Details with stable `traverse_code`.
- **SC-009**: The OpenAPI YAML validates in CI.
