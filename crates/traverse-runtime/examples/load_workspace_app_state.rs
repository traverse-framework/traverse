use serde_json::{Value, json};
use std::env;
use std::path::Path;
use traverse_registry::{
    DiscoveryQuery, LookupScope, ResolvedCapability, WorkspaceAppStateErrorCode,
};
use traverse_runtime::{LocalExecutionFailure, LocalExecutor, Runtime};

#[derive(Debug)]
struct ConformanceExecutor;

impl LocalExecutor for ConformanceExecutor {
    fn execute(
        &self,
        _capability: &ResolvedCapability,
        _input: &Value,
    ) -> Result<Value, LocalExecutionFailure> {
        Ok(json!({"status": "not_executed"}))
    }
}

fn main() {
    let mut args = env::args().skip(1);
    let Some(workspace_root) = args.next() else {
        fail("missing workspace root");
    };
    let Some(workspace_id) = args.next() else {
        fail("missing workspace id");
    };
    if args.next().is_some() {
        fail("expected exactly <workspace-root> <workspace-id>");
    }

    match Runtime::from_workspace_app_state(
        Path::new(&workspace_root),
        &workspace_id,
        ConformanceExecutor,
        "downstream-app-conformance",
    ) {
        Ok(runtime) => {
            let capabilities = runtime
                .capability_registry()
                .discover(LookupScope::PreferPrivate, &DiscoveryQuery::default());
            let workflows = runtime
                .workflow_registry()
                .discover(LookupScope::PreferPrivate);
            println!(
                "{}",
                json!({
                    "status": "loaded",
                    "workspace_id": workspace_id,
                    "capability_count": capabilities.len(),
                    "workflow_count": workflows.len(),
                    "capability_ids": capabilities.iter().map(|entry| entry.id.clone()).collect::<Vec<_>>(),
                    "workflow_ids": workflows.iter().map(|entry| entry.id.clone()).collect::<Vec<_>>()
                })
            );
        }
        Err(failure) => {
            let errors = failure
                .errors
                .into_iter()
                .map(|error| {
                    json!({
                        "code": error_code(error.code),
                        "path": error.path,
                        "message": error.message
                    })
                })
                .collect::<Vec<_>>();
            eprintln!("{}", json!({"status": "failed", "errors": errors}));
            std::process::exit(1);
        }
    }
}

fn fail(message: &str) -> ! {
    eprintln!("{}", json!({"status": "failed", "message": message}));
    std::process::exit(2);
}

fn error_code(code: WorkspaceAppStateErrorCode) -> &'static str {
    match code {
        WorkspaceAppStateErrorCode::MissingWorkspaceState => "missing_workspace_state",
        WorkspaceAppStateErrorCode::StateReadFailed => "state_read_failed",
        WorkspaceAppStateErrorCode::StateParseFailed => "state_parse_failed",
        WorkspaceAppStateErrorCode::IncompatibleSchemaVersion => "incompatible_schema_version",
        WorkspaceAppStateErrorCode::IncompatibleWorkspaceState => "incompatible_workspace_state",
        WorkspaceAppStateErrorCode::CorruptWorkspaceState => "corrupt_workspace_state",
        WorkspaceAppStateErrorCode::CapabilityRegistrationFailed => {
            "capability_registration_failed"
        }
        WorkspaceAppStateErrorCode::WorkflowRegistrationFailed => "workflow_registration_failed",
    }
}
