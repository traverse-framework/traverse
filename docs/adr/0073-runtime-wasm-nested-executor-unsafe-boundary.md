# ADR-0073: Audited `unsafe_code` Exception for the `runtime.wasm` Nested Executor

- Status: Accepted
- Date: 2026-09-15
- Governing specs: `1402-runtime-wasm-orchestrator-convergence` (v1.1.0,
  FR-005, FR-011), `076-production-swift-wasmi-cabi` (precedent)
- Related issues: `#1407`, `#1402`
- Related: `docs/decision-log.md` Decision 87, Decision 88
- Owner: Traverse maintainers

## Context

Decision 87 (spike `#1403`) proved nested `wasmi` can host a capability WASM
module from inside a `wasm32`-compiled guest, and explicitly deferred one
piece: "browser-instantiate C-ABI packaging of the outer module until an
audited `unsafe_code` export boundary is approved." Implementing `#1407`
(Phase 2 of `1402`) now requires that boundary for real: `runtime.wasm` must
export the unchanged `runtime-wasm-bridge/1.0.0` ABI (`071` FR-006) —
`traverse_init`, `traverse_submit`, `traverse_next_event`, `traverse_cancel`,
`traverse_shutdown`, `traverse_alloc`/`traverse_dealloc` — as `extern "C"`
functions receiving raw `(ptr, len)` pairs into the guest's own linear
memory. Converting those into Rust slices (`slice::from_raw_parts`) has no
safe-Rust equivalent; this is inherent to any Rust C-ABI guest boundary, not
specific to this codebase.

The workspace denies `unsafe_code` globally (`Cargo.toml` `[workspace.lints]`
`unsafe_code = "deny"`), with exactly one existing named exception:
`crates/traverse-swift-host` (`#![allow(unsafe_code)] // Audited C-ABI
exception; see ADR-0015 and Spec 076`) — the native Swift-side boundary that
embeds `wasmi` as a *host* engine. `crates/traverse-nested-wasm-spike`
(`#1403`), by contrast, was deliberately kept `#![deny(unsafe_code)]`: it
proved the interpreter-hosts-interpreter pattern with a hand-linked WASI
stub, not a production export boundary, and its own header comment says so.

## Decision

1. **A new, dedicated crate** — `crates/traverse-runtime-wasm` — houses the
   nested `wasmi` executor and the `071` FR-006 C-ABI export boundary. It is
   not added to `traverse-native-bridge` (the *native-side* artifact
   builder/packager, a different concern) or `traverse-nested-wasm-spike`
   (kept as-is, historical spike record, still `#![deny(unsafe_code)]`).
2. **This new crate may declare `#![allow(unsafe_code)]`**, scoped to the
   minimum surface the ptr/len-to-slice ABI conversions at the export
   boundary require — mirroring the `traverse-swift-host` precedent exactly:
   one narrow, named, ADR-backed exception per crate, not a workspace-wide
   relaxation.
3. **The `emit_event` validation logic itself stays outside this exception.**
   Per spec `1402` FR-005/FR-011, the shared validation core (payload-bound
   check, declared-`emits` check, error-code mapping) lives in
   `crates/traverse-contracts` as ordinary safe `std` Rust operating on
   `&[u8]` and `&[EventReference]` — it never touches raw pointers itself,
   so it carries no part of this exception. Both `crates/traverse-runtime`
   (native, Wasmtime) and `crates/traverse-runtime-wasm` (nested, wasmi) call
   into it after their own engine-specific code has already produced a safe
   slice.
4. **CI enforcement**: `scripts/ci/scoped_unsafe_boundary_check.sh` already
   existed to audit the `traverse-swift-host` exception (grep-verifying it is
   the only crate-level `#![allow(unsafe_code)]` opt-out, and enumerating its
   exact exported C-ABI symbol set). It is extended, not replaced: it now
   verifies the opt-out set is exactly `{traverse-swift-host,
   traverse-runtime-wasm}` and enumerates `traverse-runtime-wasm`'s own
   exact exported symbol set the same way. This script runs as part of
   `scripts/ci/rust_checks.sh` (the standard local/CI gate).

## Consequences

- The exception surface grows from one crate to two, each independently
  narrow and independently audited — not a general loosening of the
  workspace's `unsafe_code` policy.
- `traverse-runtime-wasm` becomes the second crate every future contributor
  and reviewer must treat as security-sensitive (raw-pointer FFI boundary),
  matching how `traverse-swift-host` is already treated.
- `traverse-nested-wasm-spike` is unaffected and stays as a historical,
  intentionally minimal proof — it is not repurposed into production code.

## Alternatives Considered

- **Promote `traverse-nested-wasm-spike` itself to production**, adding the
  export boundary and lifting its `#![deny(unsafe_code)]` in place. Rejected:
  the spike was reviewed and merged under an explicit "spike-only, not
  production-ready" scope; repurposing it blurs that line and its existing
  tests/benchmarks were written for spike purposes, not ABI conformance.
- **Extend `traverse-swift-host`'s existing exception** to cover this work.
  Rejected: unrelated crate and unrelated boundary (native Swift-side host
  embedding vs. the `wasm32` guest's own exports) — bundling them would make
  a future audit of either harder to scope.
- **Avoid `unsafe` entirely** via a hypothetical safe wrapper crate. Rejected
  after research: raw ptr/len-to-slice conversion at a Rust C-ABI guest
  boundary has no safe-Rust equivalent; any such wrapper would itself use
  `unsafe` internally, just relocating the exception rather than removing it.

## Approval

Accepted as the recorded outcome of the `/brainstorm` session in
`docs/decision-log.md` Decision 88 (2026-09-15).
