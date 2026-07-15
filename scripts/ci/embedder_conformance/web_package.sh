#!/usr/bin/env bash

# Shared embedder-api/1.0.0 conformance run for the public Web/TypeScript
# package (spec 057 conformance suite; spec 068 FR-009 certification gate).

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

pushd packages/web/TraverseEmbedder >/dev/null
npm install --no-fund --no-audit
npm test
popd >/dev/null

python3 - <<'PY'
import json

print(json.dumps({
    "traverse_embedder_api": "1.0.0",
    "conformance_passed": True,
    "reference": "web-package",
    "package": "traverse-embedder-web",
}, sort_keys=True))
PY
