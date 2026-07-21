#!/usr/bin/env bash

set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
artifact_dir="$script_dir/artifacts"
artifact_path="$artifact_dir/analyze-agent.wasm"

mkdir -p "$artifact_dir"

rustup run "$(rustup show active-toolchain | awk '{print $1}')" rustc \
  "$script_dir/src/agent.rs" --target wasm32-unknown-unknown --crate-type cdylib -O \
  -o "$artifact_path"

printf 'built %s\n' "$artifact_path"
