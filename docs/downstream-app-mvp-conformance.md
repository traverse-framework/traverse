# Downstream App MVP Conformance

This document defines the Traverse-side conformance path for the first downstream app MVP boundary governed by `044-application-bundle-manifest`, `045-governed-model-dependency-resolution`, and `046-public-cli-app-registration`.

The suite proves that a downstream app bundle can be validated and registered, the public CLI app registration path writes durable local workspace state, the runtime can load that state, a real WASM workflow can execute, governed model dependency readiness and resolution evidence is present, and the same public behavior is visible through HTTP/JSON and MCP surfaces.

## CI-Safe Suite

Run the deterministic CI-safe suite with:

```bash
bash scripts/ci/downstream_app_mvp_conformance.sh
```

That aggregate runs:

```bash
bash scripts/ci/downstream_app_bundle_registration_smoke.sh
bash scripts/ci/downstream_public_app_registration_smoke.sh
bash scripts/ci/downstream_wasm_workflow_smoke.sh
bash scripts/ci/downstream_model_dependency_smoke.sh
bash scripts/ci/downstream_http_json_smoke.sh
bash scripts/ci/downstream_mcp_smoke.sh
```

## Evidence

The CI-safe suite verifies:

- the checked-in downstream application manifest references real component manifests and a real WASM binary
- `traverse-cli app validate --manifest <path> --json` validates the downstream app manifest through the public CLI surface
- `traverse-cli app register --manifest <path> --workspace <workspace-id> --json` writes durable workspace app state under `.traverse/workspaces/<workspace-id>/apps/.../registration.json`
- the runtime loads the CLI-produced durable workspace state and discovers the registered app capability and workflow
- app bundle registration is atomic and records effective non-sensitive config
- model dependency schema and readiness evidence are validated
- execution-time model dependency resolution emits public non-secret evidence
- a real WASM workflow executes and writes trace evidence
- HTTP/JSON execution returns a completed trace and public trace fetch hides internal/private fields
- MCP consumption and observation reports expose the governed model-resolution evidence

## Local Provider Check

The default model dependency smoke uses deterministic local HTTP provider fixtures. To require a real local Ollama provider in a developer environment, run:

```bash
TRAVERSE_RUN_LOCAL_OLLAMA_CONFORMANCE=1 bash scripts/ci/downstream_model_dependency_smoke.sh
```

Use this local-provider-required mode only when the machine has Ollama available with the expected model. CI should keep the default deterministic mode.

## Expected Outcomes

- All scripts exit with status `0`.
- Public evidence includes app, workflow, component, and model selection metadata needed for audit.
- Public evidence does not include workspace-local secrets, private prompts, private source text, or raw model credentials.
- Failures identify the broken surface: bundle registration, public CLI app registration, runtime workspace-state loading, WASM workflow, model dependency resolution, HTTP/JSON, or MCP.
