#!/usr/bin/env bash

set -euo pipefail

hook_path="$(git rev-parse --git-path hooks/pre-push)"

cat >"$hook_path" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

if ! command -v gh >/dev/null 2>&1; then
  echo "Traverse pre-push: gh is unavailable; run scripts/ci/local_preflight.sh manually." >&2
  exit 1
fi

if ! pr_number="$(gh pr view --json number --jq .number 2>/dev/null)"; then
  echo "Traverse pre-push: no PR exists for this branch." >&2
  echo "Run BASE_SHA=origin/main bash scripts/ci/local_preflight.sh --pr-body <pr-body-file> before the first push." >&2
  exit 1
fi

bash scripts/ci/local_preflight.sh --pr "$pr_number"
EOF

chmod +x "$hook_path"
echo "Installed Traverse CI pre-push hook at $hook_path"
