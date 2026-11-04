#!/usr/bin/env bash
# Downloads every input from its published location. No git checkout is used.
set -euo pipefail
cd "$(dirname "$0")"
mkdir -p published/package bundle/runtime bundle/components/model-execute
TAG=https://raw.githubusercontent.com/traverse-framework/traverse/v0.16.1
for f in model.manifest.json model.wasm model.sig.json; do
  curl -fsSL "$TAG/fixtures/models/digits-onnx-1.0.0/$f" -o "published/package/$f"
done
curl -fsSL "$TAG/fixtures/models/conformance/signed-digits-onnx.json" -o published/package/vector.json
curl -fsSL "$TAG/fixtures/cross-host/model-execute-v1/input.json" -o published/input.json
curl -fsSL "$TAG/runtime/runtime.wasm" -o bundle/runtime/runtime.wasm
curl -fsSL "$TAG/runtime/runtime.wasm.sha256" -o bundle/runtime/runtime.wasm.sha256
curl -fsSL "$TAG/runtime/runtime-release.json" -o published/runtime-release.json
curl -fsSL https://github.com/traverse-framework/registry/releases/download/artifacts/model.execute-1.0.0/model-execute-agent.wasm \
  -o bundle/components/model-execute/model-execute-agent.wasm
curl -fsSL https://registry.traverse-framework.com/catalog.json -o published/catalog.json
node src/make-bundle.mjs
npm install --no-audit --no-fund --cache "$PWD/.npm-cache" --save-exact traverse-embedder-web@0.16.1
