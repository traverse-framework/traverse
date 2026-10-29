#!/usr/bin/env bash

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

failures=()

note_failure() {
  failures+=("$1")
}

check_command() {
  local command_name="$1"
  local install_hint="$2"

  if command -v "${command_name}" >/dev/null 2>&1; then
    printf '[ok] %s is installed\n' "${command_name}"
  else
    printf '[missing] %s\n' "${command_name}"
    note_failure "${command_name}: ${install_hint}"
  fi
}

version_at_least() {
  local actual="$1"
  local required="$2"

  local lower
  lower="$(printf '%s\n%s\n' "${actual}" "${required}" | sort -V | head -n1)"
  [[ "${lower}" == "${required}" ]]
}

check_rust_toolchain() {
  local rustc_version
  local cargo_version
  local rustc_semver
  local cargo_semver

  if ! command -v rustc >/dev/null 2>&1; then
    printf '[missing] rustc\n'
    note_failure "rustc: install Rust 1.95 or later from https://rustup.rs"
    return
  fi

  if ! command -v cargo >/dev/null 2>&1; then
    printf '[missing] cargo\n'
    note_failure "cargo: install Rust 1.95 or later from https://rustup.rs"
    return
  fi

  rustc_version="$(rustc --version)"
  cargo_version="$(cargo --version)"
  rustc_semver="$(printf '%s\n' "${rustc_version}" | awk '{print $2}')"
  cargo_semver="$(printf '%s\n' "${cargo_version}" | awk '{print $2}')"

  if version_at_least "${rustc_semver}" "1.95.0"; then
    printf '[ok] rustc %s\n' "${rustc_semver}"
  else
    printf '[unsupported] rustc %s\n' "${rustc_semver}"
    note_failure "rustc: expected 1.95.0 or later, found ${rustc_semver}"
  fi

  if version_at_least "${cargo_semver}" "1.95.0"; then
    printf '[ok] cargo %s\n' "${cargo_semver}"
  else
    printf '[unsupported] cargo %s\n' "${cargo_semver}"
    note_failure "cargo: expected 1.95.0 or later, found ${cargo_semver}"
  fi
}

check_node() {
  local node_version
  local node_semver

  if ! command -v node >/dev/null 2>&1; then
    printf '[missing] node\n'
    note_failure "node: install Node.js 20 or later for the browser quickstart"
    return
  fi

  node_version="$(node --version)"
  node_semver="${node_version#v}"

  if version_at_least "${node_semver}" "20.0.0"; then
    printf '[ok] node %s\n' "${node_semver}"
  else
    printf '[unsupported] node %s\n' "${node_semver}"
    note_failure "node: expected 20.0.0 or later, found ${node_semver}"
  fi
}

check_repo_shape() {
  if [[ -f "${repo_root}/README.md" && -f "${repo_root}/Cargo.toml" ]]; then
    printf '[ok] repository root detected at %s\n' "${repo_root}"
  else
    printf '[missing] repository root markers\n'
    note_failure "repo: run this script from a Traverse checkout with README.md and Cargo.toml present"
  fi
}

main() {
  printf 'Traverse setup validation\n'
  printf 'Repository root: %s\n\n' "${repo_root}"

  check_repo_shape
  check_rust_toolchain
  check_command bash "use a shell that can run the checked-in validation scripts"
  check_node

  printf '\n'

  if ((${#failures[@]} > 0)); then
    printf 'Setup is not ready yet.\n'
    printf 'Fix these items before starting the documented developer paths:\n'
    for failure in "${failures[@]}"; do
      printf -- '- %s\n' "${failure}"
    done
    exit 1
  fi

  printf 'Setup looks ready for the documented Traverse developer paths.\n'
  printf 'Next steps:\n'
  printf -- '- bash scripts/ci/repository_checks.sh\n'
  printf -- '- cargo build\n'
  printf -- '- Follow docs/getting-started.md or quickstart.md\n'
}

main "$@"
