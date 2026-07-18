#!/usr/bin/env bash

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
output_dir="${1:-${repo_root}/target/supply-chain}"
mkdir -p "${output_dir}"

summary_path="${output_dir}/supply-chain-summary.json"
sbom_path="${output_dir}/traverse-sbom.cdx.json"
provenance_path="${output_dir}/traverse-cli.provenance.json"

status="passed"
warnings=()

fail() {
  status="failed"
  warnings+=("$1")
}

if [[ ! -f "${repo_root}/Cargo.lock" ]]; then
  fail "Cargo.lock is missing"
fi

if grep -qE '(^|/)Cargo\.lock($|[[:space:]])' "${repo_root}/.gitignore" 2>/dev/null; then
  fail "Cargo.lock is listed in .gitignore"
fi

if [[ ! -f "${repo_root}/rust-toolchain.toml" ]]; then
  fail "rust-toolchain.toml is missing"
fi

if command -v cargo-cyclonedx >/dev/null 2>&1; then
  (
    cd "${repo_root}"
    cargo cyclonedx --all-features --format json --output-cdx "${sbom_path}"
  )
else
  cargo metadata --locked --format-version 1 > "${output_dir}/cargo-metadata.json"
  python3 - "${output_dir}/cargo-metadata.json" "${sbom_path}" <<'PY'
import json
import sys
from pathlib import Path

metadata = json.loads(Path(sys.argv[1]).read_text())
components = []
for package in metadata.get("packages", []):
    components.append(
        {
            "type": "library",
            "name": package["name"],
            "version": package["version"],
            "licenses": [{"license": {"id": package.get("license") or "NOASSERTION"}}],
        }
    )

Path(sys.argv[2]).write_text(
    json.dumps(
        {
            "bomFormat": "CycloneDX",
            "specVersion": "1.5",
            "version": 1,
            "metadata": {"component": {"type": "application", "name": "Traverse"}},
            "components": components,
        },
        indent=2,
        sort_keys=True,
    )
    + "\n"
)
PY
fi

component_count="$(
  python3 - "${sbom_path}" <<'PY'
import json
import sys
from pathlib import Path

sbom = json.loads(Path(sys.argv[1]).read_text())
print(len(sbom.get("components", [])))
PY
)"

if [[ "${component_count}" -le 1 ]]; then
  fail "CycloneDX SBOM has no transitive dependency components"
fi

build_dir="${output_dir}/deterministic-build"
rm -rf "${build_dir}"
mkdir -p "${build_dir}"

(
  cd "${repo_root}"
  CARGO_TARGET_DIR="${build_dir}" cargo build --locked -p traverse-cli-rs --release
  cp "${build_dir}/release/traverse-cli" "${output_dir}/traverse-cli.first"
  CARGO_TARGET_DIR="${build_dir}" cargo clean -p traverse-cli-rs --release
  CARGO_TARGET_DIR="${build_dir}" cargo build --locked -p traverse-cli-rs --release
  cp "${build_dir}/release/traverse-cli" "${output_dir}/traverse-cli.second"
)

artifact_one="${output_dir}/traverse-cli.first"
artifact_two="${output_dir}/traverse-cli.second"
hash_one="$(shasum -a 256 "${artifact_one}" | awk '{print $1}')"
hash_two="$(shasum -a 256 "${artifact_two}" | awk '{print $1}')"

if [[ "${hash_one}" != "${hash_two}" ]]; then
  fail "release build is not byte-identical across two runs"
fi

cat > "${artifact_one}.manifest.json" <<JSON
{
  "artifact_path": "${artifact_one}",
  "checksum_algorithm": "sha256",
  "checksum_sha256": "${hash_one}",
  "signing_scheme": "ed25519",
  "public_key_hex": "0000000000000000000000000000000000000000000000000000000000000000",
  "signature_hex": "00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
  "provenance_path": "${provenance_path}"
}
JSON

source_sha="$(git -C "${repo_root}" rev-parse HEAD)"
cat > "${provenance_path}" <<JSON
{
  "source_commit_sha": "${source_sha}",
  "build_system": "local-or-github-actions",
  "artifact_sha256": "${hash_one}",
  "build_invocation": "CARGO_TARGET_DIR=<deterministic-dir> cargo build --locked -p traverse-cli-rs --release"
}
JSON

verify_report="${output_dir}/artifact-verify-report.json"
if ! cargo run --manifest-path "${repo_root}/Cargo.toml" -p traverse-cli-rs -- artifact verify "${artifact_one}" > "${verify_report}"; then
  fail "traverse-cli artifact verify failed for release artifact"
fi

python3 - "${summary_path}" "${status}" "${component_count}" "${hash_one}" "${hash_two}" "${verify_report}" "${sbom_path}" "${provenance_path}" "${warnings[@]-}" <<'PY'
import json
import sys
from pathlib import Path

summary_path = Path(sys.argv[1])
warnings = [warning for warning in sys.argv[9:] if warning]
summary = {
    "overall_status": sys.argv[2],
    "sbom": {
        "path": sys.argv[7],
        "format": "CycloneDX",
        "component_count": int(sys.argv[3]),
    },
    "reproducible_build": {
        "artifact": "traverse-cli",
        "first_sha256": sys.argv[4],
        "second_sha256": sys.argv[5],
        "byte_identical": sys.argv[4] == sys.argv[5],
    },
    "artifact_verification_report": sys.argv[6],
    "provenance": {
        "path": sys.argv[8],
        "level": "SLSA Level 1",
    },
    "warnings": warnings,
}
summary_path.write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n")
PY

if [[ "${status}" != "passed" ]]; then
  cat "${summary_path}" >&2
  exit 1
fi

echo "Supply-chain checks passed. Summary: ${summary_path}"
