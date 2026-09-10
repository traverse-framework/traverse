use serde_json::{Value, json};
use std::env;
use std::path::Path;
use traverse_registry::{ResolvedCapability, WorkspaceAppStateErrorCode};
use traverse_runtime::{LocalExecutionFailure, LocalExecutionOutput, LocalExecutor, Runtime};

#[derive(Debug)]
struct ConformanceExecutor;

impl LocalExecutor for ConformanceExecutor {
    fn execute(
        &self,
        _capability: &ResolvedCapability,
        _input: &Value,
    ) -> Result<LocalExecutionOutput, LocalExecutionFailure> {
        Ok(LocalExecutionOutput {
            value: json!({"status": "not_executed"}),
            emitted_events: Vec::new(),
        })
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
            let capabilities = runtime.capability_metadata_index().len();
            let workflows = runtime.indexed_workflows().count();
            println!(
                "{}",
                json!({
                    "status": "loaded",
                    "workspace_id": workspace_id,
                    "capability_count": capabilities,
                    "workflow_count": workflows,
                    "capability_ids": runtime
                        .capability_metadata_index()
                        .entries()
                        .map(|entry| entry.capability_id.clone())
                        .collect::<Vec<_>>(),
                    "workflow_ids": runtime
                        .indexed_workflows()
                        .map(|workflow| workflow.definition.id.clone())
                        .collect::<Vec<_>>()
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
