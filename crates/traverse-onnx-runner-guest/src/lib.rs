//! Generic Spec 138 ONNX runner guest (Decision 105/106, `#1591`; multi-IO `#1661`).
//!
//! One audited runner serves every ONNX model: `traverse-cli model
//! package-onnx` appends a *model blob* (tensor config + ONNX bytes) to a copy
//! of this module and points [`abi::TRAVERSE_MODEL_BLOB`] at it. The runner
//! parses the blob on first use, builds a tract plan, and runs N named input
//! frames → M named output frames (concatenated Spec 138 frame bundles; raw
//! values, no post-processing). Legacy single-IO tensor JSON still works.
//! Element types include `f32` and `i64` (token ids / masks).
//!
//! The only `unsafe` code is the audited guest-ABI v2 boundary in `abi`
//! (ADR-0077 pattern; `scripts/ci/scoped_unsafe_boundary_check.sh`).

pub mod runner;

#[cfg(target_arch = "wasm32")]
#[allow(unsafe_code)]
mod abi;
