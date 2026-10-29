//! `traverse-mcp sync` (Spec 119 0.4.0, Decision 121).
//!
//! Writes a Spec 055 synced-state file for `prepare-cache` through the
//! shared `traverse-registry-sync` crate, the same fetch path as
//! `traverse-cli registry sync`. This is the only network step before
//! `prepare-cache`; the stdio host stays offline.

use serde_json::{Value, json};
use std::path::Path;
use traverse_registry::SyncedPublicRegistryState;
use traverse_registry_sync::{
    CurlGitHubRegistryIndexFetcher, RegistryIndexFetcher, RegistrySyncError, sync_to_file,
    unix_timestamp_string,
};

const GOVERNING_SPEC: &str = "119-verified-registry-mcp-mode-a";

/// Stable `traverse-mcp sync` failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpSyncError {
    /// Stable registry-sync code (for example `registry_index_release_missing`).
    pub code: String,
    /// Human-readable explanation.
    pub message: String,
}

impl McpSyncError {
    fn from_sync(error: RegistrySyncError) -> Self {
        Self {
            code: error.code.to_string(),
            message: error.message,
        }
    }

    /// Machine-readable error envelope for `--json` callers.
    #[must_use]
    pub fn envelope(&self) -> Value {
        json!({
            "kind": "mcp_registry_sync_error",
            "governing_spec": GOVERNING_SPEC,
            "code": self.code,
            "message": self.message,
        })
    }
}

impl std::fmt::Display for McpSyncError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for McpSyncError {}

/// Evidence of one written synced-state file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpSyncEvidence {
    /// The synced state as written.
    pub state: SyncedPublicRegistryState,
}

impl McpSyncEvidence {
    /// Machine-readable success envelope for `--json` callers.
    #[must_use]
    pub fn envelope(&self) -> Value {
        json!({
            "kind": "mcp_registry_sync",
            "governing_spec": GOVERNING_SPEC,
            "status": "synced",
            "source": self.state.source_repo,
            "release_tag": self.state.release_tag,
            "index_version": self.state.index_version,
            "record_count": self.state.record_count,
            "synced_at": self.state.synced_at,
        })
    }

    /// Human-readable success text.
    #[must_use]
    pub fn render(&self) -> String {
        format!(
            "synced {} {} ({} capability record(s))",
            self.state.source_repo, self.state.release_tag, self.state.record_count
        )
    }
}

/// Fetch the registry index and atomically write it to `out`.
///
/// Takes the highest `index-v*` release with an `index.json` asset unless
/// `release` pins one. `source_repo` replaces the default public registry;
/// `TRAVERSE_REGISTRY_TOKEN` is read only for such an override.
///
/// # Errors
///
/// Returns a stable registry-sync code; on failure `out` is unchanged.
pub fn sync_registry_state(
    out: &Path,
    release: Option<String>,
    source_repo: Option<String>,
) -> Result<McpSyncEvidence, McpSyncError> {
    sync_with_fetcher(
        &CurlGitHubRegistryIndexFetcher::for_source(source_repo, release),
        out,
    )
}

fn sync_with_fetcher(
    fetcher: &dyn RegistryIndexFetcher,
    out: &Path,
) -> Result<McpSyncEvidence, McpSyncError> {
    sync_to_file(fetcher, out, &unix_timestamp_string())
        .map(|state| McpSyncEvidence { state })
        .map_err(McpSyncError::from_sync)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;
    use crate::prepare_verified_cache;
    use std::fs;
    use std::path::PathBuf;
    use traverse_registry::{PublicRegistryCapabilityRecord, PublicRegistryIndex};
    use traverse_registry_sync::FetchedRegistryIndex;

    struct StaticFetcher(Result<FetchedRegistryIndex, RegistrySyncError>);

    impl RegistryIndexFetcher for StaticFetcher {
        fn fetch_latest_index(&self) -> Result<FetchedRegistryIndex, RegistrySyncError> {
            self.0.clone()
        }
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("traverse-mcp-sync-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    fn sha256(bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        use std::fmt::Write as _;
        Sha256::digest(bytes)
            .iter()
            .fold("sha256:".to_string(), |mut out, byte| {
                let _ = write!(out, "{byte:02x}");
                out
            })
    }

    /// The core-normalize-participants kit as a one-record index served
    /// from local files.
    fn kit_index() -> PublicRegistryIndex {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let kit = root.join("examples/core-normalize-participants");
        let wasm = kit.join("artifacts/core-normalize-participants.wasm");
        let contract = kit.join("contract.json");
        PublicRegistryIndex {
            index_version: 3,
            generated_at: "2026-10-29T00:00:00Z".to_string(),
            source_commit: None,
            capabilities: vec![PublicRegistryCapabilityRecord {
                namespace: "core".to_string(),
                id: "core.normalize-participants".to_string(),
                version: "1.1.0".to_string(),
                digest: sha256(&fs::read(&wasm).expect("kit wasm")),
                artifact_url: format!("file://{}", wasm.display()),
                contract_digest: sha256(&fs::read(&contract).expect("kit contract")),
                contract_url: format!("file://{}", contract.display()),
                deprecated: false,
                summary: "Normalize participants".to_string(),
                description: String::new(),
                use_cases: Vec::new(),
                service_type: "stateless".to_string(),
                permitted_targets: Vec::new(),
                lifecycle: "active".to_string(),
                provenance: None,
                commercial_use: "unknown".to_string(),
                redistribution: "unknown".to_string(),
                verification_status: "unknown".to_string(),
                license_expression: None,
                ai: None,
                status: None,
                model_usage: Vec::new(),
                revocation: None,
            }],
            events: Vec::new(),
        }
    }

    #[test]
    fn sync_writes_a_state_that_prepare_cache_accepts() {
        let dir = temp_dir("ok");
        let out = dir.join("synced-state.json");
        let fetcher = StaticFetcher(Ok(FetchedRegistryIndex {
            source_repo: "traverse-framework/registry".to_string(),
            release_tag: "index-v3".to_string(),
            index: kit_index(),
        }));
        let evidence = sync_with_fetcher(&fetcher, &out).expect("sync");
        let envelope = evidence.envelope();
        assert_eq!(envelope["kind"], json!("mcp_registry_sync"));
        assert_eq!(envelope["release_tag"], json!("index-v3"));
        assert_eq!(envelope["record_count"], json!(1));
        assert_eq!(
            evidence.render(),
            "synced traverse-framework/registry index-v3 (1 capability record(s))"
        );
        prepare_verified_cache(&out, &dir.join("cache"), &[]).expect("prepare-cache");
    }

    #[test]
    fn sync_failure_reports_a_stable_code_and_keeps_the_file() {
        let dir = temp_dir("fail");
        let out = dir.join("synced-state.json");
        fs::write(&out, b"previous").expect("existing");
        let fetcher = StaticFetcher(Err(RegistrySyncError::new(
            "registry_index_release_missing",
            "release index-v99 was not found".to_string(),
        )));
        let error = sync_with_fetcher(&fetcher, &out).expect_err("missing release");
        assert_eq!(error.code, "registry_index_release_missing");
        assert_eq!(error.envelope()["kind"], json!("mcp_registry_sync_error"));
        assert_eq!(
            error.to_string(),
            "registry_index_release_missing: release index-v99 was not found"
        );
        assert_eq!(fs::read(&out).expect("kept"), b"previous");
    }
}
