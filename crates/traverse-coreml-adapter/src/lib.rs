//! Spec 138 Core ML accelerator adapter (Decisions 112 and 113, ADR-0082).
//!
//! [`CoreMlAdapter`] runs a package's signed `mlpackage-zip` variant through
//! Core ML on Apple platforms, behind the runtime's
//! [`AcceleratorAdapter`](traverse_runtime::exact_model::AcceleratorAdapter)
//! trait. The connector keeps every Spec 138 rule (placement order,
//! self-check, tolerance, ceilings, cancellation, evidence); this crate only
//! runs predictions. Its `unsafe` is confined to the audited `coreml`
//! module. Off Apple platforms only the safe helpers build.

#[cfg(target_vendor = "apple")]
#[allow(unsafe_code)]
mod coreml;
// Off Apple platforms only the zip and compute-unit helpers are exported;
// the tensor helpers serve the Core ML module alone.
#[cfg_attr(not(target_vendor = "apple"), allow(dead_code))]
mod package;

#[cfg(target_vendor = "apple")]
pub use coreml::CoreMlAdapter;
pub use package::{
    ComputeUnits, check_mlpackage_manifest, mlpackage_entries, unpack_mlpackage_zip,
};

#[cfg(target_vendor = "apple")]
use traverse_runtime::exact_model::AcceleratorAdapter;

/// Builds the Core ML adapter from a host's `accelerators` configuration
/// (the `create` header of the framed model protocol, Decision 112 rule 6):
///
/// ```json
/// { "coreml": { "compute_units": "all", "cache_dir": "/path/to/Caches/traverse-coreml" } }
/// ```
///
/// No `coreml` entry means no adapter, so every package runs on `wasm-cpu`.
/// `compute_units` defaults to `all`; `cache_dir` defaults to
/// `<temp>/traverse-coreml`.
///
/// # Errors
///
/// Returns the name of a field that is present but invalid.
#[cfg(target_vendor = "apple")]
pub fn coreml_adapters(
    config: &serde_json::Value,
) -> Result<Vec<Box<dyn AcceleratorAdapter>>, &'static str> {
    let Some(coreml) = config.get("coreml") else {
        return Ok(Vec::new());
    };
    if !coreml.is_object() {
        return Err("accelerators.coreml");
    }
    let compute_units = match coreml.get("compute_units") {
        None => ComputeUnits::default(),
        Some(value) => value
            .as_str()
            .and_then(ComputeUnits::parse)
            .ok_or("accelerators.coreml.compute_units")?,
    };
    let cache_dir = match coreml.get("cache_dir") {
        None => std::env::temp_dir().join("traverse-coreml"),
        Some(value) => value
            .as_str()
            .filter(|path| !path.is_empty())
            .map(std::path::PathBuf::from)
            .ok_or("accelerators.coreml.cache_dir")?,
    };
    Ok(vec![Box::new(CoreMlAdapter::new(compute_units, cache_dir))])
}
