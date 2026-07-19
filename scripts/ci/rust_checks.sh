#!/usr/bin/env bash

set -euo pipefail

cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

bash scripts/ci/scoped_unsafe_boundary_check.sh

bash scripts/ci/contractual_enforcement_gate.sh

echo "Rust checks passed."
