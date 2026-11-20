#!/usr/bin/env bash
# Claim a Ready issue for one agent (.github#57).
#
#   scripts/agent/claim.sh <issue-number> <claude|codex|cursor> [--repo OWNER/REPO] [--config PATH] [--dry-run]
#
# Checks, in order, and changes nothing if any fails:
#   1. the issue is open;
#   2. its item on the configured Project has Status "Ready";
#   3. it carries no agent:* label;
#   4. no remote branch from any tool matches */issue-<N>-*;
#   5. no open pull request has a head branch matching */issue-<N>-*.
# Then it adds agent:<tool>, sets the Project item to "In Progress", and posts
# an audit comment naming the tool, the time and the suggested branch.
#
# Project and field ids come from the per-repo config file (default
# .agent-claim.json at the repository root, or $AGENT_CLAIM_CONFIG); see
# docs/ai-agent-hardening.md. --dry-run runs every check and prints what it
# would do.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=scripts/agent/claim_lib.sh
source "${here}/claim_lib.sh"

usage="usage: claim.sh <issue> <claude|codex|cursor> [--repo OWNER/REPO] [--config PATH] [--dry-run]"
[[ $# -ge 2 ]] || claim_die 2 "${usage}"
ISSUE="$1"
AGENT="$2"
shift 2
REPO=""
CONFIG="${AGENT_CLAIM_CONFIG:-}"
DRY_RUN=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --repo) REPO="${2:?--repo needs OWNER/REPO}"; shift 2 ;;
    --config) CONFIG="${2:?--config needs a path}"; shift 2 ;;
    --dry-run) DRY_RUN=1; shift ;;
    *) claim_die 2 "unknown argument: $1 (${usage})" ;;
  esac
done

[[ "${ISSUE}" =~ ^[0-9]+$ ]] || claim_die 2 "issue must be a number: ${ISSUE}"
case "${AGENT}" in
  claude|codex|cursor) ;;
  *) claim_die 2 "agent must be claude, codex or cursor" ;;
esac

claim_load_config "${CONFIG}" "${REPO}"

state="$(gh issue view "${ISSUE}" --repo "${CLAIM_REPO}" --json state --jq .state)"
[[ "${state}" == "OPEN" ]] || claim_die 1 "${CLAIM_REPO}#${ISSUE} is ${state}"

claim_project_item "${ISSUE}"
[[ -n "${CLAIM_ITEM_ID}" ]] || claim_die 1 "${CLAIM_REPO}#${ISSUE} has no item on the configured Project"
[[ "${CLAIM_ITEM_STATUS}" == "Ready" ]] ||
  claim_die 1 "${CLAIM_REPO}#${ISSUE} is '${CLAIM_ITEM_STATUS:-no status}' on the Project, not Ready"

existing="$(gh issue view "${ISSUE}" --repo "${CLAIM_REPO}" --json labels \
  --jq '[.labels[].name | select(startswith("agent:"))] | join(" ")')"
[[ -z "${existing}" ]] || claim_die 1 "${CLAIM_REPO}#${ISSUE} is already claimed: ${existing}"

branches="$(claim_issue_branches "${ISSUE}")"
[[ -z "${branches}" ]] || claim_die 1 "a branch for issue ${ISSUE} already exists: ${branches}"

prs="$(claim_issue_prs "${ISSUE}")"
[[ -z "${prs}" ]] || claim_die 1 "an open pull request for issue ${ISSUE} already exists: ${prs}"

branch="${AGENT}/issue-${ISSUE}-work"
if [[ "${DRY_RUN}" -eq 1 ]]; then
  echo "dry run: ${CLAIM_REPO}#${ISSUE} can be claimed by agent:${AGENT}"
  echo "would add label agent:${AGENT}, set Project item ${CLAIM_ITEM_ID} to In Progress, and comment (branch ${branch})"
  exit 0
fi

gh issue edit "${ISSUE}" --repo "${CLAIM_REPO}" --add-label "agent:${AGENT}" >/dev/null
claim_set_status "${CLAIM_ITEM_ID}" "In Progress"
gh issue comment "${ISSUE}" --repo "${CLAIM_REPO}" \
  --body "Claimed by \`agent:${AGENT}\` at $(date -u +%Y-%m-%dT%H:%MZ). Branch: \`${branch}\`." >/dev/null
echo "claimed ${CLAIM_REPO}#${ISSUE} as agent:${AGENT} (branch ${branch})"
