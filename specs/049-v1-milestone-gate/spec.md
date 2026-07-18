# Feature Specification: v1 Milestone Gate

**Feature Branch**: `049-v1-milestone-gate`
**Created**: 2026-07-03
**Status**: Approved
**Input**: No formal v1 definition exists. The project is at v0.7.0 with a growing spec surface but no stated stability promise, no crates.io presence, and no documented exit criteria for the 0.x phase. This spec names the gate conditions that must all be true before `v1.0.0` is tagged.

## Purpose

This spec defines exactly what v1.0.0 means for Traverse. It is not a feature spec — it is a gate spec. It names the conditions and creates the CI/checklist mechanism that enforces them. No condition may be waived; all must be green before the v1.0.0 tag is created.

v1.0.0 signals:
- Public API surfaces are stable — downstream consumers can depend on them without expecting breaking changes between patch releases.
- All six crates are published on crates.io at the same version.
- Thread isolation works correctly on all five supported platforms.
- The security and supply-chain surface is hardened.
- Documentation is accurate, complete, and covers the developer entry path end to end.

## Gate Conditions

### G-01: Semver publishing pipeline live
All six crates are published on crates.io at `v1.0.0`. CI `publish` job runs on tag push with no manual steps. `Cargo.toml` version matches the tag. Governed by `spec 048`.

### G-02: Thread pool executor stable and stress-tested
`ThreadPoolExecutor` implementation is merged, 100% unit and integration coverage, all 5-platform stress CI matrix green. Governed by `spec 047`.

### G-03: Public API compatibility promise documented
`docs/compatibility-policy.md` states clearly: what is public API, what semver guarantees apply at v1, what is explicitly internal and may change. No v1 tag until this document exists and is reviewed.

### G-04: Cross-platform CI matrix green on all five targets
`ci.yml` runs the full test suite on Linux x86_64, Linux aarch64, macOS x86_64, macOS arm64, Windows x86_64. All five legs are required status checks. Governed by `spec 047` stress test job.

### G-05: Security and supply-chain gate passing
`supply-chain.yml` runs clean: SBOM generated, no critical advisories, dependency audit passes. Governed by `spec 031`.

### G-06: Developer entry path validated end-to-end
The quickstart in `README.md` (`cargo run -p traverse-cli-rs -- bundle inspect ...`) runs to documented expected output on a clean clone on Linux and macOS in CI. A downstream app can register, execute a workflow, and receive a result without private Traverse internals. Governed by `spec 046` conformance suite.

### G-07: MCP library surface stable
`traverse-mcp` library surface (`spec 042`) is complete: `discover_capabilities()` and `execute_capability()` are callable from a Rust binary without spawning a subprocess. Thread safety verified.

### G-08: All approved specs have 100% test coverage
Every spec in `approved-specs.json` that governs `crates/` has corresponding tests with 100% line coverage on the governed files. CI coverage gate enforces this.

### G-09: No open P0 or P1 bugs in the GitHub Project
Project 1 has zero open issues labelled `bug` at priority P0 or P1 at the time of tagging.

### G-10: `traverse-framework/Traverse` is the canonical repo
The repo lives at `traverse-framework/Traverse`. Old redirect from `enricopiovesan/Traverse` is in place. All CI badges, docs, and `Cargo.toml` metadata point to the new org. Already complete — included as a gate to verify no regressions.

## v1 Release Checklist Script

`scripts/ci/v1_gate_check.sh` — runs all verifiable gate conditions locally and in CI:

```
[G-01] cargo search traverse-runtime | grep "^traverse-runtime = \"1.0.0\""
[G-02] cargo test -p traverse-runtime --test thread_pool_stress -- --ignored
[G-03] test -f docs/compatibility-policy.md && grep -q "v1 stability" docs/compatibility-policy.md
[G-04] CI matrix: all 5 legs green (checked via gh run list)
[G-05] cargo audit — exits 0
[G-06] cargo run -p traverse-cli-rs -- bundle inspect examples/expedition/... (output matches)
[G-07] cargo test -p traverse-mcp (all tests pass)
[G-08] cargo llvm-cov — coverage >= 100% for all governed crate paths
[G-09] gh issue list --label bug --state open — count = 0
[G-10] grep "traverse-framework/Traverse" Cargo.toml
```

## What v1 Does NOT Require

- A reference app in this repo (reference apps live in `traverse-framework/App-References`)
- An HTTP admin API
- A service registry or service discovery mechanism
- A cloud deployment surface
- A stable WASM ABI beyond what `spec 038` already governs

## Files Governed

- `scripts/ci/v1_gate_check.sh` (new)
- `docs/v1-milestone.md` (new — human-readable version of this gate list)
- `docs/compatibility-policy.md` (must be updated to include v1 stability statement)
