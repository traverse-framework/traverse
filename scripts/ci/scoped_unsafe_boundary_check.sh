#!/usr/bin/env bash

set -euo pipefail

readonly swift_boundary="crates/traverse-swift-host/src/lib.rs"
readonly runtime_wasm_boundary="crates/traverse-runtime-wasm/src/lib.rs"
readonly expedition_boundary="crates/traverse-expedition-wasm/src/wasi_stdio.rs"
readonly expedition_root="crates/traverse-expedition-wasm/src/main.rs"

if ! grep -Fqx 'unsafe_code = "deny"' Cargo.toml; then
  echo "Workspace unsafe-code lint must remain set to deny." >&2
  exit 1
fi

# ADR-0073 / spec 1402 FR-011: a second, independently audited crate-level
# opt-out for the runtime.wasm nested-executor's C-ABI export boundary —
# same pattern as the Swift boundary, not a general loosening.
allowed_opt_outs=("${swift_boundary}" "${runtime_wasm_boundary}")
opt_outs=()
while IFS= read -r path; do
  opt_outs+=("${path}")
done < <(grep -RIlF --include='*.rs' '#![allow(unsafe_code)]' crates | sort || true)
expected_opt_outs=()
while IFS= read -r path; do
  expected_opt_outs+=("${path}")
done < <(printf '%s\n' "${allowed_opt_outs[@]}" | sort)
if [[ "${opt_outs[*]:-}" != "${expected_opt_outs[*]:-}" ]]; then
  echo "Only ${allowed_opt_outs[*]} may use a crate-level unsafe-code opt-out." >&2
  exit 1
fi

unsafe_files=()
while IFS= read -r path; do
  unsafe_files+=("${path}")
done < <(grep -RIl --include='*.rs' -E '#\[unsafe\(|unsafe[[:space:]]*(\{|fn|impl|trait|extern)' crates || true)
for path in "${unsafe_files[@]}"; do
  if [[ "${path}" != "${swift_boundary}" && "${path}" != "${runtime_wasm_boundary}" && "${path}" != "${expedition_boundary}" ]]; then
    echo "Unsafe syntax is permitted only in ${swift_boundary}, ${runtime_wasm_boundary}, or ${expedition_boundary}." >&2
    exit 1
  fi
done

if [[ -f "${expedition_boundary}" ]]; then
  if ! grep -Fqx '#[allow(unsafe_code)]' "${expedition_root}"; then
    echo "The expedition guest must scope its unsafe-code allowance to wasi_stdio." >&2
    exit 1
  fi
  if [[ "$(grep -Fc 'mod wasi_stdio;' "${expedition_root}")" -ne 1 ]]; then
    echo "The expedition guest must expose exactly one wasi_stdio module." >&2
    exit 1
  fi
  if [[ "$(grep -Fc 'wasi_snapshot_preview1' "${expedition_boundary}")" -ne 1 ]]; then
    echo "The expedition boundary must import exactly one WASI Preview 1 module." >&2
    exit 1
  fi
  for symbol in fd_read fd_write proc_exit; do
    if [[ "$(grep -Ec "fn ${symbol}\\(" "${expedition_boundary}")" -ne 1 ]]; then
      echo "Missing or duplicate audited WASI symbol: ${symbol}" >&2
      exit 1
    fi
  done
  if [[ "$(grep -Ec '^[[:space:]]*(pub[[:space:]]+)?unsafe[[:space:]]+extern[[:space:]]+"C"' "${expedition_boundary}")" -ne 1 ]]; then
    echo "The expedition boundary must contain exactly one audited unsafe extern block." >&2
    exit 1
  fi
  if grep -Eq 'environ_get|path_|fd_(open|close|seek|sync)|random_get|clock_|sock_|proc_raise' "${expedition_boundary}"; then
    echo "The expedition boundary imports a forbidden WASI capability." >&2
    exit 1
  fi
fi

exports=(
  traverse_swift_host_abi_version
  traverse_swift_host_create
  traverse_swift_host_invoke
  traverse_swift_host_destroy
  traverse_swift_host_status_message
)
for symbol in "${exports[@]}"; do
  if [[ "$(grep -Fc "fn ${symbol}" "${swift_boundary}")" -ne 1 ]]; then
    echo "Missing or duplicate audited C-ABI symbol: ${symbol}" >&2
    exit 1
  fi
done
if [[ "$(grep -Fc '#[unsafe(no_mangle)]' "${swift_boundary}")" -ne 5 ]]; then
  echo "The audited Swift host must expose exactly five production C-ABI symbols." >&2
  exit 1
fi

runtime_wasm_exports=(
  traverse_bridge_abi_version
  traverse_alloc
  traverse_dealloc
  traverse_init
  traverse_submit
  traverse_next_event
  traverse_cancel
  traverse_shutdown
  traverse_compatible_start
  traverse_compatible_stop
  traverse_compatible_kill
)
for symbol in "${runtime_wasm_exports[@]}"; do
  if [[ "$(grep -Fc "fn ${symbol}" "${runtime_wasm_boundary}")" -ne 1 ]]; then
    echo "Missing or duplicate audited C-ABI symbol: ${symbol}" >&2
    exit 1
  fi
done
if [[ "$(grep -Fc '#[unsafe(no_mangle)]' "${runtime_wasm_boundary}")" -ne "${#runtime_wasm_exports[@]}" ]]; then
  echo "runtime.wasm must expose exactly ${#runtime_wasm_exports[@]} production C-ABI symbols (spec 071 FR-006)." >&2
  exit 1
fi

echo "Scoped unsafe C-ABI boundary check passed."
