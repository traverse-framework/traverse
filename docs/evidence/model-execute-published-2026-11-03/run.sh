#!/usr/bin/env bash
# Node and native runs, from an empty folder. Run the browser with `node src/server.mjs`, open
# http://127.0.0.1:8791/ until the page says DONE, then run `node src/compare.mjs`.
set -euo pipefail
cd "$(dirname "$0")"
mkdir -p proof
bash fetch.sh
node src/node-run.mjs
export CARGO_HOME="$PWD/.cargo-home" CARGO_TARGET_DIR="$PWD/.cargo-target"
(cd native-host && cargo build --release)
.cargo-target/release/clean-model-execute-native > proof/native.json
