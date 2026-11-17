#!/usr/bin/env bash
# Cross-tool review-record check (Decision 133 / .github#56).
#
# Warning mode in governance 1.3.x: a missing or failing record prints a
# warning annotation and exits 0. A later major release can flip
# REVIEW_RECORD_WARN_ONLY=0 to fail the job.
#
# Inputs (env):
#   REVIEW_RECORD_COMMENTS_JSON  path to a JSON array of PR comments
#   REVIEW_RECORD_HEAD_SHA       full head commit SHA under review
#   REVIEW_RECORD_BRANCH         PR head branch name (authoring-tool prefix)
#   REVIEW_RECORD_AUTHOR_LOGIN   PR author login (Dependabot exemption)
#   REVIEW_RECORD_CHANGED_FILES  newline-separated changed paths
#   REVIEW_RECORD_LABELS_JSON    JSON array of label names on the PR
#   REVIEW_RECORD_LABEL_EVENTS_JSON  optional timeline events for needs-review
#   REVIEW_RECORD_NOW            optional ISO-8601 now (tests)
#   REVIEW_RECORD_WARN_ONLY      default 1; set 0 to fail closed
set -euo pipefail

WARN_ONLY="${REVIEW_RECORD_WARN_ONLY:-1}"
COMMENTS_JSON="${REVIEW_RECORD_COMMENTS_JSON:?REVIEW_RECORD_COMMENTS_JSON is required}"
HEAD_SHA="${REVIEW_RECORD_HEAD_SHA:?REVIEW_RECORD_HEAD_SHA is required}"
BRANCH="${REVIEW_RECORD_BRANCH:?REVIEW_RECORD_BRANCH is required}"
AUTHOR_LOGIN="${REVIEW_RECORD_AUTHOR_LOGIN:-}"
CHANGED_FILES="${REVIEW_RECORD_CHANGED_FILES:-}"
LABELS_JSON="${REVIEW_RECORD_LABELS_JSON:-[]}"
LABEL_EVENTS_JSON="${REVIEW_RECORD_LABEL_EVENTS_JSON:-[]}"
NOW="${REVIEW_RECORD_NOW:-}"

tool_from_branch() {
  case "$1" in
    claude/*) echo claude ;;
    codex/*) echo codex ;;
    cursor/*) echo cursor ;;
    *) echo unknown ;;
  esac
}

authoring_tool="$(tool_from_branch "$BRANCH")"

is_exempt() {
  case "$AUTHOR_LOGIN" in
    'dependabot[bot]'|dependabot) return 0 ;;
  esac
  if [[ -z "${CHANGED_FILES//[[:space:]]/}" ]]; then
    return 1
  fi
  local path
  while IFS= read -r path; do
    [[ -z "$path" ]] && continue
    case "$path" in
      Cargo.toml|Cargo.lock|*/Cargo.toml|*/Cargo.lock) ;;
      package.json|package-lock.json|*/package.json|*/package-lock.json) ;;
      .governance-version|CHANGELOG.md|docs/releases/*|*/Package.swift|Package.swift) ;;
      *.csproj|Directory.Build.props|*/Directory.Build.props) ;;
      *) return 1 ;;
    esac
  done <<<"$CHANGED_FILES"
  return 0
}

warn() {
  echo "::warning::$1"
  echo "review-record: WARN: $1" >&2
}

fail_or_warn() {
  local msg="$1"
  if [[ "$WARN_ONLY" == "0" ]]; then
    echo "::error::$msg"
    echo "review-record: FAIL: $msg" >&2
    exit 1
  fi
  warn "$msg"
  exit 0
}

# Docs-only uses the same classifier as full CI (#1867 / Decision 133).
if [[ -x scripts/ci/pr_change_classification.sh ]] && [[ -n "${CHANGED_FILES//[[:space:]]/}" ]]; then
  if printf '%s\n' "$CHANGED_FILES" | bash scripts/ci/pr_change_classification.sh --paths-stdin \
      | grep -q '^full_ci=false$'; then
    echo "review-record: exempt (documentation-only per pr_change_classification)"
    exit 0
  fi
fi

if is_exempt; then
  echo "review-record: exempt (Dependabot or version-only release paths)"
  exit 0
fi

if [[ ! -f "$COMMENTS_JSON" ]]; then
  fail_or_warn "comments JSON file not found: $COMMENTS_JSON"
fi

eval "$(
  python3 - "$COMMENTS_JSON" "$HEAD_SHA" "$LABELS_JSON" "$LABEL_EVENTS_JSON" "$NOW" <<'PY'
import json, re, sys
from datetime import datetime, timezone

comments_path, head_sha, labels_json, events_json, now_raw = sys.argv[1:6]
comments = json.load(open(comments_path))
labels = json.loads(labels_json) if labels_json else []
events = json.loads(events_json) if events_json else []

header_re = re.compile(
    r"<!--\s*traverse-review-record\s*\n"
    r"tool:\s*(?P<tool>\S+)\s*\n"
    r"commit:\s*(?P<commit>[0-9a-fA-F]{40})\s*\n"
    r"verdict:\s*(?P<verdict>pass|changes-requested)\s*\n"
    r"fallback:\s*(?P<fallback>true|false)\s*\n"
    r"-->",
    re.MULTILINE,
)

matches = []
for comment in comments:
    body = comment.get("body") or ""
    m = header_re.search(body)
    if not m:
        continue
    matches.append({
        "tool": m.group("tool").lower(),
        "commit": m.group("commit").lower(),
        "verdict": m.group("verdict"),
        "fallback": m.group("fallback") == "true",
        "created_at": comment.get("created_at") or "",
    })

for_sha = [m for m in matches if m["commit"] == head_sha.lower()]
for_sha.sort(key=lambda m: m["created_at"])
latest = for_sha[-1] if for_sha else None

needs_review = "needs-review" in labels
needs_since = None
for event in events:
    if event.get("event") != "labeled":
        continue
    label = (event.get("label") or {}).get("name") or event.get("label_name")
    if label != "needs-review":
        continue
    ts = event.get("created_at")
    if ts and (needs_since is None or ts < needs_since):
        needs_since = ts

def parse_ts(value: str):
    if not value:
        return None
    return datetime.fromisoformat(value.replace("Z", "+00:00"))

now = parse_ts(now_raw) if now_raw else datetime.now(timezone.utc)
age_hours = None
if needs_since and now is not None:
    started = parse_ts(needs_since)
    if started is not None:
        age_hours = (now - started).total_seconds() / 3600.0

def shell_quote(s: str) -> str:
    return "'" + s.replace("'", "'\"'\"'") + "'"

print(f"HAS_RECORD={'1' if latest else '0'}")
if latest:
    print(f"RECORD_TOOL={shell_quote(latest['tool'])}")
    print(f"RECORD_VERDICT={shell_quote(latest['verdict'])}")
    print(f"RECORD_FALLBACK={'1' if latest['fallback'] else '0'}")
else:
    print("RECORD_TOOL=''")
    print("RECORD_VERDICT=''")
    print("RECORD_FALLBACK=0")
print(f"NEEDS_REVIEW={'1' if needs_review else '0'}")
print(f"NEEDS_AGE_HOURS={age_hours if age_hours is not None else ''}")
PY
)"

if [[ "${HAS_RECORD}" != "1" ]]; then
  fail_or_warn "no traverse-review-record comment for head ${HEAD_SHA}; add needs-review and wait for a different tool, or post a record (warning mode)"
fi

if [[ "${RECORD_VERDICT}" == "changes-requested" ]]; then
  fail_or_warn "latest review record for ${HEAD_SHA} is changes-requested (tool=${RECORD_TOOL})"
fi

if [[ "${RECORD_VERDICT}" != "pass" ]]; then
  fail_or_warn "latest review record for ${HEAD_SHA} has unsupported verdict '${RECORD_VERDICT}'"
fi

if [[ "${RECORD_TOOL}" != "${authoring_tool}" && "${RECORD_TOOL}" != "unknown" && "${authoring_tool}" != "unknown" ]]; then
  echo "review-record: cross-tool pass (${RECORD_TOOL} reviewed ${authoring_tool} branch)"
  exit 0
fi

if [[ "${RECORD_FALLBACK}" != "1" ]]; then
  fail_or_warn "same-tool pass from ${RECORD_TOOL} is not marked fallback=true; wait for a different tool or mark the fallback after 4h on needs-review"
fi

if [[ "${NEEDS_REVIEW}" != "1" ]]; then
  fail_or_warn "same-tool fallback requires the needs-review label"
fi

if [[ -z "${NEEDS_AGE_HOURS}" ]]; then
  fail_or_warn "same-tool fallback requires a needs-review label timestamp (none found)"
fi

awk -v age="${NEEDS_AGE_HOURS}" 'BEGIN { exit (age + 0 >= 4.0) ? 0 : 1 }' || \
  fail_or_warn "same-tool fallback too early: needs-review age ${NEEDS_AGE_HOURS}h < 4h"

echo "review-record: same-tool fallback pass after ${NEEDS_AGE_HOURS}h (tool=${RECORD_TOOL})"
exit 0
