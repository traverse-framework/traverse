#!/usr/bin/env bash

set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
cd "${repo_root}"

api_id="$(python3 - "${repo_root}/specs/057-embeddable-runtime-host/embedder-api-1.0.0.json" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as handle:
    print(json.load(handle)["$id"])
PY
)"

if [[ "${api_id}" != "https://traverse.dev/embedder-api/1.0.0" ]]; then
  echo "embedder API id mismatch: ${api_id}" >&2
  exit 1
fi

grep -q "init-shutdown" specs/057-embeddable-runtime-host/conformance.md
grep -q "wasm-capability-submit" specs/057-embeddable-runtime-host/conformance.md
grep -q "compatible-lifecycle" specs/057-embeddable-runtime-host/conformance.md
grep -q "platform-guard" specs/057-embeddable-runtime-host/conformance.md
grep -q "determinism" specs/057-embeddable-runtime-host/conformance.md

cargo test -q -p traverse-registry --test application_manifest \
  defaults_component_execution_mode_to_wasm
cargo test -q -p traverse-registry --test application_manifest \
  loads_compatible_component_manifest_without_wasm_artifact
cargo test -q -p traverse-registry --test application_manifest \
  rejects_compatible_component_without_platforms

validation_json="$(
  cargo run -q -p traverse-cli-rs -- app validate \
    --manifest examples/applications/traverse-starter/app.manifest.json \
    --json
)"

python3 - "${validation_json}" <<'PY'
import json
import sys

data = json.loads(sys.argv[1])
components = data.get("components", [])
if not components:
    raise SystemExit("app validate returned no components")
mode = components[0].get("execution_mode")
if mode != "wasm":
    raise SystemExit(f"expected starter component execution_mode=wasm, got {mode!r}")
if not data.get("digest_verification"):
    raise SystemExit("expected digest verification for wasm component")

print(json.dumps({
    "traverse_embedder_api": "1.0.0",
    "conformance_passed": True,
    "reference": "cli",
}, sort_keys=True))
PY
