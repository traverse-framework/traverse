#!/usr/bin/env bash

set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
cd "${repo_root}"

bash scripts/ci/downstream_app_bundle_registration_smoke.sh
bash scripts/ci/downstream_public_app_registration_smoke.sh
bash scripts/ci/downstream_wasm_workflow_smoke.sh
bash scripts/ci/downstream_model_dependency_smoke.sh
bash scripts/ci/downstream_http_json_smoke.sh
bash scripts/ci/downstream_mcp_smoke.sh

echo "downstream app MVP conformance suite passed."
