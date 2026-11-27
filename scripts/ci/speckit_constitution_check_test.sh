#!/usr/bin/env bash
# Fixture tests for scripts/ci/speckit_constitution_check.sh (#1936): the real
# skills pass, and removing each guarded phrase fails with a message naming it.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
check="${root}/scripts/ci/speckit_constitution_check.sh"
work="$(mktemp -d)"
failures=0

fresh() { # -> copies the three real skills into ${work}/skills
  local s
  rm -rf "${work}/skills"
  for s in speckit-analyze speckit-plan speckit-constitution; do
    mkdir -p "${work}/skills/${s}"
    cp "${root}/.agents/skills/${s}/SKILL.md" "${work}/skills/${s}/SKILL.md"
  done
}

# skill phrase expected -> removes every (case-insensitive) copy of phrase from the
# skill, runs the check, and requires exit 1 with expected on stderr
expect_fail() {
  fresh
  PHRASE="$2" perl -0pi -e 's/\Q$ENV{PHRASE}\E//gi' "${work}/skills/$1/SKILL.md"
  local rc=0
  bash "${check}" "${work}/skills" 2>"${work}/err" || rc=$?
  if [[ "${rc}" -eq 1 ]] && grep -qF -- "$3" "${work}/err"; then
    echo "ok: removing '$2' from $1 fails"
  else
    echo "FAIL: removing '$2' from $1 gave rc=${rc}: $(cat "${work}/err")"
    failures=$((failures + 1))
  fi
}

fresh
if bash "${check}" "${work}/skills"; then
  echo "ok: real skills pass"
else
  echo "FAIL: real skills do not pass"
  failures=$((failures + 1))
fi

fetch="repos/traverse-framework/.github/contents/constitution.md?ref=\$(tr -d '[:space:]' < .governance-version)"
stop="If the fetch fails, stop and report it"
for s in speckit-analyze speckit-plan speckit-constitution; do
  expect_fail "${s}" "${fetch}" "${s}/SKILL.md is missing: ${fetch}"
  expect_fail "${s}" "${stop}" "${s}/SKILL.md is missing: ${stop}"
done
rules="\`## Traverse-Specific Rules\` section of \`.specify/memory/constitution.md\`"
for s in speckit-analyze speckit-plan; do
  expect_fail "${s}" "${rules}" "${s}/SKILL.md is missing: ${rules}"
done
edit="Edit only the \`## Traverse-Specific Rules\` section"
expect_fail speckit-constitution "${edit}" "speckit-constitution/SKILL.md is missing: ${edit}"

fresh
echo "This file is a TEMPLATE" >> "${work}/skills/speckit-constitution/SKILL.md"
rc=0
bash "${check}" "${work}/skills" 2>"${work}/err" || rc=$?
if [[ "${rc}" -eq 1 ]] && grep -qF "treats .specify/memory/constitution.md as a template" "${work}/err"; then
  echo "ok: template wording fails"
else
  echo "FAIL: template wording gave rc=${rc}"
  failures=$((failures + 1))
fi

rm -rf "${work}"
if [[ "${failures}" -ne 0 ]]; then
  echo "${failures} speckit constitution check test(s) failed" >&2
  exit 1
fi
echo "speckit constitution check tests passed"
