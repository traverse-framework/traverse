#!/usr/bin/env bash
# Decision 108: build the Android JNI model host for the *host* JVM so Kotlin
# unit tests (testDebugUnitTest) load the identical Rust + JNI code that ships
# in the AAR (whose arm64-v8a/x86_64 .so files cargo-ndk builds at publish).
#
#   bash scripts/build_android_host_jvm.sh <output-library-path>
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
output="${1:?usage: build_android_host_jvm.sh <output-library-path>}"

cargo build --quiet --locked --release -p traverse-android-host --manifest-path "${repo_root}/Cargo.toml"
target_dir="$(cargo metadata --no-deps --format-version 1 --manifest-path "${repo_root}/Cargo.toml" \
  | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"
case "$(uname -s)" in
  Darwin) built="${target_dir}/release/libtraverse_android_host.dylib" ;;
  *) built="${target_dir}/release/libtraverse_android_host.so" ;;
esac
mkdir -p "$(dirname "${output}")"
cp "${built}" "${output}"
echo "host JVM model library: ${output}"
