# ADR-0014: Select wasmi 1.1.0 for the Apple Runtime Profile

- Status: Accepted
- Date: 2026-07-20
- Governing specs: `071-native-runtime-wasm-bridge`,
  `073-native-embedder-release-baseline`,
  `074-swift-native-resource-control-certification`, and
  `075-native-runtime-distribution-contract`
- Owner: Traverse maintainers

## Context

ADR-0011 rejected the existing WasmKit profile because its memory limiter is
SPI and it exposes no supported deterministic interruption control. Spec 074
requires an approved successor ADR, dependency/security review, Apple
distribution evidence, and the complete conformance corpus before #647 may
replace that engine.

Traverse #769 (PR #772) proved a narrow Rust static-library host using
`wasmi` 1.1.0 on macOS and a physical iOS device. Its fixtures stop memory
growth at the configured limit and terminate a non-terminating module when
fuel is exhausted; the physical-device app remained responsive after both
fixtures. The resolved crate metadata records `wasmi` 1.1.0 as
`MIT/Apache-2.0`, pinned in `Cargo.lock` with its crates.io checksum.

## Decision

Select `wasmi` 1.1.0 as the engine for the Swift Apple runtime profile. The
profile is a Rust `staticlib`, packaged as `TraverseSwiftHost.xcframework` for
the reviewed `aarch64-apple-ios`, `aarch64-apple-ios-sim`, and
`aarch64-apple-darwin` targets.

The production bridge implementation must use only the documented `wasmi`
APIs demonstrated by #769:

- `StoreLimitsBuilder::memory_size` with `trap_on_grow_failure` for bounded
  memory growth;
- `Config::consume_fuel` and `Store::set_fuel` for deterministic instruction
  budgeting; and
- normal trap handling for `GrowthOperationLimited` and `OutOfFuel`.

The profile must retain the core-Wasm no-ambient-import rule, bounded artifact
and event sizes, and the reviewed C-ABI boundary from ADR-0013. It must not
use WasmKit SPI, a watchdog that leaves guest execution alive, raw-pointer
access beyond the audited export boundary, ambient host services, or an
unreviewed engine upgrade.

`wasmi` 1.1.0 is selected for implementation; this ADR does **not** certify a
Native Embedder Baseline release. #647 must still implement the production
bridge, validate the digest-pinned `runtime.wasm` artifact and bridge 1.1 ABI,
and pass the bridge and embedder conformance corpora. #758 must record the
same runtime digest, engine version, limits, host matrix, and conformance
results in release evidence before a Swift certification claim.

## Dependency and Distribution Review

- The engine is pinned as `wasmi` 1.1.0 in `Cargo.lock`; its resolved license
  is `MIT/Apache-2.0` and its repository is `wasmi-labs/wasmi`.
- The host remains a small Rust static library; no new network, filesystem,
  clock, environment, or process authority is introduced by this selection.
- Runtime bytes and their bridge version remain identified by
  `runtime/runtime-release.json` and distributed through the registry model in
  ADR-0012 / Spec 075. Swift packages resolve and embed those bytes at release
  time, never by a runtime sidecar or network fetch.
- Any critical unfixed `wasmi` advisory, license change, or change to public
  resource-control semantics blocks release pending renewed review.

## Consequences

- #647 is unblocked to implement the bounded Apple bridge against this
  selected profile.
- #758 remains blocked until #647 and the reference-app integrations produce
  real-artifact cross-host evidence.
- #767 remains an independent tracker for a future qualifying WasmKit release;
  it is not a prerequisite for the selected `wasmi` profile.

## Alternatives Considered

- Keep WasmKit 0.3.1: rejected because its relevant controls remain SPI or
  absent, violating Spec 074.
- Use WAMR 2.4.5: rejected by the #762 screen because it lacks supported iOS
  evidence.
- Use a watchdog or a platform-local runtime: rejected because neither
  provides the supported deterministic interruption and portable,
  runtime-owned semantics required by Specs 068 and 071.
