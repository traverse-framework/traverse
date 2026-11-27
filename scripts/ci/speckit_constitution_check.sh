#!/usr/bin/env bash
# The speckit skills read the org constitution at the pinned release and stop if
# it cannot be fetched; speckit-constitution edits only the Traverse-specific
# rules (#1932, #1936). Each failure names the skill and the missing text.
#
#   scripts/ci/speckit_constitution_check.sh [skills-dir]   (default .agents/skills)
set -euo pipefail

skills_dir="${1:-.agents/skills}"

require_skill_text() { # skill text -> exits 1 with a message unless the skill's SKILL.md contains text (case-insensitive)
  if ! grep -qiF -- "$2" "${skills_dir}/$1/SKILL.md"; then
    echo "error: ${skills_dir}/$1/SKILL.md is missing: $2" >&2
    exit 1
  fi
}

for skill in speckit-analyze speckit-plan speckit-constitution; do
  require_skill_text "${skill}" "repos/traverse-framework/.github/contents/constitution.md?ref=\$(tr -d '[:space:]' < .governance-version)"
  require_skill_text "${skill}" "If the fetch fails, stop and report it"
done
for skill in speckit-analyze speckit-plan; do
  require_skill_text "${skill}" "\`## Traverse-Specific Rules\` section of \`.specify/memory/constitution.md\`"
done
require_skill_text speckit-constitution "Edit only the \`## Traverse-Specific Rules\` section"
if grep -q "This file is a TEMPLATE" "${skills_dir}/speckit-constitution/SKILL.md"; then
  echo "error: speckit-constitution treats .specify/memory/constitution.md as a template and would overwrite the pointer" >&2
  exit 1
fi
