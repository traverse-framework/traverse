//! Security and identity controls for governed runtime execution.

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use traverse_registry::{
    ArtifactSignature, ArtifactSignatureScheme, ResolvedCapability, SourceKind,
};
use zeroize::Zeroizing;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeIdentity {
    pub subject_id: String,
    #[serde(default)]
    pub actor_id: Option<String>,
    pub token_reference_hash: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeSecurityMode {
    Production,
    Development,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeSecurityConfig {
    pub mode: RuntimeSecurityMode,
}

impl RuntimeSecurityConfig {
    #[must_use]
    pub fn production() -> Self {
        Self {
            mode: RuntimeSecurityMode::Production,
        }
    }

    #[must_use]
    pub fn development() -> Self {
        Self {
            mode: RuntimeSecurityMode::Development,
        }
    }
}

impl Default for RuntimeSecurityConfig {
    /// Production is the default security posture: unsigned local artifacts are
    /// rejected unless a caller explicitly opts into [`Self::development`]
    /// (spec 030-security-identity-model FR-013).
    fn default() -> Self {
        Self::production()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactVerificationRecord {
    pub status: ArtifactVerificationStatus,
    pub trust_level: ArtifactTrustLevel,
    #[serde(default)]
    pub scheme: Option<ArtifactVerificationScheme>,
    #[serde(default)]
    pub warning_code: Option<String>,
    #[serde(default)]
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactVerificationStatus {
    Verified,
    Warning,
    Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactTrustLevel {
    LocalDev,
    PublishedGoverned,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactVerificationScheme {
    Ed25519,
    Sigstore,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeWarning {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtifactVerificationFailure {
    MissingChecksum(ArtifactVerificationRecord),
    ChecksumMismatch(ArtifactVerificationRecord),
    MissingSignature(ArtifactVerificationRecord),
    SignatureVerificationFailed(ArtifactVerificationRecord),
    SigstoreUnreachable(ArtifactVerificationRecord),
}

impl ArtifactVerificationFailure {
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::MissingChecksum(_) => "missing_checksum",
            Self::ChecksumMismatch(_) => "checksum_mismatch",
            Self::MissingSignature(_) => "missing_signature",
            Self::SignatureVerificationFailed(_) => "signature_verification_failed",
            Self::SigstoreUnreachable(_) => "sigstore_unreachable",
        }
    }

    #[must_use]
    pub fn record(&self) -> &ArtifactVerificationRecord {
        match self {
            Self::MissingChecksum(record)
            | Self::ChecksumMismatch(record)
            | Self::MissingSignature(record)
            | Self::SignatureVerificationFailed(record)
            | Self::SigstoreUnreachable(record) => record,
        }
    }
}

/// Attribute a caller identity from a JWT bearer token for **tracing and audit
/// only**.
///
/// This decodes the token payload without verifying its signature and returns a
/// [`RuntimeIdentity`] (subject, optional actor, token hash) used purely to
/// label execution traces. It carries **no privilege claim** and MUST NOT be
/// used for any authorization decision. Access control lives at the HTTP
/// boundary (`traverse-cli` `http_api`), which verifies the JWT signature and
/// an `alg` allow-list before honoring any privileged claim (see spec
/// 033-http-json-api and issue #580).
#[must_use]
pub fn derive_identity_from_jwt(token: &str) -> Option<RuntimeIdentity> {
    let mut parts = token.split('.');
    let header = parts.next();
    let payload = parts.next();
    let signature = parts.next();
    if header.is_none() || payload.is_none() || signature.is_none() || parts.next().is_some() {
        return None;
    }
    let payload = payload?;
    // Zeroize the decoded credential bytes when this scope ends, on both the
    // success and the early-return paths (spec 030 NFR-001).
    let payload_bytes = Zeroizing::new(base64url_decode(payload).ok()?);
    let value = serde_json::from_slice::<Value>(&payload_bytes).ok()?;
    let subject_id = value
        .get("sub")
        .and_then(Value::as_str)
        .filter(|sub| !sub.trim().is_empty())?
        .to_string();
    let actor_id = value
        .get("act")
        .and_then(|act| act.get("sub"))
        .and_then(Value::as_str)
        .filter(|actor| !actor.trim().is_empty())
        .map(ToString::to_string);
    Some(RuntimeIdentity {
        subject_id,
        actor_id,
        token_reference_hash: sha256_hex(token.as_bytes()),
    })
}

/// Verify artifact trust metadata before execution.
///
/// # Errors
///
/// Returns [`ArtifactVerificationFailure`] when a governed artifact is missing a
/// required signature, an Ed25519 signature does not verify, or Sigstore
/// verification cannot be completed.
pub fn verify_artifact(
    capability: &ResolvedCapability,
    artifact_bytes: &[u8],
    config: &RuntimeSecurityConfig,
) -> Result<ArtifactVerificationRecord, ArtifactVerificationFailure> {
    let trust_level = artifact_trust_level(capability);
    let Some(binary) = capability.artifact.binary.as_ref() else {
        return Ok(verified_local_record(trust_level));
    };
    let Some(signature) = binary.signature.as_ref() else {
        if trust_level == ArtifactTrustLevel::LocalDev
            && config.mode == RuntimeSecurityMode::Development
        {
            return Ok(ArtifactVerificationRecord {
                status: ArtifactVerificationStatus::Warning,
                trust_level,
                scheme: None,
                warning_code: Some("unsigned_local_dev_artifact".to_string()),
                error_code: None,
            });
        }
        let record = rejected_record(trust_level, None, "missing_signature");
        return Err(ArtifactVerificationFailure::MissingSignature(record));
    };

    let verification = match signature.scheme {
        ArtifactSignatureScheme::Ed25519 => verify_ed25519(signature, artifact_bytes, trust_level),
        ArtifactSignatureScheme::Sigstore => verify_sigstore(signature, trust_level),
    }?;
    verify_checksum(capability, artifact_bytes, trust_level)?;
    Ok(verification)
}

fn verify_checksum(
    capability: &ResolvedCapability,
    artifact_bytes: &[u8],
    trust_level: ArtifactTrustLevel,
) -> Result<(), ArtifactVerificationFailure> {
    if trust_level != ArtifactTrustLevel::PublishedGoverned {
        return Ok(());
    }
    let Some(expected) = capability.artifact.digests.binary_digest.as_deref() else {
        return Err(ArtifactVerificationFailure::MissingChecksum(
            rejected_record(trust_level, None, "missing_checksum"),
        ));
    };
    let expected = expected.strip_prefix("sha256:").unwrap_or(expected);
    let actual = sha256_hex(artifact_bytes);
    if expected.eq_ignore_ascii_case(&actual) {
        Ok(())
    } else {
        Err(ArtifactVerificationFailure::ChecksumMismatch(
            rejected_record(trust_level, None, "checksum_mismatch"),
        ))
    }
}

/// Classifies an artifact's trust level per spec 030-security-identity-model
/// FR-007/FR-008.
///
/// `SourceKind::Local` is a structured provenance field set at registration
/// time (for example, the canonical bundled example capabilities), not a
/// path heuristic; it always yields `LocalDev`. Everything else is resolved
/// against the governed-path registry (`contracts/` and every path declared
/// in `specs/governance/approved-specs.json`'s `governs` lists) rather than
/// pattern-matching the contract path or source URL for substrings, which a
/// registrant fully controls and could spoof in either direction.
fn artifact_trust_level(capability: &ResolvedCapability) -> ArtifactTrustLevel {
    if capability.artifact.source.kind == SourceKind::Local {
        return ArtifactTrustLevel::LocalDev;
    }
    if traverse_registry::is_governed_artifact_path(&capability.record.contract_path) {
        ArtifactTrustLevel::PublishedGoverned
    } else {
        ArtifactTrustLevel::LocalDev
    }
}

fn verify_ed25519(
    signature: &ArtifactSignature,
    artifact_bytes: &[u8],
    trust_level: ArtifactTrustLevel,
) -> Result<ArtifactVerificationRecord, ArtifactVerificationFailure> {
    let Some(public_key_hex) = signature.public_key_hex.as_deref() else {
        let record = rejected_record(
            trust_level,
            Some(ArtifactVerificationScheme::Ed25519),
            "signature_verification_failed",
        );
        return Err(ArtifactVerificationFailure::SignatureVerificationFailed(
            record,
        ));
    };
    let Some(signature_hex) = signature.signature_hex.as_deref() else {
        let record = rejected_record(
            trust_level,
            Some(ArtifactVerificationScheme::Ed25519),
            "signature_verification_failed",
        );
        return Err(ArtifactVerificationFailure::SignatureVerificationFailed(
            record,
        ));
    };
    let Ok(public_key_bytes) = hex_decode(public_key_hex) else {
        let record = rejected_record(
            trust_level,
            Some(ArtifactVerificationScheme::Ed25519),
            "signature_verification_failed",
        );
        return Err(ArtifactVerificationFailure::SignatureVerificationFailed(
            record,
        ));
    };
    let Ok(signature_bytes) = hex_decode(signature_hex) else {
        let record = rejected_record(
            trust_level,
            Some(ArtifactVerificationScheme::Ed25519),
            "signature_verification_failed",
        );
        return Err(ArtifactVerificationFailure::SignatureVerificationFailed(
            record,
        ));
    };
    let Ok(public_key_array) = <[u8; 32]>::try_from(public_key_bytes.as_slice()) else {
        let record = rejected_record(
            trust_level,
            Some(ArtifactVerificationScheme::Ed25519),
            "signature_verification_failed",
        );
        return Err(ArtifactVerificationFailure::SignatureVerificationFailed(
            record,
        ));
    };
    let Ok(signature_array) = <[u8; 64]>::try_from(signature_bytes.as_slice()) else {
        return Err(signature_verification_failed(trust_level));
    };
    let key = VerifyingKey::from_bytes(&public_key_array)
        .map_err(|_| signature_verification_failed(trust_level))?;
    let signature = Signature::from_bytes(&signature_array);
    if key.verify(artifact_bytes, &signature).is_err() {
        return Err(signature_verification_failed(trust_level));
    }
    Ok(ArtifactVerificationRecord {
        status: ArtifactVerificationStatus::Verified,
        trust_level,
        scheme: Some(ArtifactVerificationScheme::Ed25519),
        warning_code: None,
        error_code: None,
    })
}

fn signature_verification_failed(trust_level: ArtifactTrustLevel) -> ArtifactVerificationFailure {
    ArtifactVerificationFailure::SignatureVerificationFailed(rejected_record(
        trust_level,
        Some(ArtifactVerificationScheme::Ed25519),
        "signature_verification_failed",
    ))
}

fn verify_sigstore(
    _signature: &ArtifactSignature,
    trust_level: ArtifactTrustLevel,
) -> Result<ArtifactVerificationRecord, ArtifactVerificationFailure> {
    let record = rejected_record(
        trust_level,
        Some(ArtifactVerificationScheme::Sigstore),
        "sigstore_unreachable",
    );
    Err(ArtifactVerificationFailure::SigstoreUnreachable(record))
}

fn verified_local_record(trust_level: ArtifactTrustLevel) -> ArtifactVerificationRecord {
    ArtifactVerificationRecord {
        status: ArtifactVerificationStatus::Verified,
        trust_level,
        scheme: None,
        warning_code: None,
        error_code: None,
    }
}

fn rejected_record(
    trust_level: ArtifactTrustLevel,
    scheme: Option<ArtifactVerificationScheme>,
    error_code: &str,
) -> ArtifactVerificationRecord {
    ArtifactVerificationRecord {
        status: ArtifactVerificationStatus::Rejected,
        trust_level,
        scheme,
        warning_code: None,
        error_code: Some(error_code.to_string()),
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        output.push(char::from(HEX_TABLE[(byte >> 4) as usize]));
        output.push(char::from(HEX_TABLE[(byte & 0x0f) as usize]));
    }
    output
}

const HEX_TABLE: &[u8; 16] = b"0123456789abcdef";

fn hex_decode(input: &str) -> Result<Vec<u8>, ()> {
    if !input.len().is_multiple_of(2) {
        return Err(());
    }
    let mut output = Vec::with_capacity(input.len() / 2);
    for pair in input.as_bytes().chunks_exact(2) {
        let high = hex_nibble(pair[0])?;
        let low = hex_nibble(pair[1])?;
        output.push((high << 4) | low);
    }
    Ok(output)
}

fn hex_nibble(byte: u8) -> Result<u8, ()> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(()),
    }
}

fn base64url_decode(input: &str) -> Result<Vec<u8>, ()> {
    if input.contains('=') {
        return Err(());
    }
    let mut sextets = Vec::with_capacity(input.len());
    for ch in input.chars() {
        let val = match ch {
            'A'..='Z' => (ch as u8) - b'A',
            'a'..='z' => (ch as u8) - b'a' + 26,
            '0'..='9' => (ch as u8) - b'0' + 52,
            '-' => 62,
            '_' => 63,
            _ => return Err(()),
        };
        sextets.push(val);
    }
    match sextets.len() % 4 {
        0 | 2 | 3 => {}
        _ => return Err(()),
    }
    let mut out = Vec::with_capacity((sextets.len() * 3) / 4);
    let mut i = 0;
    while i + 4 <= sextets.len() {
        let n = (u32::from(sextets[i]) << 18)
            | (u32::from(sextets[i + 1]) << 12)
            | (u32::from(sextets[i + 2]) << 6)
            | u32::from(sextets[i + 3]);
        out.push(((n >> 16) & 0xff) as u8);
        out.push(((n >> 8) & 0xff) as u8);
        out.push((n & 0xff) as u8);
        i += 4;
    }
    let rem = sextets.len() - i;
    if rem == 2 {
        let n = (u32::from(sextets[i]) << 18) | (u32::from(sextets[i + 1]) << 12);
        out.push(((n >> 16) & 0xff) as u8);
    } else if rem == 3 {
        let n = (u32::from(sextets[i]) << 18)
            | (u32::from(sextets[i + 1]) << 12)
            | (u32::from(sextets[i + 2]) << 6);
        out.push(((n >> 16) & 0xff) as u8);
        out.push(((n >> 8) & 0xff) as u8);
    }
    Ok(out)
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use serde_json::json;
    use traverse_contracts::{
        CapabilityContract, Entrypoint, EntrypointKind, Execution, ExecutionConstraints,
        ExecutionTarget, FilesystemAccess, HostApiAccess, Lifecycle, NetworkAccess, Owner,
        Provenance, ProvenanceSource, SchemaContainer, ServiceType,
    };
    use traverse_registry::{
        ArtifactDigests, BinaryFormat, BinaryReference, CapabilityArtifactRecord,
        CapabilityRegistryRecord, ComposabilityMetadata, CompositionKind, CompositionPattern,
        DiscoveryIndexEntry, ImplementationKind, RegistrationEvidence, RegistrationResult,
        RegistryProvenance, RegistryScope, SourceReference,
    };

    #[allow(clippy::too_many_lines)]
    fn test_capability(
        contract_path: &str,
        source_kind: SourceKind,
        binary: Option<BinaryReference>,
    ) -> ResolvedCapability {
        let owner = Owner {
            team: "comments".to_string(),
            contact: "comments@example.com".to_string(),
        };
        let contract = CapabilityContract {
            kind: "capability_contract".to_string(),
            schema_version: "1.0.0".to_string(),
            id: "content.comments.create-comment-draft".to_string(),
            namespace: "content.comments".to_string(),
            name: "create-comment-draft".to_string(),
            version: "1.0.0".to_string(),
            lifecycle: Lifecycle::Active,
            owner: owner.clone(),
            summary: "Create a comment draft for a resource".to_string(),
            description: "Creates a draft comment and returns the generated draft identifier."
                .to_string(),
            inputs: SchemaContainer {
                schema: json!({"type": "object"}),
            },
            outputs: SchemaContainer {
                schema: json!({"type": "object"}),
            },
            preconditions: Vec::new(),
            postconditions: Vec::new(),
            side_effects: vec![traverse_contracts::SideEffect {
                kind: traverse_contracts::SideEffectKind::MemoryOnly,
                description: "Produces a draft representation in memory.".to_string(),
            }],
            emits: Vec::new(),
            consumes: Vec::new(),
            permissions: Vec::new(),
            execution: Execution {
                binary_format: traverse_contracts::BinaryFormat::Wasm,
                entrypoint: Entrypoint {
                    kind: EntrypointKind::WasiCommand,
                    command: "run".to_string(),
                },
                preferred_targets: vec![ExecutionTarget::Local],
                constraints: ExecutionConstraints {
                    host_api_access: HostApiAccess::None,
                    network_access: NetworkAccess::Forbidden,
                    filesystem_access: FilesystemAccess::None,
                },
            },
            policies: Vec::new(),
            dependencies: Vec::new(),
            provenance: Provenance {
                source: ProvenanceSource::Greenfield,
                author: "Enrico Piovesan".to_string(),
                created_at: "2026-03-27T00:00:00Z".to_string(),
                spec_ref: Some("030-security-identity-model".to_string()),
                adr_refs: Vec::new(),
                exception_refs: Vec::new(),
            },
            evidence: Vec::new(),
            service_type: ServiceType::Stateless,
            permitted_targets: vec![ExecutionTarget::Local],
            event_trigger: None,
            connector_requirements: Vec::new(),
            state_schema: None,
        };
        let record = CapabilityRegistryRecord {
            scope: RegistryScope::Private,
            id: contract.id.clone(),
            version: contract.version.clone(),
            lifecycle: Lifecycle::Active,
            owner: owner.clone(),
            contract_path: contract_path.to_string(),
            contract_digest: "digest".to_string(),
            implementation_kind: ImplementationKind::Executable,
            artifact_ref: "artifact:content.comments.create-comment-draft:1.0.0".to_string(),
            registered_at: "2026-03-27T00:00:00Z".to_string(),
            provenance: RegistryProvenance {
                source: "test".to_string(),
                author: "Enrico Piovesan".to_string(),
                created_at: "2026-03-27T00:00:00Z".to_string(),
            },
            evidence: RegistrationEvidence {
                evidence_id: "evidence".to_string(),
                artifact_ref: "artifact:content.comments.create-comment-draft:1.0.0".to_string(),
                capability_id: contract.id.clone(),
                capability_version: contract.version.clone(),
                scope: RegistryScope::Private,
                governing_spec: "030-security-identity-model".to_string(),
                validator_version: "0.1.0".to_string(),
                produced_at: "2026-03-27T00:00:00Z".to_string(),
                result: RegistrationResult::Passed,
            },
        };
        let artifact = CapabilityArtifactRecord {
            artifact_ref: "artifact:content.comments.create-comment-draft:1.0.0".to_string(),
            implementation_kind: ImplementationKind::Executable,
            source: SourceReference {
                kind: source_kind,
                location: "https://github.com/traverse-framework/traverse".to_string(),
            },
            binary,
            workflow_ref: None,
            digests: ArtifactDigests {
                source_digest: "src-digest".to_string(),
                binary_digest: None,
            },
            provenance: RegistryProvenance {
                source: "test".to_string(),
                author: "Enrico Piovesan".to_string(),
                created_at: "2026-03-27T00:00:00Z".to_string(),
            },
        };
        let index_entry = DiscoveryIndexEntry {
            scope: RegistryScope::Private,
            id: contract.id.clone(),
            version: contract.version.clone(),
            lifecycle: Lifecycle::Active,
            owner,
            summary: "Create a comment draft for a resource".to_string(),
            tags: Vec::new(),
            permissions: Vec::new(),
            emits: Vec::new(),
            consumes: Vec::new(),
            implementation_kind: ImplementationKind::Executable,
            composability: ComposabilityMetadata {
                kind: CompositionKind::Atomic,
                patterns: vec![CompositionPattern::Sequential],
                provides: Vec::new(),
                requires: Vec::new(),
            },
            artifact_ref: "artifact:content.comments.create-comment-draft:1.0.0".to_string(),
            registered_at: "2026-03-27T00:00:00Z".to_string(),
        };
        ResolvedCapability {
            contract,
            record,
            artifact,
            index_entry,
        }
    }

    fn signed_binary(bytes: &[u8]) -> BinaryReference {
        let signing_key = SigningKey::from_bytes(&[9_u8; 32]);
        let signature = signing_key.sign(bytes);
        BinaryReference {
            format: BinaryFormat::Wasm,
            location: "unused.wasm".to_string(),
            signature: Some(ArtifactSignature {
                scheme: ArtifactSignatureScheme::Ed25519,
                public_key_hex: Some(hex_encode(signing_key.verifying_key().as_bytes())),
                signature_hex: Some(hex_encode(&signature.to_bytes())),
                sigstore_bundle_ref: None,
            }),
        }
    }

    fn unsigned_binary() -> BinaryReference {
        BinaryReference {
            format: BinaryFormat::Wasm,
            location: "unused.wasm".to_string(),
            signature: None,
        }
    }

    fn hex_encode(bytes: &[u8]) -> String {
        let mut out = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            out.push(char::from(HEX_TABLE[(byte >> 4) as usize]));
            out.push(char::from(HEX_TABLE[(byte & 0x0f) as usize]));
        }
        out
    }

    // ------------------------------------------------------------------
    // artifact_trust_level classification (spec 030 FR-007/FR-008)
    // ------------------------------------------------------------------

    #[test]
    fn local_source_is_always_local_dev_regardless_of_contract_path() {
        let capability = test_capability(
            "contracts/approved/comment-draft.json",
            SourceKind::Local,
            None,
        );
        assert_eq!(
            artifact_trust_level(&capability),
            ArtifactTrustLevel::LocalDev
        );
    }

    #[test]
    fn contract_under_contracts_directory_is_published_governed() {
        let capability = test_capability(
            "contracts/approved/comment-draft.json",
            SourceKind::Git,
            None,
        );
        assert_eq!(
            artifact_trust_level(&capability),
            ArtifactTrustLevel::PublishedGoverned
        );
    }

    #[test]
    fn contract_outside_any_governed_path_is_local_dev() {
        let capability = test_capability(
            "workspaces/ws-test/registry/private/comment-draft@1.0.0/contract.json",
            SourceKind::Git,
            None,
        );
        assert_eq!(
            artifact_trust_level(&capability),
            ArtifactTrustLevel::LocalDev
        );
    }

    #[test]
    fn path_containing_specs_substring_outside_a_governed_prefix_is_not_governed() {
        // Regression guard: the old heuristic treated any path containing
        // "/specs/" anywhere as governed. A workspace-local path that merely
        // has a "specs" segment must not spoof governed trust.
        let capability = test_capability("my-app/specs/comment-draft.json", SourceKind::Git, None);
        assert_eq!(
            artifact_trust_level(&capability),
            ArtifactTrustLevel::LocalDev
        );
    }

    #[test]
    fn approved_keyword_in_url_and_path_no_longer_spoofs_governed_trust() {
        // Regression guard: the old heuristic granted governed trust to any
        // Git+https source whose contract path merely contained the word
        // "approved", regardless of whether it was actually registry-governed.
        let mut capability = test_capability(
            "workspaces/ws-test/approved/comment-draft.json",
            SourceKind::Git,
            None,
        );
        capability.artifact.source.location = "https://example.com/not-governed".to_string();
        assert_eq!(
            artifact_trust_level(&capability),
            ArtifactTrustLevel::LocalDev
        );
    }

    // ------------------------------------------------------------------
    // verify_artifact end-to-end trust enforcement
    // ------------------------------------------------------------------

    #[test]
    fn published_governed_unsigned_artifact_is_rejected_even_in_development_mode() {
        let capability = test_capability(
            "contracts/approved/comment-draft.json",
            SourceKind::Git,
            Some(unsigned_binary()),
        );
        let result = verify_artifact(&capability, b"bytes", &RuntimeSecurityConfig::development());
        assert!(matches!(
            result,
            Err(ArtifactVerificationFailure::MissingSignature(_))
        ));
    }

    #[test]
    fn published_governed_signed_artifact_with_matching_checksum_verifies() {
        let bytes = b"wasm-bytes";
        let mut capability = test_capability(
            "contracts/approved/comment-draft.json",
            SourceKind::Git,
            Some(signed_binary(bytes)),
        );
        capability.artifact.digests.binary_digest = Some(format!("sha256:{}", sha256_hex(bytes)));
        let result = verify_artifact(&capability, bytes, &RuntimeSecurityConfig::production());
        let record = result.expect("signed governed artifact with matching checksum must verify");
        assert_eq!(record.status, ArtifactVerificationStatus::Verified);
        assert_eq!(record.trust_level, ArtifactTrustLevel::PublishedGoverned);
    }

    #[test]
    fn local_dev_unsigned_artifact_warns_in_development_mode() {
        let capability = test_capability(
            "workspaces/ws-test/registry/private/comment-draft@1.0.0/contract.json",
            SourceKind::Local,
            Some(unsigned_binary()),
        );
        let result = verify_artifact(&capability, b"bytes", &RuntimeSecurityConfig::development());
        let record =
            result.expect("unsigned local artifact must be allowed-but-warned in dev mode");
        assert_eq!(record.status, ArtifactVerificationStatus::Warning);
        assert_eq!(record.trust_level, ArtifactTrustLevel::LocalDev);
        assert_eq!(
            record.warning_code.as_deref(),
            Some("unsigned_local_dev_artifact")
        );
    }

    #[test]
    fn local_dev_unsigned_artifact_is_rejected_in_production_mode() {
        let capability = test_capability(
            "workspaces/ws-test/registry/private/comment-draft@1.0.0/contract.json",
            SourceKind::Local,
            Some(unsigned_binary()),
        );
        let result = verify_artifact(&capability, b"bytes", &RuntimeSecurityConfig::production());
        assert!(matches!(
            result,
            Err(ArtifactVerificationFailure::MissingSignature(_))
        ));
    }
}
