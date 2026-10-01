//! Spec `138-governed-exact-model-execution`: host-staged I/O, package store,
//! and CPU-WASM model guest execution behind Spec 137 `model.execute`.

use crate::host_connector_dispatch::{
    HostConnectorError, HostConnectorErrorCode, HostConnectorHostRequest, HostConnectorHostResult,
    HostConnectorPort, MODEL_EXECUTE_OPERATION, MODEL_RUNTIME_CONNECTOR, ModelFailureReason,
    ModelRightsDenialDetail,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Governing spec id.
pub const GOVERNING_SPEC: &str = "138-governed-exact-model-execution";
/// First guest ABI version.
pub const MODEL_GUEST_ABI_VERSION: u16 = 1;
/// Placement for the CPU-WASM conformance baseline.
pub const PLACEMENT_WASM_CPU: &str = "wasm-cpu";
/// Guest export name.
pub const MODEL_EXECUTE_EXPORT: &str = "model_execute";
/// Guest ABI v2 buffer allocator export (`model_alloc(len) -> ptr`, Decision 105).
pub const MODEL_ALLOC_EXPORT: &str = "model_alloc";
/// Highest supported manifest `abi_version`: 1 = host places buffers at fixed
/// offsets; 2 = the guest allocates them via [`MODEL_ALLOC_EXPORT`]. The
/// little-endian frame format ([`MODEL_GUEST_ABI_VERSION`]) is unchanged.
pub const MAX_MODEL_ABI_VERSION: u16 = 2;

/// Model package manifest schema version (Spec 138 0.4.0, Decision 101).
pub const MODEL_PACKAGE_SCHEMA_VERSION: &str = "2.0.0";
/// Manifest schema version that adds optional `rights.derivation`
/// (Spec 138 0.8.0, Decision 107). Hosts accept both versions.
pub const MODEL_PACKAGE_SCHEMA_VERSION_DERIVATION: &str = "2.1.0";
/// The only accepted package signature algorithm.
pub const MODEL_SIGNATURE_ALG_ED25519: &str = "ed25519";

/// Commercial-use terms carried in signed package rights.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommercialUse {
    /// Commercial use permitted.
    Allowed,
    /// Commercial use permitted only under the license's conditions.
    Restricted,
    /// Commercial use not permitted.
    Prohibited,
}

impl CommercialUse {
    /// Stable wire value.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Allowed => "allowed",
            Self::Restricted => "restricted",
            Self::Prohibited => "prohibited",
        }
    }

    /// Permissiveness order `prohibited < restricted < allowed` (Decision 107).
    fn rank(self) -> u8 {
        match self {
            Self::Prohibited => 0,
            Self::Restricted => 1,
            Self::Allowed => 2,
        }
    }
}

/// How the application uses its models (app manifest `model_usage`,
/// Spec 138 0.8.0, Decision 107).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelUsage {
    /// Commercial use: only `allowed` / pin-acknowledged `restricted` packages.
    Commercial,
    /// Non-commercial use: `prohibited` packages are also accepted.
    NonCommercial,
}

/// How a derivative package was produced from its source (Decision 107).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DerivationKind {
    /// Format conversion (for example ONNX to a WASM runner package).
    Converted,
    /// Quantized from the source weights.
    Quantized,
    /// Fine-tuned from the source weights.
    FineTuned,
}

/// Signed provenance of a derivative package (`rights.derivation`, manifest
/// schema `2.1.0`). Every field is required.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelDerivation {
    /// How the package was derived.
    pub kind: DerivationKind,
    /// SHA-256 of the source artifact (hex, optionally `sha256:` prefixed).
    pub source_digest: String,
    /// SPDX license identifier of the source.
    pub source_license_id: String,
    /// Commercial-use terms of the source.
    pub source_commercial_use: CommercialUse,
    /// Source URL (identity is `source_digest`, never this URL).
    pub source_url: String,
}

/// Host-owned lifecycle status of a package (Decision 107).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackageStatus {
    /// Not listed in the host status map.
    Active,
    /// Runs normally but is flagged in the rights record and evidence.
    Deprecated,
    /// Fails closed with `package_revoked`.
    Revoked,
}

/// One host status-map entry for a package digest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageStatusEntry {
    /// `deprecated` or `revoked` (`active` is the same as no entry).
    pub status: PackageStatus,
    /// Host-supplied reason shown to the app.
    pub reason: String,
}

/// Verified rights of a package as the host and every execution report it
/// (Spec 138 0.8.0 FR-040).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelRightsRecord {
    /// Model identity.
    pub model_id: String,
    /// Semantic version.
    pub version: String,
    /// Package (manifest-bytes) digest.
    pub digest: String,
    /// Signed rights, unchanged.
    pub rights: ModelRights,
    /// Host package status (`revoked` only on a host query; executions fail).
    pub status: PackageStatus,
    /// Host status reason, when not `active`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_reason: Option<String>,
    /// Usage the rights were checked against.
    pub effective_usage: ModelUsage,
}

/// Signed model rights, exposed read-only to hosts/UIs unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelRights {
    /// SPDX license identifier.
    pub license_id: String,
    /// Attribution text a UI must be able to show.
    pub attribution: String,
    /// Redistribution terms summary.
    pub redistribution: String,
    /// Commercial-use terms.
    pub commercial_use: CommercialUse,
    /// Source URL of the model/weights (identity is the digest, never this URL).
    pub source_url: String,
    /// Derivation provenance (manifest schema `2.1.0` only, Decision 107).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub derivation: Option<ModelDerivation>,
}

/// Rights an application pin expects the signed package to carry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PinRights {
    /// Expected SPDX license identifier.
    pub license_id: String,
    /// Expected commercial-use terms.
    pub commercial_use: CommercialUse,
}

/// Exact app-manifest pin (Spec 044 `exact_model_dependencies`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExactModelPin {
    /// Model identity.
    pub model_id: String,
    /// Semantic version.
    pub version: String,
    /// SHA-256 of the exact signed `model.manifest.json` bytes (hex,
    /// optionally `sha256:` prefixed).
    pub digest: String,
    /// Whether offline execute is allowed when the package is cached.
    pub offline_allowed: bool,
    /// Execution target profile (`wasm-cpu`).
    pub target: String,
    /// Rights the signed package must carry.
    pub rights: PinRights,
    /// Optional narrowing to one host-trusted signer `key_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_id: Option<String>,
}

/// Versioned model package manifest (sidecar `model.manifest.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelPackageManifest {
    /// Schema version for this manifest document ([`MODEL_PACKAGE_SCHEMA_VERSION`]).
    pub schema_version: String,
    /// Model identity.
    pub model_id: String,
    /// Semantic version.
    pub version: String,
    /// Digest of the WASM bytes (hex).
    pub wasm_digest: String,
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
    /// Signed rights metadata.
    pub rights: ModelRights,
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

fn model_error(
    code: HostConnectorErrorCode,
    reason: ModelFailureReason,
    message: &str,
) -> HostConnectorError {
    HostConnectorError {
        code,
        reason: Some(reason),
        detail: None,
        message: message.to_string(),
    }
}

fn incompatible(reason: ModelFailureReason, message: &str) -> HostConnectorError {
    model_error(HostConnectorErrorCode::ModelIncompatible, reason, message)
}

/// Rights denial detail without package identity (filled in by
/// [`for_package`]).
fn denial(field: &str, expected: &str, actual: &str) -> ModelRightsDenialDetail {
    ModelRightsDenialDetail {
        model_id: None,
        version: None,
        digest: None,
        field: field.to_string(),
        expected: expected.to_string(),
        actual: actual.to_string(),
        effective_usage: None,
    }
}

fn with_detail(
    mut error: HostConnectorError,
    detail: ModelRightsDenialDetail,
) -> HostConnectorError {
    error.detail = Some(Box::new(detail));
    error
}

/// Attach package identity to an error's rights detail, if it has one.
fn for_package(
    mut error: HostConnectorError,
    manifest: &ModelPackageManifest,
    digest: &str,
) -> HostConnectorError {
    if let Some(detail) = error.detail.as_mut() {
        detail.model_id = Some(manifest.model_id.clone());
        detail.version = Some(manifest.version.clone());
        detail.digest = Some(digest.to_string());
    }
    error
}

impl ModelPackageManifest {
    /// Fail closed if required governance fields are missing or empty.
    ///
    /// # Errors
    ///
    /// Returns `model_incompatible` with `rights_incomplete`,
    /// `manifest_invalid`, or `target_unsupported`.
    pub fn validate(&self) -> Result<(), HostConnectorError> {
        let mut rights = vec![
            ("rights.license_id", self.rights.license_id.as_str()),
            ("rights.attribution", self.rights.attribution.as_str()),
            ("rights.redistribution", self.rights.redistribution.as_str()),
            ("rights.source_url", self.rights.source_url.as_str()),
        ];
        if let Some(derivation) = &self.rights.derivation {
            rights.extend([
                (
                    "rights.derivation.source_digest",
                    derivation.source_digest.as_str(),
                ),
                (
                    "rights.derivation.source_license_id",
                    derivation.source_license_id.as_str(),
                ),
                (
                    "rights.derivation.source_url",
                    derivation.source_url.as_str(),
                ),
            ]);
        }
        if let Some((field, value)) = rights.iter().find(|(_, value)| value.trim().is_empty()) {
            return Err(with_detail(
                incompatible(
                    ModelFailureReason::RightsIncomplete,
                    "model manifest rights are incomplete",
                ),
                denial(field, "non-empty", value),
            ));
        }
        let required = [
            ("wasm_digest", self.wasm_digest.as_str()),
            ("executable_format", self.executable_format.as_str()),
            ("input_schema_ref", self.input_schema_ref.as_str()),
            ("output_schema_ref", self.output_schema_ref.as_str()),
        ];
        for (name, value) in required {
            if value.trim().is_empty() {
                return Err(incompatible(
                    ModelFailureReason::ManifestInvalid,
                    &format!("model manifest missing required field {name}"),
                ));
            }
        }
        let schema_supported = self.schema_version == MODEL_PACKAGE_SCHEMA_VERSION
            || self.schema_version == MODEL_PACKAGE_SCHEMA_VERSION_DERIVATION;
        let derivation_allowed = self.rights.derivation.is_none()
            || self.schema_version == MODEL_PACKAGE_SCHEMA_VERSION_DERIVATION;
        if !schema_supported
            || !derivation_allowed
            || self
                .rights
                .derivation
                .as_ref()
                .is_some_and(|derivation| !is_sha256_hex(&derivation.source_digest))
            || self.abi_version == 0
            || self.abi_version > MAX_MODEL_ABI_VERSION
            || self.max_memory_bytes == 0
            || self.max_fuel == 0
            || self.max_input_bytes == 0
            || self.max_output_bytes == 0
            || self.max_execution_ms == 0
        {
            return Err(incompatible(
                ModelFailureReason::ManifestInvalid,
                "model manifest schema version, resource limits, or ABI are invalid",
            ));
        }
        if !self
            .supported_profiles
            .iter()
            .any(|profile| profile == PLACEMENT_WASM_CPU)
        {
            return Err(incompatible(
                ModelFailureReason::TargetUnsupported,
                "model manifest does not support wasm-cpu",
            ));
        }
        if let Some(derivation) = &self.rights.derivation
            && self.rights.commercial_use.rank() > derivation.source_commercial_use.rank()
        {
            return Err(with_detail(
                incompatible(
                    ModelFailureReason::RightsInconsistent,
                    "package commercial_use is more permissive than its derivation source",
                ),
                denial(
                    "rights.commercial_use",
                    &format!(
                        "no more permissive than {}",
                        derivation.source_commercial_use.as_str()
                    ),
                    self.rights.commercial_use.as_str(),
                ),
            ));
        }
        Ok(())
    }
}

fn is_sha256_hex(value: &str) -> bool {
    let hex = normalize_digest(value);
    hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Detached package signature (`model.sig.json`) over the exact
/// `model.manifest.json` bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelPackageSignature {
    /// Must be [`MODEL_SIGNATURE_ALG_ED25519`].
    pub alg: String,
    /// Signer key id ([`model_signing_key_id`]).
    pub key_id: String,
    /// Lowercase hex of the 64-byte Ed25519 signature.
    pub signature: String,
}

/// `key_id` for an Ed25519 public key: `ed25519:` + hex SHA-256 of the raw
/// 32-byte key (Decision 101).
#[must_use]
pub fn model_signing_key_id(public_key: &[u8; 32]) -> String {
    format!("{MODEL_SIGNATURE_ALG_ED25519}:{}", digest_hex(public_key))
}

/// Sign exact manifest bytes with a raw 32-byte Ed25519 secret key
/// (package tooling and conformance fixtures).
#[must_use]
pub fn sign_model_manifest(secret_key: &[u8; 32], manifest_bytes: &[u8]) -> ModelPackageSignature {
    use ed25519_dalek::{Signer, SigningKey};
    let signing = SigningKey::from_bytes(secret_key);
    ModelPackageSignature {
        alg: MODEL_SIGNATURE_ALG_ED25519.to_string(),
        key_id: model_signing_key_id(&signing.verifying_key().to_bytes()),
        signature: hex_encode(&signing.sign(manifest_bytes).to_bytes()),
    }
}

/// Host-owned set of trusted Ed25519 model-signing keys. Application
/// manifests can never add trust (Decision 101).
#[derive(Debug, Clone, Default)]
pub struct TrustedModelKeys {
    keys: HashMap<String, ed25519_dalek::VerifyingKey>,
}

impl TrustedModelKeys {
    /// Empty set (trusts nothing).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Trust a raw 32-byte Ed25519 public key; returns its `key_id`.
    ///
    /// # Errors
    ///
    /// Returns `model_incompatible` / `key_untrusted` for a non-curve point.
    pub fn trust(&mut self, public_key: &[u8; 32]) -> Result<String, HostConnectorError> {
        let key = ed25519_dalek::VerifyingKey::from_bytes(public_key).map_err(|_| {
            incompatible(
                ModelFailureReason::KeyUntrusted,
                "model signing public key is invalid",
            )
        })?;
        let key_id = model_signing_key_id(public_key);
        self.keys.insert(key_id.clone(), key);
        Ok(key_id)
    }

    fn verify(
        &self,
        signature: &ModelPackageSignature,
        manifest_bytes: &[u8],
    ) -> Result<(), HostConnectorError> {
        use ed25519_dalek::Verifier;
        if signature.alg != MODEL_SIGNATURE_ALG_ED25519 {
            return Err(incompatible(
                ModelFailureReason::SignatureInvalid,
                "unsupported model signature algorithm",
            ));
        }
        let key = self.keys.get(&signature.key_id).ok_or_else(|| {
            incompatible(
                ModelFailureReason::KeyUntrusted,
                "model signing key is not host-trusted",
            )
        })?;
        let bytes = hex_decode(&signature.signature)
            .and_then(|bytes| <[u8; 64]>::try_from(bytes).ok())
            .ok_or_else(|| {
                incompatible(
                    ModelFailureReason::SignatureInvalid,
                    "model signature is malformed",
                )
            })?;
        key.verify(
            manifest_bytes,
            &ed25519_dalek::Signature::from_bytes(&bytes),
        )
        .map_err(|_| {
            incompatible(
                ModelFailureReason::SignatureInvalid,
                "model signature verification failed",
            )
        })
    }
}

/// Provisioned model package in the host-owned verified digest store.
#[derive(Debug, Clone)]
pub struct VerifiedModelPackage {
    /// Parsed manifest.
    pub manifest: ModelPackageManifest,
    /// Exact signed manifest bytes (their SHA-256 is the pin digest).
    pub manifest_bytes: Vec<u8>,
    /// WASM bytes.
    pub wasm: Vec<u8>,
}

impl VerifiedModelPackage {
    /// Re-hash cached bytes against the pinned digest (every execute).
    fn recheck(&self, pinned_digest: &str) -> Result<(), HostConnectorError> {
        if digest_hex(&self.manifest_bytes) != normalize_digest(pinned_digest)
            || digest_hex(&self.wasm) != normalize_digest(&self.manifest.wasm_digest)
        {
            return Err(incompatible(
                ModelFailureReason::DigestMismatch,
                "cached model package bytes no longer match the pinned digest",
            ));
        }
        Ok(())
    }
}

/// Content-addressed model package store keyed by manifest-bytes digest
/// (Spec 080-shaped; Spec 526 lifecycle later). Packages enter only through
/// [`ExactModelHostConnector::register_package`].
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

    /// Validate and insert a package keyed by SHA-256 of its manifest bytes.
    ///
    /// # Errors
    ///
    /// Returns `model_incompatible` when manifest validation fails or the
    /// WASM digest does not match its bytes.
    fn insert_verified(
        &mut self,
        package: VerifiedModelPackage,
    ) -> Result<String, HostConnectorError> {
        package.manifest.validate()?;
        if normalize_digest(&package.manifest.wasm_digest) != digest_hex(&package.wasm) {
            return Err(incompatible(
                ModelFailureReason::DigestMismatch,
                "model wasm digest mismatch",
            ));
        }
        let key = digest_hex(&package.manifest_bytes);
        self.by_digest.insert(key.clone(), package);
        Ok(key)
    }

    /// Resolve by pin digest without network.
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
                reason: None,
                detail: None,
                message: "model package not present in verified cache".to_string(),
            })
    }

    /// Signed rights of a cached package, for host/UI display.
    #[must_use]
    pub fn rights(&self, digest: &str) -> Option<&ModelRights> {
        self.by_digest
            .get(&normalize_digest(digest))
            .map(|package| &package.manifest.rights)
    }
}

/// Host-staged tensor buffers (not Spec 526 entries).
#[derive(Debug, Default)]
pub struct ModelIoStore {
    inputs: HashMap<String, Vec<u8>>,
    outputs: HashMap<String, Vec<u8>>,
    artifacts: HashMap<String, Vec<u8>>,
    next_input: u64,
    next_output: u64,
    next_artifact: u64,
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
                reason: None,
                detail: None,
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
                reason: None,
                detail: None,
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
                reason: None,
                detail: None,
                message: "output_ref missing or expired".to_string(),
            })?;
        if bytes.len() > max_bytes {
            return Err(HostConnectorError {
                code: HostConnectorErrorCode::InputLimitExceeded,
                reason: None,
                detail: None,
                message: "output exceeds read ceiling".to_string(),
            });
        }
        Ok(bytes.clone())
    }

    /// Stage bounded bytes → multi-read `artifact_ref` (Spec 140 / Spec 138 0.2.0).
    ///
    /// The ref is opaque (never a path or URL) and stays readable until
    /// [`Self::drop_ref`] or [`Self::shutdown`], so runtime-owned retries can
    /// re-read it. Model `input_ref` keeps its single-consume rule.
    ///
    /// # Errors
    ///
    /// Returns `input_limit_exceeded` when empty or over `max_bytes`.
    pub fn stage_artifact(
        &mut self,
        bytes: &[u8],
        max_bytes: usize,
    ) -> Result<String, HostConnectorError> {
        if bytes.is_empty() || bytes.len() > max_bytes {
            return Err(HostConnectorError {
                code: HostConnectorErrorCode::InputLimitExceeded,
                reason: None,
                detail: None,
                message: "staged artifact empty or exceeds ceiling".to_string(),
            });
        }
        self.next_artifact = self.next_artifact.saturating_add(1);
        let id = format!("artifact-{}", self.next_artifact);
        self.artifacts.insert(id.clone(), bytes.to_vec());
        Ok(id)
    }

    /// Runtime-mediated bounded read of an `artifact_ref`. Repeatable.
    ///
    /// # Errors
    ///
    /// Returns `unavailable` when missing, dropped, or invalidated by
    /// shutdown; `input_limit_exceeded` when the artifact exceeds `max_bytes`.
    pub fn read_artifact(
        &self,
        artifact_ref: &str,
        max_bytes: usize,
    ) -> Result<Vec<u8>, HostConnectorError> {
        let bytes = self
            .artifacts
            .get(artifact_ref)
            .ok_or_else(|| HostConnectorError {
                code: HostConnectorErrorCode::Unavailable,
                reason: None,
                detail: None,
                message: "artifact_ref missing or expired".to_string(),
            })?;
        if bytes.len() > max_bytes {
            return Err(HostConnectorError {
                code: HostConnectorErrorCode::InputLimitExceeded,
                reason: None,
                detail: None,
                message: "artifact exceeds read ceiling".to_string(),
            });
        }
        Ok(bytes.clone())
    }

    /// Drop an input, output, or artifact ref.
    pub fn drop_ref(&mut self, reference: &str) {
        self.inputs.remove(reference);
        self.outputs.remove(reference);
        self.artifacts.remove(reference);
    }

    /// Invalidate every staged ref (runtime shutdown).
    pub fn shutdown(&mut self) {
        self.inputs.clear();
        self.outputs.clear();
        self.artifacts.clear();
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

/// Engine that executes the `wasm-cpu` model guest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelEngine {
    /// Cranelift JIT (desktop/server native hosts).
    Wasmtime,
    /// `wasmi` interpreter (JIT-forbidden targets such as iOS; Decision 104).
    /// Runs in fuel slices so cancellation and deadlines interrupt mid-run.
    Wasmi,
}

impl Default for ModelEngine {
    /// Wasmtime when compiled in, otherwise the `wasmi` interpreter.
    #[cfg(feature = "wasmtime-executor")]
    fn default() -> Self {
        Self::Wasmtime
    }

    /// Wasmtime when compiled in, otherwise the `wasmi` interpreter.
    #[cfg(not(feature = "wasmtime-executor"))]
    fn default() -> Self {
        Self::Wasmi
    }
}

/// Host-configured ceilings a package's declared limits must fit within
/// (Decision 104). Registration fails closed with `host_limit_exceeded`;
/// execution uses manifest ∩ host ∩ per-call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostModelLimits {
    /// Max `model.manifest.json` + `model.wasm` bytes.
    pub max_package_bytes: u64,
    /// Max guest linear memory bytes.
    pub max_memory_bytes: u64,
    /// Max fuel (engine-relative units) per execution.
    pub max_fuel: u64,
}

impl Default for HostModelLimits {
    /// Generous desktop defaults; mobile hosts pass tighter ceilings.
    fn default() -> Self {
        Self {
            max_package_bytes: 256 * 1024 * 1024,
            max_memory_bytes: 1024 * 1024 * 1024,
            max_fuel: 50_000_000_000,
        }
    }
}

/// Fuel granted per `wasmi` slice between cancellation/deadline checks.
pub const WASMI_FUEL_SLICE: u64 = 1_000_000;

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
    /// Host-owned trusted model-signing keys.
    pub trusted_keys: TrustedModelKeys,
    /// Guest execution engine.
    pub engine: ModelEngine,
    /// Host ceilings (Decision 104).
    pub host_limits: HostModelLimits,
    /// Caller-managed cancellation flag, observed between `wasmi` fuel slices.
    /// The connector never clears it; the owner resets it per execution.
    pub cancel: Arc<AtomicBool>,
    /// App manifest `model_usage`; required whenever pins exist (Decision 107).
    pub model_usage: Option<ModelUsage>,
    /// Host tightening: when true the effective usage is always `commercial`.
    /// A host can never relax an app's usage (Decision 107).
    pub host_requires_commercial: bool,
    /// Host-owned package status map keyed by normalized manifest digest; see
    /// [`ExactModelHostConnector::set_package_status`].
    package_status: HashMap<String, PackageStatusEntry>,
    /// Engines and compiled guests reused across executions.
    compiled: CompiledGuests,
}

impl ExactModelHostConnector {
    /// Construct with pins, host-trusted signing keys, and empty stores.
    #[must_use]
    pub fn new(pins: Vec<ExactModelPin>, trusted_keys: TrustedModelKeys) -> Self {
        Self {
            pins,
            packages: ModelPackageStore::new(),
            io: ModelIoStore::new(),
            policies: HashMap::new(),
            offline_mode: true,
            trusted_keys,
            engine: ModelEngine::default(),
            host_limits: HostModelLimits::default(),
            cancel: Arc::new(AtomicBool::new(false)),
            model_usage: None,
            host_requires_commercial: false,
            package_status: HashMap::new(),
            compiled: CompiledGuests::default(),
        }
    }

    /// Replace the host-owned package status map (digest → status). Takes
    /// effect at the next registration or execute (Decision 107).
    pub fn set_package_status(
        &mut self,
        entries: impl IntoIterator<Item = (String, PackageStatusEntry)>,
    ) {
        self.package_status = entries
            .into_iter()
            .map(|(digest, entry)| (normalize_digest(&digest), entry))
            .collect();
    }

    /// The usage rights are checked against: `commercial` when the host
    /// requires it, otherwise the app's declared `model_usage`.
    ///
    /// # Errors
    ///
    /// Returns `model_incompatible` / `usage_undeclared` when the app
    /// declares no `model_usage`.
    pub fn effective_usage(&self) -> Result<ModelUsage, HostConnectorError> {
        let declared = self.model_usage.ok_or_else(|| {
            with_detail(
                incompatible(
                    ModelFailureReason::UsageUndeclared,
                    "app declares exact_model_dependencies but no model_usage",
                ),
                denial("model_usage", "commercial|non_commercial", "undeclared"),
            )
        })?;
        Ok(if self.host_requires_commercial {
            ModelUsage::Commercial
        } else {
            declared
        })
    }

    /// Verified rights record of a registered package for host/UI display,
    /// including `revoked` status. `None` when the digest is not registered or
    /// the app declares no `model_usage`.
    #[must_use]
    pub fn model_rights_record(&self, digest: &str) -> Option<ModelRightsRecord> {
        let key = normalize_digest(digest);
        let package = self.packages.by_digest.get(&key)?;
        let usage = self.effective_usage().ok()?;
        let entry = self.package_status.get(&key);
        Some(rights_record(&package.manifest, &key, usage, entry))
    }

    /// Usage policy and package status for a verified package (registration
    /// and every execute). Returns the rights record on success.
    fn check_rights_and_status(
        &self,
        manifest: &ModelPackageManifest,
        digest: &str,
    ) -> Result<ModelRightsRecord, HostConnectorError> {
        let usage = self
            .effective_usage()
            .map_err(|error| for_package(error, manifest, digest))?;
        // `restricted` passes: the exact pin match (FR-020) already required
        // the pin to declare it.
        if manifest.rights.commercial_use == CommercialUse::Prohibited
            && usage == ModelUsage::Commercial
        {
            let mut detail = denial(
                "rights.commercial_use",
                "allowed|restricted",
                CommercialUse::Prohibited.as_str(),
            );
            detail.effective_usage = Some(usage);
            return Err(for_package(
                with_detail(
                    incompatible(
                        ModelFailureReason::RightsPolicyDenied,
                        "package commercial_use is not permitted for the effective model_usage",
                    ),
                    detail,
                ),
                manifest,
                digest,
            ));
        }
        let entry = self.package_status.get(digest);
        if entry.is_some_and(|entry| entry.status == PackageStatus::Revoked) {
            let mut detail = denial("status", "active|deprecated", "revoked");
            detail.effective_usage = Some(usage);
            return Err(for_package(
                with_detail(
                    model_error(
                        HostConnectorErrorCode::ModelUnavailable,
                        ModelFailureReason::PackageRevoked,
                        "the host package status map marks this package revoked",
                    ),
                    detail,
                ),
                manifest,
                digest,
            ));
        }
        Ok(rights_record(manifest, digest, usage, entry))
    }

    /// Verify and admit a signed package into the host cache (Decision 101):
    /// signature by a host-trusted key over the exact manifest bytes, manifest
    /// digest equal to exactly one declared pin, WASM digest, rights, target,
    /// and limits. Returns the pin digest the package is cached under.
    ///
    /// # Errors
    ///
    /// Returns `model_unavailable` / `model_incompatible` with a stable
    /// [`ModelFailureReason`].
    pub fn register_package(
        &mut self,
        manifest_bytes: &[u8],
        wasm: Vec<u8>,
        signature_bytes: &[u8],
    ) -> Result<String, HostConnectorError> {
        let signature: ModelPackageSignature =
            serde_json::from_slice(signature_bytes).map_err(|_| {
                incompatible(
                    ModelFailureReason::SignatureInvalid,
                    "model signature document is malformed",
                )
            })?;
        self.trusted_keys.verify(&signature, manifest_bytes)?;
        let digest = digest_hex(manifest_bytes);
        let pin = self
            .pins
            .iter()
            .find(|pin| normalize_digest(&pin.digest) == digest)
            .ok_or_else(|| {
                model_error(
                    HostConnectorErrorCode::ModelUnavailable,
                    ModelFailureReason::PinMismatch,
                    "signed package digest does not match an exact_model_dependencies pin",
                )
            })?;
        self.require_unambiguous(pin)?;
        let manifest: ModelPackageManifest =
            serde_json::from_slice(manifest_bytes).map_err(|_| {
                incompatible(
                    ModelFailureReason::ManifestInvalid,
                    "model manifest is malformed or has unknown fields",
                )
            })?;
        check_pin_against_manifest(pin, &signature, &manifest)
            .map_err(|error| for_package(error, &manifest, &digest))?;
        self.check_rights_and_status(&manifest, &digest)?;
        check_host_limits(
            &self.host_limits,
            &manifest,
            manifest_bytes.len(),
            wasm.len(),
        )?;
        // Compile once at registration so execute deadlines cover guest
        // execution only. Keyed by the bytes' own hash, so the cache can never
        // serve a module for other bytes; a module that fails to compile is
        // not cached and still fails closed at execute.
        self.compiled.warm(self.engine, &digest_hex(&wasm), &wasm);
        self.packages.insert_verified(VerifiedModelPackage {
            manifest,
            manifest_bytes: manifest_bytes.to_vec(),
            wasm,
        })
    }

    /// Signed rights of a registered package, for host/UI display.
    #[must_use]
    pub fn model_rights(&self, digest: &str) -> Option<&ModelRights> {
        self.packages.rights(digest)
    }

    fn require_unambiguous(&self, pin: &ExactModelPin) -> Result<(), HostConnectorError> {
        let same_identity = self
            .pins
            .iter()
            .filter(|other| other.model_id == pin.model_id && other.version == pin.version)
            .count();
        if same_identity > 1 {
            return Err(incompatible(
                ModelFailureReason::PinAmbiguous,
                "more than one exact_model_dependencies pin names this model id and version",
            ));
        }
        Ok(())
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
            .ok_or_else(|| {
                model_error(
                    HostConnectorErrorCode::ModelUnavailable,
                    ModelFailureReason::PinMismatch,
                    "model_ref does not match an exact_model_dependencies pin",
                )
            })
    }
}

fn check_host_limits(
    host: &HostModelLimits,
    manifest: &ModelPackageManifest,
    manifest_len: usize,
    wasm_len: usize,
) -> Result<(), HostConnectorError> {
    let package_bytes = (manifest_len as u64).saturating_add(wasm_len as u64);
    if package_bytes > host.max_package_bytes
        || manifest.max_memory_bytes > host.max_memory_bytes
        || manifest.max_fuel > host.max_fuel
    {
        return Err(incompatible(
            ModelFailureReason::HostLimitExceeded,
            "package size or declared limits exceed the host ceilings",
        ));
    }
    Ok(())
}

fn check_pin_against_manifest(
    pin: &ExactModelPin,
    signature: &ModelPackageSignature,
    manifest: &ModelPackageManifest,
) -> Result<(), HostConnectorError> {
    if pin
        .key_id
        .as_ref()
        .is_some_and(|key_id| key_id != &signature.key_id)
    {
        return Err(incompatible(
            ModelFailureReason::KeyUntrusted,
            "package signer is not the key the pin requires",
        ));
    }
    if manifest.model_id != pin.model_id || manifest.version != pin.version {
        return Err(incompatible(
            ModelFailureReason::PinMismatch,
            "signed package identity does not match its pin",
        ));
    }
    manifest.validate()?;
    if pin.target != PLACEMENT_WASM_CPU
        || !manifest
            .supported_profiles
            .iter()
            .any(|profile| profile == &pin.target)
    {
        return Err(incompatible(
            ModelFailureReason::TargetUnsupported,
            "pin target is not supported by the package or the wasm-cpu executor",
        ));
    }
    let mismatch = if manifest.rights.license_id == pin.rights.license_id {
        None
    } else {
        Some(denial(
            "rights.license_id",
            &pin.rights.license_id,
            &manifest.rights.license_id,
        ))
    }
    .or_else(|| {
        (manifest.rights.commercial_use != pin.rights.commercial_use).then(|| {
            denial(
                "rights.commercial_use",
                pin.rights.commercial_use.as_str(),
                manifest.rights.commercial_use.as_str(),
            )
        })
    });
    if let Some(detail) = mismatch {
        return Err(with_detail(
            incompatible(
                ModelFailureReason::RightsMismatch,
                "signed package rights differ from the rights the pin declares",
            ),
            detail,
        ));
    }
    Ok(())
}

fn rights_record(
    manifest: &ModelPackageManifest,
    digest: &str,
    effective_usage: ModelUsage,
    entry: Option<&PackageStatusEntry>,
) -> ModelRightsRecord {
    ModelRightsRecord {
        model_id: manifest.model_id.clone(),
        version: manifest.version.clone(),
        digest: digest.to_string(),
        rights: manifest.rights.clone(),
        status: entry.map_or(PackageStatus::Active, |entry| entry.status),
        status_reason: entry.map(|entry| entry.reason.clone()),
        effective_usage,
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
                reason: None,
                detail: None,
                message: "ExactModelHostConnector only serves model.execute".to_string(),
            });
        }
        if request.cancel_requested {
            return Err(HostConnectorError {
                code: HostConnectorErrorCode::Cancelled,
                reason: None,
                detail: None,
                message: "model.execute cancelled before invoke".to_string(),
            });
        }

        let payload: ModelExecutePayload = serde_json::from_value(request.payload.clone())
            .map_err(|_| HostConnectorError {
                code: HostConnectorErrorCode::InvalidInput,
                reason: None,
                detail: None,
                message: "model.execute payload failed schema validation".to_string(),
            })?;

        let pin = self.require_pin(&payload.model_ref)?;
        if self.offline_mode && !pin.offline_allowed {
            return Err(HostConnectorError {
                code: HostConnectorErrorCode::ModelUnavailable,
                reason: None,
                detail: None,
                message: "pin does not allow offline execution".to_string(),
            });
        }

        let policy = self
            .policies
            .get(&payload.policy_ref)
            .ok_or_else(|| HostConnectorError {
                code: HostConnectorErrorCode::PolicyDenied,
                reason: None,
                detail: None,
                message: "policy_ref is not activated".to_string(),
            })?;
        if !policy
            .allowed_classifications
            .iter()
            .any(|class| class == &payload.data_classification)
        {
            return Err(HostConnectorError {
                code: HostConnectorErrorCode::PolicyDenied,
                reason: None,
                detail: None,
                message: "data_classification denied by policy".to_string(),
            });
        }

        let package = self.packages.resolve_offline(&payload.model_ref.digest)?;
        package.recheck(&payload.model_ref.digest)?;
        let evidence = self.check_rights_and_status(
            &package.manifest,
            &normalize_digest(&payload.model_ref.digest),
        )?;
        if package.manifest.model_id != payload.model_ref.model_id
            || package.manifest.version != payload.model_ref.version
        {
            return Err(HostConnectorError {
                code: HostConnectorErrorCode::ModelIncompatible,
                reason: None,
                detail: None,
                message: "cached package identity does not match model_ref".to_string(),
            });
        }
        if package.manifest.input_schema_ref != payload.input_schema_ref
            || package.manifest.input_schema_version != payload.input_schema_version
        {
            return Err(HostConnectorError {
                code: HostConnectorErrorCode::ModelIncompatible,
                reason: None,
                detail: None,
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
                reason: None,
                detail: None,
                message: "output ceiling is zero after policy intersection".to_string(),
            });
        }

        let input = self.io.take_input(&payload.input_ref)?;
        if input.len() as u64 > package.manifest.max_input_bytes {
            return Err(HostConnectorError {
                code: HostConnectorErrorCode::ResourceExhausted,
                reason: None,
                detail: None,
                message: "input exceeds model manifest ceiling".to_string(),
            });
        }

        let memory = payload
            .max_memory_bytes
            .unwrap_or(package.manifest.max_memory_bytes)
            .min(package.manifest.max_memory_bytes)
            .min(self.host_limits.max_memory_bytes);
        let fuel = payload
            .max_fuel
            .unwrap_or(package.manifest.max_fuel)
            .min(package.manifest.max_fuel)
            .min(self.host_limits.max_fuel);
        let timeout = Duration::from_millis(
            payload
                .timeout_ms
                .unwrap_or(package.manifest.max_execution_ms)
                .min(package.manifest.max_execution_ms),
        );

        let _ = &payload.feature_metadata;
        let guest_limits = GuestLimits {
            memory,
            fuel,
            max_output: call_max_out,
            abi: package.manifest.abi_version,
        };
        let started = Instant::now();
        // `recheck` above proved the bytes still hash to `wasm_digest`.
        let key = normalize_digest(&package.manifest.wasm_digest);
        let output = match self.engine {
            ModelEngine::Wasmtime => execute_wasm_cpu_model(
                &mut self.compiled,
                &key,
                &package.wasm,
                &input,
                &guest_limits,
            )?,
            ModelEngine::Wasmi => execute_wasmi_model(
                &mut self.compiled,
                &key,
                &package.wasm,
                &input,
                &guest_limits,
                &SliceControl {
                    cancel: &self.cancel,
                    deadline: started + timeout,
                },
            )?,
        };
        if started.elapsed() > timeout {
            return Err(HostConnectorError {
                code: HostConnectorErrorCode::Timeout,
                reason: None,
                detail: None,
                message: "model.execute exceeded timeout".to_string(),
            });
        }

        let output_ref = self.io.put_output(output);
        Ok(HostConnectorHostResult {
            artifact_ref: Some(output_ref),
            permission_state: None,
            model_evidence: Some(Box::new(evidence)),
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
            reason: None,
            detail: None,
            message: "guest frame too short".to_string(),
        });
    }
    let abi = u16::from_le_bytes([bytes[0], bytes[1]]);
    if abi != MODEL_GUEST_ABI_VERSION {
        return Err(HostConnectorError {
            code: HostConnectorErrorCode::ModelIncompatible,
            reason: None,
            detail: None,
            message: "unsupported guest ABI version".to_string(),
        });
    }
    let dtype = bytes[2];
    let rank = bytes[3] as usize;
    let header = 4 + rank * 4 + 4;
    if bytes.len() < header {
        return Err(HostConnectorError {
            code: HostConnectorErrorCode::InvalidInput,
            reason: None,
            detail: None,
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
            reason: None,
            detail: None,
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

fn hex_decode(value: &str) -> Option<Vec<u8>> {
    if !value.len().is_multiple_of(2) {
        return None;
    }
    (0..value.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(value.get(index..index + 2)?, 16).ok())
        .collect()
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
        reason: None,
        detail: None,
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
    compiled: &mut CompiledGuests,
    key: &str,
    wasm: &[u8],
    input: &[u8],
    guest: &GuestLimits,
) -> Result<Vec<u8>, HostConnectorError> {
    use wasmtime::{Linker, Store, StoreLimitsBuilder};
    let (max_memory_bytes, max_fuel, max_output_bytes) =
        (guest.memory, guest.fuel, guest.max_output);

    let (engine, module) = compiled.wasmtime(key, wasm)?;

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

    let out_cap = output_capacity(max_output_bytes);
    let (in_ptr, out_ptr) = place_wasmtime_buffers(
        &mut store,
        &instance,
        memory,
        input.len(),
        out_cap,
        guest.abi,
    )?;
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

/// Engines and compiled guests reused across executions, keyed by the
/// verified `wasm_digest` (#1591): compiling a multi-megabyte guest such as
/// the ONNX runner on every call dominated latency. Every execution still
/// gets a fresh `Store` and instance, so no guest state crosses calls.
#[derive(Default)]
struct CompiledGuests {
    #[cfg(feature = "wasmtime-executor")]
    wasmtime: Option<(wasmtime::Engine, HashMap<String, wasmtime::Module>)>,
    #[cfg(feature = "wasmi-executor")]
    wasmi: Option<(wasmi::Engine, HashMap<String, wasmi::Module>)>,
}

impl CompiledGuests {
    fn warm(&mut self, engine: ModelEngine, key: &str, wasm: &[u8]) {
        let _ = match engine {
            ModelEngine::Wasmtime => self.wasmtime(key, wasm).is_ok(),
            ModelEngine::Wasmi => self.wasmi(key, wasm).is_ok(),
        };
    }

    #[cfg(not(feature = "wasmtime-executor"))]
    #[allow(clippy::unused_self, clippy::unnecessary_wraps)]
    fn wasmtime(&mut self, _key: &str, _wasm: &[u8]) -> Result<(), HostConnectorError> {
        Ok(())
    }

    #[cfg(not(feature = "wasmi-executor"))]
    #[allow(clippy::unused_self, clippy::unnecessary_wraps)]
    fn wasmi(&mut self, _key: &str, _wasm: &[u8]) -> Result<(), HostConnectorError> {
        Ok(())
    }

    #[cfg(feature = "wasmtime-executor")]
    fn wasmtime(
        &mut self,
        key: &str,
        wasm: &[u8],
    ) -> Result<(wasmtime::Engine, wasmtime::Module), HostConnectorError> {
        let (engine, modules) = self.wasmtime.get_or_insert_with(|| {
            let mut config = wasmtime::Config::new();
            config.consume_fuel(true);
            // Engine::new only fails on illegal config; consume_fuel config is always legal.
            #[allow(clippy::unwrap_used)]
            let engine = wasmtime::Engine::new(&config).unwrap();
            (engine, HashMap::new())
        });
        if let Some(module) = modules.get(key) {
            return Ok((engine.clone(), module.clone()));
        }
        let Ok(module) = wasmtime::Module::new(engine, wasm) else {
            return Err(model_host_err(
                HostConnectorErrorCode::ModelIncompatible,
                "model wasm failed validation",
            ));
        };
        modules.insert(key.to_string(), module.clone());
        Ok((engine.clone(), module))
    }

    #[cfg(feature = "wasmi-executor")]
    fn wasmi(
        &mut self,
        key: &str,
        wasm: &[u8],
    ) -> Result<(wasmi::Engine, wasmi::Module), HostConnectorError> {
        let (engine, modules) = self.wasmi.get_or_insert_with(|| {
            let mut config = wasmi::Config::default();
            config.consume_fuel(true);
            // Fixed-width SIMD: the ONNX runner guest ships as a simd128
            // build (Decision 106); wasmtime enables it by default.
            config.wasm_simd(true);
            // Eager translation: lazy mode charges per-function compile fuel
            // mid-call and reports running out of it as a non-resumable
            // error, which breaks fuel slicing for large guests (#1591).
            config.compilation_mode(wasmi::CompilationMode::Eager);
            (wasmi::Engine::new(&config), HashMap::new())
        });
        if let Some(module) = modules.get(key) {
            return Ok((engine.clone(), module.clone()));
        }
        let Ok(module) = wasmi::Module::new(engine, wasm) else {
            return Err(model_error_plain(
                HostConnectorErrorCode::ModelIncompatible,
                "model wasm failed validation",
            ));
        };
        modules.insert(key.to_string(), module.clone());
        Ok((engine.clone(), module))
    }
}

/// Guest ceilings after manifest ∩ host ∩ per-call intersection.
struct GuestLimits {
    memory: u64,
    fuel: u64,
    max_output: u64,
    /// Manifest `abi_version` (1 = fixed offsets, 2 = guest `model_alloc`).
    abi: u16,
}

/// Output capacity handed to the guest, clamped to `i32`.
#[allow(clippy::cast_possible_truncation)]
fn output_capacity(max_output: u64) -> i32 {
    i32::try_from(max_output.min(u64::from(i32::MAX as u32))).unwrap_or(i32::MAX)
}

/// Guest ABI v1: host-chosen fixed offsets; returns `(in_ptr, out_ptr, end)`.
fn v1_placement(input_len: usize, out_cap: i32) -> (i32, i32, u64) {
    let in_ptr = 64_i32;
    let out_ptr = in_ptr + i32::try_from(input_len).unwrap_or(i32::MAX) + 64;
    let end = u64::try_from(out_ptr).unwrap_or(0) + u64::try_from(out_cap).unwrap_or(0);
    (in_ptr, out_ptr, end)
}

/// Guest ABI v2 (Decision 105): regions returned by `model_alloc` must be
/// positive, inside current linear memory, and disjoint.
fn check_v2_regions(
    in_ptr: i32,
    in_len: usize,
    out_ptr: i32,
    out_cap: i32,
    memory_size: usize,
) -> Result<(), HostConnectorError> {
    let invalid = || {
        model_error_plain(
            HostConnectorErrorCode::ExecutionFailed,
            "model_alloc returned an invalid region",
        )
    };
    let (Ok(in_start), Ok(out_start), Ok(out_len)) = (
        usize::try_from(in_ptr),
        usize::try_from(out_ptr),
        usize::try_from(out_cap),
    ) else {
        return Err(invalid());
    };
    let in_end = in_start.saturating_add(in_len);
    let out_end = out_start.saturating_add(out_len);
    if in_start == 0
        || out_start == 0
        || in_end > memory_size
        || out_end > memory_size
        || (in_start < out_end && out_start < in_end)
    {
        return Err(invalid());
    }
    Ok(())
}

fn alloc_failed() -> HostConnectorError {
    model_error_plain(
        HostConnectorErrorCode::ExecutionFailed,
        "model_alloc trapped or ran out of fuel",
    )
}

fn missing_alloc() -> HostConnectorError {
    model_error_plain(
        HostConnectorErrorCode::ModelIncompatible,
        "abi_version 2 model wasm missing model_alloc export",
    )
}

/// Mid-run interruption checked between `wasmi` fuel slices (Decision 104).
struct SliceControl<'a> {
    cancel: &'a AtomicBool,
    deadline: Instant,
}

#[cfg(feature = "wasmi-executor")]
#[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
fn execute_wasmi_model(
    compiled: &mut CompiledGuests,
    key: &str,
    wasm: &[u8],
    input: &[u8],
    limits: &GuestLimits,
    control: &SliceControl<'_>,
) -> Result<Vec<u8>, HostConnectorError> {
    use wasmi::{Linker, Store, StoreLimitsBuilder};

    let (engine, module) = compiled.wasmi(key, wasm)?;
    let store_limits = StoreLimitsBuilder::new()
        .memory_size(usize::try_from(limits.memory).unwrap_or(usize::MAX))
        .build();
    let mut store = Store::new(&engine, store_limits);
    store.limiter(|state| state);
    // Fuel metering is enabled on this engine, so set_fuel cannot fail; the
    // first slice is granted here and the rest in `run_fuel_slices`.
    let _ = store.set_fuel(limits.fuel.min(WASMI_FUEL_SLICE));

    // Empty linker: deny-by-default (no WASI / no host imports).
    let Ok(instance) = Linker::new(&engine).instantiate_and_start(&mut store, &module) else {
        return Err(model_error_plain(
            HostConnectorErrorCode::ExecutionFailed,
            "model wasm instantiation failed",
        ));
    };
    let Some(memory) = instance.get_memory(&store, "memory") else {
        return Err(model_error_plain(
            HostConnectorErrorCode::ModelIncompatible,
            "model wasm missing memory export",
        ));
    };
    let Ok(func) =
        instance.get_typed_func::<(i32, i32, i32, i32), i32>(&store, MODEL_EXECUTE_EXPORT)
    else {
        return Err(model_error_plain(
            HostConnectorErrorCode::ModelIncompatible,
            "model wasm missing model_execute export",
        ));
    };

    let out_cap = output_capacity(limits.max_output);
    let (in_ptr, out_ptr) = if limits.abi >= 2 {
        let alloc = instance
            .get_typed_func::<i32, i32>(&store, MODEL_ALLOC_EXPORT)
            .map_err(|_| missing_alloc())?;
        let in_len = i32::try_from(input.len()).unwrap_or(i32::MAX);
        let in_ptr = alloc.call(&mut store, in_len).map_err(|_| alloc_failed())?;
        let out_ptr = alloc
            .call(&mut store, out_cap)
            .map_err(|_| alloc_failed())?;
        check_v2_regions(
            in_ptr,
            input.len(),
            out_ptr,
            out_cap,
            memory.data_size(&store),
        )?;
        (in_ptr, out_ptr)
    } else {
        let (in_ptr, out_ptr, end) = v1_placement(input.len(), out_cap);
        let current_pages = (memory.data_size(&store) as u64).div_ceil(65_536);
        let needed_pages = end.div_ceil(65_536);
        if needed_pages > current_pages
            && memory
                .grow(&mut store, needed_pages - current_pages)
                .is_err()
        {
            return Err(model_error_plain(
                HostConnectorErrorCode::ResourceExhausted,
                "model memory grow failed",
            ));
        }
        (in_ptr, out_ptr)
    };
    // Region sizing above guarantees the staged window fits.
    let _ = memory.write(&mut store, usize::try_from(in_ptr).unwrap_or(0), input);

    let args = (
        in_ptr,
        i32::try_from(input.len()).unwrap_or(i32::MAX),
        out_ptr,
        out_cap,
    );
    let out_len = run_fuel_slices(&mut store, func, args, limits, control)?;
    if out_len < 0 || out_len as u64 > limits.max_output {
        return Err(model_error_plain(
            HostConnectorErrorCode::ResourceExhausted,
            "model returned invalid output length",
        ));
    }
    let mut output = vec![0_u8; out_len as usize];
    let _ = memory.read(&store, usize::try_from(out_ptr).unwrap_or(0), &mut output);
    Ok(output)
}

/// Drives a resumable `wasmi` call in fuel slices, checking cancellation and
/// the deadline between slices (Decision 104).
#[cfg(feature = "wasmi-executor")]
fn run_fuel_slices(
    store: &mut wasmi::Store<wasmi::StoreLimits>,
    func: wasmi::TypedFunc<(i32, i32, i32, i32), i32>,
    args: (i32, i32, i32, i32),
    limits: &GuestLimits,
    control: &SliceControl<'_>,
) -> Result<i32, HostConnectorError> {
    use wasmi::TypedResumableCall as Call;

    let mut granted = limits.fuel.min(WASMI_FUEL_SLICE);
    let trapped = || {
        model_error_plain(
            HostConnectorErrorCode::ExecutionFailed,
            "model_execute trap or fuel exhausted",
        )
    };
    let mut call = func
        .call_resumable(&mut *store, args)
        .map_err(|_| trapped())?;
    loop {
        let Call::OutOfFuel(paused) = call else {
            // No host imports exist, so the only other outcome is Finished;
            // anything else maps to an invalid length and fails closed below.
            break Ok(if let Call::Finished(n) = call { n } else { -1 });
        };
        if control.cancel.load(Ordering::SeqCst) {
            return Err(model_error_plain(
                HostConnectorErrorCode::Cancelled,
                "model.execute cancelled mid-run",
            ));
        }
        if Instant::now() > control.deadline {
            return Err(model_error_plain(
                HostConnectorErrorCode::Timeout,
                "model.execute exceeded timeout mid-run",
            ));
        }
        if granted >= limits.fuel {
            return Err(trapped());
        }
        let next = (limits.fuel - granted).min(WASMI_FUEL_SLICE);
        granted += next;
        let _ = store.set_fuel(next);
        call = paused.resume(&mut *store).map_err(|_| trapped())?;
    }
}

#[cfg(not(feature = "wasmi-executor"))]
fn execute_wasmi_model(
    _compiled: &mut CompiledGuests,
    _key: &str,
    _wasm: &[u8],
    _input: &[u8],
    _limits: &GuestLimits,
    _control: &SliceControl<'_>,
) -> Result<Vec<u8>, HostConnectorError> {
    Err(model_error_plain(
        HostConnectorErrorCode::Unavailable,
        "wasm-cpu interpreter requires the wasmi-executor feature",
    ))
}

fn model_error_plain(code: HostConnectorErrorCode, message: &str) -> HostConnectorError {
    HostConnectorError {
        code,
        reason: None,
        detail: None,
        message: message.to_string(),
    }
}

/// Stages the input/output windows in guest memory for the wasmtime executor:
/// fixed offsets (ABI v1) or guest `model_alloc` regions (ABI v2).
#[cfg(feature = "wasmtime-executor")]
fn place_wasmtime_buffers(
    store: &mut wasmtime::Store<wasmtime::StoreLimits>,
    instance: &wasmtime::Instance,
    memory: wasmtime::Memory,
    input_len: usize,
    out_cap: i32,
    abi: u16,
) -> Result<(i32, i32), HostConnectorError> {
    if abi >= 2 {
        let alloc = instance
            .get_typed_func::<i32, i32>(&mut *store, MODEL_ALLOC_EXPORT)
            .map_err(|_| missing_alloc())?;
        let in_len = i32::try_from(input_len).unwrap_or(i32::MAX);
        let in_ptr = alloc
            .call(&mut *store, in_len)
            .map_err(|_| alloc_failed())?;
        let out_ptr = alloc
            .call(&mut *store, out_cap)
            .map_err(|_| alloc_failed())?;
        check_v2_regions(
            in_ptr,
            input_len,
            out_ptr,
            out_cap,
            memory.data_size(&*store),
        )?;
        Ok((in_ptr, out_ptr))
    } else {
        let (in_ptr, out_ptr, end) = v1_placement(input_len, out_cap);
        let current_pages = u64::try_from(memory.data_size(&*store))
            .unwrap_or(0)
            .div_ceil(65_536);
        let needed_pages = end.div_ceil(65_536);
        if needed_pages > current_pages {
            require_ok(
                memory
                    .grow(&mut *store, needed_pages - current_pages)
                    .is_ok(),
                HostConnectorErrorCode::ResourceExhausted,
                "model memory grow failed",
            )?;
        }
        Ok((in_ptr, out_ptr))
    }
}

#[cfg(not(feature = "wasmtime-executor"))]
fn execute_wasm_cpu_model(
    _compiled: &mut CompiledGuests,
    _key: &str,
    _wasm: &[u8],
    _input: &[u8],
    _guest: &GuestLimits,
) -> Result<Vec<u8>, HostConnectorError> {
    Err(HostConnectorError {
        code: HostConnectorErrorCode::Unavailable,
        reason: None,
        detail: None,
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

    const TEST_SIGNING_KEY: &str = include_str!("../../../fixtures/models/test-signing-key.json");

    fn fixture_rights() -> ModelRights {
        ModelRights {
            license_id: "Apache-2.0".to_string(),
            attribution: "Traverse fixture".to_string(),
            redistribution: "test-only".to_string(),
            commercial_use: CommercialUse::Allowed,
            source_url: "https://example.invalid/fixture".to_string(),
            derivation: None,
        }
    }

    /// Re-serialize the (possibly mutated) manifest so bytes and digest agree.
    fn seal(mut package: VerifiedModelPackage) -> VerifiedModelPackage {
        package.manifest_bytes = serde_json::to_vec(&package.manifest).expect("manifest json");
        package
    }

    fn derivative_manifest(
        commercial_use: CommercialUse,
        source_commercial_use: CommercialUse,
    ) -> ModelPackageManifest {
        let mut manifest = fixture_package().manifest;
        manifest.schema_version = MODEL_PACKAGE_SCHEMA_VERSION_DERIVATION.to_string();
        manifest.rights.commercial_use = commercial_use;
        manifest.rights.derivation = Some(ModelDerivation {
            kind: DerivationKind::Quantized,
            source_digest: format!("sha256:{}", "a".repeat(64)),
            source_license_id: "Apache-2.0".to_string(),
            source_commercial_use,
            source_url: "https://example.invalid/source".to_string(),
        });
        manifest
    }

    #[test]
    fn derivation_validation_orders_permissiveness_and_checks_the_source_digest() {
        let reason =
            |manifest: &ModelPackageManifest| manifest.validate().err().and_then(|e| e.reason);
        let equal = derivative_manifest(CommercialUse::Restricted, CommercialUse::Restricted);
        assert_eq!(reason(&equal), None);
        let stricter = derivative_manifest(CommercialUse::Prohibited, CommercialUse::Allowed);
        assert_eq!(reason(&stricter), None);
        let looser = derivative_manifest(CommercialUse::Restricted, CommercialUse::Prohibited);
        let error = looser.validate().expect_err("inconsistent");
        assert_eq!(error.reason, Some(ModelFailureReason::RightsInconsistent));
        let detail = error.detail.expect("detail");
        assert_eq!(
            (
                detail.field.as_str(),
                detail.expected.as_str(),
                detail.actual.as_str()
            ),
            (
                "rights.commercial_use",
                "no more permissive than prohibited",
                "restricted"
            )
        );
        let mut bad_digest = derivative_manifest(CommercialUse::Allowed, CommercialUse::Allowed);
        if let Some(derivation) = bad_digest.rights.derivation.as_mut() {
            derivation.source_digest = "not-a-digest".to_string();
        }
        assert_eq!(
            reason(&bad_digest),
            Some(ModelFailureReason::ManifestInvalid)
        );
        let mut unknown_schema = fixture_package().manifest;
        unknown_schema.schema_version = "2.2.0".to_string();
        assert_eq!(
            reason(&unknown_schema),
            Some(ModelFailureReason::ManifestInvalid)
        );
        let kinds: Vec<Value> = [
            DerivationKind::Converted,
            DerivationKind::Quantized,
            DerivationKind::FineTuned,
        ]
        .iter()
        .map(|kind| serde_json::to_value(kind).expect("kind"))
        .collect();
        assert_eq!(
            kinds,
            [json!("converted"), json!("quantized"), json!("fine_tuned")]
        );
    }

    #[test]
    fn rights_record_is_none_until_registered_and_usage_declared() {
        let (mut host, digest) = seeded_host();
        assert_eq!(host.model_rights_record("deadbeef"), None);
        let record = host
            .model_rights_record(&format!("sha256:{digest}"))
            .expect("record");
        assert_eq!(
            (record.status, record.effective_usage, record.status_reason),
            (PackageStatus::Active, ModelUsage::Commercial, None)
        );
        host.host_requires_commercial = true;
        host.model_usage = Some(ModelUsage::NonCommercial);
        assert_eq!(host.effective_usage().ok(), Some(ModelUsage::Commercial));
        host.model_usage = None;
        assert_eq!(host.model_rights_record(&digest), None);
        let error = host.effective_usage().expect_err("undeclared");
        assert_eq!(error.reason, Some(ModelFailureReason::UsageUndeclared));
        assert_eq!(error.detail.expect("detail").model_id, None);
    }

    fn manifest_digest(manifest: &ModelPackageManifest) -> String {
        digest_hex(&serde_json::to_vec(manifest).expect("manifest json"))
    }

    fn test_pin(model_id: &str, digest: &str) -> ExactModelPin {
        ExactModelPin {
            model_id: model_id.to_string(),
            version: "1.0.0".to_string(),
            digest: digest.to_string(),
            offline_allowed: true,
            target: PLACEMENT_WASM_CPU.to_string(),
            rights: PinRights {
                license_id: "Apache-2.0".to_string(),
                commercial_use: CommercialUse::Allowed,
            },
            key_id: None,
        }
    }

    fn test_key() -> ([u8; 32], [u8; 32]) {
        let key: Value = serde_json::from_str(TEST_SIGNING_KEY).expect("key json");
        let decode = |field: &str| -> [u8; 32] {
            hex_decode(key[field].as_str().expect("hex"))
                .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
                .expect("32 bytes")
        };
        (decode("secret_key_hex"), decode("public_key_hex"))
    }

    fn fixture_package() -> VerifiedModelPackage {
        let wasm = wat::parse_str(FIXTURE_ECHO_WAT).expect("wat");
        let wasm_digest = digest_hex(&wasm);
        let manifest = ModelPackageManifest {
            schema_version: MODEL_PACKAGE_SCHEMA_VERSION.to_string(),
            model_id: "fixture.echo".to_string(),
            version: "1.0.0".to_string(),
            wasm_digest,
            registry_ref: "registry:fixture.echo@1.0.0".to_string(),
            executable_format: "traverse-model-wasm".to_string(),
            abi_version: MODEL_GUEST_ABI_VERSION,
            input_schema_ref: "schema:fixture-in".to_string(),
            input_schema_version: "1.0.0".to_string(),
            output_schema_ref: "schema:fixture-out".to_string(),
            output_schema_version: "1.0.0".to_string(),
            rights: fixture_rights(),
            supported_profiles: vec![PLACEMENT_WASM_CPU.to_string()],
            max_memory_bytes: 2 * 64 * 1024,
            max_fuel: 1_000_000,
            max_input_bytes: 4096,
            max_output_bytes: 4096,
            max_execution_ms: 5_000,
            offline_allowed: true,
        };
        seal(VerifiedModelPackage {
            manifest,
            manifest_bytes: Vec::new(),
            wasm,
        })
    }

    #[test]
    fn stage_execute_read_echo_model_round_trip() {
        let package = fixture_package();
        let digest = manifest_digest(&package.manifest);
        let pin = test_pin("fixture.echo", &digest);
        let mut host = ExactModelHostConnector::new(vec![pin], TrustedModelKeys::new());
        host.model_usage = Some(ModelUsage::Commercial);
        host.packages
            .insert_verified(seal(package))
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
        let mut host = ExactModelHostConnector::new(
            vec![test_pin("fixture.echo", "deadbeef")],
            TrustedModelKeys::new(),
        );
        host.model_usage = Some(ModelUsage::Commercial);
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
        let digest = manifest_digest(&package.manifest);
        let pin = test_pin("fixture.echo", &format!("sha256:{digest}"));
        let mut host = ExactModelHostConnector::new(vec![pin], TrustedModelKeys::new());
        host.model_usage = Some(ModelUsage::Commercial);
        host.packages
            .insert_verified(seal(package))
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
        missing_license.manifest.rights.license_id.clear();
        assert_eq!(
            missing_license
                .manifest
                .validate()
                .expect_err("license")
                .reason,
            Some(ModelFailureReason::RightsIncomplete)
        );

        let mut missing_source = fixture_package();
        missing_source.manifest.rights.source_url = " ".to_string();
        assert_eq!(
            missing_source
                .manifest
                .validate()
                .expect_err("source")
                .reason,
            Some(ModelFailureReason::RightsIncomplete)
        );

        let mut missing_format = fixture_package();
        missing_format.manifest.executable_format.clear();
        assert_eq!(
            missing_format
                .manifest
                .validate()
                .expect_err("format")
                .reason,
            Some(ModelFailureReason::ManifestInvalid)
        );

        let mut old_schema = fixture_package();
        old_schema.manifest.schema_version = "1.0.0".to_string();
        assert_eq!(
            old_schema.manifest.validate().expect_err("schema").reason,
            Some(ModelFailureReason::ManifestInvalid)
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
            store
                .insert_verified(seal(mismatched))
                .expect_err("digest")
                .code,
            HostConnectorErrorCode::ModelIncompatible
        );
        assert_eq!(
            store.resolve_offline("missing").expect_err("offline").code,
            HostConnectorErrorCode::ModelUnavailable
        );
    }

    #[test]
    fn artifact_refs_are_multi_read_bounded_and_opaque() {
        let mut io = ModelIoStore::new();
        assert_eq!(
            io.stage_artifact(b"", 8).expect_err("empty").code,
            HostConnectorErrorCode::InputLimitExceeded
        );
        assert_eq!(
            io.stage_artifact(b"abcdef", 4).expect_err("over").code,
            HostConnectorErrorCode::InputLimitExceeded
        );
        let artifact_ref = io.stage_artifact(b"abc", 8).expect("stage");
        assert_eq!(artifact_ref, "artifact-1");
        assert!(!artifact_ref.contains('/') && !artifact_ref.contains(':'));
        // Multi-read: repeated reads (runtime-owned retries) all succeed.
        assert_eq!(io.read_artifact(&artifact_ref, 8).expect("first"), b"abc");
        assert_eq!(io.read_artifact(&artifact_ref, 8).expect("retry"), b"abc");
        assert_eq!(
            io.read_artifact(&artifact_ref, 2).expect_err("cap").code,
            HostConnectorErrorCode::InputLimitExceeded
        );
        assert_eq!(
            io.read_artifact("artifact-9", 8).expect_err("missing").code,
            HostConnectorErrorCode::Unavailable
        );
        // Model refs live in separate namespaces: input_ref stays single-consume
        // and is never readable through the artifact path.
        let input_ref = io.stage_model_input(b"abc", 8).expect("input");
        assert_eq!(
            io.read_artifact(&input_ref, 8).expect_err("ns").code,
            HostConnectorErrorCode::Unavailable
        );
        io.take_input(&input_ref).expect("consume once");
        assert_eq!(io.read_artifact(&artifact_ref, 8).expect("still"), b"abc");
        io.drop_ref(&artifact_ref);
        assert!(io.read_artifact(&artifact_ref, 8).is_err());
    }

    #[test]
    fn shutdown_invalidates_every_staged_ref() {
        let mut io = ModelIoStore::new();
        let artifact_ref = io.stage_artifact(b"abc", 8).expect("artifact");
        let input_ref = io.stage_model_input(b"abc", 8).expect("input");
        let output_ref = io.put_output(b"abc".to_vec());
        io.shutdown();
        assert!(io.read_artifact(&artifact_ref, 8).is_err());
        assert!(io.take_input(&input_ref).is_err());
        assert!(io.read_model_output(&output_ref, 8).is_err());
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

        // A tampered cache entry (bytes no longer hash to the pin) fails closed
        // on the per-execute digest re-check.
        let mut tampered = fixture_package();
        tampered.manifest.model_id = "other".to_string();
        let tampered = seal(tampered);
        host.packages
            .by_digest
            .insert(normalize_digest(&digest), tampered);
        let input_ref = host.io.stage_model_input(b"abc", 64).expect("stage");
        let err = host
            .invoke(&execute_request(
                &digest,
                &input_ref,
                serde_json::Map::new(),
            ))
            .expect_err("tampered");
        assert_eq!(err.code, HostConnectorErrorCode::ModelIncompatible);
        assert_eq!(err.reason, Some(ModelFailureReason::DigestMismatch));
        let mut tampered_wasm = fixture_package();
        tampered_wasm.wasm = b"swapped".to_vec();
        host.packages
            .by_digest
            .insert(normalize_digest(&digest), tampered_wasm);
        let input_ref = host.io.stage_model_input(b"abc", 64).expect("stage");
        assert_eq!(
            host.invoke(&execute_request(
                &digest,
                &input_ref,
                serde_json::Map::new()
            ))
            .expect_err("tampered wasm")
            .reason,
            Some(ModelFailureReason::DigestMismatch)
        );

        // Restore a matching package for remaining cases.
        let restored = fixture_package();
        host.packages
            .insert_verified(seal(restored))
            .expect("restore");

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
        let bad_digest = manifest_digest(&bad_wasm.manifest);
        host.pins.push(test_pin("fixture.echo", &bad_digest));
        host.packages
            .insert_verified(seal(bad_wasm))
            .expect("insert bad");
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
        let digest_missing = manifest_digest(&pkg.manifest);
        host.pins.push(test_pin("fixture.echo", &digest_missing));
        host.packages.insert_verified(seal(pkg)).expect("insert");
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
        let digest_mem = manifest_digest(&pkg.manifest);
        host.pins.push(test_pin("fixture.echo", &digest_mem));
        host.packages.insert_verified(seal(pkg)).expect("insert");
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
        pkg.manifest.max_fuel = 10;
        let digest_fuel = manifest_digest(&pkg.manifest);
        host.pins.push(test_pin("fixture.echo", &digest_fuel));
        host.packages.insert_verified(seal(pkg)).expect("insert");
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
        let digest_len = manifest_digest(&pkg.manifest);
        host.pins.push(test_pin("fixture.echo", &digest_len));
        host.packages.insert_verified(seal(pkg)).expect("insert");
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
        grow_pkg.manifest.max_memory_bytes = 4 * 64 * 1024;
        grow_pkg.manifest.max_output_bytes = 70_000;
        let grow_digest = manifest_digest(&grow_pkg.manifest);
        host.pins.push(test_pin("fixture.echo", &grow_digest));
        host.packages
            .insert_verified(seal(grow_pkg))
            .expect("insert");
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
        blocked.manifest.max_memory_bytes = 64 * 1024;
        blocked.manifest.max_output_bytes = 70_000;
        let blocked_digest = manifest_digest(&blocked.manifest);
        host.pins.push(test_pin("fixture.echo", &blocked_digest));
        host.packages
            .insert_verified(seal(blocked))
            .expect("insert");
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
        let import_digest = manifest_digest(&pkg.manifest);
        host.pins.push(test_pin("fixture.echo", &import_digest));
        host.packages.insert_verified(seal(pkg)).expect("insert");
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
            schema_version: MODEL_PACKAGE_SCHEMA_VERSION.to_string(),
            model_id: "fixture.classifier".to_string(),
            version: "1.0.0".to_string(),
            wasm_digest,
            registry_ref: "registry:fixture.classifier@1.0.0".to_string(),
            executable_format: "traverse-model-wasm".to_string(),
            abi_version: MODEL_GUEST_ABI_VERSION,
            input_schema_ref: "schema:fixture-classifier-in".to_string(),
            input_schema_version: "1.0.0".to_string(),
            output_schema_ref: "schema:fixture-classifier-out".to_string(),
            output_schema_version: "1.0.0".to_string(),
            rights: fixture_rights(),
            supported_profiles: vec![PLACEMENT_WASM_CPU.to_string()],
            max_memory_bytes: 2 * 64 * 1024,
            max_fuel: 1_000_000,
            max_input_bytes: 4096,
            max_output_bytes: 4096,
            max_execution_ms: 5_000,
            offline_allowed: true,
        };
        seal(VerifiedModelPackage {
            manifest,
            manifest_bytes: Vec::new(),
            wasm,
        })
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
        let digest = manifest_digest(&package.manifest);
        let pin = test_pin("fixture.classifier", &digest);
        let mut host = ExactModelHostConnector::new(vec![pin], TrustedModelKeys::new());
        host.model_usage = Some(ModelUsage::Commercial);
        host.packages
            .insert_verified(seal(package))
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

    const CLASSIFIER_DIR: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/models/fixture-classifier-1.0.0"
    );

    fn read_fixture(name: &str) -> Vec<u8> {
        std::fs::read(format!("{CLASSIFIER_DIR}/{name}")).expect("fixture file")
    }

    fn classifier_pin(manifest_bytes: &[u8]) -> ExactModelPin {
        test_pin("fixture.classifier", &digest_hex(manifest_bytes))
    }

    fn trusted_host(pins: Vec<ExactModelPin>) -> ExactModelHostConnector {
        let (_, public) = test_key();
        let mut keys = TrustedModelKeys::new();
        keys.trust(&public).expect("trust");
        let mut host = ExactModelHostConnector::new(pins, keys);
        host.model_usage = Some(ModelUsage::Commercial);
        host.model_usage = Some(ModelUsage::Commercial);
        host
    }

    fn register_err(
        host: &mut ExactModelHostConnector,
        manifest: &[u8],
        wasm: Vec<u8>,
        sig: &[u8],
    ) -> (HostConnectorErrorCode, Option<ModelFailureReason>) {
        let err = host
            .register_package(manifest, wasm, sig)
            .expect_err("register must fail closed");
        (err.code, err.reason)
    }

    fn signed(manifest: &[u8]) -> Vec<u8> {
        let (secret, _) = test_key();
        serde_json::to_vec(&sign_model_manifest(&secret, manifest)).expect("sig json")
    }

    #[test]
    fn checked_in_signed_fixture_registers_and_exposes_rights() {
        let manifest = read_fixture("model.manifest.json");
        let pin = classifier_pin(&manifest);
        let mut host = trusted_host(vec![pin.clone()]);
        let (_, public) = test_key();
        let key: Value = serde_json::from_str(TEST_SIGNING_KEY).expect("key json");
        assert_eq!(key["key_id"], json!(model_signing_key_id(&public)));
        let digest = host
            .register_package(
                &manifest,
                read_fixture("model.wasm"),
                &read_fixture("model.sig.json"),
            )
            .expect("checked-in signed fixture must verify");
        assert_eq!(digest, normalize_digest(&pin.digest));
        let rights = host.model_rights(&digest).expect("rights");
        assert_eq!(rights.license_id, "Apache-2.0");
        assert_eq!(rights.commercial_use, CommercialUse::Allowed);
        assert!(rights.source_url.starts_with("https://"));
        assert!(host.model_rights("missing").is_none());
    }

    #[test]
    fn signed_conformance_vector_matches_native_execution() {
        let vector: Value = serde_json::from_str(include_str!(
            "../../../fixtures/models/conformance/signed-classifier.json"
        ))
        .expect("vector json");
        let pin: ExactModelPin = serde_json::from_value(vector["pin"].clone()).expect("pin");
        let public = hex_decode(vector["trusted_public_key_hex"].as_str().expect("key"))
            .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
            .expect("public key");
        let mut keys = TrustedModelKeys::new();
        keys.trust(&public).expect("trust");
        let mut host = ExactModelHostConnector::new(vec![pin.clone()], keys);
        host.model_usage = Some(ModelUsage::Commercial);
        host.policies.insert(
            "policy-1".to_string(),
            ExecutionPolicy {
                policy_ref: "policy-1".to_string(),
                allowed_classifications: vec!["sensitive".to_string()],
                max_output_bytes: 4096,
            },
        );
        host.register_package(
            &read_fixture("model.manifest.json"),
            read_fixture("model.wasm"),
            &read_fixture("model.sig.json"),
        )
        .expect("register");
        let request = &vector["request"];
        let input = hex_decode(request["input_frame_hex"].as_str().expect("input")).expect("hex");
        let input_ref = host.io.stage_model_input(&input, 4096).expect("stage");
        let result = host
            .invoke(&classifier_execute_request(&pin.digest, &input_ref))
            .expect("execute");
        let output = host
            .io
            .read_model_output(result.artifact_ref.as_deref().expect("output_ref"), 4096)
            .expect("read");
        assert_eq!(
            hex_encode(&output),
            vector["expected"]["output_frame_hex"]
                .as_str()
                .expect("expected")
        );
    }

    #[test]
    fn register_rejects_bad_signatures_and_untrusted_keys() {
        use HostConnectorErrorCode::ModelIncompatible as Inc;
        use ModelFailureReason as R;
        let manifest = read_fixture("model.manifest.json");
        let wasm = read_fixture("model.wasm");
        let sig = read_fixture("model.sig.json");
        let pin = classifier_pin(&manifest);

        // No trusted keys at all.
        let mut untrusting =
            ExactModelHostConnector::new(vec![pin.clone()], TrustedModelKeys::new());
        untrusting.model_usage = Some(ModelUsage::Commercial);
        assert_eq!(
            register_err(&mut untrusting, &manifest, wasm.clone(), &sig),
            (Inc, Some(R::KeyUntrusted))
        );

        let mut host = trusted_host(vec![pin.clone()]);
        assert_eq!(
            register_err(&mut host, &manifest, wasm.clone(), b"not json"),
            (Inc, Some(R::SignatureInvalid))
        );
        let mut doc: ModelPackageSignature = serde_json::from_slice(&sig).expect("sig");
        let original = doc.clone();
        doc.alg = "rsa".to_string();
        let bad_alg = serde_json::to_vec(&doc).expect("json");
        assert_eq!(
            register_err(&mut host, &manifest, wasm.clone(), &bad_alg),
            (Inc, Some(R::SignatureInvalid))
        );
        doc = original.clone();
        doc.signature = "zz".to_string();
        let malformed = serde_json::to_vec(&doc).expect("json");
        assert_eq!(
            register_err(&mut host, &manifest, wasm.clone(), &malformed),
            (Inc, Some(R::SignatureInvalid))
        );
        doc.signature = "abc".to_string();
        let odd = serde_json::to_vec(&doc).expect("json");
        assert_eq!(
            register_err(&mut host, &manifest, wasm.clone(), &odd),
            (Inc, Some(R::SignatureInvalid))
        );
        doc = original.clone();
        doc.signature = "00".repeat(64);
        let wrong = serde_json::to_vec(&doc).expect("json");
        assert_eq!(
            register_err(&mut host, &manifest, wasm.clone(), &wrong),
            (Inc, Some(R::SignatureInvalid))
        );
        // Signature over different bytes than the manifest presented.
        let mut tampered = manifest.clone();
        tampered.push(b'\n');
        assert_eq!(
            register_err(&mut host, &tampered, wasm.clone(), &sig),
            (Inc, Some(R::SignatureInvalid))
        );
        // Signed by a key the host does not trust.
        let other = sign_model_manifest(&[7_u8; 32], &manifest);
        assert_eq!(
            register_err(
                &mut host,
                &manifest,
                wasm.clone(),
                &serde_json::to_vec(&other).expect("json")
            ),
            (Inc, Some(R::KeyUntrusted))
        );
        // Pin narrows to a different trusted key id.
        let mut narrowed = pin.clone();
        narrowed.key_id = Some("ed25519:other".to_string());
        let mut host = trusted_host(vec![narrowed]);
        assert_eq!(
            register_err(&mut host, &manifest, wasm.clone(), &sig),
            (Inc, Some(R::KeyUntrusted))
        );
        let mut exact = pin;
        exact.key_id = Some(original.key_id);
        let mut host = trusted_host(vec![exact]);
        host.register_package(&manifest, wasm, &sig)
            .expect("matching key_id narrows successfully");

        // Find a 32-byte string that is not a valid curve point encoding.
        let mut keys = TrustedModelKeys::new();
        let rejected = (0_u8..=255)
            .map(|byte| keys.trust(&[byte; 32]))
            .find_map(Result::err)
            .expect("some constant byte string is not a curve point");
        assert_eq!(rejected.reason, Some(R::KeyUntrusted));
    }

    #[test]
    fn register_rejects_pin_digest_rights_and_target_mismatches() {
        use HostConnectorErrorCode::{ModelIncompatible as Inc, ModelUnavailable as Unav};
        use ModelFailureReason as R;
        let manifest_bytes = read_fixture("model.manifest.json");
        let wasm = read_fixture("model.wasm");
        let sig = read_fixture("model.sig.json");
        let pin = classifier_pin(&manifest_bytes);

        // Missing pin.
        let mut host = trusted_host(vec![test_pin("fixture.classifier", "00")]);
        assert_eq!(
            register_err(&mut host, &manifest_bytes, wasm.clone(), &sig),
            (Unav, Some(R::PinMismatch))
        );
        // Ambiguous pins for the same identity.
        let mut host = trusted_host(vec![pin.clone(), test_pin("fixture.classifier", "11")]);
        assert_eq!(
            register_err(&mut host, &manifest_bytes, wasm.clone(), &sig),
            (Inc, Some(R::PinAmbiguous))
        );
        // Pin identity differs from signed manifest identity.
        let mut renamed = pin.clone();
        renamed.model_id = "fixture.other".to_string();
        let mut host = trusted_host(vec![renamed]);
        assert_eq!(
            register_err(&mut host, &manifest_bytes, wasm.clone(), &sig),
            (Inc, Some(R::PinMismatch))
        );
        // Unsupported pin target.
        let mut gpu = pin.clone();
        gpu.target = "gpu".to_string();
        let mut host = trusted_host(vec![gpu]);
        assert_eq!(
            register_err(&mut host, &manifest_bytes, wasm.clone(), &sig),
            (Inc, Some(R::TargetUnsupported))
        );
        // License and commercial-use mismatches.
        let mut license = pin.clone();
        license.rights.license_id = "MIT".to_string();
        let mut host = trusted_host(vec![license]);
        assert_eq!(
            register_err(&mut host, &manifest_bytes, wasm.clone(), &sig),
            (Inc, Some(R::RightsMismatch))
        );
        let mut commercial = pin.clone();
        commercial.rights.commercial_use = CommercialUse::Prohibited;
        let mut host = trusted_host(vec![commercial]);
        assert_eq!(
            register_err(&mut host, &manifest_bytes, wasm.clone(), &sig),
            (Inc, Some(R::RightsMismatch))
        );
        // WASM bytes do not match the signed wasm_digest.
        let mut host = trusted_host(vec![pin]);
        assert_eq!(
            register_err(&mut host, &manifest_bytes, b"other".to_vec(), &sig),
            (Inc, Some(R::DigestMismatch))
        );

        // Correctly signed but malformed / incomplete manifests.
        let manifest: Value = serde_json::from_slice(&manifest_bytes).expect("manifest");
        let resign = |value: &Value| -> (Vec<u8>, Vec<u8>, ExactModelPin) {
            let bytes = serde_json::to_vec(value).expect("json");
            let sig = signed(&bytes);
            let pin = classifier_pin(&bytes);
            (bytes, sig, pin)
        };
        let mut unknown = manifest.clone();
        unknown["package_digest"] = json!("legacy");
        let (bytes, sig, pin) = resign(&unknown);
        let mut host = trusted_host(vec![pin]);
        assert_eq!(
            register_err(&mut host, &bytes, wasm.clone(), &sig),
            (Inc, Some(R::ManifestInvalid))
        );
        let mut no_rights = manifest.clone();
        no_rights["rights"]["attribution"] = json!("");
        let (bytes, sig, pin) = resign(&no_rights);
        let mut host = trusted_host(vec![pin]);
        assert_eq!(
            register_err(&mut host, &bytes, wasm.clone(), &sig),
            (Inc, Some(R::RightsIncomplete))
        );
        let mut no_cpu = manifest;
        no_cpu["supported_profiles"] = json!(["gpu"]);
        let (bytes, sig, pin) = resign(&no_cpu);
        let mut host = trusted_host(vec![pin]);
        assert_eq!(
            register_err(&mut host, &bytes, wasm, &sig),
            (Inc, Some(R::TargetUnsupported))
        );
    }

    #[test]
    fn identity_mismatch_behind_a_matching_digest_fails_closed() {
        let mut other = fixture_package();
        other.manifest.model_id = "other".to_string();
        let digest = manifest_digest(&other.manifest);
        let mut host = ExactModelHostConnector::new(
            vec![test_pin("fixture.echo", &digest)],
            TrustedModelKeys::new(),
        );
        host.model_usage = Some(ModelUsage::Commercial);
        host.packages.insert_verified(seal(other)).expect("insert");
        host.policies.insert(
            "policy-1".to_string(),
            ExecutionPolicy {
                policy_ref: "policy-1".to_string(),
                allowed_classifications: vec!["sensitive".to_string()],
                max_output_bytes: 4096,
            },
        );
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
    }

    #[test]
    fn failure_reasons_have_stable_wire_names() {
        use ModelFailureReason as R;
        let all = [
            (R::PinMismatch, "pin_mismatch"),
            (R::PinAmbiguous, "pin_ambiguous"),
            (R::SignatureInvalid, "signature_invalid"),
            (R::KeyUntrusted, "key_untrusted"),
            (R::DigestMismatch, "digest_mismatch"),
            (R::ManifestInvalid, "manifest_invalid"),
            (R::RightsIncomplete, "rights_incomplete"),
            (R::RightsMismatch, "rights_mismatch"),
            (R::TargetUnsupported, "target_unsupported"),
            (R::CryptoUnavailable, "crypto_unavailable"),
            (R::CandidateUnsupported, "candidate_unsupported"),
            (R::HostLimitExceeded, "host_limit_exceeded"),
        ];
        for (reason, name) in all {
            assert_eq!(reason.as_str(), name);
            assert_eq!(serde_json::to_value(reason).expect("json"), json!(name));
        }
        let err = HostConnectorError {
            code: HostConnectorErrorCode::Timeout,
            reason: None,
            detail: None,
            message: "x".to_string(),
        };
        assert!(
            serde_json::to_value(&err)
                .expect("json")
                .get("reason")
                .is_none()
        );
    }

    #[test]
    fn rights_schemas_match_the_rust_types() {
        fn sorted(value: &Value) -> Vec<String> {
            let mut keys: Vec<String> =
                value.as_object().expect("object").keys().cloned().collect();
            keys.sort();
            keys
        }
        let parse = |text: &str| -> Value { serde_json::from_str(text).expect("schema") };
        let manifest_schema = parse(include_str!(
            "../../../contracts/connectors/traverse.model-runtime/schemas/model-package-manifest-2.1.0.json"
        ));
        let record_schema = parse(include_str!(
            "../../../contracts/connectors/traverse.model-runtime/schemas/model-rights-record-1.0.0.json"
        ));
        let detail_schema = parse(include_str!(
            "../../../contracts/connectors/traverse.model-runtime/schemas/model-rights-denial-detail-1.0.0.json"
        ));
        assert_eq!(
            manifest_schema["properties"]["schema_version"]["const"],
            json!(MODEL_PACKAGE_SCHEMA_VERSION_DERIVATION)
        );
        let manifest = derivative_manifest(CommercialUse::Allowed, CommercialUse::Allowed);
        let manifest_json = serde_json::to_value(&manifest).expect("manifest");
        assert_eq!(
            sorted(&manifest_schema["properties"]),
            sorted(&manifest_json)
        );
        assert_eq!(
            sorted(&manifest_schema["$defs"]["rights"]["properties"]),
            sorted(&manifest_json["rights"])
        );
        assert_eq!(
            sorted(&manifest_schema["$defs"]["derivation"]["properties"]),
            sorted(&manifest_json["rights"]["derivation"])
        );
        let mut record = rights_record(
            &manifest,
            "a",
            ModelUsage::NonCommercial,
            Some(&PackageStatusEntry {
                status: PackageStatus::Deprecated,
                reason: "r".to_string(),
            }),
        );
        assert_eq!(
            sorted(&record_schema["properties"]),
            sorted(&serde_json::to_value(&record).expect("record"))
        );
        record.status = PackageStatus::Revoked;
        assert_eq!(
            serde_json::to_value(&record).expect("record")["status"],
            json!("revoked")
        );
        let mut detail = denial("rights.commercial_use", "allowed", "prohibited");
        detail.effective_usage = Some(ModelUsage::Commercial);
        let detail = for_package(
            with_detail(
                incompatible(ModelFailureReason::RightsPolicyDenied, "x"),
                detail,
            ),
            &manifest,
            "a",
        )
        .detail
        .expect("detail");
        assert_eq!(
            sorted(&detail_schema["properties"]),
            sorted(&serde_json::to_value(&detail).expect("detail"))
        );
    }

    #[test]
    fn versioned_schemas_match_the_checked_in_signed_fixtures() {
        fn keys(value: &Value) -> Vec<String> {
            let mut keys: Vec<String> =
                value.as_object().expect("object").keys().cloned().collect();
            keys.sort();
            keys
        }
        fn schema_keys(schema: &str, field: &str) -> Vec<String> {
            let schema: Value = serde_json::from_str(schema).expect("schema json");
            let mut keys: Vec<String> = match field {
                "required" => schema["required"]
                    .as_array()
                    .expect("required")
                    .iter()
                    .map(|key| key.as_str().expect("key").to_string())
                    .collect(),
                _ => schema["properties"]
                    .as_object()
                    .expect("properties")
                    .keys()
                    .cloned()
                    .collect(),
            };
            keys.sort();
            keys
        }
        let manifest_schema = include_str!(
            "../../../contracts/connectors/traverse.model-runtime/schemas/model-package-manifest-2.0.0.json"
        );
        let signature_schema = include_str!(
            "../../../contracts/connectors/traverse.model-runtime/schemas/model-package-signature-1.0.0.json"
        );
        let pin_schema = include_str!(
            "../../../contracts/connectors/traverse.model-runtime/schemas/exact-model-pin-2.0.0.json"
        );
        let manifest: Value =
            serde_json::from_slice(&read_fixture("model.manifest.json")).expect("manifest");
        let signature: Value =
            serde_json::from_slice(&read_fixture("model.sig.json")).expect("signature");
        let pin = serde_json::to_value(classifier_pin(&read_fixture("model.manifest.json")))
            .expect("pin");
        for (schema, document) in [(manifest_schema, &manifest), (signature_schema, &signature)] {
            assert_eq!(schema_keys(schema, "required"), keys(document));
            assert_eq!(schema_keys(schema, "properties"), keys(document));
        }
        let manifest_schema_value: Value = serde_json::from_str(manifest_schema).expect("schema");
        assert_eq!(
            manifest_schema_value["properties"]["schema_version"]["const"],
            json!(MODEL_PACKAGE_SCHEMA_VERSION)
        );
        assert_eq!(
            keys(&manifest_schema_value["$defs"]["rights"]["properties"]),
            keys(&manifest["rights"])
        );
        // `key_id` is the only optional pin field (skipped when `None`).
        assert_eq!(schema_keys(pin_schema, "required"), keys(&pin));
        let mut with_key = schema_keys(pin_schema, "required");
        with_key.push("key_id".to_string());
        with_key.sort();
        assert_eq!(schema_keys(pin_schema, "properties"), with_key);
    }
    /// Registers a variant of the echo fixture under its own pin and returns
    /// its digest (bypasses signing: executor behaviour only).
    fn add_variant(
        host: &mut ExactModelHostConnector,
        wasm: Vec<u8>,
        tweak: impl FnOnce(&mut ModelPackageManifest),
    ) -> String {
        let mut pkg = fixture_package();
        pkg.wasm = wasm;
        pkg.manifest.wasm_digest = digest_hex(&pkg.wasm);
        tweak(&mut pkg.manifest);
        let digest = manifest_digest(&pkg.manifest);
        host.pins.push(test_pin("fixture.echo", &digest));
        host.packages.insert_verified(seal(pkg)).expect("insert");
        digest
    }

    const LOOPER_WAT: &str = r#"(module
      (memory (export "memory") 1)
      (func (export "model_execute") (param i32 i32 i32 i32) (result i32)
        (loop $spin (br $spin))
        i32.const 0))"#;

    fn run(
        host: &mut ExactModelHostConnector,
        digest: &str,
        extras: serde_json::Map<String, Value>,
    ) -> Result<Vec<u8>, HostConnectorError> {
        let input_ref = host.io.stage_model_input(b"abc", 64).expect("stage");
        let result = host.invoke(&execute_request(digest, &input_ref, extras))?;
        Ok(host
            .io
            .read_model_output(
                result.artifact_ref.as_deref().expect("artifact_ref"),
                70_000,
            )
            .expect("read"))
    }

    #[test]
    fn wasmi_engine_produces_the_same_outputs_as_wasmtime() {
        let (mut host, digest) = seeded_classifier_host();
        let frame = classifier_input_frame([1.0, 2.0, -1.0, 4.0]);
        let mut outputs = Vec::new();
        for engine in [ModelEngine::Wasmtime, ModelEngine::Wasmi] {
            host.engine = engine;
            let input_ref = host.io.stage_model_input(&frame, 4096).expect("stage");
            let result = host
                .invoke(&classifier_execute_request(&digest, &input_ref))
                .expect("execute");
            outputs.push(
                host.io
                    .read_model_output(result.artifact_ref.as_deref().expect("ref"), 4096)
                    .expect("read"),
            );
        }
        assert_eq!(outputs[0], outputs[1]);

        let (mut echo, echo_digest) = seeded_host();
        echo.engine = ModelEngine::Wasmi;
        assert_eq!(
            run(&mut echo, &echo_digest, serde_json::Map::new()).expect("echo"),
            b"abc"
        );
    }

    #[test]
    fn wasmi_engine_fails_closed_like_wasmtime() {
        let (mut host, _) = seeded_host();
        host.engine = ModelEngine::Wasmi;
        let code = |host: &mut ExactModelHostConnector, digest: &str| {
            run(host, digest, serde_json::Map::new())
                .expect_err("fails")
                .code
        };
        let bad = add_variant(&mut host, b"not-wasm".to_vec(), |_| {});
        assert_eq!(
            code(&mut host, &bad),
            HostConnectorErrorCode::ModelIncompatible
        );
        let no_export = add_variant(
            &mut host,
            wat::parse_str(r#"(module (memory (export "memory") 1))"#).expect("wat"),
            |_| {},
        );
        assert_eq!(
            code(&mut host, &no_export),
            HostConnectorErrorCode::ModelIncompatible
        );
        let no_memory = add_variant(
            &mut host,
            wat::parse_str(
                r#"(module (func (export "model_execute") (param i32 i32 i32 i32) (result i32) i32.const 0))"#,
            )
            .expect("wat"),
            |_| {},
        );
        assert_eq!(
            code(&mut host, &no_memory),
            HostConnectorErrorCode::ModelIncompatible
        );
        let imports = add_variant(
            &mut host,
            wat::parse_str(
                r#"(module (import "env" "f" (func)) (memory (export "memory") 1)
                   (func (export "model_execute") (param i32 i32 i32 i32) (result i32) i32.const 0))"#,
            )
            .expect("wat"),
            |_| {},
        );
        assert_eq!(
            code(&mut host, &imports),
            HostConnectorErrorCode::ExecutionFailed
        );
        let trap = add_variant(
            &mut host,
            wat::parse_str(
                r#"(module (memory (export "memory") 1)
                   (func (export "model_execute") (param i32 i32 i32 i32) (result i32) unreachable))"#,
            )
            .expect("wat"),
            |_| {},
        );
        assert_eq!(
            code(&mut host, &trap),
            HostConnectorErrorCode::ExecutionFailed
        );
        let bad_len = add_variant(
            &mut host,
            wat::parse_str(
                r#"(module (memory (export "memory") 1)
                   (func (export "model_execute") (param i32 i32 i32 i32) (result i32) i32.const -1))"#,
            )
            .expect("wat"),
            |_| {},
        );
        assert_eq!(
            code(&mut host, &bad_len),
            HostConnectorErrorCode::ResourceExhausted
        );
        // Output window spanning a second page: grow succeeds under the
        // ceiling and fails when the store limiter caps memory at one page.
        let grow = wat::parse_str(
            r#"(module (memory (export "memory") 1)
               (func (export "model_execute") (param i32 i32 i32 i32) (result i32) (local.get 1)))"#,
        )
        .expect("wat");
        let grows = add_variant(&mut host, grow.clone(), |m| {
            m.max_memory_bytes = 4 * 64 * 1024;
            m.max_output_bytes = 70_000;
        });
        let capped = add_variant(&mut host, grow, |m| {
            m.max_memory_bytes = 64 * 1024;
            m.max_output_bytes = 70_000;
            m.max_input_bytes = 4095;
        });
        host.policies
            .get_mut("policy-1")
            .expect("policy")
            .max_output_bytes = 70_000;
        let mut big = serde_json::Map::new();
        big.insert("max_output_bytes".to_string(), json!(70_000));
        assert_eq!(run(&mut host, &grows, big.clone()).expect("grow").len(), 3);
        assert_eq!(
            run(&mut host, &capped, big).expect_err("capped").code,
            HostConnectorErrorCode::ResourceExhausted
        );
    }

    #[test]
    fn wasmi_engine_slices_fuel_and_interrupts_mid_run() {
        let (mut host, _) = seeded_host();
        host.engine = ModelEngine::Wasmi;
        let looper = wat::parse_str(LOOPER_WAT).expect("wat");
        // Exhausts fuel across several resumed slices.
        let multi = add_variant(&mut host, looper.clone(), |m| {
            m.max_fuel = 3 * WASMI_FUEL_SLICE + 7;
        });
        assert_eq!(
            run(&mut host, &multi, serde_json::Map::new())
                .expect_err("fuel")
                .code,
            HostConnectorErrorCode::ExecutionFailed
        );
        // Deadline reached mid-run (first slice boundary) -> timeout.
        let long = add_variant(&mut host, looper, |m| m.max_fuel = 1_000 * WASMI_FUEL_SLICE);
        let mut zero = serde_json::Map::new();
        zero.insert("timeout_ms".to_string(), json!(0));
        assert_eq!(
            run(&mut host, &long, zero).expect_err("timeout").code,
            HostConnectorErrorCode::Timeout
        );
        // Cancellation observed at the next slice boundary.
        host.cancel.store(true, Ordering::SeqCst);
        assert_eq!(
            run(&mut host, &long, serde_json::Map::new())
                .expect_err("cancel")
                .code,
            HostConnectorErrorCode::Cancelled
        );
        // The connector never clears the caller-managed flag itself.
        assert!(host.cancel.load(Ordering::SeqCst));
        // Host fuel ceiling intersects the manifest's.
        host.cancel.store(false, Ordering::SeqCst);
        host.host_limits.max_fuel = WASMI_FUEL_SLICE / 2;
        assert_eq!(
            run(&mut host, &long, serde_json::Map::new())
                .expect_err("host fuel")
                .code,
            HostConnectorErrorCode::ExecutionFailed
        );
    }

    #[test]
    fn registration_enforces_host_ceilings() {
        let manifest = read_fixture("model.manifest.json");
        let wasm = read_fixture("model.wasm");
        let sig = read_fixture("model.sig.json");
        let package_bytes = (manifest.len() + wasm.len()) as u64;
        let parsed: ModelPackageManifest = serde_json::from_slice(&manifest).expect("manifest");
        let tight = [
            HostModelLimits {
                max_package_bytes: package_bytes - 1,
                ..HostModelLimits::default()
            },
            HostModelLimits {
                max_memory_bytes: parsed.max_memory_bytes - 1,
                ..HostModelLimits::default()
            },
            HostModelLimits {
                max_fuel: parsed.max_fuel - 1,
                ..HostModelLimits::default()
            },
        ];
        for limits in tight {
            let mut host = trusted_host(vec![classifier_pin(&manifest)]);
            host.host_limits = limits;
            let err = host
                .register_package(&manifest, wasm.clone(), &sig)
                .expect_err("over host ceiling");
            assert_eq!(err.code, HostConnectorErrorCode::ModelIncompatible);
            assert_eq!(err.reason, Some(ModelFailureReason::HostLimitExceeded));
        }
        let mut host = trusted_host(vec![classifier_pin(&manifest)]);
        host.host_limits = HostModelLimits {
            max_package_bytes: package_bytes,
            max_memory_bytes: parsed.max_memory_bytes,
            max_fuel: parsed.max_fuel,
        };
        host.register_package(&manifest, wasm, &sig)
            .expect("exactly at the ceilings registers");
    }
    const ECHO_V2_DIR: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/models/fixture-echo-v2-1.0.0"
    );

    /// A v2 guest whose `model_alloc` body is `alloc` (param `$len`).
    fn v2_guest(alloc: &str) -> Vec<u8> {
        wat::parse_str(format!(
            r#"(module (memory (export "memory") 1)
               (func (export "model_alloc") (param $len i32) (result i32) {alloc})
               (func (export "model_execute") (param i32 i32 i32 i32) (result i32) i32.const 0))"#
        ))
        .expect("wat")
    }

    #[test]
    fn abi_v2_guest_matches_the_v1_echo_on_both_engines() {
        let v2 = std::fs::read(format!("{ECHO_V2_DIR}/model.wasm")).expect("v2 wasm");
        for engine in [ModelEngine::Wasmtime, ModelEngine::Wasmi] {
            let (mut host, v1_digest) = seeded_host();
            host.engine = engine;
            let v2_digest = add_variant(&mut host, v2.clone(), |m| m.abi_version = 2);
            let v1 = run(&mut host, &v1_digest, serde_json::Map::new()).expect("v1");
            let out = run(&mut host, &v2_digest, serde_json::Map::new()).expect("v2");
            assert_eq!(out, b"abc", "{engine:?}");
            assert_eq!(out, v1, "{engine:?}");
            // Input larger than the fixed-offset window still round-trips:
            // the guest allocator grows memory itself.
            let big = vec![7_u8; 3000];
            let input_ref = host.io.stage_model_input(&big, 4096).expect("stage");
            let result = host
                .invoke(&execute_request(
                    &v2_digest,
                    &input_ref,
                    serde_json::Map::new(),
                ))
                .expect("big v2");
            let output = host
                .io
                .read_model_output(result.artifact_ref.as_deref().expect("ref"), 4096)
                .expect("read");
            assert_eq!(output, big, "{engine:?}");
        }
    }

    #[test]
    fn checked_in_signed_v2_fixture_registers_and_runs() {
        let read = |name: &str| std::fs::read(format!("{ECHO_V2_DIR}/{name}")).expect(name);
        let manifest = read("model.manifest.json");
        let parsed: ModelPackageManifest = serde_json::from_slice(&manifest).expect("manifest");
        assert_eq!(parsed.abi_version, 2);
        let mut host = trusted_host(vec![test_pin("fixture.echo-v2", &digest_hex(&manifest))]);
        host.register_package(&manifest, read("model.wasm"), &read("model.sig.json"))
            .expect("signed v2 fixture registers");
    }

    #[test]
    fn abi_v2_fails_closed_on_bad_allocators() {
        let v1_echo = wat::parse_str(FIXTURE_ECHO_WAT).expect("wat");
        let cases: [(Vec<u8>, HostConnectorErrorCode); 6] = [
            (v1_echo, HostConnectorErrorCode::ModelIncompatible),
            (
                v2_guest("unreachable"),
                HostConnectorErrorCode::ExecutionFailed,
            ),
            (
                v2_guest("i32.const 0"),
                HostConnectorErrorCode::ExecutionFailed,
            ),
            (
                v2_guest("i32.const -8"),
                HostConnectorErrorCode::ExecutionFailed,
            ),
            (
                v2_guest("i32.const 2147483000"),
                HostConnectorErrorCode::ExecutionFailed,
            ),
            (
                v2_guest("i32.const 1024"),
                HostConnectorErrorCode::ExecutionFailed,
            ),
        ];
        for engine in [ModelEngine::Wasmtime, ModelEngine::Wasmi] {
            let (mut host, _) = seeded_host();
            host.engine = engine;
            for (index, (wasm, code)) in cases.iter().enumerate() {
                let digest = add_variant(&mut host, wasm.clone(), |m| m.abi_version = 2);
                assert_eq!(
                    run(&mut host, &digest, serde_json::Map::new())
                        .expect_err("fails closed")
                        .code,
                    *code,
                    "{engine:?} case {index}"
                );
            }
        }
        assert!(check_v2_regions(64, 8, 128, 8, 256).is_ok());
        assert!(
            check_v2_regions(64, 8, 128, 8, 130).is_err(),
            "output past memory"
        );
        assert!(check_v2_regions(64, 300, 128, 8, 4096).is_err(), "overlap");
        assert!(check_v2_regions(64, 8, -1, 8, 4096).is_err(), "negative");
    }

    #[test]
    fn manifest_rejects_unknown_guest_abi_versions() {
        let mut future = fixture_package();
        future.manifest.abi_version = MAX_MODEL_ABI_VERSION + 1;
        assert_eq!(
            future.manifest.validate().expect_err("abi 3").reason,
            Some(ModelFailureReason::ManifestInvalid)
        );
    }
}
