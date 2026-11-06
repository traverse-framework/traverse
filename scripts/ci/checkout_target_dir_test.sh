#!/usr/bin/env bash
# Fixture tests for scripts/ci/checkout_target_dir.sh (#1801).

set -euo pipefail

# Reachable from a git hook, like bump_version_test.sh: keep git scoped to the fixture.
unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_COMMON_DIR

repo_root="$(cd "$(dirname "$0")/../.." && pwd)"

fixture_root="$(cd "$(mktemp -d "${TMPDIR:-/tmp}/traverse-checkout-target-dir-test.XXXXXX")" && pwd -P)"
trap 'rm -rf "${fixture_root}"' EXIT

for checkout in a b; do
  mkdir -p "${fixture_root}/${checkout}/scripts/ci"
  cp "${repo_root}/scripts/ci/checkout_target_dir.sh" "${fixture_root}/${checkout}/scripts/ci/"
done

# Prints CARGO_TARGET_DIR after sourcing the helper `times` times in `checkout`.
resolve() {
  local checkout=$1 value=$2 times=${3:-1}
  (
    if [[ -n "${value}" ]]; then export CARGO_TARGET_DIR="${value}"; else unset CARGO_TARGET_DIR; fi
    for ((i = 0; i < times; i++)); do
      source "${fixture_root}/${checkout}/scripts/ci/checkout_target_dir.sh"
    done
    printf '%s' "${CARGO_TARGET_DIR:-}"
  )
}

fail() {
  echo "checkout_target_dir_test: $1" >&2
  exit 1
}

shared="${fixture_root}/shared"
from_a="$(resolve a "${shared}")"
from_b="$(resolve b "${shared}")"

[[ "${from_a}" == "${fixture_root}/a/target" ]] || fail "not the checkout's own target/: ${from_a}"
[[ "${from_b}" == "${fixture_root}/b/target" ]] || fail "not the checkout's own target/: ${from_b}"
[[ "$(resolve a "${fixture_root}/b/target")" == "${from_a}" ]] || fail "uses another checkout's target/"
[[ "$(resolve a "${shared}")" == "${from_a}" ]] || fail "not stable across runs"
[[ "$(resolve a "${shared}" 2)" == "${from_a}" ]] || fail "sourcing twice nests the directory"
[[ "$(resolve a "${shared}/")" == "${from_a}" ]] || fail "a trailing slash changes the directory"
[[ -z "$(resolve a "")" ]] || fail "an unset value must stay unset"
[[ "$(resolve a "${fixture_root}/a/target")" == "${fixture_root}/a/target" ]] || fail "a directory inside the checkout must not change"
[[ "$(resolve a "relative/target")" == "relative/target" ]] || fail "a relative directory must not change"

echo "checkout_target_dir_test: all cases passed"
