#!/usr/bin/env bash
# Syntax-check every GitHub Actions `run:` step with `bash -n`, including the
# macOS runners' bash 3.2 (`/bin/bash` on macOS). Catches quoting mistakes
# (e.g. an apostrophe inside a heredoc within `$(...)`) on the PR instead of
# at release time, when a publish step fails mid-run.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
scratch="$(mktemp -d)"
trap 'rm -rf "${scratch}"' EXIT

# Ruby's stdlib YAML (psych) is preinstalled on GitHub's Ubuntu and macOS runners.
ruby -ryaml -e '
  workflows, out = ARGV
  count = 0
  Dir.glob(File.join(workflows, "*.{yml,yaml}")).sort.each do |path|
    doc = YAML.safe_load(File.read(path), aliases: true) || {}
    (doc["jobs"] || {}).each do |job_name, job|
      default_shell = ((job["defaults"] || {})["run"] || {})["shell"] || "bash"
      (job["steps"] || []).each_with_index do |step, index|
        run = step["run"]
        shell = (step["shell"] || default_shell).to_s
        next if run.nil? || !shell.start_with?("bash")
        count += 1
        File.write(File.join(out, "#{File.basename(path, ".*")}__#{job_name}__#{index}.sh"), run)
      end
    end
  end
  warn "extracted #{count} bash run steps"
' "${repo_root}/.github/workflows" "${scratch}"

failures=0
shells=("bash")
[[ -x /bin/bash ]] && shells+=("/bin/bash")
for script in "${scratch}"/*.sh; do
  for shell in "${shells[@]}"; do
    if ! "${shell}" -n "${script}" 2>"${scratch}/err"; then
      echo "workflow-shell-syntax: $(basename "${script}" .sh) fails '${shell} -n':" >&2
      sed 's/^/  /' "${scratch}/err" >&2
      failures=$((failures + 1))
    fi
  done
done
if [[ "${failures}" -ne 0 ]]; then
  exit 1
fi
echo "Workflow shell syntax check passed ($(ls "${scratch}"/*.sh | wc -l | tr -d ' ') run steps; shells: ${shells[*]})."
