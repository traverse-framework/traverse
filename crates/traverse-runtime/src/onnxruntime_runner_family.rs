//! Feasibility model for the signed, cross-host `onnxruntime-cpu` runner family.

#[cfg(test)]
use crate::exact_model::digest_hex;
use crate::exact_model::{ModelPackageSignature, sign_model_manifest};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Closed host profile identifiers. They are deliberately not application API.
pub const RUNNER_PROFILES: [&str; 6] = [
    "browser",
    "node",
    "rust-native",
    "swift",
    "kotlin-android",
    "dotnet",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunnerArtifact {
    pub digest: String,
    pub source_revision: String,
    pub build_flags: Vec<String>,
    pub license_id: String,
    pub provenance_url: String,
    pub memory_profile: String,
    pub conformance_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunnerFamilyManifest {
    pub schema_version: String,
    pub family_id: String,
    pub release: String,
    pub cpu_only: bool,
    pub revoked: bool,
    pub artifacts: BTreeMap<String, RunnerArtifact>,
}

impl RunnerFamilyManifest {
    /// Validates the complete signed-runner family metadata before selection.
    ///
    /// # Errors
    ///
    /// Returns an error when the family is revoked, its target matrix differs from
    /// the governed profiles, or an artifact lacks required provenance metadata.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.schema_version != "1.0.0"
            || self.family_id.trim().is_empty()
            || self.release.trim().is_empty()
            || !self.cpu_only
            || self.revoked
        {
            return Err("runner family is invalid or revoked");
        }
        let expected = RUNNER_PROFILES.into_iter().collect::<BTreeSet<_>>();
        if self
            .artifacts
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>()
            != expected
        {
            return Err("runner family target matrix is incomplete");
        }
        if self.artifacts.values().any(|a| {
            !digest(&a.digest)
                || !digest(&a.conformance_digest)
                || a.source_revision.trim().is_empty()
                || a.build_flags.is_empty()
                || a.license_id.trim().is_empty()
                || a.provenance_url.trim().is_empty()
                || a.memory_profile.trim().is_empty()
        }) {
            return Err("runner artifact metadata is invalid");
        }
        Ok(())
    }

    /// Serializes this manifest deterministically for signing and verification.
    ///
    /// # Errors
    ///
    /// Returns a serialization error when the manifest cannot be encoded as JSON.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }

    /// Selects the exact verified artifact for a governed host profile.
    ///
    /// # Errors
    ///
    /// Returns an error when the family is invalid, the profile is absent, or the
    /// caller's expected digest does not match the recorded artifact digest.
    pub fn select(
        &self,
        profile: &str,
        expected_digest: &str,
    ) -> Result<&RunnerArtifact, &'static str> {
        self.validate()?;
        let artifact = self
            .artifacts
            .get(profile)
            .ok_or("runner profile is unknown")?;
        if normalize(&artifact.digest) != normalize(expected_digest) {
            return Err("runner artifact digest mismatch");
        }
        Ok(artifact)
    }
}

/// Signs the deterministic runner-family manifest with the supplied release key.
///
/// # Errors
///
/// Returns a serialization error when the manifest cannot be canonicalized.
pub fn sign_runner_family(
    secret: &[u8; 32],
    manifest: &RunnerFamilyManifest,
) -> Result<ModelPackageSignature, serde_json::Error> {
    Ok(sign_model_manifest(secret, &manifest.canonical_bytes()?))
}

fn normalize(value: &str) -> &str {
    value.strip_prefix("sha256:").unwrap_or(value)
}
fn digest(value: &str) -> bool {
    normalize(value).len() == 64 && normalize(value).bytes().all(|b| b.is_ascii_hexdigit())
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    fn artifact(tag: &str) -> RunnerArtifact {
        RunnerArtifact {
            digest: digest_hex(tag.as_bytes()),
            source_revision: "deadbeef".into(),
            build_flags: vec!["cpu-only".into()],
            license_id: "MIT".into(),
            provenance_url: "https://example.invalid/ort".into(),
            memory_profile: "cpu-512m".into(),
            conformance_digest: digest_hex(format!("{tag}-report").as_bytes()),
        }
    }
    fn family() -> RunnerFamilyManifest {
        RunnerFamilyManifest {
            schema_version: "1.0.0".into(),
            family_id: "onnxruntime-cpu".into(),
            release: "1.0.0".into(),
            cpu_only: true,
            revoked: false,
            artifacts: RUNNER_PROFILES
                .into_iter()
                .map(|p| (p.into(), artifact(p)))
                .collect(),
        }
    }
    #[test]
    fn canonical_family_selects_only_matching_profile_artifact() {
        let f = family();
        let browser = f.artifacts["browser"].digest.clone();
        assert_eq!(
            f.select("browser", &browser).expect("match").memory_profile,
            "cpu-512m"
        );
        assert!(f.select("browser", "00").is_err());
        assert!(f.select("unsupported-host", &browser).is_err());
    }
    #[test]
    fn incomplete_or_revoked_family_fails_closed() {
        let mut f = family();
        f.artifacts.remove("node");
        assert!(f.validate().is_err());
        let mut f = family();
        f.revoked = true;
        assert!(f.validate().is_err());
    }

    #[test]
    fn malformed_artifact_digest_fails_closed() {
        let mut f = family();
        f.artifacts.get_mut("browser").expect("browser").digest = "not-a-digest".into();
        assert_eq!(f.validate(), Err("runner artifact metadata is invalid"));
    }
    #[test]
    fn signed_canonical_bytes_are_stable() {
        let f = family();
        let first = f.canonical_bytes().expect("bytes");
        assert_eq!(first, f.canonical_bytes().expect("bytes"));
        let sig = sign_runner_family(&[7; 32], &f).expect("sign");
        assert_eq!(sig.signature.len(), 128);
    }
}
