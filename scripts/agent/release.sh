#!/usr/bin/env bash
# Release a claimed issue (.github#57).
#
#   scripts/agent/release.sh <issue-number> <Ready|Blocked|Done|In Progress> [--repo OWNER/REPO] [--config PATH] [--dry-run]
#
# Removes every agent:* label, sets the issue's item on the configured Project
# to the named status, and posts an audit comment. Project and field ids come
# from the per-repo config file (default .agent-claim.json, or
# $AGENT_CLAIM_CONFIG).
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=scripts/agent/claim_lib.sh
source "${here}/claim_lib.sh"

usage="usage: release.sh <issue> <Ready|Blocked|Done|In Progress> [--repo OWNER/REPO] [--config PATH] [--dry-run]"
[[ $# -ge 2 ]] || claim_die 2 "${usage}"
ISSUE="$1"
STATUS="$2"
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
case "${STATUS}" in
  Ready|Blocked|Done|"In Progress") ;;
  *) claim_die 2 "status must be Ready, Blocked, Done or In Progress" ;;
esac

claim_load_config "${CONFIG}" "${REPO}"
claim_status_option "${STATUS}" >/dev/null
claim_project_item "${ISSUE}"
[[ -n "${CLAIM_ITEM_ID}" ]] || claim_die 1 "${CLAIM_REPO}#${ISSUE} has no item on the configured Project"

labels="$(gh issue view "${ISSUE}" --repo "${CLAIM_REPO}" --json labels \
  --jq '[.labels[].name | select(startswith("agent:"))] | join(" ")')"

if [[ "${DRY_RUN}" -eq 1 ]]; then
  echo "dry run: would remove '${labels:-no agent labels}' from ${CLAIM_REPO}#${ISSUE}, set Project item ${CLAIM_ITEM_ID} to ${STATUS}, and comment"
  exit 0
fi

for label in ${labels}; do
  gh issue edit "${ISSUE}" --repo "${CLAIM_REPO}" --remove-label "${label}" >/dev/null
done
claim_set_status "${CLAIM_ITEM_ID}" "${STATUS}"
gh issue comment "${ISSUE}" --repo "${CLAIM_REPO}" \
  --body "Released claim; Project status set to \`${STATUS}\` at $(date -u +%Y-%m-%dT%H:%MZ)." >/dev/null
echo "released ${CLAIM_REPO}#${ISSUE} -> ${STATUS}"
