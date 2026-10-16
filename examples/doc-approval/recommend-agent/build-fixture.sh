#!/usr/bin/env bash

set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
artifact_dir="$script_dir/artifacts"
artifact_path="$artifact_dir/recommend-agent.wasm"

mkdir -p "$artifact_dir"

tmp_out="$(mktemp -d "${TMPDIR:-/tmp}/traverse-fixture.XXXXXX")"
trap 'rm -rf "$tmp_out"' EXIT
rustup run "$(rustup show active-toolchain | awk '{print $1}')" rustc "$script_dir/src/agent.rs" --target wasm32-unknown-unknown --crate-type cdylib -O -o "$tmp_out/agent.wasm"
mv "$tmp_out/agent.wasm" "$artifact_path"

printf 'built %s\n' "$artifact_path"
