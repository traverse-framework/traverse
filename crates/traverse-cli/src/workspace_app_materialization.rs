//! Materialize persisted workspace app registration state for `serve`.
//!
//! Command dispatch and proposal handling consume the durable
//! `registration.json` written by `app register`. They do not reopen or
//! re-resolve the source application manifest.

use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use traverse_contracts::{ExecutionTarget, parse_contract};
use traverse_registry::{
    ApplicationBundleManifest, ApplicationComponent, ApplicationComponentRef,
    ApplicationConnectorBinding, ApplicationEffectiveConfig, ApplicationModelDependency,
    ApplicationState, ApplicationStateInvoke, ApplicationStateMachine, ApplicationStateTransition,
    ApplicationStateTransitionCondition, ApplicationStateTransitionConditionOp,
    ApplicationWorkflowRef, ComponentExecutionMode, WasmComponentManifest,
    WorkspaceApplicationRegistration,
};

pub(crate) const APP_REGISTRATION_REQUIRES_REFRESH: &str = "app_registration_requires_refresh";
pub(crate) const APP_UNAVAILABLE: &str = "app_unavailable";

const REFRESH_MESSAGE: &str = "persisted application state machine is missing; re-register the app";
const UNAVAILABLE_MESSAGE: &str = "persisted application state machine cannot be materialized";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AppLoadFailure {
    pub(crate) status: u16,
    pub(crate) reason: &'static str,
    pub(crate) code: &'static str,
    pub(crate) message: String,
}

impl AppLoadFailure {
    fn requires_refresh() -> Self {
        Self {
            status: 409,
            reason: "Conflict",
            code: APP_REGISTRATION_REQUIRES_REFRESH,
            message: REFRESH_MESSAGE.to_string(),
        }
    }

    fn unavailable() -> Self {
        Self {
            status: 503,
            reason: "Service Unavailable",
            code: APP_UNAVAILABLE,
            message: UNAVAILABLE_MESSAGE.to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct MaterializedWorkspaceApp {
    pub(crate) app_id: String,
    pub(crate) machine: Option<ApplicationStateMachine>,
    pub(crate) failure: Option<AppLoadFailure>,
    pub(crate) proposal_manifest: Option<ApplicationBundleManifest>,
}

#[derive(Debug, Deserialize)]
struct PersistedRegistration {
    #[serde(default)]
    app_id: String,
    #[serde(default)]
    app_version: String,
    #[serde(default)]
    schema_version: String,
    #[serde(default)]
    components: Vec<PersistedComponent>,
    #[serde(default)]
    workflows: Vec<ApplicationWorkflowRef>,
    #[serde(default)]
    model_dependencies: Vec<ApplicationModelDependency>,
    #[serde(default)]
    connector_bindings: Vec<ApplicationConnectorBinding>,
    #[serde(default)]
    public_surfaces: Vec<String>,
    #[serde(default)]
    workspace_defaults: Value,
    #[serde(default)]
    config_schema: Value,
    #[serde(default)]
    default_config: Value,
    #[serde(default)]
    placement_policy: Value,
    #[serde(default)]
    effective_config: Option<ApplicationEffectiveConfig>,
    state_machine: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct PersistedComponent {
    component_id: String,
    #[serde(default)]
    component_version: String,
    capability_id: String,
    #[serde(default)]
    capability_version: String,
    #[serde(default)]
    wasm_digest: Option<String>,
    #[serde(default)]
    manifest_path: String,
    #[serde(default)]
    contract_path: String,
    #[serde(default)]
    artifact_ref: Option<String>,
    #[serde(default)]
    execution_mode: Option<String>,
    #[serde(default)]
    platforms: Vec<String>,
    #[serde(default)]
    wrapper_path: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PersistedStateMachine {
    initial_state: String,
    #[serde(default)]
    list_context_fields: Vec<String>,
    states: Vec<PersistedState>,
}

#[derive(Debug, Deserialize)]
struct PersistedState {
    id: String,
    invoke: Option<PersistedInvoke>,
    #[serde(default)]
    transitions: Vec<PersistedTransition>,
}

#[derive(Debug, Deserialize)]
struct PersistedInvoke {
    capability_id: String,
    input_from: String,
}

#[derive(Debug, Deserialize)]
struct PersistedTransition {
    on: String,
    to: String,
    #[serde(default)]
    condition: Option<PersistedCondition>,
    #[serde(default)]
    with_last_payload: bool,
}

#[derive(Debug, Deserialize)]
struct PersistedCondition {
    field: String,
    op: PersistedConditionOp,
    #[serde(default)]
    value: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum PersistedConditionOp {
    Eq,
    Neq,
    Gt,
    Gte,
    Lt,
    Lte,
    In,
    Exists,
}

/// Load every registered workspace app's command surface from durable state.
pub(crate) fn materialize_workspace_apps(
    apps: &[WorkspaceApplicationRegistration],
) -> Vec<MaterializedWorkspaceApp> {
    apps.iter().map(materialize_workspace_app).collect()
}

fn materialize_workspace_app(app: &WorkspaceApplicationRegistration) -> MaterializedWorkspaceApp {
    match read_persisted_registration(&app.state_path) {
        Ok(registration) => materialize_from_registration(app, &registration),
        Err(()) => MaterializedWorkspaceApp {
            app_id: app.app_id.clone(),
            machine: None,
            failure: Some(AppLoadFailure::unavailable()),
            proposal_manifest: None,
        },
    }
}

fn read_persisted_registration(state_path: &Path) -> Result<PersistedRegistration, ()> {
    let bytes = fs::read(state_path).map_err(|_| ())?;
    serde_json::from_slice(&bytes).map_err(|_| ())
}

fn materialize_from_registration(
    app: &WorkspaceApplicationRegistration,
    registration: &PersistedRegistration,
) -> MaterializedWorkspaceApp {
    let registered_capabilities = registration
        .components
        .iter()
        .map(|component| component.capability_id.clone())
        .collect::<BTreeSet<_>>();
    let proposal_manifest = reconstruct_proposal_manifest(registration);

    match registration.state_machine.as_ref() {
        None | Some(Value::Null) => MaterializedWorkspaceApp {
            app_id: app.app_id.clone(),
            machine: None,
            failure: Some(AppLoadFailure::requires_refresh()),
            proposal_manifest,
        },
        Some(value) => match decode_state_machine(value, &registered_capabilities) {
            Ok(machine) => MaterializedWorkspaceApp {
                app_id: app.app_id.clone(),
                machine: Some(machine),
                failure: None,
                proposal_manifest,
            },
            Err(()) => MaterializedWorkspaceApp {
                app_id: app.app_id.clone(),
                machine: None,
                failure: Some(AppLoadFailure::unavailable()),
                proposal_manifest,
            },
        },
    }
}

fn decode_state_machine(
    value: &Value,
    registered_capabilities: &BTreeSet<String>,
) -> Result<ApplicationStateMachine, ()> {
    let persisted: PersistedStateMachine = serde_json::from_value(value.clone()).map_err(|_| ())?;
    if persisted.initial_state.trim().is_empty() || persisted.states.is_empty() {
        return Err(());
    }
    let state_ids = persisted
        .states
        .iter()
        .map(|state| state.id.clone())
        .collect::<BTreeSet<_>>();
    if state_ids.len() != persisted.states.len() || !state_ids.contains(&persisted.initial_state) {
        return Err(());
    }
    let mut states = Vec::new();
    for state in persisted.states {
        if let Some(invoke) = &state.invoke
            && !registered_capabilities.contains(&invoke.capability_id)
        {
            return Err(());
        }
        states.push(ApplicationState {
            id: state.id,
            invoke: state.invoke.map(|invoke| ApplicationStateInvoke {
                capability_id: invoke.capability_id,
                input_from: invoke.input_from,
            }),
            transitions: state
                .transitions
                .into_iter()
                .map(application_state_transition)
                .collect(),
        });
    }
    Ok(ApplicationStateMachine {
        initial_state: persisted.initial_state,
        list_context_fields: persisted.list_context_fields,
        states,
    })
}

fn application_state_transition(transition: PersistedTransition) -> ApplicationStateTransition {
    ApplicationStateTransition {
        on: transition.on,
        to: transition.to,
        condition: transition
            .condition
            .map(|condition| ApplicationStateTransitionCondition {
                field: condition.field,
                op: match condition.op {
                    PersistedConditionOp::Eq => ApplicationStateTransitionConditionOp::Eq,
                    PersistedConditionOp::Neq => ApplicationStateTransitionConditionOp::Neq,
                    PersistedConditionOp::Gt => ApplicationStateTransitionConditionOp::Gt,
                    PersistedConditionOp::Gte => ApplicationStateTransitionConditionOp::Gte,
                    PersistedConditionOp::Lt => ApplicationStateTransitionConditionOp::Lt,
                    PersistedConditionOp::Lte => ApplicationStateTransitionConditionOp::Lte,
                    PersistedConditionOp::In => ApplicationStateTransitionConditionOp::In,
                    PersistedConditionOp::Exists => ApplicationStateTransitionConditionOp::Exists,
                },
                value: condition.value,
            }),
        with_last_payload: transition.with_last_payload,
    }
}

fn reconstruct_proposal_manifest(
    registration: &PersistedRegistration,
) -> Option<ApplicationBundleManifest> {
    let mut components = Vec::new();
    for component in &registration.components {
        let contents = fs::read_to_string(&component.contract_path).ok()?;
        let contract = parse_contract(&contents).ok()?;
        let execution_mode = match component.execution_mode.as_deref() {
            Some("compatible") => ComponentExecutionMode::Compatible,
            _ => ComponentExecutionMode::Wasm,
        };
        components.push(ApplicationComponent {
            reference: ApplicationComponentRef {
                component_id: component.component_id.clone(),
                version: component.component_version.clone(),
                digest: component.wasm_digest.clone().unwrap_or_default(),
                manifest_path: component.manifest_path.clone(),
            },
            manifest_path: Path::new(&component.manifest_path).to_path_buf(),
            manifest: WasmComponentManifest {
                component_id: component.component_id.clone(),
                version: component.component_version.clone(),
                schema_version: registration.schema_version.clone(),
                execution_mode,
                capability_id: component.capability_id.clone(),
                capability_version: component.capability_version.clone(),
                contract_path: Some(component.contract_path.clone()),
                registry_ref: None,
                wasm_binary_path: component.artifact_ref.clone(),
                wasm_digest: component.wasm_digest.clone(),
                platforms: component.platforms.clone(),
                wrapper_path: component.wrapper_path.clone(),
                runtime_constraints: Value::Object(serde_json::Map::new()),
                permitted_targets: vec![ExecutionTarget::Local],
                dependencies: Vec::new(),
                connector_requirements: Vec::new(),
                validation_evidence: Vec::new(),
                executable_pin: None,
            },
            contract_path: Path::new(&component.contract_path).to_path_buf(),
            contract,
            wasm_binary_path: component.artifact_ref.as_ref().map(PathBuf::from),
            verified_wasm_digest: component.wasm_digest.clone(),
        });
    }
    Some(ApplicationBundleManifest {
        app_id: registration.app_id.clone(),
        version: registration.app_version.clone(),
        schema_version: registration.schema_version.clone(),
        workspace_defaults: registration.workspace_defaults.clone(),
        components,
        workflows: registration.workflows.clone(),
        connector_bindings: registration.connector_bindings.clone(),
        model_dependencies: registration.model_dependencies.clone(),
        config_schema: registration.config_schema.clone(),
        default_config: registration.default_config.clone(),
        effective_config: registration.effective_config.clone().unwrap_or(
            ApplicationEffectiveConfig {
                values: Value::Object(serde_json::Map::new()),
                redacted_secret_keys: Vec::new(),
            },
        ),
        placement_policy: registration.placement_policy.clone(),
        public_surfaces: registration.public_surfaces.clone(),
        state_machine: None,
    })
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};
    use traverse_registry::WorkspaceApplicationRegistration;

    fn unique_dir() -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time must be valid")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "traverse-cli-app-materialization-{}-{suffix}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("temp dir must create");
        dir
    }

    fn app_for(state_path: &Path) -> WorkspaceApplicationRegistration {
        WorkspaceApplicationRegistration {
            app_id: "registry-consumer".to_string(),
            app_version: "1.0.0".to_string(),
            manifest_path: "/nonexistent/app.manifest.json".to_string(),
            manifest_digest: "sha256:test".to_string(),
            bundle_digest: "sha256:bundle".to_string(),
            model_dependencies: Vec::new(),
            state_path: state_path.to_path_buf(),
        }
    }

    fn write_registration(state_path: &Path, body: &Value) {
        if let Some(parent) = state_path.parent() {
            fs::create_dir_all(parent).expect("registration parent must create");
        }
        fs::write(
            state_path,
            serde_json::to_vec_pretty(body).expect("registration must serialize"),
        )
        .expect("registration must write");
    }

    fn base_registration(state_machine: Option<Value>) -> Value {
        let mut body = serde_json::json!({
            "app_id": "registry-consumer",
            "app_version": "1.0.0",
            "schema_version": "1.0.0",
            "components": [{
                "component_id": "registry-consumer.process-component",
                "component_version": "1.0.0",
                "capability_id": "traverse-starter.process",
                "capability_version": "1.0.0",
                "wasm_digest": "sha256:test",
                "manifest_path": "component.manifest.json",
                "contract_path": "missing-contract.json",
                "artifact_ref": "missing.wasm"
            }]
        });
        if let Some(state_machine) = state_machine {
            body["state_machine"] = state_machine;
        }
        body
    }

    fn valid_machine() -> Value {
        serde_json::json!({
            "initial_state": "idle",
            "states": [
                {
                    "id": "idle",
                    "transitions": [{ "on": "submit", "to": "processing" }]
                },
                {
                    "id": "processing",
                    "invoke": {
                        "capability_id": "traverse-starter.process",
                        "input_from": "command.payload"
                    },
                    "transitions": [
                        { "on": "capability_succeeded", "to": "results" },
                        { "on": "capability_failed", "to": "error" }
                    ]
                },
                { "id": "results", "transitions": [] },
                { "id": "error", "transitions": [] }
            ]
        })
    }

    #[test]
    fn missing_state_machine_requires_refresh_without_leaking_paths() {
        let dir = unique_dir();
        let state_path = dir.join("registration.json");
        write_registration(&state_path, &base_registration(None));

        let loaded = materialize_workspace_app(&app_for(&state_path));
        let failure = loaded.failure.expect("missing declaration must fail");
        assert_eq!(failure.code, APP_REGISTRATION_REQUIRES_REFRESH);
        assert_eq!(failure.status, 409);
        assert!(loaded.machine.is_none());
        assert!(
            !failure
                .message
                .contains(state_path.to_string_lossy().as_ref())
        );
        assert!(!failure.message.contains("/nonexistent"));
    }

    #[test]
    fn undeclared_invoke_capability_is_unavailable() {
        let dir = unique_dir();
        let state_path = dir.join("registration.json");
        let mut machine = valid_machine();
        machine["states"][1]["invoke"]["capability_id"] = serde_json::json!("not.registered");
        write_registration(&state_path, &base_registration(Some(machine)));

        let loaded = materialize_workspace_app(&app_for(&state_path));
        let failure = loaded.failure.expect("invalid declaration must fail");
        assert_eq!(failure.code, APP_UNAVAILABLE);
        assert_eq!(failure.status, 503);
        assert!(loaded.machine.is_none());
        assert_eq!(failure.message, UNAVAILABLE_MESSAGE);
    }

    #[test]
    fn valid_persisted_state_machine_materializes() {
        let dir = unique_dir();
        let state_path = dir.join("registration.json");
        write_registration(&state_path, &base_registration(Some(valid_machine())));

        let loaded = materialize_workspace_app(&app_for(&state_path));
        assert!(loaded.failure.is_none());
        let machine = loaded.machine.expect("valid declaration must load");
        assert_eq!(machine.initial_state, "idle");
        assert_eq!(
            machine.states[1]
                .invoke
                .as_ref()
                .expect("processing invoke")
                .capability_id,
            "traverse-starter.process"
        );
    }

    #[test]
    fn malformed_state_machine_is_unavailable() {
        let dir = unique_dir();
        let state_path = dir.join("registration.json");
        write_registration(
            &state_path,
            &base_registration(Some(serde_json::json!("not-an-object"))),
        );

        let loaded = materialize_workspace_app(&app_for(&state_path));
        let failure = loaded.failure.expect("malformed declaration must fail");
        assert_eq!(failure.code, APP_UNAVAILABLE);
        assert!(!failure.message.contains("not-an-object"));
    }
}
