#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
RESULTS_DIR="$REPO_ROOT/benchmarks/results"
SUMMARY="$RESULTS_DIR/summary.json"

COLD_START_SAMPLES="${COLD_START_SAMPLES:-5}"
STEADY_STATE_SAMPLES="${STEADY_STATE_SAMPLES:-20}"

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

now_ms() {
  if command -v gdate &>/dev/null; then
    gdate +%s%3N
  elif date +%s%3N 2>/dev/null | grep -q '^[0-9]'; then
    date +%s%3N
  else
    # fallback: seconds * 1000
    echo $(( $(date +%s) * 1000 ))
  fi
}

mean() {
  local sum=0
  local count=0
  for v in "$@"; do
    sum=$(( sum + v ))
    count=$(( count + 1 ))
  done
  echo $(( sum / count ))
}

min_val() {
  local m="$1"; shift
  for v in "$@"; do
    [ "$v" -lt "$m" ] && m="$v"
  done
  echo "$m"
}

max_val() {
  local m="$1"; shift
  for v in "$@"; do
    [ "$v" -gt "$m" ] && m="$v"
  done
  echo "$m"
}

# ---------------------------------------------------------------------------
# Pre-flight
# ---------------------------------------------------------------------------

echo "=== Traverse Benchmarks ==="
echo "Repo: $REPO_ROOT"
echo ""

cd "$REPO_ROOT"

echo "Building release binary..."
cargo build --release -p traverse-cli-rs -q
TRAVERSE_CLI="$REPO_ROOT/target/release/traverse-cli"

GIT_SHA="$(git rev-parse --short HEAD 2>/dev/null || echo unknown)"
PLATFORM="$(uname -ms)"
RUN_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

mkdir -p "$RESULTS_DIR"

# ---------------------------------------------------------------------------
# Cold-start benchmark
# ---------------------------------------------------------------------------

echo "Running cold-start benchmark ($COLD_START_SAMPLES samples)..."
CS_SAMPLES=()

for i in $(seq 1 "$COLD_START_SAMPLES"); do
  t0=$(now_ms)
  "$TRAVERSE_CLI" expedition execute \
    "$REPO_ROOT/benchmarks/fixtures/benchmark-request.json" \
    > /dev/null 2>&1 || true
  t1=$(now_ms)
  elapsed=$(( t1 - t0 ))
  CS_SAMPLES+=("$elapsed")
  echo "  sample $i: ${elapsed}ms"
done

CS_MIN=$(min_val "${CS_SAMPLES[@]}")
CS_MAX=$(max_val "${CS_SAMPLES[@]}")
CS_MEAN=$(mean "${CS_SAMPLES[@]}")

# ---------------------------------------------------------------------------
# Steady-state benchmark
# ---------------------------------------------------------------------------

echo ""
echo "Running steady-state benchmark ($STEADY_STATE_SAMPLES samples)..."
SS_SAMPLES=()

for i in $(seq 1 "$STEADY_STATE_SAMPLES"); do
  t0=$(now_ms)
  "$TRAVERSE_CLI" expedition execute \
    "$REPO_ROOT/benchmarks/fixtures/benchmark-request.json" \
    > /dev/null 2>&1 || true
  t1=$(now_ms)
  elapsed=$(( t1 - t0 ))
  SS_SAMPLES+=("$elapsed")
  if (( i % 5 == 0 )); then
    echo "  sample $i: ${elapsed}ms"
  fi
done

SS_MIN=$(min_val "${SS_SAMPLES[@]}")
SS_MAX=$(max_val "${SS_SAMPLES[@]}")
SS_MEAN=$(mean "${SS_SAMPLES[@]}")

# ---------------------------------------------------------------------------
# Write results
# ---------------------------------------------------------------------------

cat > "$SUMMARY" <<EOF
{
  "run_at": "$RUN_AT",
  "git_sha": "$GIT_SHA",
  "platform": "$PLATFORM",
  "cold_start_ms": {
    "min": $CS_MIN,
    "max": $CS_MAX,
    "mean": $CS_MEAN,
    "samples": $COLD_START_SAMPLES
  },
  "steady_state_ms": {
    "min": $SS_MIN,
    "max": $SS_MAX,
    "mean": $SS_MEAN,
    "samples": $STEADY_STATE_SAMPLES
  }
}
EOF

# ---------------------------------------------------------------------------
# Human summary
# ---------------------------------------------------------------------------

echo ""
echo "=== Results ==="
echo "Platform : $PLATFORM"
echo "Git SHA  : $GIT_SHA"
echo ""
echo "Cold start   (n=$COLD_START_SAMPLES):  min=${CS_MIN}ms  max=${CS_MAX}ms  mean=${CS_MEAN}ms"
echo "Steady state (n=$STEADY_STATE_SAMPLES): min=${SS_MIN}ms  max=${SS_MAX}ms  mean=${SS_MEAN}ms"
echo ""
echo "Written to: $SUMMARY"
