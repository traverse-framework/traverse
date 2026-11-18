#!/usr/bin/env bash
# Decision 103 (#1567): re-sign Spec 138 model packages with the production
# model-signing key. Run ONLY by .github/workflows/sign-model-packages.yml in
# the protected `model-signing` environment (workflow_dispatch; never on a
# pull request). The secret arrives as MODEL_SIGNING_KEY_HEX and is written
# only to a 0600 temp file that is deleted on exit.
#
#   MODEL_SIGNING_KEY_HEX=... bash scripts/model-signing/sign-packages.sh digits-mlp-1.0.0 [...]
#
# Each argument names a package directory under fixtures/models/. After
# signing, every package must verify against the committed public keys in
# keys/model-signing/ (so the secret must belong to a published key).
#
# SIGN_PACKAGES_ROOT (Decision 135, #1891): the checkout that holds the
# packages to sign, i.e. the pinned PR commit. Defaults to this repository.
# The CLI and the trusted public keys always come from this repository (the
# trusted `main` checkout), never from the pinned commit.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${repo_root}"

if [[ -z "${MODEL_SIGNING_KEY_HEX:-}" ]]; then
  echo "MODEL_SIGNING_KEY_HEX is not set (run in the model-signing environment)." >&2
  exit 1
fi
if [[ "$#" -eq 0 ]]; then
  echo "usage: sign-packages.sh <package-dir-name>..." >&2
  exit 2
fi

trusted=()
while IFS= read -r public_key; do
  trusted+=(--trusted-key "$(tr -d '[:space:]' < "${public_key}")")
done < <(find keys/model-signing -name '*.pub' | sort)
if [[ "${#trusted[@]}" -eq 0 ]]; then
  echo "No committed public keys in keys/model-signing/; generate and commit the key first." >&2
  exit 1
fi

key_file="$(mktemp)"
chmod 600 "${key_file}"
trap 'rm -f "${key_file}"' EXIT
printf '%s\n' "${MODEL_SIGNING_KEY_HEX}" > "${key_file}"

packages_root="${SIGN_PACKAGES_ROOT:-${repo_root}}"
if [[ ! -d "${packages_root}/fixtures/models" ]]; then
  echo "SIGN_PACKAGES_ROOT has no fixtures/models: ${packages_root}" >&2
  exit 1
fi

cargo build --quiet --locked --release -p traverse-cli-rs
cli="$(cargo metadata --no-deps --format-version 1 | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')/release/traverse-cli"

for package in "$@"; do
  if [[ ! "${package}" =~ ^[A-Za-z0-9._-]+$ ]] || [[ ! -f "${packages_root}/fixtures/models/${package}/model.manifest.json" ]]; then
    echo "Not a model package under fixtures/models/: ${package}" >&2
    exit 1
  fi
  dir="${packages_root}/fixtures/models/${package}"
  "${cli}" model sign "${dir}/model.manifest.json" --key "${key_file}"
  "${cli}" model verify "${dir}" "${trusted[@]}"
done
