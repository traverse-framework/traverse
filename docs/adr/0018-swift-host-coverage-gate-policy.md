# ADR-0018: Protect the Production Swift Host with a Measured Coverage Ratchet

- Status: Accepted
- Date: 2026-07-22
- Governing specs: `001-foundation-v0-1`, `076-production-swift-wasmi-cabi`
- Owner: Traverse maintainers
- Review by: 2026-10-22

## Context

`traverse-swift-host` is the audited Rust C-ABI boundary used by the Swift
embedder. It contains the only approved unsafe boundary for this host profile,
but was absent from `ci/coverage-targets.txt`; consequently CI did not measure
it at all. The local baseline measured on 2026-07-22 is 78.66% line coverage
(365 of 464 lines) with its three focused boundary tests passing.

The workspace's standing quality target for core runtime logic is 100% line
coverage. Enforcing that target immediately would make the gate red before the
missing tests can be supplied; leaving the crate ungated would retain the
unmeasured FFI risk.

## Decision

1. Add `traverse-swift-host` to `ci/coverage-targets.txt` with a 78% line
   coverage ratchet floor. The gate runs `cargo llvm-cov --package
   traverse-swift-host`; a lower result fails CI.
2. Retain 100% as the required destination. Before the next production release
   that includes a change to this crate, a dedicated follow-up must raise the
   floor to 100% and add coverage for every reviewed status, lifetime, limit,
   and error conversion path.
3. Do not add `traverse-native-bridge` (build-time fixture generator) or
   `traverse-expedition-wasm` (demo guest) to the protected list. If either
   obtains a production runtime role, a new policy decision must set its floor
   before that role ships.

## Evidence and Failure Behavior

The `coverage-gate` CI job is the enforcement point. It must fail a pull
request whenever the measured `traverse-swift-host` line percentage falls below
78%; the existing target-list parser and error output provide the auditable
crate, measured percentage, and configured floor. This ADR does not relax
existing floors or exempt unsafe code from ordinary test, lint, or boundary
certification requirements.

## Consequences

The audited boundary becomes immediately regression-protected without claiming
that its present test suite is complete. The explicit release prerequisite
prevents the temporary measured floor from becoming an undocumented permanent
exception.
