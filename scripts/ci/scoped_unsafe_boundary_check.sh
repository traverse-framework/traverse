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

unsafe_sites=()
while IFS= read -r site; do
  unsafe_sites+=("${site}")
done < <(grep -RIn --include='*.rs' -E '#\[unsafe\(|unsafe[[:space:]]*(\{|fn|impl|trait|extern)' crates || true)
for site in "${unsafe_sites[@]}"; do
  if [[ "${site}" != "${approved_boundary}:"* ]] || [[ "${site}" != *'#[unsafe(no_mangle)]'* ]]; then
    echo "Unsafe syntax is permitted only for no_mangle exports in ${approved_boundary}: ${site}" >&2
    exit 1
  fi
done

if [[ "${#unsafe_sites[@]}" -ne 3 ]]; then
  echo "The audited Swift host must expose exactly three C-ABI symbols." >&2
  exit 1
fi

echo "Scoped unsafe C-ABI boundary check passed."
