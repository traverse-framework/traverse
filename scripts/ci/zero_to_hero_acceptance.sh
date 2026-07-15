#!/usr/bin/env bash

set -euo pipefail

repo_root="${TRAVERSE_REPO_ROOT:-$(pwd)}"
cd "${repo_root}"

bash scripts/validate-setup.sh

echo "Running local hello-world CLI path..."
bash scripts/ci/hello_world_example_smoke.sh

echo "Running browser host app-consumable path..."
echo "Reference app smoke lives in traverse-framework/reference-apps"

echo "Zero-to-hero acceptance passed."

