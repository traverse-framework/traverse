#!/usr/bin/env bash
# Fixture tests for scripts/ci/review_record_check.sh (.github#56).
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
check="$root/scripts/ci/review_record_check.sh"
tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT

SHA=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
OTHER=bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb

record() {
  local tool="$1" commit="$2" verdict="$3" fallback="$4"
  cat <<REC
<!-- traverse-review-record
tool: ${tool}
commit: ${commit}
verdict: ${verdict}
fallback: ${fallback}
-->
review body
REC
}

run_check() {
  local comments_file="$1"
  shift
  REVIEW_RECORD_COMMENTS_JSON="$comments_file" \
  REVIEW_RECORD_HEAD_SHA="$SHA" \
  REVIEW_RECORD_BRANCH="${BRANCH:-claude/issue-1-x}" \
  REVIEW_RECORD_AUTHOR_LOGIN="${AUTHOR:-enrico}" \
  REVIEW_RECORD_CHANGED_FILES="${FILES:-crates/traverse-runtime/src/lib.rs}" \
  REVIEW_RECORD_LABELS_JSON="${LABELS:-[\"needs-review\"]}" \
  REVIEW_RECORD_LABEL_EVENTS_JSON="${EVENTS:-[]}" \
  REVIEW_RECORD_NOW="${NOW:-2026-11-16T12:00:00Z}" \
  REVIEW_RECORD_WARN_ONLY="${WARN_ONLY:-0}" \
    bash "$check" "$@"
}

assert_exit() {
  local want="$1"
  shift
  set +e
  out="$("$@" 2>&1)"
  code=$?
  set -e
  if [[ "$code" -ne "$want" ]]; then
    echo "expected exit $want, got $code" >&2
    echo "$out" >&2
    exit 1
  fi
  printf '%s\n' "$out"
}

# 1. Dependabot exempt
AUTHOR='dependabot[bot]' FILES=$'Cargo.lock\nCargo.toml' \
  assert_exit 0 run_check <(echo '[]') | grep -q exempt

# 2. Version-only release paths exempt
AUTHOR=enrico FILES=$'CHANGELOG.md\n.governance-version' \
  assert_exit 0 run_check <(echo '[]') | grep -q exempt

# 3. Cross-tool pass
AUTHOR=enrico FILES='crates/x.rs' \
  python3 - "$tmpdir/cross.json" <<PY
import json, pathlib, os, textwrap
body = """<!-- traverse-review-record
tool: codex
commit: ${SHA}
verdict: pass
fallback: false
-->
ok"""
pathlib.Path(os.environ.get("OUT","$tmpdir/cross.json".replace("$tmpdir","$tmpdir"))).write_text("")
PY
python3 - <<PY
import json
from pathlib import Path
body = """<!-- traverse-review-record
tool: codex
commit: $SHA
verdict: pass
fallback: false
-->
ok"""
Path("$tmpdir/cross.json").write_text(json.dumps([{"body": body, "created_at": "2026-11-16T10:00:00Z"}]))
PY
AUTHOR=enrico FILES='crates/x.rs' LABELS='[]' \
  assert_exit 0 run_check "$tmpdir/cross.json" | grep -q 'cross-tool pass'

# 4. Same-tool pass before 4h warns/fails closed under WARN_ONLY=0
python3 - <<PY
import json
from pathlib import Path
body = """<!-- traverse-review-record
tool: claude
commit: $SHA
verdict: pass
fallback: true
-->
ok"""
Path("$tmpdir/early.json").write_text(json.dumps([{"body": body, "created_at": "2026-11-16T11:00:00Z"}]))
PY
EVENTS='[{"event":"labeled","label_name":"needs-review","created_at":"2026-11-16T10:00:00Z"}]' \
NOW='2026-11-16T12:00:00Z' WARN_ONLY=0 \
  assert_exit 1 run_check "$tmpdir/early.json" | grep -q 'too early'

# 5. Same-tool fallback after 4h passes
EVENTS='[{"event":"labeled","label_name":"needs-review","created_at":"2026-11-16T06:00:00Z"}]' \
NOW='2026-11-16T12:00:00Z' WARN_ONLY=0 \
  assert_exit 0 run_check "$tmpdir/early.json" | grep -q 'fallback pass'

# 6. Record for older SHA warns under WARN_ONLY=0
python3 - <<PY
import json
from pathlib import Path
body = """<!-- traverse-review-record
tool: codex
commit: $OTHER
verdict: pass
fallback: false
-->
ok"""
Path("$tmpdir/old.json").write_text(json.dumps([{"body": body, "created_at": "2026-11-16T10:00:00Z"}]))
PY
WARN_ONLY=0 assert_exit 1 run_check "$tmpdir/old.json" | grep -q 'no traverse-review-record'

# 7. changes-requested as latest record fails/warns
python3 - <<PY
import json
from pathlib import Path
comments = [
  {"body": """<!-- traverse-review-record
tool: codex
commit: $SHA
verdict: pass
fallback: false
-->
ok""", "created_at": "2026-11-16T09:00:00Z"},
  {"body": """<!-- traverse-review-record
tool: codex
commit: $SHA
verdict: changes-requested
fallback: false
-->
fix""", "created_at": "2026-11-16T11:00:00Z"},
]
Path("$tmpdir/cr.json").write_text(json.dumps(comments))
PY
WARN_ONLY=0 assert_exit 1 run_check "$tmpdir/cr.json" | grep -q 'changes-requested'

# 8. Warning mode exits 0 on missing record
WARN_ONLY=1 assert_exit 0 run_check <(echo '[]') | grep -q 'WARN'

echo "review_record_check_test: all cases passed"

# Docs-only paths: same signal as full_ci=false (#1867).
export BRANCH=claude/issue-1-x AUTHOR=enrico
export FILES=$'docs/agent-pitfalls.md\nREADME.md'
export WARN_ONLY=0
comments="$tmpdir/empty.json"
echo '[]' >"$comments"
if ! out="$(run_check "$comments" 2>&1)"; then
  echo "docs-only should exempt, got failure: $out" >&2
  exit 1
fi
echo "$out" | grep -q 'documentation-only'
echo "review_record_check_test: docs-only exempt ok"

