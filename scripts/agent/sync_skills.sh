#!/usr/bin/env bash
# Generate every tool's copy of the agent skills from the single source
# (.github#58).
#
#   scripts/agent/sync_skills.sh            # write .claude/skills and .cursor/skills
#   scripts/agent/sync_skills.sh --check    # exit 1 if any generated copy differs
#
# Source: .agents/skills/<skill>/... (read by Codex directly).
# Generated: .claude/skills/<skill>/... and .cursor/skills/<skill>/..., which
# this script owns completely: a file there with no source is removed (and
# reported by --check). Every file of a skill is copied. Each SKILL.md gets a
# "generated" marker right after its YAML frontmatter, because the frontmatter
# must stay on line 1 for the tools to load the skill.
#
# SKILLS_ROOT overrides the repository root (tests use it).
set -euo pipefail

root="${SKILLS_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
src="${root}/.agents/skills"
targets=(".claude/skills" ".cursor/skills")
marker="<!-- GENERATED from .agents/skills by scripts/agent/sync_skills.sh. Edit the source, then re-run the script. -->"

mode="write"
case "${1:-}" in
  "") ;;
  --check) mode="check" ;;
  *) echo "usage: sync_skills.sh [--check]" >&2; exit 2 ;;
esac

if [[ ! -d "${src}" ]] || [[ -z "$(find "${src}" -mindepth 2 -name SKILL.md -print -quit)" ]]; then
  echo "no skills under .agents/skills; nothing to sync"
  exit 0
fi

# render <out-dir>: write the generated tree for one tool into <out-dir>.
render() {
  local out="$1" skill_dir name file rel dest
  mkdir -p "${out}"
  for skill_dir in "${src}"/*/; do
    [[ -f "${skill_dir}SKILL.md" ]] || continue
    name="$(basename "${skill_dir}")"
    while IFS= read -r -d '' file; do
      rel="${file#"${skill_dir}"}"
      dest="${out}/${name}/${rel}"
      mkdir -p "$(dirname "${dest}")"
      if [[ "${rel}" == "SKILL.md" ]]; then
        awk -v marker="${marker}" '
          NR == 1 && $0 == "---" { infm = 1; print; next }
          infm && $0 == "---" { print; print ""; print marker; infm = 0; done = 1; next }
          { print }
          END { if (!done) exit 3 }
        ' "${file}" > "${dest}" || {
          echo "${file}: SKILL.md must start with YAML frontmatter (--- ... ---)" >&2
          return 1
        }
      else
        cp "${file}" "${dest}"
      fi
    done < <(find "${skill_dir}" -type f ! -name '.DS_Store' -print0 | sort -z)
  done
}

work="$(mktemp -d)"
trap 'rm -rf "${work}"' EXIT
render "${work}/expected"

if [[ "${mode}" == "write" ]]; then
  for target in "${targets[@]}"; do
    rm -rf "${root:?}/${target}"
    mkdir -p "$(dirname "${root}/${target}")"
    cp -R "${work}/expected" "${root}/${target}"
  done
  echo "synced $(find "${src}" -mindepth 2 -maxdepth 2 -name SKILL.md | wc -l | tr -d ' ') skill(s) into ${targets[*]}"
  exit 0
fi

drift=0
for target in "${targets[@]}"; do
  actual="${root}/${target}"
  if [[ ! -d "${actual}" ]]; then
    echo "missing: ${target} (run scripts/agent/sync_skills.sh)" >&2
    drift=1
    continue
  fi
  if ! diff -r "${work}/expected" "${actual}" > "${work}/diff.txt" 2>&1; then
    while IFS= read -r line; do
      case "${line}" in
        "Only in ${work}/expected"*) echo "missing in ${target}: ${line#*: }" >&2 ;;
        "Only in ${actual}"*) echo "extra in ${target} (no source): ${line#*: }" >&2 ;;
        "diff -r "*) echo "drifted: ${target}/${line##*"${actual}/"}" >&2 ;;
      esac
    done < "${work}/diff.txt"
    drift=1
  fi
done
if [[ "${drift}" -ne 0 ]]; then
  echo "generated skill copies differ from .agents/skills; edit the source and run scripts/agent/sync_skills.sh" >&2
  exit 1
fi
echo "skill copies match .agents/skills"
