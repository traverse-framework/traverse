#!/usr/bin/env bash
# Fixture tests for scripts/ci/publish_runtime_wasm_release.sh (spec 048 FR-020).
# Runs against a throwaway git repo and a fake `gh`. Nothing reaches GitHub.

set -euo pipefail

# Reachable from a git hook, like bump_version_test.sh: keep git scoped to the fixture.
unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_COMMON_DIR

repo_root="$(cd "$(dirname "$0")/../.." && pwd)"
script="${repo_root}/scripts/ci/publish_runtime_wasm_release.sh"

fixture_root="$(mktemp -d "${TMPDIR:-/tmp}/traverse-runtime-wasm-release-test.XXXXXX")"
trap 'rm -rf "${fixture_root}"' EXIT
export GIT_TEMPLATE_DIR="${fixture_root}/empty-template"
mkdir -p "${GIT_TEMPLATE_DIR}" "${fixture_root}/bin"

# Fake gh: a release is a directory holding its title, its arguments and its assets.
cat >"${fixture_root}/bin/gh" <<'GH'
#!/usr/bin/env bash
set -euo pipefail
release_dir="${FAKE_GH_STATE}/$3"
case "$2" in
  view)
    [[ -d "${release_dir}" ]] || exit 1
    cat "${release_dir}/title"
    ;;
  create)
    args="$*"
    mkdir -p "${release_dir}/assets"
    shift 3
    while [[ $# -gt 0 ]]; do
      case "$1" in
        --title) printf '%s\n' "$2" >"${release_dir}/title"; shift 2 ;;
        --repo|--target|--notes) shift 2 ;;
        --latest=false) shift ;;
        *) cp "$1" "${release_dir}/assets/"; shift ;;
      esac
    done
    printf '%s\n' "${args}" >"${release_dir}/args"
    ;;
  download)
    cp "${release_dir}/assets/"* "$7/"
    ;;
esac
GH
chmod +x "${fixture_root}/bin/gh"
export PATH="${fixture_root}/bin:${PATH}"

sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

# Seeds a repo tagged v1.2.3. Args: runtime_version, recorded digest, .sha256 digest
# ("actual" means the digest of the committed bytes).
seed() {
  local root="$1" runtime_version="$2" recorded="$3" pinned="$4"
  mkdir -p "${root}/scripts/ci" "${root}/runtime"
  cp "${script}" "${root}/scripts/ci/"
  printf 'wasm-bytes' >"${root}/runtime/runtime.wasm"
  local digest
  digest="$(sha256_of "${root}/runtime/runtime.wasm")"
  [[ "${recorded}" == "actual" ]] && recorded="${digest}"
  [[ "${pinned}" == "actual" ]] && pinned="${digest}"
  printf 'sha256:%s\n' "${pinned}" >"${root}/runtime/runtime.wasm.sha256"
  printf '{"runtime_version":"%s","bridge_version":"1.1.0","bridge_abi_version":10100,"sha256":"%s"}\n' \
    "${runtime_version}" "${recorded}" >"${root}/runtime/runtime-release.json"
  git -C "${root}" init -q
  git -C "${root}" add -A
  git -C "${root}" -c user.name=t -c user.email=t@example.invalid commit -q -m seed
  git -C "${root}" tag v1.2.3
  export FAKE_GH_STATE="${root}.gh"
  mkdir -p "${FAKE_GH_STATE}"
}

run() {
  bash "$1/scripts/ci/publish_runtime_wasm_release.sh" "${@:2}" 2>&1
}

expect_failure() {
  local name="$1" expected="$2" output
  shift 2
  if output="$(run "$@")"; then
    echo "${name}: expected a failure" >&2
    exit 1
  fi
  if [[ "${output}" != *"${expected}"* ]]; then
    echo "${name}: expected '${expected}', got: ${output}" >&2
    exit 1
  fi
}

# Publishes the tag's three assets, never Latest, at the tag's commit.
good="${fixture_root}/good"
seed "${good}" 1.2.3 actual actual
run "${good}" v1.2.3 >/dev/null
release="${FAKE_GH_STATE}/runtime-wasm-v1.2.3"
grep -qx 'runtime.wasm v1.2.3 (bridge 1.1.0)' "${release}/title"
grep -q -- '--latest=false' "${release}/args"
grep -q -- "--target $(git -C "${good}" rev-parse v1.2.3)" "${release}/args"
for asset in runtime.wasm runtime.wasm.sha256 runtime-release.json; do
  cmp -s "${good}/runtime/${asset}" "${release}/assets/${asset}"
done

# A rerun creates nothing and still passes.
rm "${release}/args"
run "${good}" v1.2.3 >/dev/null
[[ ! -e "${release}/args" ]]

# It publishes the tag's bytes, whatever the checkout holds.
printf 'newer-bytes' >"${good}/runtime/runtime.wasm"
run "${good}" v1.2.3 >/dev/null

# An existing release holding other bytes, or another title, fails.
printf 'other-bytes' >"${release}/assets/runtime.wasm"
expect_failure "changed asset" "differs from v1.2.3" "${good}" v1.2.3
rm "${release}/assets/runtime.wasm.sha256"
expect_failure "missing asset" "is missing or differs" "${good}" v1.2.3
printf 'Traverse v1.2.3\n' >"${release}/title"
expect_failure "wrong title" 'exists with title "Traverse v1.2.3"' "${good}" v1.2.3

# --verify-only never touches a release.
verify="${fixture_root}/verify"
seed "${verify}" 1.2.3 actual actual
run "${verify}" v1.2.3 --verify-only >/dev/null
[[ ! -e "${FAKE_GH_STATE}/runtime-wasm-v1.2.3" ]]

# Pins that don't match the tag fail before anything is published.
stale="${fixture_root}/stale"
seed "${stale}" 1.2.2 actual actual
expect_failure "stale version" "says runtime_version 1.2.2" "${stale}" v1.2.3
recorded="${fixture_root}/recorded"
seed "${recorded}" 1.2.3 "$(printf '0%.0s' {1..64})" actual
expect_failure "recorded digest" "runtime-release.json records" "${recorded}" v1.2.3
pinned="${fixture_root}/pinned"
seed "${pinned}" 1.2.3 actual "$(printf '0%.0s' {1..64})"
expect_failure "pinned digest" "runtime.wasm.sha256 records" "${pinned}" v1.2.3
[[ ! -e "${FAKE_GH_STATE}/runtime-wasm-v1.2.3" ]]

expect_failure "unknown tag" "does not exist" "${good}" v9.9.9
expect_failure "bad tag" "Usage:" "${good}" 1.2.3
expect_failure "bad flag" "Usage:" "${good}" v1.2.3 --nope

echo "publish_runtime_wasm_release.sh fixture tests passed."
