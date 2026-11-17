#!/usr/bin/env bash
# Install the Traverse pre-push hook (#1877).
#
# First push (no open PR yet): set TRAVERSE_PR_BODY to a PR-body markdown file
# (or place one at .git/traverse-pr-body.md). The hook runs local preflight
# against the merge-base with origin/main — no --no-verify.
set -euo pipefail

hook_path="$(git rev-parse --git-path hooks/pre-push)"

cat >"$hook_path" <<'HOOK'
#!/usr/bin/env bash
set -euo pipefail

# Drain git-provided ref lines so child processes cannot steal stdin.
while IFS= read -r _; do :; done

if ! command -v gh >/dev/null 2>&1; then
  echo "Traverse pre-push: gh is unavailable; run scripts/ci/local_preflight.sh manually." >&2
  exit 1
fi

git_dir="$(git rev-parse --git-dir)"
pr_body_file=""
cleanup_body=0

if pr_number="$(gh pr view --json number --jq .number 2>/dev/null)"; then
  base_branch="$(gh pr view "$pr_number" --json baseRefName --jq .baseRefName)"
  git fetch -q origin "$base_branch"
  base_sha="$(git merge-base HEAD "origin/${base_branch}")"
  pr_body_file="$(mktemp -t traverse-pr-body.XXXXXX.md)"
  cleanup_body=1
  gh pr view "$pr_number" --json body --jq .body >"$pr_body_file"
  echo "Traverse pre-push: PR #${pr_number} BASE_SHA=${base_sha}" >&2
else
  # First push: accept TRAVERSE_PR_BODY or .git/traverse-pr-body.md
  if [[ -n "${TRAVERSE_PR_BODY:-}" && -f "${TRAVERSE_PR_BODY}" ]]; then
    pr_body_file="${TRAVERSE_PR_BODY}"
  elif [[ -f "${git_dir}/traverse-pr-body.md" ]]; then
    pr_body_file="${git_dir}/traverse-pr-body.md"
  else
    echo "Traverse pre-push: no PR and no PR-body file." >&2
    echo "Set TRAVERSE_PR_BODY=/path/to/pr-body.md (or write .git/traverse-pr-body.md)," >&2
    echo "then push again. Do not use --no-verify." >&2
    exit 1
  fi
  git fetch -q origin main
  base_sha="$(git merge-base HEAD origin/main)"
  echo "Traverse pre-push: first push BASE_SHA=${base_sha} body=${pr_body_file}" >&2
fi

preflight_log="$(mktemp -t traverse-pre-push.XXXXXX.log)"
echo "Traverse pre-push: log ${preflight_log}" >&2

# Docs-only changes: PR-body + spec-alignment only (#1877).
classify_out="$(GITHUB_BASE_SHA="$base_sha" GITHUB_HEAD_SHA="$(git rev-parse HEAD)" \
  bash scripts/ci/pr_change_classification.sh "$base_sha" "$(git rev-parse HEAD)" || true)"
full_ci="$(printf '%s\n' "$classify_out" | awk -F= '/^full_ci=/{print $2; exit}')"

if [[ "${full_ci}" != "true" ]]; then
  echo "Traverse pre-push: documentation-only — running PR-body + spec-alignment only" >&2
  if ! {
    bash scripts/ci/pr_body_check.sh "$pr_body_file"
    GITHUB_BASE_SHA="$base_sha" GITHUB_HEAD_SHA="$(git rev-parse HEAD)" \
      bash scripts/ci/spec_alignment_check.sh "$pr_body_file"
  } >"$preflight_log" 2>&1; then
    echo "Traverse pre-push: docs-only gates failed. Last 80 lines:" >&2
    tail -n 80 "$preflight_log" >&2 || true
    [[ "$cleanup_body" -eq 1 ]] && rm -f "$pr_body_file"
    exit 1
  fi
else
  if ! BASE_SHA="$base_sha" bash scripts/ci/local_preflight.sh --pr-body "$pr_body_file" \
      >"$preflight_log" 2>&1; then
    echo "Traverse pre-push: local_preflight failed. Last 80 lines:" >&2
    tail -n 80 "$preflight_log" >&2 || true
    [[ "$cleanup_body" -eq 1 ]] && rm -f "$pr_body_file"
    exit 1
  fi
fi

[[ "$cleanup_body" -eq 1 ]] && rm -f "$pr_body_file"
echo "Traverse pre-push: passed." >&2
HOOK

chmod +x "$hook_path"
echo "Installed Traverse CI pre-push hook at $hook_path"
