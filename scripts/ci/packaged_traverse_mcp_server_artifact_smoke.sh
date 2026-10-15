#!/usr/bin/env bash

set -euo pipefail

repo_root=$(git rev-parse --show-toplevel)

required_files=(
  "docs/packaged-traverse-mcp-server-artifact.md"
  "docs/mcp-stdio-server.md"
  "docs/mcp-consumption-validation.md"
  "docs/app-consumable-release-checklist.md"
  "docs/mcp-mode-a-release-evidence.md"
  ".github/workflows/mcp-server-publish.yml"
)

for file in "${required_files[@]}"; do
  test -s "${repo_root}/${file}"
done

doc="${repo_root}/docs/packaged-traverse-mcp-server-artifact.md"
grep -q "traverse-mcp-v" "${doc}"
grep -q "SHA256SUMS" "${doc}"
grep -q "gh attestation verify" "${doc}"
grep -q "traverse-mcp --version" "${doc}"
grep -q "mcp-server-publish.yml" "${doc}"
grep -q "bash scripts/ci/packaged_traverse_mcp_server_artifact_smoke.sh" "${doc}"

workflow="${repo_root}/.github/workflows/mcp-server-publish.yml"
grep -q "traverse-mcp-v" "${workflow}"
grep -q -- "--latest=false" "${workflow}"
grep -q "attest-build-provenance" "${workflow}"
grep -q "x86_64-unknown-linux-gnu" "${workflow}"
grep -q "aarch64-apple-darwin" "${workflow}"

# Optional post-publish smoke: download, checksum, --version against a real release.
if [[ -n "${TRAVERSE_MCP_RELEASE_TAG:-}" ]]; then
  target="${TRAVERSE_MCP_TARGET:-}"
  if [[ -z "${target}" ]]; then
    case "$(uname -s)-$(uname -m)" in
      Linux-x86_64) target="x86_64-unknown-linux-gnu" ;;
      Darwin-arm64) target="aarch64-apple-darwin" ;;
      *)
        echo "Set TRAVERSE_MCP_TARGET for this host." >&2
        exit 1
        ;;
    esac
  fi
  work="$(mktemp -d)"
  cleanup() { rm -rf "${work}"; }
  trap cleanup EXIT
  repo="${TRAVERSE_MCP_REPO:-traverse-framework/traverse}"
  base="https://github.com/${repo}/releases/download/${TRAVERSE_MCP_RELEASE_TAG}"
  curl -fsSL -o "${work}/archive.tar.gz" "${base}/traverse-mcp-${target}.tar.gz"
  curl -fsSL -o "${work}/SHA256SUMS" "${base}/SHA256SUMS"
  (
    cd "${work}"
    # Rename so --ignore-missing can match the SUMS entry basename.
    mv archive.tar.gz "traverse-mcp-${target}.tar.gz"
    sha256sum -c --ignore-missing SHA256SUMS
    tar -xzf "traverse-mcp-${target}.tar.gz"
    ./traverse-mcp --version
  )
fi

echo "Traverse packaged MCP server artifact is ready."
