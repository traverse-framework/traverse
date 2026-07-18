#!/usr/bin/env bash

set -euo pipefail

crates=(
  "traverse-contracts"
  "traverse-registry"
  "traverse-runtime"
  "traverse-embedder"
  "traverse-mcp"
  "traverse-cli-rs"
  "traverse-expedition-wasm"
)

if [[ -n "${TRAVERSE_PUBLISH_CRATES:-}" ]]; then
  read -r -a crates <<< "${TRAVERSE_PUBLISH_CRATES}"
fi

repo_root="$(git rev-parse --show-toplevel)"
cd "${repo_root}"

workspace_version="$(awk '
  /^\[workspace\.package\]$/ { in_workspace_package = 1; next }
  /^\[/ { in_workspace_package = 0 }
  in_workspace_package && $1 == "version" {
    gsub(/"/, "", $3)
    print $3
    found = 1
    exit
  }
  END { if (!found) exit 1 }
' Cargo.toml)"

dry_run="${TRAVERSE_PUBLISH_DRY_RUN:-0}"
dry_run_before="${TRAVERSE_PUBLISH_DRY_RUN_BEFORE:-0}"
no_verify="${TRAVERSE_PUBLISH_NO_VERIFY:-0}"
allow_dirty="${TRAVERSE_PUBLISH_ALLOW_DIRTY:-0}"
sleep_seconds="${TRAVERSE_PUBLISH_SLEEP_SECONDS:-30}"

publish_args=()
dry_run_publish_args=("--dry-run" "--no-verify")
if [[ "${dry_run}" == "1" ]]; then
  publish_args+=("--dry-run")
fi
if [[ "${no_verify}" == "1" ]]; then
  publish_args+=("--no-verify")
fi
if [[ "${allow_dirty}" == "1" ]]; then
  publish_args+=("--allow-dirty")
  dry_run_publish_args+=("--allow-dirty")
fi

if [[ "${dry_run}" != "1" && -z "${CARGO_REGISTRY_TOKEN:-}" ]]; then
  echo "CARGO_REGISTRY_TOKEN is required for real publishing." >&2
  exit 1
fi

publish_one() {
  local crate="$1"
  local output_file
  local dry_run_file
  local rc

  if [[ "${dry_run}" != "1" && "${dry_run_before}" == "1" ]]; then
    dry_run_file="$(mktemp)"
    echo "Dry-running ${crate} ${workspace_version}..."
    if cargo publish -p "${crate}" "${dry_run_publish_args[@]}" >"${dry_run_file}" 2>&1; then
      cat "${dry_run_file}"
      rm -f "${dry_run_file}"
    else
      rc="$?"
      cat "${dry_run_file}" >&2
      rm -f "${dry_run_file}"
      return "${rc}"
    fi
  fi

  output_file="$(mktemp)"

  echo "Publishing ${crate} ${workspace_version}..."
  if [[ "${#publish_args[@]}" -gt 0 ]]; then
    if cargo publish -p "${crate}" "${publish_args[@]}" >"${output_file}" 2>&1; then
      rc=0
    else
      rc="$?"
    fi
  else
    if cargo publish -p "${crate}" >"${output_file}" 2>&1; then
      rc=0
    else
      rc="$?"
    fi
  fi

  if [[ "${rc}" -eq 0 ]]; then
    cat "${output_file}"
    rm -f "${output_file}"
    return 0
  fi

  cat "${output_file}" >&2
  if grep -Eiq "already uploaded|crate version .* is already uploaded" "${output_file}"; then
    echo "${crate} ${workspace_version} is already published; continuing."
    rm -f "${output_file}"
    return 0
  fi

  rm -f "${output_file}"
  return "${rc}"
}

sparse_index_path() {
  local crate="$1"
  local len="${#crate}"

  if [[ "${len}" -eq 1 ]]; then
    echo "1/${crate}"
  elif [[ "${len}" -eq 2 ]]; then
    echo "2/${crate}"
  elif [[ "${len}" -eq 3 ]]; then
    echo "3/${crate:0:1}/${crate}"
  else
    echo "${crate:0:2}/${crate:2:2}/${crate}"
  fi
}

confirm_published() {
  local crate="$1"
  local path

  if [[ "${dry_run}" == "1" ]]; then
    return 0
  fi

  # crates.io's search index (used by `cargo search`) can lag well behind
  # publish for a brand-new crate name, sometimes past 10 minutes. The
  # sparse index that cargo itself uses for dependency resolution updates
  # within seconds of a real publish, so check that directly instead.
  path="$(sparse_index_path "${crate}")"

  for attempt in {1..20}; do
    if curl -fsS "https://index.crates.io/${path}" 2>/dev/null | grep -Fq "\"vers\":\"${workspace_version}\""; then
      echo "Confirmed ${crate} ${workspace_version} on crates.io."
      return 0
    fi

    echo "Waiting for ${crate} ${workspace_version} to appear on crates.io (${attempt}/20)..."
    sleep "${sleep_seconds}"
  done

  echo "Timed out waiting for ${crate} ${workspace_version} on crates.io." >&2
  return 1
}

for index in "${!crates[@]}"; do
  crate="${crates[$index]}"
  publish_one "${crate}"
  confirm_published "${crate}"

  if [[ "${dry_run}" != "1" && "${index}" -lt "$((${#crates[@]} - 1))" ]]; then
    echo "Waiting ${sleep_seconds} seconds before publishing the next crate."
    sleep "${sleep_seconds}"
  fi
done

if [[ "${dry_run}" == "1" ]]; then
  echo "Dry-run publish check completed for ${workspace_version}."
else
  echo "Published all Traverse crates at ${workspace_version}."
fi
