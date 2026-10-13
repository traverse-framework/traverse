#!/usr/bin/env bash
# Decision 113 / Spec 138 0.13.3 (#1663): every committed Core ML variant must
# be reproducible from its source ONNX with the pinned converter.
#
# For each fixture package declaring a `coreml` accelerator variant:
# 1. `conversion.tool_version` names the current tools/coreml/requirements.lock
#    digest (`lock-sha256:<hex>`), so a lock change forces re-conversion.
# 2. The source ONNX is the fixtures/onnx file whose SHA-256 equals
#    `conversion.source_digest`.
# 3. Re-converting it with tools/coreml/convert_variant.py yields exactly the
#    committed variant bytes and the manifest's `digest`.
#
# Byte reproducibility is established on macOS arm64 with the hash-locked
# environment (pip --require-hashes); run this there. The interpreter comes
# from TRAVERSE_COREML_PYTHON (default python3).
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
python="${TRAVERSE_COREML_PYTHON:-python3}"
work="$(mktemp -d)"
trap 'rm -rf "${work}"' EXIT

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

lock_version="lock-sha256:$(sha256 "${repo_root}/tools/coreml/requirements.lock")"
checked=0
failed=0

for manifest in "${repo_root}"/fixtures/models/*/model.manifest.json; do
  package="$(dirname "${manifest}")"
  variant="$(jq -c '(.accelerator_variants // [])[] | select(.adapter == "coreml")' "${manifest}")"
  [[ -n "${variant}" ]] || continue
  name="$(basename "${package}")"
  path="$(jq -r .path <<<"${variant}")"
  digest="$(jq -r '.digest | ltrimstr("sha256:")' <<<"${variant}")"
  source_digest="$(jq -r '.conversion.source_digest | ltrimstr("sha256:") | ascii_downcase' <<<"${variant}")"
  tool_version="$(jq -r .conversion.tool_version <<<"${variant}")"

  if [[ "${tool_version}" != "${lock_version}" ]]; then
    echo "${name}: conversion.tool_version ${tool_version} is not the current ${lock_version}; re-convert it." >&2
    failed=$((failed + 1))
    continue
  fi
  if [[ "$(sha256 "${package}/${path}")" != "${digest}" ]]; then
    echo "${name}: committed ${path} does not match the manifest digest." >&2
    failed=$((failed + 1))
    continue
  fi

  source=""
  for candidate in "${repo_root}"/fixtures/onnx/*.onnx; do
    if [[ "$(sha256 "${candidate}")" == "${source_digest}" ]]; then
      source="${candidate}"
      break
    fi
  done
  if [[ -z "${source}" ]]; then
    echo "${name}: no fixtures/onnx file has the source digest ${source_digest}." >&2
    failed=$((failed + 1))
    continue
  fi

  out="${work}/${name}.mlpackage.zip"
  "${python}" "${repo_root}/tools/coreml/convert_variant.py" "${source}" "${out}" >/dev/null
  reconverted="$(sha256 "${out}")"
  if [[ "${reconverted}" != "${digest}" ]]; then
    echo "${name}: re-conversion produced ${reconverted}, committed ${digest}." >&2
    failed=$((failed + 1))
    continue
  fi
  echo "${name}: ${path} re-converts to ${digest}"
  checked=$((checked + 1))
done

if [[ "${failed}" -ne 0 ]]; then
  exit 1
fi
if [[ "${checked}" -eq 0 ]]; then
  echo "No committed Core ML variant found; expected at least fixtures/models/digits-onnx-coreml-1.0.0." >&2
  exit 1
fi
