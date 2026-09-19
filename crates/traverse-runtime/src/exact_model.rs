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

        let output_ref = self.io.put_output(output);
        Ok(HostConnectorHostResult {
            artifact_ref: Some(output_ref),
            permission_state: None,
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
fn model_host_err(code: HostConnectorErrorCode, message: &str) -> HostConnectorError {
    HostConnectorError {
        code,
        message: message.to_string(),
    }
}

#[cfg(feature = "wasmtime-executor")]
fn require_ok(
    ok: bool,
    code: HostConnectorErrorCode,
    message: &str,
) -> Result<(), HostConnectorError> {
    if ok {
        Ok(())
    } else {
        Err(model_host_err(code, message))
    }
}

#[cfg(feature = "wasmtime-executor")]
#[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
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
    // Engine::new only fails on illegal config; consume_fuel config is always legal.
    #[allow(clippy::unwrap_used)]
    let engine = Engine::new(&config).unwrap();
    let Some(module) = Module::new(&engine, wasm).ok() else {
        return Err(model_host_err(
            HostConnectorErrorCode::ModelIncompatible,
            "model wasm failed validation",
        ));
    };

    let limits = StoreLimitsBuilder::new()
        .memory_size(usize::try_from(max_memory_bytes).unwrap_or(usize::MAX))
        .build();
    let mut store = Store::new(&engine, limits);
    store.limiter(|state| state);
    // Fuel is enabled on the engine config; set_fuel only fails when fuel is disabled.
    let _ = store.set_fuel(max_fuel);

    // Empty linker: deny-by-default (no WASI / no host imports).
    let linker = Linker::new(&engine);
    let Some(instance) = linker.instantiate(&mut store, &module).ok() else {
        return Err(model_host_err(
            HostConnectorErrorCode::ExecutionFailed,
            "model wasm instantiation failed",
        ));
    };

    let Some(memory) = instance.get_memory(&mut store, "memory") else {
        return Err(model_host_err(
            HostConnectorErrorCode::ModelIncompatible,
            "model wasm missing memory export",
        ));
    };
    let Some(func) = instance
        .get_typed_func::<(i32, i32, i32, i32), i32>(&mut store, MODEL_EXECUTE_EXPORT)
        .ok()
    else {
        return Err(model_host_err(
            HostConnectorErrorCode::ModelIncompatible,
            "model wasm missing model_execute export",
        ));
    };

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
        require_ok(
            memory
                .grow(&mut store, needed_pages - current_pages)
                .is_ok(),
            HostConnectorErrorCode::ResourceExhausted,
            "model memory grow failed",
        )?;
    }
    // Region sizing above ensures the staged write/read windows fit; allocator faults
    // after a successful grow are not distinguishable from guest traps below.
    let _ = memory.write(&mut store, usize::try_from(in_ptr).unwrap_or(0), input);

    let Some(out_len) = func
        .call(
            &mut store,
            (
                in_ptr,
                i32::try_from(input.len()).unwrap_or(i32::MAX),
                out_ptr,
                out_cap,
            ),
        )
        .ok()
    else {
        return Err(model_host_err(
            HostConnectorErrorCode::ExecutionFailed,
            "model_execute trap or fuel exhausted",
        ));
    };
    if out_len < 0 || u64::try_from(out_len).unwrap_or(u64::MAX) > max_output_bytes {
        return Err(model_host_err(
            HostConnectorErrorCode::ResourceExhausted,
            "model returned invalid output length",
        ));
    }
    let mut output = vec![0_u8; usize::try_from(out_len).unwrap_or(0)];
    let _ = memory.read(&store, usize::try_from(out_ptr).unwrap_or(0), &mut output);
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

/// WAT source for the signed real-inference conformance fixture: a fixed-weight
/// linear classifier over 4 `f32` features, proving genuinely computed
/// inference (not a pass-through) through the same governed pipeline as the
/// echo fixture. Input/output frames use the Spec 138 guest ABI
/// (`encode_guest_frame`/`decode_guest_frame`): input dtype 2, dims `[4]`,
/// payload = 4 little-endian `f32` features; output dtype 3, dims `[2]`,
/// payload = `[score, label]` as little-endian `f32` (label is 1.0 or 0.0).
/// Fails closed (`-1`) when the input or output-capacity ceilings are too
/// small for that fixed frame shape.
pub const FIXTURE_CLASSIFIER_WAT: &str = r#"
(module
  (memory (export "memory") 2)
  (func (export "model_execute")
    (param $in_ptr i32) (param $in_len i32) (param $out_ptr i32) (param $out_cap i32) (result i32)
    (local $x0 f32) (local $x1 f32) (local $x2 f32) (local $x3 f32) (local $score f32) (local $label f32)
    (if (i32.lt_u (local.get $in_len) (i32.const 28))
      (then (return (i32.const -1))))
    (if (i32.lt_u (local.get $out_cap) (i32.const 20))
      (then (return (i32.const -1))))
    (local.set $x0 (f32.load offset=12 (local.get $in_ptr)))
    (local.set $x1 (f32.load offset=16 (local.get $in_ptr)))
    (local.set $x2 (f32.load offset=20 (local.get $in_ptr)))
    (local.set $x3 (f32.load offset=24 (local.get $in_ptr)))
    (local.set $score
      (f32.sub
        (f32.add
          (f32.add
            (f32.mul (local.get $x0) (f32.const 0.5))
            (f32.mul (local.get $x1) (f32.const -0.25)))
          (f32.add
            (f32.mul (local.get $x2) (f32.const 1.0))
            (f32.mul (local.get $x3) (f32.const 0.75))))
        (f32.const 0.5)))
    (local.set $label
      (select (f32.const 1.0) (f32.const 0.0) (f32.ge (local.get $score) (f32.const 0.0))))
    (i32.store16 offset=0 (local.get $out_ptr) (i32.const 1))
    (i32.store8 offset=2 (local.get $out_ptr) (i32.const 3))
    (i32.store8 offset=3 (local.get $out_ptr) (i32.const 1))
    (i32.store offset=4 (local.get $out_ptr) (i32.const 2))
    (i32.store offset=8 (local.get $out_ptr) (i32.const 8))
    (f32.store offset=12 (local.get $out_ptr) (local.get $score))
    (f32.store offset=16 (local.get $out_ptr) (local.get $label))
    (i32.const 20)
  )
)
"#;

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines,
    clippy::unwrap_used
)]
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

    fn seeded_host() -> (ExactModelHostConnector, String) {
        let package = fixture_package();
        let digest = package.manifest.package_digest.clone();
        let pin = ExactModelPin {
            model_id: "fixture.echo".to_string(),
            version: "1.0.0".to_string(),
            digest: format!("sha256:{digest}"),
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
        (host, digest)
    }

    fn execute_request(
        digest: &str,
        input_ref: &str,
        extras: serde_json::Map<String, Value>,
    ) -> HostConnectorHostRequest {
        let mut payload = json!({
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
        });
        if let Some(object) = payload.as_object_mut() {
            object.extend(extras);
        }
        HostConnectorHostRequest {
            connector_id: MODEL_RUNTIME_CONNECTOR.to_string(),
            operation: MODEL_EXECUTE_OPERATION.to_string(),
            binding_id: "b".to_string(),
            target_family: "macos".to_string(),
            correlation_id: "c".to_string(),
            payload,
            cancel_requested: false,
        }
    }

    #[test]
    fn manifest_validate_and_package_store_reject_invalid_packages() {
        let good = fixture_package();
        good.manifest.validate().expect("valid");

        let mut missing_license = fixture_package();
        missing_license.manifest.license_id.clear();
        assert_eq!(
            missing_license
                .manifest
                .validate()
                .expect_err("license")
                .code,
            HostConnectorErrorCode::ModelIncompatible
        );

        let mut bad_limits = fixture_package();
        bad_limits.manifest.abi_version = 0;
        assert_eq!(
            bad_limits.manifest.validate().expect_err("limits").code,
            HostConnectorErrorCode::ModelIncompatible
        );

        let mut no_cpu = fixture_package();
        no_cpu.manifest.supported_profiles = vec!["gpu".to_string()];
        assert_eq!(
            no_cpu.manifest.validate().expect_err("profile").code,
            HostConnectorErrorCode::ModelIncompatible
        );

        let mut store = ModelPackageStore::new();
        let mut mismatched = fixture_package();
        mismatched.manifest.wasm_digest = "00".repeat(32);
        assert_eq!(
            store.insert_verified(mismatched).expect_err("digest").code,
            HostConnectorErrorCode::ModelIncompatible
        );
        assert_eq!(
            store.resolve_offline("missing").expect_err("offline").code,
            HostConnectorErrorCode::ModelUnavailable
        );
    }

    #[test]
    fn model_io_store_stage_read_drop_edges() {
        let mut io = ModelIoStore::new();
        assert_eq!(
            io.stage_model_input(b"", 8).expect_err("empty").code,
            HostConnectorErrorCode::InputLimitExceeded
        );
        assert_eq!(
            io.stage_model_input(b"abcdef", 4).expect_err("over").code,
            HostConnectorErrorCode::InputLimitExceeded
        );
        let input_ref = io.stage_model_input(b"abc", 8).expect("stage");
        assert_eq!(io.take_input(&input_ref).expect("take"), b"abc");
        assert_eq!(
            io.take_input(&input_ref).expect_err("consumed").code,
            HostConnectorErrorCode::InvalidInput
        );

        let output_ref = io.put_output(vec![1, 2, 3, 4]);
        assert_eq!(
            io.read_model_output(&output_ref, 2).expect_err("cap").code,
            HostConnectorErrorCode::InputLimitExceeded
        );
        assert_eq!(
            io.read_model_output(&output_ref, 8).expect("read"),
            vec![1, 2, 3, 4]
        );
        assert_eq!(
            io.read_model_output("missing", 8).expect_err("miss").code,
            HostConnectorErrorCode::Unavailable
        );
        io.drop_ref(&output_ref);
        assert!(io.read_model_output(&output_ref, 8).is_err());
    }

    #[test]
    fn guest_frame_round_trip_and_decode_failures() {
        let encoded = encode_guest_frame(7, &[2, 3], b"abcdef");
        let (dtype, dims, payload) = decode_guest_frame(&encoded).expect("decode");
        assert_eq!(dtype, 7);
        assert_eq!(dims, vec![2, 3]);
        assert_eq!(payload, b"abcdef");
        assert_eq!(normalize_digest(" sha256:AbCd "), "abcd");
        assert_eq!(normalize_digest("SHA256:Ab"), "sha256:ab");
        assert_eq!(digest_hex(b"x").len(), 64);

        assert_eq!(
            decode_guest_frame(&[0, 1, 2]).expect_err("short").code,
            HostConnectorErrorCode::InvalidInput
        );
        let mut bad_abi = encoded.clone();
        bad_abi[0] = 9;
        assert_eq!(
            decode_guest_frame(&bad_abi).expect_err("abi").code,
            HostConnectorErrorCode::ModelIncompatible
        );
        // ABI + dtype/rank present, but dim bytes truncated before payload length.
        let mut truncated_header = encode_guest_frame(1, &[1, 2, 3], b"");
        truncated_header.truncate(8);
        assert_eq!(
            decode_guest_frame(&truncated_header).expect_err("hdr").code,
            HostConnectorErrorCode::InvalidInput
        );
        let mut truncated_payload = encode_guest_frame(1, &[1], b"abcd");
        truncated_payload.truncate(truncated_payload.len() - 1);
        assert_eq!(
            decode_guest_frame(&truncated_payload)
                .expect_err("payload")
                .code,
            HostConnectorErrorCode::InvalidInput
        );
        let huge = encode_guest_frame(1, &vec![1; 300], b"z");
        assert_eq!(huge[3], 0); // rank saturates via unwrap_or(0) for >255 dims
    }

    #[test]
    fn invoke_rejects_wrong_route_cancel_and_invalid_payload() {
        let (mut host, digest) = seeded_host();
        assert_eq!(
            host.invoke(&HostConnectorHostRequest {
                connector_id: "other".to_string(),
                operation: MODEL_EXECUTE_OPERATION.to_string(),
                binding_id: "b".to_string(),
                target_family: "macos".to_string(),
                correlation_id: "c".to_string(),
                payload: json!({}),
                cancel_requested: false,
            })
            .expect_err("route")
            .code,
            HostConnectorErrorCode::Incompatible
        );

        let mut cancelled = execute_request(&digest, "input-1", serde_json::Map::new());
        cancelled.cancel_requested = true;
        assert_eq!(
            host.invoke(&cancelled).expect_err("cancel").code,
            HostConnectorErrorCode::Cancelled
        );

        let bad = HostConnectorHostRequest {
            connector_id: MODEL_RUNTIME_CONNECTOR.to_string(),
            operation: MODEL_EXECUTE_OPERATION.to_string(),
            binding_id: "b".to_string(),
            target_family: "macos".to_string(),
            correlation_id: "c".to_string(),
            payload: json!("not-an-object"),
            cancel_requested: false,
        };
        assert_eq!(
            host.invoke(&bad).expect_err("payload").code,
            HostConnectorErrorCode::InvalidInput
        );
    }

    #[test]
    fn invoke_policy_pin_schema_and_resource_failures() {
        let (mut host, digest) = seeded_host();
        let input_ref = host.io.stage_model_input(b"abc", 64).expect("stage");

        let mut unknown_pin = execute_request(&digest, &input_ref, serde_json::Map::new());
        unknown_pin.payload["model_ref"]["model_id"] = json!("other.model");
        assert_eq!(
            host.invoke(&unknown_pin).expect_err("pin").code,
            HostConnectorErrorCode::ModelUnavailable
        );

        host.pins[0].offline_allowed = false;
        let input_ref = host.io.stage_model_input(b"abc", 64).expect("stage");
        assert_eq!(
            host.invoke(&execute_request(
                &digest,
                &input_ref,
                serde_json::Map::new()
            ))
            .expect_err("offline")
            .code,
            HostConnectorErrorCode::ModelUnavailable
        );
        host.pins[0].offline_allowed = true;

        let input_ref = host.io.stage_model_input(b"abc", 64).expect("stage");
        let mut missing_policy = execute_request(&digest, &input_ref, serde_json::Map::new());
        missing_policy.payload["policy_ref"] = json!("missing");
        assert_eq!(
            host.invoke(&missing_policy).expect_err("policy").code,
            HostConnectorErrorCode::PolicyDenied
        );

        let input_ref = host.io.stage_model_input(b"abc", 64).expect("stage");
        let mut denied = execute_request(&digest, &input_ref, serde_json::Map::new());
        denied.payload["data_classification"] = json!("secret");
        assert_eq!(
            host.invoke(&denied).expect_err("class").code,
            HostConnectorErrorCode::PolicyDenied
        );

        let mut identity_mismatch = fixture_package();
        identity_mismatch.manifest.model_id = "other".to_string();
        identity_mismatch.manifest.package_digest = format!("{}aa", &digest[..62]);
        identity_mismatch.manifest.wasm_digest = digest_hex(&identity_mismatch.wasm);
        // Re-key under the requested digest by forging package_digest after byte digest match.
        identity_mismatch.manifest.package_digest = digest.clone();
        host.packages
            .insert_verified(identity_mismatch)
            .expect("overwrite");
        let input_ref = host.io.stage_model_input(b"abc", 64).expect("stage");
        assert_eq!(
            host.invoke(&execute_request(
                &digest,
                &input_ref,
                serde_json::Map::new()
            ))
            .expect_err("identity")
            .code,
            HostConnectorErrorCode::ModelIncompatible
        );

        // Restore a matching package for remaining cases.
        let restored = fixture_package();
        host.packages.insert_verified(restored).expect("restore");

        let input_ref = host.io.stage_model_input(b"abc", 64).expect("stage");
        let mut schema = execute_request(&digest, &input_ref, serde_json::Map::new());
        schema.payload["input_schema_ref"] = json!("schema:other");
        assert_eq!(
            host.invoke(&schema).expect_err("schema").code,
            HostConnectorErrorCode::ModelIncompatible
        );

        host.policies
            .get_mut("policy-1")
            .expect("policy")
            .max_output_bytes = 0;
        let input_ref = host.io.stage_model_input(b"abc", 64).expect("stage");
        assert_eq!(
            host.invoke(&execute_request(
                &digest,
                &input_ref,
                serde_json::Map::new()
            ))
            .expect_err("zero out")
            .code,
            HostConnectorErrorCode::ResourceExhausted
        );
        host.policies
            .get_mut("policy-1")
            .expect("policy")
            .max_output_bytes = 4096;

        let oversized = vec![9_u8; 5000];
        let input_ref = host.io.stage_model_input(&oversized, 8000).expect("stage");
        assert_eq!(
            host.invoke(&execute_request(
                &digest,
                &input_ref,
                serde_json::Map::new()
            ))
            .expect_err("input ceiling")
            .code,
            HostConnectorErrorCode::ResourceExhausted
        );
    }

    #[test]
    fn invoke_honors_optional_resource_overrides_and_bad_wasm() {
        let (mut host, digest) = seeded_host();
        let frame = encode_guest_frame(1, &[2], b"ok");
        let input_ref = host.io.stage_model_input(&frame, 4096).expect("stage");
        let mut extras = serde_json::Map::new();
        extras.insert("max_memory_bytes".to_string(), json!(2 * 64 * 1024));
        extras.insert("max_fuel".to_string(), json!(100_000));
        extras.insert("timeout_ms".to_string(), json!(1_000));
        extras.insert("feature_metadata".to_string(), json!({"k": "v"}));
        let result = host
            .invoke(&execute_request(&digest, &input_ref, extras))
            .expect("execute");
        let output = host
            .io
            .read_model_output(result.artifact_ref.as_deref().expect("artifact_ref"), 4096)
            .expect("read");
        assert_eq!(output, frame);

        let mut bad_wasm = fixture_package();
        bad_wasm.wasm = b"not-wasm".to_vec();
        bad_wasm.manifest.wasm_digest = digest_hex(&bad_wasm.wasm);
        bad_wasm.manifest.package_digest = bad_wasm.manifest.wasm_digest.clone();
        let bad_digest = bad_wasm.manifest.package_digest.clone();
        host.pins.push(ExactModelPin {
            model_id: "fixture.echo".to_string(),
            version: "1.0.0".to_string(),
            digest: bad_digest.clone(),
            offline_allowed: true,
        });
        host.packages.insert_verified(bad_wasm).expect("insert bad");
        let input_ref = host.io.stage_model_input(b"abc", 64).expect("stage");
        assert_eq!(
            host.invoke(&execute_request(
                &bad_digest,
                &input_ref,
                serde_json::Map::new()
            ))
            .expect_err("bad wasm")
            .code,
            HostConnectorErrorCode::ModelIncompatible
        );

        // Missing model_execute export.
        let missing_export = wat::parse_str(
            r#"(module (memory (export "memory") 1) (func (export "other") (result i32) i32.const 0))"#,
        )
        .expect("wat");
        let mut pkg = fixture_package();
        pkg.wasm = missing_export;
        pkg.manifest.wasm_digest = digest_hex(&pkg.wasm);
        pkg.manifest.package_digest = pkg.manifest.wasm_digest.clone();
        let digest_missing = pkg.manifest.package_digest.clone();
        host.pins.push(ExactModelPin {
            model_id: "fixture.echo".to_string(),
            version: "1.0.0".to_string(),
            digest: digest_missing.clone(),
            offline_allowed: true,
        });
        host.packages.insert_verified(pkg).expect("insert");
        let input_ref = host.io.stage_model_input(b"abc", 64).expect("stage");
        assert_eq!(
            host.invoke(&execute_request(
                &digest_missing,
                &input_ref,
                serde_json::Map::new()
            ))
            .expect_err("export")
            .code,
            HostConnectorErrorCode::ModelIncompatible
        );

        // Missing memory export.
        let missing_memory = wat::parse_str(
            r#"(module (func (export "model_execute") (param i32 i32 i32 i32) (result i32) i32.const 0))"#,
        )
        .expect("wat");
        let mut pkg = fixture_package();
        pkg.wasm = missing_memory;
        pkg.manifest.wasm_digest = digest_hex(&pkg.wasm);
        pkg.manifest.package_digest = pkg.manifest.wasm_digest.clone();
        let digest_mem = pkg.manifest.package_digest.clone();
        host.pins.push(ExactModelPin {
            model_id: "fixture.echo".to_string(),
            version: "1.0.0".to_string(),
            digest: digest_mem.clone(),
            offline_allowed: true,
        });
        host.packages.insert_verified(pkg).expect("insert");
        let input_ref = host.io.stage_model_input(b"abc", 64).expect("stage");
        assert_eq!(
            host.invoke(&execute_request(
                &digest_mem,
                &input_ref,
                serde_json::Map::new()
            ))
            .expect_err("memory")
            .code,
            HostConnectorErrorCode::ModelIncompatible
        );

        // Fuel exhaustion / trap.
        let looper = wat::parse_str(
            r#"(module
              (memory (export "memory") 1)
              (func (export "model_execute") (param i32 i32 i32 i32) (result i32)
                (loop $spin (br $spin))
                i32.const 0))"#,
        )
        .expect("wat");
        let mut pkg = fixture_package();
        pkg.wasm = looper;
        pkg.manifest.wasm_digest = digest_hex(&pkg.wasm);
        pkg.manifest.package_digest = pkg.manifest.wasm_digest.clone();
        pkg.manifest.max_fuel = 10;
        let digest_fuel = pkg.manifest.package_digest.clone();
        host.pins.push(ExactModelPin {
            model_id: "fixture.echo".to_string(),
            version: "1.0.0".to_string(),
            digest: digest_fuel.clone(),
            offline_allowed: true,
        });
        host.packages.insert_verified(pkg).expect("insert");
        let input_ref = host.io.stage_model_input(b"abc", 64).expect("stage");
        assert_eq!(
            host.invoke(&execute_request(
                &digest_fuel,
                &input_ref,
                serde_json::Map::new()
            ))
            .expect_err("fuel")
            .code,
            HostConnectorErrorCode::ExecutionFailed
        );

        // Negative / oversized guest return length.
        let bad_len = wat::parse_str(
            r#"(module
              (memory (export "memory") 1)
              (func (export "model_execute") (param i32 i32 i32 i32) (result i32)
                i32.const -1))"#,
        )
        .expect("wat");
        let mut pkg = fixture_package();
        pkg.wasm = bad_len;
        pkg.manifest.wasm_digest = digest_hex(&pkg.wasm);
        pkg.manifest.package_digest = pkg.manifest.wasm_digest.clone();
        let digest_len = pkg.manifest.package_digest.clone();
        host.pins.push(ExactModelPin {
            model_id: "fixture.echo".to_string(),
            version: "1.0.0".to_string(),
            digest: digest_len.clone(),
            offline_allowed: true,
        });
        host.packages.insert_verified(pkg).expect("insert");
        let input_ref = host.io.stage_model_input(b"abc", 64).expect("stage");
        assert_eq!(
            host.invoke(&execute_request(
                &digest_len,
                &input_ref,
                serde_json::Map::new()
            ))
            .expect_err("len")
            .code,
            HostConnectorErrorCode::ResourceExhausted
        );

        // Zero timeout fails closed after guest returns (elapsed > 0).
        let input_ref = host.io.stage_model_input(b"abc", 64).expect("stage");
        let mut zero_timeout = serde_json::Map::new();
        zero_timeout.insert("timeout_ms".to_string(), json!(0));
        assert_eq!(
            host.invoke(&execute_request(&digest, &input_ref, zero_timeout))
                .expect_err("timeout")
                .code,
            HostConnectorErrorCode::Timeout
        );
    }

    #[test]
    fn require_ok_and_host_err_helpers_cover_both_branches() {
        assert!(require_ok(true, HostConnectorErrorCode::ExecutionFailed, "ok").is_ok());
        let err =
            require_ok(false, HostConnectorErrorCode::ExecutionFailed, "no").expect_err("false");
        assert_eq!(err.code, HostConnectorErrorCode::ExecutionFailed);
        assert_eq!(err.message, "no");
        let built = model_host_err(HostConnectorErrorCode::Unavailable, "x");
        assert_eq!(built.code, HostConnectorErrorCode::Unavailable);
    }

    #[test]
    fn memory_grow_success_and_failure_and_unresolved_imports() {
        let (mut host, _) = seeded_host();

        // 1-page module needs grow when output ceiling spans a second page.
        let grow_wat = r#"(module
          (memory (export "memory") 1)
          (func (export "model_execute")
            (param $in_ptr i32) (param $in_len i32) (param $out_ptr i32) (param $out_cap i32) (result i32)
            (local.get $in_len)
          ))"#;
        let mut grow_pkg = fixture_package();
        grow_pkg.wasm = wat::parse_str(grow_wat).expect("wat");
        grow_pkg.manifest.wasm_digest = digest_hex(&grow_pkg.wasm);
        grow_pkg.manifest.package_digest = grow_pkg.manifest.wasm_digest.clone();
        grow_pkg.manifest.max_memory_bytes = 4 * 64 * 1024;
        grow_pkg.manifest.max_output_bytes = 70_000;
        let grow_digest = grow_pkg.manifest.package_digest.clone();
        host.pins.push(ExactModelPin {
            model_id: "fixture.echo".to_string(),
            version: "1.0.0".to_string(),
            digest: grow_digest.clone(),
            offline_allowed: true,
        });
        host.packages.insert_verified(grow_pkg).expect("insert");
        let input_ref = host.io.stage_model_input(b"abc", 64).expect("stage");
        let mut extras = serde_json::Map::new();
        extras.insert("max_output_bytes".to_string(), json!(70_000));
        host.policies
            .get_mut("policy-1")
            .expect("policy")
            .max_output_bytes = 70_000;
        assert!(
            host.invoke(&execute_request(&grow_digest, &input_ref, extras))
                .is_ok()
        );

        // Distinct 1-page module so the store limiter can block grow independently.
        let mut blocked = fixture_package();
        let alt = r#"(module
          (memory (export "memory") 1)
          (func (export "model_execute")
            (param i32 i32 i32 i32) (result i32) (i32.const 0)))"#;
        blocked.wasm = wat::parse_str(alt).expect("wat");
        blocked.manifest.wasm_digest = digest_hex(&blocked.wasm);
        blocked.manifest.package_digest = blocked.manifest.wasm_digest.clone();
        blocked.manifest.max_memory_bytes = 64 * 1024;
        blocked.manifest.max_output_bytes = 70_000;
        let blocked_digest = blocked.manifest.package_digest.clone();
        host.pins.push(ExactModelPin {
            model_id: "fixture.echo".to_string(),
            version: "1.0.0".to_string(),
            digest: blocked_digest.clone(),
            offline_allowed: true,
        });
        host.packages.insert_verified(blocked).expect("insert");
        let input_ref = host.io.stage_model_input(b"abc", 64).expect("stage");
        let mut extras = serde_json::Map::new();
        extras.insert("max_output_bytes".to_string(), json!(70_000));
        assert_eq!(
            host.invoke(&execute_request(&blocked_digest, &input_ref, extras))
                .expect_err("grow")
                .code,
            HostConnectorErrorCode::ResourceExhausted
        );

        // Unresolved import → instantiation failed.
        let imports = wat::parse_str(
            r#"(module
              (import "env" "abort" (func (param i32)))
              (memory (export "memory") 1)
              (func (export "model_execute") (param i32 i32 i32 i32) (result i32) i32.const 0))"#,
        )
        .expect("wat");
        let mut pkg = fixture_package();
        pkg.wasm = imports;
        pkg.manifest.wasm_digest = digest_hex(&pkg.wasm);
        pkg.manifest.package_digest = pkg.manifest.wasm_digest.clone();
        let import_digest = pkg.manifest.package_digest.clone();
        host.pins.push(ExactModelPin {
            model_id: "fixture.echo".to_string(),
            version: "1.0.0".to_string(),
            digest: import_digest.clone(),
            offline_allowed: true,
        });
        host.packages.insert_verified(pkg).expect("insert");
        let input_ref = host.io.stage_model_input(b"abc", 64).expect("stage");
        assert_eq!(
            host.invoke(&execute_request(
                &import_digest,
                &input_ref,
                serde_json::Map::new()
            ))
            .expect_err("imports")
            .code,
            HostConnectorErrorCode::ExecutionFailed
        );
    }

    fn fixture_classifier_package() -> VerifiedModelPackage {
        let wasm = wat::parse_str(FIXTURE_CLASSIFIER_WAT).expect("wat");
        let wasm_digest = digest_hex(&wasm);
        let manifest = ModelPackageManifest {
            schema_version: "1.0.0".to_string(),
            model_id: "fixture.classifier".to_string(),
            version: "1.0.0".to_string(),
            wasm_digest: wasm_digest.clone(),
            package_digest: wasm_digest.clone(),
            registry_ref: "registry:fixture.classifier@1.0.0".to_string(),
            executable_format: "traverse-model-wasm".to_string(),
            abi_version: MODEL_GUEST_ABI_VERSION,
            input_schema_ref: "schema:fixture-classifier-in".to_string(),
            input_schema_version: "1.0.0".to_string(),
            output_schema_ref: "schema:fixture-classifier-out".to_string(),
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

    fn classifier_input_frame(features: [f32; 4]) -> Vec<u8> {
        let mut payload = Vec::with_capacity(16);
        for feature in features {
            payload.extend_from_slice(&feature.to_le_bytes());
        }
        encode_guest_frame(2, &[4], &payload)
    }

    fn classifier_execute_request(digest: &str, input_ref: &str) -> HostConnectorHostRequest {
        HostConnectorHostRequest {
            connector_id: MODEL_RUNTIME_CONNECTOR.to_string(),
            operation: MODEL_EXECUTE_OPERATION.to_string(),
            binding_id: "b".to_string(),
            target_family: "macos".to_string(),
            correlation_id: "c".to_string(),
            payload: json!({
                "model_ref": {
                    "model_id": "fixture.classifier",
                    "version": "1.0.0",
                    "digest": digest
                },
                "input_ref": input_ref,
                "policy_ref": "policy-1",
                "data_classification": "sensitive",
                "input_schema_ref": "schema:fixture-classifier-in",
                "input_schema_version": "1.0.0",
                "max_output_bytes": 4096
            }),
            cancel_requested: false,
        }
    }

    fn seeded_classifier_host() -> (ExactModelHostConnector, String) {
        let package = fixture_classifier_package();
        let digest = package.manifest.package_digest.clone();
        let pin = ExactModelPin {
            model_id: "fixture.classifier".to_string(),
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
        (host, digest)
    }

    #[test]
    fn classifier_fixture_computes_real_inference_not_a_pass_through() {
        let (mut host, digest) = seeded_classifier_host();
        let frame = classifier_input_frame([1.0, 2.0, -1.0, 4.0]);
        let input_ref = host.io.stage_model_input(&frame, 4096).expect("stage");
        let result = host
            .invoke(&classifier_execute_request(&digest, &input_ref))
            .expect("execute");
        let output = host
            .io
            .read_model_output(result.artifact_ref.as_deref().expect("artifact_ref"), 4096)
            .expect("read");
        assert_ne!(output, frame, "classifier output must not echo the input");
        let (dtype, dims, payload) = decode_guest_frame(&output).expect("decode output");
        assert_eq!(dtype, 3);
        assert_eq!(dims, vec![2]);
        assert_eq!(payload.len(), 8);
        let score = f32::from_le_bytes(payload[0..4].try_into().expect("score bytes"));
        let label = f32::from_le_bytes(payload[4..8].try_into().expect("label bytes"));
        // 0.5*1.0 - 0.25*2.0 + 1.0*-1.0 + 0.75*4.0 - 0.5 == 1.5
        assert!((score - 1.5).abs() < f32::EPSILON);
        assert!((label - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn classifier_fixture_yields_zero_label_below_threshold() {
        let (mut host, digest) = seeded_classifier_host();
        let frame = classifier_input_frame([-4.0, 0.0, 0.0, 0.0]);
        let input_ref = host.io.stage_model_input(&frame, 4096).expect("stage");
        let result = host
            .invoke(&classifier_execute_request(&digest, &input_ref))
            .expect("execute");
        let output = host
            .io
            .read_model_output(result.artifact_ref.as_deref().expect("artifact_ref"), 4096)
            .expect("read");
        let (_, _, payload) = decode_guest_frame(&output).expect("decode output");
        let score = f32::from_le_bytes(payload[0..4].try_into().expect("score bytes"));
        let label = f32::from_le_bytes(payload[4..8].try_into().expect("label bytes"));
        // 0.5*-4.0 - 0.5 == -2.5
        assert!((score - (-2.5)).abs() < f32::EPSILON);
        assert!(label.abs() < f32::EPSILON);
    }

    #[test]
    fn classifier_fixture_fails_closed_on_undersized_input_and_output() {
        let (mut host, digest) = seeded_classifier_host();
        let short_input_ref = host.io.stage_model_input(b"too-short", 64).expect("stage");
        assert_eq!(
            host.invoke(&classifier_execute_request(&digest, &short_input_ref))
                .expect_err("undersized input")
                .code,
            HostConnectorErrorCode::ResourceExhausted
        );

        let frame = classifier_input_frame([1.0, 1.0, 1.0, 1.0]);
        let input_ref = host.io.stage_model_input(&frame, 4096).expect("stage");
        let mut extras = serde_json::Map::new();
        extras.insert("max_output_bytes".to_string(), json!(10));
        let mut request = classifier_execute_request(&digest, &input_ref);
        if let Some(object) = request.payload.as_object_mut() {
            object.extend(extras);
        }
        assert_eq!(
            host.invoke(&request).expect_err("undersized output").code,
            HostConnectorErrorCode::ResourceExhausted
        );
    }
}
