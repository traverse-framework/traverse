//! `no_std` Spec 138 exact-ref guest for the trained digits MLP
//! (Decision 102, `#1461`).
//!
//! The safe forward pass lives in [`mlp`]. The only `unsafe` code is the
//! audited guest-ABI boundary in `abi` (ADR-0077): it turns the host-staged
//! `(in_ptr, in_len)` / `(out_ptr, out_cap)` linear-memory offsets into byte
//! slices. `scripts/ci/scoped_unsafe_boundary_check.sh` enforces that scope.

// `no_std` for the shipped wasm32 guest; host builds (tests) keep `std`.
#![cfg_attr(target_arch = "wasm32", no_std)]

pub mod mlp;

#[cfg(target_arch = "wasm32")]
#[allow(unsafe_code)]
mod abi;

#[cfg(target_arch = "wasm32")]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    core::arch::wasm32::unreachable()
}
