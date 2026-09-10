# Implementation Context: HTTP+JSON Runtime API

## Owns

- `traverse-cli serve` app-facing HTTP surface.
- Local discovery file `.traverse/server.json`.
- `GET /healthz`.
- `POST /v1/workspaces/{workspace_id}/execute`.
- `GET /v1/workspaces/{workspace_id}/executions/{execution_id}`.
- `GET /v1/workspaces/{workspace_id}/traces/{execution_id}`.
- `GET /v1/workspaces/{workspace_id}/apps/status`.
- JSON envelopes, links, API versioning, CORS, idempotency key behavior, Problem Details shape, and OpenAPI structure.

## Does Not Own

- Registration semantics; use `034-programmatic-registration`.
- Workspace auth, scopes, runtime grants, and audit rules; use `035-multi-agent-isolation`.
- Complete telemetry/export semantics; use `029-integrated-observability`.
- Contract enforcement semantics; use `040-contractual-enforcement-gate`.

## Key Invariants

- `v1` paths must not receive breaking changes after approval.
- Response envelopes include `api_version`.
- Mutation requests reject unknown fields.
- Errors use RFC 9457 Problem Details with `traverse_code`.
- Local dev may default to `local-default`; production requires explicit workspace.
- Dev-loopback default bind is `127.0.0.1:8787`.
- If `.traverse/server.json` contains a token, Unix-like permissions must be `0600`.
- CORS permits common loopback origins in local dev and exact configured origins in production; wildcard production CORS is not allowed.

## Artifacts

- Governing spec: `specs/033-http-json-api/spec.md`.
- OpenAPI surface: `specs/033-http-json-api/openapi.yaml`.
- Structural validation: `scripts/ci/openapi_structural_validation.sh`.

## Implementation Tickets

- `#387` implements `traverse-cli serve` defaults and repo-local discovery.
- `#390` implements `GET /healthz`.
- `#391` implements sync and async execute envelopes.
- `#392` implements execution status fetch.
- `#393` implements public trace fetch.
- `#394` implements Problem Details errors.
- `#395` implements mutation `Idempotency-Key`.
- `#396` implements CORS policy.
