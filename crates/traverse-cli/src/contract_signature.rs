//! Spec 124 v0.2.0 FR-007–FR-010: verification of the registry spec 026
//! v1.1.0 contract signature carried in a capability's `signature.json`.
//!
//! `contract_sha256` is 64 lowercase hex: the SHA-256 of the exact committed
//! `contract.json` bytes. `contract_signature_hex` is an Ed25519 signature,
//! by the same key as the artifact signature, over those 32 raw digest
//! bytes. Nothing is canonicalized: the bytes hashed are exactly the bytes
//! the consumer read. A `signature.json` without both fields predates the
//! signing change and is reported as `absent` unless the host requires it.

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{decode_hex, sha256_hex};

/// Outcome recorded next to the artifact signature.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContractSignatureStatus {
    Verified,
    Absent,
}

impl ContractSignatureStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Verified => "verified",
            Self::Absent => "absent",
        }
    }
}

/// Stable failure suffixes; each surface prefixes its own namespace
/// (`artifact_materialize_`, `registry_prepare_`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContractSignatureError {
    /// Strict mode is on and the signature carries no contract fields.
    Missing,
    /// Only one of `contract_sha256` / `contract_signature_hex` is present.
    Incomplete,
    /// `contract_sha256` does not match the exact contract bytes.
    DigestMismatch,
    /// `contract_signature_hex` is not 128 hex characters.
    Invalid,
    /// The Ed25519 signature does not verify over the digest.
    Mismatch,
}

impl ContractSignatureError {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Missing => "contract_signature_missing",
            Self::Incomplete => "contract_signature_incomplete",
            Self::DigestMismatch => "contract_digest_mismatch",
            Self::Invalid => "contract_signature_invalid",
            Self::Mismatch => "contract_signature_mismatch",
        }
    }
}

/// Verify the contract signature fields of a parsed `signature.json`.
pub(crate) fn verify_contract_signature(
    signature: &Value,
    contract_bytes: &[u8],
    key: &VerifyingKey,
    require: bool,
) -> Result<ContractSignatureStatus, ContractSignatureError> {
    let field = |name: &str| {
        signature
            .get(name)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
    };
    let (declared_digest, signature_hex) =
        match (field("contract_sha256"), field("contract_signature_hex")) {
            (None, None) if require => return Err(ContractSignatureError::Missing),
            (None, None) => return Ok(ContractSignatureStatus::Absent),
            (Some(digest), Some(signature_hex)) => (digest, signature_hex),
            _ => return Err(ContractSignatureError::Incomplete),
        };
    if declared_digest != sha256_hex(contract_bytes) {
        return Err(ContractSignatureError::DigestMismatch);
    }
    let digest: [u8; 32] = Sha256::digest(contract_bytes).into();
    let signature_bytes: [u8; 64] = decode_hex(signature_hex)
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or(ContractSignatureError::Invalid)?;
    key.verify(&digest, &Signature::from_bytes(&signature_bytes))
        .map_err(|_| ContractSignatureError::Mismatch)?;
    Ok(ContractSignatureStatus::Verified)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encode_hex;
    use ed25519_dalek::{Signer, SigningKey};
    use serde_json::json;

    const CONTRACT: &[u8] = br#"{"id": "demo",  "version": "1.0.0", "score": 0.1000}"#;

    fn key() -> SigningKey {
        SigningKey::from_bytes(&[7; 32])
    }

    fn signed(contract: &[u8], signer: &SigningKey) -> Value {
        let digest: [u8; 32] = Sha256::digest(contract).into();
        json!({
            "scheme": "ed25519",
            "contract_sha256": encode_hex(&digest),
            "contract_signature_hex": encode_hex(&signer.sign(&digest).to_bytes()),
        })
    }

    #[test]
    fn valid_signature_over_exact_bytes_verifies() {
        let signer = key();
        let status = verify_contract_signature(
            &signed(CONTRACT, &signer),
            CONTRACT,
            &signer.verifying_key(),
            true,
        );
        assert_eq!(status, Ok(ContractSignatureStatus::Verified));
        assert_eq!(ContractSignatureStatus::Verified.as_str(), "verified");
    }

    #[test]
    fn reformatted_json_with_same_meaning_fails_the_digest() {
        let signer = key();
        let reformatted = br#"{"id":"demo","version":"1.0.0","score":0.1}"#;
        assert_eq!(
            verify_contract_signature(
                &signed(CONTRACT, &signer),
                reformatted,
                &signer.verifying_key(),
                false,
            ),
            Err(ContractSignatureError::DigestMismatch)
        );
    }

    #[test]
    fn wrong_key_and_tampered_signature_fail() {
        let signer = key();
        let other = SigningKey::from_bytes(&[9; 32]);
        let evidence = signed(CONTRACT, &signer);
        assert_eq!(
            verify_contract_signature(&evidence, CONTRACT, &other.verifying_key(), false),
            Err(ContractSignatureError::Mismatch)
        );
        let mut tampered = evidence.clone();
        tampered["contract_signature_hex"] = json!("00".repeat(64));
        assert_eq!(
            verify_contract_signature(&tampered, CONTRACT, &signer.verifying_key(), false),
            Err(ContractSignatureError::Mismatch)
        );
        for bad in ["zz".repeat(64), "00".repeat(10), "0".to_string()] {
            tampered["contract_signature_hex"] = json!(bad);
            assert_eq!(
                verify_contract_signature(&tampered, CONTRACT, &signer.verifying_key(), false),
                Err(ContractSignatureError::Invalid)
            );
        }
    }

    #[test]
    fn absent_fields_pass_unless_required_and_half_present_fails() {
        let signer = key();
        let legacy = json!({"scheme": "ed25519", "contract_sha256": " "});
        assert_eq!(
            verify_contract_signature(&legacy, CONTRACT, &signer.verifying_key(), false),
            Ok(ContractSignatureStatus::Absent)
        );
        assert_eq!(ContractSignatureStatus::Absent.as_str(), "absent");
        assert_eq!(
            verify_contract_signature(&legacy, CONTRACT, &signer.verifying_key(), true),
            Err(ContractSignatureError::Missing)
        );
        let mut half = signed(CONTRACT, &signer);
        half.as_object_mut()
            .map(|object| object.remove("contract_signature_hex"));
        assert_eq!(
            verify_contract_signature(&half, CONTRACT, &signer.verifying_key(), false),
            Err(ContractSignatureError::Incomplete)
        );
    }

    #[test]
    fn error_codes_are_stable() {
        let codes = [
            ContractSignatureError::Missing,
            ContractSignatureError::Incomplete,
            ContractSignatureError::DigestMismatch,
            ContractSignatureError::Invalid,
            ContractSignatureError::Mismatch,
        ]
        .map(ContractSignatureError::as_str);
        assert_eq!(
            codes,
            [
                "contract_signature_missing",
                "contract_signature_incomplete",
                "contract_digest_mismatch",
                "contract_signature_invalid",
                "contract_signature_mismatch",
            ]
        );
    }
}
