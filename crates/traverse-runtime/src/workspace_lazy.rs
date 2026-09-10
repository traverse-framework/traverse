//! Workspace load without parsing component contracts (spec `134`).

use crate::capability_metadata::{CapabilityMetadataIndex, IndexedCapability};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use traverse_contracts::CapabilityContract;
use traverse_registry::{
    ApplicationModelDependency, ArtifactDigests, BinaryFormat, BinaryReference,
    CapabilityArtifactRecord, CapabilityRegistryRecord, ComposabilityMetadata, CompositionKind,
    CompositionPattern, DiscoveryIndexEntry, ImplementationKind, LookupScope, RegistrationEvidence,
    RegistrationResult, RegistryProvenance, RegistryScope, ResolvedCapability, ResolvedWorkflow,
    SourceKind, SourceReference, WorkflowDefinition, WorkflowDiscoveryIndexEntry, WorkflowEdge,
    WorkflowEdgeTrigger, WorkflowRegistrationEvidence, WorkflowRegistrationResult,
    WorkflowRegistryRecord, WorkspaceAppStateError, WorkspaceAppStateErrorCode,
    WorkspaceAppStateFailure, WorkspaceApplicationRegistration,
};

const WORKSPACE_APP_STATE_SCHEMA_VERSION: &str = "1.0.0";
const WORKSPACE_APP_STATE_SCOPE: &str = "workspace_persisted";
const INDEXED_REGISTERED_AT: &str = "1970-01-01T00:00:00Z";

#[derive(Debug, Clone, Default)]
pub struct LazyWorkspaceLoad {
    pub applications: Vec<WorkspaceApplicationRegistration>,
    pub workflows: BTreeMap<(String, String), ResolvedWorkflow>,
}

/// Loads persisted app registrations and workflow definitions without parsing
/// component contracts.
///
/// # Errors
///
/// Returns [`WorkspaceAppStateFailure`] when workspace state is missing,
/// unreadable, incompatible, or a workflow fails index-backed topology checks.
pub fn load_lazy_workspace(
    workspace_root: &Path,
    workspace_id: &str,
    validator_version: &str,
) -> Result<LazyWorkspaceLoad, WorkspaceAppStateFailure> {
    let state_files = workspace_application_state_files(workspace_root, workspace_id)?;
    let mut parsed = Vec::new();
    for state_path in &state_files {
        parsed.push(read_application(workspace_root, workspace_id, state_path)?);
    }
    let applications = parsed
        .iter()
        .map(|item| item.registration.clone())
        .collect::<Vec<_>>();
    let index = CapabilityMetadataIndex::from_workspace_applications(&applications);
    let mut workflows = BTreeMap::new();
    for item in &parsed {
        for workflow in
            load_workflows_for_app(workspace_root, &item.workflows, &index, validator_version)?
        {
            workflows.insert(
                (
                    workflow.definition.id.clone(),
                    workflow.definition.version.clone(),
                ),
                workflow,
            );
        }
    }
    Ok(LazyWorkspaceLoad {
        applications,
        workflows,
    })
}

pub(crate) fn lookup_indexed_workflow<'a>(
    workflows: &'a BTreeMap<(String, String), ResolvedWorkflow>,
    lookup_scope: LookupScope,
    workflow_id: &str,
    workflow_version: &str,
) -> Option<&'a ResolvedWorkflow> {
    if lookup_scope == LookupScope::PublicOnly {
        return None;
    }
    workflows.get(&(workflow_id.to_string(), workflow_version.to_string()))
}

pub(crate) fn resolved_from_index(
    index: &CapabilityMetadataIndex,
    contract: CapabilityContract,
    capability_id: &str,
    capability_version: &str,
    validator_version: &str,
) -> ResolvedCapability {
    let indexed = index.get(capability_id, capability_version);
    let source = index.source(capability_id, capability_version);
    let (indexed, artifact_path, contract_path, manifest_path) =
        if let (Some(indexed), Some(source)) = (indexed, source) {
            (
                indexed.clone(),
                source.artifact.display().to_string(),
                source.contract.display().to_string(),
                source.manifest.display().to_string(),
            )
        } else {
            (
                IndexedCapability {
                    app_id: String::new(),
                    app_version: String::new(),
                    component_id: contract.id.clone(),
                    component_version: contract.version.clone(),
                    capability_id: contract.id.clone(),
                    capability_version: contract.version.clone(),
                    contract_digest: String::new(),
                    artifact_digest: None,
                    execution_mode: None,
                    platforms: Vec::new(),
                    workflow_refs: Vec::new(),
                },
                String::new(),
                String::new(),
                String::new(),
            )
        };
    let artifact_ref = indexed_artifact_ref(&indexed, artifact_path.is_empty());
    let provenance = RegistryProvenance {
        source: format!("workspace_app_state:{}", indexed.app_id),
        author: indexed.app_id.clone(),
        created_at: INDEXED_REGISTERED_AT.to_string(),
    };
    ResolvedCapability {
        record: CapabilityRegistryRecord {
            scope: RegistryScope::Private,
            id: contract.id.clone(),
            version: contract.version.clone(),
            lifecycle: contract.lifecycle.clone(),
            owner: contract.owner.clone(),
            contract_path,
            contract_digest: indexed.contract_digest.clone(),
            implementation_kind: ImplementationKind::Executable,
            artifact_ref: artifact_ref.clone(),
            registered_at: INDEXED_REGISTERED_AT.to_string(),
            provenance: provenance.clone(),
            evidence: RegistrationEvidence {
                evidence_id: format!("idx_{}_{}", contract.id, contract.version),
                artifact_ref: artifact_ref.clone(),
                capability_id: contract.id.clone(),
                capability_version: contract.version.clone(),
                scope: RegistryScope::Private,
                governing_spec: "134-lazy-capability-registry-reconstruction".to_string(),
                validator_version: validator_version.to_string(),
                produced_at: INDEXED_REGISTERED_AT.to_string(),
                result: RegistrationResult::Passed,
            },
        },
        artifact: CapabilityArtifactRecord {
            artifact_ref: artifact_ref.clone(),
            implementation_kind: ImplementationKind::Executable,
            source: SourceReference {
                kind: SourceKind::Local,
                location: manifest_path,
            },
            binary: wasm_binary(&artifact_path),
            workflow_ref: None,
            digests: ArtifactDigests {
                source_digest: indexed.contract_digest.clone(),
                binary_digest: indexed.artifact_digest.clone(),
            },
            provenance,
        },
        index_entry: indexed_discovery_entry(&contract, &artifact_ref),
        contract,
    }
}

fn indexed_artifact_ref(indexed: &crate::IndexedCapability, artifact_missing: bool) -> String {
    if artifact_missing {
        format!(
            "app:{}:{}:{}",
            indexed.app_id, indexed.app_version, indexed.capability_id
        )
    } else {
        format!(
            "app:{}:{}:component:{}:{}",
            indexed.app_id, indexed.app_version, indexed.component_id, indexed.component_version
        )
    }
}

fn wasm_binary(artifact_path: &str) -> Option<BinaryReference> {
    if artifact_path.is_empty() {
        None
    } else {
        Some(BinaryReference {
            format: BinaryFormat::Wasm,
            location: artifact_path.to_string(),
            signature: None,
        })
    }
}

fn indexed_discovery_entry(
    contract: &CapabilityContract,
    artifact_ref: &str,
) -> DiscoveryIndexEntry {
    DiscoveryIndexEntry {
        scope: RegistryScope::Private,
        id: contract.id.clone(),
        version: contract.version.clone(),
        lifecycle: contract.lifecycle.clone(),
        owner: contract.owner.clone(),
        summary: contract.summary.clone(),
        tags: Vec::new(),
        permissions: contract
            .permissions
            .iter()
            .map(|permission| permission.id.clone())
            .collect(),
        emits: contract
            .emits
            .iter()
            .map(|event| event.event_id.clone())
            .collect(),
        consumes: contract
            .consumes
            .iter()
            .map(|event| event.event_id.clone())
            .collect(),
        implementation_kind: ImplementationKind::Executable,
        composability: ComposabilityMetadata {
            kind: CompositionKind::Atomic,
            patterns: vec![CompositionPattern::Validation],
            provides: Vec::new(),
            requires: Vec::new(),
        },
        artifact_ref: artifact_ref.to_string(),
        registered_at: INDEXED_REGISTERED_AT.to_string(),
    }
}

#[derive(Debug, Deserialize)]
struct PersistedWorkspaceApplicationState {
    app_id: String,
    app_version: String,
    schema_version: String,
    #[serde(default)]
    manifest_path: String,
    #[serde(default)]
    manifest_digest: String,
    #[serde(default)]
    bundle_digest: String,
    workspace_id: String,
    state_scope: String,
    #[serde(default)]
    components: Vec<Value>,
    #[serde(default)]
    workflows: Vec<PersistedWorkspaceWorkflow>,
    #[serde(default)]
    model_dependencies: Vec<ApplicationModelDependency>,
    registration_fingerprint: Value,
}

#[derive(Debug, Deserialize, Clone)]
struct PersistedWorkspaceWorkflow {
    workflow_id: String,
    workflow_version: String,
    #[serde(default)]
    workflow_digest: String,
    path: String,
}

fn workspace_application_state_files(
    workspace_root: &Path,
    workspace_id: &str,
) -> Result<Vec<PathBuf>, WorkspaceAppStateFailure> {
    let apps_dir = workspace_root
        .join(".traverse")
        .join("workspaces")
        .join(workspace_id)
        .join("apps");
    if !apps_dir.exists() {
        return Err(single_error(
            WorkspaceAppStateErrorCode::MissingWorkspaceState,
            &apps_dir,
            format!("workspace {workspace_id} has no durable app registration state"),
        ));
    }
    let mut files = Vec::new();
    collect_registration_files(&apps_dir, &mut files)?;
    if files.is_empty() {
        return Err(single_error(
            WorkspaceAppStateErrorCode::MissingWorkspaceState,
            &apps_dir,
            format!("workspace {workspace_id} has no app registration files"),
        ));
    }
    files.sort();
    Ok(files)
}

fn collect_registration_files(
    root: &Path,
    files: &mut Vec<PathBuf>,
) -> Result<(), WorkspaceAppStateFailure> {
    let entries = fs::read_dir(root).map_err(|error| {
        single_error(
            WorkspaceAppStateErrorCode::StateReadFailed,
            root,
            format!("failed to read workspace app state directory: {error}"),
        )
    })?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_registration_files(&path, files)?;
        } else if path.file_name().and_then(|name| name.to_str()) == Some("registration.json") {
            files.push(path);
        }
    }
    Ok(())
}

struct ParsedApplication {
    registration: WorkspaceApplicationRegistration,
    workflows: Vec<PersistedWorkspaceWorkflow>,
}

fn read_application(
    workspace_root: &Path,
    workspace_id: &str,
    state_path: &Path,
) -> Result<ParsedApplication, WorkspaceAppStateFailure> {
    let bytes = fs::read(state_path).map_err(|error| {
        single_error(
            WorkspaceAppStateErrorCode::StateReadFailed,
            state_path,
            format!("failed to read workspace app registration state: {error}"),
        )
    })?;
    let state: PersistedWorkspaceApplicationState =
        serde_json::from_slice(&bytes).map_err(|error| {
            single_error(
                WorkspaceAppStateErrorCode::StateParseFailed,
                state_path,
                format!("failed to parse workspace app registration state: {error}"),
            )
        })?;
    if state.schema_version != WORKSPACE_APP_STATE_SCHEMA_VERSION {
        return Err(single_error(
            WorkspaceAppStateErrorCode::IncompatibleSchemaVersion,
            state_path,
            format!(
                "workspace app state schema {} is not supported",
                state.schema_version
            ),
        ));
    }
    if state.workspace_id != workspace_id || state.state_scope != WORKSPACE_APP_STATE_SCOPE {
        return Err(single_error(
            WorkspaceAppStateErrorCode::IncompatibleWorkspaceState,
            state_path,
            "workspace app state does not belong to the requested workspace".to_string(),
        ));
    }
    if state.components.is_empty() {
        return Err(single_error(
            WorkspaceAppStateErrorCode::CorruptWorkspaceState,
            state_path,
            "workspace app state must include at least one component".to_string(),
        ));
    }
    let fingerprint = &state.registration_fingerprint;
    let matches_identity = fingerprint
        .get("app_id")
        .and_then(Value::as_str)
        .is_some_and(|value| value == state.app_id)
        && fingerprint
            .get("app_version")
            .and_then(Value::as_str)
            .is_some_and(|value| value == state.app_version)
        && fingerprint
            .get("manifest_digest")
            .and_then(Value::as_str)
            .is_some_and(|value| value == state.manifest_digest);
    if !matches_identity {
        return Err(single_error(
            WorkspaceAppStateErrorCode::CorruptWorkspaceState,
            state_path,
            "workspace app registration fingerprint does not match state identity".to_string(),
        ));
    }
    let _ = workspace_root;
    Ok(ParsedApplication {
        registration: WorkspaceApplicationRegistration {
            app_id: state.app_id,
            app_version: state.app_version,
            manifest_path: state.manifest_path,
            manifest_digest: state.manifest_digest,
            bundle_digest: state.bundle_digest,
            model_dependencies: state.model_dependencies,
            state_path: state_path.to_path_buf(),
        },
        workflows: state.workflows,
    })
}

fn load_workflows_for_app(
    workspace_root: &Path,
    workflows: &[PersistedWorkspaceWorkflow],
    index: &CapabilityMetadataIndex,
    validator_version: &str,
) -> Result<Vec<ResolvedWorkflow>, WorkspaceAppStateFailure> {
    let mut loaded = Vec::new();
    for workflow in workflows {
        let path = resolve_workspace_state_path(workspace_root, &workflow.path);
        let contents = fs::read_to_string(&path).map_err(|error| {
            single_error(
                WorkspaceAppStateErrorCode::StateReadFailed,
                &path,
                format!("failed to read registered workflow: {error}"),
            )
        })?;
        let definition: WorkflowDefinition = serde_json::from_str(&contents).map_err(|error| {
            single_error(
                WorkspaceAppStateErrorCode::CorruptWorkspaceState,
                &path,
                format!("registered workflow is invalid JSON: {error}"),
            )
        })?;
        if definition.id != workflow.workflow_id || definition.version != workflow.workflow_version
        {
            return Err(single_error(
                WorkspaceAppStateErrorCode::CorruptWorkspaceState,
                &path,
                "registered workflow identity does not match workspace state".to_string(),
            ));
        }
        validate_workflow_against_index(&definition, index, &path)?;
        loaded.push(resolved_workflow(
            definition,
            path.display().to_string(),
            workflow.workflow_digest.clone(),
            validator_version,
        ));
    }
    Ok(loaded)
}

fn validate_workflow_against_index(
    definition: &WorkflowDefinition,
    index: &CapabilityMetadataIndex,
    path: &Path,
) -> Result<(), WorkspaceAppStateFailure> {
    let node_ids: BTreeSet<&str> = definition
        .nodes
        .iter()
        .map(|node| node.node_id.as_str())
        .collect();
    if !node_ids.contains(definition.start_node.as_str()) {
        return Err(single_error(
            WorkspaceAppStateErrorCode::WorkflowRegistrationFailed,
            path,
            "workflow start_node is not present in the definition".to_string(),
        ));
    }
    for terminal in &definition.terminal_nodes {
        if !node_ids.contains(terminal.as_str()) {
            return Err(single_error(
                WorkspaceAppStateErrorCode::WorkflowRegistrationFailed,
                path,
                format!("workflow terminal node {terminal} is not present in the definition"),
            ));
        }
    }
    for node in &definition.nodes {
        if index
            .get(&node.capability_id, &node.capability_version)
            .is_none()
        {
            return Err(single_error(
                WorkspaceAppStateErrorCode::WorkflowRegistrationFailed,
                path,
                format!(
                    "workflow node {} references capability {}@{} missing from the metadata index",
                    node.node_id, node.capability_id, node.capability_version
                ),
            ));
        }
    }
    for edge in &definition.edges {
        if !node_ids.contains(edge.from.as_str()) || !node_ids.contains(edge.to.as_str()) {
            return Err(single_error(
                WorkspaceAppStateErrorCode::WorkflowRegistrationFailed,
                path,
                format!("workflow edge {} references a missing node", edge.edge_id),
            ));
        }
    }
    if workflow_has_cycle(&definition.edges, &node_ids) {
        return Err(single_error(
            WorkspaceAppStateErrorCode::WorkflowRegistrationFailed,
            path,
            "workflow topology contains a cycle".to_string(),
        ));
    }
    Ok(())
}

fn workflow_has_cycle(edges: &[WorkflowEdge], node_ids: &BTreeSet<&str>) -> bool {
    let mut adjacency: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for id in node_ids {
        adjacency.insert(*id, Vec::new());
    }
    for edge in edges {
        if edge.trigger == WorkflowEdgeTrigger::Direct
            && let Some(targets) = adjacency.get_mut(edge.from.as_str())
        {
            targets.push(edge.to.as_str());
        }
    }
    let mut visiting = BTreeSet::new();
    let mut visited = BTreeSet::new();
    node_ids
        .iter()
        .any(|node| dfs_cycle(node, &adjacency, &mut visiting, &mut visited))
}

fn dfs_cycle<'a>(
    node: &'a str,
    adjacency: &BTreeMap<&str, Vec<&'a str>>,
    visiting: &mut BTreeSet<&'a str>,
    visited: &mut BTreeSet<&'a str>,
) -> bool {
    if visited.contains(node) {
        return false;
    }
    if !visiting.insert(node) {
        return true;
    }
    let cyclic = adjacency
        .get(node)
        .into_iter()
        .flatten()
        .any(|next| dfs_cycle(next, adjacency, visiting, visited));
    visiting.remove(node);
    visited.insert(node);
    cyclic
}

fn resolved_workflow(
    definition: WorkflowDefinition,
    workflow_path: String,
    workflow_digest: String,
    validator_version: &str,
) -> ResolvedWorkflow {
    let participating_capabilities = definition
        .nodes
        .iter()
        .map(|node| node.capability_id.clone())
        .collect::<Vec<_>>();
    let record = WorkflowRegistryRecord {
        scope: RegistryScope::Private,
        id: definition.id.clone(),
        version: definition.version.clone(),
        lifecycle: definition.lifecycle.clone(),
        owner: definition.owner.clone(),
        workflow_path,
        workflow_digest,
        registered_at: INDEXED_REGISTERED_AT.to_string(),
        governing_spec: definition.governing_spec.clone(),
        validator_version: validator_version.to_string(),
        evidence: WorkflowRegistrationEvidence {
            evidence_id: format!("wfidx_{}_{}", definition.id, definition.version),
            workflow_id: definition.id.clone(),
            workflow_version: definition.version.clone(),
            scope: RegistryScope::Private,
            governing_spec: definition.governing_spec.clone(),
            validator_version: validator_version.to_string(),
            produced_at: INDEXED_REGISTERED_AT.to_string(),
            result: WorkflowRegistrationResult::Passed,
        },
    };
    let index_entry = WorkflowDiscoveryIndexEntry {
        scope: RegistryScope::Private,
        id: definition.id.clone(),
        version: definition.version.clone(),
        lifecycle: definition.lifecycle.clone(),
        owner: definition.owner.clone(),
        summary: definition.summary.clone(),
        tags: definition.tags.clone(),
        participating_capabilities,
        events_used: Vec::new(),
        start_node: definition.start_node.clone(),
        terminal_nodes: definition.terminal_nodes.clone(),
        registered_at: INDEXED_REGISTERED_AT.to_string(),
    };
    ResolvedWorkflow {
        definition,
        record,
        index_entry,
    }
}

fn resolve_workspace_state_path(workspace_root: &Path, raw: &str) -> PathBuf {
    let path = PathBuf::from(raw);
    if path.is_absolute() {
        path
    } else {
        workspace_root.join(path)
    }
}

fn single_error(
    code: WorkspaceAppStateErrorCode,
    path: &Path,
    message: String,
) -> WorkspaceAppStateFailure {
    WorkspaceAppStateFailure {
        errors: vec![WorkspaceAppStateError {
            code,
            path: path.display().to_string(),
            message,
        }],
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use traverse_contracts::Owner;

    fn unique_dir() -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let dir = std::env::temp_dir().join(format!(
            "traverse-runtime-lazy-workspace-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    #[test]
    fn missing_workspace_state_is_reported() {
        let root = unique_dir();
        let failure = load_lazy_workspace(&root, "local", "test").expect_err("missing");
        assert_eq!(
            failure.errors[0].code,
            WorkspaceAppStateErrorCode::MissingWorkspaceState
        );
    }

    #[test]
    fn apps_path_that_is_a_file_is_a_read_failure() {
        let root = unique_dir();
        let apps = root.join(".traverse/workspaces/local/apps");
        fs::create_dir_all(apps.parent().expect("parent")).expect("parent");
        fs::write(&apps, b"not-a-dir").expect("file");
        let failure = load_lazy_workspace(&root, "local", "test").expect_err("file");
        assert_eq!(
            failure.errors[0].code,
            WorkspaceAppStateErrorCode::StateReadFailed
        );
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_registration_is_a_read_failure() {
        use std::os::unix::fs::PermissionsExt;
        let root = unique_dir();
        let contract = root.join("c.json");
        fs::write(&contract, real_contract()).expect("contract");
        let path = write_state(&root, &base_state(&contract, json!({})));
        let mut permissions = fs::metadata(&path).expect("meta").permissions();
        permissions.set_mode(0o000);
        fs::set_permissions(&path, permissions.clone()).expect("chmod");
        let failure = load_lazy_workspace(&root, "local", "test");
        permissions.set_mode(0o644);
        fs::set_permissions(&path, permissions).expect("restore");
        assert_eq!(
            failure.expect_err("unreadable").errors[0].code,
            WorkspaceAppStateErrorCode::StateReadFailed
        );
    }

    #[test]
    fn empty_apps_directory_is_missing_state() {
        let root = unique_dir();
        let apps = root.join(".traverse/workspaces/local/apps");
        fs::create_dir_all(&apps).expect("apps");
        let failure = load_lazy_workspace(&root, "local", "test").expect_err("empty");
        assert_eq!(
            failure.errors[0].code,
            WorkspaceAppStateErrorCode::MissingWorkspaceState
        );
    }

    #[test]
    fn lookup_indexed_workflow_hides_private_entries_from_public_scope() {
        let mut workflows = BTreeMap::new();
        workflows.insert(
            ("wf".to_string(), "1.0.0".to_string()),
            resolved_workflow(
                WorkflowDefinition {
                    kind: "workflow_definition".to_string(),
                    schema_version: "1.0.0".to_string(),
                    id: "wf".to_string(),
                    name: "wf".to_string(),
                    version: "1.0.0".to_string(),
                    lifecycle: traverse_contracts::Lifecycle::Active,
                    owner: Owner {
                        team: "t".to_string(),
                        contact: "t@example.com".to_string(),
                    },
                    summary: "s".to_string(),
                    inputs: traverse_contracts::SchemaContainer {
                        schema: serde_json::json!({"type": "object"}),
                    },
                    outputs: traverse_contracts::SchemaContainer {
                        schema: serde_json::json!({"type": "object"}),
                    },
                    nodes: Vec::new(),
                    edges: Vec::new(),
                    start_node: "start".to_string(),
                    terminal_nodes: Vec::new(),
                    output_projection: Vec::new(),
                    tags: Vec::new(),
                    governing_spec: "007-workflow-registry-traversal".to_string(),
                },
                "wf.json".to_string(),
                "sha256:x".to_string(),
                "test",
            ),
        );
        assert!(
            lookup_indexed_workflow(&workflows, LookupScope::PublicOnly, "wf", "1.0.0").is_none()
        );
        assert!(
            lookup_indexed_workflow(&workflows, LookupScope::PreferPrivate, "wf", "1.0.0")
                .is_some()
        );
    }

    fn write_state(root: &Path, body: &Value) -> PathBuf {
        let state_path =
            root.join(".traverse/workspaces/local/apps/demo.app/1.0.0/registration.json");
        fs::create_dir_all(state_path.parent().expect("parent")).expect("parent");
        fs::write(&state_path, serde_json::to_vec(body).expect("serialize")).expect("write");
        state_path
    }

    fn base_state(contract: &Path, extra: Value) -> Value {
        let mut body = json!({
            "app_id": "demo.app",
            "app_version": "1.0.0",
            "schema_version": "1.0.0",
            "manifest_path": "app.manifest.json",
            "manifest_digest": "sha256:manifest",
            "bundle_digest": "sha256:bundle",
            "workspace_id": "local",
            "state_scope": "workspace_persisted",
            "components": [{
                "component_id": "demo.component",
                "component_version": "1.0.0",
                "capability_id": "demo.capability",
                "capability_version": "1.0.0",
                "contract_path": contract.display().to_string(),
                "artifact_ref": contract.display().to_string(),
                "manifest_path": contract.display().to_string()
            }],
            "workflows": [],
            "registration_fingerprint": {
                "app_id": "demo.app",
                "app_version": "1.0.0",
                "manifest_digest": "sha256:manifest"
            }
        });
        if let Value::Object(extra) = extra
            && let Value::Object(root) = &mut body
        {
            root.extend(extra);
        }
        body
    }

    fn real_contract() -> String {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
            "../../contracts/examples/expedition/capabilities/validate-team-readiness/contract.json",
        );
        fs::read_to_string(path).expect("fixture contract")
    }

    fn workflow_json(
        id: &str,
        nodes: &Value,
        edges: &Value,
        start: &str,
        terminals: &Value,
    ) -> Value {
        json!({
            "kind": "workflow_definition",
            "schema_version": "1.0.0",
            "id": id,
            "name": id,
            "version": "1.0.0",
            "lifecycle": "active",
            "owner": {"team": "t", "contact": "t@example.com"},
            "summary": "s",
            "inputs": {"schema": {"type": "object"}},
            "outputs": {"schema": {"type": "object"}},
            "nodes": nodes,
            "edges": edges,
            "start_node": start,
            "terminal_nodes": terminals,
            "tags": [],
            "governing_spec": "007-workflow-registry-traversal"
        })
    }

    fn write_workflow_app(root: &Path, definition: &Value, capability_id: &str) {
        let contract = root.join("c.json");
        let mut contract_json: Value = serde_json::from_str(&real_contract()).expect("json");
        contract_json["id"] = json!(capability_id);
        fs::write(&contract, contract_json.to_string()).expect("contract");
        let wf = root.join("wf.json");
        fs::write(&wf, definition.to_string()).expect("workflow");
        let mut body = base_state(&contract, json!({}));
        body["components"][0]["capability_id"] = json!(capability_id);
        body["workflows"] = json!([{
            "workflow_id": definition["id"],
            "workflow_version": "1.0.0",
            "workflow_digest": "sha256:wf",
            "path": wf.display().to_string()
        }]);
        write_state(root, &body);
    }

    #[test]
    fn parse_and_schema_failures_are_stable() {
        let root = unique_dir();
        write_state(&root, &json!("nope"));
        let failure = load_lazy_workspace(&root, "local", "test").expect_err("parse");
        assert_eq!(
            failure.errors[0].code,
            WorkspaceAppStateErrorCode::StateParseFailed
        );

        let root = unique_dir();
        let contract = root.join("c.json");
        fs::write(&contract, real_contract()).expect("contract");
        let mut body = base_state(&contract, json!({}));
        body["schema_version"] = json!("9.0.0");
        write_state(&root, &body);
        let failure = load_lazy_workspace(&root, "local", "test").expect_err("schema");
        assert_eq!(
            failure.errors[0].code,
            WorkspaceAppStateErrorCode::IncompatibleSchemaVersion
        );

        let root = unique_dir();
        let contract = root.join("c.json");
        fs::write(&contract, real_contract()).expect("contract");
        let mut body = base_state(&contract, json!({}));
        body["workspace_id"] = json!("other");
        write_state(&root, &body);
        let failure = load_lazy_workspace(&root, "local", "test").expect_err("workspace");
        assert_eq!(
            failure.errors[0].code,
            WorkspaceAppStateErrorCode::IncompatibleWorkspaceState
        );

        let root = unique_dir();
        let contract = root.join("c.json");
        fs::write(&contract, real_contract()).expect("contract");
        let mut body = base_state(&contract, json!({}));
        body["state_scope"] = json!("other");
        write_state(&root, &body);
        let failure = load_lazy_workspace(&root, "local", "test").expect_err("scope");
        assert_eq!(
            failure.errors[0].code,
            WorkspaceAppStateErrorCode::IncompatibleWorkspaceState
        );
    }

    #[test]
    fn corrupt_fingerprint_and_empty_components_fail() {
        let root = unique_dir();
        let contract = root.join("c.json");
        fs::write(&contract, real_contract()).expect("contract");
        let mut body = base_state(&contract, json!({}));
        body["components"] = json!([]);
        write_state(&root, &body);
        let failure = load_lazy_workspace(&root, "local", "test").expect_err("empty");
        assert_eq!(
            failure.errors[0].code,
            WorkspaceAppStateErrorCode::CorruptWorkspaceState
        );

        let root = unique_dir();
        let contract = root.join("c.json");
        fs::write(&contract, real_contract()).expect("contract");
        let mut body = base_state(&contract, json!({}));
        body["registration_fingerprint"]["app_id"] = json!("other");
        write_state(&root, &body);
        let failure = load_lazy_workspace(&root, "local", "test").expect_err("fingerprint");
        assert_eq!(
            failure.errors[0].code,
            WorkspaceAppStateErrorCode::CorruptWorkspaceState
        );
    }

    #[test]
    fn workflow_topology_and_identity_failures() {
        let root = unique_dir();
        let missing_wf = root.join("missing-wf.json");
        let contract = root.join("c.json");
        fs::write(&contract, real_contract()).expect("contract");
        let mut body = base_state(&contract, json!({}));
        body["workflows"] = json!([{
            "workflow_id": "wf",
            "workflow_version": "1.0.0",
            "path": missing_wf.display().to_string()
        }]);
        write_state(&root, &body);
        let failure = load_lazy_workspace(&root, "local", "test").expect_err("missing wf");
        assert_eq!(
            failure.errors[0].code,
            WorkspaceAppStateErrorCode::StateReadFailed
        );

        let root = unique_dir();
        let wf = root.join("wf.json");
        fs::write(&wf, "{").expect("bad json");
        let contract = root.join("c.json");
        fs::write(&contract, real_contract()).expect("contract");
        let mut body = base_state(&contract, json!({}));
        body["workflows"] = json!([{
            "workflow_id": "wf",
            "workflow_version": "1.0.0",
            "path": wf.display().to_string()
        }]);
        write_state(&root, &body);
        let failure = load_lazy_workspace(&root, "local", "test").expect_err("bad json");
        assert_eq!(
            failure.errors[0].code,
            WorkspaceAppStateErrorCode::CorruptWorkspaceState
        );

        let root = unique_dir();
        let contract = root.join("c.json");
        let mut contract_json: Value = serde_json::from_str(&real_contract()).expect("json");
        contract_json["id"] = json!("demo.capability");
        fs::write(&contract, contract_json.to_string()).expect("contract");
        let wf = root.join("wf.json");
        fs::write(
            &wf,
            workflow_json(
                "other.wf",
                &json!([{
                    "node_id": "n1",
                    "capability_id": "demo.capability",
                    "capability_version": "1.0.0",
                    "input": {"from_workflow_input": []},
                    "output": {"to_workflow_state": []}
                }]),
                &json!([]),
                "n1",
                &json!(["n1"]),
            )
            .to_string(),
        )
        .expect("workflow");
        let mut body = base_state(&contract, json!({}));
        body["workflows"] = json!([{
            "workflow_id": "wf",
            "workflow_version": "1.0.0",
            "path": wf.display().to_string()
        }]);
        write_state(&root, &body);
        let failure = load_lazy_workspace(&root, "local", "test").expect_err("identity");
        assert_eq!(
            failure.errors[0].code,
            WorkspaceAppStateErrorCode::CorruptWorkspaceState
        );
    }

    fn expect_workflow_registration_failure(definition: &Value) {
        let root = unique_dir();
        write_workflow_app(root.as_path(), definition, "demo.capability");
        assert_eq!(
            load_lazy_workspace(&root, "local", "test")
                .expect_err("workflow")
                .errors[0]
                .code,
            WorkspaceAppStateErrorCode::WorkflowRegistrationFailed
        );
    }

    #[test]
    fn workflow_index_validation_covers_topology() {
        let node = |id: &str, cap: &str| {
            json!({
                "node_id": id,
                "capability_id": cap,
                "capability_version": "1.0.0",
                "input": {"from_workflow_input": []},
                "output": {"to_workflow_state": []}
            })
        };
        let edge = |id: &str, from: &str, to: &str| {
            json!({
                "edge_id": id,
                "from": from,
                "to": to,
                "trigger": "direct"
            })
        };
        expect_workflow_registration_failure(&workflow_json(
            "wf",
            &json!([node("n1", "demo.capability")]),
            &json!([]),
            "missing",
            &json!(["n1"]),
        ));
        expect_workflow_registration_failure(&workflow_json(
            "wf",
            &json!([node("n1", "demo.capability")]),
            &json!([]),
            "n1",
            &json!(["missing"]),
        ));
        expect_workflow_registration_failure(&workflow_json(
            "wf",
            &json!([node("n1", "missing.cap")]),
            &json!([]),
            "n1",
            &json!(["n1"]),
        ));
        expect_workflow_registration_failure(&workflow_json(
            "wf",
            &json!([node("n1", "demo.capability")]),
            &json!([edge("e1", "n1", "ghost")]),
            "n1",
            &json!(["n1"]),
        ));
        expect_workflow_registration_failure(&workflow_json(
            "wf",
            &json!([node("n1", "demo.capability"), node("n2", "demo.capability")]),
            &json!([edge("e1", "n1", "n2"), edge("e2", "n2", "n1")]),
            "n1",
            &json!(["n2"]),
        ));
    }

    #[test]
    fn successful_workflow_load_and_relative_path() {
        let root = unique_dir();
        let contract = root.join("c.json");
        let mut contract_json: Value = serde_json::from_str(&real_contract()).expect("json");
        contract_json["id"] = json!("demo.capability");
        fs::write(&contract, contract_json.to_string()).expect("contract");
        fs::write(
            root.join("wf.json"),
            workflow_json(
                "wf",
                &json!([
                    {
                        "node_id": "n1",
                        "capability_id": "demo.capability",
                        "capability_version": "1.0.0",
                        "input": {"from_workflow_input": []},
                        "output": {"to_workflow_state": []}
                    },
                    {
                        "node_id": "n2",
                        "capability_id": "demo.capability",
                        "capability_version": "1.0.0",
                        "input": {"from_workflow_input": []},
                        "output": {"to_workflow_state": []}
                    }
                ]),
                &json!([
                    {
                        "edge_id": "e1",
                        "from": "n1",
                        "to": "n2",
                        "trigger": "direct"
                    },
                    {
                        "edge_id": "e2",
                        "from": "n1",
                        "to": "n2",
                        "trigger": "event",
                        "event": {"event_id": "demo.event", "version": "1.0.0"}
                    }
                ]),
                "n1",
                &json!(["n2"]),
            )
            .to_string(),
        )
        .expect("wf");
        let mut body = base_state(&contract, json!({}));
        body["workflows"] = json!([{
            "workflow_id": "wf",
            "workflow_version": "1.0.0",
            "workflow_digest": "sha256:wf",
            "path": "wf.json"
        }]);
        write_state(&root, &body);
        let loaded = load_lazy_workspace(&root, "local", "test").expect("load");
        assert_eq!(loaded.workflows.len(), 1);
        assert!(
            lookup_indexed_workflow(&loaded.workflows, LookupScope::PreferPrivate, "wf", "1.0.0")
                .is_some()
        );
        let missing = resolved_from_index(
            &CapabilityMetadataIndex::from_workspace_applications(&loaded.applications),
            traverse_contracts::parse_contract(&real_contract()).expect("contract"),
            "missing",
            "1.0.0",
            "test",
        );
        assert!(missing.artifact.binary.is_none());
        assert!(missing.record.contract_path.is_empty());
    }

    #[test]
    fn empty_artifact_resolves_without_binary() {
        let root = unique_dir();
        let contract = root.join("c.json");
        let mut contract_json: Value = serde_json::from_str(&real_contract()).expect("json");
        contract_json["id"] = json!("demo.capability");
        fs::write(&contract, contract_json.to_string()).expect("contract");
        let mut body = base_state(&contract, json!({}));
        body["components"][0]["artifact_ref"] = json!("");
        write_state(&root, &body);
        let loaded = load_lazy_workspace(&root, "local", "test").expect("load");
        let index = CapabilityMetadataIndex::from_workspace_applications(&loaded.applications);
        let resolved = resolved_from_index(
            &index,
            traverse_contracts::parse_contract(&contract_json.to_string()).expect("parse"),
            "demo.capability",
            "1.0.0",
            "test",
        );
        assert!(resolved.artifact.binary.is_none());
    }
}
