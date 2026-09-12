//! Mode B host CLI: prepare a Spec 080 / Spec 520 verified registry cache.
//!
//! Network I/O is confined to this explicit prepare step. The MCP stdio host
//! then serves discover/validate/execute/report from the resulting cache only.

use serde_json::{Value, json};
use std::fs;
use std::path::Path;
use std::process::Command;
use traverse_embedder::{
    HostRegistryCache, RegistryArtifactFetcher, RegistryCacheError, RegistryCacheErrorCode,
    RegistryPrepareEvidence, prepare_registry_dependency, publish_public_metadata,
};
use traverse_registry::{RegistryReference, SyncedPublicRegistryState};

const GOVERNING_SPEC: &str = "080-embedded-registry-cache";

/// Secret-free Mode B prepare-cache failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrepareCacheError {
    /// Stable Spec 080 FR-007 code (or `registry_ref_invalid` for CLI parse).
    pub code: String,
    /// Human-readable explanation without paths, credentials, or bytes.
    pub message: String,
}

impl PrepareCacheError {
    fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }

    fn from_cache(error: RegistryCacheError) -> Self {
        Self {
            code: error.code.as_str().to_string(),
            message: error.message,
        }
    }

    /// Machine-readable error envelope for `--json` callers.
    #[must_use]
    pub fn envelope(&self) -> Value {
        json!({
            "kind": "mcp_mode_b_prepare_cache_error",
            "governing_spec": GOVERNING_SPEC,
            "code": self.code,
            "message": self.message,
        })
    }
}

impl std::fmt::Display for PrepareCacheError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for PrepareCacheError {}

/// Successful Mode B cache preparation evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrepareCacheEvidence {
    /// One prepare result per requested (or snapshot-derived) `registry_ref`.
    pub entries: Vec<RegistryPrepareEvidence>,
}

impl PrepareCacheEvidence {
    /// Machine-readable success envelope for `--json` callers.
    #[must_use]
    pub fn envelope(&self) -> Value {
        json!({
            "kind": "mcp_mode_b_prepare_cache",
            "governing_spec": GOVERNING_SPEC,
            "cache_prepared": true,
            "entry_count": self.entries.len(),
            "entries": self.entries.iter().map(|entry| json!({
                "namespace": entry.namespace,
                "id": entry.id,
                "selected_version": entry.selected_version,
                "version_range": entry.version_range,
                "source_release": entry.source_release,
                "index_digest": entry.index_digest,
                "artifact_digest": entry.artifact_digest,
                "outcome": entry.outcome,
            })).collect::<Vec<_>>(),
        })
    }

    /// Human-readable success text without cache paths.
    #[must_use]
    pub fn render(&self) -> String {
        let mut lines = vec![format!(
            "prepared {} registry_ref(s) into a verified Spec 520 cache",
            self.entries.len()
        )];
        for entry in &self.entries {
            lines.push(format!(
                "{}@{} {}",
                entry.id, entry.selected_version, entry.artifact_digest
            ));
        }
        lines.join("\n")
    }
}

/// Prepare one or more public `registry_ref` values into a host-owned cache.
///
/// When `refs` is empty, every non-deprecated snapshot capability is prepared
/// at its exact published version (`={version}`).
///
/// # Errors
///
/// Returns a stable, secret-free code when the synced snapshot is missing or
/// invalid, a ref cannot be parsed, or Spec 080 prepare fails.
pub fn prepare_verified_cache(
    synced_state_path: &Path,
    cache_root: &Path,
    refs: &[String],
) -> Result<PrepareCacheEvidence, PrepareCacheError> {
    let snapshot = load_synced_state(synced_state_path)?;
    let references = if refs.is_empty() {
        snapshot_exact_refs(&snapshot)?
    } else {
        refs.iter()
            .map(|raw| parse_registry_ref(raw))
            .collect::<Result<Vec<_>, _>>()?
    };
    if references.is_empty() {
        return Err(PrepareCacheError::new(
            RegistryCacheErrorCode::RegistrySyncMissing.as_str(),
            "synced registry index snapshot contains no preparable capabilities",
        ));
    }

    prepare_refs(&snapshot, cache_root, &references)
}

/// Parse `--ref <namespace>/<id>@<version_range>`.
///
/// # Errors
///
/// Returns `registry_ref_invalid` when the token is missing a namespace, id,
/// or version range.
pub fn parse_registry_ref(raw: &str) -> Result<RegistryReference, PrepareCacheError> {
    let (namespace, rest) = raw.split_once('/').ok_or_else(|| {
        PrepareCacheError::new(
            "registry_ref_invalid",
            "registry_ref must be namespace/id@version_range",
        )
    })?;
    let (id, version_range) = rest.split_once('@').ok_or_else(|| {
        PrepareCacheError::new(
            "registry_ref_invalid",
            "registry_ref must be namespace/id@version_range",
        )
    })?;
    if namespace.is_empty() || id.is_empty() || version_range.is_empty() {
        return Err(PrepareCacheError::new(
            "registry_ref_invalid",
            "registry_ref must be namespace/id@version_range",
        ));
    }
    Ok(RegistryReference {
        namespace: namespace.to_string(),
        id: id.to_string(),
        version_range: version_range.to_string(),
    })
}

fn load_synced_state(path: &Path) -> Result<SyncedPublicRegistryState, PrepareCacheError> {
    let raw = fs::read(path).map_err(|_| {
        PrepareCacheError::new(
            RegistryCacheErrorCode::RegistrySyncMissing.as_str(),
            "synced registry index snapshot is missing",
        )
    })?;
    serde_json::from_slice(&raw).map_err(|_| {
        PrepareCacheError::new(
            RegistryCacheErrorCode::RegistrySyncMissing.as_str(),
            "synced registry index snapshot is missing or malformed",
        )
    })
}

fn snapshot_exact_refs(
    snapshot: &SyncedPublicRegistryState,
) -> Result<Vec<RegistryReference>, PrepareCacheError> {
    let refs = snapshot
        .capabilities
        .iter()
        .filter(|record| !record.deprecated)
        .map(|record| RegistryReference {
            namespace: record.namespace.clone(),
            id: record.id.clone(),
            version_range: format!("={}", record.version),
        })
        .collect::<Vec<_>>();
    if refs.is_empty() {
        return Err(PrepareCacheError::new(
            RegistryCacheErrorCode::RegistrySyncMissing.as_str(),
            "synced registry index snapshot contains no preparable capabilities",
        ));
    }
    Ok(refs)
}

fn prepare_refs(
    snapshot: &SyncedPublicRegistryState,
    cache_root: &Path,
    references: &[RegistryReference],
) -> Result<PrepareCacheEvidence, PrepareCacheError> {
    let cache = HostRegistryCache::new(cache_root);
    let fetcher = HostCliFetcher;
    let mut entries = Vec::with_capacity(references.len());
    for reference in references {
        let evidence = prepare_registry_dependency(&cache, snapshot, reference, &fetcher)
            .map_err(PrepareCacheError::from_cache)?;
        entries.push(evidence);
    }
    publish_public_metadata(&cache, snapshot, false).map_err(PrepareCacheError::from_cache)?;
    Ok(PrepareCacheEvidence { entries })
}

/// Host-owned fetcher: `file://` from local bytes, `http(s)://` via curl.
///
/// Errors are secret-free and never echo URLs, paths, or artifact bytes.
struct HostCliFetcher;

impl RegistryArtifactFetcher for HostCliFetcher {
    fn fetch(&self, url: &str) -> Result<Vec<u8>, String> {
        if let Some(path) = url.strip_prefix("file://") {
            return fs::read(path).map_err(|_| "host registry artifact fetch failed".to_string());
        }
        if url.starts_with("https://") || url.starts_with("http://") {
            return fetch_http(url);
        }
        Err("host registry artifact fetch failed: unsupported url scheme".to_string())
    }
}

fn fetch_http(url: &str) -> Result<Vec<u8>, String> {
    let output = Command::new("curl")
        .args(["-fsSL", url])
        .output()
        .map_err(|_| "host registry artifact fetch failed".to_string())?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err("host registry artifact fetch failed".to_string())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use sha2::{Digest, Sha256};
    use std::fmt::Write as _;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use traverse_embedder::read_public_metadata;
    use traverse_registry::{PublicRegistryCapabilityRecord, PublicUseCaseSummary};

    const KIT_ID: &str = "core.normalize-participants";
    const KIT_NAMESPACE: &str = "core";
    const KIT_VERSION: &str = "1.1.0";

    fn repo_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("workspace root")
            .to_path_buf()
    }

    fn sha256_prefixed(bytes: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        let mut out = String::from("sha256:");
        for byte in hasher.finalize() {
            write!(out, "{byte:02x}").expect("writing to a String cannot fail");
        }
        out
    }

    fn kit_wasm_bytes() -> Vec<u8> {
        fs::read(repo_root().join(
            "examples/core-normalize-participants/artifacts/core-normalize-participants.wasm",
        ))
        .expect("kit wasm")
    }

    fn kit_contract_bytes() -> Vec<u8> {
        fs::read(repo_root().join("examples/core-normalize-participants/contract.json"))
            .expect("kit contract")
    }

    fn fresh_root(tag: &str) -> PathBuf {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "traverse-mcp-mode-b-{tag}-{}-{seq}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("temp root");
        root
    }

    fn write_snapshot(dir: &Path) -> PathBuf {
        let wasm = repo_root().join(
            "examples/core-normalize-participants/artifacts/core-normalize-participants.wasm",
        );
        let contract = repo_root().join("examples/core-normalize-participants/contract.json");
        let record = PublicRegistryCapabilityRecord {
            namespace: KIT_NAMESPACE.to_string(),
            id: KIT_ID.to_string(),
            version: KIT_VERSION.to_string(),
            digest: sha256_prefixed(&kit_wasm_bytes()),
            artifact_url: format!("file://{}", wasm.display()),
            contract_digest: sha256_prefixed(&kit_contract_bytes()),
            contract_url: format!("file://{}", contract.display()),
            deprecated: false,
            summary: "Normalize raw participants into canonical records.".to_string(),
            description: "Verified public kit fixture for Mode B prepare.".to_string(),
            use_cases: vec![PublicUseCaseSummary {
                scenario: "Resolve extracted names and emails to workspace members.".to_string(),
            }],
            service_type: "stateless".to_string(),
            permitted_targets: vec!["wasm".to_string()],
            lifecycle: "active".to_string(),
            provenance: None,
        };
        let snapshot = SyncedPublicRegistryState {
            schema_version: "1.0.0".to_string(),
            workspace_id: "mode-b-fixture".to_string(),
            state_scope: "public_registry_synced".to_string(),
            source_repo: "traverse-framework/registry".to_string(),
            release_tag: "index-v1".to_string(),
            index_version: 1,
            generated_at: "2026-09-11T00:00:00Z".to_string(),
            source_commit: None,
            synced_at: "2026-09-11T00:00:00Z".to_string(),
            record_count: 1,
            validation_status: "valid".to_string(),
            governing_spec: "055-registry-sync".to_string(),
            capabilities: vec![record],
            events: Vec::new(),
        };
        let path = dir.join("synced-state.json");
        fs::write(&path, serde_json::to_vec(&snapshot).expect("snapshot json")).expect("write");
        path
    }

    #[test]
    fn parse_registry_ref_accepts_namespace_id_and_range() {
        let parsed = parse_registry_ref("core/core.normalize-participants@=1.1.0").unwrap();
        assert_eq!(parsed.namespace, "core");
        assert_eq!(parsed.id, KIT_ID);
        assert_eq!(parsed.version_range, "=1.1.0");
    }

    #[test]
    fn parse_registry_ref_rejects_malformed_tokens() {
        for raw in ["", "core", "core/id", "/id@=1", "core/@=1", "core/id@"] {
            let error = parse_registry_ref(raw).expect_err(raw);
            assert_eq!(error.code, "registry_ref_invalid");
            assert!(!error.envelope().to_string().contains(raw) || raw.is_empty());
        }
    }

    #[test]
    fn prepare_writes_verified_cache_and_public_metadata() {
        let root = fresh_root("prepare");
        let state = write_snapshot(&root);
        let cache = root.join("cache");
        let evidence = prepare_verified_cache(
            &state,
            &cache,
            &[format!("{KIT_NAMESPACE}/{KIT_ID}@={KIT_VERSION}")],
        )
        .expect("prepare");
        assert_eq!(evidence.entries.len(), 1);
        assert_eq!(evidence.entries[0].id, KIT_ID);
        assert_eq!(evidence.entries[0].selected_version, KIT_VERSION);
        assert_eq!(evidence.entries[0].outcome, "prepared");
        let envelope = evidence.envelope();
        assert_eq!(envelope["kind"], json!("mcp_mode_b_prepare_cache"));
        assert_eq!(envelope["governing_spec"], json!(GOVERNING_SPEC));
        assert!(
            !envelope
                .to_string()
                .contains(cache.to_string_lossy().as_ref())
        );
        let generation = read_public_metadata(&HostRegistryCache::new(&cache)).expect("metadata");
        assert_eq!(generation.records.len(), 1);
        assert_eq!(generation.records[0].id, KIT_ID);
        let rendered = evidence.render();
        assert!(rendered.contains(KIT_ID));
        assert!(!rendered.contains(cache.to_string_lossy().as_ref()));
    }

    #[test]
    fn prepare_without_refs_uses_exact_snapshot_versions() {
        let root = fresh_root("all-refs");
        let state = write_snapshot(&root);
        let cache = root.join("cache");
        let evidence = prepare_verified_cache(&state, &cache, &[]).expect("prepare all");
        assert_eq!(evidence.entries[0].version_range, format!("={KIT_VERSION}"));
    }

    #[test]
    fn prepare_fails_closed_for_missing_and_malformed_state() {
        let root = fresh_root("missing");
        let cache = root.join("cache");
        let missing = prepare_verified_cache(&root.join("absent.json"), &cache, &[])
            .expect_err("missing state");
        assert_eq!(missing.code, "registry_sync_missing");
        fs::write(root.join("bad.json"), b"{not-json").expect("bad json");
        let malformed =
            prepare_verified_cache(&root.join("bad.json"), &cache, &[]).expect_err("malformed");
        assert_eq!(malformed.code, "registry_sync_missing");
        assert_eq!(
            malformed.envelope()["kind"],
            json!("mcp_mode_b_prepare_cache_error")
        );
    }

    #[test]
    fn prepare_fails_closed_for_unknown_ref() {
        let root = fresh_root("unknown-ref");
        let state = write_snapshot(&root);
        let cache = root.join("cache");
        let error = prepare_verified_cache(
            &state,
            &cache,
            &["other/unknown.capability@=1.0.0".to_string()],
        )
        .expect_err("unknown ref");
        assert_eq!(error.code, "registry_version_not_found");
    }

    #[test]
    fn host_fetcher_reads_file_urls_and_rejects_unknown_schemes() {
        let wasm = repo_root().join(
            "examples/core-normalize-participants/artifacts/core-normalize-participants.wasm",
        );
        let bytes = HostCliFetcher
            .fetch(&format!("file://{}", wasm.display()))
            .expect("file fetch");
        assert_eq!(bytes, kit_wasm_bytes());
        let error = HostCliFetcher
            .fetch("ftp://example.test/module.wasm")
            .expect_err("scheme");
        assert_eq!(
            error,
            "host registry artifact fetch failed: unsupported url scheme"
        );
        let missing = HostCliFetcher
            .fetch("file:///definitely-missing-traverse-mode-b.wasm")
            .expect_err("missing file");
        assert_eq!(missing, "host registry artifact fetch failed");
    }
}
