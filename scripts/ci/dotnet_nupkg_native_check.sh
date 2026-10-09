#!/usr/bin/env bash
# Decision 111 / #1643: prove a packed TraverseEmbedder .nupkg carries the
# traverse-dotnet-host model engine for every shipped runtime ID, and that the
# linux-x64 library exports exactly the two audited C functions (ADR-0081).
#
#   bash scripts/ci/dotnet_nupkg_native_check.sh <TraverseEmbedder.x.y.z.nupkg>
set -euo pipefail

nupkg="${1:?usage: dotnet_nupkg_native_check.sh <nupkg>}"
expected=(
  "runtimes/win-x64/native/traverse_dotnet_host.dll"
  "runtimes/win-arm64/native/traverse_dotnet_host.dll"
  "runtimes/linux-x64/native/libtraverse_dotnet_host.so"
)
listing="$(unzip -Z1 "${nupkg}")"
for entry in "${expected[@]}"; do
  if ! grep -Fxq "${entry}" <<<"${listing}"; then
    echo "${nupkg} is missing ${entry}" >&2
    exit 1
  fi
  if [[ "$(unzip -p "${nupkg}" "${entry}" | wc -c)" -eq 0 ]]; then
    echo "${nupkg} has an empty ${entry}" >&2
    exit 1
  fi
done
extra="$(grep -E '^runtimes/' <<<"${listing}" | grep -Fxv -f <(printf '%s\n' "${expected[@]}") || true)"
if [[ -n "${extra}" ]]; then
  echo "${nupkg} carries unexpected native assets: ${extra}" >&2
  exit 1
fi

scratch="$(mktemp -d)"
trap 'rm -rf "${scratch}"' EXIT
unzip -p "${nupkg}" "runtimes/linux-x64/native/libtraverse_dotnet_host.so" >"${scratch}/lib.so"
exported="$(nm -D --defined-only "${scratch}/lib.so" | awk '$3 ~ /^traverse_dotnet_host_/ { print $3 }' | sort | tr '\n' ' ')"
if [[ "${exported}" != "traverse_dotnet_host_free traverse_dotnet_host_model_call " ]]; then
  echo "linux-x64 library must export exactly the two audited functions, found: ${exported:-nothing}" >&2
  exit 1
fi
echo "${nupkg}: native model host present for win-x64, win-arm64, linux-x64"
