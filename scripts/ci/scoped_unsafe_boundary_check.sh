#!/usr/bin/env bash

set -euo pipefail

readonly swift_boundary="crates/traverse-swift-host/src/lib.rs"
readonly runtime_wasm_boundary="crates/traverse-runtime-wasm/src/lib.rs"
# ADR-0079 / Decision 108: the Android JNI shim for Spec 138 model execution.
readonly android_boundary="crates/traverse-android-host/src/lib.rs"
# ADR-0081 / Decision 111: the .NET P/Invoke shim for Spec 138 model execution.
readonly dotnet_boundary="crates/traverse-dotnet-host/src/lib.rs"
readonly expedition_boundary="crates/traverse-expedition-wasm/src/wasi_stdio.rs"
readonly expedition_root="crates/traverse-expedition-wasm/src/main.rs"
# ADR-0077: the trained digits MLP guest's Spec 138 ABI boundary.
readonly digits_guest_boundary="crates/traverse-digits-mlp-guest/src/abi.rs"
readonly digits_guest_root="crates/traverse-digits-mlp-guest/src/lib.rs"
# ADR-0077 pattern, Decision 105: the generic ONNX runner guest's ABI v2 boundary.
readonly onnx_runner_boundary="crates/traverse-onnx-runner-guest/src/abi.rs"
readonly onnx_runner_root="crates/traverse-onnx-runner-guest/src/lib.rs"

if ! grep -Fqx 'unsafe_code = "deny"' Cargo.toml; then
  echo "Workspace unsafe-code lint must remain set to deny." >&2
  exit 1
fi

# ADR-0073 / spec 1402 FR-011: a second, independently audited crate-level
# opt-out for the runtime.wasm nested-executor's C-ABI export boundary —
# same pattern as the Swift boundary, not a general loosening.
allowed_opt_outs=("${swift_boundary}" "${runtime_wasm_boundary}" "${android_boundary}" "${dotnet_boundary}")
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
  if [[ "${path}" != "${swift_boundary}" && "${path}" != "${runtime_wasm_boundary}" && "${path}" != "${android_boundary}" && "${path}" != "${dotnet_boundary}" && "${path}" != "${expedition_boundary}" && "${path}" != "${digits_guest_boundary}" && "${path}" != "${onnx_runner_boundary}" ]]; then
    echo "Unsafe syntax is permitted only in ${swift_boundary}, ${runtime_wasm_boundary}, ${android_boundary}, ${dotnet_boundary}, ${expedition_boundary}, ${digits_guest_boundary}, or ${onnx_runner_boundary}." >&2
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

# ADR-0077: the digits guest scopes `unsafe` to one `abi` module exposing one
# `model_execute` symbol with exactly two audited slice views.
if [[ -f "${digits_guest_boundary}" ]]; then
  if ! grep -Fqx '#[allow(unsafe_code)]' "${digits_guest_root}"; then
    echo "The digits guest must scope its unsafe-code allowance to the abi module." >&2
    exit 1
  fi
  if [[ "$(grep -Fc 'mod abi;' "${digits_guest_root}")" -ne 1 ]]; then
    echo "The digits guest must expose exactly one abi module." >&2
    exit 1
  fi
  if [[ "$(grep -Fc '#[unsafe(no_mangle)]' "${digits_guest_boundary}")" -ne 1 ]] ||
    [[ "$(grep -Fc 'fn model_execute(' "${digits_guest_boundary}")" -ne 1 ]]; then
    echo "The digits guest boundary must export exactly one model_execute symbol." >&2
    exit 1
  fi
  if [[ "$(grep -Ec 'unsafe[[:space:]]*\{' "${digits_guest_boundary}")" -ne 2 ]]; then
    echo "The digits guest boundary must contain exactly two audited unsafe blocks." >&2
    exit 1
  fi
  if grep -Eq 'extern[[:space:]]*"C"[[:space:]]*\{|#\[link' "${digits_guest_boundary}"; then
    echo "The digits guest boundary must not import host functions." >&2
    exit 1
  fi
fi

# Decision 105: the ONNX runner scopes `unsafe` to one `abi` module with the
# blob static, the three guest ABI v2 functions, and four audited unsafe
# blocks (blob pointer read, blob view, input view, output view).
if [[ -f "${onnx_runner_boundary}" ]]; then
  if ! grep -Fqx '#[allow(unsafe_code)]' "${onnx_runner_root}" ||
    [[ "$(grep -Fc 'mod abi;' "${onnx_runner_root}")" -ne 1 ]]; then
    echo "The ONNX runner must scope its unsafe-code allowance to exactly one abi module." >&2
    exit 1
  fi
  if [[ "$(grep -Fc '#[unsafe(no_mangle)]' "${onnx_runner_boundary}")" -ne 4 ]]; then
    echo "The ONNX runner boundary must export exactly four audited symbols." >&2
    exit 1
  fi
  for symbol in 'static TRAVERSE_MODEL_BLOB:' 'fn model_alloc(' 'fn model_prepare(' 'fn model_execute('; do
    if [[ "$(grep -Fc "${symbol}" "${onnx_runner_boundary}")" -ne 1 ]]; then
      echo "Missing or duplicate audited ONNX runner symbol: ${symbol}" >&2
      exit 1
    fi
  done
  if [[ "$(grep -Ec 'unsafe[[:space:]]*\{' "${onnx_runner_boundary}")" -ne 4 ]]; then
    echo "The ONNX runner boundary must contain exactly four audited unsafe blocks." >&2
    exit 1
  fi
  if grep -Eq 'extern[[:space:]]*"C"[[:space:]]*\{|#\[link' "${onnx_runner_boundary}"; then
    echo "The ONNX runner boundary must not import host functions." >&2
    exit 1
  fi
fi

exports=(
  traverse_swift_host_abi_version
  traverse_swift_host_create
  traverse_swift_host_invoke
  traverse_swift_host_destroy
  traverse_swift_host_status_message
  traverse_swift_host_model_call
)
for symbol in "${exports[@]}"; do
  if [[ "$(grep -Fc "fn ${symbol}" "${swift_boundary}")" -ne 1 ]]; then
    echo "Missing or duplicate audited C-ABI symbol: ${symbol}" >&2
    exit 1
  fi
done
# ADR-0015 five symbols + ADR-0078 / Decision 104 `traverse_swift_host_model_call`.
if [[ "$(grep -Fc '#[unsafe(no_mangle)]' "${swift_boundary}")" -ne 6 ]]; then
  echo "The audited Swift host must expose exactly six production C-ABI symbols." >&2
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

# ADR-0079: exactly one exported JNI method; its only unsafe syntax is the
# no_mangle attribute (byte conversion goes through the `jni` crate).
if [[ "$(grep -Fc '#[unsafe(no_mangle)]' "${android_boundary}")" -ne 1 ]] ||
  [[ "$(grep -Fc 'fn Java_dev_traverse_embedder_ExactModelNative_modelCall' "${android_boundary}")" -ne 1 ]]; then
  echo "The Android host must export exactly one audited JNI method (ExactModelNative.modelCall)." >&2
  exit 1
fi
if grep -Eq 'unsafe[[:space:]]*(\{|fn|impl|trait|extern)' "${android_boundary}"; then
  echo "The Android host may not contain unsafe blocks, functions, impls, or extern blocks." >&2
  exit 1
fi

# ADR-0081: exactly two exported C functions, the framed call and its free.
if [[ "$(grep -Fc '#[unsafe(no_mangle)]' "${dotnet_boundary}")" -ne 2 ]] ||
  [[ "$(grep -Fc 'pub unsafe extern "C" fn traverse_dotnet_host_model_call(' "${dotnet_boundary}")" -ne 1 ]] ||
  [[ "$(grep -Fc 'pub unsafe extern "C" fn traverse_dotnet_host_free(' "${dotnet_boundary}")" -ne 1 ]]; then
  echo "The .NET host must export exactly two audited C functions (model_call and free)." >&2
  exit 1
fi

echo "Scoped unsafe C-ABI boundary check passed."
