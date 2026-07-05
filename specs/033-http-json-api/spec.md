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
- **FR-007**: The API MUST support synchronous and asynchronous execution.
- **FR-008**: Synchronous success MUST return a stable execution envelope.
- **FR-009**: Asynchronous acceptance MUST return `202` with `execution_id`, status URL, trace URL, and optional subscription URL.
- **FR-010**: Trace fetch MUST return a public trace envelope.
- **FR-011**: All errors MUST use RFC 9457 Problem Details with `traverse_code`.
- **FR-012**: Validation failures MUST use `422`.
- **FR-013**: Immutable registry conflicts MUST use `409`.
- **FR-014**: Unauthenticated access MUST use `401`; unauthorized access MUST use `403`.
- **FR-015**: Mutation endpoints SHOULD support optional `Idempotency-Key`.
- **FR-016**: The server MUST write `.traverse/server.json` in dev-loopback mode.
- **FR-017**: Discovery files containing a token MUST be owner-read/write only (`0600`) on Unix-like systems.
- **FR-018**: The server MUST expose coarse `auth_mode` in health output.
- **FR-019**: Local dev MAY use `local-default` when `workspace_id` is omitted; production MUST require explicit workspace.
- **FR-020**: The API MUST reject unknown request fields in core API envelopes and governed payloads.
- **FR-021**: The API MUST include `api_version` in response envelopes.
- **FR-022**: The API MUST include stable links for next actions.
- **FR-023**: CORS MUST follow the dev-loopback and production rules above.
- **FR-024**: CI MUST validate `specs/033-http-json-api/openapi.yaml`.
- **FR-025**: The server MUST expose mobile URL provisioning through a `traverse://connect` URL that carries `base_url`, `workspace_default`, and `auth_mode`.
- **FR-026**: The server MUST render an ASCII QR code for the mobile provisioning URL when `--qr` is supplied.
- **FR-027**: `--auth dev-any` MUST support real-device LAN development by accepting RFC 1918 private IPv4 callers and rejecting public callers.

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
- **SC-006**: Invalid requests produce RFC 9457 Problem Details with stable `traverse_code`.
- **SC-007**: The OpenAPI YAML validates in CI.
