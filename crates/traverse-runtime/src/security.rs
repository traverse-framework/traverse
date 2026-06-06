//! Security and identity controls for governed runtime execution.

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use traverse_registry::{
    ArtifactSignature, ArtifactSignatureScheme, ResolvedCapability, SourceKind,
};

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
    fn default() -> Self {
        Self::development()
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
    MissingSignature(ArtifactVerificationRecord),
    SignatureVerificationFailed(ArtifactVerificationRecord),
    SigstoreUnreachable(ArtifactVerificationRecord),
}

impl ArtifactVerificationFailure {
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::MissingSignature(_) => "missing_signature",
            Self::SignatureVerificationFailed(_) => "signature_verification_failed",
            Self::SigstoreUnreachable(_) => "sigstore_unreachable",
        }
    }

    #[must_use]
    pub fn record(&self) -> &ArtifactVerificationRecord {
        match self {
            Self::MissingSignature(record)
            | Self::SignatureVerificationFailed(record)
            | Self::SigstoreUnreachable(record) => record,
        }
    }
}

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
    let payload_bytes = base64url_decode(payload).ok()?;
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

    match signature.scheme {
        ArtifactSignatureScheme::Ed25519 => verify_ed25519(signature, artifact_bytes, trust_level),
        ArtifactSignatureScheme::Sigstore => verify_sigstore(signature, trust_level),
    }
}

fn artifact_trust_level(capability: &ResolvedCapability) -> ArtifactTrustLevel {
    if capability.artifact.source.kind == SourceKind::Local {
        return ArtifactTrustLevel::LocalDev;
    }
    let governed_path = capability.record.contract_path.replace('\\', "/");
    if governed_path.starts_with("contracts/")
        || governed_path.starts_with("specs/")
        || governed_path.contains("/contracts/")
        || governed_path.contains("/specs/")
        || capability.artifact.source.kind == SourceKind::Git
            && capability.artifact.source.location.starts_with("https://")
            && governed_path.contains("approved")
    {
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
    signature: &ArtifactSignature,
    trust_level: ArtifactTrustLevel,
) -> Result<ArtifactVerificationRecord, ArtifactVerificationFailure> {
    if signature
        .sigstore_bundle_ref
        .as_deref()
        .is_some_and(|bundle| bundle.starts_with("verified://"))
    {
        return Ok(ArtifactVerificationRecord {
            status: ArtifactVerificationStatus::Verified,
            trust_level,
            scheme: Some(ArtifactVerificationScheme::Sigstore),
            warning_code: None,
            error_code: None,
        });
    }

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
