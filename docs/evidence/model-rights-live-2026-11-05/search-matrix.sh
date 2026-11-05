#!/usr/bin/env bash
# Runs the registry search filter matrix: search-matrix.sh <work> <cli> <out>.
set -euo pipefail
work="$1"; cli="$2"; out="$3"
cd "${work}"
for q in text.detect-entities text.redact-entities audio.detect-speech-segments audio.transcribe-speech report.translate-fr-semantic; do
  for f in "--model-commercial-use allowed" "--model-commercial-use conditional" "--model-commercial-use forbidden" "--model-commercial-use unknown" "--status active" "--status deprecated" "--status revoked"; do
    name="${q}-$(echo "$f" | sed 's/^--//; s/ /=/')"
    # shellcheck disable=SC2086 # $f holds a flag and its value.
    "${cli}" registry search "${q}" --workspace rights-live $f --json > "${out}/${name}.json"
    printf '%-30s %-34s %s\n' "${q}" "${f}" "$(python3 -I -c 'import json,sys;d=json.load(open(sys.argv[1]));print(sorted(r["version"] for r in d["records"] if r["id"]==sys.argv[2]))' "${out}/${name}.json" "${q}")"
  done
done
