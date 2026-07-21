use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt::Write;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize)]
struct ArtifactManifest {
    artifact_path: Option<String>,
    checksum_algorithm: Option<String>,
    checksum_sha256: Option<String>,
    signing_scheme: Option<String>,
    signature: Option<String>,
    signature_hex: Option<String>,
    public_key_hex: Option<String>,
    sigstore_bundle_ref: Option<String>,
    provenance: Option<String>,
    provenance_path: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct ProvenanceStatement {
    source_commit_sha: Option<String>,
    build_system: Option<String>,
    artifact_sha256: Option<String>,
    build_invocation: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OverallStatus {
    Passed,
    Failed,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Matched,
    Verified,
    Missing,
    Mismatch,
    Invalid,
    UnsupportedChecksumAlgorithm,
    UnsupportedSignatureScheme,
    ReadError,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CheckEvidence {
    pub status: CheckStatus,
    pub message: String,
    pub expected: Option<String>,
    pub actual: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProvenanceEvidence {
    pub status: CheckStatus,
    pub message: String,
    pub source_commit_sha: Option<String>,
    pub build_system: Option<String>,
    pub artifact_sha256: Option<String>,
    pub build_invocation: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ArtifactVerificationReport {
    pub overall_status: OverallStatus,
    pub artifact_path: String,
    pub manifest_path: Option<String>,
    pub provenance_path: Option<String>,
    pub checksum_status: CheckStatus,
    pub signature_status: CheckStatus,
    pub provenance_status: CheckStatus,
    pub checksum: CheckEvidence,
    pub signature: CheckEvidence,
    pub provenance: ProvenanceEvidence,
    pub warnings: Vec<String>,
}

impl ArtifactVerificationReport {
    pub fn passed(&self) -> bool {
        self.overall_status == OverallStatus::Passed
    }
}

pub fn verify_artifact(input_path: &Path) -> ArtifactVerificationReport {
    let (manifest_path, manifest) = load_manifest(input_path);
    let artifact_path = artifact_path(input_path, manifest_path.as_deref(), manifest.as_ref());
    let artifact_bytes = fs::read(&artifact_path);
    let actual_sha256 = artifact_bytes.as_ref().ok().map(|bytes| sha256_hex(bytes));

    let checksum = verify_checksum(manifest.as_ref(), actual_sha256.as_deref(), &artifact_bytes);
    let signature = verify_signature(manifest.as_ref(), artifact_bytes.as_deref().ok());
    let (provenance_path, provenance) = verify_provenance(
        input_path,
        &artifact_path,
        manifest.as_ref(),
        actual_sha256.as_deref(),
    );

    let mut warnings = Vec::new();
    if manifest.is_none() {
        warnings.push("artifact manifest is missing".to_string());
    }
    if artifact_bytes.is_err() {
        warnings.push(format!(
            "artifact file is unreadable: {}",
            artifact_path.display()
        ));
    }

    let overall_status = if checksum.status == CheckStatus::Matched
        && signature.status == CheckStatus::Verified
        && provenance.status == CheckStatus::Verified
    {
        OverallStatus::Passed
    } else {
        OverallStatus::Failed
    };

    ArtifactVerificationReport {
        overall_status,
        artifact_path: artifact_path.display().to_string(),
        manifest_path: manifest_path.map(|path| path.display().to_string()),
        provenance_path: provenance_path.map(|path| path.display().to_string()),
        checksum_status: checksum.status.clone(),
        signature_status: signature.status.clone(),
        provenance_status: provenance.status.clone(),
        checksum,
        signature,
        provenance,
        warnings,
    }
}

fn load_manifest(input_path: &Path) -> (Option<PathBuf>, Option<ArtifactManifest>) {
    if input_path.extension().and_then(|ext| ext.to_str()) == Some("json")
        && let Ok(contents) = fs::read_to_string(input_path)
        && let Ok(manifest) = serde_json::from_str::<ArtifactManifest>(&contents)
    {
        return (Some(input_path.to_path_buf()), Some(manifest));
    }

    let sidecar_path = PathBuf::from(format!("{}.manifest.json", input_path.display()));
    let manifest = fs::read_to_string(&sidecar_path)
        .ok()
        .and_then(|contents| serde_json::from_str::<ArtifactManifest>(&contents).ok());
    if manifest.is_some() {
        (Some(sidecar_path), manifest)
    } else {
        (None, None)
    }
}

fn artifact_path(
    input_path: &Path,
    manifest_path: Option<&Path>,
    manifest: Option<&ArtifactManifest>,
) -> PathBuf {
    if let Some(path) = manifest.and_then(|manifest| manifest.artifact_path.as_deref()) {
        let candidate = PathBuf::from(path);
        if candidate.is_absolute() {
            return candidate;
        }
        if let Some(parent) = manifest_path.and_then(Path::parent) {
            return parent.join(candidate);
        }
        return candidate;
    }
    if manifest_path == Some(input_path) {
        return input_path.to_path_buf();
    }
    input_path.to_path_buf()
}

fn verify_checksum(
    manifest: Option<&ArtifactManifest>,
    actual_sha256: Option<&str>,
    artifact_bytes: &Result<Vec<u8>, std::io::Error>,
) -> CheckEvidence {
    if artifact_bytes.is_err() {
        return CheckEvidence {
            status: CheckStatus::ReadError,
            message: "artifact bytes could not be read".to_string(),
            expected: manifest.and_then(|m| m.checksum_sha256.clone()),
            actual: None,
        };
    }

    let Some(manifest) = manifest else {
        return CheckEvidence {
            status: CheckStatus::Missing,
            message: "checksum manifest is missing".to_string(),
            expected: None,
            actual: actual_sha256.map(str::to_string),
        };
    };

    if let Some(algorithm) = manifest.checksum_algorithm.as_deref()
        && algorithm != "sha256"
        && algorithm != "sha-256"
    {
        return CheckEvidence {
            status: CheckStatus::UnsupportedChecksumAlgorithm,
            message: format!("unsupported checksum algorithm: {algorithm}"),
            expected: Some("sha256".to_string()),
            actual: Some(algorithm.to_string()),
        };
    }

    let Some(expected) = manifest.checksum_sha256.as_deref() else {
        return CheckEvidence {
            status: CheckStatus::Missing,
            message: "checksum_sha256 is missing".to_string(),
            expected: None,
            actual: actual_sha256.map(str::to_string),
        };
    };

    let normalized_expected = expected.strip_prefix("sha256:").unwrap_or(expected);
    if Some(normalized_expected) == actual_sha256 {
        CheckEvidence {
            status: CheckStatus::Matched,
            message: "artifact checksum matches manifest".to_string(),
            expected: Some(normalized_expected.to_string()),
            actual: actual_sha256.map(str::to_string),
        }
    } else {
        CheckEvidence {
            status: CheckStatus::Mismatch,
            message: "artifact checksum does not match manifest".to_string(),
            expected: Some(normalized_expected.to_string()),
            actual: actual_sha256.map(str::to_string),
        }
    }
}

fn verify_signature(
    manifest: Option<&ArtifactManifest>,
    artifact_bytes: Option<&[u8]>,
) -> CheckEvidence {
    let Some(manifest) = manifest else {
        return missing_signature();
    };
    let Some(scheme) = manifest.signing_scheme.as_deref() else {
        return missing_signature();
    };

    match scheme {
        "ed25519" | "Ed25519" => {
            let signature = manifest
                .signature_hex
                .as_deref()
                .or(manifest.signature.as_deref());
            let (Some(public_key), Some(signature), Some(artifact_bytes)) = (
                manifest.public_key_hex.as_deref(),
                signature,
                artifact_bytes,
            ) else {
                return CheckEvidence {
                    status: CheckStatus::Invalid,
                    message: "ed25519 signature metadata or artifact bytes are unavailable"
                        .to_string(),
                    expected: Some(
                        "signed artifact bytes with a 32-byte public key and 64-byte signature"
                            .to_string(),
                    ),
                    actual: None,
                };
            };
            let (Some(public_key), Some(signature)) = (
                decode_hex_array::<32>(public_key),
                decode_hex_array::<64>(signature),
            ) else {
                return CheckEvidence {
                    status: CheckStatus::Invalid,
                    message: "ed25519 signature metadata is malformed".to_string(),
                    expected: Some("64-char public key and 128-char signature hex".to_string()),
                    actual: Some("malformed".to_string()),
                };
            };
            let Ok(public_key) = VerifyingKey::from_bytes(&public_key) else {
                return CheckEvidence {
                    status: CheckStatus::Invalid,
                    message: "ed25519 public key is invalid".to_string(),
                    expected: Some("valid Ed25519 public key".to_string()),
                    actual: Some("invalid".to_string()),
                };
            };
            let signature = Signature::from_bytes(&signature);
            if public_key.verify(artifact_bytes, &signature).is_ok() {
                CheckEvidence {
                    status: CheckStatus::Verified,
                    message: "ed25519 signature verifies the artifact bytes".to_string(),
                    expected: Some("valid Ed25519 signature".to_string()),
                    actual: Some("verified".to_string()),
                }
            } else {
                CheckEvidence {
                    status: CheckStatus::Invalid,
                    message: "ed25519 signature does not verify the artifact bytes".to_string(),
                    expected: Some("valid Ed25519 signature".to_string()),
                    actual: Some("verification failed".to_string()),
                }
            }
        }
        "sigstore" | "Sigstore" => match manifest.sigstore_bundle_ref.as_deref() {
            Some(bundle_ref) => CheckEvidence {
                status: CheckStatus::Invalid,
                message: "sigstore bundle references require Rekor/Fulcio verification".to_string(),
                expected: Some("verified Sigstore bundle evidence".to_string()),
                actual: Some(bundle_ref.to_string()),
            },
            None => CheckEvidence {
                status: CheckStatus::Missing,
                message: "sigstore bundle reference is missing".to_string(),
                expected: Some("sigstore_bundle_ref".to_string()),
                actual: None,
            },
        },
        other => CheckEvidence {
            status: CheckStatus::UnsupportedSignatureScheme,
            message: format!("unsupported signature scheme: {other}"),
            expected: Some("ed25519 or sigstore".to_string()),
            actual: Some(other.to_string()),
        },
    }
}

fn verify_provenance(
    input_path: &Path,
    artifact_path: &Path,
    manifest: Option<&ArtifactManifest>,
    actual_sha256: Option<&str>,
) -> (Option<PathBuf>, ProvenanceEvidence) {
    let provenance_path = manifest
        .and_then(|manifest| {
            manifest
                .provenance_path
                .as_deref()
                .or(manifest.provenance.as_deref())
        })
        .map_or_else(
            || PathBuf::from(format!("{}.provenance.json", artifact_path.display())),
            PathBuf::from,
        );

    let resolved_path = if provenance_path.is_absolute() {
        provenance_path
    } else if let Some(parent) = input_path.parent() {
        parent.join(provenance_path)
    } else {
        provenance_path
    };

    let Ok(contents) = fs::read_to_string(&resolved_path) else {
        return (
            None,
            ProvenanceEvidence {
                status: CheckStatus::Missing,
                message: "provenance statement is missing".to_string(),
                source_commit_sha: None,
                build_system: None,
                artifact_sha256: None,
                build_invocation: None,
            },
        );
    };

    let Ok(statement) = serde_json::from_str::<ProvenanceStatement>(&contents) else {
        return (
            Some(resolved_path),
            ProvenanceEvidence {
                status: CheckStatus::Invalid,
                message: "provenance statement is not valid JSON".to_string(),
                source_commit_sha: None,
                build_system: None,
                artifact_sha256: None,
                build_invocation: None,
            },
        );
    };

    let missing_required = statement
        .source_commit_sha
        .as_deref()
        .is_none_or(str::is_empty)
        || statement.build_system.as_deref().is_none_or(str::is_empty)
        || statement
            .artifact_sha256
            .as_deref()
            .is_none_or(str::is_empty)
        || statement
            .build_invocation
            .as_deref()
            .is_none_or(str::is_empty);
    if missing_required {
        return (
            Some(resolved_path),
            ProvenanceEvidence {
                status: CheckStatus::Invalid,
                message: "provenance statement is missing required SLSA L1 fields".to_string(),
                source_commit_sha: statement.source_commit_sha,
                build_system: statement.build_system,
                artifact_sha256: statement.artifact_sha256,
                build_invocation: statement.build_invocation,
            },
        );
    }

    let provenance_sha = statement
        .artifact_sha256
        .as_deref()
        .and_then(|hash| hash.strip_prefix("sha256:").or(Some(hash)));
    if provenance_sha != actual_sha256 {
        return (
            Some(resolved_path),
            ProvenanceEvidence {
                status: CheckStatus::Mismatch,
                message: "provenance artifact hash does not match artifact".to_string(),
                source_commit_sha: statement.source_commit_sha,
                build_system: statement.build_system,
                artifact_sha256: statement.artifact_sha256,
                build_invocation: statement.build_invocation,
            },
        );
    }

    (
        Some(resolved_path),
        ProvenanceEvidence {
            status: CheckStatus::Verified,
            message: "provenance statement links source commit to artifact hash".to_string(),
            source_commit_sha: statement.source_commit_sha,
            build_system: statement.build_system,
            artifact_sha256: statement.artifact_sha256,
            build_invocation: statement.build_invocation,
        },
    )
}

fn missing_signature() -> CheckEvidence {
    CheckEvidence {
        status: CheckStatus::Missing,
        message: "artifact signature is missing".to_string(),
        expected: Some("ed25519 or sigstore signature metadata".to_string()),
        actual: None,
    }
}

fn is_hex_len(value: &str, expected_len: usize) -> bool {
    value.len() == expected_len && value.bytes().all(|b| b.is_ascii_hexdigit())
}

fn decode_hex_array<const N: usize>(value: &str) -> Option<[u8; N]> {
    if !is_hex_len(value, N * 2) {
        return None;
    }
    let mut bytes = [0_u8; N];
    for (index, byte) in bytes.iter_mut().enumerate() {
        let start = index.checked_mul(2)?;
        let end = start.checked_add(2)?;
        *byte = u8::from_str_radix(value.get(start..end)?, 16).ok()?;
    }
    Some(bytes)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(output, "{byte:02x}");
    }
    output
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::{CheckStatus, OverallStatus, sha256_hex, verify_artifact};
    use ed25519_dalek::{Signer, SigningKey};
    use std::fmt::Write as _;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn verifies_artifact_with_checksum_signature_and_provenance() {
        let dir = temp_dir("supply-chain-pass");
        let artifact = dir.join("artifact.wasm");
        fs::write(&artifact, b"portable bytes").expect("artifact should write");
        let hash = sha256_hex(b"portable bytes");
        write_manifest(&artifact, &hash, Some(&artifact));
        write_provenance(&artifact, &hash, "abc123");

        let report = verify_artifact(&artifact);

        assert_eq!(report.overall_status, OverallStatus::Passed);
        assert_eq!(report.checksum_status, CheckStatus::Matched);
        assert_eq!(report.signature_status, CheckStatus::Verified);
        assert_eq!(report.provenance_status, CheckStatus::Verified);
        assert!(report.passed());
    }

    #[test]
    fn rejects_a_well_formed_forged_ed25519_signature() {
        let dir = temp_dir("supply-chain-forged-signature");
        let artifact = dir.join("artifact.wasm");
        fs::write(&artifact, b"portable bytes").expect("artifact should write");
        let hash = sha256_hex(b"portable bytes");
        let signing_key = SigningKey::from_bytes(&[7_u8; 32]);
        fs::write(
            format!("{}.manifest.json", artifact.display()),
            format!(
                r#"{{
  "checksum_algorithm": "sha256",
  "checksum_sha256": "{hash}",
  "signing_scheme": "ed25519",
  "public_key_hex": "{}",
  "signature_hex": "{}"
}}"#,
                hex(&signing_key.verifying_key().to_bytes()),
                "00".repeat(64)
            ),
        )
        .expect("manifest should write");
        write_provenance(&artifact, &hash, "abc123");

        let report = verify_artifact(&artifact);

        assert_eq!(report.signature_status, CheckStatus::Invalid);
        assert_eq!(report.overall_status, OverallStatus::Failed);
    }

    #[test]
    fn reports_all_failed_checks_without_short_circuiting() {
        let dir = temp_dir("supply-chain-fail");
        let artifact = dir.join("artifact.wasm");
        fs::write(&artifact, b"changed bytes").expect("artifact should write");
        fs::write(
            format!("{}.manifest.json", artifact.display()),
            r#"{
  "checksum_algorithm": "sha256",
  "checksum_sha256": "0000"
}"#,
        )
        .expect("manifest should write");

        let report = verify_artifact(&artifact);

        assert_eq!(report.overall_status, OverallStatus::Failed);
        assert_eq!(report.checksum_status, CheckStatus::Mismatch);
        assert_eq!(report.signature_status, CheckStatus::Missing);
        assert_eq!(report.provenance_status, CheckStatus::Missing);
        assert!(!report.passed());
    }

    #[test]
    fn rejects_unsupported_checksum_algorithm_and_malformed_signature() {
        let dir = temp_dir("supply-chain-invalid");
        let artifact = dir.join("artifact.wasm");
        fs::write(&artifact, b"portable bytes").expect("artifact should write");
        fs::write(
            format!("{}.manifest.json", artifact.display()),
            r#"{
  "checksum_algorithm": "md5",
  "checksum_sha256": "abc",
  "signing_scheme": "ed25519",
  "public_key_hex": "abc",
  "signature_hex": "def"
}"#,
        )
        .expect("manifest should write");

        let report = verify_artifact(&artifact);

        assert_eq!(
            report.checksum_status,
            CheckStatus::UnsupportedChecksumAlgorithm
        );
        assert_eq!(report.signature_status, CheckStatus::Invalid);
    }

    #[test]
    fn rejects_placeholder_sigstore_bundle_in_manifest_json_input() {
        let dir = temp_dir("supply-chain-manifest-json");
        let artifact = dir.join("artifact.bin");
        let manifest = dir.join("manifest.json");
        let provenance = dir.join("provenance.json");
        fs::write(&artifact, b"portable bytes").expect("artifact should write");
        let hash = sha256_hex(b"portable bytes");
        fs::write(
            &manifest,
            format!(
                r#"{{
  "artifact_path": "artifact.bin",
  "checksum_algorithm": "sha-256",
  "checksum_sha256": "sha256:{hash}",
  "signing_scheme": "sigstore",
  "sigstore_bundle_ref": "verified://bundle",
  "provenance_path": "provenance.json"
}}"#
            ),
        )
        .expect("manifest should write");
        fs::write(
            &provenance,
            format!(
                r#"{{
  "source_commit_sha": "abc123",
  "build_system": "github-actions",
  "artifact_sha256": "sha256:{hash}",
  "build_invocation": "cargo build --release"
}}"#
            ),
        )
        .expect("provenance should write");

        let report = verify_artifact(&manifest);

        assert_eq!(report.overall_status, OverallStatus::Failed);
        assert_eq!(report.checksum_status, CheckStatus::Matched);
        assert_eq!(report.signature_status, CheckStatus::Invalid);
        assert_eq!(report.provenance_status, CheckStatus::Verified);
    }

    #[test]
    fn reports_missing_manifest_and_unreadable_artifact() {
        let dir = temp_dir("supply-chain-unreadable");
        let artifact = dir.join("missing.wasm");

        let report = verify_artifact(&artifact);

        assert_eq!(report.overall_status, OverallStatus::Failed);
        assert_eq!(report.checksum_status, CheckStatus::ReadError);
        assert_eq!(report.signature_status, CheckStatus::Missing);
        assert_eq!(report.provenance_status, CheckStatus::Missing);
        assert!(report.warnings.iter().any(|w| w.contains("manifest")));
        assert!(report.warnings.iter().any(|w| w.contains("unreadable")));
    }

    #[test]
    fn reports_missing_checksum_unsupported_signature_and_invalid_provenance_json() {
        let dir = temp_dir("supply-chain-invalid-json");
        let artifact = dir.join("artifact.wasm");
        fs::write(&artifact, b"portable bytes").expect("artifact should write");
        let provenance = dir.join("bad-provenance.json");
        fs::write(&provenance, "not-json").expect("provenance should write");
        fs::write(
            format!("{}.manifest.json", artifact.display()),
            format!(
                r#"{{
  "checksum_algorithm": "sha256",
  "signing_scheme": "rsa",
  "provenance_path": "{}"
}}"#,
                provenance.display()
            ),
        )
        .expect("manifest should write");

        let report = verify_artifact(&artifact);

        assert_eq!(report.checksum_status, CheckStatus::Missing);
        assert_eq!(
            report.signature_status,
            CheckStatus::UnsupportedSignatureScheme
        );
        assert_eq!(report.provenance_status, CheckStatus::Invalid);
    }

    #[test]
    fn reports_sigstore_and_provenance_failure_modes() {
        let dir = temp_dir("supply-chain-sigstore-failures");
        let artifact = dir.join("artifact.wasm");
        fs::write(&artifact, b"portable bytes").expect("artifact should write");
        let hash = sha256_hex(b"portable bytes");
        fs::write(
            format!("{}.manifest.json", artifact.display()),
            format!(
                r#"{{
  "checksum_sha256": "{hash}",
  "signing_scheme": "sigstore",
  "sigstore_bundle_ref": "rekor://unverified"
}}"#
            ),
        )
        .expect("manifest should write");
        write_provenance(&artifact, "deadbeef", "abc123");

        let invalid_bundle = verify_artifact(&artifact);

        assert_eq!(invalid_bundle.signature_status, CheckStatus::Invalid);
        assert_eq!(invalid_bundle.provenance_status, CheckStatus::Mismatch);

        fs::write(
            format!("{}.manifest.json", artifact.display()),
            format!(
                r#"{{
  "checksum_sha256": "{hash}",
  "signing_scheme": "sigstore",
  "provenance_path": "{}.provenance.json"
}}"#,
                artifact.display()
            ),
        )
        .expect("manifest should write");
        fs::write(
            format!("{}.provenance.json", artifact.display()),
            r#"{"source_commit_sha": "abc123"}"#,
        )
        .expect("provenance should write");

        let missing_bundle = verify_artifact(&artifact);

        assert_eq!(missing_bundle.signature_status, CheckStatus::Missing);
        assert_eq!(missing_bundle.provenance_status, CheckStatus::Invalid);
    }

    #[test]
    fn covers_manifestless_artifact_and_private_path_fallbacks() {
        let dir = temp_dir("supply-chain-helper-fallbacks");
        let artifact = dir.join("artifact.wasm");
        fs::write(&artifact, b"portable bytes").expect("artifact should write");

        let manifestless = verify_artifact(&artifact);

        assert_eq!(manifestless.checksum_status, CheckStatus::Missing);
        assert_eq!(manifestless.signature_status, CheckStatus::Missing);

        let manifest = super::ArtifactManifest {
            artifact_path: Some("relative-artifact".to_string()),
            checksum_algorithm: None,
            checksum_sha256: None,
            signing_scheme: None,
            signature: None,
            signature_hex: None,
            public_key_hex: None,
            sigstore_bundle_ref: None,
            provenance: None,
            provenance_path: Some("relative-provenance.json".to_string()),
        };

        assert_eq!(
            super::artifact_path(Path::new("input.json"), None, Some(&manifest)),
            PathBuf::from("relative-artifact")
        );
        assert_eq!(
            super::artifact_path(Path::new("input.json"), Some(Path::new("input.json")), None),
            PathBuf::from("input.json")
        );

        let (provenance_path, provenance) =
            super::verify_provenance(Path::new("/"), Path::new("artifact"), Some(&manifest), None);

        assert_eq!(provenance_path, None);
        assert_eq!(provenance.status, CheckStatus::Missing);
    }

    fn write_manifest(artifact: &Path, checksum: &str, artifact_path: Option<&Path>) {
        let signing_key = SigningKey::from_bytes(&[7_u8; 32]);
        let artifact_bytes = fs::read(artifact).expect("artifact should read for signing");
        let signature = signing_key.sign(&artifact_bytes);
        let path_field = artifact_path
            .map(|path| format!(r#""artifact_path": "{}","#, path.display()))
            .unwrap_or_default();
        fs::write(
            format!("{}.manifest.json", artifact.display()),
            format!(
                r#"{{
  {path_field}
  "checksum_algorithm": "sha256",
  "checksum_sha256": "{checksum}",
  "signing_scheme": "ed25519",
  "public_key_hex": "{}",
  "signature_hex": "{}"
}}"#,
                hex(&signing_key.verifying_key().to_bytes()),
                hex(&signature.to_bytes())
            ),
        )
        .expect("manifest should write");
    }

    fn hex(bytes: &[u8]) -> String {
        let mut encoded = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            assert!(write!(&mut encoded, "{byte:02x}").is_ok());
        }
        encoded
    }

    fn write_provenance(artifact: &Path, checksum: &str, commit: &str) {
        fs::write(
            format!("{}.provenance.json", artifact.display()),
            format!(
                r#"{{
  "source_commit_sha": "{commit}",
  "build_system": "github-actions",
  "artifact_sha256": "{checksum}",
  "build_invocation": "cargo build --release"
}}"#
            ),
        )
        .expect("provenance should write");
    }

    fn temp_dir(name: &str) -> PathBuf {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be valid")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("traverse-{name}-{now}"));
        fs::create_dir_all(&path).expect("temp dir should be created");
        path
    }
}
