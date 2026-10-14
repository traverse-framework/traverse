#!/usr/bin/env bash
# Rebuild runtime.wasm from source and fail if the digest differs from the
# committed runtime/runtime.wasm.sha256 (and companion copies). Issue #1659.

set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
cd "${repo_root}"

expected_file="${repo_root}/runtime/runtime.wasm.sha256"
if [[ ! -f "${expected_file}" ]]; then
  echo "runtime_wasm_reproducibility_check: missing ${expected_file}" >&2
  exit 1
fi
expected="$(tr -d '[:space:]' <"${expected_file}")"
expected="${expected#sha256:}"

# Rebuild into a temp dir (stdout: path, then digest). Status goes to stderr.
rebuild_out="$(mktemp "${TMPDIR:-/tmp}/traverse-runtime-rebuild.XXXXXX")"
bash "${repo_root}/scripts/build-runtime-wasm.sh" >"${rebuild_out}"
digest="$(tail -n 1 "${rebuild_out}")"
built_path="$(tail -n 2 "${rebuild_out}" | head -n 1)"
rm -f "${rebuild_out}"

if [[ "${digest}" != "${expected}" ]]; then
  echo "runtime_wasm_reproducibility_check: digest mismatch" >&2
  echo "  committed: ${expected}" >&2
  echo "  rebuilt:   ${digest}" >&2
  echo "  artifact:  ${built_path}" >&2
  exit 1
fi

python3 - "${repo_root}" "${expected}" <<'PY'
import hashlib
import json
import pathlib
import re
import sys

root = pathlib.Path(sys.argv[1])
expected = sys.argv[2]
paths = [
    root / "runtime" / "runtime.wasm",
    root / "examples" / "applications" / "audio-analysis" / "runtime" / "runtime.wasm",
    root / "examples" / "applications" / "traverse-starter" / "runtime" / "runtime.wasm",
    root / "packages" / "web" / "TraverseEmbedder" / "tests" / "fixtures" / "runtime.wasm",
]
failed = False
for path in paths:
    if not path.is_file():
        print(f"missing {path}", file=sys.stderr)
        failed = True
        continue
    got = hashlib.sha256(path.read_bytes()).hexdigest()
    if got != expected:
        print(f"digest mismatch for {path}: {got} != {expected}", file=sys.stderr)
        failed = True
release = root / "runtime" / "runtime-release.json"
meta = json.loads(release.read_text(encoding="utf-8"))
if meta.get("sha256") != expected:
    print(f"runtime-release.json sha256 mismatch: {meta.get('sha256')}", file=sys.stderr)
    failed = True
cargo = (root / "Cargo.toml").read_text(encoding="utf-8")
match = re.search(
    r'(?ms)^\[workspace\.package\].*?^version\s*=\s*"([^"]+)"',
    cargo,
)
if match is None:
    print("could not read workspace.package.version", file=sys.stderr)
    failed = True
elif meta.get("runtime_version") != match.group(1):
    print(
        f"runtime-release.json runtime_version {meta.get('runtime_version')!r} "
        f"!= workspace {match.group(1)!r}",
        file=sys.stderr,
    )
    failed = True
if failed:
    raise SystemExit(1)
print(json.dumps({"ok": True, "sha256": expected, "runtime_version": meta["runtime_version"]}))
PY

echo "runtime_wasm_reproducibility_check: ok (sha256=${digest})"
