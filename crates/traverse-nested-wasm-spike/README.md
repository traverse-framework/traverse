# Nested wasmi feasibility spike (#1403)

Spec `1402-runtime-wasm-orchestrator-convergence` Phase 1.

## What this proves

1. **wasmi 2.0.0** with `portable-dispatch` + `prefer-btree-collections`
   (no `std`) hosts a Traverse-shaped WASI command capability
   (stdin JSON → stdout JSON) via a hand-linked preview1 surface.
2. The same dependency feature set **typechecks/compiles for
   `wasm32-unknown-unknown`** (`cargo check --target wasm32-unknown-unknown`).
3. Rough nested-interpreter cost for echo / example artifacts (`spike_bench`).

## Limits (explicit)

Workspace `unsafe_code = deny` (only `traverse-swift-host` may opt out)
blocks a C-ABI export that would let a browser `WebAssembly.instantiate` the
outer module and pass capability bytes through linear memory. That export
boundary is a follow-up after accepting nested execution — not a wasmi
feasibility blocker.

## Commands

```bash
cargo test -p traverse-nested-wasm-spike
cargo check -p traverse-nested-wasm-spike --target wasm32-unknown-unknown
cargo run -p traverse-nested-wasm-spike --bin spike_bench --release -- 50
cargo run -p traverse-nested-wasm-spike --bin spike_bench --release -- 20 \
  examples/hello-world/say-hello-agent/artifacts/say-hello-agent.wasm
```

## Measured locally (2026-09-14, release `spike_bench`)

| Workload | per-call |
| --- | --- |
| Echo WAT (214 B) | ~34 µs |
| `say-hello-agent.wasm` (612 B) | ~48 µs |
