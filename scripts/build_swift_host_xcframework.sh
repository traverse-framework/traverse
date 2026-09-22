#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
toolchain="1.94.0-aarch64-apple-darwin"
rustc_path="/Users/enricopiovesan/.rustup/toolchains/${toolchain}/bin/rustc"
output_dir="${repo_root}/target/apple"
header_dir="${repo_root}/crates/traverse-swift-host/include"

for target in aarch64-apple-ios aarch64-apple-ios-sim aarch64-apple-darwin; do
  RUSTC="${rustc_path}" rustup run "${toolchain}" cargo build \
    --manifest-path "${repo_root}/Cargo.toml" \
    -p traverse-swift-host --release --target "${target}"
done

# Resolve the real cargo target directory rather than assuming
# ${repo_root}/target: CARGO_TARGET_DIR or .cargo/config.toml's
# build.target-dir may point elsewhere (e.g. a cache shared across
# repos/worktrees).
cargo_target_dir="$(rustup run "${toolchain}" cargo metadata \
  --no-deps --format-version 1 --manifest-path "${repo_root}/Cargo.toml" \
  | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"

mkdir -p "${output_dir}"
rm -rf "${output_dir}/TraverseSwiftHost.xcframework"
xcodebuild -create-xcframework \
  -library "${cargo_target_dir}/aarch64-apple-ios/release/libtraverse_swift_host.a" -headers "${header_dir}" \
  -library "${cargo_target_dir}/aarch64-apple-ios-sim/release/libtraverse_swift_host.a" -headers "${header_dir}" \
  -library "${cargo_target_dir}/aarch64-apple-darwin/release/libtraverse_swift_host.a" -headers "${header_dir}" \
  -output "${output_dir}/TraverseSwiftHost.xcframework"
