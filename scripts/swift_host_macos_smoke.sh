#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
bash "${repo_root}/scripts/build_swift_host_xcframework.sh"

swiftc \
  -I "${repo_root}/crates/traverse-swift-host/include" \
  -L "${repo_root}/target/aarch64-apple-darwin/debug" \
  -ltraverse_swift_host \
  "${repo_root}/examples/swift-wasmi-proof/main.swift" \
  -o "${repo_root}/target/apple/swift-host-smoke"
"${repo_root}/target/apple/swift-host-smoke"
