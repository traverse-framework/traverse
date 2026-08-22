use crate::events::{EventBroker, EventError};
use crate::security::{RuntimeWarning, verify_artifact};
use crate::{
    ExecutionFailureReason, ExecutionFailureState, LocalExecutor, Runtime, RuntimeError,
    RuntimeErrorCode, RuntimeExecutionOutcome, execution_failure_outcome, runtime_error,
    successful_execution_outcome, validate_payload_against_contract,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;
use traverse_contracts::{EventReference, ServiceType};
use traverse_registry::{
    LookupScope, RegistryScope, ResolvedCapability, ResolvedWorkflow, WorkflowEdge,
    WorkflowEdgePredicate, WorkflowEdgeTrigger, WorkflowNode,
};

const WORKFLOW_REQUEST_KIND: &str = "workflow_execution_request";
const WORKFLOW_EVIDENCE_KIND: &str = "workflow_traversal_evidence";
const WORKFLOW_SCHEMA_VERSION: &str = "1.0.0";
const WORKFLOW_GOVERNING_SPEC: &str = "007-workflow-registry-traversal";
/// Maximum events drained per `EventBroker::poll` call while gathering
/// candidate events for waiting event-driven edges (spec 099).
const EVENT_BROKER_POLL_BATCH: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowExecutionRequest {
    pub kind: String,
    pub schema_version: String,
    pub request_id: String,
    pub workflow_id: String,
    pub workflow_version: String,
    pub scope: WorkflowLookupScope,
    pub input: Value,
    pub governing_spec: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowLookupScope {
    PublicOnly,
    PreferPrivate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowTraversalEvidence {
    pub kind: String,
    pub schema_version: String,
    pub trace_id: String,
    pub request_id: String,
    pub workflow_id: String,
    pub workflow_version: String,
    pub governing_spec: String,
    pub visited_nodes: Vec<WorkflowTraversalStepRecord>,
    pub traversed_edges: Vec<WorkflowTraversalEdgeRecord>,
    pub emitted_events: Vec<EventReference>,
    #[serde(default)]
    pub waiting_edges: Vec<WaitingWorkflowEdgeContext>,
    #[serde(default)]
    pub event_match_records: Vec<EventMatchRecord>,
    #[serde(default)]
    pub event_wake_decisions: Vec<EventWakeDecision>,
    #[serde(default)]
    pub event_consumptions: Vec<EventConsumptionRecord>,
    pub result: WorkflowTraversalResult,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowTraversalStepRecord {
    pub step_index: usize,
    pub node_id: String,
    pub capability_id: String,
    pub capability_version: String,
    pub status: WorkflowTraversalStepStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowTraversalStepStatus {
    Entered,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowTraversalEdgeRecord {
    pub edge_id: String,
    pub from: String,
    pub to: String,
    pub trigger: WorkflowTraversalTrigger,
    #[serde(default)]
    pub event: Option<EventReference>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WaitingWorkflowEdgeContext {
    pub workflow_execution_id: String,
    pub edge_id: String,
    pub from_node_id: String,
    pub to_node_id: String,
    pub event_ref: EventReference,
    #[serde(default)]
    pub predicate: Option<WorkflowEdgePredicate>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventMatchRecord {
    pub event_id: String,
    pub event_version: String,
    pub edge_id: String,
    pub match_result: EventMatchResult,
    #[serde(default)]
    pub predicate_result: Option<EventPredicateResult>,
    #[serde(default)]
    pub rejection_reason: Option<String>,
    pub recorded_at: String,
    /// `EventBroker` subscription that sourced this event, `None` when the
    /// event came from the current node's own emitted output rather than
    /// `EventBroker` (spec 099 FR-005).
    #[serde(default)]
    pub subscription_id: Option<String>,
    /// `EventBroker` cursor the event was delivered at, `None` for a
    /// same-node emitted event (spec 099 FR-005).
    #[serde(default)]
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventMatchResult {
    Matched,
    NotMatched,
    AlreadyConsumed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventPredicateResult {
    Passed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventWakeDecision {
    pub decision_type: String,
    pub event_id: String,
    pub event_version: String,
    pub edge_id: String,
    pub workflow_execution_id: String,
    pub wake_order: usize,
    pub result: EventWakeDecisionResult,
    pub recorded_at: String,
    /// `EventBroker` subscription responsible for this wake-up, `None` when
    /// woken by the current node's own emitted output (spec 099 FR-005).
    #[serde(default)]
    pub subscription_id: Option<String>,
    /// `EventBroker` cursor responsible for this wake-up, `None` when woken
    /// by the current node's own emitted output (spec 099 FR-005).
    #[serde(default)]
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventWakeDecisionResult {
    Taken,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventConsumptionRecord {
    pub event_id: String,
    pub event_version: String,
    pub edge_id: String,
    pub workflow_execution_id: String,
    pub consumed_at: String,
    /// `EventBroker` subscription that delivered the consumed event, `None`
    /// when consumed from the current node's own emitted output (spec 099
    /// FR-005).
    #[serde(default)]
    pub subscription_id: Option<String>,
    /// `EventBroker` cursor the consumed event was delivered at, `None` for
    /// a same-node emitted event (spec 099 FR-005).
    #[serde(default)]
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowTraversalTrigger {
    Direct,
    Event,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowTraversalResult {
    pub status: WorkflowTraversalStatus,
    #[serde(default)]
    pub failure_reason: Option<WorkflowTraversalFailureReason>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowTraversalStatus {
    Completed,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowTraversalFailureReason {
    WorkflowNotFound,
    WorkflowInvalid,
    AmbiguousNextEdge,
    MissingRequiredEvent,
    TerminalNodeNotReached,
    StepExecutionFailed,
    /// A waiting edge's declared event type is not registered in
    /// `EventBroker`'s catalog, so a subscription could not be established
    /// (spec 099 FR-007).
    EventSubscriptionFailed,
    /// `EventBroker` was unreachable or internally failing when a waiting
    /// edge attempted to establish or poll its subscription, distinct from
    /// an unregistered event type (spec 099 FR-008).
    EventBrokerUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowExecutionResult {
    pub kind: String,
    pub schema_version: String,
    pub request_id: String,
    pub workflow_id: String,
    pub workflow_version: String,
    pub status: WorkflowTraversalStatus,
    #[serde(default)]
    pub output: Option<Value>,
    #[serde(default)]
    pub error: Option<RuntimeError>,
    #[serde(default)]
    pub warnings: Vec<RuntimeWarning>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowExecutionOutcome {
    pub result: WorkflowExecutionResult,
    pub evidence: WorkflowTraversalEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EmittedEventRecord {
    record_id: String,
    event: EventReference,
    payload: Option<Value>,
    /// `EventBroker` subscription/cursor this event was polled from, `None`
    /// for an event extracted from the current node's own output.
    source: Option<BrokerEventSource>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BrokerEventSource {
    subscription_id: String,
    cursor: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct WorkflowEventEvidenceBundle {
    waiting_edges: Vec<WaitingWorkflowEdgeContext>,
    match_records: Vec<EventMatchRecord>,
    wake_decisions: Vec<EventWakeDecision>,
    consumptions: Vec<EventConsumptionRecord>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct EventDrivenEvaluationOutcome {
    taken_edge_ids: Vec<String>,
    evidence: WorkflowEventEvidenceBundle,
}

impl<E> Runtime<E>
where
    E: LocalExecutor,
{
    #[must_use]
    #[allow(clippy::needless_pass_by_value)]
    pub fn execute_workflow(&self, request: WorkflowExecutionRequest) -> WorkflowExecutionOutcome {
        if let Some(error) = validate_workflow_request(&request) {
            return workflow_failure(
                &request,
                WorkflowTraversalFailureReason::WorkflowInvalid,
                error,
                Vec::new(),
                Vec::new(),
                Vec::new(),
                WorkflowEventEvidenceBundle::default(),
                Vec::new(),
            );
        }

        let lookup_scope = map_workflow_lookup_scope(request.scope);
        let Some(workflow) = self.workflow_registry.find_exact(
            lookup_scope,
            &request.workflow_id,
            &request.workflow_version,
        ) else {
            return workflow_failure(
                &request,
                WorkflowTraversalFailureReason::WorkflowNotFound,
                runtime_error(
                    RuntimeErrorCode::CapabilityNotFound,
                    "workflow definition was not found in the workflow registry",
                    json!({"workflow_id": request.workflow_id, "workflow_version": request.workflow_version}),
                ),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                WorkflowEventEvidenceBundle::default(),
                Vec::new(),
            );
        };

        if let Err(error) = validate_payload_against_contract(
            &request.input,
            &workflow.definition.inputs.schema,
            RuntimeErrorCode::RequestInvalid,
            "workflow request input does not satisfy the workflow input contract",
        ) {
            return workflow_failure(
                &request,
                WorkflowTraversalFailureReason::WorkflowInvalid,
                error,
                Vec::new(),
                Vec::new(),
                Vec::new(),
                WorkflowEventEvidenceBundle::default(),
                Vec::new(),
            );
        }

        match self.traverse_workflow(&request, &workflow) {
            Ok(success) => success,
            Err(failure) => failure,
        }
    }

    pub(crate) fn execute_workflow_capability(
        &self,
        mut context: crate::ExecutionContext,
        selected: &ResolvedCapability,
        started_execution: crate::StartedExecution,
    ) -> RuntimeExecutionOutcome {
        let Some(workflow_ref) = selected.artifact.workflow_ref.as_ref() else {
            let error = runtime_error(
                RuntimeErrorCode::ArtifactMissing,
                "workflow-backed capability is missing its workflow reference",
                json!({"artifact_ref": selected.record.artifact_ref}),
            );
            return execution_failure_outcome(
                context,
                ExecutionFailureState {
                    artifact_ref: selected.record.artifact_ref.clone(),
                    started_at: started_execution.started_at,
                    placement: started_execution.placement.clone(),
                    failure_reason: ExecutionFailureReason::ArtifactMissing,
                },
                error,
                Vec::new(),
                None,
            );
        };

        let workflow_scope = match selected.record.scope {
            RegistryScope::Public => WorkflowLookupScope::PublicOnly,
            RegistryScope::Private => WorkflowLookupScope::PreferPrivate,
        };
        let workflow = self.execute_workflow(WorkflowExecutionRequest {
            kind: WORKFLOW_REQUEST_KIND.to_string(),
            schema_version: WORKFLOW_SCHEMA_VERSION.to_string(),
            request_id: context.attempt.request.request_id.clone(),
            workflow_id: workflow_ref.workflow_id.clone(),
            workflow_version: workflow_ref.workflow_version.clone(),
            scope: workflow_scope,
            input: context.attempt.request.input.clone(),
            governing_spec: WORKFLOW_GOVERNING_SPEC.to_string(),
        });
        context
            .attempt
            .warnings
            .extend(workflow.result.warnings.iter().cloned());

        match workflow.result.status {
            WorkflowTraversalStatus::Completed => {
                let output = workflow.result.output.unwrap_or(Value::Object(Map::new()));
                let workflow_evidence = workflow.evidence;
                let emitted_events = workflow_evidence.emitted_events.clone();
                successful_execution_outcome(
                    context,
                    selected,
                    started_execution,
                    output,
                    emitted_events,
                    Some(workflow_evidence),
                )
            }
            WorkflowTraversalStatus::Error => {
                let workflow_evidence = workflow.evidence;
                let emitted_events = workflow_evidence.emitted_events.clone();
                execution_failure_outcome(
                    context,
                    ExecutionFailureState {
                        artifact_ref: selected.record.artifact_ref.clone(),
                        started_at: started_execution.started_at,
                        placement: started_execution.placement,
                        failure_reason: ExecutionFailureReason::ExecutionFailed,
                    },
                    workflow.result.error.unwrap_or(runtime_error(
                        RuntimeErrorCode::ExecutionFailed,
                        "workflow-backed capability execution failed",
                        json!({}),
                    )),
                    emitted_events,
                    Some(workflow_evidence),
                )
            }
        }
    }

    #[allow(clippy::result_large_err, clippy::too_many_lines)]
    fn traverse_workflow(
        &self,
        request: &WorkflowExecutionRequest,
        workflow: &ResolvedWorkflow,
    ) -> Result<WorkflowExecutionOutcome, WorkflowExecutionOutcome> {
        let mut state = workflow_state(&request.input);
        let mut current = workflow.definition.start_node.clone();
        let mut step_index = 0;
        let mut visited = Vec::new();
        let mut traversed = Vec::new();
        let mut emitted = Vec::new();
        let mut event_evidence = WorkflowEventEvidenceBundle::default();
        let mut consumed_event_edges = BTreeSet::new();
        let mut warnings: Vec<RuntimeWarning> = Vec::new();
        let workflow_execution_id = format!("workflow_exec_{}", request.request_id);

        loop {
            let Some(node) = workflow
                .definition
                .nodes
                .iter()
                .find(|node| node.node_id == current)
            else {
                return Err(workflow_failure(
                    request,
                    WorkflowTraversalFailureReason::WorkflowInvalid,
                    runtime_error(
                        RuntimeErrorCode::ExecutionFailed,
                        "workflow node could not be resolved during traversal",
                        json!({"node_id": current}),
                    ),
                    visited,
                    traversed,
                    emitted,
                    event_evidence,
                    warnings,
                ));
            };

            visited.push(WorkflowTraversalStepRecord {
                step_index,
                node_id: node.node_id.clone(),
                capability_id: node.capability_id.clone(),
                capability_version: node.capability_version.clone(),
                status: WorkflowTraversalStepStatus::Entered,
            });

            let lookup_scope = map_workflow_lookup_scope(request.scope);
            let Some(capability) = self.registry.find_exact(
                lookup_scope,
                &node.capability_id,
                &node.capability_version,
            ) else {
                return Err(workflow_failure(
                    request,
                    WorkflowTraversalFailureReason::WorkflowInvalid,
                    runtime_error(
                        RuntimeErrorCode::CapabilityNotFound,
                        "workflow node capability was not found in the capability registry",
                        json!({"capability_id": node.capability_id, "capability_version": node.capability_version}),
                    ),
                    visited,
                    traversed,
                    emitted,
                    event_evidence,
                    warnings,
                ));
            };

            // Artifact security gate (spec 030-security-identity-model FR-013):
            // workflow steps apply the same verification as direct capability
            // execution, so an artifact rejected by `Runtime::execute` cannot
            // run by being wrapped in a workflow pipeline step (spec 058).
            let artifact_bytes = match crate::load_artifact_bytes_for_verification(&capability) {
                Ok(bytes) => bytes,
                Err(error) => {
                    let mut failed = visited;
                    if let Some(last) = failed.last_mut() {
                        last.status = WorkflowTraversalStepStatus::Failed;
                    }
                    return Err(workflow_failure(
                        request,
                        WorkflowTraversalFailureReason::StepExecutionFailed,
                        error,
                        failed,
                        traversed,
                        emitted,
                        event_evidence,
                        warnings,
                    ));
                }
            };
            match verify_artifact(&capability, &artifact_bytes, &self.security) {
                Ok(record) => {
                    if let Some(code) = record.warning_code {
                        warnings.push(RuntimeWarning {
                            code,
                            message:
                                "unsigned local/dev artifact allowed by development security mode"
                                    .to_string(),
                        });
                    }
                }
                Err(failure) => {
                    let mut failed = visited;
                    if let Some(last) = failed.last_mut() {
                        last.status = WorkflowTraversalStepStatus::Failed;
                    }
                    return Err(workflow_failure(
                        request,
                        WorkflowTraversalFailureReason::StepExecutionFailed,
                        runtime_error(
                            RuntimeErrorCode::ContractViolation,
                            "artifact signature verification failed before workflow step execution",
                            json!({
                                "code": failure.code(),
                                "artifact_verification": failure.record(),
                                "node_id": node.node_id,
                            }),
                        ),
                        failed,
                        traversed,
                        emitted,
                        event_evidence,
                        warnings,
                    ));
                }
            }

            let node_input = node_input(&state, node);
            if let Err(error) = validate_payload_against_contract(
                &node_input,
                &capability.contract.inputs.schema,
                RuntimeErrorCode::RequestInvalid,
                "workflow node input does not satisfy the capability input contract",
            ) {
                let mut failed = visited;
                if let Some(last) = failed.last_mut() {
                    last.status = WorkflowTraversalStepStatus::Failed;
                }
                return Err(workflow_failure(
                    request,
                    WorkflowTraversalFailureReason::StepExecutionFailed,
                    error,
                    failed,
                    traversed,
                    emitted,
                    event_evidence,
                    warnings,
                ));
            }

            let output = match self.executor.execute(&capability, &node_input) {
                Ok(output) => output,
                Err(failure) => {
                    let mut failed = visited;
                    if let Some(last) = failed.last_mut() {
                        last.status = WorkflowTraversalStepStatus::Failed;
                    }
                    return Err(workflow_failure(
                        request,
                        WorkflowTraversalFailureReason::StepExecutionFailed,
                        runtime_error(
                            RuntimeErrorCode::ExecutionFailed,
                            &failure.message,
                            json!({"code": format!("{:?}", failure.code)}),
                        ),
                        failed,
                        traversed,
                        emitted,
                        event_evidence,
                        warnings,
                    ));
                }
            };

            if let Err(error) = validate_payload_against_contract(
                &output.value,
                &capability.contract.outputs.schema,
                RuntimeErrorCode::OutputValidationFailed,
                "workflow node output does not satisfy the capability output contract",
            ) {
                let mut failed = visited;
                if let Some(last) = failed.last_mut() {
                    last.status = WorkflowTraversalStepStatus::Failed;
                }
                return Err(workflow_failure(
                    request,
                    WorkflowTraversalFailureReason::StepExecutionFailed,
                    error,
                    failed,
                    traversed,
                    emitted,
                    event_evidence,
                    warnings,
                ));
            }

            // Spec 101-local-executor-event-emission FR-007/FR-008: validate
            // natively-populated events before using them for anything,
            // mirroring spec 098's WASM-boundary checks.
            if let Err(validation_message) = crate::validate_natively_emitted_events(
                &capability.contract,
                &output.emitted_events,
            ) {
                let mut failed = visited;
                if let Some(last) = failed.last_mut() {
                    last.status = WorkflowTraversalStepStatus::Failed;
                }
                return Err(workflow_failure(
                    request,
                    WorkflowTraversalFailureReason::StepExecutionFailed,
                    runtime_error(
                        RuntimeErrorCode::ContractViolation,
                        &validation_message,
                        json!({"node_id": node.node_id}),
                    ),
                    failed,
                    traversed,
                    emitted,
                    event_evidence,
                    warnings,
                ));
            }

            update_state(&mut state, node, &output.value);

            // Spec 101-local-executor-event-emission FR-005: publish to
            // EventBroker for Subscribable capabilities, structurally
            // analogous to PlacementRouter Step 5 (same gate, same
            // best-effort semantics — a publish error is recorded but does
            // not fail the workflow step).
            if capability.contract.service_type == ServiceType::Subscribable {
                for event in &output.emitted_events {
                    let _ = self.event_broker.publish(event.clone());
                }
            }

            // Spec 101-local-executor-event-emission FR-006: Pass-1
            // event-driven edge matching reads from the structured
            // `LocalExecutionOutput.emitted_events` field, not a
            // JSON-parsed "emitted_events" convention key.
            let node_emitted: Vec<EmittedEventRecord> = output
                .emitted_events
                .iter()
                .enumerate()
                .map(|(index, event)| EmittedEventRecord {
                    record_id: format!("event_record_{index}"),
                    event: EventReference {
                        event_id: event.event_type.clone(),
                        version: event.version.clone(),
                    },
                    payload: Some(event.data.clone()),
                    source: None,
                })
                .collect();
            emitted.extend(node_emitted.iter().map(|record| record.event.clone()));
            if let Some(last) = visited.last_mut() {
                last.status = WorkflowTraversalStepStatus::Completed;
            }

            let outgoing = workflow
                .definition
                .edges
                .iter()
                .filter(|edge| edge.from == node.node_id)
                .cloned()
                .collect::<Vec<_>>();
            let direct = outgoing
                .iter()
                .filter(|edge| edge.trigger == WorkflowEdgeTrigger::Direct)
                .cloned()
                .collect::<Vec<_>>();
            if direct.len() > 1 {
                return Err(workflow_failure(
                    request,
                    WorkflowTraversalFailureReason::AmbiguousNextEdge,
                    runtime_error(
                        RuntimeErrorCode::ExecutionFailed,
                        "workflow traversal found more than one direct next edge",
                        json!({"node_id": node.node_id}),
                    ),
                    visited,
                    traversed,
                    emitted,
                    event_evidence,
                    warnings,
                ));
            }
            if let Some(edge) = direct.into_iter().next() {
                traversed.push(edge_record(&edge));
                current = edge.to;
                step_index += 1;
                continue;
            }

            let waiting_edges = waiting_edge_contexts(
                &workflow_execution_id,
                outgoing
                    .iter()
                    .filter(|edge| edge.trigger == WorkflowEdgeTrigger::Event)
                    .cloned()
                    .collect::<Vec<_>>()
                    .as_slice(),
            );
            if !waiting_edges.is_empty() {
                event_evidence.waiting_edges.extend(waiting_edges.clone());
            }
            // Pass 1: match against the current node's own emitted output,
            // exactly as before spec 099 (cheapest path, and the only one
            // that doesn't need a broker at all).
            let local_evaluation = evaluate_event_driven_edges(
                &waiting_edges,
                &node_emitted,
                &mut consumed_event_edges,
                &format!("{}:step:{step_index}:local", request.request_id),
            );
            event_evidence
                .match_records
                .extend(local_evaluation.evidence.match_records.iter().cloned());
            event_evidence
                .wake_decisions
                .extend(local_evaluation.evidence.wake_decisions.iter().cloned());
            event_evidence
                .consumptions
                .extend(local_evaluation.evidence.consumptions.iter().cloned());
            let mut taken_edge_ids = local_evaluation.taken_edge_ids;

            // Pass 2 (spec 099 FR-001/FR-002): for edges the node's own
            // output didn't satisfy, poll EventBroker for events published
            // by any other execution, capability, or external publisher.
            // Edges already resolved locally never touch the broker, so an
            // unregistered/unreachable broker cannot fail a workflow whose
            // event-driven edges are still fully satisfiable from the
            // node's own output.
            let unresolved_edges: Vec<WaitingWorkflowEdgeContext> = waiting_edges
                .iter()
                .filter(|edge| !taken_edge_ids.contains(&edge.edge_id))
                .cloned()
                .collect();
            if !unresolved_edges.is_empty() {
                let broker = self.event_broker.as_ref();
                match poll_broker_events_for_waiting_edges(broker, &unresolved_edges) {
                    Ok((broker_events, broker_warnings)) => {
                        warnings.extend(broker_warnings);
                        let broker_evaluation = evaluate_event_driven_edges(
                            &unresolved_edges,
                            &broker_events,
                            &mut consumed_event_edges,
                            &format!("{}:step:{step_index}:broker", request.request_id),
                        );
                        event_evidence
                            .match_records
                            .extend(broker_evaluation.evidence.match_records.iter().cloned());
                        event_evidence
                            .wake_decisions
                            .extend(broker_evaluation.evidence.wake_decisions.iter().cloned());
                        event_evidence
                            .consumptions
                            .extend(broker_evaluation.evidence.consumptions.iter().cloned());
                        taken_edge_ids.extend(broker_evaluation.taken_edge_ids);
                    }
                    Err(BrokerQueryFailure::UnregisteredEventType(event_type)) => {
                        let mut failed = visited;
                        if let Some(last) = failed.last_mut() {
                            last.status = WorkflowTraversalStepStatus::Failed;
                        }
                        return Err(workflow_failure(
                            request,
                            WorkflowTraversalFailureReason::EventSubscriptionFailed,
                            runtime_error(
                                RuntimeErrorCode::RequestInvalid,
                                "waiting edge references an event type not registered in EventBroker's catalog",
                                json!({"node_id": node.node_id, "event_type": event_type}),
                            ),
                            failed,
                            traversed,
                            emitted,
                            event_evidence,
                            warnings,
                        ));
                    }
                    Err(BrokerQueryFailure::BrokerUnavailable(detail)) => {
                        let mut failed = visited;
                        if let Some(last) = failed.last_mut() {
                            last.status = WorkflowTraversalStepStatus::Failed;
                        }
                        return Err(workflow_failure(
                            request,
                            WorkflowTraversalFailureReason::EventBrokerUnavailable,
                            runtime_error(
                                RuntimeErrorCode::ExecutionFailed,
                                "EventBroker was unreachable or internally failing while evaluating a waiting event-driven edge",
                                json!({"node_id": node.node_id, "detail": detail}),
                            ),
                            failed,
                            traversed,
                            emitted,
                            event_evidence,
                            warnings,
                        ));
                    }
                }
            }
            let matched_event_edges = outgoing
                .iter()
                .filter(|edge| {
                    taken_edge_ids
                        .iter()
                        .any(|edge_id| edge_id == &edge.edge_id)
                })
                .cloned()
                .collect::<Vec<_>>();
            if matched_event_edges.len() > 1 {
                return Err(workflow_failure(
                    request,
                    WorkflowTraversalFailureReason::AmbiguousNextEdge,
                    runtime_error(
                        RuntimeErrorCode::ExecutionFailed,
                        "workflow traversal found more than one event next edge",
                        json!({"node_id": node.node_id}),
                    ),
                    visited,
                    traversed,
                    emitted,
                    event_evidence,
                    warnings,
                ));
            }
            if let Some(edge) = matched_event_edges.into_iter().next() {
                traversed.push(edge_record(&edge));
                current = edge.to;
                step_index += 1;
                continue;
            }

            if workflow.definition.terminal_nodes.contains(&node.node_id) {
                let final_output =
                    final_workflow_output(&state, &workflow.definition.output_projection);
                if let Err(error) = validate_payload_against_contract(
                    &final_output,
                    &workflow.definition.outputs.schema,
                    RuntimeErrorCode::OutputValidationFailed,
                    "workflow output does not satisfy the workflow output contract",
                ) {
                    return Err(workflow_failure(
                        request,
                        WorkflowTraversalFailureReason::WorkflowInvalid,
                        error,
                        visited,
                        traversed,
                        emitted,
                        event_evidence,
                        warnings,
                    ));
                }

                let evidence = WorkflowTraversalEvidence {
                    kind: WORKFLOW_EVIDENCE_KIND.to_string(),
                    schema_version: WORKFLOW_SCHEMA_VERSION.to_string(),
                    trace_id: format!("workflow_trace_{}", request.request_id),
                    request_id: request.request_id.clone(),
                    workflow_id: workflow.definition.id.clone(),
                    workflow_version: workflow.definition.version.clone(),
                    governing_spec: WORKFLOW_GOVERNING_SPEC.to_string(),
                    visited_nodes: visited,
                    traversed_edges: traversed,
                    emitted_events: emitted,
                    waiting_edges: event_evidence.waiting_edges,
                    event_match_records: event_evidence.match_records,
                    event_wake_decisions: event_evidence.wake_decisions,
                    event_consumptions: event_evidence.consumptions,
                    result: WorkflowTraversalResult {
                        status: WorkflowTraversalStatus::Completed,
                        failure_reason: None,
                    },
                };

                return Ok(WorkflowExecutionOutcome {
                    result: WorkflowExecutionResult {
                        kind: WORKFLOW_REQUEST_KIND.to_string(),
                        schema_version: WORKFLOW_SCHEMA_VERSION.to_string(),
                        request_id: request.request_id.clone(),
                        workflow_id: workflow.definition.id.clone(),
                        workflow_version: workflow.definition.version.clone(),
                        status: WorkflowTraversalStatus::Completed,
                        output: Some(final_output),
                        error: None,
                        warnings,
                    },
                    evidence,
                });
            }

            let failure_reason = if outgoing
                .iter()
                .any(|edge| edge.trigger == WorkflowEdgeTrigger::Event)
            {
                WorkflowTraversalFailureReason::MissingRequiredEvent
            } else {
                WorkflowTraversalFailureReason::TerminalNodeNotReached
            };

            return Err(workflow_failure(
                request,
                failure_reason,
                runtime_error(
                    RuntimeErrorCode::ExecutionFailed,
                    "workflow traversal could not reach a valid next node",
                    json!({"node_id": node.node_id}),
                ),
                visited,
                traversed,
                emitted,
                event_evidence,
                warnings,
            ));
        }
    }
}

fn validate_workflow_request(request: &WorkflowExecutionRequest) -> Option<RuntimeError> {
    if request.kind != WORKFLOW_REQUEST_KIND {
        return Some(runtime_error(
            RuntimeErrorCode::RequestInvalid,
            "kind must equal workflow_execution_request",
            json!({"path": "$.kind"}),
        ));
    }
    if request.schema_version != WORKFLOW_SCHEMA_VERSION {
        return Some(runtime_error(
            RuntimeErrorCode::RequestInvalid,
            "schema_version must equal 1.0.0",
            json!({"path": "$.schema_version"}),
        ));
    }
    if request.governing_spec != WORKFLOW_GOVERNING_SPEC {
        return Some(runtime_error(
            RuntimeErrorCode::RequestInvalid,
            "governing_spec must equal 007-workflow-registry-traversal",
            json!({"path": "$.governing_spec"}),
        ));
    }
    if request.request_id.trim().is_empty()
        || request.workflow_id.trim().is_empty()
        || request.workflow_version.trim().is_empty()
    {
        return Some(runtime_error(
            RuntimeErrorCode::RequestInvalid,
            "request_id, workflow_id, and workflow_version must be non-empty",
            json!({"path": "$"}),
        ));
    }
    None
}

fn map_workflow_lookup_scope(scope: WorkflowLookupScope) -> LookupScope {
    match scope {
        WorkflowLookupScope::PublicOnly => LookupScope::PublicOnly,
        WorkflowLookupScope::PreferPrivate => LookupScope::PreferPrivate,
    }
}

fn workflow_state(input: &Value) -> Map<String, Value> {
    match input {
        Value::Object(map) => map.clone(),
        other => {
            let mut map = Map::new();
            map.insert("input".to_string(), other.clone());
            map
        }
    }
}

fn node_input(state: &Map<String, Value>, node: &WorkflowNode) -> Value {
    let mut input = Map::new();
    for key in &node.input.from_workflow_input {
        if let Some(value) = state.get(key) {
            input.insert(key.clone(), value.clone());
        }
    }
    Value::Object(input)
}

fn update_state(state: &mut Map<String, Value>, node: &WorkflowNode, output: &Value) {
    let Value::Object(object) = output else {
        return;
    };
    for key in &node.output.to_workflow_state {
        if let Some(value) = object.get(key) {
            state.insert(key.clone(), value.clone());
        }
    }
    if let Some(namespace) = &node.output.publish_to_state_as {
        state.insert(namespace.clone(), output.clone());
    }
}

fn final_workflow_output(state: &Map<String, Value>, output_projection: &[String]) -> Value {
    if output_projection.is_empty() {
        return Value::Object(state.clone());
    }
    let mut projected = Map::new();
    for key in output_projection {
        if let Some(value) = state.get(key) {
            projected.insert(key.clone(), value.clone());
        }
    }
    Value::Object(projected)
}

fn waiting_edge_contexts(
    workflow_execution_id: &str,
    edges: &[WorkflowEdge],
) -> Vec<WaitingWorkflowEdgeContext> {
    edges
        .iter()
        .filter_map(|edge| {
            Some(WaitingWorkflowEdgeContext {
                workflow_execution_id: workflow_execution_id.to_string(),
                edge_id: edge.edge_id.clone(),
                from_node_id: edge.from.clone(),
                to_node_id: edge.to.clone(),
                event_ref: edge.event.clone()?,
                predicate: edge.predicate.clone(),
            })
        })
        .collect()
}

/// A waiting edge's subscription to `EventBroker` could not be established
/// or polled (spec 099 FR-007/FR-008).
#[derive(Debug, Clone, PartialEq, Eq)]
enum BrokerQueryFailure {
    /// The declared event type is not registered in `EventBroker`'s catalog
    /// (FR-007).
    UnregisteredEventType(String),
    /// `EventBroker` was unreachable or internally failing (FR-008).
    BrokerUnavailable(String),
}

/// Subscribes to `EventBroker` for every distinct event type referenced by
/// `waiting_edges`, drains all currently available events for each, and
/// cancels the subscription immediately after (spec 099 FR-001/FR-002).
///
/// Event types are visited in a fixed, sorted order, and each type's events
/// are appended in delivery order, so the resulting candidate pool is
/// deterministic and explainable (FR-003).
fn poll_broker_events_for_waiting_edges(
    broker: &dyn EventBroker,
    waiting_edges: &[WaitingWorkflowEdgeContext],
) -> Result<(Vec<EmittedEventRecord>, Vec<RuntimeWarning>), BrokerQueryFailure> {
    let mut event_types: Vec<&str> = waiting_edges
        .iter()
        .map(|edge| edge.event_ref.event_id.as_str())
        .collect();
    event_types.sort_unstable();
    event_types.dedup();

    let mut records = Vec::new();
    let mut warnings = Vec::new();

    for event_type in event_types {
        let subscription = broker
            .subscribe(event_type, "0")
            .map_err(|error| match error {
                EventError::UnregisteredEventType(event_type) => {
                    BrokerQueryFailure::UnregisteredEventType(event_type)
                }
                other => BrokerQueryFailure::BrokerUnavailable(other.to_string()),
            })?;

        let mut delivered = Vec::new();
        let poll_failure = loop {
            match broker.poll(&subscription.subscription_id, EVENT_BROKER_POLL_BATCH) {
                Ok(batch) => {
                    let batch_len = batch.events.len();
                    delivered.extend(batch.events);
                    if batch_len < EVENT_BROKER_POLL_BATCH {
                        break None;
                    }
                }
                Err(error) => break Some(BrokerQueryFailure::BrokerUnavailable(error.to_string())),
            }
        };

        if let Err(cancel_error) = broker.cancel(&subscription.subscription_id) {
            warnings.push(RuntimeWarning {
                code: "event_broker_subscription_cleanup_failed".to_string(),
                message: format!(
                    "failed to cancel workflow event-driven edge subscription {}: {cancel_error}",
                    subscription.subscription_id
                ),
            });
        }

        if let Some(failure) = poll_failure {
            return Err(failure);
        }

        for broker_event in delivered {
            records.push(EmittedEventRecord {
                record_id: format!("broker:{event_type}:{}", broker_event.cursor),
                event: EventReference {
                    event_id: event_type.to_string(),
                    version: broker_event.event.version.clone(),
                },
                payload: Some(broker_event.event.data.clone()),
                source: Some(BrokerEventSource {
                    subscription_id: subscription.subscription_id.clone(),
                    cursor: broker_event.cursor.clone(),
                }),
            });
        }
    }

    Ok((records, warnings))
}

#[allow(clippy::too_many_lines)]
fn evaluate_event_driven_edges(
    waiting_edges: &[WaitingWorkflowEdgeContext],
    emitted_events: &[EmittedEventRecord],
    consumed_event_edges: &mut BTreeSet<String>,
    record_prefix: &str,
) -> EventDrivenEvaluationOutcome {
    let mut ordered_waiting_edges = waiting_edges.to_vec();
    ordered_waiting_edges.sort_by(|left, right| {
        left.workflow_execution_id
            .cmp(&right.workflow_execution_id)
            .then_with(|| left.edge_id.cmp(&right.edge_id))
    });

    let mut outcome = EventDrivenEvaluationOutcome::default();
    if emitted_events.is_empty() {
        outcome
            .evidence
            .match_records
            .extend(ordered_waiting_edges.iter().map(|edge| EventMatchRecord {
                event_id: edge.event_ref.event_id.clone(),
                event_version: edge.event_ref.version.clone(),
                edge_id: edge.edge_id.clone(),
                match_result: EventMatchResult::NotMatched,
                predicate_result: None,
                rejection_reason: Some("required event was not emitted".to_string()),
                recorded_at: format!("{record_prefix}:no_event:{}", edge.edge_id),
                subscription_id: None,
                cursor: None,
            }));
        return outcome;
    }

    let mut wake_order = 1;
    for (event_index, emitted_event) in emitted_events.iter().enumerate() {
        let subscription_id = emitted_event
            .source
            .as_ref()
            .map(|source| source.subscription_id.clone());
        let cursor = emitted_event
            .source
            .as_ref()
            .map(|source| source.cursor.clone());
        for waiting_edge in &ordered_waiting_edges {
            let match_recorded_at = format!(
                "{record_prefix}:event:{event_index}:match:{}",
                waiting_edge.edge_id
            );
            if emitted_event.event != waiting_edge.event_ref {
                outcome.evidence.match_records.push(EventMatchRecord {
                    event_id: emitted_event.event.event_id.clone(),
                    event_version: emitted_event.event.version.clone(),
                    edge_id: waiting_edge.edge_id.clone(),
                    match_result: EventMatchResult::NotMatched,
                    predicate_result: None,
                    rejection_reason: Some(
                        "event id/version did not match the waiting edge".to_string(),
                    ),
                    recorded_at: match_recorded_at,
                    subscription_id: subscription_id.clone(),
                    cursor: cursor.clone(),
                });
                continue;
            }

            if let Some(predicate) = waiting_edge.predicate.as_ref() {
                let predicate_passed =
                    event_payload_field(emitted_event.payload.as_ref(), &predicate.field)
                        .is_some_and(|value| value == &predicate.equals);
                if !predicate_passed {
                    outcome.evidence.match_records.push(EventMatchRecord {
                        event_id: emitted_event.event.event_id.clone(),
                        event_version: emitted_event.event.version.clone(),
                        edge_id: waiting_edge.edge_id.clone(),
                        match_result: EventMatchResult::NotMatched,
                        predicate_result: Some(EventPredicateResult::Failed),
                        rejection_reason: Some(
                            "event predicate did not match the emitted payload".to_string(),
                        ),
                        recorded_at: match_recorded_at,
                        subscription_id: subscription_id.clone(),
                        cursor: cursor.clone(),
                    });
                    continue;
                }
            }

            let consumption_key = format!(
                "{}|{}|{}",
                emitted_event.record_id, waiting_edge.workflow_execution_id, waiting_edge.edge_id
            );
            if consumed_event_edges.contains(&consumption_key) {
                outcome.evidence.match_records.push(EventMatchRecord {
                    event_id: emitted_event.event.event_id.clone(),
                    event_version: emitted_event.event.version.clone(),
                    edge_id: waiting_edge.edge_id.clone(),
                    match_result: EventMatchResult::AlreadyConsumed,
                    predicate_result: waiting_edge
                        .predicate
                        .as_ref()
                        .map(|_| EventPredicateResult::Passed),
                    rejection_reason: Some(
                        "event record was already consumed for this waiting edge".to_string(),
                    ),
                    recorded_at: match_recorded_at,
                    subscription_id: subscription_id.clone(),
                    cursor: cursor.clone(),
                });
                continue;
            }

            consumed_event_edges.insert(consumption_key);
            outcome.evidence.match_records.push(EventMatchRecord {
                event_id: emitted_event.event.event_id.clone(),
                event_version: emitted_event.event.version.clone(),
                edge_id: waiting_edge.edge_id.clone(),
                match_result: EventMatchResult::Matched,
                predicate_result: waiting_edge
                    .predicate
                    .as_ref()
                    .map(|_| EventPredicateResult::Passed),
                rejection_reason: None,
                recorded_at: match_recorded_at.clone(),
                subscription_id: subscription_id.clone(),
                cursor: cursor.clone(),
            });
            outcome.taken_edge_ids.push(waiting_edge.edge_id.clone());
            let wake_recorded_at = format!(
                "{record_prefix}:event:{event_index}:wake:{}",
                waiting_edge.edge_id
            );
            outcome.evidence.wake_decisions.push(EventWakeDecision {
                decision_type: "event_wake".to_string(),
                event_id: emitted_event.event.event_id.clone(),
                event_version: emitted_event.event.version.clone(),
                edge_id: waiting_edge.edge_id.clone(),
                workflow_execution_id: waiting_edge.workflow_execution_id.clone(),
                wake_order,
                result: EventWakeDecisionResult::Taken,
                recorded_at: wake_recorded_at.clone(),
                subscription_id: subscription_id.clone(),
                cursor: cursor.clone(),
            });
            outcome.evidence.consumptions.push(EventConsumptionRecord {
                event_id: emitted_event.event.event_id.clone(),
                event_version: emitted_event.event.version.clone(),
                edge_id: waiting_edge.edge_id.clone(),
                workflow_execution_id: waiting_edge.workflow_execution_id.clone(),
                consumed_at: wake_recorded_at,
                subscription_id: subscription_id.clone(),
                cursor: cursor.clone(),
            });
            wake_order += 1;
        }
    }
    outcome
}

fn event_payload_field<'a>(payload: Option<&'a Value>, field: &str) -> Option<&'a Value> {
    let payload = payload?;
    let path = field.strip_prefix("payload.").unwrap_or(field);
    if path == "payload" || path.is_empty() {
        return Some(payload);
    }

    let mut current = payload;
    for segment in path.split('.') {
        let Value::Object(map) = current else {
            return None;
        };
        current = map.get(segment)?;
    }
    Some(current)
}

fn edge_record(edge: &WorkflowEdge) -> WorkflowTraversalEdgeRecord {
    WorkflowTraversalEdgeRecord {
        edge_id: edge.edge_id.clone(),
        from: edge.from.clone(),
        to: edge.to.clone(),
        trigger: match edge.trigger {
            WorkflowEdgeTrigger::Direct => WorkflowTraversalTrigger::Direct,
            WorkflowEdgeTrigger::Event => WorkflowTraversalTrigger::Event,
        },
        event: edge.event.clone(),
    }
}

#[allow(clippy::too_many_arguments)]
fn workflow_failure(
    request: &WorkflowExecutionRequest,
    failure_reason: WorkflowTraversalFailureReason,
    error: RuntimeError,
    visited_nodes: Vec<WorkflowTraversalStepRecord>,
    traversed_edges: Vec<WorkflowTraversalEdgeRecord>,
    emitted_events: Vec<EventReference>,
    event_evidence: WorkflowEventEvidenceBundle,
    warnings: Vec<RuntimeWarning>,
) -> WorkflowExecutionOutcome {
    let evidence = WorkflowTraversalEvidence {
        kind: WORKFLOW_EVIDENCE_KIND.to_string(),
        schema_version: WORKFLOW_SCHEMA_VERSION.to_string(),
        trace_id: format!("workflow_trace_{}", request.request_id),
        request_id: request.request_id.clone(),
        workflow_id: request.workflow_id.clone(),
        workflow_version: request.workflow_version.clone(),
        governing_spec: WORKFLOW_GOVERNING_SPEC.to_string(),
        visited_nodes,
        traversed_edges,
        emitted_events,
        waiting_edges: event_evidence.waiting_edges,
        event_match_records: event_evidence.match_records,
        event_wake_decisions: event_evidence.wake_decisions,
        event_consumptions: event_evidence.consumptions,
        result: WorkflowTraversalResult {
            status: WorkflowTraversalStatus::Error,
            failure_reason: Some(failure_reason),
        },
    };

    WorkflowExecutionOutcome {
        result: WorkflowExecutionResult {
            kind: WORKFLOW_REQUEST_KIND.to_string(),
            schema_version: WORKFLOW_SCHEMA_VERSION.to_string(),
            request_id: request.request_id.clone(),
            workflow_id: request.workflow_id.clone(),
            workflow_version: request.workflow_version.clone(),
            status: WorkflowTraversalStatus::Error,
            output: None,
            error: Some(error),
            warnings,
        },
        evidence,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events;
    use crate::events::InProcessBroker;
    use crate::security::RuntimeSecurityConfig;
    use crate::{
        CandidateCollectionRecord, LocalExecutionFailure, LocalExecutionFailureCode,
        LocalExecutionOutput, RuntimeContext, RuntimeIntent, RuntimeLookup, RuntimeLookupScope,
        RuntimeRequest, RuntimeResultStatus, SelectionRecord,
    };
    use serde_json::json;
    use std::sync::Arc;
    use traverse_contracts::{
        BinaryFormat as ContractBinaryFormat, CapabilityContract, Condition, Entrypoint,
        EntrypointKind, EventReference, EvidenceStatus, EvidenceType, Execution,
        ExecutionConstraints, ExecutionTarget, FilesystemAccess, HostApiAccess, IdReference,
        Lifecycle, NetworkAccess, Owner, Provenance, ProvenanceSource, SchemaContainer,
        ServiceType, SideEffect, SideEffectKind, ValidationEvidence,
    };
    use traverse_registry::{
        ArtifactDigests, ArtifactSignature, ArtifactSignatureScheme, BinaryFormat, BinaryReference,
        CapabilityArtifactRecord, CapabilityRegistration, CapabilityRegistry,
        ComposabilityMetadata, CompositionKind, CompositionPattern, ImplementationKind,
        RegistryProvenance, RegistryScope, SourceKind, SourceReference, WorkflowDefinition,
        WorkflowEdge, WorkflowEdgeTrigger, WorkflowNode, WorkflowNodeInput, WorkflowNodeOutput,
        WorkflowRegistration, WorkflowRegistry, WorkflowRegistryRecord, workflow_artifact_record,
    };

    #[test]
    fn workflow_request_validation_rejects_invalid_guards() {
        let mut request = valid_workflow_request();
        request.kind = "bad".to_string();
        assert_eq!(
            validate_workflow_request(&request).map(|error| error.code),
            Some(RuntimeErrorCode::RequestInvalid)
        );

        let mut request = valid_workflow_request();
        request.schema_version = "2.0.0".to_string();
        assert_eq!(
            validate_workflow_request(&request).map(|error| error.code),
            Some(RuntimeErrorCode::RequestInvalid)
        );

        let mut request = valid_workflow_request();
        request.governing_spec = "bad".to_string();
        assert_eq!(
            validate_workflow_request(&request).map(|error| error.code),
            Some(RuntimeErrorCode::RequestInvalid)
        );

        let mut request = valid_workflow_request();
        request.request_id.clear();
        assert_eq!(
            validate_workflow_request(&request).map(|error| error.code),
            Some(RuntimeErrorCode::RequestInvalid)
        );
    }

    #[test]
    fn workflow_helpers_cover_state_and_edge_paths() {
        let scalar = workflow_state(&json!("value"));
        assert_eq!(scalar.get("input"), Some(&json!("value")));

        let mut state = workflow_state(&json!({"comment_text": "hello"}));
        let node = WorkflowNode {
            node_id: "node".to_string(),
            capability_id: "content.comments.create-comment-draft".to_string(),
            capability_version: "1.0.0".to_string(),
            input: WorkflowNodeInput {
                from_workflow_input: vec!["comment_text".to_string(), "missing".to_string()],
            },
            output: WorkflowNodeOutput {
                to_workflow_state: vec!["draft_id".to_string()],
                publish_to_state_as: None,
            },
        };
        assert_eq!(node_input(&state, &node), json!({"comment_text": "hello"}));
        update_state(&mut state, &node, &json!({"draft_id": "draft-1"}));
        assert_eq!(state.get("draft_id"), Some(&json!("draft-1")));
        update_state(&mut state, &node, &json!("not-an-object"));

        let edge = WorkflowEdge {
            edge_id: "edge".to_string(),
            from: "a".to_string(),
            to: "b".to_string(),
            trigger: WorkflowEdgeTrigger::Event,
            event: Some(EventReference {
                event_id: "content.comments.draft-created".to_string(),
                version: "1.0.0".to_string(),
            }),
            predicate: None,
        };
        assert_eq!(edge_record(&edge).trigger, WorkflowTraversalTrigger::Event);
        assert_eq!(
            map_workflow_lookup_scope(WorkflowLookupScope::PreferPrivate),
            LookupScope::PreferPrivate
        );
        assert_eq!(
            event_payload_field(Some(&json!({"severity": "normal"})), "payload.severity"),
            Some(&json!("normal"))
        );
        assert_eq!(
            event_payload_field(Some(&json!({"severity": "normal"})), "payload"),
            Some(&json!({"severity": "normal"}))
        );
        assert_eq!(
            event_payload_field(Some(&json!("normal")), "payload.severity"),
            None
        );
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn executes_workflow_deterministically_and_supports_workflow_backed_capabilities() {
        let workflow_registry = workflow_registry_fixture();
        let runtime = Runtime::new(capability_registry_fixture(), WorkflowExecutor)
            .with_workflow_registry(workflow_registry)
            .with_security_config(RuntimeSecurityConfig::development());

        let workflow = runtime.execute_workflow(valid_workflow_request());
        assert_eq!(workflow.result.status, WorkflowTraversalStatus::Completed);
        assert_eq!(
            workflow.result.output,
            Some(
                json!({"comment_text": "hello", "draft_id": "draft-1", "comment_id": "comment-1"})
            )
        );
        assert_eq!(workflow.evidence.visited_nodes.len(), 3);
        assert_eq!(workflow.evidence.traversed_edges.len(), 2);
        assert_eq!(workflow.evidence.waiting_edges.len(), 2);
        assert_eq!(workflow.evidence.event_wake_decisions.len(), 2);
        assert_eq!(workflow.evidence.event_consumptions.len(), 2);
        assert!(
            workflow
                .evidence
                .event_match_records
                .iter()
                .all(|record| record.match_result == EventMatchResult::Matched)
        );

        let mut composed_registry = capability_registry_fixture();
        register_capability_ok(
            &mut composed_registry,
            CapabilityRegistration {
                scope: RegistryScope::Public,
                contract: capability_contract(
                    "content.comments.publish-comment",
                    vec![],
                    json!({
                        "type": "object",
                        "properties": { "comment_text": { "type": "string" } },
                        "required": ["comment_text"],
                        "additionalProperties": true
                    }),
                    json!({
                        "type": "object",
                        "properties": { "comment_id": { "type": "string" } },
                        "required": ["comment_id"],
                        "additionalProperties": true
                    }),
                ),
                contract_path: "contracts/publish-comment.json".to_string(),
                artifact: workflow_artifact_record(
                    "content.comments.publish-comment",
                    "1.0.0",
                    "artifact-workflow",
                ),
                registered_at: "2026-03-27T00:10:00Z".to_string(),
                tags: vec!["comments".to_string()],
                composability: ComposabilityMetadata {
                    kind: CompositionKind::Composite,
                    patterns: vec![
                        CompositionPattern::Sequential,
                        CompositionPattern::EventDriven,
                    ],
                    provides: vec!["published-comment".to_string()],
                    requires: vec!["draft".to_string()],
                },
                governing_spec: "005-capability-registry".to_string(),
                validator_version: "validator".to_string(),
            },
        );

        let runtime = Runtime::new(composed_registry, WorkflowExecutor)
            .with_workflow_registry(workflow_registry_fixture())
            .with_security_config(RuntimeSecurityConfig::development());
        let result = runtime.execute(RuntimeRequest {
            kind: "runtime_request".to_string(),
            schema_version: "1.0.0".to_string(),
            request_id: "request-workflow".to_string(),
            intent: RuntimeIntent {
                capability_id: Some("content.comments.publish-comment".to_string()),
                capability_version: Some("1.0.0".to_string()),
                version_range: None,
                intent_key: None,
            },
            input: json!({"comment_text": "hello"}),
            lookup: RuntimeLookup {
                scope: RuntimeLookupScope::PublicOnly,
                allow_ambiguity: false,
            },
            context: RuntimeContext {
                requested_target: crate::PlacementTarget::Local,
                correlation_id: None,
                caller: None,
                traceparent: None,
                tracestate: None,
                metadata: None,
                identity: None,
            },
            governing_spec: "006-runtime-request-execution".to_string(),
        });
        assert_eq!(result.result.status, RuntimeResultStatus::Completed);
        assert_eq!(
            result.result.output,
            Some(
                json!({"comment_text": "hello", "draft_id": "draft-1", "comment_id": "comment-1"})
            )
        );
        assert!(
            result
                .result
                .warnings
                .iter()
                .any(|warning| warning.code == "unsigned_local_dev_artifact")
        );
    }

    /// Spec 101-local-executor-event-emission FR-005 / acceptance scenario 2:
    /// a workflow node's declared, `Subscribable`-gated event both satisfies
    /// same-execution waiting edges (already covered by
    /// `executes_workflow_deterministically_and_supports_workflow_backed_capabilities`)
    /// AND is published to `EventBroker` for external consumers, mirroring
    /// `PlacementRouter` Step 5's publish semantics.
    #[test]
    fn workflow_node_emitted_events_are_published_to_event_broker() {
        let broker = event_catalog_broker_fixture("content.comments.draft-created", "1.0.0");
        let subscription = broker
            .subscribe("content.comments.draft-created", "0")
            .unwrap_or_else(|error| unreachable!("{error:?}"));

        let workflow_registry = workflow_registry_fixture();
        let runtime = Runtime::new(capability_registry_fixture(), WorkflowExecutor)
            .with_workflow_registry(workflow_registry)
            .with_security_config(RuntimeSecurityConfig::development())
            .with_event_broker(broker.clone());

        let workflow = runtime.execute_workflow(valid_workflow_request());
        assert_eq!(workflow.result.status, WorkflowTraversalStatus::Completed);

        let poll = broker
            .poll(&subscription.subscription_id, 10)
            .unwrap_or_else(|error| unreachable!("{error:?}"));
        assert_eq!(poll.events.len(), 1);
        assert_eq!(
            poll.events[0].event.event_type,
            "content.comments.draft-created"
        );
    }

    /// Spec 101-local-executor-event-emission FR-005 / acceptance scenario 7:
    /// an `EventBroker` publish failure at workflow-node publish time is
    /// recorded (best-effort, matching `PlacementRouter` Step 5) but does not
    /// fail the workflow step — a broker outage does not fail traversal.
    #[test]
    fn workflow_node_publish_failure_does_not_fail_workflow_step() {
        let workflow_registry = workflow_registry_fixture();
        let runtime = Runtime::new(capability_registry_fixture(), WorkflowExecutor)
            .with_workflow_registry(workflow_registry)
            .with_security_config(RuntimeSecurityConfig::development())
            .with_event_broker(Arc::new(AlwaysFailingBroker));

        let workflow = runtime.execute_workflow(valid_workflow_request());
        assert_eq!(workflow.result.status, WorkflowTraversalStatus::Completed);
    }

    #[test]
    fn workflow_failures_cover_not_found_missing_events_and_step_failures() {
        let workflow_registry = workflow_registry_fixture();
        let runtime = Runtime::new(capability_registry_fixture(), WorkflowExecutor)
            .with_workflow_registry(workflow_registry);

        let mut missing_request = valid_workflow_request();
        missing_request.workflow_id = "missing".to_string();
        let missing = runtime.execute_workflow(missing_request);
        assert_eq!(
            missing.evidence.result.failure_reason,
            Some(WorkflowTraversalFailureReason::WorkflowNotFound)
        );

        let workflow_registry = workflow_registry_fixture();
        // The event type is registered (spec 099: EventBroker's catalog is
        // consulted before a waiting edge is declared unsatisfiable) but
        // never published, so this still exercises the genuine
        // "not-yet-happened" MissingRequiredEvent path rather than
        // EventSubscriptionFailed's unregistered-type path.
        let runtime = Runtime::new(capability_registry_fixture(), MissingEventWorkflowExecutor)
            .with_workflow_registry(workflow_registry)
            .with_security_config(RuntimeSecurityConfig::development())
            .with_event_broker(event_catalog_broker_fixture(
                "content.comments.validated",
                "1.0.0",
            ));
        let missing_event = runtime.execute_workflow(valid_workflow_request());
        assert_eq!(
            missing_event.evidence.result.failure_reason,
            Some(WorkflowTraversalFailureReason::MissingRequiredEvent)
        );

        let runtime = Runtime::new(capability_registry_fixture(), FailingWorkflowExecutor)
            .with_workflow_registry(workflow_registry_fixture())
            .with_security_config(RuntimeSecurityConfig::development());
        let failed = runtime.execute_workflow(valid_workflow_request());
        assert_eq!(
            failed.evidence.result.failure_reason,
            Some(WorkflowTraversalFailureReason::StepExecutionFailed)
        );
    }

    #[test]
    fn event_driven_helpers_are_deterministic_and_prevent_duplicate_consumption() {
        let event = EmittedEventRecord {
            record_id: "event_record_0".to_string(),
            event: EventReference {
                event_id: "content.comments.validated".to_string(),
                version: "1.0.0".to_string(),
            },
            payload: Some(json!({"severity": "normal"})),
            source: None,
        };
        let waiting_edges = vec![
            WaitingWorkflowEdgeContext {
                workflow_execution_id: "wf_exec_b".to_string(),
                edge_id: "edge_b".to_string(),
                from_node_id: "from".to_string(),
                to_node_id: "to".to_string(),
                event_ref: event.event.clone(),
                predicate: Some(WorkflowEdgePredicate {
                    field: "payload.severity".to_string(),
                    equals: json!("normal"),
                }),
            },
            WaitingWorkflowEdgeContext {
                workflow_execution_id: "wf_exec_a".to_string(),
                edge_id: "edge_a".to_string(),
                from_node_id: "from".to_string(),
                to_node_id: "to".to_string(),
                event_ref: event.event.clone(),
                predicate: None,
            },
        ];
        let mut consumed = BTreeSet::new();
        let first = evaluate_event_driven_edges(
            &waiting_edges,
            std::slice::from_ref(&event),
            &mut consumed,
            "trace",
        );
        assert_eq!(
            first.taken_edge_ids,
            vec!["edge_a".to_string(), "edge_b".to_string()]
        );
        assert_eq!(
            first
                .evidence
                .wake_decisions
                .iter()
                .map(|decision| (&decision.workflow_execution_id, decision.wake_order))
                .collect::<Vec<_>>(),
            vec![(&"wf_exec_a".to_string(), 1), (&"wf_exec_b".to_string(), 2)]
        );

        let second = evaluate_event_driven_edges(&waiting_edges, &[event], &mut consumed, "trace");
        assert!(second.taken_edge_ids.is_empty());
        assert!(
            second
                .evidence
                .match_records
                .iter()
                .all(|record| record.match_result == EventMatchResult::AlreadyConsumed)
        );
    }

    #[test]
    fn event_driven_helpers_reject_non_matching_predicates() {
        let waiting_edges = vec![WaitingWorkflowEdgeContext {
            workflow_execution_id: "wf_exec_1".to_string(),
            edge_id: "edge_predicate".to_string(),
            from_node_id: "assess".to_string(),
            to_node_id: "validate".to_string(),
            event_ref: EventReference {
                event_id: "expedition.conditions.summary-assessed".to_string(),
                version: "1.0.0".to_string(),
            },
            predicate: Some(WorkflowEdgePredicate {
                field: "payload.severity".to_string(),
                equals: json!("high"),
            }),
        }];
        let emitted = vec![EmittedEventRecord {
            record_id: "event_record_0".to_string(),
            event: EventReference {
                event_id: "expedition.conditions.summary-assessed".to_string(),
                version: "1.0.0".to_string(),
            },
            payload: Some(json!({"severity": "normal"})),
            source: None,
        }];
        let outcome =
            evaluate_event_driven_edges(&waiting_edges, &emitted, &mut BTreeSet::new(), "trace");
        assert!(outcome.taken_edge_ids.is_empty());
        assert_eq!(
            outcome.evidence.match_records,
            vec![EventMatchRecord {
                event_id: "expedition.conditions.summary-assessed".to_string(),
                event_version: "1.0.0".to_string(),
                edge_id: "edge_predicate".to_string(),
                match_result: EventMatchResult::NotMatched,
                predicate_result: Some(EventPredicateResult::Failed),
                rejection_reason: Some(
                    "event predicate did not match the emitted payload".to_string()
                ),
                recorded_at: "trace:event:0:match:edge_predicate".to_string(),
                subscription_id: None,
                cursor: None,
            }]
        );
    }

    #[test]
    fn event_driven_helpers_record_non_matching_event_identity() {
        let waiting_edges = vec![WaitingWorkflowEdgeContext {
            workflow_execution_id: "wf_exec_1".to_string(),
            edge_id: "edge_identity".to_string(),
            from_node_id: "create".to_string(),
            to_node_id: "validate".to_string(),
            event_ref: EventReference {
                event_id: "content.comments.validated".to_string(),
                version: "1.0.0".to_string(),
            },
            predicate: None,
        }];
        let emitted = vec![EmittedEventRecord {
            record_id: "event_record_0".to_string(),
            event: EventReference {
                event_id: "content.comments.other".to_string(),
                version: "1.0.0".to_string(),
            },
            payload: None,
            source: None,
        }];
        let outcome =
            evaluate_event_driven_edges(&waiting_edges, &emitted, &mut BTreeSet::new(), "trace");
        assert!(outcome.taken_edge_ids.is_empty());
        assert_eq!(
            outcome.evidence.match_records,
            vec![EventMatchRecord {
                event_id: "content.comments.other".to_string(),
                event_version: "1.0.0".to_string(),
                edge_id: "edge_identity".to_string(),
                match_result: EventMatchResult::NotMatched,
                predicate_result: None,
                rejection_reason: Some(
                    "event id/version did not match the waiting edge".to_string()
                ),
                recorded_at: "trace:event:0:match:edge_identity".to_string(),
                subscription_id: None,
                cursor: None,
            }]
        );
    }

    fn event_catalog_broker_fixture(event_type: &str, version: &str) -> Arc<InProcessBroker> {
        let catalog = Arc::new(events::EventCatalog::new());
        catalog
            .register(events::EventCatalogEntry {
                event_type: event_type.to_string(),
                owner: "content.comments".to_string(),
                version: version.to_string(),
                lifecycle_status: events::LifecycleStatus::Active,
                consumer_count: 0,
            })
            .unwrap_or_else(|error| unreachable!("{error:?}"));
        Arc::new(InProcessBroker::new(catalog).unwrap_or_else(|error| unreachable!("{error:?}")))
    }

    fn sample_traverse_event(event_type: &str, version: &str) -> events::TraverseEvent {
        events::TraverseEvent {
            id: "evt-fixture-1".to_string(),
            source: "traverse-runtime/content.comments.validate-comment".to_string(),
            event_type: event_type.to_string(),
            datacontenttype: "application/json".to_string(),
            time: "2026-08-06T00:00:00Z".to_string(),
            data: json!({"comment_id": "comment-1"}),
            owner: "content.comments".to_string(),
            version: version.to_string(),
            lifecycle_status: events::LifecycleStatus::Active,
            deduplication_id: None,
            ordering_scope: None,
            correlation_id: None,
            causation_id: None,
            subject_id: None,
            actor_id: None,
        }
    }

    /// Spec 099 FR-001/FR-002: a waiting event-driven edge advances from an
    /// event published to `EventBroker` by another publisher, not just an
    /// event declared in the same execution's own node output.
    #[test]
    fn event_driven_edge_advances_from_broker_published_event() {
        let broker = event_catalog_broker_fixture("content.comments.validated", "1.0.0");
        broker
            .publish(sample_traverse_event("content.comments.validated", "1.0.0"))
            .unwrap_or_else(|error| unreachable!("{error:?}"));

        let runtime = Runtime::new(capability_registry_fixture(), MissingEventWorkflowExecutor)
            .with_workflow_registry(workflow_registry_fixture())
            .with_security_config(RuntimeSecurityConfig::development())
            .with_event_broker(broker);

        let outcome = runtime.execute_workflow(valid_workflow_request());
        assert_eq!(
            outcome.evidence.result.status,
            WorkflowTraversalStatus::Completed
        );
        assert_eq!(outcome.evidence.result.failure_reason, None);

        let wake = outcome
            .evidence
            .event_wake_decisions
            .iter()
            .find(|decision| decision.edge_id == "validate_to_persist")
            .unwrap_or_else(|| unreachable!("expected a wake decision for validate_to_persist"));
        assert!(wake.subscription_id.is_some());
        assert!(wake.cursor.is_some());

        let consumption = outcome
            .evidence
            .event_consumptions
            .iter()
            .find(|record| record.edge_id == "validate_to_persist")
            .unwrap_or_else(|| unreachable!("expected a consumption record"));
        assert!(consumption.subscription_id.is_some());
        assert!(consumption.cursor.is_some());
    }

    /// Spec 099 FR-005: a match sourced from the current node's own emitted
    /// output (not `EventBroker`) still carries no subscription/cursor,
    /// distinguishing same-execution matches from broker-sourced ones.
    #[test]
    fn event_driven_edge_same_node_match_has_no_broker_provenance() {
        let broker = event_catalog_broker_fixture("content.comments.validated", "1.0.0");
        let runtime = Runtime::new(capability_registry_fixture(), WorkflowExecutor)
            .with_workflow_registry(workflow_registry_fixture())
            .with_security_config(RuntimeSecurityConfig::development())
            .with_event_broker(broker);

        let outcome = runtime.execute_workflow(valid_workflow_request());
        assert_eq!(
            outcome.evidence.result.status,
            WorkflowTraversalStatus::Completed
        );
        let wake = outcome
            .evidence
            .event_wake_decisions
            .iter()
            .find(|decision| decision.edge_id == "validate_to_persist")
            .unwrap_or_else(|| unreachable!("expected a wake decision for validate_to_persist"));
        assert_eq!(wake.subscription_id, None);
        assert_eq!(wake.cursor, None);
    }

    /// Spec 099 FR-007: a waiting edge whose declared event type is not
    /// registered in `EventBroker`'s catalog surfaces a stable,
    /// machine-readable error rather than silently never waking.
    #[test]
    fn event_driven_edge_fails_with_event_subscription_failed_for_unregistered_type() {
        let empty_catalog = Arc::new(events::EventCatalog::new());
        let broker = Arc::new(
            InProcessBroker::new(empty_catalog).unwrap_or_else(|error| unreachable!("{error:?}")),
        );

        let runtime = Runtime::new(capability_registry_fixture(), MissingEventWorkflowExecutor)
            .with_workflow_registry(workflow_registry_fixture())
            .with_security_config(RuntimeSecurityConfig::development())
            .with_event_broker(broker);

        let outcome = runtime.execute_workflow(valid_workflow_request());
        assert_eq!(
            outcome.evidence.result.failure_reason,
            Some(WorkflowTraversalFailureReason::EventSubscriptionFailed)
        );
    }

    struct AlwaysFailingBroker;

    impl EventBroker for AlwaysFailingBroker {
        fn publish(&self, _event: events::TraverseEvent) -> Result<(), EventError> {
            Err(EventError::JournalWrite(
                "simulated broker failure".to_string(),
            ))
        }

        fn subscribe(
            &self,
            _event_type: &str,
            _from_cursor: &str,
        ) -> Result<events::Subscription, EventError> {
            Err(EventError::JournalRead(
                "simulated broker failure".to_string(),
            ))
        }

        fn subscribe_for_subject(
            &self,
            _event_type: &str,
            _from_cursor: &str,
            _subject_id: Option<&str>,
        ) -> Result<events::Subscription, EventError> {
            Err(EventError::JournalRead(
                "simulated broker failure".to_string(),
            ))
        }

        fn poll(
            &self,
            _subscription_id: &str,
            _max_events: usize,
        ) -> Result<events::SubscriptionPoll, EventError> {
            Err(EventError::JournalRead(
                "simulated broker failure".to_string(),
            ))
        }

        fn cancel(&self, _subscription_id: &str) -> Result<(), EventError> {
            Err(EventError::SubscriptionNotFound("unknown".to_string()))
        }
    }

    /// Spec 099 FR-008: `EventBroker` being unreachable or internally
    /// failing when a waiting edge tries to establish its subscription
    /// surfaces a stable, retryable error distinct from FR-007's
    /// unregistered-type case, without crashing the workflow execution.
    #[test]
    fn event_driven_edge_fails_with_event_broker_unavailable_when_broker_errors() {
        let runtime = Runtime::new(capability_registry_fixture(), MissingEventWorkflowExecutor)
            .with_workflow_registry(workflow_registry_fixture())
            .with_security_config(RuntimeSecurityConfig::development())
            .with_event_broker(Arc::new(AlwaysFailingBroker));

        let outcome = runtime.execute_workflow(valid_workflow_request());
        assert_eq!(
            outcome.evidence.result.failure_reason,
            Some(WorkflowTraversalFailureReason::EventBrokerUnavailable)
        );
    }

    /// Exercises every `AlwaysFailingBroker` operation directly, confirming
    /// the double fails deterministically the way its name promises (only
    /// `subscribe` is reachable through the full workflow path above, since
    /// a failed subscribe short-circuits before poll/cancel/publish).
    #[test]
    fn always_failing_broker_fails_every_operation() {
        let broker = AlwaysFailingBroker;
        assert!(matches!(
            broker.publish(sample_traverse_event("content.comments.validated", "1.0.0")),
            Err(EventError::JournalWrite(_))
        ));
        assert!(matches!(
            broker.subscribe("content.comments.validated", "0"),
            Err(EventError::JournalRead(_))
        ));
        assert!(matches!(
            broker.subscribe_for_subject("content.comments.validated", "0", None),
            Err(EventError::JournalRead(_))
        ));
        assert!(matches!(
            broker.poll("sub-1", 1),
            Err(EventError::JournalRead(_))
        ));
        assert!(matches!(
            broker.cancel("sub-1"),
            Err(EventError::SubscriptionNotFound(_))
        ));
    }

    struct PollFailingBroker(InProcessBroker);

    impl EventBroker for PollFailingBroker {
        fn publish(&self, event: events::TraverseEvent) -> Result<(), EventError> {
            self.0.publish(event)
        }

        fn subscribe(
            &self,
            event_type: &str,
            from_cursor: &str,
        ) -> Result<events::Subscription, EventError> {
            self.0.subscribe(event_type, from_cursor)
        }

        fn subscribe_for_subject(
            &self,
            event_type: &str,
            from_cursor: &str,
            subject_id: Option<&str>,
        ) -> Result<events::Subscription, EventError> {
            self.0
                .subscribe_for_subject(event_type, from_cursor, subject_id)
        }

        fn poll(
            &self,
            _subscription_id: &str,
            _max_events: usize,
        ) -> Result<events::SubscriptionPoll, EventError> {
            Err(EventError::JournalRead(
                "simulated poll failure".to_string(),
            ))
        }

        fn cancel(&self, subscription_id: &str) -> Result<(), EventError> {
            self.0.cancel(subscription_id)
        }
    }

    /// Spec 099 FR-008: a broker whose subscription establishes fine but
    /// whose `poll` fails mid-drain is a distinct failure point from
    /// subscribe failing outright, and must surface the same stable,
    /// retryable `EventBrokerUnavailable` reason.
    #[test]
    fn event_driven_edge_fails_with_event_broker_unavailable_when_poll_errors() {
        let catalog = Arc::new(events::EventCatalog::new());
        catalog
            .register(events::EventCatalogEntry {
                event_type: "content.comments.validated".to_string(),
                owner: "content.comments".to_string(),
                version: "1.0.0".to_string(),
                lifecycle_status: events::LifecycleStatus::Active,
                consumer_count: 0,
            })
            .unwrap_or_else(|error| unreachable!("{error:?}"));
        let inner = InProcessBroker::new(catalog).unwrap_or_else(|error| unreachable!("{error:?}"));
        let broker = Arc::new(PollFailingBroker(inner));

        // `publish`/`subscribe_for_subject` delegate straight through to the
        // wrapped broker; only `poll` is overridden to fail.
        broker
            .publish(sample_traverse_event("content.comments.validated", "1.0.0"))
            .unwrap_or_else(|error| unreachable!("{error:?}"));
        broker
            .subscribe_for_subject("content.comments.validated", "0", None)
            .unwrap_or_else(|error| unreachable!("{error:?}"));

        let runtime = Runtime::new(capability_registry_fixture(), MissingEventWorkflowExecutor)
            .with_workflow_registry(workflow_registry_fixture())
            .with_security_config(RuntimeSecurityConfig::development())
            .with_event_broker(broker);

        let outcome = runtime.execute_workflow(valid_workflow_request());
        assert_eq!(
            outcome.evidence.result.failure_reason,
            Some(WorkflowTraversalFailureReason::EventBrokerUnavailable)
        );
    }

    struct CancelFailingBroker(InProcessBroker);

    impl EventBroker for CancelFailingBroker {
        fn publish(&self, event: events::TraverseEvent) -> Result<(), EventError> {
            self.0.publish(event)
        }

        fn subscribe(
            &self,
            event_type: &str,
            from_cursor: &str,
        ) -> Result<events::Subscription, EventError> {
            self.0.subscribe(event_type, from_cursor)
        }

        fn subscribe_for_subject(
            &self,
            event_type: &str,
            from_cursor: &str,
            subject_id: Option<&str>,
        ) -> Result<events::Subscription, EventError> {
            self.0
                .subscribe_for_subject(event_type, from_cursor, subject_id)
        }

        fn poll(
            &self,
            subscription_id: &str,
            max_events: usize,
        ) -> Result<events::SubscriptionPoll, EventError> {
            self.0.poll(subscription_id, max_events)
        }

        fn cancel(&self, _subscription_id: &str) -> Result<(), EventError> {
            Err(EventError::SubscriptionNotFound(
                "simulated cancel failure".to_string(),
            ))
        }
    }

    /// A broker subscription that fails to cancel after a successful drain
    /// does not fail the workflow (cleanup is best-effort); it surfaces a
    /// non-fatal `RuntimeWarning` instead.
    #[test]
    fn event_driven_edge_surfaces_warning_when_subscription_cancel_fails() {
        let catalog = Arc::new(events::EventCatalog::new());
        catalog
            .register(events::EventCatalogEntry {
                event_type: "content.comments.validated".to_string(),
                owner: "content.comments".to_string(),
                version: "1.0.0".to_string(),
                lifecycle_status: events::LifecycleStatus::Active,
                consumer_count: 0,
            })
            .unwrap_or_else(|error| unreachable!("{error:?}"));
        let inner = InProcessBroker::new(catalog).unwrap_or_else(|error| unreachable!("{error:?}"));
        inner
            .publish(sample_traverse_event("content.comments.validated", "1.0.0"))
            .unwrap_or_else(|error| unreachable!("{error:?}"));
        let broker = Arc::new(CancelFailingBroker(inner));

        // `publish`/`subscribe_for_subject` delegate straight through to the
        // wrapped broker; only `cancel` is overridden to fail.
        broker
            .publish(sample_traverse_event("content.comments.validated", "1.0.0"))
            .unwrap_or_else(|error| unreachable!("{error:?}"));
        broker
            .subscribe_for_subject("content.comments.validated", "0", None)
            .unwrap_or_else(|error| unreachable!("{error:?}"));

        let runtime = Runtime::new(capability_registry_fixture(), MissingEventWorkflowExecutor)
            .with_workflow_registry(workflow_registry_fixture())
            .with_security_config(RuntimeSecurityConfig::development())
            .with_event_broker(broker);

        let outcome = runtime.execute_workflow(valid_workflow_request());
        assert_eq!(
            outcome.evidence.result.status,
            WorkflowTraversalStatus::Completed
        );
        assert!(
            outcome
                .result
                .warnings
                .iter()
                .any(|warning| warning.code == "event_broker_subscription_cleanup_failed")
        );
    }

    /// Spec 099 FR-004: within one evaluation, a broker-sourced event that
    /// has already been consumed by a waiting edge does not advance that
    /// edge a second time even though the broker still has the event
    /// buffered (simulating post-restart durable replay).
    #[test]
    fn poll_broker_events_for_waiting_edges_supports_exact_once_across_repeated_polls() {
        let broker = event_catalog_broker_fixture("content.comments.validated", "1.0.0");
        broker
            .publish(sample_traverse_event("content.comments.validated", "1.0.0"))
            .unwrap_or_else(|error| unreachable!("{error:?}"));

        let waiting_edges = vec![WaitingWorkflowEdgeContext {
            workflow_execution_id: "wf_exec_replay".to_string(),
            edge_id: "validate_to_persist".to_string(),
            from_node_id: "validate_comment".to_string(),
            to_node_id: "persist_comment".to_string(),
            event_ref: EventReference {
                event_id: "content.comments.validated".to_string(),
                version: "1.0.0".to_string(),
            },
            predicate: None,
        }];

        let mut consumed = BTreeSet::new();

        let (first_batch, first_warnings) =
            poll_broker_events_for_waiting_edges(broker.as_ref(), &waiting_edges)
                .unwrap_or_else(|error| unreachable!("{error:?}"));
        assert!(first_warnings.is_empty());
        let first = evaluate_event_driven_edges(&waiting_edges, &first_batch, &mut consumed, "t1");
        assert_eq!(
            first.taken_edge_ids,
            vec!["validate_to_persist".to_string()]
        );

        // The broker still has the event buffered (it was never cancelled
        // mid-retention), simulating a fresh subscription replaying the
        // same durable history after a restart.
        let (second_batch, second_warnings) =
            poll_broker_events_for_waiting_edges(broker.as_ref(), &waiting_edges)
                .unwrap_or_else(|error| unreachable!("{error:?}"));
        assert!(second_warnings.is_empty());
        let second =
            evaluate_event_driven_edges(&waiting_edges, &second_batch, &mut consumed, "t2");
        assert!(second.taken_edge_ids.is_empty());
        assert!(
            second
                .evidence
                .match_records
                .iter()
                .all(|record| record.match_result == EventMatchResult::AlreadyConsumed)
        );
    }

    /// Spec 099: `poll_broker_events_for_waiting_edges` visits distinct
    /// event types in a fixed sorted order and cancels each subscription
    /// after draining it, so it does not leak broker subscriptions.
    #[test]
    fn poll_broker_events_for_waiting_edges_cancels_subscriptions_after_draining() {
        let catalog = Arc::new(events::EventCatalog::new());
        for event_type in [
            "content.comments.draft-created",
            "content.comments.validated",
        ] {
            catalog
                .register(events::EventCatalogEntry {
                    event_type: event_type.to_string(),
                    owner: "content.comments".to_string(),
                    version: "1.0.0".to_string(),
                    lifecycle_status: events::LifecycleStatus::Active,
                    consumer_count: 0,
                })
                .unwrap_or_else(|error| unreachable!("{error:?}"));
        }
        let broker =
            InProcessBroker::new(catalog).unwrap_or_else(|error| unreachable!("{error:?}"));
        broker
            .publish(sample_traverse_event("content.comments.validated", "1.0.0"))
            .unwrap_or_else(|error| unreachable!("{error:?}"));

        let waiting_edges = vec![
            WaitingWorkflowEdgeContext {
                workflow_execution_id: "wf_exec_multi".to_string(),
                edge_id: "edge_a".to_string(),
                from_node_id: "a".to_string(),
                to_node_id: "b".to_string(),
                event_ref: EventReference {
                    event_id: "content.comments.validated".to_string(),
                    version: "1.0.0".to_string(),
                },
                predicate: None,
            },
            WaitingWorkflowEdgeContext {
                workflow_execution_id: "wf_exec_multi".to_string(),
                edge_id: "edge_b".to_string(),
                from_node_id: "a".to_string(),
                to_node_id: "c".to_string(),
                event_ref: EventReference {
                    event_id: "content.comments.draft-created".to_string(),
                    version: "1.0.0".to_string(),
                },
                predicate: None,
            },
        ];

        let (records, warnings) = poll_broker_events_for_waiting_edges(&broker, &waiting_edges)
            .unwrap_or_else(|error| unreachable!("{error:?}"));
        assert!(warnings.is_empty());
        assert_eq!(records.len(), 1);
        let source = records[0]
            .source
            .as_ref()
            .unwrap_or_else(|| unreachable!("expected broker provenance on the record"));

        // The subscription was cancelled after draining, so polling it
        // again must fail with `SubscriptionNotFound`.
        assert!(matches!(
            broker.poll(&source.subscription_id, 1),
            Err(EventError::SubscriptionNotFound(id)) if id == source.subscription_id
        ));
    }

    /// `poll_broker_events_for_waiting_edges` drains a subscription across
    /// multiple internal `EventBroker::poll` calls when more events are
    /// buffered than one poll batch can return.
    #[test]
    fn poll_broker_events_for_waiting_edges_paginates_beyond_one_batch() {
        let broker = event_catalog_broker_fixture("content.comments.validated", "1.0.0");
        let published_count = EVENT_BROKER_POLL_BATCH + 5;
        for index in 0..published_count {
            let mut event = sample_traverse_event("content.comments.validated", "1.0.0");
            event.id = format!("evt-fixture-{index}");
            event.deduplication_id = Some(event.id.clone());
            broker
                .publish(event)
                .unwrap_or_else(|error| unreachable!("{error:?}"));
        }

        let waiting_edges = vec![WaitingWorkflowEdgeContext {
            workflow_execution_id: "wf_exec_paginate".to_string(),
            edge_id: "validate_to_persist".to_string(),
            from_node_id: "validate_comment".to_string(),
            to_node_id: "persist_comment".to_string(),
            event_ref: EventReference {
                event_id: "content.comments.validated".to_string(),
                version: "1.0.0".to_string(),
            },
            predicate: None,
        }];

        let (records, warnings) =
            poll_broker_events_for_waiting_edges(broker.as_ref(), &waiting_edges)
                .unwrap_or_else(|error| unreachable!("{error:?}"));
        assert!(warnings.is_empty());
        assert_eq!(records.len(), published_count);
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn workflow_runtime_covers_additional_failure_and_helper_branches() {
        let runtime = Runtime::new(capability_registry_fixture(), WorkflowExecutor)
            .with_workflow_registry(workflow_registry_fixture())
            .with_security_config(RuntimeSecurityConfig::development());

        let invalid_input = runtime.execute_workflow(WorkflowExecutionRequest {
            input: json!({}),
            ..valid_workflow_request()
        });
        assert_eq!(
            invalid_input.evidence.result.failure_reason,
            Some(WorkflowTraversalFailureReason::WorkflowInvalid)
        );

        let invalid_request = runtime.execute_workflow(WorkflowExecutionRequest {
            kind: "bad".to_string(),
            ..valid_workflow_request()
        });
        assert_eq!(
            invalid_request.evidence.result.failure_reason,
            Some(WorkflowTraversalFailureReason::WorkflowInvalid)
        );

        let missing_node = runtime.traverse_workflow(
            &valid_workflow_request(),
            &resolved_workflow(WorkflowDefinition {
                start_node: "missing".to_string(),
                ..workflow_definition_fixture(
                    Some(EventReference {
                        event_id: "content.comments.validated".to_string(),
                        version: "1.0.0".to_string(),
                    }),
                    None,
                )
            }),
        );
        assert!(missing_node.is_err());

        let missing_capability_runtime = Runtime::new(CapabilityRegistry::new(), WorkflowExecutor)
            .with_workflow_registry(workflow_registry_fixture());
        let missing_capability =
            missing_capability_runtime.execute_workflow(valid_workflow_request());
        assert_eq!(
            missing_capability.evidence.result.failure_reason,
            Some(WorkflowTraversalFailureReason::WorkflowInvalid)
        );

        let strict_runtime =
            Runtime::new(strict_input_capability_registry_fixture(), WorkflowExecutor)
                .with_workflow_registry(workflow_registry_fixture())
                .with_security_config(RuntimeSecurityConfig::development());
        let input_mismatch = strict_runtime.traverse_workflow(
            &valid_workflow_request(),
            &resolved_workflow(WorkflowDefinition {
                nodes: vec![WorkflowNode {
                    input: WorkflowNodeInput {
                        from_workflow_input: vec!["missing".to_string()],
                    },
                    ..workflow_definition_fixture(
                        Some(EventReference {
                            event_id: "content.comments.validated".to_string(),
                            version: "1.0.0".to_string(),
                        }),
                        None,
                    )
                    .nodes[0]
                        .clone()
                }],
                edges: Vec::new(),
                start_node: "create_draft".to_string(),
                terminal_nodes: vec!["create_draft".to_string()],
                ..workflow_definition_fixture(
                    Some(EventReference {
                        event_id: "content.comments.validated".to_string(),
                        version: "1.0.0".to_string(),
                    }),
                    None,
                )
            }),
        );
        assert!(input_mismatch.is_err());

        let bad_output_runtime =
            Runtime::new(capability_registry_fixture(), BadOutputWorkflowExecutor)
                .with_workflow_registry(workflow_registry_fixture())
                .with_security_config(RuntimeSecurityConfig::development());
        let bad_output = bad_output_runtime.execute_workflow(valid_workflow_request());
        assert_eq!(
            bad_output.evidence.result.failure_reason,
            Some(WorkflowTraversalFailureReason::StepExecutionFailed)
        );

        // Spec 101-local-executor-event-emission FR-007/FR-008: a workflow
        // node that emits an event its contract does not declare must fail
        // the step with a contract-violation error, mirroring
        // `PlacementRouter`'s native-boundary validation.
        let undeclared_event_runtime = Runtime::new(
            capability_registry_fixture(),
            UndeclaredEventWorkflowExecutor,
        )
        .with_workflow_registry(workflow_registry_fixture())
        .with_security_config(RuntimeSecurityConfig::development());
        let undeclared_event = undeclared_event_runtime.execute_workflow(valid_workflow_request());
        assert_eq!(
            undeclared_event.evidence.result.failure_reason,
            Some(WorkflowTraversalFailureReason::StepExecutionFailed)
        );

        let direct_success = runtime.traverse_workflow(
            &valid_workflow_request(),
            &resolved_workflow(workflow_definition_fixture(
                Some(EventReference {
                    event_id: "content.comments.validated".to_string(),
                    version: "1.0.0".to_string(),
                }),
                Some(WorkflowEdge {
                    edge_id: "direct".to_string(),
                    from: "create_draft".to_string(),
                    to: "validate_comment".to_string(),
                    trigger: WorkflowEdgeTrigger::Direct,
                    event: None,
                    predicate: None,
                }),
            )),
        );
        assert!(direct_success.is_ok());

        let ambiguous_direct = runtime.traverse_workflow(
            &valid_workflow_request(),
            &resolved_workflow(WorkflowDefinition {
                edges: vec![
                    WorkflowEdge {
                        edge_id: "direct-1".to_string(),
                        from: "create_draft".to_string(),
                        to: "validate_comment".to_string(),
                        trigger: WorkflowEdgeTrigger::Direct,
                        event: None,
                        predicate: None,
                    },
                    WorkflowEdge {
                        edge_id: "direct-2".to_string(),
                        from: "create_draft".to_string(),
                        to: "persist_comment".to_string(),
                        trigger: WorkflowEdgeTrigger::Direct,
                        event: None,
                        predicate: None,
                    },
                ],
                ..workflow_definition_fixture(
                    Some(EventReference {
                        event_id: "content.comments.validated".to_string(),
                        version: "1.0.0".to_string(),
                    }),
                    None,
                )
            }),
        );
        assert!(ambiguous_direct.is_err());

        let ambiguous_event = runtime.traverse_workflow(
            &valid_workflow_request(),
            &resolved_workflow(WorkflowDefinition {
                edges: vec![
                    WorkflowEdge {
                        edge_id: "draft_to_validate".to_string(),
                        from: "create_draft".to_string(),
                        to: "validate_comment".to_string(),
                        trigger: WorkflowEdgeTrigger::Event,
                        event: Some(EventReference {
                            event_id: "content.comments.draft-created".to_string(),
                            version: "1.0.0".to_string(),
                        }),
                        predicate: None,
                    },
                    WorkflowEdge {
                        edge_id: "draft_to_persist".to_string(),
                        from: "create_draft".to_string(),
                        to: "persist_comment".to_string(),
                        trigger: WorkflowEdgeTrigger::Event,
                        event: Some(EventReference {
                            event_id: "content.comments.draft-created".to_string(),
                            version: "1.0.0".to_string(),
                        }),
                        predicate: None,
                    },
                ],
                ..workflow_definition_fixture(
                    Some(EventReference {
                        event_id: "content.comments.validated".to_string(),
                        version: "1.0.0".to_string(),
                    }),
                    None,
                )
            }),
        );
        assert!(ambiguous_event.is_err());

        let terminal_miss = runtime.traverse_workflow(
            &valid_workflow_request(),
            &resolved_workflow(WorkflowDefinition {
                edges: Vec::new(),
                terminal_nodes: vec!["persist_comment".to_string()],
                ..workflow_definition_fixture(
                    Some(EventReference {
                        event_id: "content.comments.validated".to_string(),
                        version: "1.0.0".to_string(),
                    }),
                    None,
                )
            }),
        );
        assert!(terminal_miss.is_err());

        let invalid_final_output = runtime.traverse_workflow(
            &valid_workflow_request(),
            &resolved_workflow(WorkflowDefinition {
                outputs: SchemaContainer {
                    schema: json!({
                        "type": "object",
                        "properties": { "missing": { "type": "string" } },
                        "required": ["missing"],
                        "additionalProperties": true
                    }),
                },
                ..workflow_definition_fixture(
                    Some(EventReference {
                        event_id: "content.comments.validated".to_string(),
                        version: "1.0.0".to_string(),
                    }),
                    None,
                )
            }),
        );
        assert!(invalid_final_output.is_err());

        let selection = SelectionRecord {
            status: crate::SelectionStatus::Selected,
            selected_capability_id: Some("content.comments.publish-comment".to_string()),
            selected_capability_version: Some("1.0.0".to_string()),
            failure_reason: None,
            remaining_candidates: Vec::new(),
        };
        let mut selected = runtime
            .registry
            .find_exact(
                LookupScope::PublicOnly,
                "content.comments.create-comment-draft",
                "1.0.0",
            )
            .unwrap_or_else(|| unreachable!("fixture capability missing"));
        selected.record.implementation_kind = ImplementationKind::Workflow;
        let (attempt, mut emitter) = super::super::begin_attempt(
            RuntimeRequest {
                kind: "runtime_request".to_string(),
                schema_version: "1.0.0".to_string(),
                request_id: "workflow-capability".to_string(),
                intent: RuntimeIntent {
                    capability_id: Some("content.comments.publish-comment".to_string()),
                    capability_version: Some("1.0.0".to_string()),
                    version_range: None,
                    intent_key: None,
                },
                input: json!({"comment_text": "hello"}),
                lookup: RuntimeLookup {
                    scope: RuntimeLookupScope::PublicOnly,
                    allow_ambiguity: false,
                },
                context: RuntimeContext {
                    requested_target: crate::PlacementTarget::Local,
                    correlation_id: None,
                    caller: None,
                    traceparent: None,
                    tracestate: None,
                    metadata: None,
                    identity: None,
                },
                governing_spec: "006-runtime-request-execution".to_string(),
            },
            crate::RuntimeObservabilityConfig::default(),
        );
        emitter.push(
            crate::RuntimeState::Discovering,
            crate::RuntimeTransitionReasonCode::RequestStarted,
            json!({"lookup_scope": RuntimeLookupScope::PublicOnly}),
        );
        emitter.push(
            crate::RuntimeState::EvaluatingConstraints,
            crate::RuntimeTransitionReasonCode::CandidatesCollected,
            json!({"candidate_count": 1}),
        );
        emitter.push(
            crate::RuntimeState::Selecting,
            crate::RuntimeTransitionReasonCode::ConstraintsEvaluated,
            json!({"eligible_candidates": 1, "rejected_candidates": 0}),
        );
        let started_execution = crate::start_selected_execution(
            &mut emitter,
            &selected,
            crate::resolve_placement(crate::PlacementTarget::Local)
                .unwrap_or_else(|_| unreachable!("local placement should resolve")),
            None,
        );
        let outcome = runtime.execute_workflow_capability(
            crate::ExecutionContext {
                attempt,
                emitter,
                candidate_collection: CandidateCollectionRecord {
                    lookup_scope: RuntimeLookupScope::PublicOnly,
                    candidates: Vec::new(),
                    rejected_candidates: Vec::new(),
                },
                selection,
            },
            &selected,
            started_execution,
        );
        assert_eq!(outcome.result.status, RuntimeResultStatus::Error);

        let mut selected = runtime
            .registry
            .find_exact(
                LookupScope::PublicOnly,
                "content.comments.create-comment-draft",
                "1.0.0",
            )
            .unwrap_or_else(|| unreachable!("fixture capability missing"));
        selected.record.scope = RegistryScope::Private;
        selected.record.implementation_kind = ImplementationKind::Workflow;
        selected.artifact.workflow_ref = Some(traverse_registry::WorkflowReference {
            workflow_id: "content.comments.publish-comment".to_string(),
            workflow_version: "1.0.0".to_string(),
        });
        let (attempt, mut emitter) = super::super::begin_attempt(
            RuntimeRequest {
                request_id: "workflow-private".to_string(),
                ..valid_runtime_request()
            },
            crate::RuntimeObservabilityConfig::default(),
        );
        emitter.push(
            crate::RuntimeState::Discovering,
            crate::RuntimeTransitionReasonCode::RequestStarted,
            json!({"lookup_scope": RuntimeLookupScope::PreferPrivate}),
        );
        emitter.push(
            crate::RuntimeState::EvaluatingConstraints,
            crate::RuntimeTransitionReasonCode::CandidatesCollected,
            json!({"candidate_count": 1}),
        );
        emitter.push(
            crate::RuntimeState::Selecting,
            crate::RuntimeTransitionReasonCode::ConstraintsEvaluated,
            json!({"eligible_candidates": 1, "rejected_candidates": 0}),
        );
        let started_execution = crate::start_selected_execution(
            &mut emitter,
            &selected,
            crate::resolve_placement(crate::PlacementTarget::Local)
                .unwrap_or_else(|_| unreachable!("local placement should resolve")),
            None,
        );
        let failing_runtime = Runtime::new(capability_registry_fixture(), FailingWorkflowExecutor)
            .with_workflow_registry(workflow_registry_fixture())
            .with_security_config(RuntimeSecurityConfig::development());
        let outcome = failing_runtime.execute_workflow_capability(
            crate::ExecutionContext {
                attempt,
                emitter,
                candidate_collection: CandidateCollectionRecord {
                    lookup_scope: RuntimeLookupScope::PreferPrivate,
                    candidates: Vec::new(),
                    rejected_candidates: Vec::new(),
                },
                selection: SelectionRecord {
                    status: crate::SelectionStatus::Selected,
                    selected_capability_id: Some("content.comments.publish-comment".to_string()),
                    selected_capability_version: Some("1.0.0".to_string()),
                    failure_reason: None,
                    remaining_candidates: Vec::new(),
                },
            },
            &selected,
            started_execution,
        );
        assert_eq!(outcome.result.status, RuntimeResultStatus::Error);

        let mut unknown = runtime
            .registry
            .find_exact(
                LookupScope::PublicOnly,
                "content.comments.create-comment-draft",
                "1.0.0",
            )
            .unwrap_or_else(|| unreachable!("fixture capability missing"));
        unknown.record.id = "unknown".to_string();
        let _ = WorkflowExecutor.execute(&unknown, &json!({}));
        let _ = MissingEventWorkflowExecutor.execute(&unknown, &json!({}));
        let _ = BadOutputWorkflowExecutor.execute(&unknown, &json!({}));
        unknown.record.id = "content.comments.persist-comment".to_string();
        let _ = MissingEventWorkflowExecutor.execute(&unknown, &json!({}));
    }

    #[test]
    fn pipeline_workflow_merges_namespaced_step_outputs_deterministically() {
        let registry = pipeline_capability_registry();
        let mut workflows = WorkflowRegistry::new();
        register_workflow_ok(&mut workflows, &registry, pipeline_workflow_registration());
        let runtime = Runtime::new(registry, PipelineExecutor)
            .with_workflow_registry(workflows)
            .with_security_config(RuntimeSecurityConfig::development());

        let first = runtime.execute_workflow(pipeline_workflow_request());
        let second = runtime.execute_workflow(pipeline_workflow_request());

        assert_eq!(first.result.status, WorkflowTraversalStatus::Completed);
        assert_eq!(
            first.result.output,
            Some(json!({
                "validate": {"valid": true, "issues": []},
                "process": {
                    "title": "Hello world",
                    "tags": ["hello", "world"],
                    "noteType": "fleeting",
                    "suggestedNextAction": "archive",
                    "status": "complete"
                },
                "summarize": {"summary": "Hello world (fleeting)", "wordCount": 3}
            }))
        );
        assert_eq!(first.result, second.result);
        assert_eq!(first.evidence.visited_nodes, second.evidence.visited_nodes);

        let steps = &first.evidence.visited_nodes;
        assert_eq!(steps.len(), 3);
        assert_eq!(steps[0].step_index, 0);
        assert_eq!(steps[0].capability_id, "content.comments.pipeline-validate");
        assert_eq!(steps[1].step_index, 1);
        assert_eq!(steps[1].capability_id, "content.comments.pipeline-process");
        assert_eq!(steps[2].step_index, 2);
        assert_eq!(
            steps[2].capability_id,
            "content.comments.pipeline-summarize"
        );
        assert!(
            steps
                .iter()
                .all(|step| step.status == WorkflowTraversalStepStatus::Completed)
        );
    }

    #[test]
    fn pipeline_workflow_stops_on_failed_step_with_failed_step_id_in_trace() {
        let registry = pipeline_capability_registry();
        let mut workflows = WorkflowRegistry::new();
        register_workflow_ok(&mut workflows, &registry, pipeline_workflow_registration());
        let runtime = Runtime::new(registry, FailingPipelineExecutor)
            .with_workflow_registry(workflows)
            .with_security_config(RuntimeSecurityConfig::development());

        let outcome = runtime.execute_workflow(pipeline_workflow_request());

        assert_eq!(outcome.result.status, WorkflowTraversalStatus::Error);
        assert_eq!(
            outcome.evidence.result.failure_reason,
            Some(WorkflowTraversalFailureReason::StepExecutionFailed)
        );
        let steps = &outcome.evidence.visited_nodes;
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0].status, WorkflowTraversalStepStatus::Completed);
        assert_eq!(steps[1].node_id, "process_note");
        assert_eq!(steps[1].status, WorkflowTraversalStepStatus::Failed);
    }

    #[test]
    fn workflow_step_rejects_unsigned_artifact_under_default_security_config() {
        // No explicit security config: the default (Production) posture must
        // reject the unsigned artifact before the workflow step executes,
        // exactly like direct capability execution (spec 030 FR-013).
        let runtime = Runtime::new(capability_registry_fixture(), WorkflowExecutor)
            .with_workflow_registry(workflow_registry_fixture());

        let outcome = runtime.execute_workflow(valid_workflow_request());

        assert_eq!(outcome.result.status, WorkflowTraversalStatus::Error);
        assert_eq!(
            outcome.evidence.result.failure_reason,
            Some(WorkflowTraversalFailureReason::StepExecutionFailed)
        );
        let error = outcome.result.error;
        assert_eq!(
            error.as_ref().map(|error| error.code),
            Some(RuntimeErrorCode::ContractViolation)
        );
        assert_eq!(
            error
                .as_ref()
                .and_then(|error| error.details.get("code"))
                .and_then(Value::as_str),
            Some("missing_signature")
        );
        assert_eq!(
            error
                .as_ref()
                .and_then(|error| error.details.get("node_id"))
                .and_then(Value::as_str),
            Some("create_draft")
        );
        let steps = &outcome.evidence.visited_nodes;
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].node_id, "create_draft");
        assert_eq!(steps[0].status, WorkflowTraversalStepStatus::Failed);
        assert!(outcome.result.warnings.is_empty());
    }

    #[test]
    fn workflow_step_unsigned_artifact_warns_and_executes_in_development_mode() {
        let runtime = Runtime::new(capability_registry_fixture(), WorkflowExecutor)
            .with_workflow_registry(workflow_registry_fixture())
            .with_security_config(RuntimeSecurityConfig::development());

        let outcome = runtime.execute_workflow(valid_workflow_request());

        assert_eq!(outcome.result.status, WorkflowTraversalStatus::Completed);
        assert_eq!(outcome.result.warnings.len(), 3);
        assert!(
            outcome
                .result
                .warnings
                .iter()
                .all(|warning| warning.code == "unsigned_local_dev_artifact")
        );
    }

    #[test]
    fn workflow_step_fails_when_signed_artifact_bytes_cannot_be_loaded() {
        let runtime = Runtime::new(
            signed_missing_binary_capability_registry_fixture(),
            WorkflowExecutor,
        )
        .with_workflow_registry(workflow_registry_fixture());

        let outcome = runtime.execute_workflow(valid_workflow_request());

        assert_eq!(outcome.result.status, WorkflowTraversalStatus::Error);
        assert_eq!(
            outcome.evidence.result.failure_reason,
            Some(WorkflowTraversalFailureReason::StepExecutionFailed)
        );
        let error = outcome.result.error;
        assert_eq!(
            error.as_ref().map(|error| error.code),
            Some(RuntimeErrorCode::ArtifactMissing)
        );
        assert_eq!(
            error
                .as_ref()
                .and_then(|error| error.details.get("code"))
                .and_then(Value::as_str),
            Some("artifact_load_failed")
        );
        let steps = &outcome.evidence.visited_nodes;
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].status, WorkflowTraversalStepStatus::Failed);
    }

    fn pipeline_capability_registry() -> CapabilityRegistry {
        let mut registry = CapabilityRegistry::new();
        for id in [
            "content.comments.pipeline-validate",
            "content.comments.pipeline-process",
            "content.comments.pipeline-summarize",
        ] {
            register_capability_ok(
                &mut registry,
                CapabilityRegistration {
                    scope: RegistryScope::Public,
                    contract: capability_contract(
                        id,
                        Vec::new(),
                        json!({"type": "object", "additionalProperties": true}),
                        json!({"type": "object", "additionalProperties": true}),
                    ),
                    contract_path: format!("registry/{id}.json"),
                    artifact: CapabilityArtifactRecord {
                        artifact_ref: format!("artifact-{id}"),
                        implementation_kind: ImplementationKind::Executable,
                        source: SourceReference {
                            kind: SourceKind::Git,
                            location: format!("https://example.com/{id}.git"),
                        },
                        binary: Some(BinaryReference {
                            format: BinaryFormat::Wasm,
                            location: format!("{id}.wasm"),
                            signature: None,
                        }),
                        workflow_ref: None,
                        digests: ArtifactDigests {
                            source_digest: "source".to_string(),
                            binary_digest: Some("binary".to_string()),
                        },
                        provenance: RegistryProvenance {
                            source: "fixtures".to_string(),
                            author: "Enrico".to_string(),
                            created_at: "2026-03-27T00:00:00Z".to_string(),
                        },
                    },
                    registered_at: "2026-03-27T00:00:00Z".to_string(),
                    tags: vec!["pipeline".to_string()],
                    composability: ComposabilityMetadata {
                        kind: CompositionKind::Atomic,
                        patterns: vec![CompositionPattern::Sequential],
                        provides: vec!["pipeline-step".to_string()],
                        requires: Vec::new(),
                    },
                    governing_spec: "005-capability-registry".to_string(),
                    validator_version: "validator".to_string(),
                },
            );
        }
        registry
    }

    #[allow(clippy::too_many_lines)]
    fn pipeline_workflow_registration() -> WorkflowRegistration {
        WorkflowRegistration {
            scope: RegistryScope::Public,
            definition: WorkflowDefinition {
                kind: "workflow_definition".to_string(),
                schema_version: "1.0.0".to_string(),
                id: "content.comments.pipeline".to_string(),
                name: "pipeline".to_string(),
                version: "1.0.0".to_string(),
                lifecycle: Lifecycle::Active,
                owner: Owner {
                    team: "traverse-core".to_string(),
                    contact: "test@example.com".to_string(),
                },
                summary: "Deterministic three-step pipeline fixture.".to_string(),
                inputs: SchemaContainer {
                    schema: json!({
                        "type": "object",
                        "required": ["note"],
                        "properties": {"note": {"type": "string"}},
                        "additionalProperties": false
                    }),
                },
                outputs: SchemaContainer {
                    schema: json!({
                        "type": "object",
                        "required": ["validate", "process", "summarize"],
                        "properties": {
                            "validate": {"type": "object"},
                            "process": {"type": "object"},
                            "summarize": {"type": "object"}
                        },
                        "additionalProperties": false
                    }),
                },
                nodes: vec![
                    WorkflowNode {
                        node_id: "validate_note".to_string(),
                        capability_id: "content.comments.pipeline-validate".to_string(),
                        capability_version: "1.0.0".to_string(),
                        input: WorkflowNodeInput {
                            from_workflow_input: vec!["note".to_string()],
                        },
                        output: WorkflowNodeOutput {
                            to_workflow_state: Vec::new(),
                            publish_to_state_as: Some("validate".to_string()),
                        },
                    },
                    WorkflowNode {
                        node_id: "process_note".to_string(),
                        capability_id: "content.comments.pipeline-process".to_string(),
                        capability_version: "1.0.0".to_string(),
                        input: WorkflowNodeInput {
                            from_workflow_input: vec!["note".to_string()],
                        },
                        output: WorkflowNodeOutput {
                            to_workflow_state: vec![
                                "title".to_string(),
                                "tags".to_string(),
                                "noteType".to_string(),
                                "suggestedNextAction".to_string(),
                                "status".to_string(),
                            ],
                            publish_to_state_as: Some("process".to_string()),
                        },
                    },
                    WorkflowNode {
                        node_id: "summarize_note".to_string(),
                        capability_id: "content.comments.pipeline-summarize".to_string(),
                        capability_version: "1.0.0".to_string(),
                        input: WorkflowNodeInput {
                            from_workflow_input: vec![
                                "title".to_string(),
                                "tags".to_string(),
                                "noteType".to_string(),
                                "suggestedNextAction".to_string(),
                                "status".to_string(),
                            ],
                        },
                        output: WorkflowNodeOutput {
                            to_workflow_state: Vec::new(),
                            publish_to_state_as: Some("summarize".to_string()),
                        },
                    },
                ],
                edges: vec![
                    WorkflowEdge {
                        edge_id: "validate_to_process".to_string(),
                        from: "validate_note".to_string(),
                        to: "process_note".to_string(),
                        trigger: WorkflowEdgeTrigger::Direct,
                        event: None,
                        predicate: None,
                    },
                    WorkflowEdge {
                        edge_id: "process_to_summarize".to_string(),
                        from: "process_note".to_string(),
                        to: "summarize_note".to_string(),
                        trigger: WorkflowEdgeTrigger::Direct,
                        event: None,
                        predicate: None,
                    },
                ],
                start_node: "validate_note".to_string(),
                terminal_nodes: vec!["summarize_note".to_string()],
                output_projection: vec![
                    "validate".to_string(),
                    "process".to_string(),
                    "summarize".to_string(),
                ],
                tags: vec!["pipeline".to_string()],
                governing_spec: "007-workflow-registry-traversal".to_string(),
            },
            workflow_path: "workflows/content.comments.pipeline/workflow.json".to_string(),
            registered_at: "2026-07-08T00:00:00Z".to_string(),
            validator_version: "validator".to_string(),
        }
    }

    fn pipeline_workflow_request() -> WorkflowExecutionRequest {
        WorkflowExecutionRequest {
            kind: "workflow_execution_request".to_string(),
            schema_version: "1.0.0".to_string(),
            request_id: "pipeline-request".to_string(),
            workflow_id: "content.comments.pipeline".to_string(),
            workflow_version: "1.0.0".to_string(),
            scope: WorkflowLookupScope::PublicOnly,
            input: json!({"note": "Hello world"}),
            governing_spec: "007-workflow-registry-traversal".to_string(),
        }
    }

    struct PipelineExecutor;

    impl LocalExecutor for PipelineExecutor {
        fn execute(
            &self,
            capability: &ResolvedCapability,
            _input: &Value,
        ) -> Result<LocalExecutionOutput, LocalExecutionFailure> {
            let value = match capability.record.id.as_str() {
                "content.comments.pipeline-validate" => json!({"valid": true, "issues": []}),
                "content.comments.pipeline-process" => json!({
                    "title": "Hello world",
                    "tags": ["hello", "world"],
                    "noteType": "fleeting",
                    "suggestedNextAction": "archive",
                    "status": "complete"
                }),
                _ => json!({"summary": "Hello world (fleeting)", "wordCount": 3}),
            };
            Ok(LocalExecutionOutput {
                value,
                emitted_events: Vec::new(),
            })
        }
    }

    struct FailingPipelineExecutor;

    impl LocalExecutor for FailingPipelineExecutor {
        fn execute(
            &self,
            capability: &ResolvedCapability,
            _input: &Value,
        ) -> Result<LocalExecutionOutput, LocalExecutionFailure> {
            match capability.record.id.as_str() {
                "content.comments.pipeline-validate" => Ok(LocalExecutionOutput {
                    value: json!({"valid": true, "issues": []}),
                    emitted_events: Vec::new(),
                }),
                other => Err(LocalExecutionFailure {
                    code: LocalExecutionFailureCode::ExecutionFailed,
                    message: format!("step failed: {other}"),
                }),
            }
        }
    }

    fn capability_registry_fixture() -> CapabilityRegistry {
        build_capability_registry(false, None)
    }

    fn signed_missing_binary_capability_registry_fixture() -> CapabilityRegistry {
        build_capability_registry(
            false,
            Some(ArtifactSignature {
                scheme: ArtifactSignatureScheme::Ed25519,
                public_key_hex: Some("00".repeat(32)),
                signature_hex: Some("00".repeat(64)),
                sigstore_bundle_ref: None,
            }),
        )
    }

    #[allow(clippy::too_many_lines, clippy::needless_pass_by_value)]
    fn build_capability_registry(
        strict_inputs: bool,
        signature: Option<ArtifactSignature>,
    ) -> CapabilityRegistry {
        let mut registry = CapabilityRegistry::new();
        for (id, emits, output, required_key) in [
            (
                "content.comments.create-comment-draft",
                vec![EventReference {
                    event_id: "content.comments.draft-created".to_string(),
                    version: "1.0.0".to_string(),
                }],
                json!({
                    "type": "object",
                    "properties": {
                        "draft_id": { "type": "string" },
                        "emitted_events": { "type": "array" }
                    },
                    "required": ["draft_id"],
                    "additionalProperties": true
                }),
                "comment_text",
            ),
            (
                "content.comments.validate-comment",
                vec![EventReference {
                    event_id: "content.comments.validated".to_string(),
                    version: "1.0.0".to_string(),
                }],
                json!({
                    "type": "object",
                    "properties": {
                        "draft_id": { "type": "string" },
                        "emitted_events": { "type": "array" }
                    },
                    "required": ["draft_id"],
                    "additionalProperties": true
                }),
                "draft_id",
            ),
            (
                "content.comments.persist-comment",
                vec![],
                json!({
                    "type": "object",
                    "properties": { "comment_id": { "type": "string" } },
                    "required": ["comment_id"],
                    "additionalProperties": true
                }),
                "draft_id",
            ),
        ] {
            register_capability_ok(
                &mut registry,
                CapabilityRegistration {
                    scope: RegistryScope::Public,
                    contract: capability_contract(
                        id,
                        emits,
                        json!({
                            "type": "object",
                            "properties": {
                                "comment_text": { "type": "string" },
                                "draft_id": { "type": "string" }
                            },
                            "required": if strict_inputs {
                                vec![required_key]
                            } else {
                                Vec::<&str>::new()
                            },
                            "additionalProperties": true
                        }),
                        output,
                    ),
                    contract_path: format!("registry/{id}.json"),
                    artifact: CapabilityArtifactRecord {
                        artifact_ref: format!("artifact-{id}"),
                        implementation_kind: ImplementationKind::Executable,
                        source: SourceReference {
                            kind: SourceKind::Git,
                            location: format!("https://example.com/{id}.git"),
                        },
                        binary: Some(BinaryReference {
                            format: BinaryFormat::Wasm,
                            location: format!("{id}.wasm"),
                            signature: signature.clone(),
                        }),
                        workflow_ref: None,
                        digests: ArtifactDigests {
                            source_digest: "source".to_string(),
                            binary_digest: Some("binary".to_string()),
                        },
                        provenance: RegistryProvenance {
                            source: "fixtures".to_string(),
                            author: "Enrico".to_string(),
                            created_at: "2026-03-27T00:00:00Z".to_string(),
                        },
                    },
                    registered_at: "2026-03-27T00:00:00Z".to_string(),
                    tags: vec!["comments".to_string()],
                    composability: ComposabilityMetadata {
                        kind: CompositionKind::Atomic,
                        patterns: vec![CompositionPattern::Sequential],
                        provides: vec!["comment".to_string()],
                        requires: Vec::new(),
                    },
                    governing_spec: "005-capability-registry".to_string(),
                    validator_version: "validator".to_string(),
                },
            );
        }
        registry
    }

    fn strict_input_capability_registry_fixture() -> CapabilityRegistry {
        build_capability_registry(true, None)
    }

    fn workflow_registry_fixture() -> WorkflowRegistry {
        let registry = capability_registry_fixture();
        let mut workflows = WorkflowRegistry::new();
        register_workflow_ok(
            &mut workflows,
            &registry,
            WorkflowRegistration {
                scope: RegistryScope::Public,
                definition: workflow_definition_fixture(
                    Some(EventReference {
                        event_id: "content.comments.validated".to_string(),
                        version: "1.0.0".to_string(),
                    }),
                    None,
                ),
                workflow_path: "workflows/publish-comment.json".to_string(),
                registered_at: "2026-03-27T00:00:00Z".to_string(),
                validator_version: "workflow-validator".to_string(),
            },
        );
        workflows
    }

    fn workflow_definition_fixture(
        second_event: Option<EventReference>,
        direct_edge: Option<WorkflowEdge>,
    ) -> WorkflowDefinition {
        let mut edges = vec![
            WorkflowEdge {
                edge_id: "draft_to_validate".to_string(),
                from: "create_draft".to_string(),
                to: "validate_comment".to_string(),
                trigger: WorkflowEdgeTrigger::Event,
                event: Some(EventReference {
                    event_id: "content.comments.draft-created".to_string(),
                    version: "1.0.0".to_string(),
                }),
                predicate: None,
            },
            WorkflowEdge {
                edge_id: "validate_to_persist".to_string(),
                from: "validate_comment".to_string(),
                to: "persist_comment".to_string(),
                trigger: WorkflowEdgeTrigger::Event,
                event: second_event,
                predicate: None,
            },
        ];
        if let Some(edge) = direct_edge {
            edges.push(edge);
        }
        WorkflowDefinition {
            kind: "workflow_definition".to_string(),
            schema_version: "1.0.0".to_string(),
            id: "content.comments.publish-comment".to_string(),
            name: "publish-comment".to_string(),
            version: "1.0.0".to_string(),
            lifecycle: Lifecycle::Active,
            owner: Owner {
                team: "comments".to_string(),
                contact: "comments@example.com".to_string(),
            },
            summary: "Publish a comment deterministically.".to_string(),
            inputs: SchemaContainer {
                schema: json!({
                    "type": "object",
                    "properties": { "comment_text": { "type": "string" } },
                    "required": ["comment_text"],
                    "additionalProperties": true
                }),
            },
            outputs: SchemaContainer {
                schema: json!({
                    "type": "object",
                    "properties": { "comment_id": { "type": "string" } },
                    "required": ["comment_id"],
                    "additionalProperties": true
                }),
            },
            nodes: vec![
                WorkflowNode {
                    node_id: "create_draft".to_string(),
                    capability_id: "content.comments.create-comment-draft".to_string(),
                    capability_version: "1.0.0".to_string(),
                    input: WorkflowNodeInput {
                        from_workflow_input: vec!["comment_text".to_string()],
                    },
                    output: WorkflowNodeOutput {
                        to_workflow_state: vec!["draft_id".to_string()],
                        publish_to_state_as: None,
                    },
                },
                WorkflowNode {
                    node_id: "validate_comment".to_string(),
                    capability_id: "content.comments.validate-comment".to_string(),
                    capability_version: "1.0.0".to_string(),
                    input: WorkflowNodeInput {
                        from_workflow_input: vec!["draft_id".to_string()],
                    },
                    output: WorkflowNodeOutput {
                        to_workflow_state: vec!["draft_id".to_string()],
                        publish_to_state_as: None,
                    },
                },
                WorkflowNode {
                    node_id: "persist_comment".to_string(),
                    capability_id: "content.comments.persist-comment".to_string(),
                    capability_version: "1.0.0".to_string(),
                    input: WorkflowNodeInput {
                        from_workflow_input: vec!["draft_id".to_string()],
                    },
                    output: WorkflowNodeOutput {
                        to_workflow_state: vec!["comment_id".to_string()],
                        publish_to_state_as: None,
                    },
                },
            ],
            edges,
            start_node: "create_draft".to_string(),
            terminal_nodes: vec!["persist_comment".to_string()],
            output_projection: Vec::new(),
            tags: vec!["comments".to_string()],
            governing_spec: "007-workflow-registry-traversal".to_string(),
        }
    }

    fn capability_contract(
        id: &str,
        emits: Vec<EventReference>,
        inputs: Value,
        outputs: Value,
    ) -> CapabilityContract {
        // Subscribable requires a non-empty event_trigger (registry
        // ContractValidationFailed otherwise); only capabilities that
        // actually declare `emits` need to be Subscribable at all (spec
        // 101-local-executor-event-emission FR-007/FR-008 rejects a
        // non-Subscribable capability's emitted events).
        let has_emits = !emits.is_empty();
        CapabilityContract {
            kind: "capability_contract".to_string(),
            schema_version: "1.0.0".to_string(),
            id: id.to_string(),
            namespace: "content.comments".to_string(),
            name: id.rsplit('.').next().unwrap_or("capability").to_string(),
            version: "1.0.0".to_string(),
            lifecycle: Lifecycle::Active,
            owner: Owner {
                team: "comments".to_string(),
                contact: "comments@example.com".to_string(),
            },
            summary: "workflow fixture capability".to_string(),
            description: "workflow fixture capability used in runtime tests".to_string(),
            inputs: SchemaContainer { schema: inputs },
            outputs: SchemaContainer { schema: outputs },
            preconditions: vec![Condition {
                id: "precondition".to_string(),
                description: "must be valid".to_string(),
            }],
            postconditions: vec![Condition {
                id: "postcondition".to_string(),
                description: "must produce output".to_string(),
            }],
            side_effects: vec![SideEffect {
                kind: SideEffectKind::MemoryOnly,
                description: "memory only".to_string(),
            }],
            emits,
            consumes: Vec::new(),
            permissions: vec![IdReference {
                id: "permission".to_string(),
            }],
            execution: Execution {
                binary_format: ContractBinaryFormat::Wasm,
                entrypoint: Entrypoint {
                    kind: EntrypointKind::WasiCommand,
                    command: "run".to_string(),
                },
                preferred_targets: vec![ExecutionTarget::Local],
                constraints: ExecutionConstraints {
                    host_api_access: HostApiAccess::None,
                    network_access: NetworkAccess::Forbidden,
                    filesystem_access: FilesystemAccess::None,
                },
            },
            policies: Vec::new(),
            dependencies: Vec::new(),
            provenance: Provenance {
                source: ProvenanceSource::Greenfield,
                author: "Enrico".to_string(),
                created_at: "2026-03-27T00:00:00Z".to_string(),
                spec_ref: Some("007-workflow-registry-traversal".to_string()),
                adr_refs: Vec::new(),
                exception_refs: Vec::new(),
            },
            evidence: vec![ValidationEvidence {
                evidence_id: "evidence".to_string(),
                evidence_type: EvidenceType::ContractValidation,
                status: EvidenceStatus::Passed,
            }],
            service_type: if has_emits {
                ServiceType::Subscribable
            } else {
                ServiceType::Stateless
            },
            permitted_targets: vec![
                ExecutionTarget::Local,
                ExecutionTarget::Cloud,
                ExecutionTarget::Edge,
                ExecutionTarget::Device,
            ],
            event_trigger: if has_emits {
                Some(format!("{id}.triggered"))
            } else {
                None
            },
            connector_requirements: Vec::new(),
            state_schema: None,
            use_cases: Vec::new(),
            risk: traverse_contracts::default_risk_metadata(),
        }
    }

    fn valid_workflow_request() -> WorkflowExecutionRequest {
        WorkflowExecutionRequest {
            kind: "workflow_execution_request".to_string(),
            schema_version: "1.0.0".to_string(),
            request_id: "workflow-request".to_string(),
            workflow_id: "content.comments.publish-comment".to_string(),
            workflow_version: "1.0.0".to_string(),
            scope: WorkflowLookupScope::PublicOnly,
            input: json!({"comment_text": "hello"}),
            governing_spec: "007-workflow-registry-traversal".to_string(),
        }
    }

    fn valid_runtime_request() -> RuntimeRequest {
        RuntimeRequest {
            kind: "runtime_request".to_string(),
            schema_version: "1.0.0".to_string(),
            request_id: "runtime-request".to_string(),
            intent: RuntimeIntent {
                capability_id: Some("content.comments.publish-comment".to_string()),
                capability_version: Some("1.0.0".to_string()),
                version_range: None,
                intent_key: None,
            },
            input: json!({"comment_text": "hello"}),
            lookup: RuntimeLookup {
                scope: RuntimeLookupScope::PublicOnly,
                allow_ambiguity: false,
            },
            context: RuntimeContext {
                requested_target: crate::PlacementTarget::Local,
                correlation_id: None,
                caller: None,
                traceparent: None,
                tracestate: None,
                metadata: None,
                identity: None,
            },
            governing_spec: "006-runtime-request-execution".to_string(),
        }
    }

    struct WorkflowExecutor;

    impl LocalExecutor for WorkflowExecutor {
        fn execute(
            &self,
            capability: &ResolvedCapability,
            _input: &Value,
        ) -> Result<LocalExecutionOutput, LocalExecutionFailure> {
            let (value, emitted_events) = match capability.record.id.as_str() {
                "content.comments.create-comment-draft" => (
                    json!({"draft_id": "draft-1"}),
                    vec![sample_traverse_event(
                        "content.comments.draft-created",
                        "1.0.0",
                    )],
                ),
                "content.comments.validate-comment" => (
                    json!({"draft_id": "draft-1"}),
                    vec![sample_traverse_event("content.comments.validated", "1.0.0")],
                ),
                "content.comments.persist-comment" => {
                    (json!({"comment_id": "comment-1"}), Vec::new())
                }
                _ => (json!({}), Vec::new()),
            };
            Ok(LocalExecutionOutput {
                value,
                emitted_events,
            })
        }
    }

    struct FailingWorkflowExecutor;

    impl LocalExecutor for FailingWorkflowExecutor {
        fn execute(
            &self,
            _capability: &ResolvedCapability,
            _input: &Value,
        ) -> Result<LocalExecutionOutput, LocalExecutionFailure> {
            Err(LocalExecutionFailure {
                code: LocalExecutionFailureCode::ExecutionFailed,
                message: "boom".to_string(),
            })
        }
    }

    struct MissingEventWorkflowExecutor;

    struct BadOutputWorkflowExecutor;

    impl LocalExecutor for MissingEventWorkflowExecutor {
        fn execute(
            &self,
            capability: &ResolvedCapability,
            _input: &Value,
        ) -> Result<LocalExecutionOutput, LocalExecutionFailure> {
            let (value, emitted_events) = match capability.record.id.as_str() {
                "content.comments.create-comment-draft" => (
                    json!({"draft_id": "draft-1"}),
                    vec![sample_traverse_event(
                        "content.comments.draft-created",
                        "1.0.0",
                    )],
                ),
                "content.comments.validate-comment" => (json!({"draft_id": "draft-1"}), Vec::new()),
                "content.comments.persist-comment" => {
                    (json!({"comment_id": "comment-1"}), Vec::new())
                }
                _ => (json!({}), Vec::new()),
            };
            Ok(LocalExecutionOutput {
                value,
                emitted_events,
            })
        }
    }

    impl LocalExecutor for BadOutputWorkflowExecutor {
        fn execute(
            &self,
            capability: &ResolvedCapability,
            _input: &Value,
        ) -> Result<LocalExecutionOutput, LocalExecutionFailure> {
            let (value, emitted_events) = match capability.record.id.as_str() {
                "content.comments.create-comment-draft" => (
                    json!({}),
                    vec![sample_traverse_event(
                        "content.comments.draft-created",
                        "1.0.0",
                    )],
                ),
                _ => (json!({}), Vec::new()),
            };
            Ok(LocalExecutionOutput {
                value,
                emitted_events,
            })
        }
    }

    struct UndeclaredEventWorkflowExecutor;

    impl LocalExecutor for UndeclaredEventWorkflowExecutor {
        fn execute(
            &self,
            _capability: &ResolvedCapability,
            _input: &Value,
        ) -> Result<LocalExecutionOutput, LocalExecutionFailure> {
            // The workflow under test in `workflow_failures_cover_not_found_missing_events_and_step_failures`
            // fails FR-007/FR-008 validation at its start node, so this
            // executor is only ever invoked for
            // "content.comments.create-comment-draft" and never reaches a
            // second node.
            Ok(LocalExecutionOutput {
                value: json!({"draft_id": "draft-1"}),
                emitted_events: vec![sample_traverse_event(
                    "content.comments.undeclared-event",
                    "1.0.0",
                )],
            })
        }
    }

    fn register_capability_ok(registry: &mut CapabilityRegistry, request: CapabilityRegistration) {
        match registry.register(request) {
            Ok(_) => {}
            Err(error) => unreachable!("{error:?}"),
        }
    }

    fn register_workflow_ok(
        registry: &mut WorkflowRegistry,
        capabilities: &CapabilityRegistry,
        request: WorkflowRegistration,
    ) {
        match registry.register(capabilities, request) {
            Ok(_) => {}
            Err(error) => unreachable!("{error:?}"),
        }
    }

    #[test]
    fn helper_guards_cover_unreachable_branches() {
        let capability_panic = std::panic::catch_unwind(|| {
            register_capability_ok(
                &mut CapabilityRegistry::new(),
                CapabilityRegistration {
                    scope: RegistryScope::Public,
                    contract: capability_contract("bad", Vec::new(), json!({}), json!({})),
                    contract_path: String::new(),
                    artifact: workflow_artifact_record("bad", "1.0.0", "artifact"),
                    registered_at: String::new(),
                    tags: Vec::new(),
                    composability: ComposabilityMetadata {
                        kind: CompositionKind::Atomic,
                        patterns: Vec::new(),
                        provides: Vec::new(),
                        requires: Vec::new(),
                    },
                    governing_spec: "005-capability-registry".to_string(),
                    validator_version: "validator".to_string(),
                },
            );
        });
        assert!(capability_panic.is_err());

        let workflow_panic = std::panic::catch_unwind(|| {
            register_workflow_ok(
                &mut WorkflowRegistry::new(),
                &CapabilityRegistry::new(),
                WorkflowRegistration {
                    scope: RegistryScope::Public,
                    definition: workflow_definition_fixture(
                        None,
                        Some(WorkflowEdge {
                            edge_id: "direct".to_string(),
                            from: "create_draft".to_string(),
                            to: "validate_comment".to_string(),
                            trigger: WorkflowEdgeTrigger::Direct,
                            event: None,
                            predicate: None,
                        }),
                    ),
                    workflow_path: String::new(),
                    registered_at: String::new(),
                    validator_version: "validator".to_string(),
                },
            );
        });
        assert!(workflow_panic.is_err());
    }

    fn resolved_workflow(definition: WorkflowDefinition) -> ResolvedWorkflow {
        ResolvedWorkflow {
            record: WorkflowRegistryRecord {
                scope: RegistryScope::Public,
                id: definition.id.clone(),
                version: definition.version.clone(),
                lifecycle: definition.lifecycle.clone(),
                owner: definition.owner.clone(),
                workflow_path: "workflows/manual.json".to_string(),
                workflow_digest: "digest".to_string(),
                registered_at: "2026-03-27T00:00:00Z".to_string(),
                governing_spec: "007-workflow-registry-traversal".to_string(),
                validator_version: "validator".to_string(),
                evidence: traverse_registry::WorkflowRegistrationEvidence {
                    evidence_id: "evidence".to_string(),
                    workflow_id: definition.id.clone(),
                    workflow_version: definition.version.clone(),
                    scope: RegistryScope::Public,
                    governing_spec: "007-workflow-registry-traversal".to_string(),
                    validator_version: "validator".to_string(),
                    produced_at: "2026-03-27T00:00:00Z".to_string(),
                    result: traverse_registry::WorkflowRegistrationResult::Passed,
                },
            },
            index_entry: traverse_registry::WorkflowDiscoveryIndexEntry {
                scope: RegistryScope::Public,
                id: definition.id.clone(),
                version: definition.version.clone(),
                lifecycle: definition.lifecycle.clone(),
                owner: definition.owner.clone(),
                summary: definition.summary.clone(),
                tags: definition.tags.clone(),
                participating_capabilities: definition
                    .nodes
                    .iter()
                    .map(|node| node.capability_id.clone())
                    .collect(),
                events_used: Vec::new(),
                start_node: definition.start_node.clone(),
                terminal_nodes: definition.terminal_nodes.clone(),
                registered_at: "2026-03-27T00:00:00Z".to_string(),
            },
            definition,
        }
    }
}
