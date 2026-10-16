#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
artifact_dir="$script_dir/artifacts"
artifact_path="$artifact_dir/capture-expedition-objective-agent.wasm"
manifest_path="$script_dir/manifest.json"

mkdir -p "$artifact_dir"

fnv_digest() {
  python3 - "$1" <<'PY'
import sys
from pathlib import Path

data = Path(sys.argv[1]).read_bytes()
h = 0xcbf29ce484222325
for b in data:
    h ^= b
    h = (h * 0x100000001b3) & 0xFFFFFFFFFFFFFFFF
print(f"fnv1a64:{h:016x}")
PY
}

digest_matches() {
  [[ -f "$artifact_path" && -f "$manifest_path" ]] || return 1
  local expected actual
  expected="$(python3 - "$manifest_path" <<'PY'
import json
import sys
from pathlib import Path

print(json.loads(Path(sys.argv[1]).read_text())["binary"]["expected_digest"])
PY
)"
  actual="$(fnv_digest "$artifact_path")"
  [[ "$actual" == "$expected" ]]
}

if digest_matches; then
  printf 'reused %s\n' "$artifact_path"
  exit 0
fi

lock_dir="$artifact_dir/.build-fixture.lockdir"
for _ in $(seq 1 600); do
  if mkdir "$lock_dir" 2>/dev/null; then
    break
  fi
  sleep 0.1
done
if [[ ! -d "$lock_dir" ]]; then
  echo "build-fixture: timed out waiting for lock at $lock_dir" >&2
  exit 1
fi

cleanup() {
  rm -rf "$tmp_out" "$lock_dir"
}
tmp_out=""
trap cleanup EXIT

# Re-check under the lock — another builder may have finished.
if digest_matches; then
  printf 'reused %s\n' "$artifact_path"
  exit 0
fi

tmp_out="$(mktemp -d "${TMPDIR:-/tmp}/traverse-fixture.XXXXXX")"
rustup run "$(rustup show active-toolchain | awk '{print $1}')" rustc "$script_dir/src/agent.rs" --target wasm32-unknown-unknown --crate-type cdylib -O -C panic=abort -C strip=symbols --remap-path-prefix "$script_dir=/traverse-repo/agent" -o "$tmp_out/agent.wasm"
mv "$tmp_out/agent.wasm" "$artifact_path"
printf 'built %s\n' "$artifact_path"
