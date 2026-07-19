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
    -p traverse-swift-host --target "${target}"
done

mkdir -p "${output_dir}"
rm -rf "${output_dir}/TraverseSwiftHost.xcframework"
xcodebuild -create-xcframework \
  -library "${repo_root}/target/aarch64-apple-ios/debug/libtraverse_swift_host.a" -headers "${header_dir}" \
  -library "${repo_root}/target/aarch64-apple-ios-sim/debug/libtraverse_swift_host.a" -headers "${header_dir}" \
  -library "${repo_root}/target/aarch64-apple-darwin/debug/libtraverse_swift_host.a" -headers "${header_dir}" \
  -output "${output_dir}/TraverseSwiftHost.xcframework"
