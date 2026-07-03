# v1.0.0 Milestone

Governed by `spec 049-v1-milestone-gate`.

v1.0.0 signals that Traverse public API surfaces are stable, all six crates are on crates.io, and the runtime has been stress-tested on all supported platforms. No condition may be waived before the `v1.0.0` tag is created.

## Gate Conditions

| Gate | Condition | Verifiable |
|---|---|---|
| G-01 | All 6 crates published on crates.io at `v1.0.0` | `cargo search` for each Traverse crate |
| G-02 | ThreadPoolExecutor stress tests green on all 5 platforms | `cargo test -p traverse-runtime --test thread_pool_stress -- --ignored` and CI stress-test matrix |
| G-03 | `docs/compatibility-policy.md` contains v1 stability statement | `grep "v1 stability"` |
| G-04 | 5-platform CI matrix all green (Linux x86_64/aarch64, macOS x86_64/arm64, Windows x86_64) | latest `ci.yml` GitHub Actions run status |
| G-05 | `cargo audit` clean, SBOM generated | `supply-chain.yml` passing |
| G-06 | Quickstart command produces documented output on clean clone on Linux + macOS in CI | Smoke test |
| G-07 | MCP library surface callable without subprocess (`discover_capabilities`, `execute_capability`) | Integration tests |
| G-08 | 100% test coverage on all governed crate paths | Coverage gate |
| G-09 | Zero open P0/P1 bugs in Project 1 | `gh issue list --label bug` filtered to P0/P1 priority labels |
| G-10 | `Cargo.toml` and all doc badges point to `traverse-framework/Traverse` | metadata and documentation grep |

## How to check locally

```bash
bash scripts/ci/v1_gate_check.sh
```

Exits 0 only when all verifiable conditions pass. Names which gate failed otherwise.

## What v1.0.0 does NOT require

- A reference app in this repo (reference apps live in `traverse-framework/App-References`)
- An HTTP admin API or service registry
- A cloud deployment surface
- A stable WASM ABI beyond what spec 038 already governs
- Message-passing worker isolation (spec 050, planned for v2)
