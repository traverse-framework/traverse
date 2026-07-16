# Durable Event Journal Operational Limits

This note records the first representative local measurements for the durable
event journal implemented under Specs 066 and 067. It is evidence for issue
#629, not a cross-platform service-level guarantee.

## Reproduce

Run the release-mode harness from the repository root:

```bash
cargo run --release -p traverse-runtime --example journal_operational_limits

TRAVERSE_JOURNAL_BENCH_EVENTS=2500 \
TRAVERSE_JOURNAL_BENCH_PAYLOAD_BYTES=4096 \
cargo run --release -p traverse-runtime --example journal_operational_limits
```

The harness emits stable JSON covering fsync-before-ack append latency,
restart recovery, replay throughput, whole-segment retention pruning, segment
counts, and disk growth. Workload size and payload bytes can be overridden by
environment variables without changing the implementation.

## Measurement environment

- Date: 2026-07-15
- Host: Darwin arm64, macOS 26.5.2
- Toolchain: `rustc 1.96.0 (ac68faa20 2026-05-25)`
- Build: Cargo release profile
- Storage: host-local filesystem
- Writer model: one journal writer, matching the initial implementation

Raw results are checked in at
`docs/evidence/journal-operational-limits-2026-07-15.json`.

## Results

| Workload | Append p50 | Append p95 | Append p99 | Max | Recovery | Replay | Prune |
|---|---:|---:|---:|---:|---:|---:|---:|
| 1,000 events, 512-byte payload | 3.975 ms | 4.639 ms | 5.062 ms | 8.108 ms | 5.856 ms | 208,203 events/s | 49 segments in 4.118 ms |
| 2,500 events, 4,096-byte payload | 3.971 ms | 5.077 ms | 6.205 ms | 10.809 ms | 55.108 ms | 146,404 events/s | 622 segments in 24.932 ms |

The deliberately small 16 KiB benchmark segment forces rollover and stresses
recovery and pruning. The production default remains 64 MiB or 10 minutes,
whichever comes first. Disk growth was payload plus approximately 390 bytes
per event in both measured workloads. Whole-segment size retention reduced the
workloads to 62,214 bytes and 53,852 bytes respectively, while correctly
retaining the active segment.

## Recommended defaults and limits

- Keep the Spec 067 defaults: 64 MiB maximum segment size, 10-minute maximum
  segment age, and a 2-second durable-write timeout. The measured worst append
  was 10.809 ms, leaving substantial timeout headroom without weakening
  fsync-before-acknowledgement.
- Configure size retention explicitly for production workspaces. A starting
  budget is `peak retained events * (average payload bytes + 400 bytes)`, plus
  one full active-segment allowance because the active segment is never
  pruned.
- Use age retention when the business requirement is time based; combine it
  with size retention when disk usage must also be bounded.
- Treat append p99 above 25 ms, recovery above 1 second, or sustained disk
  growth beyond the configured retention budget as investigation thresholds,
  not automatic tuning triggers. Re-run this harness on the affected storage
  class before changing defaults.

## Conclusions

The initial journal meets its current local operational targets for the two
representative workloads: append latency remains far below the 2-second write
timeout, restart and replay remain interactive, and whole-segment retention
reclaims disk without rewriting records. No measured target was violated, so
no storage-engine migration or provider abstraction is justified by this
evidence.

One operational risk remains: these measurements cover a single Darwin arm64
host and local filesystem. Cross-platform and slower-storage regression
tracking is a separate follow-up so it does not expand the initial evaluation.

## Cross-platform regression workflow

`.github/workflows/journal-operational-limits.yml` runs weekly and on manual
dispatch. It executes the same release-mode harness on Linux, macOS, and
Windows, plus a Linux `fsync-pressure` profile that places a bounded synchronous
writer beside the journal workload. The pressure profile is a repeatable
constrained-storage signal, not a claim about a named production disk class.

Each matrix entry uploads a 90-day JSON artifact with the same workload,
latency, recovery, replay, pruning, and disk-growth fields. The environment
block records OS, architecture, and storage profile so results remain
comparable. The workflow summary marks the investigation thresholds above but
does not convert benchmark variance into a pull-request or unit-test gate.

To collect a targeted run, dispatch **Journal operational limits** from GitHub
Actions. Download all four artifacts and compare like-for-like profiles. Open a
focused remediation issue when a threshold is exceeded on two consecutive
runs or the same regression reproduces locally; attach both JSON artifacts and
the runner image identifiers.
