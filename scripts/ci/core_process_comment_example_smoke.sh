#!/usr/bin/env bash
# End-to-end smoke for examples/core-process-comment (core.process-comment@1.0.0).

set -euo pipefail

repo_root="${TRAVERSE_REPO_ROOT:-$(pwd)}"
cd "$repo_root"

pkg="examples/core-process-comment"
cli=(cargo run -q -p traverse-cli-rs --)

fail() {
  printf 'FAIL: %s\n' "$1" >&2
  exit 1
}

require_match() {
  local haystack="$1"
  local needle="$2"
  local label="$3"
  grep -q "$needle" <<<"$haystack" || fail "$label (missing: $needle)"
}

echo "==> build-fixture"
bash "$pkg/build-fixture.sh"

echo "==> capability inspect"
contract_out="$("${cli[@]}" capability inspect "$pkg/contract.json")"
printf '%s\n' "$contract_out"
require_match "$contract_out" "id: core.process-comment" "contract inspect id"
require_match "$contract_out" "version: 1.0.1" "contract inspect version"

echo "==> wasm abi verify"
abi_out="$("${cli[@]}" wasm abi verify "$pkg/artifacts/core-process-comment.wasm")"
printf '%s\n' "$abi_out"
require_match "$abi_out" "import whitelist passed" "abi verify"

echo "==> capability-package inspect"
pkg_out="$("${cli[@]}" capability-package inspect "$pkg/manifest.json")"
printf '%s\n' "$pkg_out"
require_match "$pkg_out" "package_id: core.process-comment-agent" "package_id"
require_match "$pkg_out" "capability_version: 1.0.1" "capability_version"

assert_execute() {
  local request="$1"
  local decision="$2"
  local code="$3"
  local label="$4"
  local extra="${5:-}"

  echo "==> execute $label"
  local out
  out="$("${cli[@]}" capability-package execute "$pkg/manifest.json" "$request")"
  printf '%s\n' "$out"
  require_match "$out" "status: completed" "$label status"
  require_match "$out" "capability_version: 1.0.1" "$label capability_version"
  require_match "$out" "\"decision\": \"$decision\"" "$label decision"
  require_match "$out" "\"reason_code\": \"$code\"" "$label reason_code"
  if [[ -n "$extra" ]]; then
    require_match "$out" "$extra" "$label extra"
  fi
}

assert_execute "$pkg/runtime-requests/uc01-create-mentions-allow.json" "allow" "ok" "UC-01" '"type": "notify"'
assert_execute "$pkg/runtime-requests/uc02-edit-not-owner-deny.json" "deny" "not_owner" "UC-02"
assert_execute "$pkg/runtime-requests/uc03-reply-depth-deny.json" "deny" "max_thread_depth_exceeded" "UC-03"
assert_execute "$pkg/runtime-requests/uc04-moderation-quarantine-allow.json" "allow" "moderation_quarantine" "UC-04" '"type": "quarantine"'
assert_execute "$pkg/runtime-requests/uc05-react-allow.json" "allow" "ok" "UC-05" '"emoji": "thumbsup"'
assert_execute "$pkg/runtime-requests/uc06-soft-delete-allow.json" "allow" "ok" "UC-06" '"deleted": true'
assert_execute "$pkg/runtime-requests/uc07-tenant-isolation-deny.json" "deny" "tenant_isolation_violation" "UC-07"
assert_execute "$pkg/runtime-requests/uc08-empty-body-deny.json" "deny" "empty_body" "UC-08"

echo "OK: core.process-comment 1.0.1 E2E smoke passed"
