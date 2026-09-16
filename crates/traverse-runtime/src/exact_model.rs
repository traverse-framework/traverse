//! Spec `138-governed-exact-model-execution`: host-staged I/O, package store,
//! and CPU-WASM model guest execution behind Spec 137 `model.execute`.

use crate::host_connector_dispatch::{
    HostConnectorError, HostConnectorErrorCode, HostConnectorHostRequest, HostConnectorHostResult,
    HostConnectorPort, MODEL_EXECUTE_OPERATION, MODEL_RUNTIME_CONNECTOR,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Governing spec id.
pub const GOVERNING_SPEC: &str = "138-governed-exact-model-execution";
/// First guest ABI version.
pub const MODEL_GUEST_ABI_VERSION: u16 = 1;
/// Placement for the CPU-WASM conformance baseline.
pub const PLACEMENT_WASM_CPU: &str = "wasm-cpu";
/// Guest export name.
pub const MODEL_EXECUTE_EXPORT: &str = "model_execute";

/// Exact app-manifest pin (Spec 044 `exact_model_dependencies`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExactModelPin {
    /// Model identity.
    pub model_id: String,
    /// Semantic version.
    pub version: String,
    /// Package/pair digest (hex sha256, optionally `sha256:` prefixed).
    pub digest: String,
    /// Whether offline execute is allowed when the package is cached.
    pub offline_allowed: bool,
}

/// Versioned model package manifest (sidecar).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelPackageManifest {
    /// Schema version for this manifest document.
    pub schema_version: String,
    /// Model identity.
    pub model_id: String,
    /// Semantic version.
    pub version: String,
    /// Digest of the WASM bytes (hex).
    pub wasm_digest: String,
    /// Package/pair digest (hex).
    pub package_digest: String,
    /// Registry reference string.
    pub registry_ref: String,
    /// Executable format (`traverse-model-wasm`).
    pub executable_format: String,
    /// Guest ABI version.
    pub abi_version: u16,
    /// Input schema ref.
    pub input_schema_ref: String,
    /// Input schema version.
    pub input_schema_version: String,
    /// Output schema ref.
    pub output_schema_ref: String,
    /// Output schema version.
    pub output_schema_version: String,
    /// SPDX or equivalent license id.
    pub license_id: String,
    /// Attribution text.
    pub attribution: String,
    /// Redistribution terms summary.
    pub redistribution: String,
    /// Supported placement profiles.
    pub supported_profiles: Vec<String>,
    /// Max linear memory bytes.
    pub max_memory_bytes: u64,
    /// Max fuel.
    pub max_fuel: u64,
    /// Max input bytes.
    pub max_input_bytes: u64,
    /// Max output bytes.
    pub max_output_bytes: u64,
    /// Max execution time milliseconds.
    pub max_execution_ms: u64,
    /// Offline allowed after provisioning.
    pub offline_allowed: bool,
}

impl ModelPackageManifest {
    /// Fail closed if required governance fields are missing or empty.
    ///
    /// # Errors
    ///
    /// Returns `model_incompatible` when required fields or limits are invalid.
    pub fn validate(&self) -> Result<(), HostConnectorError> {
        let required = [
            ("license_id", self.license_id.as_str()),
            ("wasm_digest", self.wasm_digest.as_str()),
            ("package_digest", self.package_digest.as_str()),
            ("executable_format", self.executable_format.as_str()),
            ("input_schema_ref", self.input_schema_ref.as_str()),
            ("output_schema_ref", self.output_schema_ref.as_str()),
        ];
        for (name, value) in required {
            if value.trim().is_empty() {
                return Err(HostConnectorError {
                    code: HostConnectorErrorCode::ModelIncompatible,
                    message: format!("model manifest missing required field {name}"),
                });
            }
        }
        if self.abi_version == 0
            || self.max_memory_bytes == 0
            || self.max_fuel == 0
            || self.max_input_bytes == 0
            || self.max_output_bytes == 0
            || self.max_execution_ms == 0
        {
            return Err(HostConnectorError {
                code: HostConnectorErrorCode::ModelIncompatible,
                message: "model manifest resource limits or ABI are invalid".to_string(),
            });
        }
        if !self
            .supported_profiles
            .iter()
            .any(|profile| profile == PLACEMENT_WASM_CPU)
        {
            return Err(HostConnectorError {
                code: HostConnectorErrorCode::ModelIncompatible,
                message: "model manifest does not support wasm-cpu".to_string(),
            });
        }
        Ok(())
    }
}

/// Provisioned model package in the host-owned verified digest store.
#[derive(Debug, Clone)]
pub struct VerifiedModelPackage {
    /// Manifest.
    pub manifest: ModelPackageManifest,
    /// WASM bytes.
    pub wasm: Vec<u8>,
}

/// Content-addressed model package store (Spec 080-shaped; Spec 526 lifecycle later).
#[derive(Debug, Default)]
pub struct ModelPackageStore {
    by_digest: HashMap<String, VerifiedModelPackage>,
}

impl ModelPackageStore {
    /// Empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a verified package keyed by normalized package digest.
    ///
    /// # Errors
    ///
    /// Returns `model_incompatible` when manifest validation fails or digests mismatch bytes.
    pub fn insert_verified(
        &mut self,
        package: VerifiedModelPackage,
    ) -> Result<String, HostConnectorError> {
        package.manifest.validate()?;
        let wasm_digest = digest_hex(&package.wasm);
        if normalize_digest(&package.manifest.wasm_digest) != wasm_digest {
            return Err(HostConnectorError {
                code: HostConnectorErrorCode::ModelIncompatible,
                message: "model wasm digest mismatch".to_string(),
            });
        }
        let key = normalize_digest(&package.manifest.package_digest);
        self.by_digest.insert(key.clone(), package);
        Ok(key)
    }

    /// Resolve by package digest without network.
    ///
    /// # Errors
    ///
    /// Returns `model_unavailable` when the digest is not in the store.
    pub fn resolve_offline(
        &self,
        digest: &str,
    ) -> Result<&VerifiedModelPackage, HostConnectorError> {
        self.by_digest
            .get(&normalize_digest(digest))
            .ok_or_else(|| HostConnectorError {
                code: HostConnectorErrorCode::ModelUnavailable,
                message: "model package not present in verified cache".to_string(),
            })
    }
}

/// Host-staged tensor buffers (not Spec 526 entries).
#[derive(Debug, Default)]
pub struct ModelIoStore {
    inputs: HashMap<String, Vec<u8>>,
    outputs: HashMap<String, Vec<u8>>,
    next_input: u64,
    next_output: u64,
}

impl ModelIoStore {
    /// Empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Stage input bytes → single-consume `input_ref`.
    ///
    /// # Errors
    ///
    /// Returns `input_limit_exceeded` when empty or over `max_bytes`.
    pub fn stage_model_input(
        &mut self,
        bytes: &[u8],
        max_bytes: usize,
    ) -> Result<String, HostConnectorError> {
        if bytes.is_empty() || bytes.len() > max_bytes {
            return Err(HostConnectorError {
                code: HostConnectorErrorCode::InputLimitExceeded,
                message: "staged model input empty or exceeds ceiling".to_string(),
            });
        }
        self.next_input = self.next_input.saturating_add(1);
        let id = format!("input-{}", self.next_input);
        self.inputs.insert(id.clone(), bytes.to_vec());
        Ok(id)
    }

    /// Consume an `input_ref` (single-use).
    ///
    /// # Errors
    ///
    /// Returns `invalid_input` when the ref is missing or already consumed.
    pub fn take_input(&mut self, input_ref: &str) -> Result<Vec<u8>, HostConnectorError> {
        self.inputs
            .remove(input_ref)
            .ok_or_else(|| HostConnectorError {
                code: HostConnectorErrorCode::InvalidInput,
                message: "input_ref missing or already consumed".to_string(),
            })
    }

    /// Store output bytes → `output_ref`.
    pub fn put_output(&mut self, bytes: Vec<u8>) -> String {
        self.next_output = self.next_output.saturating_add(1);
        let id = format!("output-{}", self.next_output);
        self.outputs.insert(id.clone(), bytes);
        id
    }

    /// Read output bytes by ref.
    ///
    /// # Errors
    ///
    /// Returns `unavailable` when missing; `input_limit_exceeded` when over cap.
    pub fn read_model_output(
        &self,
        output_ref: &str,
        max_bytes: usize,
    ) -> Result<Vec<u8>, HostConnectorError> {
        let bytes = self
            .outputs
            .get(output_ref)
            .ok_or_else(|| HostConnectorError {
                code: HostConnectorErrorCode::Unavailable,
                message: "output_ref missing or expired".to_string(),
            })?;
        if bytes.len() > max_bytes {
            return Err(HostConnectorError {
                code: HostConnectorErrorCode::InputLimitExceeded,
                message: "output exceeds read ceiling".to_string(),
            });
        }
        Ok(bytes.clone())
    }

    /// Drop an input or output ref.
    pub fn drop_ref(&mut self, reference: &str) {
        self.inputs.remove(reference);
        self.outputs.remove(reference);
    }
}

/// Allowed data classifications for a policy.
#[derive(Debug, Clone, Default)]
pub struct ExecutionPolicy {
    /// Opaque policy id / `policy_ref`.
    pub policy_ref: String,
    /// Allowed classification strings.
    pub allowed_classifications: Vec<String>,
    /// Max output bytes under this policy.
    pub max_output_bytes: u64,
}

/// Production Spec 138 host adapter for `traverse.model-runtime`.
pub struct ExactModelHostConnector {
    /// Declared exact pins.
    pub pins: Vec<ExactModelPin>,
    /// Verified packages.
    pub packages: ModelPackageStore,
    /// Staged I/O.
    pub io: ModelIoStore,
    /// Policies keyed by `policy_ref`.
    pub policies: HashMap<String, ExecutionPolicy>,
    /// When true, resolve_offline-only (no provision path during execute).
    pub offline_mode: bool,
}

impl ExactModelHostConnector {
    /// Construct with pins and empty stores.
    #[must_use]
    pub fn new(pins: Vec<ExactModelPin>) -> Self {
        Self {
            pins,
            packages: ModelPackageStore::new(),
            io: ModelIoStore::new(),
            policies: HashMap::new(),
            offline_mode: true,
        }
    }

    fn require_pin(&self, model_ref: &ModelRef) -> Result<&ExactModelPin, HostConnectorError> {
        let digest = normalize_digest(&model_ref.digest);
        self.pins
            .iter()
            .find(|pin| {
                pin.model_id == model_ref.model_id
                    && pin.version == model_ref.version
                    && normalize_digest(&pin.digest) == digest
            })
            .ok_or_else(|| HostConnectorError {
                code: HostConnectorErrorCode::ModelUnavailable,
                message: "model_ref does not match an exact_model_dependencies pin".to_string(),
            })
    }
}

#[derive(Debug, Clone, Deserialize)]
struct ModelRef {
    model_id: String,
    version: String,
    digest: String,
}

#[derive(Debug, Deserialize)]
struct ModelExecutePayload {
    model_ref: ModelRef,
    input_ref: String,
    policy_ref: String,
    data_classification: String,
    input_schema_ref: String,
    input_schema_version: String,
    max_output_bytes: u64,
    #[serde(default)]
    max_memory_bytes: Option<u64>,
    #[serde(default)]
    max_fuel: Option<u64>,
    #[serde(default)]
    timeout_ms: Option<u64>,
    #[serde(default)]
    feature_metadata: Option<Value>,
}

impl HostConnectorPort for ExactModelHostConnector {
    #[allow(clippy::too_many_lines)]
    fn invoke(
        &mut self,
        request: &HostConnectorHostRequest,
    ) -> Result<HostConnectorHostResult, HostConnectorError> {
        if request.connector_id != MODEL_RUNTIME_CONNECTOR
            || request.operation != MODEL_EXECUTE_OPERATION
        {
            return Err(HostConnectorError {
                code: HostConnectorErrorCode::Incompatible,
                message: "ExactModelHostConnector only serves model.execute".to_string(),
            });
        }
        if request.cancel_requested {
            return Err(HostConnectorError {
                code: HostConnectorErrorCode::Cancelled,
                message: "model.execute cancelled before invoke".to_string(),
            });
        }

        let payload: ModelExecutePayload = serde_json::from_value(request.payload.clone())
            .map_err(|_| HostConnectorError {
                code: HostConnectorErrorCode::InvalidInput,
                message: "model.execute payload failed schema validation".to_string(),
            })?;

        let pin = self.require_pin(&payload.model_ref)?;
        if self.offline_mode && !pin.offline_allowed {
            return Err(HostConnectorError {
                code: HostConnectorErrorCode::ModelUnavailable,
                message: "pin does not allow offline execution".to_string(),
            });
        }

        let policy = self
            .policies
            .get(&payload.policy_ref)
            .ok_or_else(|| HostConnectorError {
                code: HostConnectorErrorCode::PolicyDenied,
                message: "policy_ref is not activated".to_string(),
            })?;
        if !policy
            .allowed_classifications
            .iter()
            .any(|class| class == &payload.data_classification)
        {
            return Err(HostConnectorError {
                code: HostConnectorErrorCode::PolicyDenied,
                message: "data_classification denied by policy".to_string(),
            });
        }

        let package = self.packages.resolve_offline(&payload.model_ref.digest)?;
        if package.manifest.model_id != payload.model_ref.model_id
            || package.manifest.version != payload.model_ref.version
        {
            return Err(HostConnectorError {
                code: HostConnectorErrorCode::ModelIncompatible,
                message: "cached package identity does not match model_ref".to_string(),
            });
        }
        if package.manifest.input_schema_ref != payload.input_schema_ref
            || package.manifest.input_schema_version != payload.input_schema_version
        {
            return Err(HostConnectorError {
                code: HostConnectorErrorCode::ModelIncompatible,
                message: "input schema does not match model manifest".to_string(),
            });
        }

        let call_max_out = payload
            .max_output_bytes
            .min(policy.max_output_bytes)
            .min(package.manifest.max_output_bytes);
        if call_max_out == 0 {
            return Err(HostConnectorError {
                code: HostConnectorErrorCode::ResourceExhausted,
                message: "output ceiling is zero after policy intersection".to_string(),
            });
        }

        let input = self.io.take_input(&payload.input_ref)?;
        if input.len() as u64 > package.manifest.max_input_bytes {
            return Err(HostConnectorError {
                code: HostConnectorErrorCode::ResourceExhausted,
                message: "input exceeds model manifest ceiling".to_string(),
            });
        }

        let memory = payload
            .max_memory_bytes
            .unwrap_or(package.manifest.max_memory_bytes)
            .min(package.manifest.max_memory_bytes);
        let fuel = payload
            .max_fuel
            .unwrap_or(package.manifest.max_fuel)
            .min(package.manifest.max_fuel);
        let timeout = Duration::from_millis(
            payload
                .timeout_ms
                .unwrap_or(package.manifest.max_execution_ms)
                .min(package.manifest.max_execution_ms),
        );

        let _ = &payload.feature_metadata;
        let started = Instant::now();
        let output = execute_wasm_cpu_model(&package.wasm, &input, memory, fuel, call_max_out)?;
        if started.elapsed() > timeout {
            return Err(HostConnectorError {
                code: HostConnectorErrorCode::Timeout,
                message: "model.execute exceeded timeout".to_string(),
            });
        }
        if output.len() as u64 > call_max_out {
            return Err(HostConnectorError {
                code: HostConnectorErrorCode::ResourceExhausted,
                message: "model output exceeds ceiling".to_string(),
            });
        }

        let output_ref = self.io.put_output(output);
        Ok(HostConnectorHostResult {
            artifact_ref: output_ref,
        })
    }
}

/// Encode a versioned little-endian feature/output frame.
#[must_use]
pub fn encode_guest_frame(dtype: u8, dims: &[u32], payload: &[u8]) -> Vec<u8> {
    let rank = u8::try_from(dims.len()).unwrap_or(0);
    let mut out = Vec::with_capacity(8 + dims.len() * 4 + payload.len());
    out.extend_from_slice(&MODEL_GUEST_ABI_VERSION.to_le_bytes());
    out.push(dtype);
    out.push(rank);
    for dim in dims {
        out.extend_from_slice(&dim.to_le_bytes());
    }
    let len = u32::try_from(payload.len()).unwrap_or(u32::MAX);
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(payload);
    out
}

/// Decode a guest frame; fail closed on truncation.
///
/// # Errors
///
/// Returns `invalid_input` when the frame is malformed.
pub fn decode_guest_frame(bytes: &[u8]) -> Result<(u8, Vec<u32>, Vec<u8>), HostConnectorError> {
    if bytes.len() < 8 {
        return Err(HostConnectorError {
            code: HostConnectorErrorCode::InvalidInput,
            message: "guest frame too short".to_string(),
        });
    }
    let abi = u16::from_le_bytes([bytes[0], bytes[1]]);
    if abi != MODEL_GUEST_ABI_VERSION {
        return Err(HostConnectorError {
            code: HostConnectorErrorCode::ModelIncompatible,
            message: "unsupported guest ABI version".to_string(),
        });
    }
    let dtype = bytes[2];
    let rank = bytes[3] as usize;
    let header = 4 + rank * 4 + 4;
    if bytes.len() < header {
        return Err(HostConnectorError {
            code: HostConnectorErrorCode::InvalidInput,
            message: "guest frame header truncated".to_string(),
        });
    }
    let mut dims = Vec::with_capacity(rank);
    for index in 0..rank {
        let start = 4 + index * 4;
        dims.push(u32::from_le_bytes([
            bytes[start],
            bytes[start + 1],
            bytes[start + 2],
            bytes[start + 3],
        ]));
    }
    let len_start = 4 + rank * 4;
    let payload_len = u32::from_le_bytes([
        bytes[len_start],
        bytes[len_start + 1],
        bytes[len_start + 2],
        bytes[len_start + 3],
    ]) as usize;
    let payload_start = header;
    let payload_end = payload_start.saturating_add(payload_len);
    if bytes.len() < payload_end {
        return Err(HostConnectorError {
            code: HostConnectorErrorCode::InvalidInput,
            message: "guest frame payload truncated".to_string(),
        });
    }
    Ok((dtype, dims, bytes[payload_start..payload_end].to_vec()))
}

/// SHA-256 hex digest of bytes.
#[must_use]
pub fn digest_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex_encode(&hasher.finalize())
}

/// Normalize `sha256:` prefix away.
#[must_use]
pub fn normalize_digest(value: &str) -> String {
    value
        .trim()
        .strip_prefix("sha256:")
        .unwrap_or(value.trim())
        .to_ascii_lowercase()
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0xf) as usize] as char);
    }
    out
}

#[cfg(feature = "wasmtime-executor")]
#[allow(
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation,
    clippy::unwrap_used
)]
fn execute_wasm_cpu_model(
    wasm: &[u8],
    input: &[u8],
    max_memory_bytes: u64,
    max_fuel: u64,
    max_output_bytes: u64,
) -> Result<Vec<u8>, HostConnectorError> {
    use wasmtime::{Config, Engine, Linker, Module, Store, StoreLimitsBuilder};

    let mut config = Config::new();
    config.consume_fuel(true);
    let engine = Engine::new(&config).map_err(|_| HostConnectorError {
        code: HostConnectorErrorCode::ExecutionFailed,
        message: "failed to create wasm-cpu engine".to_string(),
    })?;
    let module = Module::new(&engine, wasm).map_err(|_| HostConnectorError {
        code: HostConnectorErrorCode::ModelIncompatible,
        message: "model wasm failed validation".to_string(),
    })?;

    let _memory_pages = max_memory_bytes
        .max(65_536)
        .div_ceil(65_536)
        .min(u64::from(u32::MAX));
    let limits = StoreLimitsBuilder::new()
        .memory_size(usize::try_from(max_memory_bytes).unwrap_or(usize::MAX))
        .build();
    let mut store = Store::new(&engine, limits);
    store.limiter(|state| state);
    store.set_fuel(max_fuel).map_err(|_| HostConnectorError {
        code: HostConnectorErrorCode::ExecutionFailed,
        message: "failed to set fuel".to_string(),
    })?;

    // Empty linker: deny-by-default (no WASI / no host imports).
    let linker = Linker::new(&engine);
    let instance = linker
        .instantiate(&mut store, &module)
        .map_err(|_| HostConnectorError {
            code: HostConnectorErrorCode::ExecutionFailed,
            message: "model wasm instantiation failed".to_string(),
        })?;

    let memory = instance
        .get_memory(&mut store, "memory")
        .ok_or_else(|| HostConnectorError {
            code: HostConnectorErrorCode::ModelIncompatible,
            message: "model wasm missing memory export".to_string(),
        })?;
    let func = instance
        .get_typed_func::<(i32, i32, i32, i32), i32>(&mut store, MODEL_EXECUTE_EXPORT)
        .map_err(|_| HostConnectorError {
            code: HostConnectorErrorCode::ModelIncompatible,
            message: "model wasm missing model_execute export".to_string(),
        })?;

    let in_ptr = 64_i32;
    let out_ptr = in_ptr + i32::try_from(input.len()).unwrap_or(i32::MAX) + 64;
    let out_cap =
        i32::try_from(max_output_bytes.min(u64::from(i32::MAX as u32))).unwrap_or(i32::MAX);
    let end = usize::try_from(out_ptr).unwrap_or(0) + usize::try_from(out_cap).unwrap_or(0);
    let current_pages = u64::try_from(memory.data_size(&store))
        .unwrap_or(0)
        .div_ceil(65_536);
    let needed_pages = u64::try_from(end).unwrap_or(0).div_ceil(65_536);
    if needed_pages > current_pages {
        memory
            .grow(&mut store, needed_pages - current_pages)
            .map_err(|_| HostConnectorError {
                code: HostConnectorErrorCode::ResourceExhausted,
                message: "model memory grow failed".to_string(),
            })?;
    }
    memory
        .write(&mut store, usize::try_from(in_ptr).unwrap_or(0), input)
        .map_err(|_| HostConnectorError {
            code: HostConnectorErrorCode::ExecutionFailed,
            message: "failed to write model input".to_string(),
        })?;

    let out_len = func
        .call(
            &mut store,
            (
                in_ptr,
                i32::try_from(input.len()).unwrap_or(i32::MAX),
                out_ptr,
                out_cap,
            ),
        )
        .map_err(|_| HostConnectorError {
            code: HostConnectorErrorCode::ExecutionFailed,
            message: "model_execute trap or fuel exhausted".to_string(),
        })?;
    if out_len < 0 || u64::try_from(out_len).unwrap_or(u64::MAX) > max_output_bytes {
        return Err(HostConnectorError {
            code: HostConnectorErrorCode::ResourceExhausted,
            message: "model returned invalid output length".to_string(),
        });
    }
    let mut output = vec![0_u8; usize::try_from(out_len).unwrap_or(0)];
    memory
        .read(&store, usize::try_from(out_ptr).unwrap_or(0), &mut output)
        .map_err(|_| HostConnectorError {
            code: HostConnectorErrorCode::ExecutionFailed,
            message: "failed to read model output".to_string(),
        })?;
    Ok(output)
}

#[cfg(not(feature = "wasmtime-executor"))]
fn execute_wasm_cpu_model(
    _wasm: &[u8],
    _input: &[u8],
    _max_memory_bytes: u64,
    _max_fuel: u64,
    _max_output_bytes: u64,
) -> Result<Vec<u8>, HostConnectorError> {
    Err(HostConnectorError {
        code: HostConnectorErrorCode::Unavailable,
        message: "wasm-cpu executor requires wasmtime-executor feature".to_string(),
    })
}

/// WAT source for the signed echo fixture model.
pub const FIXTURE_ECHO_WAT: &str = r#"
(module
  (memory (export "memory") 2)
  (func (export "model_execute")
    (param $in_ptr i32) (param $in_len i32) (param $out_ptr i32) (param $out_cap i32) (result i32)
    (local $i i32)
    (local $n i32)
    (local.set $n (local.get $in_len))
    (if (i32.gt_u (local.get $n) (local.get $out_cap))
      (then (local.set $n (local.get $out_cap))))
    (block $done
      (loop $copy
        (br_if $done (i32.ge_u (local.get $i) (local.get $n)))
        (i32.store8
          (i32.add (local.get $out_ptr) (local.get $i))
          (i32.load8_u (i32.add (local.get $in_ptr) (local.get $i))))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $copy)))
    (local.get $n)
  )
)
"#;

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::host_connector_dispatch::{
        COMMAND_KIND, HostConnectorActivationSet, HostConnectorAppCommand,
        HostConnectorAppManifest, HostConnectorBinding, HostConnectorCommandRoute,
        HostConnectorDispatchContext, HostConnectorIdempotencyStore, SCHEMA_VERSION,
        dispatch_host_connector_command,
    };
    use serde_json::json;

    fn fixture_package() -> VerifiedModelPackage {
        let wasm = wat::parse_str(FIXTURE_ECHO_WAT).expect("wat");
        let wasm_digest = digest_hex(&wasm);
        let manifest = ModelPackageManifest {
            schema_version: "1.0.0".to_string(),
            model_id: "fixture.echo".to_string(),
            version: "1.0.0".to_string(),
            wasm_digest: wasm_digest.clone(),
            package_digest: wasm_digest.clone(),
            registry_ref: "registry:fixture.echo@1.0.0".to_string(),
            executable_format: "traverse-model-wasm".to_string(),
            abi_version: MODEL_GUEST_ABI_VERSION,
            input_schema_ref: "schema:fixture-in".to_string(),
            input_schema_version: "1.0.0".to_string(),
            output_schema_ref: "schema:fixture-out".to_string(),
            output_schema_version: "1.0.0".to_string(),
            license_id: "Apache-2.0".to_string(),
            attribution: "Traverse fixture".to_string(),
            redistribution: "test-only".to_string(),
            supported_profiles: vec![PLACEMENT_WASM_CPU.to_string()],
            max_memory_bytes: 2 * 64 * 1024,
            max_fuel: 1_000_000,
            max_input_bytes: 4096,
            max_output_bytes: 4096,
            max_execution_ms: 5_000,
            offline_allowed: true,
        };
        VerifiedModelPackage { manifest, wasm }
    }

    #[test]
    fn stage_execute_read_echo_model_round_trip() {
        let package = fixture_package();
        let digest = package.manifest.package_digest.clone();
        let pin = ExactModelPin {
            model_id: "fixture.echo".to_string(),
            version: "1.0.0".to_string(),
            digest: digest.clone(),
            offline_allowed: true,
        };
        let mut host = ExactModelHostConnector::new(vec![pin]);
        host.packages
            .insert_verified(package)
            .expect("insert package");
        host.policies.insert(
            "policy-1".to_string(),
            ExecutionPolicy {
                policy_ref: "policy-1".to_string(),
                allowed_classifications: vec!["sensitive".to_string()],
                max_output_bytes: 4096,
            },
        );

        let frame = encode_guest_frame(1, &[4], b"test");
        let input_ref = host.io.stage_model_input(&frame, 4096).expect("stage");
        let manifest = HostConnectorAppManifest {
            app_id: "fixture.app".to_string(),
            connector_bindings: vec![HostConnectorBinding {
                binding_id: "default-local-model".to_string(),
                connector_id: MODEL_RUNTIME_CONNECTOR.to_string(),
                version: "2.0.0".to_string(),
                config_ref: "authority:local".to_string(),
                placement_targets: vec!["macos".to_string(), "local".to_string()],
            }],
            command_routes: vec![HostConnectorCommandRoute {
                command: "run_local_model".to_string(),
                connector_id: MODEL_RUNTIME_CONNECTOR.to_string(),
                operation: MODEL_EXECUTE_OPERATION.to_string(),
            }],
        };
        let mut activations = HostConnectorActivationSet::default();
        activations.activate("default-local-model");
        let command = HostConnectorAppCommand {
            kind: COMMAND_KIND.to_string(),
            schema_version: SCHEMA_VERSION.to_string(),
            command: "run_local_model".to_string(),
            command_id: "cmd-model-1".to_string(),
            correlation_id: "corr-model-1".to_string(),
            idempotency_key: "idem-model-1".to_string(),
            target_family: "macos".to_string(),
            cancel_requested: false,
            payload: json!({
                "model_ref": {
                    "model_id": "fixture.echo",
                    "version": "1.0.0",
                    "digest": digest
                },
                "input_ref": input_ref,
                "policy_ref": "policy-1",
                "data_classification": "sensitive",
                "input_schema_ref": "schema:fixture-in",
                "input_schema_version": "1.0.0",
                "max_output_bytes": 4096
            }),
        };
        let mut idempotency = HostConnectorIdempotencyStore::new();
        let mut ctx = HostConnectorDispatchContext {
            manifest: &manifest,
            activations: &activations,
            idempotency: &mut idempotency,
            host: &mut host,
        };
        let dispatch = dispatch_host_connector_command(&command, &mut ctx).expect("dispatch");
        let output_ref = dispatch.artifact_ref.expect("output_ref");
        let output = host.io.read_model_output(&output_ref, 4096).expect("read");
        assert_eq!(output, frame);
        assert!(host.io.take_input("input-1").is_err());
    }

    #[test]
    fn offline_cache_miss_is_model_unavailable() {
        let mut host = ExactModelHostConnector::new(vec![ExactModelPin {
            model_id: "fixture.echo".to_string(),
            version: "1.0.0".to_string(),
            digest: "deadbeef".to_string(),
            offline_allowed: true,
        }]);
        host.policies.insert(
            "policy-1".to_string(),
            ExecutionPolicy {
                policy_ref: "policy-1".to_string(),
                allowed_classifications: vec!["sensitive".to_string()],
                max_output_bytes: 4096,
            },
        );
        let input_ref = host.io.stage_model_input(b"abc", 64).expect("stage");
        let err = host
            .invoke(&HostConnectorHostRequest {
                connector_id: MODEL_RUNTIME_CONNECTOR.to_string(),
                operation: MODEL_EXECUTE_OPERATION.to_string(),
                binding_id: "b".to_string(),
                target_family: "macos".to_string(),
                correlation_id: "c".to_string(),
                payload: json!({
                    "model_ref": {
                        "model_id": "fixture.echo",
                        "version": "1.0.0",
                        "digest": "deadbeef"
                    },
                    "input_ref": input_ref,
                    "policy_ref": "policy-1",
                    "data_classification": "sensitive",
                    "input_schema_ref": "schema:fixture-in",
                    "input_schema_version": "1.0.0",
                    "max_output_bytes": 64
                }),
                cancel_requested: false,
            })
            .expect_err("miss");
        assert_eq!(err.code, HostConnectorErrorCode::ModelUnavailable);
    }
}
