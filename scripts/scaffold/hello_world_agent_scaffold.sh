#!/usr/bin/env bash

set -euo pipefail

repo_root="${TRAVERSE_REPO_ROOT:-$(pwd)}"
cd "${repo_root}"

src_dir="examples/hello-world/say-hello-agent"
if [[ ! -d "${src_dir}" ]]; then
  echo "Missing source template directory: ${src_dir}" >&2
  exit 1
fi

tmpdir="$(mktemp -d)"
dest_dir="${tmpdir}/say-hello-agent"

cp -R "${src_dir}" "${dest_dir}"

cat <<EOF
Scaffold created at:
  ${dest_dir}

Next steps:
1) Open and edit:
   - manifest.json
   - runtime-requests (if you add new ones)
2) Build the deterministic fixture:
   bash ${dest_dir}/build-fixture.sh
3) Inspect and execute via Traverse CLI:
   cargo run -p traverse-cli-rs -- agent inspect ${dest_dir}/manifest.json
   cargo run -p traverse-cli-rs -- agent execute ${dest_dir}/manifest.json examples/hello-world/runtime-requests/say-hello.json

When you are ready to turn this into a real package, follow:
  docs/getting-started.md
EOF

