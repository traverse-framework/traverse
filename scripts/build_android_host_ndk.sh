#!/usr/bin/env bash
# Decision 108 (#1580 slice 3): cross-build traverse-android-host for the AAR.
#
# Builds arm64-v8a (phones) and x86_64 (emulators) at API 28 (the embedder's
# minSdk) with cargo-ndk into <jniLibs-dir>/<abi>/libtraverse_android_host.so,
# links them with 16 KB page alignment (required for Android 15+ devices), and
# checks that each library exports exactly the one audited JNI method
# (ADR-0079). Needs an Android NDK (ANDROID_NDK_HOME or
# ANDROID_NDK_LATEST_HOME, as on GitHub runners), cargo-ndk, and the
# aarch64-linux-android / x86_64-linux-android Rust targets.
#
#   bash scripts/build_android_host_ndk.sh packages/kotlin/TraverseEmbedder/traverse-embedder/build/jniLibs
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
out_dir="${1:?usage: build_android_host_ndk.sh <jniLibs-dir>}"
ndk="${ANDROID_NDK_HOME:-${ANDROID_NDK_LATEST_HOME:-}}"
if [[ -z "${ndk}" || ! -d "${ndk}" ]]; then
  echo "Android NDK not found: set ANDROID_NDK_HOME (or ANDROID_NDK_LATEST_HOME)." >&2
  exit 1
fi
export ANDROID_NDK_HOME="${ndk}"
export RUSTFLAGS="${RUSTFLAGS:-} -C link-arg=-Wl,-z,max-page-size=16384"

mkdir -p "${out_dir}"
cargo ndk --manifest-path "${repo_root}/Cargo.toml" -t arm64-v8a -t x86_64 -P 28 -o "${out_dir}" \
  build --release --locked -p traverse-android-host

nm_tool="$(find "${ndk}/toolchains/llvm/prebuilt" -name llvm-nm -type f | head -1)"
readelf_tool="$(find "${ndk}/toolchains/llvm/prebuilt" -name llvm-readelf -type f | head -1)"
for abi in arm64-v8a x86_64; do
  library="${out_dir}/${abi}/libtraverse_android_host.so"
  if [[ ! -f "${library}" ]]; then
    echo "missing ${library}" >&2
    exit 1
  fi
  exported="$("${nm_tool}" -D --defined-only "${library}" | awk '$3 ~ /^Java_/ { print $3 }')"
  if [[ "${exported}" != "Java_dev_traverse_embedder_ExactModelNative_modelCall" ]]; then
    echo "${library} must export exactly the audited JNI method, found: ${exported:-nothing}" >&2
    exit 1
  fi
  if "${readelf_tool}" -lW "${library}" | awk '$1 == "LOAD" && $NF != "0x4000" { bad = 1 } END { exit !bad }'; then
    echo "${library} has a LOAD segment that is not 16 KB aligned" >&2
    exit 1
  fi
  echo "${abi}: $(wc -c < "${library}") bytes, exports ${exported}, 16 KB aligned"
done
