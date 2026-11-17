#!/usr/bin/env bash
# Block gate-bypass and protected-file edits (#1877).
# Usage: scripts/agent/guard_command.sh <git-commit|git-push|edit> [args...]
set -euo pipefail

mode="${1:-}"
shift || true

deny() {
  echo "Traverse guard: $1" >&2
  echo "Exception process: traverse-framework/.github docs/exception-process.md (human-approved only)." >&2
  exit 1
}

case "$mode" in
  git-commit|git-push)
    for arg in "$@"; do
      case "$arg" in
        --no-verify|-n) deny "refusing $mode with $arg — run the hooks; do not bypass them" ;;
      esac
    done
    # Detect redirected hooksPath in the effective git config for this command.
    hooks_path="$(git config --get core.hooksPath || true)"
    if [[ -n "$hooks_path" ]]; then
      deny "refusing $mode while core.hooksPath=$hooks_path is set — unset it and use the repo hooks"
    fi
    ;;
  edit)
    for path in "$@"; do
      case "$path" in
        specs/governance/approved-specs.json|.governance-version)
          deny "refusing edit to $path — agents must not self-approve specs or repin governance"
          ;;
        scripts/ci/coverage_gate.sh)
          # Allow the edit path through only if the diff does not lower a floor.
          # Callers that cannot supply a diff still get a hard block on this file.
          deny "refusing direct agent edit to $path — coverage floors are merge-gating; change via human-reviewed PR only"
          ;;
      esac
    done
    ;;
  *)
    echo "usage: guard_command.sh <git-commit|git-push|edit> [args...]" >&2
    exit 2
    ;;
esac
