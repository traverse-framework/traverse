#!/usr/bin/env bash

set -euo pipefail

readonly approved_boundary="crates/traverse-swift-host/src/lib.rs"

if ! grep -Fqx 'unsafe_code = "deny"' Cargo.toml; then
  echo "Workspace unsafe-code lint must remain set to deny." >&2
  exit 1
fi

opt_outs=()
while IFS= read -r path; do
  opt_outs+=("${path}")
done < <(grep -RIlF --include='*.rs' '#![allow(unsafe_code)]' crates || true)
if [[ "${#opt_outs[@]}" -ne 1 || "${opt_outs[0]:-}" != "${approved_boundary}" ]]; then
  echo "Only ${approved_boundary} may opt out of the workspace unsafe-code lint." >&2
  exit 1
fi

unsafe_files=()
while IFS= read -r path; do
  unsafe_files+=("${path}")
done < <(grep -RIl --include='*.rs' -E '#\[unsafe\(|unsafe[[:space:]]*(\{|fn|impl|trait|extern)' crates || true)
if [[ "${#unsafe_files[@]}" -ne 1 || "${unsafe_files[0]:-}" != "${approved_boundary}" ]]; then
  echo "Unsafe syntax is permitted only in ${approved_boundary}." >&2
  exit 1
fi

exports=(
  traverse_swift_host_abi_version
  traverse_swift_host_create
  traverse_swift_host_invoke
  traverse_swift_host_destroy
  traverse_swift_host_status_message
)
for symbol in "${exports[@]}"; do
  if [[ "$(grep -Fc "fn ${symbol}" "${approved_boundary}")" -ne 1 ]]; then
    echo "Missing or duplicate audited C-ABI symbol: ${symbol}" >&2
    exit 1
  fi
done
if [[ "$(grep -Fc '#[unsafe(no_mangle)]' "${approved_boundary}")" -ne 5 ]]; then
  echo "The audited Swift host must expose exactly five production C-ABI symbols." >&2
  exit 1
fi

echo "Scoped unsafe C-ABI boundary check passed."
