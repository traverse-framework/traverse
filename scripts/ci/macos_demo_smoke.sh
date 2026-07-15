#!/usr/bin/env bash
set -euo pipefail

repo_root="${TRAVERSE_REPO_ROOT:-$(pwd)}"

required_files=(
  "examples/fixtures/expedition-runtime-session.json"
  "apps/macos-demo/Package.swift"
  "apps/macos-demo/README.md"
  "apps/macos-demo/Sources/TraverseMacOSDemoApp/TraverseMacOSDemoApp.swift"
  "apps/macos-demo/Sources/TraverseMacOSDemoApp/DemoSession.swift"
  "apps/macos-demo/Sources/TraverseMacOSDemoApp/DemoContentView.swift"
)

for file in "${required_files[@]}"; do
  if [[ ! -f "${repo_root}/${file}" ]]; then
    echo "missing macOS demo artifact: ${file}" >&2
    exit 1
  fi
done

grep -q 'import SwiftUI' "${repo_root}/apps/macos-demo/Sources/TraverseMacOSDemoApp/TraverseMacOSDemoApp.swift"
grep -q 'WindowGroup' "${repo_root}/apps/macos-demo/Sources/TraverseMacOSDemoApp/TraverseMacOSDemoApp.swift"
grep -q '"status": "completed"' "${repo_root}/examples/fixtures/expedition-runtime-session.json"
grep -q '"state_updates"' "${repo_root}/examples/fixtures/expedition-runtime-session.json"

echo "macOS demo smoke passed"
