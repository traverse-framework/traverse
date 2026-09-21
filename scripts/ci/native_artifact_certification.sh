#!/usr/bin/env bash

# Runs the generated production bridge through every native package without a
# CLI, HTTP, or other sidecar.  The package tests consume the one artifact
# directory supplied through TRAVERSE_NATIVE_ARTIFACT_ROOT.

set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
artifact_root="$(mktemp -d "${TMPDIR:-/tmp}/traverse-native-artifact.XXXXXX")"
cleanup() { rm -rf "${artifact_root}"; }
trap cleanup EXIT

# traverse-native-bridge builds the real crates/traverse-runtime-wasm
# artifact for wasm32-unknown-unknown; this job's pinned toolchain does not
# install that target by default. Idempotent no-op if already present.
rustup target add wasm32-unknown-unknown

cargo run -q -p traverse-native-bridge --bin traverse-native-bridge -- "${artifact_root}/runtime"

python3 - "${artifact_root}/runtime" <<'PY'
import hashlib
import json
import pathlib
import sys

runtime_dir = pathlib.Path(sys.argv[1])
wasm = (runtime_dir / "runtime.wasm").read_bytes()
release = json.loads((runtime_dir / "runtime-release.json").read_text(encoding="utf-8"))
if release["sha256"] != hashlib.sha256(wasm).hexdigest():
    raise SystemExit("runtime-release.json digest does not match runtime.wasm")
if release["bridge_version"] != "1.1.0" or release["bridge_abi_version"] != 10100:
    raise SystemExit("runtime-release.json bridge identity is incompatible")
if not release["runtime_version"]:
    raise SystemExit("runtime-release.json runtime version is required")
print(json.dumps({"artifact": "runtime.wasm", "release": release, "digest_verified": True}, sort_keys=True))
PY

export TRAVERSE_NATIVE_ARTIFACT_ROOT="${artifact_root}"
swift test --package-path "${repo_root}/packages/swift/TraverseEmbedder"
pushd "${repo_root}/packages/kotlin/TraverseEmbedder" >/dev/null
"${GRADLE:-gradle}" --no-daemon :traverse-embedder:testDebugUnitTest
popd >/dev/null
dotnet test "${repo_root}/packages/dotnet/TraverseEmbedder/TraverseEmbedder.Tests/TraverseEmbedder.Tests.csproj"
# The web embedder runs the same shared app state-machine golden against the generated artifact
# (spec 139 / 140 FR-013, #1502); its checked-in test runtime.wasm predates that state machine.
pushd "${repo_root}/packages/web/TraverseEmbedder" >/dev/null
npm install --no-fund --no-audit
npm run build
node --test tests/appStateMachineConformance.test.mjs
popd >/dev/null

python3 - <<'PY'
import json
print(json.dumps({
    "traverse_embedder_api": "1.0.0",
    "bridge_version": "1.1.0",
    "hosts": ["swift", "kotlin", "dotnet", "web"],
    "no_sidecar": True,
    "conformance_passed": True,
}, sort_keys=True))
PY
