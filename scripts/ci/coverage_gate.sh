#!/usr/bin/env bash

set -euo pipefail

readonly TARGETS_FILE="ci/coverage-targets.txt"

if [[ ! -f "${TARGETS_FILE}" ]]; then
  echo "Missing coverage target configuration: ${TARGETS_FILE}" >&2
  exit 1
fi

targets=()
while IFS= read -r line; do
  targets+=("${line}")
done < <(grep -Ev '^\s*(#|$)' "${TARGETS_FILE}" || true)

if [[ ${#targets[@]} -eq 0 ]]; then
  echo "No protected coverage targets configured yet. Coverage gate passes by design."
  exit 0
fi

if ! command -v cargo-llvm-cov >/dev/null 2>&1; then
  echo "cargo-llvm-cov is required for the coverage gate." >&2
  exit 1
fi

failed=0

for entry in "${targets[@]}"; do
  crate_name="$(awk '{print $1}' <<<"${entry}")"
  minimum_percent="$(awk '{print $2}' <<<"${entry}")"

  if [[ -z "${crate_name}" || -z "${minimum_percent}" ]]; then
    echo "Invalid coverage target entry: ${entry}" >&2
    failed=1
    continue
  fi

  echo "Measuring line coverage for ${crate_name} with threshold ${minimum_percent}%"
  coverage_output="$(cargo llvm-cov --package "${crate_name}" --lcov -- --test-threads=1)"
  line_counts="$(
    awk -F '[:,]' '
      /^SF:/ { current_file = substr($0, 4); next }
      /^DA:/ {
        key = current_file ":" $2
        counts[key] += $3
      }
      END {
        total = 0
        hit = 0
        for (key in counts) {
          total += 1
          if (counts[key] > 0) {
            hit += 1
          }
        }
        if (total > 0) {
          printf "%s %s", hit, total
        }
      }
    ' <<<"${coverage_output}"
  )"

  if [[ -z "${line_counts}" ]]; then
    echo "Unable to parse line coverage for ${crate_name}" >&2
    echo "${coverage_output}" >&2
    failed=1
    continue
  fi

  hit_lines="$(awk '{print $1}' <<<"${line_counts}")"
  total_lines="$(awk '{print $2}' <<<"${line_counts}")"
  line_percent="$(
    awk -v hit="${hit_lines}" -v total="${total_lines}" \
      'BEGIN { printf "%.2f", (hit / total) * 100 }'
  )"

  printf 'Line coverage for %s: %s%% (%s/%s lines)\n' \
    "${crate_name}" "${line_percent}" "${hit_lines}" "${total_lines}"

  if ! awk -v actual="${line_percent}" -v required="${minimum_percent}" \
    'BEGIN { exit (actual + 0 >= required + 0) ? 0 : 1 }'; then
    echo "Coverage gate failed for ${crate_name}: ${line_percent}% < ${minimum_percent}%." >&2
    failed=1
  fi
done

if [[ ${failed} -ne 0 ]]; then
  exit 1
fi

echo "Coverage gate passed."
