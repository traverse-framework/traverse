#!/usr/bin/env bash
# Decision 108 (#1580): the published AAR must carry the JNI model host for
# both shipped ABIs; without them every model call fails closed with
# engine_unavailable on device.
#
#   bash scripts/ci/android_aar_check.sh <path-to.aar>
set -euo pipefail

aar="${1:?usage: android_aar_check.sh <path-to.aar>}"
listing="$(unzip -l "${aar}")"
for abi in arm64-v8a x86_64; do
  if ! grep -q "jni/${abi}/libtraverse_android_host.so" <<<"${listing}"; then
    echo "${aar} is missing jni/${abi}/libtraverse_android_host.so" >&2
    exit 1
  fi
done
echo "AAR check passed: ${aar} carries libtraverse_android_host.so for arm64-v8a and x86_64."
