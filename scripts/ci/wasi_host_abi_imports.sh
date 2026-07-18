#!/usr/bin/env bash

set -euo pipefail

repo_root="${TRAVERSE_REPO_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "${repo_root}"

wasm_artifacts=()
while IFS= read -r artifact; do
  wasm_artifacts+=("${artifact}")
done < <(find examples -type f -name '*.wasm' | sort)

if [[ "${#wasm_artifacts[@]}" -eq 0 ]]; then
  echo "No checked-in WASM artifacts found for Traverse Host ABI verification." >&2
  exit 1
fi

cargo run -q -p traverse-cli-rs -- wasm abi verify "${wasm_artifacts[@]}"
