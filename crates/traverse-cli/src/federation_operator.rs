use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};
use traverse_registry::{
    FederationFailure, FederationPeer, FederationRegistry, FederationStatusSummary,
    FederationSyncOutcome, FederationSyncStatus, FederationTrustState, RegistryScope, TrustRecord,
    export_peer_state,
};

#[derive(Debug, Clone, Deserialize)]
pub struct FederationOperatorManifest {
    pub peer: FederationPeerManifest,
    pub trust: TrustRecordManifest,
    pub bundle_manifest_path: PathBuf,
    pub started_at: String,
    pub finished_at: String,
    pub evidence_ref: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FederationPeerManifest {
    pub peer_id: String,
    pub display_name: String,
    pub trust_state: FederationTrustStateManifest,
    pub identity_fingerprint: String,
    pub sync_enabled: bool,
    pub last_sync_at: Option<String>,
    pub last_sync_status: FederationSyncStatusManifest,
    pub visible_registry_scopes: Vec<RegistryScopeManifest>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TrustRecordManifest {
    pub peer_id: String,
    pub trust_model: String,
    pub allowed_scopes: Vec<RegistryScopeManifest>,
    pub approved_spec_refs: Vec<String>,
    pub approved_at: String,
    pub revoked_at: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum FederationTrustStateManifest {
    Trusted,
    Pending,
    Blocked,
    Revoked,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum FederationSyncStatusManifest {
    Unknown,
    Success,
    Partial,
    Failed,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum RegistryScopeManifest {
    Public,
    Private,
}

#[derive(Debug)]
struct LoadedFederationContext {
    manifest: FederationOperatorManifest,
    peer: FederationPeer,
    trust: TrustRecord,
    federation: FederationRegistry,
    registered_bundle: super::RegisteredBundle,
}

pub fn render_federation_peers(manifest_path: &Path) -> Result<String, String> {
    let context = load_context(manifest_path)?;
    Ok(render_peer_listing(
        &context.federation.status_summary(),
        &context.federation,
    ))
}

pub fn render_federation_sync(manifest_path: &Path) -> Result<String, String> {
    let mut context = load_context(manifest_path)?;
    let outcome = sync_context(&mut context)?;
    Ok(render_sync_report(
        &context.federation.status_summary(),
        &context.federation,
        &outcome,
    ))
}

pub fn render_federation_status(manifest_path: &Path) -> Result<String, String> {
    let mut context = load_context(manifest_path)?;
    let outcome = sync_context(&mut context)?;
    Ok(format!(
        "{}\n{}",
        render_sync_report(
            &context.federation.status_summary(),
            &context.federation,
            &outcome
        ),
        render_peer_listing(&context.federation.status_summary(), &context.federation)
    ))
}

fn load_context(manifest_path: &Path) -> Result<LoadedFederationContext, String> {
    let manifest = load_manifest(manifest_path)?;
    let bundle_manifest_path = resolve_relative_path(manifest_path, &manifest.bundle_manifest_path);
    let registered_bundle =
        super::load_governed_public_bundle(&bundle_manifest_path).map_err(|e| e.to_string())?;
    let peer = manifest.peer.clone().into_peer();
    let trust = manifest.trust.clone().into_trust();
    let mut federation = FederationRegistry::new();
    federation
        .register_peer(peer.clone(), trust.clone())
        .map_err(render_federation_failure)?;

    Ok(LoadedFederationContext {
        manifest,
        peer,
        trust,
        federation,
        registered_bundle,
    })
}

fn sync_context(context: &mut LoadedFederationContext) -> Result<FederationSyncOutcome, String> {
    let export = export_peer_state(
        context.peer.clone(),
        context.trust.clone(),
        &context.registered_bundle.capability_registry,
        &context.registered_bundle.event_registry,
        &context.registered_bundle.workflow_registry,
    );
    context
        .federation
        .sync_peer(
            export,
            &context.registered_bundle.capability_registry,
            &context.registered_bundle.event_registry,
            &context.registered_bundle.workflow_registry,
            &context.manifest.started_at,
            &context.manifest.finished_at,
            &context.manifest.evidence_ref,
        )
        .map_err(render_federation_failure)
}

fn render_peer_listing(
    summary: &FederationStatusSummary,
    federation: &FederationRegistry,
) -> String {
    let mut lines = vec![
        format!("peer_count: {}", summary.peer_count),
        format!("trusted_peer_count: {}", summary.trusted_peer_count),
        format!("last_sync_outcome: {:?}", summary.last_sync_outcome).to_lowercase(),
        format!(
            "sync_age: {}",
            summary
                .sync_age
                .clone()
                .unwrap_or_else(|| "none".to_string())
        ),
        format!("conflict_count: {}", summary.conflict_count),
        format!("blocked_entries: {}", summary.blocked_entries),
        format!("route_failures: {}", summary.route_failures),
    ];

    for peer in federation.list_peers() {
        lines.push(format!("peer_id: {}", peer.peer_id));
        lines.push(format!("display_name: {}", peer.display_name));
        lines.push(format!("trust_state: {:?}", peer.trust_state).to_lowercase());
        lines.push(format!("sync_enabled: {}", peer.sync_enabled));
        lines.push(format!(
            "last_sync_at: {}",
            peer.last_sync_at
                .clone()
                .unwrap_or_else(|| "none".to_string())
        ));
        lines.push(format!("last_sync_status: {:?}", peer.last_sync_status).to_lowercase());
        lines.push(format!(
            "visible_registry_scopes: {}",
            peer.visible_registry_scopes
                .iter()
                .map(|scope| format!("{scope:?}").to_lowercase())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    lines.join("\n")
}

fn render_sync_report(
    summary: &FederationStatusSummary,
    federation: &FederationRegistry,
    outcome: &FederationSyncOutcome,
) -> String {
    let session = &outcome.session;
    let mut lines = vec![
        format!("session_id: {}", session.session_id),
        format!("peer_id: {}", session.peer_id),
        format!("sync_status: {:?}", session.status).to_lowercase(),
        format!(
            "registry_types: {}",
            session
                .registry_types
                .iter()
                .map(|kind| format!("{kind:?}").to_lowercase())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        format!("validated_entries: {}", session.validated_entries),
        format!("rejected_entries: {}", session.rejected_entries),
        format!("conflict_count: {}", session.conflict_count),
        format!("evidence_ref: {}", session.evidence_ref),
        format!(
            "finished_at: {}",
            session
                .finished_at
                .clone()
                .unwrap_or_else(|| "none".to_string())
        ),
        format!("peer_count: {}", summary.peer_count),
        format!("trusted_peer_count: {}", summary.trusted_peer_count),
    ];

    for conflict in &outcome.conflicts {
        lines.push(format!(
            "conflict: {} {} {} {} {:?}",
            conflict.conflict_id,
            format!("{:?}", conflict.registry_type).to_lowercase(),
            conflict.entry_key,
            conflict.conflict_reason,
            conflict.resolution_state
        ));
    }

    lines.push(format!(
        "registered_peers: {}",
        federation
            .list_peers()
            .iter()
            .map(|peer| peer.peer_id.clone())
            .collect::<Vec<_>>()
            .join(", ")
    ));

    lines.join("\n")
}

fn load_manifest(manifest_path: &Path) -> Result<FederationOperatorManifest, String> {
    let contents = fs::read_to_string(manifest_path).map_err(|error| {
        format!(
            "failed to read federation operator manifest {}: {error}",
            manifest_path.display()
        )
    })?;
    serde_json::from_str::<FederationOperatorManifest>(&contents).map_err(|error| {
        format!(
            "failed to parse federation operator manifest {}: {error}",
            manifest_path.display()
        )
    })
}

fn resolve_relative_path(base_path: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(path)
    }
}

fn render_federation_failure(failure: FederationFailure) -> String {
    failure
        .errors
        .into_iter()
        .map(|error| format!("{:?} {}: {}", error.severity, error.target, error.message))
        .collect::<Vec<_>>()
        .join("\n")
}

impl FederationPeerManifest {
    fn into_peer(self) -> FederationPeer {
        FederationPeer {
            peer_id: self.peer_id,
            display_name: self.display_name,
            trust_state: self.trust_state.into(),
            identity_fingerprint: self.identity_fingerprint,
            sync_enabled: self.sync_enabled,
            last_sync_at: self.last_sync_at,
            last_sync_status: self.last_sync_status.into(),
            visible_registry_scopes: self
                .visible_registry_scopes
                .into_iter()
                .map(Into::into)
                .collect(),
        }
    }
}

impl TrustRecordManifest {
    fn into_trust(self) -> TrustRecord {
        TrustRecord {
            peer_id: self.peer_id,
            trust_model: self.trust_model,
            allowed_scopes: self.allowed_scopes.into_iter().map(Into::into).collect(),
            approved_spec_refs: self.approved_spec_refs,
            approved_at: self.approved_at,
            revoked_at: self.revoked_at,
        }
    }
}

impl From<RegistryScopeManifest> for RegistryScope {
    fn from(value: RegistryScopeManifest) -> Self {
        match value {
            RegistryScopeManifest::Public => RegistryScope::Public,
            RegistryScopeManifest::Private => RegistryScope::Private,
        }
    }
}

impl From<FederationTrustStateManifest> for FederationTrustState {
    fn from(value: FederationTrustStateManifest) -> Self {
        match value {
            FederationTrustStateManifest::Trusted => FederationTrustState::Trusted,
            FederationTrustStateManifest::Pending => FederationTrustState::Pending,
            FederationTrustStateManifest::Blocked => FederationTrustState::Blocked,
            FederationTrustStateManifest::Revoked => FederationTrustState::Revoked,
        }
    }
}

impl From<FederationSyncStatusManifest> for FederationSyncStatus {
    fn from(value: FederationSyncStatusManifest) -> Self {
        match value {
            FederationSyncStatusManifest::Unknown => FederationSyncStatus::Unknown,
            FederationSyncStatusManifest::Success => FederationSyncStatus::Success,
            FederationSyncStatusManifest::Partial => FederationSyncStatus::Partial,
            FederationSyncStatusManifest::Failed => FederationSyncStatus::Failed,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::{
        FederationPeerManifest, FederationSyncStatusManifest, FederationTrustStateManifest,
        RegistryScopeManifest, TrustRecordManifest, load_manifest, render_federation_failure,
        render_federation_peers, render_federation_status, render_federation_sync,
        resolve_relative_path,
    };
    use serde_json::Value;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};
    use traverse_contracts::ErrorSeverity;
    use traverse_registry::{
        FederationError, FederationErrorCode, FederationFailure, FederationSyncStatus,
        FederationTrustState, RegistryScope,
    };

    #[test]
    fn federation_peers_and_status_renders_peer_listing_and_sync_summary() {
        let temp_dir = unique_temp_dir();
        let manifest_path = temp_dir.join("federation-operator.json");
        let bundle_manifest_path = public_bundle_manifest_fixture(&temp_dir);

        fs::write(
            &manifest_path,
            format!(
                r#"{{
  "peer": {{
    "peer_id": "peer-a",
    "display_name": "Peer A",
    "trust_state": "Trusted",
    "identity_fingerprint": "fingerprint:peer-a",
    "sync_enabled": true,
    "last_sync_at": null,
    "last_sync_status": "Unknown",
    "visible_registry_scopes": ["Public"]
  }},
  "trust": {{
    "peer_id": "peer-a",
    "trust_model": "allowlist",
    "allowed_scopes": ["Public"],
    "approved_spec_refs": ["026-federation-registry-routing"],
    "approved_at": "2026-04-10T00:00:00Z",
    "revoked_at": null
  }},
  "bundle_manifest_path": "{}",
  "started_at": "2026-04-10T00:00:01Z",
  "finished_at": "2026-04-10T00:00:02Z",
  "evidence_ref": "evidence:federation-sync:peer-a"
}}"#,
                bundle_manifest_path.display()
            ),
        )
        .expect("manifest should write");

        let peers = render_federation_peers(&manifest_path).expect("peers should render");
        assert!(peers.contains("peer_count: 1"));
        assert!(peers.contains("peer_id: peer-a"));
        assert!(peers.contains("last_sync_status: unknown"));

        let sync = render_federation_sync(&manifest_path).expect("sync should render");
        assert!(sync.contains("session_id: sync_peer-a_1"));
        assert!(sync.contains("sync_status: success"));
        assert!(sync.contains("evidence_ref: evidence:federation-sync:peer-a"));

        let status = render_federation_status(&manifest_path).expect("status should render");
        assert!(status.contains("peer_count: 1"));
        assert!(status.contains("last_sync_status: success"));
        assert!(status.contains("registered_peers: peer-a"));
    }

    fn canonical_bundle_manifest_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/expedition/registry-bundle/manifest.json")
    }

    fn public_bundle_manifest_fixture(temp_dir: &Path) -> PathBuf {
        let source = canonical_bundle_manifest_path();
        let source_parent = source.parent().expect("bundle manifest must have parent");
        let mut manifest: Value = serde_json::from_str(
            &fs::read_to_string(&source).expect("canonical bundle manifest should read"),
        )
        .expect("canonical bundle manifest should parse");
        manifest["scope"] = Value::String("public".to_string());
        for collection in ["capabilities", "events", "workflows"] {
            for artifact in manifest[collection]
                .as_array_mut()
                .expect("artifact collection should be an array")
            {
                let relative = artifact["path"]
                    .as_str()
                    .expect("artifact path should be a string");
                artifact["path"] =
                    Value::String(source_parent.join(relative).display().to_string());
            }
        }
        let fixture = temp_dir.join("public-registry-bundle.json");
        fs::write(
            &fixture,
            serde_json::to_string_pretty(&manifest).expect("manifest should serialize"),
        )
        .expect("public bundle fixture should write");
        fixture
    }

    fn unique_temp_dir() -> PathBuf {
        let mut path = std::env::temp_dir();
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic")
            .as_nanos();
        path.push(format!("cogolo-federation-operator-{nonce}"));
        fs::create_dir_all(&path).expect("temp dir should create");
        path
    }

    fn peer_manifest(
        trust_state: FederationTrustStateManifest,
        last_sync_status: FederationSyncStatusManifest,
    ) -> FederationPeerManifest {
        FederationPeerManifest {
            peer_id: "peer-a".to_string(),
            display_name: "Peer A".to_string(),
            trust_state,
            identity_fingerprint: "fingerprint:peer-a".to_string(),
            sync_enabled: true,
            last_sync_at: None,
            last_sync_status,
            visible_registry_scopes: vec![RegistryScopeManifest::Public],
        }
    }

    fn trust_manifest() -> TrustRecordManifest {
        TrustRecordManifest {
            peer_id: "peer-a".to_string(),
            trust_model: "allowlist".to_string(),
            allowed_scopes: vec![
                RegistryScopeManifest::Public,
                RegistryScopeManifest::Private,
            ],
            approved_spec_refs: vec!["026-federation-registry-routing".to_string()],
            approved_at: "2026-04-10T00:00:00Z".to_string(),
            revoked_at: None,
        }
    }

    #[test]
    fn trust_state_manifest_maps_to_every_federation_trust_state() {
        let cases = [
            (
                FederationTrustStateManifest::Trusted,
                FederationTrustState::Trusted,
            ),
            (
                FederationTrustStateManifest::Pending,
                FederationTrustState::Pending,
            ),
            (
                FederationTrustStateManifest::Blocked,
                FederationTrustState::Blocked,
            ),
            (
                FederationTrustStateManifest::Revoked,
                FederationTrustState::Revoked,
            ),
        ];
        for (manifest_state, expected) in cases {
            let peer =
                peer_manifest(manifest_state, FederationSyncStatusManifest::Unknown).into_peer();
            assert_eq!(peer.trust_state, expected);
        }
    }

    #[test]
    fn sync_status_manifest_maps_to_every_federation_sync_status() {
        let cases = [
            (
                FederationSyncStatusManifest::Unknown,
                FederationSyncStatus::Unknown,
            ),
            (
                FederationSyncStatusManifest::Success,
                FederationSyncStatus::Success,
            ),
            (
                FederationSyncStatusManifest::Partial,
                FederationSyncStatus::Partial,
            ),
            (
                FederationSyncStatusManifest::Failed,
                FederationSyncStatus::Failed,
            ),
        ];
        for (manifest_status, expected) in cases {
            let peer =
                peer_manifest(FederationTrustStateManifest::Trusted, manifest_status).into_peer();
            assert_eq!(peer.last_sync_status, expected);
        }
    }

    #[test]
    fn registry_scope_manifest_maps_to_every_registry_scope() {
        assert_eq!(
            RegistryScope::from(RegistryScopeManifest::Public),
            RegistryScope::Public
        );
        assert_eq!(
            RegistryScope::from(RegistryScopeManifest::Private),
            RegistryScope::Private
        );

        let trust = trust_manifest().into_trust();
        assert_eq!(
            trust.allowed_scopes,
            vec![RegistryScope::Public, RegistryScope::Private]
        );
    }

    #[test]
    fn load_manifest_reports_missing_file_and_invalid_json() {
        let missing = load_manifest(Path::new("/definitely/missing/federation-operator.json"))
            .expect_err("missing manifest must fail");
        assert!(missing.contains("failed to read"));

        let temp_dir = unique_temp_dir();
        let manifest_path = temp_dir.join("federation-operator.json");
        fs::write(&manifest_path, b"not json").expect("temp file should write");
        let invalid = load_manifest(&manifest_path).expect_err("invalid JSON manifest must fail");
        assert!(invalid.contains("failed to parse"));
    }

    #[test]
    fn resolve_relative_path_joins_against_manifest_parent_directory() {
        let base = Path::new("/some/dir/federation-operator.json");
        let resolved = resolve_relative_path(base, Path::new("bundle/manifest.json"));
        assert_eq!(resolved, Path::new("/some/dir/bundle/manifest.json"));

        let absolute = Path::new("/already/absolute/manifest.json");
        let resolved_absolute = resolve_relative_path(base, absolute);
        assert_eq!(resolved_absolute, absolute);
    }

    #[test]
    fn render_federation_failure_joins_every_error_line() {
        let failure = FederationFailure {
            errors: vec![
                FederationError {
                    code: FederationErrorCode::MissingRequiredField,
                    target: "peer.peer_id".to_string(),
                    message: "peer_id is required".to_string(),
                    severity: ErrorSeverity::Error,
                },
                FederationError {
                    code: FederationErrorCode::InvalidTrust,
                    target: "trust.trust_model".to_string(),
                    message: "trust_model is unsupported".to_string(),
                    severity: ErrorSeverity::Error,
                },
            ],
        };
        let rendered = render_federation_failure(failure);
        assert!(rendered.contains("peer.peer_id"));
        assert!(rendered.contains("trust.trust_model"));
        assert_eq!(rendered.lines().count(), 2);
    }
}
