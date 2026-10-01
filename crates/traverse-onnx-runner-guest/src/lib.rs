//! Generic Spec 138 ONNX runner guest (Decision 105/106, `#1591`).
//!
//! One audited runner serves every ONNX model: `traverse-cli model
//! package-onnx` appends a *model blob* (tensor config + ONNX bytes) to a copy
//! of this module and points [`abi::TRAVERSE_MODEL_BLOB`] at it. The runner
//! parses the blob on first use, builds a tract plan, and runs one input
//! tensor → one output tensor (raw values, no post-processing).
//!
//! The only `unsafe` code is the audited guest-ABI v2 boundary in `abi`
//! (ADR-0077 pattern; `scripts/ci/scoped_unsafe_boundary_check.sh`).

pub mod runner;

#[cfg(target_arch = "wasm32")]
#[allow(unsafe_code)]
mod abi;

#[cfg(target_arch = "wasm32")]
fn no_entropy(_: &mut [u8]) -> Result<(), getrandom::Error> {
    // Deterministic and import-free: randomness requests fail instead of
    // importing a host entropy source (BirdNET-style models never ask).
    Err(getrandom::Error::UNSUPPORTED)
}

#[cfg(target_arch = "wasm32")]
getrandom::register_custom_getrandom!(no_entropy);
