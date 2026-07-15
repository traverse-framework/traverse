#!/usr/bin/env bash

set -euo pipefail

repo_root="${TRAVERSE_REPO_ROOT:-$(pwd)}"

doc="${repo_root}/docs/local-runtime-home.md"
template_doc="${repo_root}/docs/executable-package-template.md"
authoring_doc="${repo_root}/docs/expedition-example-authoring.md"

test -f "$doc"
test -s "$doc"

grep -q 'default path: `.traverse/local/`' "$doc"
grep -q 'TRAVERSE_RUNTIME_HOME=/absolute/path' "$doc"
grep -q '.traverse/local/' "$doc"
grep -q 'bin/' "$doc"
grep -q 'fixtures/' "$doc"
grep -q 'overlays/' "$doc"
grep -q 'examples/fixtures/' "$doc"
grep -q 'package-local `./artifacts/` directories' "$doc"
grep -q 'bash scripts/ci/runtime_home_smoke.sh' "$doc"

grep -q 'docs/local-runtime-home.md' "$template_doc"
grep -q 'docs/local-runtime-home.md' "$authoring_doc"

printf 'Runtime home smoke passed.\n'
