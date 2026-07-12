//! Private (access-controlled) trace entry.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt::Write as _;

/// An access-controlled private trace entry that links to a [`super::public::PublicTraceEntry`]
/// via `trace_id`.
///
/// Raw inputs and outputs are never stored; only SHA-256 hashes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrivateTraceEntry {
    /// Links to the `id` field of the corresponding [`super::public::PublicTraceEntry`].
    pub trace_id: String,
    /// SHA-256 hex digest of the serialized inputs.
    pub inputs_hash: String,
    /// SHA-256 hex digest of the serialized outputs.
    pub outputs_hash: String,
    /// Measured resource usage in milliseconds.
    pub resource_usage_ms: u64,
}

impl PrivateTraceEntry {
    /// Creates a new [`PrivateTraceEntry`], hashing `inputs` and `outputs` with SHA-256.
    #[must_use]
    pub fn new(trace_id: String, inputs: &str, outputs: &str, resource_usage_ms: u64) -> Self {
        Self {
            trace_id,
            inputs_hash: sha256_hex(inputs),
            outputs_hash: sha256_hex(outputs),
            resource_usage_ms,
        }
    }
}

/// Returns the lowercase hex-encoded SHA-256 digest of `data`.
fn sha256_hex(data: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data.as_bytes());
    hasher
        .finalize()
        .iter()
        .fold(String::new(), |mut acc, byte| {
            let _ = write!(acc, "{byte:02x}");
            acc
        })
}
