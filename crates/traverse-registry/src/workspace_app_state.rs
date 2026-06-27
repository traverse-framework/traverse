use crate::{
    ApplicationModelDependency, ArtifactDigests, BinaryFormat, BinaryReference,
    CapabilityArtifactRecord, CapabilityRegistration, CapabilityRegistry, ComposabilityMetadata,
    CompositionKind, CompositionPattern, ImplementationKind, RegistryProvenance, RegistryScope,
    SourceKind, SourceReference, WorkflowDefinition, WorkflowRegistration, WorkflowRegistry,
};
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use traverse_contracts::{governed_content_digest, parse_contract};

const WORKSPACE_APP_STATE_SCHEMA_VERSION: &str = "1.0.0";
const WORKSPACE_APP_STATE_SCOPE: &str = "workspace_persisted";
const WORKSPACE_APP_STATE_GOVERNING_SPEC: &str = "046-public-cli-app-registration";

#[derive(Debug, Clone)]
pub struct WorkspaceApplicationRegistries {
    pub workspace_id: String,
    pub applications: Vec<WorkspaceApplicationRegistration>,
    pub capability_registry: CapabilityRegistry,
    pub workflow_registry: WorkflowRegistry,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceApplicationRegistration {
    pub app_id: String,
    pub app_version: String,
    pub manifest_path: String,
    pub manifest_digest: String,
    pub bundle_digest: String,
    pub model_dependencies: Vec<ApplicationModelDependency>,
    pub state_path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceAppStateErrorCode {
    MissingWorkspaceState,
    StateReadFailed,
    StateParseFailed,
    IncompatibleSchemaVersion,
    IncompatibleWorkspaceState,
    CorruptWorkspaceState,
    CapabilityRegistrationFailed,
    WorkflowRegistrationFailed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceAppStateError {
    pub code: WorkspaceAppStateErrorCode,
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceAppStateFailure {
    pub errors: Vec<WorkspaceAppStateError>,
}

#[derive(Debug, Clone, Deserialize)]
struct PersistedWorkspaceApplicationState {
    app_id: String,
    app_version: String,
    schema_version: String,
    manifest_path: String,
    manifest_digest: String,
    bundle_digest: String,
    workspace_id: String,
    state_scope: String,
    components: Vec<PersistedWorkspaceComponent>,
    workflows: Vec<PersistedWorkspaceWorkflow>,
    #[serde(default)]
    model_dependencies: Vec<ApplicationModelDependency>,
    registration_fingerprint: Value,
}

#[derive(Debug, Clone, Deserialize)]
struct PersistedWorkspaceComponent {
    component_id: String,
    component_version: String,
    capability_id: String,
    capability_version: String,
    wasm_digest: String,
    manifest_path: String,
    contract_path: String,
    artifact_ref: String,
}

#[derive(Debug, Clone, Deserialize)]
struct PersistedWorkspaceWorkflow {
    workflow_id: String,
    workflow_version: String,
    workflow_digest: String,
    path: String,
}

/// Loads application registration state written by `traverse-cli app register`.
///
/// # Errors
///
/// Returns [`WorkspaceAppStateFailure`] when no durable app state exists for
/// the workspace, a registration file cannot be read or parsed, the state is
/// incompatible, or reconstructed registries fail validation.
pub fn load_workspace_application_registries(
    workspace_root: &Path,
    workspace_id: &str,
    validator_version: &str,
) -> Result<WorkspaceApplicationRegistries, WorkspaceAppStateFailure> {
    let state_files = workspace_application_state_files(workspace_root, workspace_id)?;
    let mut capability_registry = CapabilityRegistry::new();
    let mut workflow_registry = WorkflowRegistry::new();
    let mut applications = Vec::new();

    for state_path in state_files {
        let state = read_workspace_application_state(workspace_root, workspace_id, &state_path)?;
        for component in &state.components {
            capability_registry
                .register(build_workspace_capability_registration(
                    workspace_root,
                    validator_version,
                    &state,
                    component,
                )?)
                .map_err(|failure| {
                    single_error(
                        WorkspaceAppStateErrorCode::CapabilityRegistrationFailed,
                        &state_path,
                        format!(
                            "failed to register capability from workspace app state: {}",
                            failure
                                .errors
                                .iter()
                                .map(|error| error.message.clone())
                                .collect::<Vec<_>>()
                                .join("; ")
                        ),
                    )
                })?;
        }
        for workflow in &state.workflows {
            workflow_registry
                .register(
                    &capability_registry,
                    build_workspace_workflow_registration(
                        workspace_root,
                        validator_version,
                        &state,
                        workflow,
                    )?,
                )
                .map_err(|failure| {
                    single_error(
                        WorkspaceAppStateErrorCode::WorkflowRegistrationFailed,
                        &state_path,
                        format!(
                            "failed to register workflow from workspace app state: {}",
                            failure
                                .errors
                                .iter()
                                .map(|error| error.message.clone())
                                .collect::<Vec<_>>()
                                .join("; ")
                        ),
                    )
                })?;
        }
        applications.push(WorkspaceApplicationRegistration {
            app_id: state.app_id,
            app_version: state.app_version,
            manifest_path: state.manifest_path,
            manifest_digest: state.manifest_digest,
            bundle_digest: state.bundle_digest,
            model_dependencies: state.model_dependencies,
            state_path,
        });
    }

    Ok(WorkspaceApplicationRegistries {
        workspace_id: workspace_id.to_string(),
        applications,
        capability_registry,
        workflow_registry,
    })
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

#[allow(unexpected_cfgs)]
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
    for entry in entries {
        #[cfg(coverage)]
        let path = entry
            .expect("workspace app state directory entries should be readable under coverage")
            .path();
        #[cfg(not(coverage))]
        let path = entry
            .map_err(|error| {
                single_error(
                    WorkspaceAppStateErrorCode::StateReadFailed,
                    root,
                    format!("failed to read workspace app state entry: {error}"),
                )
            })?
            .path();
        if path.is_dir() {
            collect_registration_files(&path, files)?;
        } else if path.file_name().and_then(|name| name.to_str()) == Some("registration.json") {
            files.push(path);
        }
    }
    Ok(())
}

fn read_workspace_application_state(
    workspace_root: &Path,
    workspace_id: &str,
    state_path: &Path,
) -> Result<PersistedWorkspaceApplicationState, WorkspaceAppStateFailure> {
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
    validate_workspace_application_state(workspace_root, workspace_id, state_path, &state)?;
    Ok(state)
}

fn validate_workspace_application_state(
    workspace_root: &Path,
    workspace_id: &str,
    state_path: &Path,
    state: &PersistedWorkspaceApplicationState,
) -> Result<(), WorkspaceAppStateFailure> {
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
    if state.components.is_empty() || state.workflows.is_empty() {
        return Err(single_error(
            WorkspaceAppStateErrorCode::CorruptWorkspaceState,
            state_path,
            "workspace app state must include at least one component and workflow".to_string(),
        ));
    }
    validate_registration_fingerprint(state_path, state)?;
    validate_unique_components(state_path, &state.components)?;
    validate_unique_workflows(state_path, &state.workflows)?;
    validate_paths_exist(workspace_root, state_path, state)
}

fn validate_registration_fingerprint(
    state_path: &Path,
    state: &PersistedWorkspaceApplicationState,
) -> Result<(), WorkspaceAppStateFailure> {
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
    if matches_identity {
        Ok(())
    } else {
        Err(single_error(
            WorkspaceAppStateErrorCode::CorruptWorkspaceState,
            state_path,
            "workspace app registration fingerprint does not match state identity".to_string(),
        ))
    }
}

fn validate_unique_components(
    state_path: &Path,
    components: &[PersistedWorkspaceComponent],
) -> Result<(), WorkspaceAppStateFailure> {
    let mut seen = BTreeSet::new();
    for component in components {
        if !seen.insert((&component.component_id, &component.component_version)) {
            return Err(single_error(
                WorkspaceAppStateErrorCode::CorruptWorkspaceState,
                state_path,
                format!(
                    "duplicate component registration {}@{}",
                    component.component_id, component.component_version
                ),
            ));
        }
    }
    Ok(())
}

fn validate_unique_workflows(
    state_path: &Path,
    workflows: &[PersistedWorkspaceWorkflow],
) -> Result<(), WorkspaceAppStateFailure> {
    let mut seen = BTreeSet::new();
    for workflow in workflows {
        if !seen.insert((&workflow.workflow_id, &workflow.workflow_version)) {
            return Err(single_error(
                WorkspaceAppStateErrorCode::CorruptWorkspaceState,
                state_path,
                format!(
                    "duplicate workflow registration {}@{}",
                    workflow.workflow_id, workflow.workflow_version
                ),
            ));
        }
    }
    Ok(())
}

fn validate_paths_exist(
    workspace_root: &Path,
    state_path: &Path,
    state: &PersistedWorkspaceApplicationState,
) -> Result<(), WorkspaceAppStateFailure> {
    for component in &state.components {
        for path in [
            &component.contract_path,
            &component.manifest_path,
            &component.artifact_ref,
        ] {
            let resolved = resolve_workspace_state_path(workspace_root, path);
            if !resolved.is_file() {
                return Err(single_error(
                    WorkspaceAppStateErrorCode::CorruptWorkspaceState,
                    state_path,
                    format!(
                        "workspace app state references missing artifact {}",
                        resolved.display()
                    ),
                ));
            }
        }
    }
    for workflow in &state.workflows {
        if workflow.workflow_digest.trim().is_empty() {
            return Err(single_error(
                WorkspaceAppStateErrorCode::CorruptWorkspaceState,
                state_path,
                format!(
                    "workflow {}@{} is missing a workflow digest",
                    workflow.workflow_id, workflow.workflow_version
                ),
            ));
        }
        let resolved = resolve_workspace_state_path(workspace_root, &workflow.path);
        if !resolved.is_file() {
            return Err(single_error(
                WorkspaceAppStateErrorCode::CorruptWorkspaceState,
                state_path,
                format!(
                    "workspace app state references missing workflow {}",
                    resolved.display()
                ),
            ));
        }
    }
    Ok(())
}

fn build_workspace_capability_registration(
    workspace_root: &Path,
    validator_version: &str,
    state: &PersistedWorkspaceApplicationState,
    component: &PersistedWorkspaceComponent,
) -> Result<CapabilityRegistration, WorkspaceAppStateFailure> {
    let contract_path = resolve_workspace_state_path(workspace_root, &component.contract_path);
    let contents = fs::read_to_string(&contract_path).map_err(|error| {
        single_error(
            WorkspaceAppStateErrorCode::StateReadFailed,
            &contract_path,
            format!("failed to read registered component contract: {error}"),
        )
    })?;
    let contract = parse_contract(&contents).map_err(|failure| {
        single_error(
            WorkspaceAppStateErrorCode::CorruptWorkspaceState,
            &contract_path,
            format!(
                "registered component contract is invalid: {}",
                failure
                    .errors
                    .iter()
                    .map(|error| error.message.clone())
                    .collect::<Vec<_>>()
                    .join("; ")
            ),
        )
    })?;
    if contract.id != component.capability_id || contract.version != component.capability_version {
        return Err(single_error(
            WorkspaceAppStateErrorCode::CorruptWorkspaceState,
            &contract_path,
            "registered component capability identity does not match contract".to_string(),
        ));
    }

    Ok(CapabilityRegistration {
        scope: RegistryScope::Private,
        contract: contract.clone(),
        contract_path: contract_path.display().to_string(),
        artifact: CapabilityArtifactRecord {
            artifact_ref: format!(
                "app:{}:{}:component:{}:{}",
                state.app_id,
                state.app_version,
                component.component_id,
                component.component_version
            ),
            implementation_kind: ImplementationKind::Executable,
            source: SourceReference {
                kind: SourceKind::Local,
                location: resolve_workspace_state_path(workspace_root, &component.manifest_path)
                    .display()
                    .to_string(),
            },
            binary: Some(BinaryReference {
                format: BinaryFormat::Wasm,
                location: resolve_workspace_state_path(workspace_root, &component.artifact_ref)
                    .display()
                    .to_string(),
                signature: None,
            }),
            workflow_ref: None,
            digests: ArtifactDigests {
                source_digest: governed_content_digest(&contract),
                binary_digest: Some(component.wasm_digest.clone()),
            },
            provenance: RegistryProvenance {
                source: format!("workspace_app_state:{}", state.app_id),
                author: state.app_id.clone(),
                created_at: workspace_state_registered_at(state),
            },
        },
        registered_at: workspace_state_registered_at(state),
        tags: vec![format!("app:{}", state.app_id)],
        composability: ComposabilityMetadata {
            kind: CompositionKind::Atomic,
            patterns: vec![CompositionPattern::Validation],
            provides: vec![contract.id.clone()],
            requires: contract
                .consumes
                .iter()
                .map(|event| event.event_id.clone())
                .collect(),
        },
        governing_spec: WORKSPACE_APP_STATE_GOVERNING_SPEC.to_string(),
        validator_version: validator_version.to_string(),
    })
}

fn build_workspace_workflow_registration(
    workspace_root: &Path,
    validator_version: &str,
    state: &PersistedWorkspaceApplicationState,
    workflow: &PersistedWorkspaceWorkflow,
) -> Result<WorkflowRegistration, WorkspaceAppStateFailure> {
    let workflow_path = resolve_workspace_state_path(workspace_root, &workflow.path);
    let contents = fs::read_to_string(&workflow_path).map_err(|error| {
        single_error(
            WorkspaceAppStateErrorCode::StateReadFailed,
            &workflow_path,
            format!("failed to read registered workflow: {error}"),
        )
    })?;
    let definition = serde_json::from_str::<WorkflowDefinition>(&contents).map_err(|error| {
        single_error(
            WorkspaceAppStateErrorCode::CorruptWorkspaceState,
            &workflow_path,
            format!("failed to parse registered workflow: {error}"),
        )
    })?;
    if definition.id != workflow.workflow_id || definition.version != workflow.workflow_version {
        return Err(single_error(
            WorkspaceAppStateErrorCode::CorruptWorkspaceState,
            &workflow_path,
            "registered workflow identity does not match workflow definition".to_string(),
        ));
    }

    Ok(WorkflowRegistration {
        scope: RegistryScope::Private,
        definition,
        workflow_path: workflow_path.display().to_string(),
        registered_at: workspace_state_registered_at(state),
        validator_version: validator_version.to_string(),
    })
}

fn workspace_state_registered_at(state: &PersistedWorkspaceApplicationState) -> String {
    format!("workspace-app:{}@{}", state.app_id, state.app_version)
}

fn resolve_workspace_state_path(workspace_root: &Path, value: &str) -> PathBuf {
    let path = PathBuf::from(value);
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
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;
    use crate::LookupScope;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn loads_workspace_app_state_into_discoverable_registries() {
        let workspace_root = unique_temp_dir();
        write_workspace_app_state_fixture(&workspace_root, "local", "1.0.0");

        let loaded =
            load_workspace_application_registries(&workspace_root, "local", "test-validator")
                .expect("workspace app state should load");

        assert_eq!(loaded.workspace_id, "local");
        assert_eq!(loaded.applications.len(), 1);
        assert_eq!(loaded.applications[0].app_id, "expedition.readiness");
        assert_eq!(
            loaded.applications[0].model_dependencies[0].interface_id,
            "traverse.inference.generate"
        );
        assert!(
            loaded
                .capability_registry
                .find_exact(
                    LookupScope::PreferPrivate,
                    "expedition.planning.validate-team-readiness",
                    "1.0.0"
                )
                .is_some()
        );
        assert!(
            loaded
                .workflow_registry
                .find_exact(
                    LookupScope::PreferPrivate,
                    "expedition.planning.plan-expedition",
                    "1.0.0",
                )
                .is_some()
        );
        assert_eq!(
            loaded
                .workflow_registry
                .discover(LookupScope::PreferPrivate)
                .len(),
            1
        );
    }

    #[test]
    fn loading_workspace_app_state_is_repeatable() {
        let workspace_root = unique_temp_dir();
        write_workspace_app_state_fixture(&workspace_root, "local", "1.0.0");

        let first =
            load_workspace_application_registries(&workspace_root, "local", "test-validator")
                .expect("first load should succeed");
        let second =
            load_workspace_application_registries(&workspace_root, "local", "test-validator")
                .expect("second load should succeed");

        assert_eq!(first.applications, second.applications);
        assert_eq!(
            first.capability_registry.discover(
                LookupScope::PreferPrivate,
                &crate::DiscoveryQuery::default()
            ),
            second.capability_registry.discover(
                LookupScope::PreferPrivate,
                &crate::DiscoveryQuery::default()
            )
        );
    }

    #[test]
    fn missing_workspace_app_state_returns_stable_error() {
        let workspace_root = unique_temp_dir();

        let failure =
            load_workspace_application_registries(&workspace_root, "local", "test-validator")
                .expect_err("missing state should fail");

        assert_eq!(
            failure.errors[0].code,
            WorkspaceAppStateErrorCode::MissingWorkspaceState
        );
    }

    #[test]
    fn corrupt_workspace_app_state_returns_stable_error() {
        let workspace_root = unique_temp_dir();
        let state_path = workspace_state_path(&workspace_root, "local");
        fs::create_dir_all(state_path.parent().expect("state path must have parent"))
            .expect("state parent should create");
        fs::write(&state_path, "{not json").expect("corrupt state should write");

        let failure =
            load_workspace_application_registries(&workspace_root, "local", "test-validator")
                .expect_err("corrupt state should fail");

        assert_eq!(
            failure.errors[0].code,
            WorkspaceAppStateErrorCode::StateParseFailed
        );
    }

    #[test]
    fn incompatible_workspace_app_state_returns_stable_error() {
        let workspace_root = unique_temp_dir();
        write_workspace_app_state_fixture(&workspace_root, "local", "9.9.9");

        let failure =
            load_workspace_application_registries(&workspace_root, "local", "test-validator")
                .expect_err("unsupported schema should fail");

        assert_eq!(
            failure.errors[0].code,
            WorkspaceAppStateErrorCode::IncompatibleSchemaVersion
        );
    }

    #[test]
    fn empty_workspace_app_directory_reports_missing_registration_files() {
        let workspace_root = unique_temp_dir();
        fs::create_dir_all(
            workspace_root
                .join(".traverse")
                .join("workspaces")
                .join("local")
                .join("apps"),
        )
        .expect("apps directory should create");

        let failure =
            load_workspace_application_registries(&workspace_root, "local", "test-validator")
                .expect_err("empty apps directory should fail");

        assert_eq!(
            failure.errors[0].code,
            WorkspaceAppStateErrorCode::MissingWorkspaceState
        );
    }

    #[test]
    fn unreadable_workspace_app_directory_reports_state_read_failure() {
        let workspace_root = unique_temp_dir();
        let apps_path = workspace_root
            .join(".traverse")
            .join("workspaces")
            .join("local")
            .join("apps");
        fs::create_dir_all(apps_path.parent().expect("apps path should have parent"))
            .expect("workspace parent should create");
        fs::write(&apps_path, "not a directory").expect("apps path file should write");

        let failure =
            load_workspace_application_registries(&workspace_root, "local", "test-validator")
                .expect_err("file-backed apps path should fail");

        assert_eq!(
            failure.errors[0].code,
            WorkspaceAppStateErrorCode::StateReadFailed
        );
    }

    #[test]
    fn workspace_identity_mismatch_reports_incompatible_state() {
        let workspace_root = unique_temp_dir();
        write_workspace_app_state_fixture_with(&workspace_root, "local", "1.0.0", |state| {
            state["workspace_id"] = Value::String("other-workspace".to_string());
        });

        let failure =
            load_workspace_application_registries(&workspace_root, "local", "test-validator")
                .expect_err("wrong workspace identity should fail");

        assert_eq!(
            failure.errors[0].code,
            WorkspaceAppStateErrorCode::IncompatibleWorkspaceState
        );
    }

    #[test]
    fn empty_components_or_workflows_report_corrupt_state() {
        let workspace_root = unique_temp_dir();
        write_workspace_app_state_fixture_with(&workspace_root, "local", "1.0.0", |state| {
            state["components"] = Value::Array(Vec::new());
        });

        let failure =
            load_workspace_application_registries(&workspace_root, "local", "test-validator")
                .expect_err("empty components should fail");

        assert_eq!(
            failure.errors[0].code,
            WorkspaceAppStateErrorCode::CorruptWorkspaceState
        );
    }

    #[test]
    fn fingerprint_mismatch_reports_corrupt_state() {
        let workspace_root = unique_temp_dir();
        write_workspace_app_state_fixture_with(&workspace_root, "local", "1.0.0", |state| {
            state["registration_fingerprint"]["manifest_digest"] =
                Value::String("sha256:other".to_string());
        });

        let failure =
            load_workspace_application_registries(&workspace_root, "local", "test-validator")
                .expect_err("fingerprint mismatch should fail");

        assert_eq!(
            failure.errors[0].code,
            WorkspaceAppStateErrorCode::CorruptWorkspaceState
        );
    }

    #[test]
    fn duplicate_component_identity_reports_corrupt_state() {
        let workspace_root = unique_temp_dir();
        write_workspace_app_state_fixture_with(&workspace_root, "local", "1.0.0", |state| {
            let components = state["components"]
                .as_array_mut()
                .expect("components should be an array");
            components.push(components[0].clone());
        });

        let failure =
            load_workspace_application_registries(&workspace_root, "local", "test-validator")
                .expect_err("duplicate component should fail");

        assert_eq!(
            failure.errors[0].code,
            WorkspaceAppStateErrorCode::CorruptWorkspaceState
        );
    }

    #[test]
    fn duplicate_workflow_identity_reports_corrupt_state() {
        let workspace_root = unique_temp_dir();
        write_workspace_app_state_fixture_with(&workspace_root, "local", "1.0.0", |state| {
            let workflows = state["workflows"]
                .as_array_mut()
                .expect("workflows should be an array");
            workflows.push(workflows[0].clone());
        });

        let failure =
            load_workspace_application_registries(&workspace_root, "local", "test-validator")
                .expect_err("duplicate workflow should fail");

        assert_eq!(
            failure.errors[0].code,
            WorkspaceAppStateErrorCode::CorruptWorkspaceState
        );
    }

    #[test]
    fn missing_component_artifact_reports_corrupt_state() {
        let workspace_root = unique_temp_dir();
        write_workspace_app_state_fixture_with(&workspace_root, "local", "1.0.0", |state| {
            state["components"][0]["artifact_ref"] =
                Value::String("/path/that/does/not/exist.wasm".to_string());
        });

        let failure =
            load_workspace_application_registries(&workspace_root, "local", "test-validator")
                .expect_err("missing artifact should fail");

        assert_eq!(
            failure.errors[0].code,
            WorkspaceAppStateErrorCode::CorruptWorkspaceState
        );
    }

    #[test]
    fn missing_workflow_digest_reports_corrupt_state() {
        let workspace_root = unique_temp_dir();
        write_workspace_app_state_fixture_with(&workspace_root, "local", "1.0.0", |state| {
            state["workflows"][0]["workflow_digest"] = Value::String(String::new());
        });

        let failure =
            load_workspace_application_registries(&workspace_root, "local", "test-validator")
                .expect_err("empty workflow digest should fail");

        assert_eq!(
            failure.errors[0].code,
            WorkspaceAppStateErrorCode::CorruptWorkspaceState
        );
    }

    #[test]
    fn missing_workflow_path_reports_corrupt_state() {
        let workspace_root = unique_temp_dir();
        write_workspace_app_state_fixture_with(&workspace_root, "local", "1.0.0", |state| {
            state["workflows"][0]["path"] =
                Value::String("/path/that/does/not/exist/workflow.json".to_string());
        });

        let failure =
            load_workspace_application_registries(&workspace_root, "local", "test-validator")
                .expect_err("missing workflow should fail");

        assert_eq!(
            failure.errors[0].code,
            WorkspaceAppStateErrorCode::CorruptWorkspaceState
        );
    }

    #[test]
    fn invalid_component_contract_reports_corrupt_state() {
        let workspace_root = unique_temp_dir();
        let invalid_contract = workspace_root.join("invalid-contract.json");
        fs::write(&invalid_contract, "{not json").expect("invalid contract should write");
        write_workspace_app_state_fixture_with(&workspace_root, "local", "1.0.0", |state| {
            state["components"][0]["contract_path"] =
                Value::String(invalid_contract.display().to_string());
        });

        let failure =
            load_workspace_application_registries(&workspace_root, "local", "test-validator")
                .expect_err("invalid contract should fail");

        assert_eq!(
            failure.errors[0].code,
            WorkspaceAppStateErrorCode::CorruptWorkspaceState
        );
    }

    #[test]
    fn component_contract_identity_mismatch_reports_corrupt_state() {
        let workspace_root = unique_temp_dir();
        write_workspace_app_state_fixture_with(&workspace_root, "local", "1.0.0", |state| {
            state["components"][0]["capability_id"] =
                Value::String("expedition.planning.other".to_string());
        });

        let failure =
            load_workspace_application_registries(&workspace_root, "local", "test-validator")
                .expect_err("contract identity mismatch should fail");

        assert_eq!(
            failure.errors[0].code,
            WorkspaceAppStateErrorCode::CorruptWorkspaceState
        );
    }

    #[test]
    fn duplicate_capability_registration_reports_stable_error() {
        let workspace_root = unique_temp_dir();
        write_workspace_app_state_fixture_with(&workspace_root, "local", "1.0.0", |state| {
            let components = state["components"]
                .as_array_mut()
                .expect("components should be an array");
            let mut duplicate = components[0].clone();
            duplicate["component_id"] =
                Value::String("expedition.readiness.conflicting-component".to_string());
            duplicate["wasm_digest"] = Value::String("sha256:other-binary".to_string());
            components.insert(1, duplicate);
        });

        let failure =
            load_workspace_application_registries(&workspace_root, "local", "test-validator")
                .expect_err("conflicting capability registration should fail");

        assert_eq!(
            failure.errors[0].code,
            WorkspaceAppStateErrorCode::CapabilityRegistrationFailed
        );
    }

    #[test]
    fn invalid_workflow_json_reports_corrupt_state() {
        let workspace_root = unique_temp_dir();
        let invalid_workflow = workspace_root.join("invalid-workflow.json");
        fs::write(&invalid_workflow, "{not json").expect("invalid workflow should write");
        write_workspace_app_state_fixture_with(&workspace_root, "local", "1.0.0", |state| {
            state["workflows"][0]["path"] = Value::String(invalid_workflow.display().to_string());
        });

        let failure =
            load_workspace_application_registries(&workspace_root, "local", "test-validator")
                .expect_err("invalid workflow should fail");

        assert_eq!(
            failure.errors[0].code,
            WorkspaceAppStateErrorCode::CorruptWorkspaceState
        );
    }

    #[test]
    fn workflow_identity_mismatch_reports_corrupt_state() {
        let workspace_root = unique_temp_dir();
        write_workspace_app_state_fixture_with(&workspace_root, "local", "1.0.0", |state| {
            state["workflows"][0]["workflow_id"] =
                Value::String("expedition.planning.other-workflow".to_string());
        });

        let failure =
            load_workspace_application_registries(&workspace_root, "local", "test-validator")
                .expect_err("workflow identity mismatch should fail");

        assert_eq!(
            failure.errors[0].code,
            WorkspaceAppStateErrorCode::CorruptWorkspaceState
        );
    }

    #[test]
    fn workflow_registration_failure_reports_stable_error() {
        let workspace_root = unique_temp_dir();
        write_workspace_app_state_fixture_with(&workspace_root, "local", "1.0.0", |state| {
            let components = state["components"]
                .as_array_mut()
                .expect("components should be an array");
            components.truncate(1);
        });

        let failure =
            load_workspace_application_registries(&workspace_root, "local", "test-validator")
                .expect_err("workflow with missing capabilities should fail");

        assert_eq!(
            failure.errors[0].code,
            WorkspaceAppStateErrorCode::WorkflowRegistrationFailed
        );
    }

    #[test]
    fn direct_state_read_failure_reports_stable_error() {
        let workspace_root = unique_temp_dir();
        let failure = read_workspace_application_state(
            &workspace_root,
            "local",
            &workspace_root.join("missing-registration.json"),
        )
        .expect_err("missing state file should fail");

        assert_eq!(
            failure.errors[0].code,
            WorkspaceAppStateErrorCode::StateReadFailed
        );
    }

    #[test]
    fn direct_component_contract_read_failure_reports_stable_error() {
        let workspace_root = unique_temp_dir();
        write_workspace_app_state_fixture(&workspace_root, "local", "1.0.0");
        let state_path = workspace_state_path(&workspace_root, "local");
        let state = read_workspace_application_state(&workspace_root, "local", &state_path)
            .expect("state should load");
        let mut component = state.components[0].clone();
        component.contract_path = workspace_root
            .join("missing-contract.json")
            .display()
            .to_string();

        let failure = build_workspace_capability_registration(
            &workspace_root,
            "test-validator",
            &state,
            &component,
        )
        .expect_err("missing contract should fail");

        assert_eq!(
            failure.errors[0].code,
            WorkspaceAppStateErrorCode::StateReadFailed
        );
    }

    #[test]
    fn direct_workflow_read_failure_reports_stable_error() {
        let workspace_root = unique_temp_dir();
        write_workspace_app_state_fixture(&workspace_root, "local", "1.0.0");
        let state_path = workspace_state_path(&workspace_root, "local");
        let state = read_workspace_application_state(&workspace_root, "local", &state_path)
            .expect("state should load");
        let mut workflow = state.workflows[0].clone();
        workflow.path = workspace_root
            .join("missing-workflow.json")
            .display()
            .to_string();

        let failure = build_workspace_workflow_registration(
            &workspace_root,
            "test-validator",
            &state,
            &workflow,
        )
        .expect_err("missing workflow should fail");

        assert_eq!(
            failure.errors[0].code,
            WorkspaceAppStateErrorCode::StateReadFailed
        );
    }

    #[test]
    fn relative_workspace_paths_load_successfully() {
        let workspace_root = unique_temp_dir();
        write_relative_workspace_artifacts(&workspace_root);
        write_workspace_app_state_fixture_with(&workspace_root, "local", "1.0.0", |state| {
            state["manifest_path"] = Value::String("fixtures/app.manifest.json".to_string());
            for component in state["components"]
                .as_array_mut()
                .expect("components should be an array")
            {
                component["manifest_path"] =
                    Value::String("fixtures/component.manifest.json".to_string());
                component["artifact_ref"] = Value::String("fixtures/component.wasm".to_string());
                let capability_id = component["capability_id"]
                    .as_str()
                    .expect("capability id should be a string");
                let leaf = capability_id
                    .rsplit('.')
                    .next()
                    .expect("capability id should include a leaf");
                component["contract_path"] =
                    Value::String(format!("fixtures/{leaf}.contract.json"));
            }
            state["workflows"][0]["path"] = Value::String("fixtures/workflow.json".to_string());
        });

        let loaded =
            load_workspace_application_registries(&workspace_root, "local", "test-validator")
                .expect("relative state paths should load");

        assert_eq!(loaded.applications.len(), 1);
    }

    fn write_workspace_app_state_fixture(
        workspace_root: &Path,
        workspace_id: &str,
        schema_version: &str,
    ) {
        write_workspace_app_state_fixture_with(
            workspace_root,
            workspace_id,
            schema_version,
            |_| {},
        );
    }

    fn write_workspace_app_state_fixture_with(
        workspace_root: &Path,
        workspace_id: &str,
        schema_version: &str,
        mutate: impl FnOnce(&mut Value),
    ) {
        let repo = repo_root();
        let state_path = workspace_state_path(workspace_root, workspace_id);
        fs::create_dir_all(state_path.parent().expect("state path must have parent"))
            .expect("state parent should create");
        let mut state = serde_json::json!({
            "status": "registered",
            "workspace_id": workspace_id,
            "app_id": "expedition.readiness",
            "app_version": "1.0.0",
            "schema_version": schema_version,
            "manifest_path": repo.join("examples/applications/expedition-readiness/app.manifest.json").display().to_string(),
            "manifest_digest": "sha256:test-manifest",
            "bundle_digest": "sha256:test-bundle",
            "component_ids": [
                "expedition.readiness.validate-team-readiness-component"
            ],
            "workflow_ids": [
                "expedition.planning.plan-expedition"
            ],
            "components": expedition_components_json(&repo),
            "workflows": [{
                "workflow_id": "expedition.planning.plan-expedition",
                "workflow_version": "1.0.0",
                "workflow_digest": "sha256:test-workflow",
                "path": repo.join("workflows/examples/expedition/plan-expedition/workflow.json").display().to_string()
            }],
            "model_dependencies": [{
                "interface_id": "traverse.inference.generate",
                "version_range": "^1.0",
                "selection_policy": {
                    "strategy": "priority",
                    "allow_fallback": true
                },
                "required_capabilities": ["text_generation"],
                "minimum_context_window": 8192,
                "candidates": [{
                    "candidate_id": "ollama-llama-3-2-readiness",
                    "provider_capability_id": "traverse.inference.generate",
                    "provider_implementation_id": "ollama.local.generate",
                    "model_identifier": "llama3.2:3b",
                    "placement_target": "local",
                    "priority": 10,
                    "required_provider_config_keys": ["ollama_base_url"],
                    "metadata": {
                        "implementation_kind": "real_local_provider",
                        "provider": "ollama",
                        "model_context_window": 8192
                    }
                }]
            }],
            "effective_config": {
                "values": {
                    "workspace_id": "expedition-local",
                    "readiness_mode": "deterministic"
                },
                "redacted_secret_keys": []
            },
            "state_scope": "workspace_persisted",
            "registration_fingerprint": {
                "app_id": "expedition.readiness",
                "app_version": "1.0.0",
                "manifest_digest": "sha256:test-manifest"
            }
        });
        mutate(&mut state);
        fs::write(
            state_path,
            serde_json::to_string_pretty(&state).expect("state JSON should serialize"),
        )
        .expect("state should write");
    }

    fn write_relative_workspace_artifacts(workspace_root: &Path) {
        let repo = repo_root();
        let fixtures = workspace_root.join("fixtures");
        fs::create_dir_all(&fixtures).expect("fixtures directory should create");
        fs::copy(
            repo.join("examples/applications/expedition-readiness/app.manifest.json"),
            fixtures.join("app.manifest.json"),
        )
        .expect("app manifest fixture should copy");
        fs::copy(
            repo.join("examples/applications/expedition-readiness/components/validate-team-readiness/component.manifest.json"),
            fixtures.join("component.manifest.json"),
        )
        .expect("component manifest fixture should copy");
        fs::copy(
            repo.join(
                "examples/agents/team-readiness-agent/artifacts/validate-team-readiness-agent.wasm",
            ),
            fixtures.join("component.wasm"),
        )
        .expect("component artifact fixture should copy");
        fs::copy(
            repo.join("workflows/examples/expedition/plan-expedition/workflow.json"),
            fixtures.join("workflow.json"),
        )
        .expect("workflow fixture should copy");
        for (leaf, _) in expedition_component_identities() {
            fs::copy(
                repo.join(format!(
                    "contracts/examples/expedition/capabilities/{leaf}/contract.json"
                )),
                fixtures.join(format!("{leaf}.contract.json")),
            )
            .expect("contract fixture should copy");
        }
    }

    fn expedition_components_json(repo: &Path) -> Vec<Value> {
        expedition_component_identities()
        .into_iter()
        .map(|(leaf, capability_id)| {
            serde_json::json!({
                "component_id": format!("expedition.readiness.{leaf}-component"),
                "component_version": "1.0.0",
                "capability_id": capability_id,
                "capability_version": "1.0.0",
                "wasm_digest": "sha256:5647c39a1d25d8728350f9619025292a62e78a602068a2ad9b6f075751c93d99",
                "manifest_path": repo.join("examples/applications/expedition-readiness/components/validate-team-readiness/component.manifest.json").display().to_string(),
                "contract_path": repo.join(format!("contracts/examples/expedition/capabilities/{leaf}/contract.json")).display().to_string(),
                "artifact_ref": repo.join("examples/agents/team-readiness-agent/artifacts/validate-team-readiness-agent.wasm").display().to_string()
            })
        })
        .collect()
    }

    fn expedition_component_identities() -> [(&'static str, &'static str); 5] {
        [
            (
                "capture-expedition-objective",
                "expedition.planning.capture-expedition-objective",
            ),
            (
                "interpret-expedition-intent",
                "expedition.planning.interpret-expedition-intent",
            ),
            (
                "assess-conditions-summary",
                "expedition.planning.assess-conditions-summary",
            ),
            (
                "validate-team-readiness",
                "expedition.planning.validate-team-readiness",
            ),
            (
                "assemble-expedition-plan",
                "expedition.planning.assemble-expedition-plan",
            ),
        ]
    }

    fn workspace_state_path(workspace_root: &Path, workspace_id: &str) -> PathBuf {
        workspace_root
            .join(".traverse/workspaces")
            .join(workspace_id)
            .join("apps/expedition.readiness/1.0.0/registration.json")
    }

    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    fn unique_temp_dir() -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let counter = TEMP_COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!(
            "traverse-workspace-state-test-{}-{nanos}-{counter}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("temporary directory should create");
        path
    }
}
