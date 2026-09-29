# ADR-0077: Audited `unsafe_code` Boundary for Trained Spec 138 Model Guests

- Status: Accepted
- Date: 2026-09-29
- Governing specs: `138-governed-exact-model-execution` (v0.5.0)
- Related issues: `#1461`, `#1565`
- Related: `docs/decision-log.md` Decision 102; ADR-0073 (precedent)
- Owner: Traverse maintainers

## Context

Decision 102 chose a hand-rolled `#![no_std]` Rust guest for the first
trained exact-ref model (`crates/traverse-digits-mlp-guest`), so the
forward-pass math is readable, reviewable, and shares its layout with the
Rust trainer. The Spec 138 guest ABI is
`model_execute(in_ptr, in_len, out_ptr, out_cap) -> out_len`: the host stages
the input frame into the guest's own linear memory and passes raw offsets.
Turning `(ptr, len)` pairs into byte slices (`core::slice::from_raw_parts`)
has no safe-Rust equivalent. This is the same shape ADR-0073 audited for
`runtime.wasm`, and is inherent to any Rust guest on a pointer-passing ABI.

The workspace denies `unsafe_code` globally, and
`scripts/ci/scoped_unsafe_boundary_check.sh` only permits audited
boundaries. Placing the guest outside `crates/` would silently evade that
check, so it is not an option.

## Decision

1. The guest crate lives at `crates/traverse-digits-mlp-guest` (inside the
   checked tree) but outside the host Cargo workspace, via its own empty
   `[workspace]` table, because it ships only as a `wasm32` `no_std` cdylib.
   It mirrors the workspace lints, including `unsafe_code = "deny"`.
2. `unsafe` is scoped to one module, `src/abi.rs`, declared as
   `#[allow(unsafe_code)] mod abi;` in `src/lib.rs`, exactly like the
   expedition guest's `wasi_stdio` boundary. It exports exactly one
   `#[unsafe(no_mangle)]` symbol, `model_execute`, and contains exactly two
   `unsafe` blocks: an immutable view of the input region and a mutable
   view of the output region. It imports nothing.
3. Before any `unsafe`, the boundary rejects negative offsets, null
   pointers, `out_cap` below the fixed output frame length, and overlapping
   input/output regions. Out-of-bounds offsets trap in the wasm engine
   instead of reading foreign memory.
4. All model logic (frame decoding and validation, the forward pass,
   argmax, frame encoding) lives in `src/mlp.rs` under
   `#![forbid(unsafe_code)]`.
5. `scripts/ci/scoped_unsafe_boundary_check.sh` enforces 2–4 (the scoped
   allow, exactly one `abi` module, one export, two `unsafe` blocks, and no
   `extern "C"` import blocks), and `scripts/ci/digits_mlp_guest_check.sh`
   proves the checked-in `model.wasm` rebuilds byte-identically with zero
   imports.

This is a narrowly scoped exception for Spec 138 model guests, not a general
loosening. A future model guest reuses this pattern by adding its own
boundary to the check, with the same constraints.

## Consequences

- There are four audited `unsafe` boundaries: the Swift host, `runtime.wasm`,
  the expedition WASI shim, and trained model guests.
- The guest stays `no_std` on wasm32, but host builds keep `std`, so the
  forward pass is unit-tested natively (bit-identical to the trainer on the
  full held-out split).
- Reviewers audit about 40 lines of ABI code; the math carries no `unsafe`.

## Alternatives Considered

- **Hand-written WAT guest** (no `unsafe`, matching the earlier fixtures):
  rejected, since it reverses Decision 102's rejection of generated or
  hand-rolled WAT for matrix code that is hard to review.
- **Change the Spec 138 ABI** so guests export their own buffers and safe
  Rust can use statics: rejected as a breaking guest-ABI change to all
  fixtures and both hosts, far outside `#1461`.
- **Guest outside `crates/`**: rejected because it would evade the
  unsafe-boundary check.

## Approval

Approved by Enrico during `#1461` implementation (2026-09-29): "Rust guest +
audited unsafe ADR".
