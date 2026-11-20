#!/usr/bin/env bash
# Shared helpers for claim.sh and release.sh (.github#57). Sourced, not run.
#
# Config file (JSON), one per repo, default <repo root>/.agent-claim.json:
#   {
#     "repo": "traverse-framework/traverse",
#     "project_id": "PVT_...",
#     "status_field_id": "PVTSSF_...",
#     "status_options": { "Ready": "...", "In Progress": "...", "Blocked": "...", "Done": "..." }
#   }

claim_die() {
  local code="$1"
  shift
  echo "$*" >&2
  exit "${code}"
}

# claim_load_config <path-or-empty> <repo-override-or-empty>
# Sets CLAIM_REPO, CLAIM_PROJECT_ID, CLAIM_STATUS_FIELD_ID, CLAIM_CONFIG_PATH.
claim_load_config() {
  local path="$1" repo_override="$2"
  if [[ -z "${path}" ]]; then
    local root
    root="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
    path="${root}/.agent-claim.json"
  fi
  [[ -f "${path}" ]] || claim_die 2 "claim config not found: ${path} (see docs/ai-agent-hardening.md)"
  jq -e '(.project_id|type=="string") and (.status_field_id|type=="string") and (.status_options|type=="object")' \
    "${path}" >/dev/null 2>&1 || claim_die 2 "claim config ${path} needs project_id, status_field_id and status_options"
  CLAIM_CONFIG_PATH="${path}"
  CLAIM_PROJECT_ID="$(jq -r .project_id "${path}")"
  CLAIM_STATUS_FIELD_ID="$(jq -r .status_field_id "${path}")"
  CLAIM_REPO="${repo_override:-$(jq -r '.repo // empty' "${path}")}"
  [[ "${CLAIM_REPO}" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]] ||
    claim_die 2 "repo must be OWNER/REPO (from --repo or the config's \"repo\"): '${CLAIM_REPO}'"
}

# claim_status_option <status name> -> option id on stdout
claim_status_option() {
  local option
  option="$(jq -r --arg s "$1" '.status_options[$s] // empty' "${CLAIM_CONFIG_PATH}")"
  [[ -n "${option}" ]] || claim_die 2 "status '$1' has no option id in ${CLAIM_CONFIG_PATH}"
  printf '%s\n' "${option}"
}

# claim_project_item <issue> -> sets CLAIM_ITEM_ID and CLAIM_ITEM_STATUS (empty if absent)
claim_project_item() {
  local row
  # shellcheck disable=SC2016
  row="$(gh api graphql \
    -f query='query($owner:String!,$name:String!,$number:Int!){repository(owner:$owner,name:$name){issue(number:$number){projectItems(first:20){nodes{id project{id} fieldValueByName(name:"Status"){... on ProjectV2ItemFieldSingleSelectValue{name}}}}}}}' \
    -f owner="${CLAIM_REPO%%/*}" -f name="${CLAIM_REPO#*/}" -F number="$1" \
    --jq "[.data.repository.issue.projectItems.nodes[] | select(.project.id == \"${CLAIM_PROJECT_ID}\")][0] | if . == null then \"\" else \"\\(.id)\\t\\(.fieldValueByName.name // \"\")\" end")"
  CLAIM_ITEM_ID="${row%%$'\t'*}"
  CLAIM_ITEM_STATUS=""
  [[ "${row}" == *$'\t'* ]] && CLAIM_ITEM_STATUS="${row#*$'\t'}"
  return 0
}

# claim_issue_branches <issue> -> matching remote branches from any tool, space-separated
claim_issue_branches() {
  git ls-remote --heads "https://github.com/${CLAIM_REPO}.git" 2>/dev/null |
    awk '{print $2}' | sed 's#^refs/heads/##' |
    grep -E "^[^/]+/issue-$1-" | tr '\n' ' ' | sed 's/ $//' || true
}

# claim_issue_prs <issue> -> open PRs whose head branch is */issue-<N>-*, space-separated
claim_issue_prs() {
  gh pr list --repo "${CLAIM_REPO}" --state open --limit 200 --json number,headRefName \
    --jq "[.[] | select(.headRefName | test(\"^[^/]+/issue-$1-\")) | \"#\\(.number) \\(.headRefName)\"] | join(\" \")"
}

# claim_set_status <item id> <status name>
claim_set_status() {
  local option
  option="$(claim_status_option "$2")"
  gh project item-edit --project-id "${CLAIM_PROJECT_ID}" --id "$1" \
    --field-id "${CLAIM_STATUS_FIELD_ID}" --single-select-option-id "${option}" >/dev/null
}
