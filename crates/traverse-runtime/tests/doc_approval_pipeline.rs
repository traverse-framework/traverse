//! Canonical doc-approval pipeline integration (spec 069):
//! `doc-approval.pipeline` executes `analyze` then `recommend` with
//! runtime-owned merged output, deterministically, and without any
//! `doc-approval.extract` capability.
#![allow(clippy::expect_used)]

use serde_json::{Value, json};
use std::path::PathBuf;
use traverse_registry::{
    ApplicationRegistrationRequest, ApplicationRegistry, CapabilityRegistry, EventRegistry,
    LookupScope, RegistryScope, ResolvedCapability, WorkflowRegistry,
};
use traverse_runtime::{
    LocalExecutionFailure, LocalExecutionFailureCode, LocalExecutor, Runtime,
    WorkflowExecutionRequest, WorkflowLookupScope, WorkflowTraversalStatus,
};

/// Deterministic stand-in for the bundled WASM agents: produces the exact
/// contract output shapes from the node input, with no randomness or state.
#[derive(Clone)]
struct DocApprovalExecutor;

impl LocalExecutor for DocApprovalExecutor {
    fn execute(
        &self,
        capability: &ResolvedCapability,
        input: &Value,
    ) -> Result<Value, LocalExecutionFailure> {
        match capability.contract.id.as_str() {
            "doc-approval.analyze" => {
                let document = input["document"].as_str().unwrap_or_default();
                Ok(json!({
                    "docType": if document.contains("Invoice") { "invoice" } else { "contract" },
                    "parties": ["Acme Corp", "Globex LLC"],
                    "amounts": ["USD 12000.00"],
                    "confidence": "high",
                    "recommendation": "review",
                }))
            }
            "doc-approval.recommend" => {
                let confidence = input["confidence"].as_str().unwrap_or_default();
                let doc_type = input["docType"].as_str().unwrap_or_default();
                Ok(json!({
                    "recommendation": if confidence == "high" { "approve" } else { "escalate" },
                    "rationale": format!(
                        "deterministic {doc_type} analysis with {confidence} confidence"
                    ),
                    "confidence": confidence,
                }))
            }
            other => Err(LocalExecutionFailure {
                code: LocalExecutionFailureCode::ConstraintViolated,
                message: format!("unexpected capability in doc-approval pipeline: {other}"),
            }),
        }
    }
}

fn doc_approval_runtime() -> Runtime<DocApprovalExecutor> {
    let manifest_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/applications/doc-approval/app.manifest.json");

    let mut applications = ApplicationRegistry::new();
    let mut capabilities = CapabilityRegistry::new();
    let events = EventRegistry::new();
    let mut workflows = WorkflowRegistry::new();
    applications
        .register_bundle(
            &mut capabilities,
            &events,
            &mut workflows,
            &ApplicationRegistrationRequest {
                scope: RegistryScope::Private,
                workspace_id: "local-default".to_string(),
                manifest_path,
                registered_at: "2026-07-14T00:00:00Z".to_string(),
                validator_version: "test".to_string(),
            },
        )
        .expect("checked-in doc-approval bundle should register");

    assert!(
        capabilities
            .find_exact(LookupScope::PreferPrivate, "doc-approval.analyze", "1.0.0")
            .is_some(),
        "existing analyze capability must remain registered unchanged"
    );
    assert!(
        capabilities
            .find_exact(LookupScope::PreferPrivate, "doc-approval.extract", "1.0.0")
            .is_none(),
        "the canonical pipeline must not introduce an extract capability"
    );

    Runtime::new(capabilities, DocApprovalExecutor).with_workflow_registry(workflows)
}

fn pipeline_request(request_id: &str) -> WorkflowExecutionRequest {
    WorkflowExecutionRequest {
        kind: "workflow_execution_request".to_string(),
        schema_version: "1.0.0".to_string(),
        request_id: request_id.to_string(),
        workflow_id: "doc-approval.pipeline".to_string(),
        workflow_version: "1.0.0".to_string(),
        scope: WorkflowLookupScope::PreferPrivate,
        input: json!({ "document": "Invoice 2026-071 between Acme Corp and Globex LLC" }),
        governing_spec: "007-workflow-registry-traversal".to_string(),
    }
}

#[test]
fn pipeline_executes_analyze_then_recommend_with_runtime_owned_output() {
    let runtime = doc_approval_runtime();

    let outcome = runtime.execute_workflow(pipeline_request("req-doc-approval-1"));

    assert_eq!(outcome.result.status, WorkflowTraversalStatus::Completed);
    let visited: Vec<&str> = outcome
        .evidence
        .visited_nodes
        .iter()
        .map(|step| step.node_id.as_str())
        .collect();
    assert_eq!(
        visited,
        ["analyze_document", "recommend_document"],
        "the canonical pipeline is exactly analyze then recommend (spec 069 FR-003)"
    );

    let output = outcome
        .result
        .output
        .expect("completed pipeline should produce output");
    assert_eq!(
        output,
        json!({
            "analysis": {
                "docType": "invoice",
                "parties": ["Acme Corp", "Globex LLC"],
                "amounts": ["USD 12000.00"],
                "confidence": "high",
                "recommendation": "review",
            },
            "recommendation": {
                "recommendation": "approve",
                "rationale": "deterministic invoice analysis with high confidence",
                "confidence": "high",
            },
        }),
        "pipeline output is runtime-owned analysis and recommendation only"
    );
}

#[test]
fn pipeline_output_is_deterministic_for_identical_input() {
    let runtime = doc_approval_runtime();

    let first = runtime.execute_workflow(pipeline_request("req-doc-approval-a"));
    let second = runtime.execute_workflow(pipeline_request("req-doc-approval-a"));

    assert_eq!(first.result.status, WorkflowTraversalStatus::Completed);
    assert_eq!(
        first.result.output, second.result.output,
        "same analysis input must produce identical recommendation output"
    );
}
