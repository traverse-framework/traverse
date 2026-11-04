#!/usr/bin/env bash
# Node and native runs, from an empty folder. Then run `bash src/browser-run.sh` and
# `node src/compare.mjs`.
set -euo pipefail
cd "$(dirname "$0")"
mkdir -p proof
bash fetch.sh
node src/node-run.mjs
export CARGO_HOME="$PWD/.cargo-home" CARGO_TARGET_DIR="$PWD/.cargo-target"
(cd native-host && cargo build --release --locked)
.cargo-target/release/clean-model-execute-native > proof/native.json
