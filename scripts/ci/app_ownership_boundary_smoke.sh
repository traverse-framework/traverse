#!/usr/bin/env bash

set -euo pipefail

repo_root="${TRAVERSE_REPO_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"

if [[ -d "${repo_root}/apps" ]]; then
  echo "Application source must live in traverse-framework/reference-apps, not apps/" >&2
  exit 1
fi

for forbidden_path in \
  "apps/demo-fixtures" \
  "apps/meeting-notes" \
  "apps/traverse-starter"
do
  if [[ -d "${repo_root}/${forbidden_path}" ]] &&
     [[ -n "$(find "${repo_root}/${forbidden_path}" -type f -print -quit)" ]]; then
    echo "Runtime-owned fixture remains under ${forbidden_path}" >&2
    exit 1
  fi
done

required_paths=(
  "examples/fixtures/expedition-runtime-session.json"
  "examples/applications/meeting-notes/app.manifest.json"
  "examples/applications/meeting-notes/components/process/component.manifest.json"
  "examples/applications/traverse-starter/app.manifest.json"
  "examples/applications/traverse-starter/components/process/component.manifest.json"
)

for required_path in "${required_paths[@]}"; do
  test -s "${repo_root}/${required_path}"
done

grep -Fq "Decision 22: Keep Application Source Out of the Traverse Runtime Repository" \
  "${repo_root}/docs/decision-log.md"

for app_id in traverse-starter meeting-notes; do
  validate_output="$(
    cd "${repo_root}"
    cargo run -q -p traverse-cli -- app validate \
      --manifest "examples/applications/${app_id}/app.manifest.json" --json
  )"
  grep -q '"status": "validated"' <<<"${validate_output}"
  grep -q "\"app_id\": \"${app_id}\"" <<<"${validate_output}"
done

starter_output="$(
  cd "${repo_root}"
  cargo run -q -p traverse-cli -- agent execute \
    examples/traverse-starter/process-agent/manifest.json \
    examples/traverse-starter/runtime-requests/process.json
)"
grep -q "status: completed" <<<"${starter_output}"
grep -q "capability_id: traverse-starter.process" <<<"${starter_output}"

meeting_output="$(
  cd "${repo_root}"
  cargo run -q -p traverse-cli -- agent execute \
    examples/meeting-notes/process-agent/manifest.json \
    examples/meeting-notes/runtime-requests/process.json
)"
grep -q "status: completed" <<<"${meeting_output}"
grep -q "capability_id: meeting-notes.process" <<<"${meeting_output}"

TRAVERSE_REPO_ROOT="${repo_root}" bash "${repo_root}/scripts/ci/runtime_home_smoke.sh"

echo "App ownership boundary smoke test passed."
