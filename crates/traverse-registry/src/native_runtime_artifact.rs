//! Native runtime artifact distribution registry (spec 075).
//!
//! Publishes and resolves the digest-pinned production `runtime.wasm`
//! artifact metadata described by
//! `075-native-runtime-distribution-contract`: an immutable artifact
//! identity, deterministic rejection of missing, tampered, incompatible, or
//! uncertified artifacts, and one host-agnostic schema shared by Swift,
//! Kotlin, and .NET consumers.

use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

const NATIVE_RUNTIME_REGISTRY_SCHEMA_VERSION: &str = "1.0.0";

/// Per-host certification evidence for one native runtime artifact release.
///
/// The schema is identical for every host (Spec 075 FR-008): no field here
/// carries a different meaning for Swift, Kotlin, or .NET.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct HostCertification {
    pub host: String,
    pub engine_name: String,
    pub engine_version: String,
    pub conformance_passed: bool,
}

/// One immutable, digest-identified native runtime artifact release.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct NativeRuntimeArtifactRecord {
    pub runtime_version: String,
    pub bridge_version: String,
    pub supported_bridge_range: String,
    pub sha256: String,
    pub artifact_url: String,
    pub host_certifications: Vec<HostCertification>,
}

/// The published index of native runtime artifact releases.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct NativeRuntimeArtifactIndex {
    pub schema_version: String,
    pub releases: Vec<NativeRuntimeArtifactRecord>,
}

impl Default for NativeRuntimeArtifactIndex {
    fn default() -> Self {
        Self {
            schema_version: NATIVE_RUNTIME_REGISTRY_SCHEMA_VERSION.to_string(),
            releases: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeRuntimeArtifactErrorCode {
    EmptyField,
    InvalidVersion,
    InvalidRange,
    DuplicateVersion,
    ArtifactNotFound,
    DigestMismatch,
    BridgeVersionMismatch,
    UncertifiedHost,
    IndexReadFailed,
    IndexParseFailed,
    IndexWriteFailed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeRuntimeArtifactError {
    pub code: NativeRuntimeArtifactErrorCode,
    pub message: String,
}

fn error(
    code: NativeRuntimeArtifactErrorCode,
    message: impl Into<String>,
) -> NativeRuntimeArtifactError {
    NativeRuntimeArtifactError {
        code,
        message: message.into(),
    }
}

fn validate_record(record: &NativeRuntimeArtifactRecord) -> Result<(), NativeRuntimeArtifactError> {
    if record.runtime_version.trim().is_empty() {
        return Err(error(
            NativeRuntimeArtifactErrorCode::EmptyField,
            "runtime_version must be non-empty",
        ));
    }
    Version::parse(&record.runtime_version).map_err(|source| {
        error(
            NativeRuntimeArtifactErrorCode::InvalidVersion,
            format!(
                "runtime_version {} is not valid semver: {source}",
                record.runtime_version
            ),
        )
    })?;
    if record.bridge_version.trim().is_empty() {
        return Err(error(
            NativeRuntimeArtifactErrorCode::EmptyField,
            "bridge_version must be non-empty",
        ));
    }
    Version::parse(&record.bridge_version).map_err(|source| {
        error(
            NativeRuntimeArtifactErrorCode::InvalidVersion,
            format!(
                "bridge_version {} is not valid semver: {source}",
                record.bridge_version
            ),
        )
    })?;
    VersionReq::parse(&record.supported_bridge_range).map_err(|source| {
        error(
            NativeRuntimeArtifactErrorCode::InvalidRange,
            format!(
                "supported_bridge_range {} is not a valid range: {source}",
                record.supported_bridge_range
            ),
        )
    })?;
    let digest = record.sha256.trim();
    if digest.len() != 64 || !digest.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(error(
            NativeRuntimeArtifactErrorCode::EmptyField,
            "sha256 must be a 64-character hex digest",
        ));
    }
    if record.artifact_url.trim().is_empty() {
        return Err(error(
            NativeRuntimeArtifactErrorCode::EmptyField,
            "artifact_url must be non-empty",
        ));
    }
    for certification in &record.host_certifications {
        if certification.host.trim().is_empty()
            || certification.engine_name.trim().is_empty()
            || certification.engine_version.trim().is_empty()
        {
            return Err(error(
                NativeRuntimeArtifactErrorCode::EmptyField,
                "host_certifications entries must have non-empty host, engine_name, and engine_version",
            ));
        }
    }
    Ok(())
}

/// Publishes one immutable native runtime artifact release into `index`.
///
/// # Errors
///
/// Returns [`NativeRuntimeArtifactError`] when the record fails field
/// validation (Spec 075 FR-001, FR-002), or when `index` already has a
/// release at the same `runtime_version` (Spec 075 FR-007 — a corrected
/// build MUST publish under a new version rather than overwrite one).
pub fn publish_native_runtime_artifact(
    index: &mut NativeRuntimeArtifactIndex,
    record: NativeRuntimeArtifactRecord,
) -> Result<(), NativeRuntimeArtifactError> {
    validate_record(&record)?;
    if index
        .releases
        .iter()
        .any(|existing| existing.runtime_version == record.runtime_version)
    {
        return Err(error(
            NativeRuntimeArtifactErrorCode::DuplicateVersion,
            format!(
                "runtime_version {} is already published; publish a corrected build under a new version",
                record.runtime_version
            ),
        ));
    }
    index.releases.push(record);
    Ok(())
}

/// Resolves and verifies one native runtime artifact release by exact
/// `runtime_version` identity (Spec 075 FR-006), deterministically rejecting
/// a missing, tampered, incompatible, or uncertified artifact before a host
/// may instantiate it (Spec 075 FR-003–FR-005).
///
/// # Errors
///
/// Returns [`NativeRuntimeArtifactError`] with:
/// - [`NativeRuntimeArtifactErrorCode::ArtifactNotFound`] when no release is
///   published at `runtime_version`;
/// - [`NativeRuntimeArtifactErrorCode::DigestMismatch`] when `fetched_sha256`
///   does not match the published digest;
/// - [`NativeRuntimeArtifactErrorCode::InvalidRange`] when
///   `required_bridge_range` is not a valid semver range;
/// - [`NativeRuntimeArtifactErrorCode::BridgeVersionMismatch`] when the
///   release's certified `bridge_version` does not satisfy
///   `required_bridge_range`;
/// - [`NativeRuntimeArtifactErrorCode::UncertifiedHost`] when no passing
///   certification exists for `requesting_host`.
pub fn resolve_native_runtime_artifact<'index>(
    index: &'index NativeRuntimeArtifactIndex,
    runtime_version: &str,
    fetched_sha256: &str,
    required_bridge_range: &str,
    requesting_host: &str,
) -> Result<&'index NativeRuntimeArtifactRecord, NativeRuntimeArtifactError> {
    let record = index
        .releases
        .iter()
        .find(|release| release.runtime_version == runtime_version)
        .ok_or_else(|| {
            error(
                NativeRuntimeArtifactErrorCode::ArtifactNotFound,
                format!(
                    "no published native runtime artifact for runtime_version {runtime_version}"
                ),
            )
        })?;

    if !fetched_sha256.eq_ignore_ascii_case(&record.sha256) {
        return Err(error(
            NativeRuntimeArtifactErrorCode::DigestMismatch,
            format!(
                "fetched artifact digest does not match published metadata for runtime_version {runtime_version}"
            ),
        ));
    }

    let requirement = VersionReq::parse(required_bridge_range).map_err(|source| {
        error(
            NativeRuntimeArtifactErrorCode::InvalidRange,
            format!("required_bridge_range {required_bridge_range} is not a valid range: {source}"),
        )
    })?;
    let bridge_version = Version::parse(&record.bridge_version).map_err(|source| {
        error(
            NativeRuntimeArtifactErrorCode::InvalidVersion,
            format!(
                "published bridge_version {} is not valid semver: {source}",
                record.bridge_version
            ),
        )
    })?;
    if !requirement.matches(&bridge_version) {
        return Err(error(
            NativeRuntimeArtifactErrorCode::BridgeVersionMismatch,
            format!(
                "runtime_version {runtime_version} bridge_version {} does not satisfy required range {required_bridge_range}",
                record.bridge_version
            ),
        ));
    }

    let certified = record.host_certifications.iter().any(|certification| {
        certification.host == requesting_host && certification.conformance_passed
    });
    if !certified {
        return Err(error(
            NativeRuntimeArtifactErrorCode::UncertifiedHost,
            format!(
                "no passing conformance certification for host {requesting_host} on runtime_version {runtime_version}"
            ),
        ));
    }

    Ok(record)
}

/// The repository-relative path a native runtime artifact index is published
/// to and resolved from, co-located with the `runtime.wasm` build output.
#[must_use]
pub fn native_runtime_registry_path(workspace_root: &Path) -> PathBuf {
    workspace_root
        .join("runtime")
        .join("native-runtime-registry.json")
}

/// Loads the published native runtime artifact index. An absent file is not
/// an error: it resolves to an empty index, since resolution against an
/// empty index already fails deterministically with `ArtifactNotFound`
/// (Spec 075 FR-009 — resolution stays local, with no sidecar dependency).
///
/// # Errors
///
/// Returns [`NativeRuntimeArtifactError`] when the file exists but cannot be
/// read or parsed as a valid index.
pub fn load_native_runtime_registry(
    workspace_root: &Path,
) -> Result<NativeRuntimeArtifactIndex, NativeRuntimeArtifactError> {
    let path = native_runtime_registry_path(workspace_root);
    if !path.exists() {
        return Ok(NativeRuntimeArtifactIndex::default());
    }
    let bytes = fs::read(&path).map_err(|source| {
        error(
            NativeRuntimeArtifactErrorCode::IndexReadFailed,
            format!("failed to read {}: {source}", path.display()),
        )
    })?;
    serde_json::from_slice(&bytes).map_err(|source| {
        error(
            NativeRuntimeArtifactErrorCode::IndexParseFailed,
            format!("failed to parse {}: {source}", path.display()),
        )
    })
}

/// Atomically writes the published native runtime artifact index.
///
/// # Errors
///
/// Returns [`NativeRuntimeArtifactError`] when the target directory cannot
/// be created or the write cannot be committed atomically.
pub fn write_native_runtime_registry(
    workspace_root: &Path,
    index: &NativeRuntimeArtifactIndex,
) -> Result<(), NativeRuntimeArtifactError> {
    let dir = workspace_root.join("runtime");
    fs::create_dir_all(&dir).map_err(|source| {
        error(
            NativeRuntimeArtifactErrorCode::IndexWriteFailed,
            format!("failed to create {}: {source}", dir.display()),
        )
    })?;
    let path = native_runtime_registry_path(workspace_root);
    let tmp_path = path.with_extension("json.tmp");
    let serialized = native_runtime_index_json_value(index).to_string();
    fs::write(&tmp_path, format!("{serialized}\n")).map_err(|source| {
        error(
            NativeRuntimeArtifactErrorCode::IndexWriteFailed,
            format!("failed to write {}: {source}", tmp_path.display()),
        )
    })?;
    fs::rename(&tmp_path, &path).map_err(|source| {
        let _ = fs::remove_file(&tmp_path);
        error(
            NativeRuntimeArtifactErrorCode::IndexWriteFailed,
            format!("failed to commit {}: {source}", path.display()),
        )
    })
}

fn native_runtime_index_json_value(index: &NativeRuntimeArtifactIndex) -> Value {
    serde_json::json!({
        "schema_version": index.schema_version,
        "releases": index
            .releases
            .iter()
            .map(native_runtime_record_json_value)
            .collect::<Vec<_>>(),
    })
}

fn native_runtime_record_json_value(record: &NativeRuntimeArtifactRecord) -> Value {
    serde_json::json!({
        "runtime_version": record.runtime_version,
        "bridge_version": record.bridge_version,
        "supported_bridge_range": record.supported_bridge_range,
        "sha256": record.sha256,
        "artifact_url": record.artifact_url,
        "host_certifications": record
            .host_certifications
            .iter()
            .map(|certification| {
                serde_json::json!({
                    "host": certification.host,
                    "engine_name": certification.engine_name,
                    "engine_version": certification.engine_version,
                    "conformance_passed": certification.conformance_passed,
                })
            })
            .collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn unique_temp_dir() -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time should be after epoch")
            .as_nanos();
        let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "traverse-native-runtime-artifact-test-{nanos}-{counter}"
        ));
        fs::create_dir_all(&path).expect("temp dir should be created");
        path
    }

    fn valid_digest(byte: u8) -> String {
        format!("{byte:02x}").repeat(32)
    }

    fn certified_record(
        runtime_version: &str,
        bridge_version: &str,
        digest_byte: u8,
    ) -> NativeRuntimeArtifactRecord {
        NativeRuntimeArtifactRecord {
            runtime_version: runtime_version.to_string(),
            bridge_version: bridge_version.to_string(),
            supported_bridge_range: ">=1.1.0,<2.0.0".to_string(),
            sha256: valid_digest(digest_byte),
            artifact_url: format!("content-addressed://runtime-{runtime_version}"),
            host_certifications: vec![
                HostCertification {
                    host: "swift".to_string(),
                    engine_name: "WasmKit".to_string(),
                    engine_version: "0.3.1".to_string(),
                    conformance_passed: true,
                },
                HostCertification {
                    host: "kotlin".to_string(),
                    engine_name: "Chicory".to_string(),
                    engine_version: "1.0.0".to_string(),
                    conformance_passed: true,
                },
                HostCertification {
                    host: "dotnet".to_string(),
                    engine_name: "Wasmtime".to_string(),
                    engine_version: "24.0.0".to_string(),
                    conformance_passed: true,
                },
            ],
        }
    }

    #[test]
    fn publishes_and_resolves_a_certified_release() {
        let mut index = NativeRuntimeArtifactIndex::default();
        let record = certified_record("0.9.0", "1.1.0", 0xaa);
        publish_native_runtime_artifact(&mut index, record.clone()).expect("publish should pass");

        for host in ["swift", "kotlin", "dotnet"] {
            let resolved = resolve_native_runtime_artifact(
                &index,
                "0.9.0",
                &record.sha256,
                ">=1.1.0,<2.0.0",
                host,
            )
            .expect("resolution should pass for every certified host");
            assert_eq!(resolved.runtime_version, "0.9.0");
        }
    }

    #[test]
    fn rejects_republishing_an_existing_runtime_version() {
        let mut index = NativeRuntimeArtifactIndex::default();
        publish_native_runtime_artifact(&mut index, certified_record("0.9.0", "1.1.0", 0xaa))
            .expect("first publish should pass");

        let failure =
            publish_native_runtime_artifact(&mut index, certified_record("0.9.0", "1.1.0", 0xbb))
                .expect_err("republishing the same runtime_version should fail");

        assert_eq!(
            failure.code,
            NativeRuntimeArtifactErrorCode::DuplicateVersion
        );
        assert_eq!(index.releases.len(), 1);
    }

    #[test]
    fn rejects_publishing_an_invalid_record() {
        let mut index = NativeRuntimeArtifactIndex::default();
        let mut record = certified_record("0.9.0", "1.1.0", 0xaa);
        record.sha256 = "not-a-digest".to_string();

        let failure = publish_native_runtime_artifact(&mut index, record)
            .expect_err("invalid digest should fail validation");

        assert_eq!(failure.code, NativeRuntimeArtifactErrorCode::EmptyField);
        assert!(index.releases.is_empty());
    }

    type RecordMutator = fn(&mut NativeRuntimeArtifactRecord);

    #[test]
    fn rejects_publishing_a_record_with_empty_required_fields() {
        let cases: Vec<(&str, RecordMutator)> = vec![
            ("empty runtime_version", |record| {
                record.runtime_version = String::new();
            }),
            ("invalid runtime_version", |record| {
                record.runtime_version = "not-a-version".to_string();
            }),
            ("empty bridge_version", |record| {
                record.bridge_version = String::new();
            }),
            ("invalid bridge_version", |record| {
                record.bridge_version = "not-a-version".to_string();
            }),
            ("invalid supported_bridge_range", |record| {
                record.supported_bridge_range = "not-a-range".to_string();
            }),
            ("empty artifact_url", |record| {
                record.artifact_url = String::new();
            }),
            ("empty host certification field", |record| {
                record.host_certifications[0].engine_version = String::new();
            }),
        ];

        for (label, mutate) in cases {
            let mut index = NativeRuntimeArtifactIndex::default();
            let mut record = certified_record("0.9.0", "1.1.0", 0xaa);
            mutate(&mut record);

            publish_native_runtime_artifact(&mut index, record)
                .expect_err(&format!("{label} should fail validation"));
        }
    }

    #[test]
    fn resolution_rejects_a_missing_artifact() {
        let index = NativeRuntimeArtifactIndex::default();

        let failure = resolve_native_runtime_artifact(
            &index,
            "0.9.0",
            &valid_digest(0xaa),
            ">=1.1.0,<2.0.0",
            "swift",
        )
        .expect_err("missing artifact should fail");

        assert_eq!(
            failure.code,
            NativeRuntimeArtifactErrorCode::ArtifactNotFound
        );
    }

    #[test]
    fn resolution_rejects_a_tampered_digest() {
        let mut index = NativeRuntimeArtifactIndex::default();
        publish_native_runtime_artifact(&mut index, certified_record("0.9.0", "1.1.0", 0xaa))
            .expect("publish should pass");

        let failure = resolve_native_runtime_artifact(
            &index,
            "0.9.0",
            &valid_digest(0xff),
            ">=1.1.0,<2.0.0",
            "swift",
        )
        .expect_err("digest mismatch should fail as tamper");

        assert_eq!(failure.code, NativeRuntimeArtifactErrorCode::DigestMismatch);
    }

    #[test]
    fn resolution_rejects_an_invalid_required_range() {
        let mut index = NativeRuntimeArtifactIndex::default();
        let record = certified_record("0.9.0", "1.1.0", 0xaa);
        publish_native_runtime_artifact(&mut index, record.clone()).expect("publish should pass");

        let failure = resolve_native_runtime_artifact(
            &index,
            "0.9.0",
            &record.sha256,
            "not-a-range",
            "swift",
        )
        .expect_err("malformed required_bridge_range should fail");

        assert_eq!(failure.code, NativeRuntimeArtifactErrorCode::InvalidRange);
    }

    #[test]
    fn resolution_rejects_a_persisted_record_with_a_malformed_bridge_version() {
        let mut record = certified_record("0.9.0", "1.1.0", 0xaa);
        // Bypasses `publish_native_runtime_artifact` validation to exercise
        // resolution's defense against a tampered or hand-edited on-disk
        // registry file, whose fields are not guaranteed to have passed
        // publish-time validation.
        record.bridge_version = "not-a-version".to_string();
        let index = NativeRuntimeArtifactIndex {
            schema_version: "1.0.0".to_string(),
            releases: vec![record.clone()],
        };

        let failure = resolve_native_runtime_artifact(
            &index,
            "0.9.0",
            &record.sha256,
            ">=1.1.0,<2.0.0",
            "swift",
        )
        .expect_err("malformed persisted bridge_version should fail");

        assert_eq!(failure.code, NativeRuntimeArtifactErrorCode::InvalidVersion);
    }

    #[test]
    fn resolution_rejects_an_incompatible_bridge_version() {
        let mut index = NativeRuntimeArtifactIndex::default();
        let record = certified_record("0.9.0", "1.0.0", 0xaa);
        publish_native_runtime_artifact(&mut index, record.clone()).expect("publish should pass");

        let failure = resolve_native_runtime_artifact(
            &index,
            "0.9.0",
            &record.sha256,
            ">=1.1.0,<2.0.0",
            "swift",
        )
        .expect_err("bridge 1.0.0 should not satisfy the >=1.1.0,<2.0.0 baseline");

        assert_eq!(
            failure.code,
            NativeRuntimeArtifactErrorCode::BridgeVersionMismatch
        );
    }

    #[test]
    fn resolution_rejects_an_uncertified_host() {
        let mut index = NativeRuntimeArtifactIndex::default();
        let mut record = certified_record("0.9.0", "1.1.0", 0xaa);
        record
            .host_certifications
            .retain(|certification| certification.host != "swift");
        publish_native_runtime_artifact(&mut index, record.clone()).expect("publish should pass");

        let failure = resolve_native_runtime_artifact(
            &index,
            "0.9.0",
            &record.sha256,
            ">=1.1.0,<2.0.0",
            "swift",
        )
        .expect_err("host without passing certification should fail");

        assert_eq!(
            failure.code,
            NativeRuntimeArtifactErrorCode::UncertifiedHost
        );
    }

    #[test]
    fn every_published_release_remains_independently_resolvable() {
        let mut index = NativeRuntimeArtifactIndex::default();
        let first = certified_record("0.9.0", "1.1.0", 0xaa);
        let second = certified_record("0.10.0", "1.1.0", 0xbb);
        publish_native_runtime_artifact(&mut index, first.clone())
            .expect("first publish should pass");
        publish_native_runtime_artifact(&mut index, second.clone())
            .expect("second publish should pass");

        let resolved_first = resolve_native_runtime_artifact(
            &index,
            "0.9.0",
            &first.sha256,
            ">=1.1.0,<2.0.0",
            "swift",
        )
        .expect("the older release should remain resolvable after a newer one publishes");
        let resolved_second = resolve_native_runtime_artifact(
            &index,
            "0.10.0",
            &second.sha256,
            ">=1.1.0,<2.0.0",
            "swift",
        )
        .expect("the newer release should resolve independently");

        assert_eq!(resolved_first.sha256, first.sha256);
        assert_eq!(resolved_second.sha256, second.sha256);
    }

    #[test]
    fn writes_and_loads_the_published_index_atomically() {
        let workspace_root = unique_temp_dir();
        let mut index = NativeRuntimeArtifactIndex::default();
        publish_native_runtime_artifact(&mut index, certified_record("0.9.0", "1.1.0", 0xaa))
            .expect("publish should pass");

        write_native_runtime_registry(&workspace_root, &index).expect("write should pass");
        let loaded = load_native_runtime_registry(&workspace_root).expect("load should pass");

        assert_eq!(loaded, index);
    }

    #[test]
    fn a_missing_registry_file_loads_as_an_empty_index() {
        let workspace_root = unique_temp_dir();

        let loaded =
            load_native_runtime_registry(&workspace_root).expect("missing file should not error");

        assert!(loaded.releases.is_empty());
    }

    #[test]
    fn write_fails_when_the_runtime_directory_cannot_be_created() {
        let workspace_root = unique_temp_dir();
        // Pre-create a plain file where the "runtime" directory needs to go,
        // so `fs::create_dir_all` fails.
        fs::write(workspace_root.join("runtime"), b"blocked").expect("blocking file should write");
        let index = NativeRuntimeArtifactIndex::default();

        let failure = write_native_runtime_registry(&workspace_root, &index)
            .expect_err("write should fail when the runtime directory cannot be created");

        assert_eq!(
            failure.code,
            NativeRuntimeArtifactErrorCode::IndexWriteFailed
        );
    }

    #[test]
    fn write_fails_when_the_temp_file_cannot_be_written() {
        let workspace_root = unique_temp_dir();
        let runtime_dir = workspace_root.join("runtime");
        fs::create_dir_all(&runtime_dir).expect("runtime dir should be created");
        // Pre-create the temp path as a directory, so writing the serialized
        // index to it fails.
        fs::create_dir_all(runtime_dir.join("native-runtime-registry.json.tmp"))
            .expect("blocking directory should be created");
        let index = NativeRuntimeArtifactIndex::default();

        let failure = write_native_runtime_registry(&workspace_root, &index)
            .expect_err("write should fail when the temp file cannot be written");

        assert_eq!(
            failure.code,
            NativeRuntimeArtifactErrorCode::IndexWriteFailed
        );
    }

    #[test]
    fn write_fails_when_the_commit_rename_cannot_complete() {
        let workspace_root = unique_temp_dir();
        let runtime_dir = workspace_root.join("runtime");
        fs::create_dir_all(&runtime_dir).expect("runtime dir should be created");
        // Pre-create the final registry path as a directory, so the atomic
        // rename of the temp file onto it fails.
        fs::create_dir_all(runtime_dir.join("native-runtime-registry.json"))
            .expect("blocking directory should be created");
        let index = NativeRuntimeArtifactIndex::default();

        let failure = write_native_runtime_registry(&workspace_root, &index)
            .expect_err("write should fail when the commit rename cannot complete");

        assert_eq!(
            failure.code,
            NativeRuntimeArtifactErrorCode::IndexWriteFailed
        );
    }

    #[test]
    fn load_fails_when_the_registry_path_cannot_be_read_as_a_file() {
        let workspace_root = unique_temp_dir();
        let path = native_runtime_registry_path(&workspace_root);
        // A directory at the registry path satisfies `path.exists()` but
        // fails `fs::read`, exercising the read-failure branch distinctly
        // from the parse-failure branch below.
        fs::create_dir_all(&path).expect("blocking directory should be created");

        let failure = load_native_runtime_registry(&workspace_root)
            .expect_err("a directory at the registry path should fail to read");

        assert_eq!(
            failure.code,
            NativeRuntimeArtifactErrorCode::IndexReadFailed
        );
    }

    #[test]
    fn malformed_registry_file_fails_to_load() {
        let workspace_root = unique_temp_dir();
        let path = native_runtime_registry_path(&workspace_root);
        fs::create_dir_all(path.parent().expect("path should have a parent"))
            .expect("dir should be created");
        fs::write(&path, "not json").expect("write should succeed");

        let failure =
            load_native_runtime_registry(&workspace_root).expect_err("malformed index should fail");

        assert_eq!(
            failure.code,
            NativeRuntimeArtifactErrorCode::IndexParseFailed
        );
    }
}
