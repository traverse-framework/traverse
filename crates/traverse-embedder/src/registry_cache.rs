//! Host-owned verified registry dependency cache (Spec `080-embedded-registry-cache`).
//!
//! Separates network-capable [`prepare`] from offline [`HostRegistryCache`]
//! resolution used at embedder `init`. The host chooses the cache root and
//! supplies the artifact fetcher; Traverse never synthesizes a production
//! path or performs network I/O outside `prepare`.

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use traverse_contracts::parse_contract;
use traverse_registry::{
    PublicRegistryCapabilityRecord, RegistryReference, ResolvedRegistryComponent,
    SyncedPublicRegistryState,
};

/// Stable, secret-free registry-cache failure codes (Spec 080 FR-007).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistryCacheErrorCode {
    /// No synced index snapshot was supplied to prepare.
    RegistrySyncMissing,
    /// No non-yanked version satisfies the requested range.
    RegistryVersionNotFound,
    /// Matching versions exist but every candidate is yanked/deprecated.
    RegistryDependencyYanked,
    /// Host fetch or cache persistence failed during prepare.
    RegistryPrepareFailed,
    /// Declared digest does not match fetched or stored bytes.
    RegistryArtifactDigestMismatch,
    /// Offline resolve found no verified cache entry for the reference.
    RegistryCacheEntryMissing,
    /// Public metadata generation is missing, malformed, incompatible, or unverified.
    RegistryMetadataCacheInvalid,
}

impl RegistryCacheErrorCode {
    /// Stable `snake_case` wire representation.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RegistrySyncMissing => "registry_sync_missing",
            Self::RegistryVersionNotFound => "registry_version_not_found",
            Self::RegistryDependencyYanked => "registry_dependency_yanked",
            Self::RegistryPrepareFailed => "registry_prepare_failed",
            Self::RegistryArtifactDigestMismatch => "registry_artifact_digest_mismatch",
            Self::RegistryCacheEntryMissing => "registry_cache_entry_missing",
            Self::RegistryMetadataCacheInvalid => "registry_metadata_cache_invalid",
        }
    }
}

/// Secret-free registry-cache failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryCacheError {
    /// Stable machine-readable code.
    pub code: RegistryCacheErrorCode,
    /// Human-readable explanation without paths, credentials, or bytes.
    pub message: String,
}

impl RegistryCacheError {
    fn new(code: RegistryCacheErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// Host-supplied network fetch used only by [`prepare`].
pub trait RegistryArtifactFetcher {
    /// Fetch raw bytes for a published registry URL.
    ///
    /// # Errors
    ///
    /// Returns a secret-free message when the host cannot retrieve the asset.
    fn fetch(&self, url: &str) -> Result<Vec<u8>, String>;
}

/// Host-chosen content-addressed registry cache root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostRegistryCache {
    root: PathBuf,
}

impl HostRegistryCache {
    /// Bind Traverse to a host-selected cache directory.
    ///
    /// The host owns creation, permissions, backup, and eviction policy.
    /// Traverse never invents a default production cache path.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Host-selected cache root.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Remove one verified entry by artifact digest (`sha256:…`).
    ///
    /// # Errors
    ///
    /// Returns [`RegistryCacheErrorCode::RegistryPrepareFailed`] when the
    /// entry cannot be removed. Missing entries succeed without error.
    pub fn evict(&self, artifact_digest: &str) -> Result<(), RegistryCacheError> {
        let digest = normalize_digest(artifact_digest).ok_or_else(|| {
            RegistryCacheError::new(
                RegistryCacheErrorCode::RegistryPrepareFailed,
                "artifact digest must be sha256: followed by 64 hex characters",
            )
        })?;
        let artifact = self.artifact_path(&digest);
        let meta = self.meta_path(&digest);
        if artifact.exists() {
            fs::remove_file(&artifact).map_err(|error| {
                RegistryCacheError::new(
                    RegistryCacheErrorCode::RegistryPrepareFailed,
                    format!("failed to evict verified registry artifact: {error}"),
                )
            })?;
        }
        if meta.exists() {
            fs::remove_file(&meta).map_err(|error| {
                RegistryCacheError::new(
                    RegistryCacheErrorCode::RegistryPrepareFailed,
                    format!("failed to evict registry cache metadata: {error}"),
                )
            })?;
        }
        Ok(())
    }

    /// Remove every verified entry under this cache root.
    ///
    /// # Errors
    ///
    /// Returns [`RegistryCacheErrorCode::RegistryPrepareFailed`] when the
    /// cache directory cannot be cleared.
    pub fn evict_all(&self) -> Result<(), RegistryCacheError> {
        for sub in ["sha256", "meta", "refs"] {
            let path = self.root.join(sub);
            if path.exists() {
                fs::remove_dir_all(&path).map_err(|error| {
                    RegistryCacheError::new(
                        RegistryCacheErrorCode::RegistryPrepareFailed,
                        format!("failed to clear host registry cache: {error}"),
                    )
                })?;
            }
        }
        Ok(())
    }

    fn artifact_path(&self, digest_hex: &str) -> PathBuf {
        self.root.join("sha256").join(digest_hex)
    }

    fn meta_path(&self, digest_hex: &str) -> PathBuf {
        self.root.join("meta").join(format!("{digest_hex}.json"))
    }

    fn ref_path(&self, reference: &RegistryReference) -> PathBuf {
        let key = sha256_hex(
            format!(
                "{}:{}:{}",
                reference.namespace, reference.id, reference.version_range
            )
            .as_bytes(),
        );
        self.root.join("refs").join(format!("{key}.json"))
    }

    fn public_metadata_path(&self) -> PathBuf {
        self.root.join("public-metadata").join("current.json")
    }
}

/// A sanitized, searchable public capability record (Spec 116).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PublicCapabilityMetadata {
    pub namespace: String,
    pub id: String,
    pub version: String,
    pub artifact_digest: String,
    pub source_release: String,
    pub index_digest: String,
    pub summary: String,
    pub description: String,
    pub scenarios: Vec<String>,
    pub service_type: String,
    pub permitted_targets: Vec<String>,
    pub lifecycle: String,
    pub provenance: Option<Value>,
}

/// One complete, verified public metadata generation.
///
/// The generation-level provenance is retained even when a consumer's query
/// matches no individual capability records.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PublicMetadataRead {
    pub records: Vec<PublicCapabilityMetadata>,
    pub stale: bool,
    pub source_release: String,
    pub index_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct PublicMetadataGeneration {
    schema_version: u32,
    stale: bool,
    source_release: String,
    index_digest: String,
    records: Vec<PublicCapabilityMetadata>,
}

/// Atomically publish the verified public-only projection of a synced registry state.
/// This is explicit preparation; it performs neither network I/O nor implicit refresh.
///
/// # Errors
///
/// Returns a stable registry-cache error when the supplied state is empty or
/// the complete generation cannot be atomically persisted.
pub fn publish_public_metadata(
    cache: &HostRegistryCache,
    snapshot: &SyncedPublicRegistryState,
    stale: bool,
) -> Result<(), RegistryCacheError> {
    if snapshot.capabilities.is_empty() {
        return Err(RegistryCacheError::new(
            RegistryCacheErrorCode::RegistrySyncMissing,
            "synced registry index snapshot contains no capabilities",
        ));
    }
    let index_digest = index_snapshot_digest(snapshot);
    let records = snapshot
        .capabilities
        .iter()
        .map(|record| PublicCapabilityMetadata {
            namespace: record.namespace.clone(),
            id: record.id.clone(),
            version: record.version.clone(),
            artifact_digest: record.digest.clone(),
            source_release: snapshot.release_tag.clone(),
            index_digest: index_digest.clone(),
            summary: record.summary.clone(),
            description: record.description.clone(),
            scenarios: record
                .use_cases
                .iter()
                .map(|use_case| use_case.scenario.clone())
                .collect(),
            service_type: record.service_type.clone(),
            permitted_targets: record.permitted_targets.clone(),
            lifecycle: record.lifecycle.clone(),
            provenance: record.provenance.clone(),
        })
        .collect();
    write_json_atomic(
        &cache.public_metadata_path(),
        &PublicMetadataGeneration {
            schema_version: 1,
            stale,
            source_release: snapshot.release_tag.clone(),
            index_digest,
            records,
        },
    )
}

/// Read one complete verified metadata generation without network access.
///
/// # Errors
///
/// Returns a stable registry-cache error when the generation is missing,
/// malformed, unsupported, or has invalid digest/provenance bindings.
pub fn read_public_metadata(
    cache: &HostRegistryCache,
) -> Result<PublicMetadataRead, RegistryCacheError> {
    let bytes = fs::read(cache.public_metadata_path()).map_err(|_| {
        RegistryCacheError::new(
            RegistryCacheErrorCode::RegistrySyncMissing,
            "public metadata cache generation is missing",
        )
    })?;
    let generation: PublicMetadataGeneration = serde_json::from_slice(&bytes).map_err(|_| {
        RegistryCacheError::new(
            RegistryCacheErrorCode::RegistryMetadataCacheInvalid,
            "public metadata cache generation is malformed",
        )
    })?;
    if generation.schema_version != 1
        || generation.source_release.is_empty()
        || normalize_digest(&generation.index_digest).is_none()
        || generation.records.iter().any(|record| {
            record.namespace.is_empty()
                || record.id.is_empty()
                || record.version.is_empty()
                || normalize_digest(&record.artifact_digest).is_none()
                || record.source_release != generation.source_release
                || record.index_digest != generation.index_digest
                || record.service_type.is_empty()
                || record.lifecycle.is_empty()
        })
    {
        return Err(RegistryCacheError::new(
            RegistryCacheErrorCode::RegistryMetadataCacheInvalid,
            "public metadata cache generation has invalid verification bindings",
        ));
    }
    Ok(PublicMetadataRead {
        records: generation.records,
        stale: generation.stale,
        source_release: generation.source_release,
        index_digest: generation.index_digest,
    })
}

/// FR-008 resolution evidence retained after a successful prepare.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryPrepareEvidence {
    /// Registry namespace.
    pub namespace: String,
    /// Capability id.
    pub id: String,
    /// Selected concrete version.
    pub selected_version: String,
    /// Requested semver range.
    pub version_range: String,
    /// Source release tag from the synced index snapshot.
    pub source_release: String,
    /// Digest of the synced index snapshot supplied to prepare.
    pub index_digest: String,
    /// Published artifact digest.
    pub artifact_digest: String,
    /// Unix-epoch seconds when the entry was verified.
    pub verified_at: u64,
    /// Outcome label (`prepared`).
    pub outcome: &'static str,
}

impl RegistryPrepareEvidence {
    /// Secret-free JSON projection of prepare evidence.
    #[must_use]
    pub fn as_value(&self) -> Value {
        json!({
            "namespace": self.namespace,
            "id": self.id,
            "selected_version": self.selected_version,
            "version_range": self.version_range,
            "source_release": self.source_release,
            "index_digest": self.index_digest,
            "artifact_digest": self.artifact_digest,
            "verified_at": self.verified_at,
            "outcome": self.outcome,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct CacheEntryMeta {
    namespace: String,
    id: String,
    selected_version: String,
    version_range: String,
    source_release: String,
    index_digest: String,
    artifact_digest: String,
    contract_digest: String,
    verified_at: u64,
}

/// Offline lookup of a previously prepared `registry_ref`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedRegistryDependency {
    /// Absolute path to digest-verified WASM bytes.
    pub wasm_binary_path: PathBuf,
    /// Absolute path to digest-verified contract JSON.
    pub contract_path: PathBuf,
    /// Digest-verified contract bytes.
    pub contract_bytes: Vec<u8>,
    /// Published artifact digest.
    pub wasm_digest: String,
    /// Prepare evidence for the selected record.
    pub evidence: RegistryPrepareEvidence,
}

/// Prepare one `registry_ref` into a host-owned verified cache.
///
/// Only this function may call `fetcher`. Partial writes never become
/// executable entries.
///
/// # Errors
///
/// Returns a Spec 080 FR-007 code when the snapshot is empty, version
/// selection fails, fetch fails, or digest verification fails.
pub fn prepare(
    cache: &HostRegistryCache,
    snapshot: &SyncedPublicRegistryState,
    reference: &RegistryReference,
    fetcher: &dyn RegistryArtifactFetcher,
) -> Result<RegistryPrepareEvidence, RegistryCacheError> {
    if snapshot.capabilities.is_empty() {
        return Err(RegistryCacheError::new(
            RegistryCacheErrorCode::RegistrySyncMissing,
            "synced registry index snapshot contains no capabilities",
        ));
    }
    let record = select_highest_active(snapshot, reference)?;
    let index_digest = index_snapshot_digest(snapshot);
    let artifact_bytes = fetcher.fetch(&record.artifact_url).map_err(|message| {
        RegistryCacheError::new(
            RegistryCacheErrorCode::RegistryPrepareFailed,
            format!("host registry artifact fetch failed: {message}"),
        )
    })?;
    let artifact_hex = verify_digest(&record.digest, &artifact_bytes)?;
    let contract_bytes = fetcher.fetch(&record.contract_url).map_err(|message| {
        RegistryCacheError::new(
            RegistryCacheErrorCode::RegistryPrepareFailed,
            format!("host registry contract fetch failed: {message}"),
        )
    })?;
    let contract_hex = verify_digest(&record.contract_digest, &contract_bytes)?;

    write_verified_bytes(cache, &artifact_hex, &artifact_bytes)?;
    write_verified_bytes(cache, &contract_hex, &contract_bytes)?;

    let verified_at = unix_seconds();
    let meta = CacheEntryMeta {
        namespace: record.namespace.clone(),
        id: record.id.clone(),
        selected_version: record.version.clone(),
        version_range: reference.version_range.clone(),
        source_release: snapshot.release_tag.clone(),
        index_digest: index_digest.clone(),
        artifact_digest: record.digest.clone(),
        contract_digest: record.contract_digest.clone(),
        verified_at,
    };
    write_json_atomic(&cache.meta_path(&artifact_hex), &meta)?;
    write_json_atomic(
        &cache.ref_path(reference),
        &json!({
            "artifact_digest": record.digest,
            "contract_digest": record.contract_digest,
        }),
    )?;

    Ok(RegistryPrepareEvidence {
        namespace: record.namespace,
        id: record.id,
        selected_version: record.version,
        version_range: reference.version_range.clone(),
        source_release: snapshot.release_tag.clone(),
        index_digest,
        artifact_digest: record.digest,
        verified_at,
        outcome: "prepared",
    })
}

/// Resolve a `registry_ref` from verified local cache entries only.
///
/// # Errors
///
/// Returns [`RegistryCacheErrorCode::RegistryCacheEntryMissing`] when no
/// verified prepare result exists, or digest mismatch when stored bytes were
/// tampered with.
pub fn resolve_offline(
    cache: &HostRegistryCache,
    reference: &RegistryReference,
) -> Result<VerifiedRegistryDependency, RegistryCacheError> {
    let ref_path = cache.ref_path(reference);
    let ref_bytes = fs::read(&ref_path).map_err(|_| {
        RegistryCacheError::new(
            RegistryCacheErrorCode::RegistryCacheEntryMissing,
            "verified registry cache entry is missing for registry_ref",
        )
    })?;
    let pointer: Value = serde_json::from_slice(&ref_bytes).map_err(|_| {
        RegistryCacheError::new(
            RegistryCacheErrorCode::RegistryCacheEntryMissing,
            "verified registry cache entry is missing for registry_ref",
        )
    })?;
    let artifact_digest = pointer
        .get("artifact_digest")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            RegistryCacheError::new(
                RegistryCacheErrorCode::RegistryCacheEntryMissing,
                "verified registry cache entry is missing for registry_ref",
            )
        })?;
    let contract_digest = pointer
        .get("contract_digest")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            RegistryCacheError::new(
                RegistryCacheErrorCode::RegistryCacheEntryMissing,
                "verified registry cache entry is missing for registry_ref",
            )
        })?;
    let artifact_hex = normalize_digest(artifact_digest).ok_or_else(|| {
        RegistryCacheError::new(
            RegistryCacheErrorCode::RegistryCacheEntryMissing,
            "verified registry cache entry is missing for registry_ref",
        )
    })?;
    let contract_hex = normalize_digest(contract_digest).ok_or_else(|| {
        RegistryCacheError::new(
            RegistryCacheErrorCode::RegistryCacheEntryMissing,
            "verified registry cache entry is missing for registry_ref",
        )
    })?;
    let (wasm_binary_path, _) = read_verified(cache, &artifact_hex, artifact_digest)?;
    let (contract_path, contract_bytes) = read_verified(cache, &contract_hex, contract_digest)?;
    let meta_bytes = fs::read(cache.meta_path(&artifact_hex)).map_err(|_| {
        RegistryCacheError::new(
            RegistryCacheErrorCode::RegistryCacheEntryMissing,
            "verified registry cache entry is missing for registry_ref",
        )
    })?;
    let meta: CacheEntryMeta = serde_json::from_slice(&meta_bytes).map_err(|_| {
        RegistryCacheError::new(
            RegistryCacheErrorCode::RegistryCacheEntryMissing,
            "verified registry cache entry is missing for registry_ref",
        )
    })?;
    Ok(VerifiedRegistryDependency {
        wasm_binary_path,
        contract_path,
        contract_bytes,
        wasm_digest: artifact_digest.to_string(),
        evidence: RegistryPrepareEvidence {
            namespace: meta.namespace,
            id: meta.id,
            selected_version: meta.selected_version,
            version_range: meta.version_range,
            source_release: meta.source_release,
            index_digest: meta.index_digest,
            artifact_digest: meta.artifact_digest,
            verified_at: meta.verified_at,
            outcome: "resolved",
        },
    })
}

/// Resolve a `registry_ref` into a registry component materialization.
///
/// # Errors
///
/// Returns Spec 080 FR-007 codes when the entry is missing, tampered, or the
/// verified contract bytes cannot be parsed.
pub fn resolve_component(
    cache: &HostRegistryCache,
    reference: &RegistryReference,
) -> Result<ResolvedRegistryComponent, RegistryCacheError> {
    let resolved = resolve_offline(cache, reference)?;
    let contract_text = String::from_utf8(resolved.contract_bytes).map_err(|_| {
        RegistryCacheError::new(
            RegistryCacheErrorCode::RegistryPrepareFailed,
            "verified registry contract is invalid: not utf-8",
        )
    })?;
    let contract = parse_contract(&contract_text).map_err(|failure| {
        RegistryCacheError::new(
            RegistryCacheErrorCode::RegistryPrepareFailed,
            format!(
                "verified registry contract is invalid: {}",
                failure
                    .errors
                    .first()
                    .map_or("parse failed", |error| error.message.as_str())
            ),
        )
    })?;
    Ok(ResolvedRegistryComponent {
        contract_path: resolved.contract_path,
        contract,
        wasm_binary_path: resolved.wasm_binary_path,
        wasm_digest: resolved.wasm_digest,
    })
}

fn select_highest_active(
    snapshot: &SyncedPublicRegistryState,
    reference: &RegistryReference,
) -> Result<PublicRegistryCapabilityRecord, RegistryCacheError> {
    let requirement = semver::VersionReq::parse(&reference.version_range).map_err(|error| {
        RegistryCacheError::new(
            RegistryCacheErrorCode::RegistryVersionNotFound,
            format!(
                "invalid registry_ref version_range {}: {error}",
                reference.version_range
            ),
        )
    })?;
    let matching = snapshot
        .capabilities
        .iter()
        .filter_map(|record| {
            if record.namespace != reference.namespace || record.id != reference.id {
                return None;
            }
            semver::Version::parse(&record.version)
                .ok()
                .filter(|version| requirement.matches(version))
                .map(|version| (version, record.clone()))
        })
        .collect::<Vec<_>>();
    let mut active = matching
        .iter()
        .filter(|(_, record)| !record.deprecated)
        .cloned()
        .collect::<Vec<_>>();
    active.sort_by(|left, right| right.0.cmp(&left.0));
    if let Some((_, record)) = active.into_iter().next() {
        return Ok(record);
    }
    if matching.is_empty() {
        Err(RegistryCacheError::new(
            RegistryCacheErrorCode::RegistryVersionNotFound,
            format!(
                "no synced public registry version for {}:{} satisfies {}",
                reference.namespace, reference.id, reference.version_range
            ),
        ))
    } else {
        Err(RegistryCacheError::new(
            RegistryCacheErrorCode::RegistryDependencyYanked,
            format!(
                "only yanked public registry versions for {}:{} satisfy {}",
                reference.namespace, reference.id, reference.version_range
            ),
        ))
    }
}

pub(crate) fn index_snapshot_digest(snapshot: &SyncedPublicRegistryState) -> String {
    let encoded = serde_json::to_vec(snapshot).unwrap_or_default();
    format!("sha256:{}", sha256_hex(&encoded))
}

fn verify_digest(declared: &str, bytes: &[u8]) -> Result<String, RegistryCacheError> {
    let expected = normalize_digest(declared).ok_or_else(|| {
        RegistryCacheError::new(
            RegistryCacheErrorCode::RegistryArtifactDigestMismatch,
            "registry digest must be sha256: followed by 64 hex characters",
        )
    })?;
    if sha256_hex(bytes) == expected {
        Ok(expected)
    } else {
        Err(RegistryCacheError::new(
            RegistryCacheErrorCode::RegistryArtifactDigestMismatch,
            "registry artifact bytes do not match the published digest",
        ))
    }
}

fn write_verified_bytes(
    cache: &HostRegistryCache,
    digest_hex: &str,
    bytes: &[u8],
) -> Result<PathBuf, RegistryCacheError> {
    let path = cache.artifact_path(digest_hex);
    if path.exists() {
        let cached = fs::read(&path).map_err(|error| {
            RegistryCacheError::new(
                RegistryCacheErrorCode::RegistryPrepareFailed,
                format!("failed to read existing verified cache entry: {error}"),
            )
        })?;
        if sha256_hex(&cached) != digest_hex {
            return Err(RegistryCacheError::new(
                RegistryCacheErrorCode::RegistryArtifactDigestMismatch,
                "existing registry cache entry digest mismatch",
            ));
        }
        return Ok(path);
    }
    let parent = cache.root.join("sha256");
    fs::create_dir_all(&parent).map_err(|error| {
        RegistryCacheError::new(
            RegistryCacheErrorCode::RegistryPrepareFailed,
            format!("failed to create host registry cache directory: {error}"),
        )
    })?;
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, bytes).map_err(|error| {
        let _ = fs::remove_file(&temporary);
        RegistryCacheError::new(
            RegistryCacheErrorCode::RegistryPrepareFailed,
            format!("failed to write registry cache entry: {error}"),
        )
    })?;
    fs::rename(&temporary, &path).map_err(|error| {
        let _ = fs::remove_file(&temporary);
        RegistryCacheError::new(
            RegistryCacheErrorCode::RegistryPrepareFailed,
            format!("failed to commit registry cache entry: {error}"),
        )
    })?;
    Ok(path)
}

fn read_verified(
    cache: &HostRegistryCache,
    digest_hex: &str,
    declared: &str,
) -> Result<(PathBuf, Vec<u8>), RegistryCacheError> {
    let path = cache.artifact_path(digest_hex);
    let bytes = fs::read(&path).map_err(|_| {
        RegistryCacheError::new(
            RegistryCacheErrorCode::RegistryCacheEntryMissing,
            "verified registry cache entry is missing for registry_ref",
        )
    })?;
    verify_digest(declared, &bytes)?;
    Ok((path, bytes))
}

fn write_json_atomic(path: &Path, value: &impl serde::Serialize) -> Result<(), RegistryCacheError> {
    let parent = path.parent().ok_or_else(|| {
        RegistryCacheError::new(
            RegistryCacheErrorCode::RegistryPrepareFailed,
            "registry cache metadata path has no parent directory",
        )
    })?;
    fs::create_dir_all(parent).map_err(|error| {
        RegistryCacheError::new(
            RegistryCacheErrorCode::RegistryPrepareFailed,
            format!("failed to create host registry cache directory: {error}"),
        )
    })?;
    let bytes = serde_json::to_vec(value).map_err(|error| {
        RegistryCacheError::new(
            RegistryCacheErrorCode::RegistryPrepareFailed,
            format!("failed to encode registry cache metadata: {error}"),
        )
    })?;
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, bytes).map_err(|error| {
        let _ = fs::remove_file(&temporary);
        RegistryCacheError::new(
            RegistryCacheErrorCode::RegistryPrepareFailed,
            format!("failed to write registry cache metadata: {error}"),
        )
    })?;
    fs::rename(&temporary, path).map_err(|error| {
        let _ = fs::remove_file(&temporary);
        RegistryCacheError::new(
            RegistryCacheErrorCode::RegistryPrepareFailed,
            format!("failed to commit registry cache metadata: {error}"),
        )
    })?;
    Ok(())
}

fn normalize_digest(digest: &str) -> Option<String> {
    let digest = digest.strip_prefix("sha256:")?;
    if digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Some(digest.to_ascii_lowercase())
    } else {
        None
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut value = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(value, "{byte:02x}");
    }
    value
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]

    use super::*;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    struct MapFetcher {
        assets: HashMap<String, Vec<u8>>,
    }

    impl RegistryArtifactFetcher for MapFetcher {
        fn fetch(&self, url: &str) -> Result<Vec<u8>, String> {
            self.assets
                .get(url)
                .cloned()
                .ok_or_else(|| "missing asset".to_string())
        }
    }

    fn digest_for(bytes: &[u8]) -> String {
        format!("sha256:{}", sha256_hex(bytes))
    }

    fn sample_snapshot(
        deprecated: bool,
    ) -> (SyncedPublicRegistryState, MapFetcher, RegistryReference) {
        let artifact = b"wasm-bytes".to_vec();
        let contract = br#"{"kind":"capability_contract"}"#.to_vec();
        let artifact_digest = digest_for(&artifact);
        let contract_digest = digest_for(&contract);
        let record = PublicRegistryCapabilityRecord {
            namespace: "demo".to_string(),
            id: "greet".to_string(),
            version: "1.2.0".to_string(),
            digest: artifact_digest,
            artifact_url: "https://example.test/greet.wasm".to_string(),
            contract_digest,
            contract_url: "https://example.test/greet.json".to_string(),
            deprecated,
            summary: "Greeting".to_string(),
            description: "Greets a person".to_string(),
            use_cases: Vec::new(),
            service_type: "stateless".to_string(),
            permitted_targets: Vec::new(),
            lifecycle: "active".to_string(),
            provenance: None,
        };
        let older = PublicRegistryCapabilityRecord {
            namespace: "demo".to_string(),
            id: "greet".to_string(),
            version: "1.0.0".to_string(),
            digest: digest_for(b"older"),
            artifact_url: "https://example.test/older.wasm".to_string(),
            contract_digest: digest_for(b"older-contract"),
            contract_url: "https://example.test/older.json".to_string(),
            deprecated: false,
            summary: String::new(),
            description: String::new(),
            use_cases: Vec::new(),
            service_type: "stateless".to_string(),
            permitted_targets: Vec::new(),
            lifecycle: "active".to_string(),
            provenance: None,
        };
        let snapshot = SyncedPublicRegistryState {
            schema_version: "1".to_string(),
            workspace_id: "ws".to_string(),
            state_scope: "public".to_string(),
            source_repo: "traverse-framework/registry".to_string(),
            release_tag: "index-v9".to_string(),
            index_version: 9,
            generated_at: "2026-07-29T00:00:00Z".to_string(),
            source_commit: None,
            synced_at: "2026-07-29T00:00:00Z".to_string(),
            record_count: 2,
            validation_status: "valid".to_string(),
            governing_spec: "055-registry-sync".to_string(),
            capabilities: vec![older, record.clone()],
            events: Vec::new(),
        };
        let mut assets = HashMap::new();
        assets.insert(record.artifact_url.clone(), artifact);
        assets.insert(record.contract_url.clone(), contract);
        let fetcher = MapFetcher { assets };
        let reference = RegistryReference {
            namespace: "demo".to_string(),
            id: "greet".to_string(),
            version_range: "^1.0.0".to_string(),
        };
        (snapshot, fetcher, reference)
    }

    fn unique_cache() -> HostRegistryCache {
        let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("traverse-embedder-cache-{nanos}-{counter}"));
        fs::create_dir_all(&root).expect("temp");
        HostRegistryCache::new(root)
    }

    #[test]
    fn prepare_then_offline_resolve_round_trip() {
        let cache = unique_cache();
        let (snapshot, fetcher, reference) = sample_snapshot(false);
        let evidence = prepare(&cache, &snapshot, &reference, &fetcher).expect("prepare");
        assert_eq!(evidence.selected_version, "1.2.0");
        assert_eq!(evidence.source_release, "index-v9");
        assert_eq!(evidence.outcome, "prepared");
        let resolved = resolve_offline(&cache, &reference).expect("offline");
        assert_eq!(resolved.evidence.selected_version, "1.2.0");
        assert_eq!(
            fs::read(&resolved.wasm_binary_path).expect("wasm"),
            b"wasm-bytes"
        );
    }

    #[test]
    fn offline_resolve_without_prepare_is_missing() {
        let cache = unique_cache();
        let reference = RegistryReference {
            namespace: "demo".to_string(),
            id: "greet".to_string(),
            version_range: "^1.0.0".to_string(),
        };
        let failure = resolve_offline(&cache, &reference).expect_err("missing");
        assert_eq!(
            failure.code,
            RegistryCacheErrorCode::RegistryCacheEntryMissing
        );
    }

    #[test]
    fn yanked_only_range_fails_closed() {
        let cache = unique_cache();
        let (mut snapshot, fetcher, reference) = sample_snapshot(true);
        snapshot
            .capabilities
            .retain(|record| record.version == "1.2.0");
        let failure = prepare(&cache, &snapshot, &reference, &fetcher).expect_err("yanked");
        assert_eq!(
            failure.code,
            RegistryCacheErrorCode::RegistryDependencyYanked
        );
        assert!(
            cache
                .root
                .join("sha256")
                .read_dir()
                .ok()
                .is_none_or(|mut d| d.next().is_none())
        );
    }

    #[test]
    fn digest_mismatch_leaves_no_usable_entry() {
        let cache = unique_cache();
        let (snapshot, mut fetcher, reference) = sample_snapshot(false);
        fetcher.assets.insert(
            "https://example.test/greet.wasm".to_string(),
            b"tampered".to_vec(),
        );
        let failure = prepare(&cache, &snapshot, &reference, &fetcher).expect_err("mismatch");
        assert_eq!(
            failure.code,
            RegistryCacheErrorCode::RegistryArtifactDigestMismatch
        );
        let missing = resolve_offline(&cache, &reference).expect_err("no entry");
        assert_eq!(
            missing.code,
            RegistryCacheErrorCode::RegistryCacheEntryMissing
        );
    }

    #[test]
    fn empty_snapshot_is_sync_missing() {
        let cache = unique_cache();
        let (mut snapshot, fetcher, reference) = sample_snapshot(false);
        snapshot.capabilities.clear();
        let failure = prepare(&cache, &snapshot, &reference, &fetcher).expect_err("empty");
        assert_eq!(failure.code, RegistryCacheErrorCode::RegistrySyncMissing);
    }

    #[test]
    fn evict_removes_prepared_entry() {
        let cache = unique_cache();
        let (snapshot, fetcher, reference) = sample_snapshot(false);
        let evidence = prepare(&cache, &snapshot, &reference, &fetcher).expect("prepare");
        cache.evict(&evidence.artifact_digest).expect("evict");
        let missing = resolve_offline(&cache, &reference).expect_err("gone");
        assert!(matches!(
            missing.code,
            RegistryCacheErrorCode::RegistryCacheEntryMissing
                | RegistryCacheErrorCode::RegistryArtifactDigestMismatch
        ));
    }

    #[test]
    fn error_codes_and_evidence_project_secret_free_values() {
        for (code, wire) in [
            (
                RegistryCacheErrorCode::RegistrySyncMissing,
                "registry_sync_missing",
            ),
            (
                RegistryCacheErrorCode::RegistryVersionNotFound,
                "registry_version_not_found",
            ),
            (
                RegistryCacheErrorCode::RegistryDependencyYanked,
                "registry_dependency_yanked",
            ),
            (
                RegistryCacheErrorCode::RegistryPrepareFailed,
                "registry_prepare_failed",
            ),
            (
                RegistryCacheErrorCode::RegistryArtifactDigestMismatch,
                "registry_artifact_digest_mismatch",
            ),
            (
                RegistryCacheErrorCode::RegistryCacheEntryMissing,
                "registry_cache_entry_missing",
            ),
        ] {
            assert_eq!(code.as_str(), wire);
        }
        let cache = unique_cache();
        assert!(cache.root().exists());
        let (snapshot, fetcher, reference) = sample_snapshot(false);
        let evidence = prepare(&cache, &snapshot, &reference, &fetcher).expect("prepare");
        let value = evidence.as_value();
        assert_eq!(value["outcome"], "prepared");
        assert_eq!(value["selected_version"], "1.2.0");
        cache.evict_all().expect("evict_all");
        assert!(
            resolve_offline(&cache, &reference)
                .expect_err("cleared")
                .code
                == RegistryCacheErrorCode::RegistryCacheEntryMissing
        );
        let invalid = cache.evict("not-a-digest").expect_err("invalid");
        assert_eq!(invalid.code, RegistryCacheErrorCode::RegistryPrepareFailed);
    }

    #[test]
    fn prepare_reports_fetch_and_version_selection_failures() {
        let cache = unique_cache();
        let (snapshot, fetcher, mut reference) = sample_snapshot(false);
        reference.version_range = "not a range!!!".to_string();
        let invalid_range = prepare(&cache, &snapshot, &reference, &fetcher).expect_err("range");
        assert_eq!(
            invalid_range.code,
            RegistryCacheErrorCode::RegistryVersionNotFound
        );

        let (snapshot, fetcher, mut reference) = sample_snapshot(false);
        reference.version_range = "^9.0.0".to_string();
        let missing = prepare(&cache, &snapshot, &reference, &fetcher).expect_err("missing");
        assert_eq!(
            missing.code,
            RegistryCacheErrorCode::RegistryVersionNotFound
        );

        let (snapshot, fetcher, reference) = sample_snapshot(false);
        let empty_fetcher = MapFetcher {
            assets: HashMap::new(),
        };
        let fetch_failed =
            prepare(&cache, &snapshot, &reference, &empty_fetcher).expect_err("fetch");
        assert_eq!(
            fetch_failed.code,
            RegistryCacheErrorCode::RegistryPrepareFailed
        );
        let _ = fetcher;
    }

    #[test]
    fn prepare_rejects_invalid_declared_digests_and_reuses_verified_hits() {
        let cache = unique_cache();
        let (mut snapshot, fetcher, reference) = sample_snapshot(false);
        snapshot.capabilities[1].digest = "sha256:short".to_string();
        let invalid = prepare(&cache, &snapshot, &reference, &fetcher).expect_err("digest");
        assert_eq!(
            invalid.code,
            RegistryCacheErrorCode::RegistryArtifactDigestMismatch
        );

        let (snapshot, fetcher, reference) = sample_snapshot(false);
        let first = prepare(&cache, &snapshot, &reference, &fetcher).expect("first");
        let second = prepare(&cache, &snapshot, &reference, &fetcher).expect("reuse");
        assert_eq!(first.artifact_digest, second.artifact_digest);

        let artifact_hex = normalize_digest(&first.artifact_digest).expect("hex");
        let path = cache.artifact_path(&artifact_hex);
        fs::write(&path, b"tampered").expect("tamper");
        let tampered = prepare(&cache, &snapshot, &reference, &fetcher).expect_err("tampered");
        assert_eq!(
            tampered.code,
            RegistryCacheErrorCode::RegistryArtifactDigestMismatch
        );
    }
    #[test]
    fn offline_resolve_rejects_corrupt_pointers_and_missing_meta() {
        let cache = unique_cache();
        let reference = RegistryReference {
            namespace: "demo".to_string(),
            id: "greet".to_string(),
            version_range: "^1.0.0".to_string(),
        };
        let ref_path = cache.ref_path(&reference);
        fs::create_dir_all(ref_path.parent().expect("parent")).expect("dirs");
        fs::write(&ref_path, b"{not-json").expect("corrupt");
        let corrupt = resolve_offline(&cache, &reference).expect_err("corrupt");
        assert_eq!(
            corrupt.code,
            RegistryCacheErrorCode::RegistryCacheEntryMissing
        );

        fs::write(
            &ref_path,
            br#"{"contract_digest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}"#,
        )
        .expect("partial artifact");
        let missing_artifact = resolve_offline(&cache, &reference).expect_err("artifact");
        assert_eq!(
            missing_artifact.code,
            RegistryCacheErrorCode::RegistryCacheEntryMissing
        );

        fs::write(
            &ref_path,
            br#"{"artifact_digest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","contract_digest":"bad"}"#,
        )
        .expect("bad contract digest");
        let bad_contract = resolve_offline(&cache, &reference).expect_err("contract dig");
        assert_eq!(
            bad_contract.code,
            RegistryCacheErrorCode::RegistryCacheEntryMissing
        );

        fs::write(
            &ref_path,
            br#"{"artifact_digest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}"#,
        )
        .expect("partial");
        let partial = resolve_offline(&cache, &reference).expect_err("partial");
        assert_eq!(
            partial.code,
            RegistryCacheErrorCode::RegistryCacheEntryMissing
        );

        let (snapshot, fetcher, reference) = sample_snapshot(false);
        let evidence = prepare(&cache, &snapshot, &reference, &fetcher).expect("prepare");
        let artifact_hex = normalize_digest(&evidence.artifact_digest).expect("hex");
        fs::remove_file(cache.meta_path(&artifact_hex)).expect("remove meta");
        let missing_meta = resolve_offline(&cache, &reference).expect_err("meta");
        assert_eq!(
            missing_meta.code,
            RegistryCacheErrorCode::RegistryCacheEntryMissing
        );
    }

    #[test]
    fn contract_fetch_failure_and_invalid_contract_digest_fail_closed() {
        let cache = unique_cache();
        let (snapshot, mut fetcher, reference) = sample_snapshot(false);
        fetcher.assets.remove("https://example.test/greet.json");
        let failure = prepare(&cache, &snapshot, &reference, &fetcher).expect_err("contract fetch");
        assert_eq!(failure.code, RegistryCacheErrorCode::RegistryPrepareFailed);

        let (snapshot, mut fetcher, reference) = sample_snapshot(false);
        fetcher.assets.insert(
            "https://example.test/greet.json".to_string(),
            b"wrong-contract".to_vec(),
        );
        let mismatch = prepare(&cache, &snapshot, &reference, &fetcher).expect_err("contract dig");
        assert_eq!(
            mismatch.code,
            RegistryCacheErrorCode::RegistryArtifactDigestMismatch
        );
    }

    #[test]
    fn evict_all_and_missing_evict_paths_are_idempotent() {
        let cache = unique_cache();
        cache.evict_all().expect("empty evict_all");
        let digest = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        cache.evict(digest).expect("missing evict ok");
        let (snapshot, fetcher, reference) = sample_snapshot(false);
        let evidence = prepare(&cache, &snapshot, &reference, &fetcher).expect("prepare");
        cache.evict_all().expect("clear");
        cache.evict(&evidence.artifact_digest).expect("again");
    }

    #[test]
    fn write_verified_bytes_reports_directory_collision_failures() {
        let cache = unique_cache();
        let bytes = b"content";
        let digest = digest_for(bytes);
        let hex = normalize_digest(&digest).expect("hex");
        let path = cache.artifact_path(&hex);
        fs::create_dir_all(&path).expect("block as directory");
        let failure = write_verified_bytes(&cache, &hex, bytes).expect_err("dir");
        assert_eq!(failure.code, RegistryCacheErrorCode::RegistryPrepareFailed);
    }

    #[test]
    fn evict_and_write_json_report_filesystem_failures() {
        // Fail artifact eviction with a non-empty directory collision.
        let cache_art = unique_cache();
        let (snapshot, fetcher, reference) = sample_snapshot(false);
        let evidence = prepare(&cache_art, &snapshot, &reference, &fetcher).expect("prepare");
        let hex = normalize_digest(&evidence.artifact_digest).expect("hex");
        let artifact = cache_art.artifact_path(&hex);
        fs::remove_file(&artifact).expect("remove");
        fs::create_dir_all(&artifact).expect("dir");
        fs::write(artifact.join("nested"), b"x").expect("nested");
        let art_evict = cache_art
            .evict(&evidence.artifact_digest)
            .expect_err("artifact evict");
        assert_eq!(
            art_evict.code,
            RegistryCacheErrorCode::RegistryPrepareFailed
        );

        let cache = unique_cache();
        let (snapshot, fetcher, reference) = sample_snapshot(false);
        let evidence = prepare(&cache, &snapshot, &reference, &fetcher).expect("prepare");
        let hex = normalize_digest(&evidence.artifact_digest).expect("hex");

        // Fail meta eviction: leave artifact removable, block meta path as a directory.
        let meta = cache.meta_path(&hex);
        fs::remove_file(&meta).expect("remove meta file");
        fs::create_dir_all(&meta).expect("meta dir");
        fs::write(meta.join("nested"), b"x").expect("nested");
        let meta_evict = cache
            .evict(&evidence.artifact_digest)
            .expect_err("meta evict");
        assert_eq!(
            meta_evict.code,
            RegistryCacheErrorCode::RegistryPrepareFailed
        );

        let cache2 = unique_cache();
        fs::write(cache2.root.join("sha256"), b"not-dir").expect("file");
        let clear_fail = cache2.evict_all().expect_err("evict_all");
        assert_eq!(
            clear_fail.code,
            RegistryCacheErrorCode::RegistryPrepareFailed
        );

        let cache3 = unique_cache();
        let meta_parent = cache3.root.join("meta");
        fs::write(&meta_parent, b"not-dir").expect("block meta dir");
        let meta_fail = write_json_atomic(
            &cache3.meta_path("cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"),
            &json!({"ok": true}),
        )
        .expect_err("meta");
        assert_eq!(
            meta_fail.code,
            RegistryCacheErrorCode::RegistryPrepareFailed
        );

        // create_dir_all failure for sha256 parent
        let cache4 = unique_cache();
        fs::write(cache4.root.join("sha256"), b"not-dir").expect("block");
        let bytes = b"fresh";
        let digest = digest_for(bytes);
        let hex = normalize_digest(&digest).expect("hex");
        let create_fail = write_verified_bytes(&cache4, &hex, bytes).expect_err("create");
        assert_eq!(
            create_fail.code,
            RegistryCacheErrorCode::RegistryPrepareFailed
        );
    }

    #[test]
    fn write_helpers_report_tmp_rename_and_existing_entry_failures() {
        // temporary write failure: tmp path is a directory
        let cache5 = unique_cache();
        let digest = digest_for(b"another");
        let hex = normalize_digest(&digest).expect("hex");
        let path = cache5.artifact_path(&hex);
        fs::create_dir_all(path.parent().expect("parent")).expect("parent");
        fs::create_dir_all(path.with_extension("tmp")).expect("tmp dir");
        let write_fail = write_verified_bytes(&cache5, &hex, b"another").expect_err("write");
        assert_eq!(
            write_fail.code,
            RegistryCacheErrorCode::RegistryPrepareFailed
        );

        // rename failure: destination is a directory
        let cache6 = unique_cache();
        let digest = digest_for(b"rename-me");
        let hex = normalize_digest(&digest).expect("hex");
        let path = cache6.artifact_path(&hex);
        fs::create_dir_all(path.parent().expect("parent")).expect("parent");
        fs::create_dir_all(&path).expect("dest dir");
        let rename_fail = write_verified_bytes(&cache6, &hex, b"rename-me").expect_err("rename");
        assert_eq!(
            rename_fail.code,
            RegistryCacheErrorCode::RegistryPrepareFailed
        );

        // write_json_atomic rename failure
        let cache7 = unique_cache();
        let target = cache7.root.join("refs").join("blocked.json");
        fs::create_dir_all(target.parent().expect("parent")).expect("parent");
        fs::create_dir_all(&target).expect("dest dir");
        let json_rename = write_json_atomic(&target, &json!({"ok": true})).expect_err("rename");
        assert_eq!(
            json_rename.code,
            RegistryCacheErrorCode::RegistryPrepareFailed
        );

        // write_json_atomic write failure: tmp is a directory
        let cache8 = unique_cache();
        let target = cache8.root.join("refs").join("writeme.json");
        fs::create_dir_all(target.parent().expect("parent")).expect("parent");
        fs::create_dir_all(target.with_extension("tmp")).expect("tmp dir");
        let json_write = write_json_atomic(&target, &json!({"ok": true})).expect_err("write");
        assert_eq!(
            json_write.code,
            RegistryCacheErrorCode::RegistryPrepareFailed
        );

        // existing cache entry that cannot be read as a file
        let cache9 = unique_cache();
        let digest = digest_for(b"readable");
        let hex = normalize_digest(&digest).expect("hex");
        let path = cache9.artifact_path(&hex);
        fs::create_dir_all(&path).expect("dir as entry");
        let read_existing = write_verified_bytes(&cache9, &hex, b"readable").expect_err("read");
        assert_eq!(
            read_existing.code,
            RegistryCacheErrorCode::RegistryPrepareFailed
        );
    }

    #[test]
    fn prepare_fails_when_refs_root_is_blocked_and_meta_can_be_corrupt() {
        let cache = unique_cache();
        fs::write(cache.root.join("refs"), b"not-a-directory").expect("block refs");
        let (snapshot, fetcher, reference) = sample_snapshot(false);
        let failure = prepare(&cache, &snapshot, &reference, &fetcher).expect_err("refs");
        assert_eq!(failure.code, RegistryCacheErrorCode::RegistryPrepareFailed);

        let cache2 = unique_cache();
        let (snapshot, fetcher, reference) = sample_snapshot(false);
        let evidence = prepare(&cache2, &snapshot, &reference, &fetcher).expect("prepare");
        let hex = normalize_digest(&evidence.artifact_digest).expect("hex");
        fs::write(cache2.meta_path(&hex), b"{not-json").expect("corrupt meta");
        let corrupt_meta = resolve_offline(&cache2, &reference).expect_err("meta");
        assert_eq!(
            corrupt_meta.code,
            RegistryCacheErrorCode::RegistryCacheEntryMissing
        );
    }

    #[test]
    fn version_selection_skips_unrelated_namespace_records() {
        let cache = unique_cache();
        let (mut snapshot, fetcher, reference) = sample_snapshot(false);
        snapshot.capabilities.insert(
            0,
            PublicRegistryCapabilityRecord {
                namespace: "other".to_string(),
                id: "thing".to_string(),
                version: "9.9.9".to_string(),
                digest: digest_for(b"other"),
                artifact_url: "https://example.test/other.wasm".to_string(),
                contract_digest: digest_for(b"other-contract"),
                contract_url: "https://example.test/other.json".to_string(),
                deprecated: false,
                summary: String::new(),
                description: String::new(),
                use_cases: Vec::new(),
                service_type: String::new(),
                permitted_targets: Vec::new(),
                lifecycle: String::new(),
                provenance: None,
            },
        );
        let evidence = prepare(&cache, &snapshot, &reference, &fetcher).expect("prepare");
        assert_eq!(evidence.selected_version, "1.2.0");
    }

    #[test]
    fn write_json_atomic_reports_serialize_failures() {
        #[derive(Debug)]
        struct Boom;
        impl serde::Serialize for Boom {
            fn serialize<S: serde::Serializer>(&self, _serializer: S) -> Result<S::Ok, S::Error> {
                Err(serde::ser::Error::custom("boom"))
            }
        }
        let cache = unique_cache();
        let path = cache.root.join("refs").join("boom.json");
        let failure = write_json_atomic(&path, &Boom).expect_err("serialize");
        assert_eq!(failure.code, RegistryCacheErrorCode::RegistryPrepareFailed);
    }

    #[test]
    fn write_json_atomic_rejects_paths_without_parent() {
        let failure = write_json_atomic(Path::new("/"), &serde_json::json!({"ok": true}))
            .expect_err("root path");
        assert_eq!(failure.code, RegistryCacheErrorCode::RegistryPrepareFailed);
        assert!(failure.message.contains("no parent directory"));
    }

    #[cfg(unix)]
    #[test]
    fn write_verified_bytes_reports_rename_failures_on_readonly_parent() {
        use std::os::unix::fs::PermissionsExt;
        let cache = unique_cache();
        let bytes = b"rename-readonly";
        let digest = digest_for(bytes);
        let hex = normalize_digest(&digest).expect("hex");
        let path = cache.artifact_path(&hex);
        let parent = path.parent().expect("parent").to_path_buf();
        fs::create_dir_all(&parent).expect("parent");
        let temporary = path.with_extension("tmp");
        fs::write(&temporary, bytes).expect("tmp");
        let mut perms = fs::metadata(&parent).expect("meta").permissions();
        perms.set_mode(0o555);
        fs::set_permissions(&parent, perms).expect("readonly");
        let failure = write_verified_bytes(&cache, &hex, bytes).expect_err("rename");
        let mut perms = fs::metadata(&parent).expect("meta").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&parent, perms).expect("restore");
        assert_eq!(failure.code, RegistryCacheErrorCode::RegistryPrepareFailed);
    }

    #[test]
    fn offline_resolve_rejects_invalid_digest_shapes_in_pointer() {
        let cache = unique_cache();
        let reference = RegistryReference {
            namespace: "demo".to_string(),
            id: "greet".to_string(),
            version_range: "^1.0.0".to_string(),
        };
        let ref_path = cache.ref_path(&reference);
        fs::create_dir_all(ref_path.parent().expect("parent")).expect("dirs");
        fs::write(
            &ref_path,
            br#"{"artifact_digest":"bad","contract_digest":"bad"}"#,
        )
        .expect("write");
        let failure = resolve_offline(&cache, &reference).expect_err("bad digests");
        assert_eq!(
            failure.code,
            RegistryCacheErrorCode::RegistryCacheEntryMissing
        );
    }

    #[test]
    fn resolve_component_rejects_non_contract_json_with_matching_digest() {
        let cache = unique_cache();
        let bogus = br#"{"kind":"not-a-capability-contract"}"#.to_vec();
        let artifact = b"wasm-bytes".to_vec();
        let artifact_digest = digest_for(&artifact);
        let contract_digest = digest_for(&bogus);
        let record = PublicRegistryCapabilityRecord {
            namespace: "demo".to_string(),
            id: "greet".to_string(),
            version: "1.0.0".to_string(),
            digest: artifact_digest,
            artifact_url: "https://example.test/greet.wasm".to_string(),
            contract_digest,
            contract_url: "https://example.test/greet.json".to_string(),
            deprecated: false,
            summary: String::new(),
            description: String::new(),
            use_cases: Vec::new(),
            service_type: String::new(),
            permitted_targets: Vec::new(),
            lifecycle: String::new(),
            provenance: None,
        };
        let snapshot = SyncedPublicRegistryState {
            schema_version: "1".to_string(),
            workspace_id: "ws".to_string(),
            state_scope: "public".to_string(),
            source_repo: "traverse-framework/registry".to_string(),
            release_tag: "index-v9".to_string(),
            index_version: 9,
            generated_at: "2026-07-29T00:00:00Z".to_string(),
            source_commit: None,
            synced_at: "2026-07-29T00:00:00Z".to_string(),
            record_count: 1,
            validation_status: "valid".to_string(),
            governing_spec: "055-registry-sync".to_string(),
            capabilities: vec![record.clone()],
            events: Vec::new(),
        };
        let mut assets = HashMap::new();
        assets.insert(record.artifact_url.clone(), artifact);
        assets.insert(record.contract_url.clone(), bogus);
        let fetcher = MapFetcher { assets };
        let reference = RegistryReference {
            namespace: "demo".to_string(),
            id: "greet".to_string(),
            version_range: "1.0.0".to_string(),
        };
        prepare(&cache, &snapshot, &reference, &fetcher).expect("prepare");
        let failure = resolve_component(&cache, &reference).expect_err("parse");
        assert_eq!(failure.code, RegistryCacheErrorCode::RegistryPrepareFailed);
    }

    #[test]
    fn resolve_component_rejects_non_utf8_contract_bytes() {
        let cache = unique_cache();
        let bogus = vec![0xff, 0xfe, 0xfd];
        let artifact = b"wasm-bytes".to_vec();
        let artifact_digest = digest_for(&artifact);
        let contract_digest = digest_for(&bogus);
        let record = PublicRegistryCapabilityRecord {
            namespace: "demo".to_string(),
            id: "greet".to_string(),
            version: "1.0.0".to_string(),
            digest: artifact_digest,
            artifact_url: "https://example.test/greet.wasm".to_string(),
            contract_digest,
            contract_url: "https://example.test/greet.json".to_string(),
            deprecated: false,
            summary: String::new(),
            description: String::new(),
            use_cases: Vec::new(),
            service_type: String::new(),
            permitted_targets: Vec::new(),
            lifecycle: String::new(),
            provenance: None,
        };
        let snapshot = SyncedPublicRegistryState {
            schema_version: "1".to_string(),
            workspace_id: "ws".to_string(),
            state_scope: "public".to_string(),
            source_repo: "traverse-framework/registry".to_string(),
            release_tag: "index-v9".to_string(),
            index_version: 9,
            generated_at: "2026-07-29T00:00:00Z".to_string(),
            source_commit: None,
            synced_at: "2026-07-29T00:00:00Z".to_string(),
            record_count: 1,
            validation_status: "valid".to_string(),
            governing_spec: "055-registry-sync".to_string(),
            capabilities: vec![record.clone()],
            events: Vec::new(),
        };
        let mut assets = HashMap::new();
        assets.insert(record.artifact_url.clone(), artifact);
        assets.insert(record.contract_url.clone(), bogus);
        let fetcher = MapFetcher { assets };
        let reference = RegistryReference {
            namespace: "demo".to_string(),
            id: "greet".to_string(),
            version_range: "1.0.0".to_string(),
        };
        prepare(&cache, &snapshot, &reference, &fetcher).expect("prepare");
        let failure = resolve_component(&cache, &reference).expect_err("utf8");
        assert_eq!(failure.code, RegistryCacheErrorCode::RegistryPrepareFailed);
    }

    #[test]
    fn public_metadata_round_trip_is_sanitized_and_can_be_stale() {
        let cache = unique_cache();
        let (mut snapshot, _, _) = sample_snapshot(false);
        let record = snapshot.capabilities.last_mut().expect("record");
        record.summary = "Greeting capability".to_string();
        record.description = "Greets a public user".to_string();
        record.use_cases = vec![traverse_registry::PublicUseCaseSummary {
            scenario: "Greet a new user".to_string(),
        }];
        publish_public_metadata(&cache, &snapshot, true).expect("publish");
        let generation = read_public_metadata(&cache).expect("read");
        assert!(generation.stale);
        assert_eq!(generation.source_release, snapshot.release_tag);
        assert!(!generation.index_digest.is_empty());
        assert!(
            generation
                .records
                .iter()
                .any(|record| record.description == "Greets a public user"
                    && record.scenarios == ["Greet a new user"])
        );
        let encoded = fs::read(cache.public_metadata_path()).expect("generation");
        let text = String::from_utf8(encoded).expect("utf8");
        assert!(!text.contains("input_example"));
        assert!(!text.contains("output_example"));
    }

    #[test]
    fn public_metadata_rejects_tampered_generation_binding() {
        let cache = unique_cache();
        let (snapshot, _, _) = sample_snapshot(false);
        publish_public_metadata(&cache, &snapshot, false).expect("publish");
        let path = cache.public_metadata_path();
        let mut generation: Value =
            serde_json::from_slice(&fs::read(&path).expect("read")).expect("json");
        generation["records"][0]["index_digest"] =
            json!("sha256:0000000000000000000000000000000000000000000000000000000000000000");
        write_json_atomic(&path, &generation).expect("tamper");
        assert_eq!(
            read_public_metadata(&cache).expect_err("invalid").code,
            RegistryCacheErrorCode::RegistryMetadataCacheInvalid
        );
    }

    #[test]
    fn public_metadata_rejects_empty_snapshots_and_has_stable_error_code() {
        let cache = unique_cache();
        let (mut snapshot, _, _) = sample_snapshot(false);
        snapshot.capabilities.clear();
        assert_eq!(
            publish_public_metadata(&cache, &snapshot, false)
                .expect_err("empty")
                .code,
            RegistryCacheErrorCode::RegistrySyncMissing
        );
        assert_eq!(
            RegistryCacheErrorCode::RegistryMetadataCacheInvalid.as_str(),
            "registry_metadata_cache_invalid"
        );
    }

    #[test]
    fn public_metadata_fails_closed_for_missing_and_malformed_generations() {
        let cache = unique_cache();
        assert_eq!(
            read_public_metadata(&cache).expect_err("missing").code,
            RegistryCacheErrorCode::RegistrySyncMissing
        );
        let path = cache.public_metadata_path();
        fs::create_dir_all(path.parent().expect("parent")).expect("directory");
        fs::write(&path, b"not json").expect("malformed generation");
        assert_eq!(
            read_public_metadata(&cache).expect_err("malformed").code,
            RegistryCacheErrorCode::RegistryMetadataCacheInvalid
        );
    }
}
