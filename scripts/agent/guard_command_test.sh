#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
g="$root/scripts/agent/guard_command.sh"
cg="$root/scripts/agent/cursor_guard.py"

expect_block() {
  local code="$1"; shift
  set +e
  "$@" >/tmp/guard-out.txt 2>&1
  local got=$?
  set -e
  [[ "$got" -eq "$code" ]] || { echo "expected exit $code from: $* (got $got)"; cat /tmp/guard-out.txt; exit 1; }
}

expect_block 1 "$g" git-push --no-verify origin HEAD
grep -q 'refusing' /tmp/guard-out.txt
expect_block 1 "$g" git-commit -n -m x
expect_block 1 "$g" edit specs/governance/approved-specs.json
expect_block 1 "$g" edit .governance-version
expect_block 1 "$g" edit scripts/ci/coverage_gate.sh
expect_block 0 "$g" edit README.md

expect_block 2 bash -c "printf '%s' '{\"tool_input\":{\"command\":\"git push --no-verify origin HEAD\"}}' | python3 \"$cg\""
expect_block 2 bash -c "printf '%s' '{\"tool_input\":{\"path\":\"specs/governance/approved-specs.json\"}}' | python3 \"$cg\""
expect_block 0 bash -c "printf '%s' '{\"tool_input\":{\"command\":\"git status\"}}' | python3 \"$cg\""

echo "guard_command_test: ok"
