# ADR-0070: Select wasmi 2.0.0 for the Apple Runtime Profile

- Status: Accepted
- Date: 2026-09-13
- Governing specs: `074-swift-native-resource-control-certification`,
  `076-production-swift-wasmi-cabi`, `136-native-embedder-publication`
- Supersedes: ADR-0014
- Related issues: #1368, #1352, #1370, #884
- Related: `docs/decision-log.md` Decision 83
- Owner: Traverse maintainers

## Context

ADR-0014 selected wasmi 1.1.0 after `#769` proved bounded memory and fuel
interruption on macOS and a physical iOS device. It forbids an unreviewed
engine upgrade and requires renewed review if public resource-control semantics
change.

`#1352` (2026-09-13) moved `crates/traverse-swift-host` to wasmi 2.0.0. The
shipped `Package.swift` binary target still downloads the v0.8.2 XCFramework
built for 1.1.0. Source and the released Apple binary do not match. ADR-0014
cannot authorize a 2.0.0 zip.

The host still uses the documented public controls required by 0014:
`StoreLimitsBuilder` (memory growth), `Config::consume_fuel`, and
`Store::set_fuel`. ADR-0015's five-symbol C ABI is unchanged.

## Decision

Select **wasmi 2.0.0** as the engine for the Swift Apple runtime profile. The
profile remains a Rust `staticlib` packaged as `TraverseSwiftHost.xcframework`
for `aarch64-apple-ios`, `aarch64-apple-ios-sim`, and `aarch64-apple-darwin`.

Production code MUST continue to use only documented wasmi APIs for resource
control:

- `StoreLimitsBuilder::memory_size` with `trap_on_grow_failure`;
- `Config::consume_fuel` and `Store::set_fuel`;
- normal trap handling for growth and out-of-fuel failures.

The profile MUST retain the core-Wasm no-ambient-import rule, bounded artifact
and event sizes, and the ADR-0015 C-ABI. It MUST NOT use WasmKit SPI, a
watchdog that leaves guest execution alive, raw-pointer access beyond the
audited export boundary, ambient host services, or a further unreviewed engine
upgrade.

**No 2.0.0 XCFramework may be attached to a GitHub release until new physical
iOS and macOS Spec 074 fixtures** (memory-growth and non-termination) are
recorded against wasmi 2.0.0. CI `macos-latest` and the 1.1.0 `#769` device
record are not sufficient. Simulator-only iOS evidence is not sufficient.

Release evidence for a 2.0.0 zip records wasmi 2.0.0, `MIT/Apache-2.0`, the
`Cargo.lock` checksum, configured limits, the arm64 Apple profile, the runtime
digest / bridge version from Spec 075, and those new device/macOS results.

Spec 136 then requires that zip to share the `v*` tag with Maven Central and
nuget.org.

## Dependency and distribution review

- Pin: `wasmi` 2.0.0 in `crates/traverse-swift-host/Cargo.toml`, checksum in
  `Cargo.lock`. License remains `MIT/Apache-2.0` (`wasmi-labs/wasmi`).
- No new filesystem, network, clock, environment, or process authority.
- `#884` (WasmKit public resource-control watch) is superseded as the
  production Apple path; close it when this ADR merges.

## Consequences

- ADR-0014 is Superseded. Its 1.1.0 pin and the v0.8.2 zip remain historical
  for already-shipped binaries.
- `#1370` may rebuild the XCFramework only after Enrico records 2.0.0 device
  and macOS fixtures.
- `#1371` / `#1372` publish on the same `v*` as that zip (Decision 83 / Spec
  136 FR-005).

## Alternatives considered

- Keep 1.1.0 as the Apple release pin and revert `#1352`: rejected; `main`
  would stay forked from the shipped binary.
- Treat `#1352` as the review and skip a successor ADR: rejected; 0014
  forbids an unreviewed upgrade and approved ADRs are not silently patched.
- Waive physical iOS evidence for 2.0.0: rejected; Spec 074 named
  “macOS-only / no physical iOS evidence” as a failure mode.
