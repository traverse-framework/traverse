# Canonical Traverse HTTP App Path for youaskm3

This page is the release-facing HTTP/JSON integration path that `youaskm3` can cite for its first real release.

Supported Traverse baseline: `v0.3.0`

## What Is Supported

For the first `youaskm3` release, the canonical Traverse app-facing path is a local source-build HTTP/JSON server:

```bash
cargo run -p traverse-cli -- serve
```

This starts the governed app-consumable API on `127.0.0.1:8787` by default and writes a local discovery file at:

```text
.traverse/server.json
```

The supported downstream app category is a local app, development shell, or browser-hosted consumer that can read the discovery file or receive the discovered `base_url`, then call the documented HTTP/JSON API.

## Public App Surface

The first-release public HTTP/JSON path is governed by:

- [specs/033-http-json-api/openapi.yaml](../specs/033-http-json-api/openapi.yaml)
- [specs/033-http-json-api/spec.md](../specs/033-http-json-api/spec.md)
- [specs/034-programmatic-registration/spec.md](../specs/034-programmatic-registration/spec.md)
- [specs/035-multi-agent-isolation/spec.md](../specs/035-multi-agent-isolation/spec.md)

The app-facing surfaces `youaskm3` can depend on at `v0.3.0` are:

- `traverse-cli serve`
- `.traverse/server.json`
- `GET /healthz`
- `POST /v1/workspaces/{workspace_id}/capabilities`
- `POST /v1/workspaces/{workspace_id}/execute`
- `GET /v1/workspaces/{workspace_id}/executions/{execution_id}`
- `GET /v1/workspaces/{workspace_id}/traces/{execution_id}`
- RFC 9457 Problem Details error envelopes

Implementation details inside `crates/traverse-cli/src/http_api.rs`, in-memory stores, test helpers, and private registry internals are not downstream app API.

## Start From The Released Tag

Downstream consumers should pin the released Traverse tag instead of following repository head:

```bash
git clone https://github.com/enricopiovesan/Traverse.git
cd Traverse
git checkout v0.3.0
cargo run -p traverse-cli -- serve
```

Requirements:

- Rust 1.94+
- local source checkout of Traverse `v0.3.0`
- an HTTP client such as `curl`
- `jq` for the copy/pasteable shell examples below

For packaging and source-build expectations, see [docs/v0.3.0-source-build-consumer-packaging.md](v0.3.0-source-build-consumer-packaging.md).
For the combined release evidence path that includes this app surface, see [docs/v0.3.0-downstream-validation-path.md](v0.3.0-downstream-validation-path.md).

## Discover The Server

In another terminal, read the local discovery file:

```bash
BASE_URL="$(jq -r '.base_url' .traverse/server.json)"
HEALTH_URL="$(jq -r '.health_url' .traverse/server.json)"
WORKSPACE_ID="$(jq -r '.workspace_default' .traverse/server.json)"
AUTH_MODE="$(jq -r '.auth_mode' .traverse/server.json)"

printf 'base_url=%s\nhealth_url=%s\nworkspace=%s\nauth_mode=%s\n' \
  "$BASE_URL" "$HEALTH_URL" "$WORKSPACE_ID" "$AUTH_MODE"
```

Expected local development values:

- `base_url`: `http://127.0.0.1:8787`
- `health_url`: `http://127.0.0.1:8787/healthz`
- `workspace_default`: `local-default`
- `auth_mode`: `dev-loopback`

If `.traverse/server.json` contains a local token, the file is host-local discovery material and must not be committed. The repository already ignores `.traverse/`.

## Check Health

```bash
curl -sS "$HEALTH_URL"
```

Expected result:

- HTTP 200
- `status: ok`
- `api_version: v1`
- the same `workspace_default` reported by `.traverse/server.json`

## Register One Capability

This example registers the checked-in hello-world capability into the default workspace. It uses only source checkout artifacts and the public HTTP registration endpoint.

```bash
ARTIFACT_PATH="$PWD/examples/hello-world/say-hello-agent/artifacts/say-hello-agent.wasm"

jq --arg artifact_path "$ARTIFACT_PATH" '
  .execution.entrypoint.command = $artifact_path
  | {
      scope: "workspace_persisted",
      registry_scope: "public",
      tags: ["youaskm3-http-path"],
      contract: .
    }
' contracts/examples/hello-world/capabilities/say-hello/contract.json \
  > /tmp/traverse-register-say-hello.json

curl -sS \
  -X POST \
  -H 'Content-Type: application/json' \
  --data @/tmp/traverse-register-say-hello.json \
  "$BASE_URL/v1/workspaces/$WORKSPACE_ID/capabilities"
```

Expected result:

- HTTP 201 for the first registration, or HTTP 200 if the same artifact is already registered
- `artifact_type: capability`
- `artifact_id: hello.world.say-hello`
- `links.execute: /v1/workspaces/{workspace_id}/execute`

## Execute The Capability

```bash
curl -sS \
  -X POST \
  -H 'Content-Type: application/json' \
  --data @examples/hello-world/runtime-requests/say-hello.json \
  "$BASE_URL/v1/workspaces/$WORKSPACE_ID/execute" \
  > /tmp/traverse-say-hello-execution.json

cat /tmp/traverse-say-hello-execution.json
```

Expected result:

- HTTP 200
- `api_version: v1`
- `status: succeeded`
- an `execution_id`
- `links.trace`

## Fetch The Public Trace

```bash
EXECUTION_ID="$(jq -r '.execution_id' /tmp/traverse-say-hello-execution.json)"

curl -sS \
  "$BASE_URL/v1/workspaces/$WORKSPACE_ID/traces/$EXECUTION_ID"
```

Expected result:

- HTTP 200
- public trace envelope
- no raw private runtime internals
- OpenTelemetry-compatible trace identifiers when present

## Validation

Run the deterministic app-consumable validation commands from the Traverse repository root:

```bash
bash scripts/ci/app_consumable_acceptance.sh
bash scripts/ci/browser_consumer_package_smoke.sh
bash scripts/ci/repository_checks.sh
```

These checks prove that the released app-consumable path remains documented, the browser-consumer package remains discoverable, and the repository-level documentation assertions still include the canonical HTTP path.

## Related Docs

- [quickstart.md](../quickstart.md)
- [docs/app-consumable-entry-path.md](app-consumable-entry-path.md)
- [docs/app-consumable-acceptance.md](app-consumable-acceptance.md)
- [docs/browser-adapter.md](browser-adapter.md)
- [docs/v0.3.0-downstream-validation-path.md](v0.3.0-downstream-validation-path.md)
- [docs/youaskm3-integration-validation.md](youaskm3-integration-validation.md)
