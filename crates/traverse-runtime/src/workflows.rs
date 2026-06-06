use crate::{
    ExecutionFailureReason, ExecutionFailureState, LocalExecutor, Runtime, RuntimeError,
    RuntimeErrorCode, RuntimeExecutionOutcome, execution_failure_outcome, runtime_error,
    successful_execution_outcome, validate_payload_against_contract,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;
use traverse_contracts::EventReference;
use traverse_registry::{
    LookupScope, RegistryScope, ResolvedCapability, ResolvedWorkflow, WorkflowEdge,
    WorkflowEdgePredicate, WorkflowEdgeTrigger, WorkflowNode,
};

const WORKFLOW_REQUEST_KIND: &str = "workflow_execution_request";
const WORKFLOW_EVIDENCE_KIND: &str = "workflow_traversal_evidence";
const WORKFLOW_SCHEMA_VERSION: &str = "1.0.0";
const WORKFLOW_GOVERNING_SPEC: &str = "007-workflow-registry-traversal";

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
            );
        }

        match self.traverse_workflow(&request, &workflow) {
            Ok(success) => success,
            Err(failure) => failure,
        }
    }

    pub(crate) fn execute_workflow_capability(
        &self,
        context: crate::ExecutionContext,
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
                ));
            };

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
                    ));
                }
            };

            if let Err(error) = validate_payload_against_contract(
                &output,
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
                ));
            }

            update_state(&mut state, node, &output);
            let node_emitted = emitted_events(&output);
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
            let evaluation = evaluate_event_driven_edges(
                &waiting_edges,
                &node_emitted,
                &mut consumed_event_edges,
                &format!("{}:step:{step_index}", request.request_id),
            );
            event_evidence
                .match_records
                .extend(evaluation.evidence.match_records.iter().cloned());
            event_evidence
                .wake_decisions
                .extend(evaluation.evidence.wake_decisions.iter().cloned());
            event_evidence
                .consumptions
                .extend(evaluation.evidence.consumptions.iter().cloned());
            let matched_event_edges = outgoing
                .iter()
                .filter(|edge| {
                    evaluation
                        .taken_edge_ids
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
                ));
            }
            if let Some(edge) = matched_event_edges.into_iter().next() {
                traversed.push(edge_record(&edge));
                current = edge.to;
                step_index += 1;
                continue;
            }

            if workflow.definition.terminal_nodes.contains(&node.node_id) {
                let final_output = Value::Object(state.clone());
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
}

fn emitted_events(output: &Value) -> Vec<EmittedEventRecord> {
    let Value::Object(object) = output else {
        return Vec::new();
    };
    let Some(Value::Array(events)) = object.get("emitted_events") else {
        return Vec::new();
    };
    events
        .iter()
        .enumerate()
        .filter_map(|(index, event)| {
            let Value::Object(event) = event else {
                return None;
            };
            Some(EmittedEventRecord {
                record_id: format!("event_record_{index}"),
                event: EventReference {
                    event_id: event.get("event_id")?.as_str()?.to_string(),
                    version: event.get("version")?.as_str()?.to_string(),
                },
                payload: event.get("payload").cloned(),
            })
        })
        .collect()
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
            }));
        return outcome;
    }

    let mut wake_order = 1;
    for (event_index, emitted_event) in emitted_events.iter().enumerate() {
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
            });
            outcome.evidence.consumptions.push(EventConsumptionRecord {
                event_id: emitted_event.event.event_id.clone(),
                event_version: emitted_event.event.version.clone(),
                edge_id: waiting_edge.edge_id.clone(),
                workflow_execution_id: waiting_edge.workflow_execution_id.clone(),
                consumed_at: wake_recorded_at,
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

fn workflow_failure(
    request: &WorkflowExecutionRequest,
    failure_reason: WorkflowTraversalFailureReason,
    error: RuntimeError,
    visited_nodes: Vec<WorkflowTraversalStepRecord>,
    traversed_edges: Vec<WorkflowTraversalEdgeRecord>,
    emitted_events: Vec<EventReference>,
    event_evidence: WorkflowEventEvidenceBundle,
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
        },
        evidence,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        CandidateCollectionRecord, LocalExecutionFailure, LocalExecutionFailureCode,
        RuntimeContext, RuntimeIntent, RuntimeLookup, RuntimeLookupScope, RuntimeRequest,
        RuntimeResultStatus, SelectionRecord,
    };
    use serde_json::json;
    use traverse_contracts::{
        BinaryFormat as ContractBinaryFormat, CapabilityContract, Condition, Entrypoint,
        EntrypointKind, EventReference, EvidenceStatus, EvidenceType, Execution,
        ExecutionConstraints, ExecutionTarget, FilesystemAccess, HostApiAccess, IdReference,
        Lifecycle, NetworkAccess, Owner, Provenance, ProvenanceSource, SchemaContainer,
        ServiceType, SideEffect, SideEffectKind, ValidationEvidence,
    };
    use traverse_registry::{
        ArtifactDigests, BinaryFormat, BinaryReference, CapabilityArtifactRecord,
        CapabilityRegistration, CapabilityRegistry, ComposabilityMetadata, CompositionKind,
        CompositionPattern, ImplementationKind, RegistryProvenance, RegistryScope, SourceKind,
        SourceReference, WorkflowDefinition, WorkflowEdge, WorkflowEdgeTrigger, WorkflowNode,
        WorkflowNodeInput, WorkflowNodeOutput, WorkflowRegistration, WorkflowRegistry,
        WorkflowRegistryRecord, workflow_artifact_record,
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
    fn workflow_helpers_cover_state_and_event_extraction_paths() {
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
            },
        };
        assert_eq!(node_input(&state, &node), json!({"comment_text": "hello"}));
        update_state(&mut state, &node, &json!({"draft_id": "draft-1"}));
        assert_eq!(state.get("draft_id"), Some(&json!("draft-1")));
        update_state(&mut state, &node, &json!("not-an-object"));

        assert!(emitted_events(&json!("nope")).is_empty());
        assert_eq!(
            emitted_events(&json!({
                "emitted_events": [
                    "bad",
                    {
                        "event_id": "content.comments.draft-created",
                        "version": "1.0.0",
                        "payload": {"severity": "normal"}
                    },
                    {"event_id": "bad"}
                ]
            })),
            vec![EmittedEventRecord {
                record_id: "event_record_1".to_string(),
                event: EventReference {
                    event_id: "content.comments.draft-created".to_string(),
                    version: "1.0.0".to_string(),
                },
                payload: Some(json!({"severity": "normal"})),
            }]
        );

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
    fn executes_workflow_deterministically_and_supports_workflow_backed_capabilities() {
        let workflow_registry = workflow_registry_fixture();
        let runtime = Runtime::new(capability_registry_fixture(), WorkflowExecutor)
            .with_workflow_registry(workflow_registry);

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
            .with_workflow_registry(workflow_registry_fixture());
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
        let runtime = Runtime::new(capability_registry_fixture(), MissingEventWorkflowExecutor)
            .with_workflow_registry(workflow_registry);
        let missing_event = runtime.execute_workflow(valid_workflow_request());
        assert_eq!(
            missing_event.evidence.result.failure_reason,
            Some(WorkflowTraversalFailureReason::MissingRequiredEvent)
        );

        let runtime = Runtime::new(capability_registry_fixture(), FailingWorkflowExecutor)
            .with_workflow_registry(workflow_registry_fixture());
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
            }]
        );
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn workflow_runtime_covers_additional_failure_and_helper_branches() {
        let runtime = Runtime::new(capability_registry_fixture(), WorkflowExecutor)
            .with_workflow_registry(workflow_registry_fixture());

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
                .with_workflow_registry(workflow_registry_fixture());
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
                .with_workflow_registry(workflow_registry_fixture());
        let bad_output = bad_output_runtime.execute_workflow(valid_workflow_request());
        assert_eq!(
            bad_output.evidence.result.failure_reason,
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
        );
        let failing_runtime = Runtime::new(capability_registry_fixture(), FailingWorkflowExecutor)
            .with_workflow_registry(workflow_registry_fixture());
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

    fn capability_registry_fixture() -> CapabilityRegistry {
        build_capability_registry(false)
    }

    #[allow(clippy::too_many_lines)]
    fn build_capability_registry(strict_inputs: bool) -> CapabilityRegistry {
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
                    contract_path: format!("contracts/{id}.json"),
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
        build_capability_registry(true)
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
                    },
                },
            ],
            edges,
            start_node: "create_draft".to_string(),
            terminal_nodes: vec!["persist_comment".to_string()],
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
            service_type: ServiceType::Stateless,
            permitted_targets: vec![
                ExecutionTarget::Local,
                ExecutionTarget::Cloud,
                ExecutionTarget::Edge,
                ExecutionTarget::Device,
            ],
            event_trigger: None,
            connector_requirements: Vec::new(),
            state_schema: None,
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
        ) -> Result<Value, LocalExecutionFailure> {
            let output = match capability.record.id.as_str() {
                "content.comments.create-comment-draft" => json!({
                    "draft_id": "draft-1",
                    "emitted_events": [
                        {"event_id": "content.comments.draft-created", "version": "1.0.0"}
                    ]
                }),
                "content.comments.validate-comment" => json!({
                    "draft_id": "draft-1",
                    "emitted_events": [
                        {"event_id": "content.comments.validated", "version": "1.0.0"}
                    ]
                }),
                "content.comments.persist-comment" => json!({
                    "comment_id": "comment-1"
                }),
                _ => json!({}),
            };
            Ok(output)
        }
    }

    struct FailingWorkflowExecutor;

    impl LocalExecutor for FailingWorkflowExecutor {
        fn execute(
            &self,
            _capability: &ResolvedCapability,
            _input: &Value,
        ) -> Result<Value, LocalExecutionFailure> {
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
        ) -> Result<Value, LocalExecutionFailure> {
            let output = match capability.record.id.as_str() {
                "content.comments.create-comment-draft" => json!({
                    "draft_id": "draft-1",
                    "emitted_events": [
                        {"event_id": "content.comments.draft-created", "version": "1.0.0"}
                    ]
                }),
                "content.comments.validate-comment" => json!({
                    "draft_id": "draft-1"
                }),
                "content.comments.persist-comment" => json!({
                    "comment_id": "comment-1"
                }),
                _ => json!({}),
            };
            Ok(output)
        }
    }

    impl LocalExecutor for BadOutputWorkflowExecutor {
        fn execute(
            &self,
            capability: &ResolvedCapability,
            _input: &Value,
        ) -> Result<Value, LocalExecutionFailure> {
            let output = match capability.record.id.as_str() {
                "content.comments.create-comment-draft" => json!({
                    "emitted_events": [
                        {"event_id": "content.comments.draft-created", "version": "1.0.0"}
                    ]
                }),
                _ => json!({}),
            };
            Ok(output)
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
