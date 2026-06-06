//! Runtime control-plane support for Traverse.

mod workflows;
pub use workflows::*;
pub mod data_store;
pub mod events;
pub mod executor;
pub mod placement;
pub mod router;
pub mod trace;

use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::fmt;
use traverse_contracts::{
    ExecutionTarget, HostApiAccess, Lifecycle, NetworkAccess, ViolationRecord,
};
use traverse_registry::{
    CapabilityRegistration, CapabilityRegistry, DiscoveryQuery, ImplementationKind, LookupScope,
    RegistrationOutcome, RegistryFailure, RegistryScope, ResolutionError, ResolvedCapability,
    WorkflowFailure, WorkflowRegistration, WorkflowRegistrationOutcome, WorkflowRegistry,
    resolve_dependencies, resolve_version_range,
};

const RUNTIME_REQUEST_KIND: &str = "runtime_request";
const RUNTIME_RESULT_KIND: &str = "runtime_result";
const RUNTIME_STATE_EVENT_KIND: &str = "runtime_state_event";
const RUNTIME_TRACE_KIND: &str = "runtime_trace";
const RUNTIME_STATE_MACHINE_VALIDATION_KIND: &str = "runtime_state_machine_validation";
const BROWSER_SUBSCRIPTION_REQUEST_KIND: &str = "browser_runtime_subscription_request";
const BROWSER_SUBSCRIPTION_ERROR_KIND: &str = "browser_runtime_subscription_error";
const BROWSER_SUBSCRIPTION_LIFECYCLE_KIND: &str = "browser_runtime_subscription_lifecycle";
const BROWSER_SUBSCRIPTION_STATE_KIND: &str = "browser_runtime_subscription_state";
const BROWSER_SUBSCRIPTION_TRACE_KIND: &str = "browser_runtime_subscription_trace_artifact";
const BROWSER_SUBSCRIPTION_TERMINAL_KIND: &str = "browser_runtime_subscription_terminal";
const SUPPORTED_SCHEMA_VERSION: &str = "1.0.0";
const GOVERNING_SPEC: &str = "006-runtime-request-execution";
const STATE_MACHINE_GOVERNING_SPEC: &str = "010-runtime-state-machine";
const BROWSER_SUBSCRIPTION_GOVERNING_SPEC: &str = "013-browser-runtime-subscription";
const EXECUTION_PREFIX: &str = "exec_";
const TRACE_PREFIX: &str = "trace_";

#[derive(Debug, Clone)]
pub struct Runtime<E> {
    registry: CapabilityRegistry,
    workflow_registry: WorkflowRegistry,
    executor: E,
    observability: RuntimeObservabilityConfig,
}

impl<E> Runtime<E> {
    #[must_use]
    pub fn new(registry: CapabilityRegistry, executor: E) -> Self {
        Self {
            registry,
            workflow_registry: WorkflowRegistry::new(),
            executor,
            observability: RuntimeObservabilityConfig::default(),
        }
    }

    #[must_use]
    pub fn with_workflow_registry(mut self, workflow_registry: WorkflowRegistry) -> Self {
        self.workflow_registry = workflow_registry;
        self
    }

    #[must_use]
    pub fn with_observability_config(mut self, observability: RuntimeObservabilityConfig) -> Self {
        self.observability = observability;
        self
    }

    #[must_use]
    pub fn observability_config(&self) -> &RuntimeObservabilityConfig {
        &self.observability
    }

    /// Returns a reference to the capability registry.
    #[must_use]
    pub fn capability_registry(&self) -> &CapabilityRegistry {
        &self.registry
    }

    /// Registers a capability into the runtime's registry.
    ///
    /// Returns `true` when the capability was newly registered, `false` when
    /// the same contract digest was already present (idempotent no-op).
    ///
    /// # Errors
    ///
    /// Returns [`RegistryFailure`] when contract validation fails or when a
    /// different contract digest conflicts with an existing immutable version.
    pub fn register_capability(
        &mut self,
        registration: CapabilityRegistration,
    ) -> Result<RegistrationOutcome, RegistryFailure> {
        self.registry.register(registration)
    }

    /// Returns a reference to the workflow registry.
    #[must_use]
    pub fn workflow_registry(&self) -> &WorkflowRegistry {
        &self.workflow_registry
    }

    /// Returns a mutable reference to the workflow registry.
    #[must_use]
    pub fn workflow_registry_mut(&mut self) -> &mut WorkflowRegistry {
        &mut self.workflow_registry
    }

    /// Registers a workflow into the runtime's workflow registry.
    ///
    /// # Errors
    ///
    /// Returns [`WorkflowFailure`] when the workflow is invalid, references a
    /// missing capability, contains a cycle, or violates immutability.
    pub fn register_workflow(
        &mut self,
        registration: WorkflowRegistration,
    ) -> Result<WorkflowRegistrationOutcome, WorkflowFailure> {
        self.workflow_registry
            .register(&self.registry, registration)
    }
}

pub trait LocalExecutor {
    /// Executes one locally selected capability.
    ///
    /// # Errors
    ///
    /// Returns [`LocalExecutionFailure`] when the executor cannot complete the
    /// selected capability.
    fn execute(
        &self,
        capability: &ResolvedCapability,
        input: &Value,
    ) -> Result<Value, LocalExecutionFailure>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalExecutionFailure {
    pub code: LocalExecutionFailureCode,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalExecutionFailureCode {
    /// The executor failed for an unclassified reason. Retryable with caution.
    ExecutionFailed,
    /// The execution did not complete within the allowed time window.
    /// Transient — retryable with exponential backoff.
    Timeout,
    /// The input provided to the capability was invalid or malformed.
    /// Fatal — do not retry; fix the input before resubmitting.
    InvalidInput,
    /// A required resource (memory, CPU, file handles, etc.) was exhausted.
    /// Transient — retry with a longer backoff interval.
    ResourceExhausted,
    /// A capability contract constraint (precondition, postcondition, or policy) was violated.
    /// Fatal — do not retry; the request violates the contract.
    ConstraintViolated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeRequest {
    pub kind: String,
    pub schema_version: String,
    pub request_id: String,
    pub intent: RuntimeIntent,
    pub input: Value,
    pub lookup: RuntimeLookup,
    pub context: RuntimeContext,
    pub governing_spec: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeIntent {
    #[serde(default)]
    pub capability_id: Option<String>,
    #[serde(default)]
    pub capability_version: Option<String>,
    /// Optional semver range expression (e.g. `^1.0.0`, `>=1.2 <2`).
    /// When present and `capability_version` is absent, the runtime uses
    /// range resolution rather than exact version lookup.
    #[serde(default)]
    pub version_range: Option<String>,
    #[serde(default)]
    pub intent_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeLookup {
    pub scope: RuntimeLookupScope,
    pub allow_ambiguity: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeLookupScope {
    PublicOnly,
    PreferPrivate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeContext {
    pub requested_target: PlacementTarget,
    #[serde(default)]
    pub correlation_id: Option<String>,
    #[serde(default)]
    pub caller: Option<String>,
    #[serde(default)]
    pub traceparent: Option<String>,
    #[serde(default)]
    pub tracestate: Option<String>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeObservabilityConfig {
    pub signals: OTelSignalConfig,
    pub exporter: OTelExporterConfig,
    pub deterministic_ids: bool,
    #[serde(default)]
    pub deterministic_seed: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OTelSignalConfig {
    pub traces_enabled: bool,
    pub logs_enabled: bool,
    pub metrics_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OTelExporterConfig {
    #[serde(default)]
    pub endpoint: Option<String>,
    pub protocol: OtlpProtocol,
}

impl Default for RuntimeObservabilityConfig {
    fn default() -> Self {
        Self {
            signals: OTelSignalConfig {
                traces_enabled: true,
                logs_enabled: false,
                metrics_enabled: false,
            },
            exporter: OTelExporterConfig {
                endpoint: None,
                protocol: OtlpProtocol::Http,
            },
            deterministic_ids: false,
            deterministic_seed: None,
        }
    }
}

impl RuntimeObservabilityConfig {
    #[must_use]
    pub fn deterministic_test(seed: &str) -> Self {
        Self {
            deterministic_ids: true,
            deterministic_seed: Some(seed.to_string()),
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OtlpProtocol {
    Http,
    Grpc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlacementTarget {
    Local,
    Browser,
    Edge,
    Cloud,
    Worker,
    Device,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlacementDecisionRecord {
    pub requested_target: PlacementTarget,
    #[serde(default)]
    pub selected_target: Option<PlacementTarget>,
    pub status: PlacementDecisionStatus,
    pub reason: PlacementDecisionReason,
    pub supported_executor_targets: Vec<PlacementTarget>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlacementDecisionStatus {
    NotAttempted,
    Selected,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlacementDecisionReason {
    SelectionNotReached,
    RequestedTargetSelected,
    RequestedTargetUnsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeStateEvent {
    pub kind: String,
    pub schema_version: String,
    pub event_id: String,
    pub execution_id: String,
    pub request_id: String,
    pub state: RuntimeState,
    pub entered_at: String,
    pub details: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeState {
    Idle,
    LoadingRegistry,
    Ready,
    Discovering,
    EvaluatingConstraints,
    Selecting,
    Executing,
    EmittingEvents,
    Completed,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeTransitionReasonCode {
    RuntimeInitializationStarted,
    RegistryLoaded,
    RegistryLoadFailed,
    RequestStarted,
    CandidatesCollected,
    NoMatch,
    ConstraintsEvaluated,
    ConstraintValidationFailed,
    CandidateSelected,
    SelectionFailed,
    ExecutionSucceededWithEvents,
    ExecutionSucceeded,
    ExecutionFailed,
    EventsEmitted,
    EventEmissionFailed,
    ExecutionClosed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeTransitionRecord {
    pub from_state: RuntimeState,
    pub to_state: RuntimeState,
    pub reason_code: RuntimeTransitionReasonCode,
    pub occurred_at: String,
    #[serde(default)]
    pub request_id: Option<String>,
    #[serde(default)]
    pub execution_id: Option<String>,
    #[serde(default)]
    pub details: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeStateMachineValidationEvidence {
    pub kind: String,
    pub schema_version: String,
    pub governing_spec: String,
    pub validated_at: String,
    pub status: RuntimeStateMachineValidationStatus,
    pub checked_states: Vec<RuntimeState>,
    pub checked_transitions: Vec<String>,
    pub violations: Vec<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeStateMachineValidationStatus {
    Passed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeTrace {
    pub kind: String,
    pub schema_version: String,
    pub trace_id: String,
    pub execution_id: String,
    pub request_id: String,
    pub governing_spec: String,
    pub request: RuntimeRequest,
    pub decision_evidence: TraceDecisionEvidence,
    pub state_progression: TraceStateProgression,
    pub terminal_outcome: TraceTerminalOutcome,
    pub emitted_events: Vec<traverse_contracts::EventReference>,
    #[serde(default)]
    pub workflow_evidence: Option<WorkflowTraversalEvidence>,
    pub state_transitions: Vec<RuntimeTransitionRecord>,
    pub state_machine_validation: RuntimeStateMachineValidationEvidence,
    pub candidate_collection: CandidateCollectionRecord,
    pub selection: SelectionRecord,
    pub execution: ExecutionRecord,
    pub result: TraceResultRecord,
    pub otel_trace: OTelTraceRecord,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OTelTraceRecord {
    pub trace_id: String,
    #[serde(default)]
    pub parent_traceparent: Option<String>,
    #[serde(default)]
    pub tracestate: Option<String>,
    pub exporter: OTelExporterRecord,
    pub spans: Vec<OTelSpanRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OTelExporterRecord {
    pub enabled: bool,
    #[serde(default)]
    pub endpoint: Option<String>,
    pub protocol: OtlpProtocol,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OTelSpanRecord {
    pub trace_id: String,
    pub span_id: String,
    #[serde(default)]
    pub parent_span_id: Option<String>,
    pub name: String,
    pub kind: OTelSpanKind,
    pub status: OTelSpanStatus,
    pub started_at: String,
    pub ended_at: String,
    pub attributes: Vec<OTelAttribute>,
    #[serde(default)]
    pub events: Vec<OTelSpanEvent>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OTelSpanKind {
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OTelSpanStatus {
    Ok,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OTelAttribute {
    pub key: String,
    pub value: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OTelSpanEvent {
    pub name: String,
    pub timestamp: String,
    pub attributes: Vec<OTelAttribute>,
}

impl RuntimeTrace {
    /// Returns the ID of the selected capability, or `None` if no capability was selected.
    #[must_use]
    pub fn selected_capability_id(&self) -> Option<&str> {
        self.selection.selected_capability_id.as_deref()
    }

    /// Returns the error from the terminal outcome, or `None` if execution succeeded.
    #[must_use]
    pub fn errors(&self) -> Option<&RuntimeError> {
        self.terminal_outcome.error.as_ref()
    }

    /// Returns all events emitted during execution.
    #[must_use]
    pub fn emitted_events(&self) -> &[traverse_contracts::EventReference] {
        self.emitted_events.as_slice()
    }

    /// Returns the output value produced by execution, or `None` if unavailable.
    #[must_use]
    pub fn output(&self) -> Option<&serde_json::Value> {
        self.result.output.as_ref()
    }

    /// Returns `true` if the execution completed successfully.
    #[must_use]
    pub fn is_success(&self) -> bool {
        self.terminal_outcome.runtime_status == RuntimeResultStatus::Completed
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceDecisionEvidence {
    pub candidate_collection: CandidateCollectionRecord,
    pub selection: SelectionRecord,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceStateProgression {
    pub state_events: Vec<RuntimeStateEvent>,
    pub transitions: Vec<RuntimeTransitionRecord>,
    pub validation: RuntimeStateMachineValidationEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceTerminalOutcome {
    pub runtime_status: RuntimeResultStatus,
    pub execution_status: ExecutionStatus,
    #[serde(default)]
    pub failure_reason: Option<ExecutionFailureReason>,
    #[serde(default)]
    pub error: Option<RuntimeError>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateCollectionRecord {
    pub lookup_scope: RuntimeLookupScope,
    pub candidates: Vec<RuntimeCandidate>,
    pub rejected_candidates: Vec<RejectedRuntimeCandidate>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeCandidate {
    pub scope: RuntimeRegistryScope,
    pub capability_id: String,
    pub capability_version: String,
    pub artifact_ref: String,
    pub implementation_kind: RuntimeImplementationKind,
    pub lifecycle: RuntimeLifecycle,
    pub reason: CandidateReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RejectedRuntimeCandidate {
    pub capability_id: String,
    pub capability_version: String,
    pub scope: RuntimeRegistryScope,
    pub reason: RejectedCandidateReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateReason {
    ExactMatch,
    IntentMatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RejectedCandidateReason {
    WrongScope,
    NotRunnableLocally,
    LifecycleNotRunnable,
    InputContractInvalid,
    ArtifactMissing,
    SupersededByPrivateOverlay,
    NotSelectedAfterOrdering,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectionRecord {
    pub status: SelectionStatus,
    #[serde(default)]
    pub selected_capability_id: Option<String>,
    #[serde(default)]
    pub selected_capability_version: Option<String>,
    #[serde(default)]
    pub failure_reason: Option<SelectionFailureReason>,
    #[serde(default)]
    pub remaining_candidates: Vec<RuntimeCandidate>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectionStatus {
    Selected,
    NoMatch,
    Ambiguous,
    InvalidRequest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectionFailureReason {
    InvalidRequest,
    NoMatch,
    Ambiguous,
    NotRunnable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionRecord {
    pub placement: PlacementDecisionRecord,
    pub placement_target: PlacementTarget,
    pub status: ExecutionStatus,
    #[serde(default)]
    pub artifact_ref: Option<String>,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub completed_at: Option<String>,
    #[serde(default)]
    pub output_digest: Option<String>,
    #[serde(default)]
    pub failure_reason: Option<ExecutionFailureReason>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatus {
    NotStarted,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionFailureReason {
    ContractInputInvalid,
    ArtifactMissing,
    ArtifactNotRunnable,
    PlacementUnsupported,
    ExecutionFailed,
    ContractOutputInvalid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceResultRecord {
    pub status: RuntimeResultStatus,
    #[serde(default)]
    pub output: Option<serde_json::Value>,
    #[serde(default)]
    pub error: Option<RuntimeError>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeResult {
    pub kind: String,
    pub schema_version: String,
    pub execution_id: String,
    pub request_id: String,
    pub status: RuntimeResultStatus,
    pub trace_ref: String,
    #[serde(default)]
    pub output: Option<Value>,
    #[serde(default)]
    pub error: Option<RuntimeError>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeResultStatus {
    Completed,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeError {
    pub code: RuntimeErrorCode,
    pub message: String,
    pub details: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeErrorCode {
    RequestInvalid,
    CapabilityNotFound,
    CapabilityAmbiguous,
    CapabilityNotRunnable,
    PlacementUnsupported,
    ArtifactMissing,
    ExecutionFailed,
    OutputValidationFailed,
    ContractViolation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeExecutionOutcome {
    pub result: RuntimeResult,
    pub trace: RuntimeTrace,
    pub state_events: Vec<RuntimeStateEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserRuntimeSubscriptionRequest {
    pub kind: String,
    pub schema_version: String,
    pub governing_spec: String,
    #[serde(default)]
    pub request_id: Option<String>,
    #[serde(default)]
    pub execution_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserRuntimeSubscriptionErrorMessage {
    pub kind: String,
    pub schema_version: String,
    pub sequence: u64,
    pub code: BrowserRuntimeSubscriptionErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserRuntimeSubscriptionErrorCode {
    InvalidRequest,
    NotFound,
    UnsupportedOperation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserRuntimeSubscriptionLifecycleMessage {
    pub kind: String,
    pub schema_version: String,
    pub sequence: u64,
    pub request_id: String,
    pub execution_id: String,
    pub status: BrowserRuntimeSubscriptionLifecycleStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserRuntimeSubscriptionLifecycleStatus {
    SubscriptionEstablished,
    StreamCompleted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserRuntimeSubscriptionStateMessage {
    pub kind: String,
    pub schema_version: String,
    pub sequence: u64,
    pub state_event: RuntimeStateEvent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserRuntimeSubscriptionTraceArtifactMessage {
    pub kind: String,
    pub schema_version: String,
    pub sequence: u64,
    pub trace: RuntimeTrace,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserRuntimeSubscriptionTerminalMessage {
    pub kind: String,
    pub schema_version: String,
    pub sequence: u64,
    pub result: RuntimeResult,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BrowserRuntimeSubscriptionMessage {
    Error(BrowserRuntimeSubscriptionErrorMessage),
    Lifecycle(Box<BrowserRuntimeSubscriptionLifecycleMessage>),
    State(Box<BrowserRuntimeSubscriptionStateMessage>),
    TraceArtifact(Box<BrowserRuntimeSubscriptionTraceArtifactMessage>),
    StreamTerminal(Box<BrowserRuntimeSubscriptionTerminalMessage>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeRegistryScope {
    Public,
    Private,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeImplementationKind {
    Executable,
    Workflow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeLifecycle {
    Draft,
    Active,
    Deprecated,
    Retired,
    Archived,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestParseFailure {
    pub message: String,
}

impl fmt::Display for RequestParseFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for RequestParseFailure {}

/// Parses a runtime request from raw JSON text.
///
/// # Errors
///
/// Returns [`RequestParseFailure`] when the JSON payload cannot be
/// deserialized into the runtime request model.
pub fn parse_runtime_request(json: &str) -> Result<RuntimeRequest, RequestParseFailure> {
    serde_json::from_str::<RuntimeRequest>(json).map_err(|error| RequestParseFailure {
        message: error.to_string(),
    })
}

#[must_use]
pub fn browser_subscription_messages(
    request: &BrowserRuntimeSubscriptionRequest,
    outcome: &RuntimeExecutionOutcome,
) -> Vec<BrowserRuntimeSubscriptionMessage> {
    if let Some(error) = validate_browser_subscription_request(request) {
        return vec![BrowserRuntimeSubscriptionMessage::Error(error)];
    }

    if !subscription_targets_outcome(request, outcome) {
        return vec![BrowserRuntimeSubscriptionMessage::Error(
            BrowserRuntimeSubscriptionErrorMessage {
                kind: BROWSER_SUBSCRIPTION_ERROR_KIND.to_string(),
                schema_version: SUPPORTED_SCHEMA_VERSION.to_string(),
                sequence: 0,
                code: BrowserRuntimeSubscriptionErrorCode::NotFound,
                message: "subscription target did not match the supplied execution outcome"
                    .to_string(),
            },
        )];
    }

    let mut sequence = 0_u64;
    let mut messages = Vec::new();
    messages.push(BrowserRuntimeSubscriptionMessage::Lifecycle(Box::new(
        BrowserRuntimeSubscriptionLifecycleMessage {
            kind: BROWSER_SUBSCRIPTION_LIFECYCLE_KIND.to_string(),
            schema_version: SUPPORTED_SCHEMA_VERSION.to_string(),
            sequence,
            request_id: outcome.result.request_id.clone(),
            execution_id: outcome.result.execution_id.clone(),
            status: BrowserRuntimeSubscriptionLifecycleStatus::SubscriptionEstablished,
        },
    )));
    sequence += 1;

    for state_event in &outcome.state_events {
        messages.push(BrowserRuntimeSubscriptionMessage::State(Box::new(
            BrowserRuntimeSubscriptionStateMessage {
                kind: BROWSER_SUBSCRIPTION_STATE_KIND.to_string(),
                schema_version: SUPPORTED_SCHEMA_VERSION.to_string(),
                sequence,
                state_event: state_event.clone(),
            },
        )));
        sequence += 1;
    }

    messages.push(BrowserRuntimeSubscriptionMessage::TraceArtifact(Box::new(
        BrowserRuntimeSubscriptionTraceArtifactMessage {
            kind: BROWSER_SUBSCRIPTION_TRACE_KIND.to_string(),
            schema_version: SUPPORTED_SCHEMA_VERSION.to_string(),
            sequence,
            trace: outcome.trace.clone(),
        },
    )));
    sequence += 1;

    messages.push(BrowserRuntimeSubscriptionMessage::StreamTerminal(Box::new(
        BrowserRuntimeSubscriptionTerminalMessage {
            kind: BROWSER_SUBSCRIPTION_TERMINAL_KIND.to_string(),
            schema_version: SUPPORTED_SCHEMA_VERSION.to_string(),
            sequence,
            result: outcome.result.clone(),
        },
    )));
    sequence += 1;

    messages.push(BrowserRuntimeSubscriptionMessage::Lifecycle(Box::new(
        BrowserRuntimeSubscriptionLifecycleMessage {
            kind: BROWSER_SUBSCRIPTION_LIFECYCLE_KIND.to_string(),
            schema_version: SUPPORTED_SCHEMA_VERSION.to_string(),
            sequence,
            request_id: outcome.result.request_id.clone(),
            execution_id: outcome.result.execution_id.clone(),
            status: BrowserRuntimeSubscriptionLifecycleStatus::StreamCompleted,
        },
    )));

    messages
}

fn validate_browser_subscription_request(
    request: &BrowserRuntimeSubscriptionRequest,
) -> Option<BrowserRuntimeSubscriptionErrorMessage> {
    if request.kind != BROWSER_SUBSCRIPTION_REQUEST_KIND {
        return Some(browser_subscription_error(
            BrowserRuntimeSubscriptionErrorCode::InvalidRequest,
            "kind must equal browser_runtime_subscription_request",
        ));
    }
    if request.schema_version != SUPPORTED_SCHEMA_VERSION {
        return Some(browser_subscription_error(
            BrowserRuntimeSubscriptionErrorCode::InvalidRequest,
            "schema_version must equal 1.0.0",
        ));
    }
    if request.governing_spec != BROWSER_SUBSCRIPTION_GOVERNING_SPEC {
        return Some(browser_subscription_error(
            BrowserRuntimeSubscriptionErrorCode::InvalidRequest,
            "governing_spec must equal 013-browser-runtime-subscription",
        ));
    }

    match (&request.request_id, &request.execution_id) {
        (Some(request_id), None) if non_empty(request_id) => None,
        (None, Some(execution_id)) if non_empty(execution_id) => None,
        (Some(_), Some(_)) => Some(browser_subscription_error(
            BrowserRuntimeSubscriptionErrorCode::InvalidRequest,
            "exactly one target selector must be supplied",
        )),
        _ => Some(browser_subscription_error(
            BrowserRuntimeSubscriptionErrorCode::InvalidRequest,
            "subscription request must include request_id or execution_id",
        )),
    }
}

fn subscription_targets_outcome(
    request: &BrowserRuntimeSubscriptionRequest,
    outcome: &RuntimeExecutionOutcome,
) -> bool {
    match (&request.request_id, &request.execution_id) {
        (Some(request_id), None) => request_id == &outcome.result.request_id,
        (None, Some(execution_id)) => execution_id == &outcome.result.execution_id,
        _ => false,
    }
}

fn browser_subscription_error(
    code: BrowserRuntimeSubscriptionErrorCode,
    message: &str,
) -> BrowserRuntimeSubscriptionErrorMessage {
    BrowserRuntimeSubscriptionErrorMessage {
        kind: BROWSER_SUBSCRIPTION_ERROR_KIND.to_string(),
        schema_version: SUPPORTED_SCHEMA_VERSION.to_string(),
        sequence: 0,
        code,
        message: message.to_string(),
    }
}

impl<E> Runtime<E>
where
    E: LocalExecutor,
{
    /// Executes one runtime request against the current registry state.
    #[must_use]
    pub fn execute(&self, request: RuntimeRequest) -> RuntimeExecutionOutcome {
        let (attempt, mut emitter) = begin_attempt(request, self.observability.clone());
        emitter.push(
            RuntimeState::Discovering,
            RuntimeTransitionReasonCode::RequestStarted,
            json!({"lookup_scope": attempt.request.lookup.scope}),
        );

        if let Some(error) = validate_request(&attempt.request) {
            return invalid_request_outcome(attempt, emitter, error);
        }

        let resolution = self.resolve_candidates(&attempt.request, &mut emitter);

        if resolution.eligible.is_empty() {
            return no_eligible_outcome(attempt, emitter, resolution.collection);
        }

        if resolution.eligible.len() > 1 {
            return ambiguous_outcome(attempt, emitter, resolution);
        }

        let mut eligible = resolution.eligible;
        let selected = eligible.remove(0);
        let selection = SelectionRecord {
            status: SelectionStatus::Selected,
            selected_capability_id: Some(selected.record.id.clone()),
            selected_capability_version: Some(selected.record.version.clone()),
            failure_reason: None,
            remaining_candidates: Vec::new(),
        };

        self.execute_selected(
            attempt,
            emitter,
            resolution.collection,
            selection,
            &selected,
        )
    }

    fn collect_candidates(
        &self,
        request: &RuntimeRequest,
        _reason: CandidateReason,
    ) -> Vec<ResolvedCapability> {
        let lookup_scope = map_lookup_scope(request.lookup.scope);

        // Exact version lookup — highest priority.
        if is_exact_target(&request.intent) {
            return request
                .intent
                .capability_id
                .as_deref()
                .zip(request.intent.capability_version.as_deref())
                .and_then(|(id, version)| self.registry.find_exact(lookup_scope, id, version))
                .into_iter()
                .collect();
        }

        // Semver range lookup — when capability_id + version_range are non-empty.
        if let (Some(capability_id), Some(range_str)) = (
            request.intent.capability_id.as_deref(),
            request.intent.version_range.as_deref(),
        ) && non_empty(capability_id)
            && non_empty(range_str)
        {
            return match resolve_version_range(
                &self.registry,
                capability_id,
                range_str,
                lookup_scope,
            ) {
                Ok(resolved) => {
                    let entry_lookup = match resolved.scope {
                        RegistryScope::Public => LookupScope::PublicOnly,
                        RegistryScope::Private => LookupScope::PreferPrivate,
                    };
                    self.registry
                        .find_exact(entry_lookup, &resolved.capability_id, &resolved.version)
                        .into_iter()
                        .collect()
                }
                Err(_) => Vec::new(),
            };
        }

        // Intent/discovery lookup — fallback.
        let target = request
            .intent
            .capability_id
            .as_deref()
            .or(request.intent.intent_key.as_deref())
            .unwrap_or_default();

        self.registry
            .discover(lookup_scope, &DiscoveryQuery::default())
            .into_iter()
            .filter(|entry| entry.id == target)
            .filter_map(|entry| {
                let scope = match entry.scope {
                    traverse_registry::RegistryScope::Public => LookupScope::PublicOnly,
                    traverse_registry::RegistryScope::Private => LookupScope::PreferPrivate,
                };
                self.registry.find_exact(scope, &entry.id, &entry.version)
            })
            .collect()
    }

    fn resolve_candidates(
        &self,
        request: &RuntimeRequest,
        emitter: &mut StateEmitter,
    ) -> CandidateResolution {
        let candidate_reason = if is_exact_target(&request.intent) {
            CandidateReason::ExactMatch
        } else {
            CandidateReason::IntentMatch
        };

        let discovered = self.collect_candidates(request, candidate_reason);
        if !discovered.is_empty() {
            emitter.push(
                RuntimeState::EvaluatingConstraints,
                RuntimeTransitionReasonCode::CandidatesCollected,
                json!({"candidate_count": discovered.len()}),
            );
        }

        let mut eligible = Vec::new();
        let mut rejected = Vec::new();
        for candidate in discovered {
            match evaluate_candidate(candidate) {
                CandidateEvaluation::Eligible(capability) => eligible.push(capability),
                CandidateEvaluation::Rejected(candidate, reason) => {
                    rejected.push(RejectedRuntimeCandidate {
                        capability_id: candidate.record.id.clone(),
                        capability_version: candidate.record.version.clone(),
                        scope: map_registry_scope(candidate.record.scope),
                        reason,
                    });
                }
            }
        }

        if !eligible.is_empty() {
            emitter.push(
                RuntimeState::Selecting,
                RuntimeTransitionReasonCode::ConstraintsEvaluated,
                json!({
                    "eligible_candidates": eligible.len(),
                    "rejected_candidates": rejected.len()
                }),
            );
        }

        CandidateResolution {
            eligible: eligible.clone(),
            collection: CandidateCollectionRecord {
                lookup_scope: request.lookup.scope,
                candidates: eligible
                    .iter()
                    .map(|capability| runtime_candidate(capability, candidate_reason))
                    .collect(),
                rejected_candidates: rejected,
            },
            candidate_reason,
        }
    }

    #[allow(clippy::too_many_lines)]
    fn execute_selected(
        &self,
        attempt: AttemptContext,
        emitter: StateEmitter,
        candidate_collection: CandidateCollectionRecord,
        selection: SelectionRecord,
        selected: &ResolvedCapability,
    ) -> RuntimeExecutionOutcome {
        let context = ExecutionContext {
            attempt,
            emitter,
            candidate_collection,
            selection,
        };
        let requested_target = context.attempt.request.context.requested_target;

        if contains_drafts_segment(&selected.record.contract_path) {
            let violation = ViolationRecord::new(
                "draft_artifact_not_executable",
                selected.record.contract_path.clone(),
                "draft artifacts are quarantined under drafts/ and must not be executable",
            );
            let error = runtime_error(
                RuntimeErrorCode::ContractViolation,
                "draft artifacts are not executable",
                json!({"violations": [violation]}),
            );
            return pre_execution_failure_outcome(
                context,
                PreExecutionFailure {
                    artifact_ref: Some(selected.record.artifact_ref.clone()),
                    failure_reason: ExecutionFailureReason::ArtifactNotRunnable,
                    placement: placement_not_attempted(
                        requested_target,
                        PlacementDecisionReason::SelectionNotReached,
                    ),
                    error,
                },
            );
        }

        let placement = match resolve_placement(requested_target) {
            Ok(placement) => placement,
            Err(error) => {
                return pre_execution_failure_outcome(
                    context,
                    PreExecutionFailure {
                        artifact_ref: Some(selected.record.artifact_ref.clone()),
                        failure_reason: ExecutionFailureReason::PlacementUnsupported,
                        placement: placement_not_attempted(
                            requested_target,
                            PlacementDecisionReason::RequestedTargetUnsupported,
                        ),
                        error,
                    },
                );
            }
        };

        // Dependency resolution gate (spec 043): resolve and verify all
        // Capability-typed dependencies before executing.
        let lookup_scope = map_lookup_scope(context.attempt.request.lookup.scope);
        if let Err(dep_error) = resolve_dependencies(
            &self.registry,
            &selected.record.id,
            &selected.contract.dependencies,
            lookup_scope,
        ) {
            let (detail_id, detail_version) = match &dep_error {
                ResolutionError::MissingDependency {
                    capability_id,
                    required_version,
                } => (capability_id.clone(), required_version.clone()),
                ResolutionError::CircularDependency { cycle } => {
                    (cycle.join(" -> "), String::new())
                }
                ResolutionError::MaxTransitiveDepthExceeded { depth, chain } => {
                    (format!("depth={depth}"), chain.join(" -> "))
                }
            };
            let error = runtime_error(
                RuntimeErrorCode::CapabilityNotFound,
                "dependency resolution failed before execution",
                serde_json::json!({
                    "dependency_id": detail_id,
                    "required_version": detail_version,
                }),
            );
            return pre_execution_failure_outcome(
                context,
                PreExecutionFailure {
                    artifact_ref: Some(selected.record.artifact_ref.clone()),
                    failure_reason: ExecutionFailureReason::ArtifactMissing,
                    placement,
                    error,
                },
            );
        }

        if let Err(error) = validate_payload_against_contract(
            &context.attempt.request.input,
            &selected.contract.inputs.schema,
            RuntimeErrorCode::RequestInvalid,
            "runtime request input does not satisfy the selected capability input contract",
        ) {
            return pre_execution_failure_outcome(
                context,
                PreExecutionFailure {
                    artifact_ref: Some(selected.record.artifact_ref.clone()),
                    failure_reason: ExecutionFailureReason::ContractInputInvalid,
                    placement,
                    error,
                },
            );
        }

        self.execute_started_selection(context, selected, placement)
    }

    fn execute_started_selection(
        &self,
        mut context: ExecutionContext,
        selected: &ResolvedCapability,
        placement: PlacementDecisionRecord,
    ) -> RuntimeExecutionOutcome {
        let started_execution = start_selected_execution(&mut context.emitter, selected, placement);
        if selected.record.implementation_kind == ImplementationKind::Workflow {
            return self.execute_workflow_capability(context, selected, started_execution);
        }

        let execution_output = match self
            .executor
            .execute(selected, &context.attempt.request.input)
        {
            Ok(output) => output,
            Err(failure) => {
                let error = runtime_error(
                    RuntimeErrorCode::ExecutionFailed,
                    &failure.message,
                    json!({"code": "execution_failed"}),
                );
                return execution_failure_outcome(
                    context,
                    ExecutionFailureState {
                        artifact_ref: selected.record.artifact_ref.clone(),
                        started_at: started_execution.started_at,
                        placement: started_execution.placement,
                        failure_reason: ExecutionFailureReason::ExecutionFailed,
                    },
                    error,
                    Vec::new(),
                    None,
                );
            }
        };

        if let Err(error) = validate_payload_against_contract(
            &execution_output,
            &selected.contract.outputs.schema,
            RuntimeErrorCode::OutputValidationFailed,
            "executor output does not satisfy the selected capability output contract",
        ) {
            return execution_failure_outcome(
                context,
                ExecutionFailureState {
                    artifact_ref: selected.record.artifact_ref.clone(),
                    started_at: started_execution.started_at,
                    placement: started_execution.placement,
                    failure_reason: ExecutionFailureReason::ContractOutputInvalid,
                },
                error,
                Vec::new(),
                None,
            );
        }

        successful_execution_outcome(
            context,
            selected,
            started_execution,
            execution_output,
            Vec::new(),
            None,
        )
    }
}

fn terminal_failure(context: FailureContext) -> RuntimeExecutionOutcome {
    let result_record = TraceResultRecord {
        status: RuntimeResultStatus::Error,
        output: None,
        error: Some(context.error.clone()),
    };
    let otel_trace = otel_trace_record(
        &context.attempt,
        &context.state_transitions,
        &context.selection,
        &context.execution,
        &result_record,
    );
    let trace = RuntimeTrace {
        kind: RUNTIME_TRACE_KIND.to_string(),
        schema_version: SUPPORTED_SCHEMA_VERSION.to_string(),
        trace_id: context.attempt.trace_id.clone(),
        execution_id: context.attempt.execution_id.clone(),
        request_id: context.attempt.request.request_id.clone(),
        governing_spec: GOVERNING_SPEC.to_string(),
        request: context.attempt.request.clone(),
        decision_evidence: TraceDecisionEvidence {
            candidate_collection: context.candidate_collection.clone(),
            selection: context.selection.clone(),
        },
        state_progression: TraceStateProgression {
            state_events: context.state_events.clone(),
            transitions: context.state_transitions.clone(),
            validation: context.state_machine_validation.clone(),
        },
        terminal_outcome: TraceTerminalOutcome {
            runtime_status: RuntimeResultStatus::Error,
            execution_status: context.execution.status,
            failure_reason: context.execution.failure_reason,
            error: Some(context.error.clone()),
        },
        emitted_events: context.emitted_events,
        workflow_evidence: context.workflow_evidence,
        state_transitions: context.state_transitions,
        state_machine_validation: context.state_machine_validation,
        candidate_collection: context.candidate_collection,
        selection: context.selection,
        execution: context.execution,
        result: result_record,
        otel_trace,
    };

    let result = RuntimeResult {
        kind: RUNTIME_RESULT_KIND.to_string(),
        schema_version: SUPPORTED_SCHEMA_VERSION.to_string(),
        execution_id: context.attempt.execution_id,
        request_id: context.attempt.request.request_id,
        status: RuntimeResultStatus::Error,
        trace_ref: context.attempt.trace_id,
        output: None,
        error: Some(context.error),
    };

    RuntimeExecutionOutcome {
        result,
        trace,
        state_events: context.state_events,
    }
}

fn supported_executor_targets() -> Vec<PlacementTarget> {
    vec![PlacementTarget::Local]
}

fn placement_not_attempted(
    requested_target: PlacementTarget,
    reason: PlacementDecisionReason,
) -> PlacementDecisionRecord {
    PlacementDecisionRecord {
        requested_target,
        selected_target: None,
        status: PlacementDecisionStatus::NotAttempted,
        reason,
        supported_executor_targets: supported_executor_targets(),
    }
}

fn resolve_placement(
    requested_target: PlacementTarget,
) -> Result<PlacementDecisionRecord, RuntimeError> {
    if requested_target == PlacementTarget::Local {
        return Ok(PlacementDecisionRecord {
            requested_target,
            selected_target: Some(PlacementTarget::Local),
            status: PlacementDecisionStatus::Selected,
            reason: PlacementDecisionReason::RequestedTargetSelected,
            supported_executor_targets: supported_executor_targets(),
        });
    }

    Err(runtime_error(
        RuntimeErrorCode::PlacementUnsupported,
        "requested placement target is not supported by the available executor set",
        json!({
            "requested_target": requested_target,
            "supported_executor_targets": supported_executor_targets(),
        }),
    ))
}

fn begin_attempt(
    request: RuntimeRequest,
    observability: RuntimeObservabilityConfig,
) -> (AttemptContext, StateEmitter) {
    let request_id = request.request_id.clone();
    let execution_id = format!("{EXECUTION_PREFIX}{request_id}");
    let trace_id = format!("{TRACE_PREFIX}{execution_id}");
    let mut emitter = StateEmitter::new(&execution_id, &request_id);
    emitter.push(
        RuntimeState::LoadingRegistry,
        RuntimeTransitionReasonCode::RuntimeInitializationStarted,
        json!({"registry_status": "available"}),
    );
    emitter.push(
        RuntimeState::Ready,
        RuntimeTransitionReasonCode::RegistryLoaded,
        json!({"governing_spec": GOVERNING_SPEC}),
    );

    (
        AttemptContext {
            request,
            execution_id,
            trace_id,
            observability,
        },
        emitter,
    )
}

fn invalid_request_outcome(
    attempt: AttemptContext,
    mut emitter: StateEmitter,
    error: RuntimeError,
) -> RuntimeExecutionOutcome {
    let placement = placement_not_attempted(
        attempt.request.context.requested_target,
        PlacementDecisionReason::SelectionNotReached,
    );
    emitter.push(
        RuntimeState::EvaluatingConstraints,
        RuntimeTransitionReasonCode::CandidatesCollected,
        json!({"candidate_count": 0}),
    );
    emitter.push(
        RuntimeState::Error,
        RuntimeTransitionReasonCode::ConstraintValidationFailed,
        json!({"code": error.code, "message": error.message}),
    );
    emitter.push(
        RuntimeState::Ready,
        RuntimeTransitionReasonCode::ExecutionClosed,
        json!({"terminal_state": RuntimeState::Error}),
    );
    let finished = emitter.finish();
    terminal_failure(FailureContext {
        attempt,
        state_events: finished.events,
        state_transitions: finished.transitions,
        state_machine_validation: finished.validation,
        candidate_collection: CandidateCollectionRecord {
            lookup_scope: RuntimeLookupScope::PreferPrivate,
            candidates: Vec::new(),
            rejected_candidates: Vec::new(),
        },
        selection: SelectionRecord {
            status: SelectionStatus::InvalidRequest,
            selected_capability_id: None,
            selected_capability_version: None,
            failure_reason: Some(SelectionFailureReason::InvalidRequest),
            remaining_candidates: Vec::new(),
        },
        execution: ExecutionRecord {
            placement: placement.clone(),
            placement_target: placement.requested_target,
            status: ExecutionStatus::NotStarted,
            artifact_ref: None,
            started_at: None,
            completed_at: None,
            output_digest: None,
            failure_reason: Some(ExecutionFailureReason::ContractInputInvalid),
        },
        error,
        emitted_events: Vec::new(),
        workflow_evidence: None,
    })
}

fn no_eligible_outcome(
    attempt: AttemptContext,
    mut emitter: StateEmitter,
    candidate_collection: CandidateCollectionRecord,
) -> RuntimeExecutionOutcome {
    let placement = placement_not_attempted(
        attempt.request.context.requested_target,
        PlacementDecisionReason::SelectionNotReached,
    );
    let error = if candidate_collection.rejected_candidates.is_empty() {
        runtime_error(
            RuntimeErrorCode::CapabilityNotFound,
            "no eligible capability matched the runtime request",
            json!({"request_id": attempt.request.request_id}),
        )
    } else {
        runtime_error(
            RuntimeErrorCode::CapabilityNotRunnable,
            "matching capabilities were found but none were runnable locally",
            json!({"rejected_candidates": candidate_collection.rejected_candidates}),
        )
    };
    let reason = if candidate_collection.rejected_candidates.is_empty() {
        RuntimeTransitionReasonCode::NoMatch
    } else {
        RuntimeTransitionReasonCode::ConstraintValidationFailed
    };
    emitter.push(RuntimeState::Error, reason, json!({"code": error.code}));
    emitter.push(
        RuntimeState::Ready,
        RuntimeTransitionReasonCode::ExecutionClosed,
        json!({"terminal_state": RuntimeState::Error}),
    );
    let failure_reason = if error.code == RuntimeErrorCode::CapabilityNotFound {
        SelectionFailureReason::NoMatch
    } else {
        SelectionFailureReason::NotRunnable
    };
    let finished = emitter.finish();

    terminal_failure(FailureContext {
        attempt,
        state_events: finished.events,
        state_transitions: finished.transitions,
        state_machine_validation: finished.validation,
        candidate_collection,
        selection: SelectionRecord {
            status: SelectionStatus::NoMatch,
            selected_capability_id: None,
            selected_capability_version: None,
            failure_reason: Some(failure_reason),
            remaining_candidates: Vec::new(),
        },
        execution: ExecutionRecord {
            placement: placement.clone(),
            placement_target: placement.requested_target,
            status: ExecutionStatus::NotStarted,
            artifact_ref: None,
            started_at: None,
            completed_at: None,
            output_digest: None,
            failure_reason: Some(ExecutionFailureReason::ArtifactNotRunnable),
        },
        error,
        emitted_events: Vec::new(),
        workflow_evidence: None,
    })
}

fn ambiguous_outcome(
    attempt: AttemptContext,
    mut emitter: StateEmitter,
    resolution: CandidateResolution,
) -> RuntimeExecutionOutcome {
    let placement = placement_not_attempted(
        attempt.request.context.requested_target,
        PlacementDecisionReason::SelectionNotReached,
    );
    let remaining_candidates = resolution
        .eligible
        .iter()
        .map(|candidate| runtime_candidate(candidate, resolution.candidate_reason))
        .collect::<Vec<_>>();
    let error = runtime_error(
        RuntimeErrorCode::CapabilityAmbiguous,
        "runtime request matched more than one eligible capability",
        json!({"remaining_candidates": remaining_candidates}),
    );
    emitter.push(
        RuntimeState::Error,
        RuntimeTransitionReasonCode::SelectionFailed,
        json!({"code": error.code}),
    );
    emitter.push(
        RuntimeState::Ready,
        RuntimeTransitionReasonCode::ExecutionClosed,
        json!({"terminal_state": RuntimeState::Error}),
    );
    let finished = emitter.finish();

    terminal_failure(FailureContext {
        attempt,
        state_events: finished.events,
        state_transitions: finished.transitions,
        state_machine_validation: finished.validation,
        candidate_collection: resolution.collection,
        selection: SelectionRecord {
            status: SelectionStatus::Ambiguous,
            selected_capability_id: None,
            selected_capability_version: None,
            failure_reason: Some(SelectionFailureReason::Ambiguous),
            remaining_candidates,
        },
        execution: ExecutionRecord {
            placement: placement.clone(),
            placement_target: placement.requested_target,
            status: ExecutionStatus::NotStarted,
            artifact_ref: None,
            started_at: None,
            completed_at: None,
            output_digest: None,
            failure_reason: Some(ExecutionFailureReason::ArtifactNotRunnable),
        },
        error,
        emitted_events: Vec::new(),
        workflow_evidence: None,
    })
}

fn pre_execution_failure_outcome(
    context: ExecutionContext,
    failure: PreExecutionFailure,
) -> RuntimeExecutionOutcome {
    let ExecutionContext {
        attempt,
        mut emitter,
        candidate_collection,
        selection,
    } = context;
    let reason = if emitter.current_state == RuntimeState::Selecting {
        RuntimeTransitionReasonCode::SelectionFailed
    } else {
        RuntimeTransitionReasonCode::ConstraintValidationFailed
    };
    emitter.push(
        RuntimeState::Error,
        reason,
        json!({"code": failure.error.code, "details": failure.error.details}),
    );
    emitter.push(
        RuntimeState::Ready,
        RuntimeTransitionReasonCode::ExecutionClosed,
        json!({"terminal_state": RuntimeState::Error}),
    );
    let finished = emitter.finish();
    terminal_failure(FailureContext {
        attempt,
        state_events: finished.events,
        state_transitions: finished.transitions,
        state_machine_validation: finished.validation,
        candidate_collection,
        selection,
        execution: ExecutionRecord {
            placement: failure.placement.clone(),
            placement_target: failure
                .placement
                .selected_target
                .unwrap_or(failure.placement.requested_target),
            status: ExecutionStatus::NotStarted,
            artifact_ref: failure.artifact_ref,
            started_at: None,
            completed_at: None,
            output_digest: None,
            failure_reason: Some(failure.failure_reason),
        },
        error: failure.error,
        emitted_events: Vec::new(),
        workflow_evidence: None,
    })
}

#[allow(clippy::too_many_arguments)]
fn execution_failure_outcome(
    context: ExecutionContext,
    failure: ExecutionFailureState,
    error: RuntimeError,
    emitted_events: Vec<traverse_contracts::EventReference>,
    workflow_evidence: Option<WorkflowTraversalEvidence>,
) -> RuntimeExecutionOutcome {
    let ExecutionContext {
        attempt,
        mut emitter,
        candidate_collection,
        selection,
    } = context;
    emitter.push(
        RuntimeState::Error,
        RuntimeTransitionReasonCode::ExecutionFailed,
        json!({"code": error.code, "details": error.details}),
    );
    let completed_at = emitter.next_timestamp();
    emitter.push(
        RuntimeState::Ready,
        RuntimeTransitionReasonCode::ExecutionClosed,
        json!({"terminal_state": RuntimeState::Error}),
    );
    let finished = emitter.finish();

    terminal_failure(FailureContext {
        attempt,
        state_events: finished.events,
        state_transitions: finished.transitions,
        state_machine_validation: finished.validation,
        candidate_collection,
        selection,
        execution: ExecutionRecord {
            placement: failure.placement.clone(),
            placement_target: failure
                .placement
                .selected_target
                .unwrap_or(failure.placement.requested_target),
            status: ExecutionStatus::Failed,
            artifact_ref: Some(failure.artifact_ref),
            started_at: Some(failure.started_at),
            completed_at: Some(completed_at),
            output_digest: None,
            failure_reason: Some(failure.failure_reason),
        },
        error,
        emitted_events,
        workflow_evidence,
    })
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn successful_execution_outcome(
    context: ExecutionContext,
    selected: &ResolvedCapability,
    started_execution: StartedExecution,
    execution_output: Value,
    emitted_events: Vec<traverse_contracts::EventReference>,
    workflow_evidence: Option<WorkflowTraversalEvidence>,
) -> RuntimeExecutionOutcome {
    let ExecutionContext {
        attempt,
        mut emitter,
        candidate_collection,
        selection,
    } = context;
    let completed_at = emitter.next_timestamp();
    let emits_events = selected.record.implementation_kind == ImplementationKind::Workflow
        || !selected.contract.emits.is_empty();
    if emits_events {
        emitter.push(
            RuntimeState::EmittingEvents,
            RuntimeTransitionReasonCode::ExecutionSucceededWithEvents,
            json!({
                "capability_id": selected.record.id,
                "capability_version": selected.record.version,
                "declared_event_count": selected.contract.emits.len(),
            }),
        );
        emitter.push(
            RuntimeState::Completed,
            RuntimeTransitionReasonCode::EventsEmitted,
            json!({
                "capability_id": selected.record.id,
                "capability_version": selected.record.version,
            }),
        );
    } else {
        emitter.push(
            RuntimeState::Completed,
            RuntimeTransitionReasonCode::ExecutionSucceeded,
            json!({
                "capability_id": selected.record.id,
                "capability_version": selected.record.version,
            }),
        );
    }
    emitter.push(
        RuntimeState::Ready,
        RuntimeTransitionReasonCode::ExecutionClosed,
        json!({"terminal_state": RuntimeState::Completed}),
    );
    let finished = emitter.finish();

    let execution = ExecutionRecord {
        placement: started_execution.placement.clone(),
        placement_target: started_execution
            .placement
            .selected_target
            .unwrap_or(started_execution.placement.requested_target),
        status: ExecutionStatus::Succeeded,
        artifact_ref: Some(selected.record.artifact_ref.clone()),
        started_at: Some(started_execution.started_at),
        completed_at: Some(completed_at),
        output_digest: Some(content_digest(&execution_output)),
        failure_reason: None,
    };
    let result_record = TraceResultRecord {
        status: RuntimeResultStatus::Completed,
        output: Some(execution_output.clone()),
        error: None,
    };
    let otel_trace = otel_trace_record(
        &attempt,
        &finished.transitions,
        &selection,
        &execution,
        &result_record,
    );

    let trace = RuntimeTrace {
        kind: RUNTIME_TRACE_KIND.to_string(),
        schema_version: SUPPORTED_SCHEMA_VERSION.to_string(),
        trace_id: attempt.trace_id.clone(),
        execution_id: attempt.execution_id.clone(),
        request_id: attempt.request.request_id.clone(),
        governing_spec: GOVERNING_SPEC.to_string(),
        request: attempt.request.clone(),
        decision_evidence: TraceDecisionEvidence {
            candidate_collection: candidate_collection.clone(),
            selection: selection.clone(),
        },
        state_progression: TraceStateProgression {
            state_events: finished.events.clone(),
            transitions: finished.transitions.clone(),
            validation: finished.validation.clone(),
        },
        terminal_outcome: TraceTerminalOutcome {
            runtime_status: RuntimeResultStatus::Completed,
            execution_status: execution.status,
            failure_reason: None,
            error: None,
        },
        emitted_events,
        workflow_evidence,
        state_transitions: finished.transitions.clone(),
        state_machine_validation: finished.validation.clone(),
        candidate_collection,
        selection,
        execution,
        result: result_record,
        otel_trace,
    };

    let result = RuntimeResult {
        kind: RUNTIME_RESULT_KIND.to_string(),
        schema_version: SUPPORTED_SCHEMA_VERSION.to_string(),
        execution_id: attempt.execution_id,
        request_id: attempt.request.request_id,
        status: RuntimeResultStatus::Completed,
        trace_ref: attempt.trace_id,
        output: Some(execution_output),
        error: None,
    };

    RuntimeExecutionOutcome {
        result,
        trace,
        state_events: finished.events,
    }
}

fn validate_request(request: &RuntimeRequest) -> Option<RuntimeError> {
    if request.kind != RUNTIME_REQUEST_KIND {
        return Some(runtime_error(
            RuntimeErrorCode::RequestInvalid,
            "kind must equal runtime_request",
            json!({"path": "$.kind"}),
        ));
    }
    if request.schema_version != SUPPORTED_SCHEMA_VERSION {
        return Some(runtime_error(
            RuntimeErrorCode::RequestInvalid,
            "schema_version must equal 1.0.0",
            json!({"path": "$.schema_version"}),
        ));
    }
    if request.governing_spec != GOVERNING_SPEC {
        return Some(runtime_error(
            RuntimeErrorCode::RequestInvalid,
            "governing_spec must equal 006-runtime-request-execution",
            json!({"path": "$.governing_spec"}),
        ));
    }
    if request.request_id.trim().is_empty() {
        return Some(runtime_error(
            RuntimeErrorCode::RequestInvalid,
            "request_id must be non-empty",
            json!({"path": "$.request_id"}),
        ));
    }
    if request.lookup.allow_ambiguity {
        return Some(runtime_error(
            RuntimeErrorCode::RequestInvalid,
            "allow_ambiguity must be false in this runtime slice",
            json!({"path": "$.lookup.allow_ambiguity"}),
        ));
    }
    if request
        .intent
        .capability_version
        .as_deref()
        .is_some_and(|version| Version::parse(version).is_err())
    {
        return Some(runtime_error(
            RuntimeErrorCode::RequestInvalid,
            "capability_version must be valid semantic versioning",
            json!({"path": "$.intent.capability_version"}),
        ));
    }

    let exact_id = request
        .intent
        .capability_id
        .as_deref()
        .is_some_and(non_empty);
    let exact_version = request
        .intent
        .capability_version
        .as_deref()
        .is_some_and(non_empty);
    let intent_key = request.intent.intent_key.as_deref().is_some_and(non_empty);

    if !(exact_id || intent_key) {
        return Some(runtime_error(
            RuntimeErrorCode::RequestInvalid,
            "runtime intent must include capability_id or intent_key",
            json!({"path": "$.intent"}),
        ));
    }

    if exact_version && !exact_id {
        return Some(runtime_error(
            RuntimeErrorCode::RequestInvalid,
            "capability_version requires capability_id",
            json!({"path": "$.intent.capability_version"}),
        ));
    }

    let has_version_range = request
        .intent
        .version_range
        .as_deref()
        .is_some_and(non_empty);

    if has_version_range && !exact_id {
        return Some(runtime_error(
            RuntimeErrorCode::RequestInvalid,
            "version_range requires capability_id",
            json!({"path": "$.intent.version_range"}),
        ));
    }

    if has_version_range && exact_version {
        return Some(runtime_error(
            RuntimeErrorCode::RequestInvalid,
            "version_range and capability_version are mutually exclusive",
            json!({"path": "$.intent.version_range"}),
        ));
    }

    None
}

fn is_exact_target(intent: &RuntimeIntent) -> bool {
    intent.capability_id.as_deref().is_some_and(non_empty)
        && intent.capability_version.as_deref().is_some_and(non_empty)
}

fn non_empty(value: &str) -> bool {
    !value.trim().is_empty()
}

fn map_lookup_scope(scope: RuntimeLookupScope) -> LookupScope {
    match scope {
        RuntimeLookupScope::PublicOnly => LookupScope::PublicOnly,
        RuntimeLookupScope::PreferPrivate => LookupScope::PreferPrivate,
    }
}

fn evaluate_candidate(candidate: ResolvedCapability) -> CandidateEvaluation {
    if !candidate.contract.lifecycle.is_runtime_eligible() {
        return CandidateEvaluation::Rejected(
            candidate,
            RejectedCandidateReason::LifecycleNotRunnable,
        );
    }
    if candidate.record.implementation_kind == ImplementationKind::Workflow {
        if candidate.artifact.workflow_ref.is_some() {
            return CandidateEvaluation::Eligible(candidate);
        }
        return CandidateEvaluation::Rejected(candidate, RejectedCandidateReason::ArtifactMissing);
    }

    let Some(binary) = candidate.artifact.binary.as_ref() else {
        return CandidateEvaluation::Rejected(candidate, RejectedCandidateReason::ArtifactMissing);
    };

    if binary.location.trim().is_empty() {
        return CandidateEvaluation::Rejected(candidate, RejectedCandidateReason::ArtifactMissing);
    }

    let execution = &candidate.contract.execution;
    if !execution
        .preferred_targets
        .contains(&ExecutionTarget::Local)
        || execution.constraints.host_api_access != HostApiAccess::None
        || execution.constraints.network_access != NetworkAccess::Forbidden
    {
        return CandidateEvaluation::Rejected(
            candidate,
            RejectedCandidateReason::NotRunnableLocally,
        );
    }

    CandidateEvaluation::Eligible(candidate)
}

fn validate_payload_against_contract(
    payload: &Value,
    schema: &Value,
    code: RuntimeErrorCode,
    message: &str,
) -> Result<(), RuntimeError> {
    let mut errors = Vec::new();
    validate_value_against_schema(payload, schema, "$", &mut errors);
    if errors.is_empty() {
        Ok(())
    } else {
        Err(runtime_error(
            code,
            message,
            json!({ "violations": errors }),
        ))
    }
}

pub(crate) fn validate_value_against_schema(
    value: &Value,
    schema: &Value,
    path: &str,
    errors: &mut Vec<Value>,
) {
    let Some(schema_object) = schema.as_object() else {
        errors.push(json!({
            "path": path,
            "message": "schema must be an object"
        }));
        return;
    };

    if let Some(schema_type) = schema_object.get("type").and_then(Value::as_str) {
        match schema_type {
            "object" => {
                let Some(instance) = value.as_object() else {
                    errors.push(type_error(path, "object"));
                    return;
                };
                validate_required(instance, schema_object, path, errors);
                validate_properties(instance, schema_object, path, errors);
            }
            "array" => {
                let Some(items) = value.as_array() else {
                    errors.push(type_error(path, "array"));
                    return;
                };
                if let Some(item_schema) = schema_object.get("items") {
                    for (index, item) in items.iter().enumerate() {
                        validate_value_against_schema(
                            item,
                            item_schema,
                            &format!("{path}[{index}]"),
                            errors,
                        );
                    }
                }
            }
            "string" if !value.is_string() => errors.push(type_error(path, "string")),
            "integer" if value.as_i64().is_none() && value.as_u64().is_none() => {
                errors.push(type_error(path, "integer"));
            }
            "number" if !value.is_number() => errors.push(type_error(path, "number")),
            "boolean" if !value.is_boolean() => errors.push(type_error(path, "boolean")),
            "null" if !value.is_null() => errors.push(type_error(path, "null")),
            _ => {}
        }
    }
}

fn validate_required(
    instance: &Map<String, Value>,
    schema_object: &Map<String, Value>,
    path: &str,
    errors: &mut Vec<Value>,
) {
    let Some(required) = schema_object.get("required").and_then(Value::as_array) else {
        return;
    };

    for required_field in required.iter().filter_map(Value::as_str) {
        if !instance.contains_key(required_field) {
            errors.push(json!({
                "path": format!("{path}.{required_field}"),
                "message": "required property is missing"
            }));
        }
    }
}

fn validate_properties(
    instance: &Map<String, Value>,
    schema_object: &Map<String, Value>,
    path: &str,
    errors: &mut Vec<Value>,
) {
    let Some(properties) = schema_object.get("properties").and_then(Value::as_object) else {
        return;
    };

    for (key, value) in instance {
        if let Some(property_schema) = properties.get(key) {
            validate_value_against_schema(value, property_schema, &format!("{path}.{key}"), errors);
        }
    }
}

fn type_error(path: &str, expected: &str) -> Value {
    json!({
        "path": path,
        "message": format!("expected {expected}")
    })
}

fn runtime_candidate(capability: &ResolvedCapability, reason: CandidateReason) -> RuntimeCandidate {
    RuntimeCandidate {
        scope: map_registry_scope(capability.record.scope),
        capability_id: capability.record.id.clone(),
        capability_version: capability.record.version.clone(),
        artifact_ref: capability.record.artifact_ref.clone(),
        implementation_kind: map_implementation_kind(capability.record.implementation_kind),
        lifecycle: map_lifecycle(&capability.record.lifecycle),
        reason,
    }
}

fn map_registry_scope(scope: RegistryScope) -> RuntimeRegistryScope {
    match scope {
        RegistryScope::Public => RuntimeRegistryScope::Public,
        RegistryScope::Private => RuntimeRegistryScope::Private,
    }
}

fn map_implementation_kind(kind: ImplementationKind) -> RuntimeImplementationKind {
    match kind {
        ImplementationKind::Executable => RuntimeImplementationKind::Executable,
        ImplementationKind::Workflow => RuntimeImplementationKind::Workflow,
    }
}

fn map_lifecycle(lifecycle: &Lifecycle) -> RuntimeLifecycle {
    match lifecycle {
        Lifecycle::Draft => RuntimeLifecycle::Draft,
        Lifecycle::Active => RuntimeLifecycle::Active,
        Lifecycle::Deprecated => RuntimeLifecycle::Deprecated,
        Lifecycle::Retired => RuntimeLifecycle::Retired,
        Lifecycle::Archived => RuntimeLifecycle::Archived,
    }
}

fn runtime_error(code: RuntimeErrorCode, message: &str, details: Value) -> RuntimeError {
    RuntimeError {
        code,
        message: message.to_string(),
        details,
    }
}

fn content_digest(value: &Value) -> String {
    let json = value.to_string();
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in json.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0001_0000_01b3);
    }
    format!("0.1.0:{hash:016x}")
}

fn otel_trace_record(
    attempt: &AttemptContext,
    state_transitions: &[RuntimeTransitionRecord],
    selection: &SelectionRecord,
    execution: &ExecutionRecord,
    result: &TraceResultRecord,
) -> OTelTraceRecord {
    let trace_id = otel_trace_id(attempt);
    let root_span_id = otel_span_id(attempt, "runtime.request", 0);
    let mut spans = vec![otel_span(OTelSpanInput {
        trace_id: &trace_id,
        span_id: &root_span_id,
        parent_span_id: None,
        name: "traverse.runtime.request",
        status: span_status(result.status),
        started_at: first_transition_time(state_transitions),
        ended_at: last_transition_time(state_transitions),
        attributes: base_otel_attributes(attempt, selection, execution),
        events: error_events(result),
    })];

    for (index, phase) in otel_phase_names().iter().enumerate() {
        spans.push(otel_span(OTelSpanInput {
            trace_id: &trace_id,
            span_id: &otel_span_id(attempt, phase, index + 1),
            parent_span_id: Some(root_span_id.clone()),
            name: phase,
            status: phase_status(phase, result.status),
            started_at: phase_started_at(state_transitions, index),
            ended_at: phase_ended_at(state_transitions, index),
            attributes: base_otel_attributes(attempt, selection, execution),
            events: if result.status == RuntimeResultStatus::Error
                && *phase == "traverse.trace.assembly"
            {
                error_events(result)
            } else {
                Vec::new()
            },
        }));
    }

    OTelTraceRecord {
        trace_id,
        parent_traceparent: attempt.request.context.traceparent.clone(),
        tracestate: attempt.request.context.tracestate.clone(),
        exporter: OTelExporterRecord {
            enabled: attempt.observability.exporter.endpoint.is_some(),
            endpoint: attempt.observability.exporter.endpoint.clone(),
            protocol: attempt.observability.exporter.protocol,
        },
        spans,
    }
}

fn otel_phase_names() -> [&'static str; 5] {
    [
        "traverse.request.intake",
        "traverse.registry.lookup",
        "traverse.contract.validation",
        "traverse.capability.execution",
        "traverse.trace.assembly",
    ]
}

fn otel_trace_id(attempt: &AttemptContext) -> String {
    if attempt.observability.deterministic_ids {
        let seed = attempt
            .observability
            .deterministic_seed
            .as_deref()
            .unwrap_or("traverse-test");
        return deterministic_hex(seed, &attempt.trace_id, 32);
    }
    deterministic_hex("traverse-runtime", &attempt.trace_id, 32)
}

fn otel_span_id(attempt: &AttemptContext, name: &str, index: usize) -> String {
    let seed = attempt
        .observability
        .deterministic_seed
        .as_deref()
        .unwrap_or("traverse-runtime");
    deterministic_hex(
        seed,
        &format!("{}:{name}:{index}", attempt.execution_id),
        16,
    )
}

fn deterministic_hex(seed: &str, value: &str, len: usize) -> String {
    let mut hash: u128 = 0x6c62_272e_07bb_0142_62b8_2175_6295_c58d;
    for byte in seed.as_bytes().iter().chain(value.as_bytes()) {
        hash ^= u128::from(*byte);
        hash = hash.wrapping_mul(0x0000_0000_0100_0000_0000_0000_0000_013b);
    }
    format!("{hash:032x}").chars().take(len).collect()
}

struct OTelSpanInput<'a> {
    trace_id: &'a str,
    span_id: &'a str,
    parent_span_id: Option<String>,
    name: &'a str,
    status: OTelSpanStatus,
    started_at: String,
    ended_at: String,
    attributes: Vec<OTelAttribute>,
    events: Vec<OTelSpanEvent>,
}

fn otel_span(input: OTelSpanInput<'_>) -> OTelSpanRecord {
    OTelSpanRecord {
        trace_id: input.trace_id.to_string(),
        span_id: input.span_id.to_string(),
        parent_span_id: input.parent_span_id,
        name: input.name.to_string(),
        kind: OTelSpanKind::Internal,
        status: input.status,
        started_at: input.started_at,
        ended_at: input.ended_at,
        attributes: input.attributes,
        events: input.events,
    }
}

fn base_otel_attributes(
    attempt: &AttemptContext,
    selection: &SelectionRecord,
    execution: &ExecutionRecord,
) -> Vec<OTelAttribute> {
    let mut attributes = vec![
        otel_attr("traverse.request.id", json!(attempt.request.request_id)),
        otel_attr("traverse.execution.id", json!(attempt.execution_id)),
        otel_attr("traverse.lookup.scope", json!(attempt.request.lookup.scope)),
        otel_attr(
            "traverse.runtime.placement.target",
            json!(execution.placement_target),
        ),
    ];
    if let Some(correlation_id) = &attempt.request.context.correlation_id {
        attributes.push(otel_attr("traverse.correlation.id", json!(correlation_id)));
    }
    if let Some(capability_id) = &selection.selected_capability_id {
        attributes.push(otel_attr("traverse.capability.id", json!(capability_id)));
    }
    if let Some(capability_version) = &selection.selected_capability_version {
        attributes.push(otel_attr(
            "traverse.capability.version",
            json!(capability_version),
        ));
    }
    attributes
}

fn otel_attr(key: &str, value: Value) -> OTelAttribute {
    OTelAttribute {
        key: key.to_string(),
        value,
    }
}

fn error_events(result: &TraceResultRecord) -> Vec<OTelSpanEvent> {
    result
        .error
        .as_ref()
        .map(|error| {
            vec![OTelSpanEvent {
                name: "exception".to_string(),
                timestamp: "1970-01-01T00:00:00Z".to_string(),
                attributes: vec![
                    otel_attr("traverse.error.classification", json!(error.code)),
                    otel_attr("traverse.error.message", json!(error.message)),
                ],
            }]
        })
        .unwrap_or_default()
}

fn span_status(status: RuntimeResultStatus) -> OTelSpanStatus {
    match status {
        RuntimeResultStatus::Completed => OTelSpanStatus::Ok,
        RuntimeResultStatus::Error => OTelSpanStatus::Error,
    }
}

fn phase_status(phase: &str, status: RuntimeResultStatus) -> OTelSpanStatus {
    if status == RuntimeResultStatus::Error && phase == "traverse.trace.assembly" {
        OTelSpanStatus::Error
    } else {
        OTelSpanStatus::Ok
    }
}

fn first_transition_time(transitions: &[RuntimeTransitionRecord]) -> String {
    transitions.first().map_or_else(
        || "1970-01-01T00:00:00Z".to_string(),
        |transition| transition.occurred_at.clone(),
    )
}

fn last_transition_time(transitions: &[RuntimeTransitionRecord]) -> String {
    transitions.last().map_or_else(
        || "1970-01-01T00:00:00Z".to_string(),
        |transition| transition.occurred_at.clone(),
    )
}

fn phase_started_at(transitions: &[RuntimeTransitionRecord], index: usize) -> String {
    transitions.get(index).map_or_else(
        || first_transition_time(transitions),
        |transition| transition.occurred_at.clone(),
    )
}

fn phase_ended_at(transitions: &[RuntimeTransitionRecord], index: usize) -> String {
    transitions.get(index + 1).map_or_else(
        || last_transition_time(transitions),
        |transition| transition.occurred_at.clone(),
    )
}

fn contains_drafts_segment(path: &str) -> bool {
    path.replace('\\', "/")
        .split('/')
        .any(|segment| segment == "drafts")
}

struct AttemptContext {
    request: RuntimeRequest,
    execution_id: String,
    trace_id: String,
    observability: RuntimeObservabilityConfig,
}

struct CandidateResolution {
    eligible: Vec<ResolvedCapability>,
    collection: CandidateCollectionRecord,
    candidate_reason: CandidateReason,
}

struct FailureContext {
    attempt: AttemptContext,
    state_events: Vec<RuntimeStateEvent>,
    state_transitions: Vec<RuntimeTransitionRecord>,
    state_machine_validation: RuntimeStateMachineValidationEvidence,
    candidate_collection: CandidateCollectionRecord,
    selection: SelectionRecord,
    execution: ExecutionRecord,
    error: RuntimeError,
    emitted_events: Vec<traverse_contracts::EventReference>,
    workflow_evidence: Option<WorkflowTraversalEvidence>,
}

struct ExecutionFailureState {
    artifact_ref: String,
    started_at: String,
    placement: PlacementDecisionRecord,
    failure_reason: ExecutionFailureReason,
}

struct ExecutionContext {
    attempt: AttemptContext,
    emitter: StateEmitter,
    candidate_collection: CandidateCollectionRecord,
    selection: SelectionRecord,
}

struct StartedExecution {
    started_at: String,
    placement: PlacementDecisionRecord,
}

struct PreExecutionFailure {
    artifact_ref: Option<String>,
    failure_reason: ExecutionFailureReason,
    placement: PlacementDecisionRecord,
    error: RuntimeError,
}

enum CandidateEvaluation {
    Eligible(ResolvedCapability),
    Rejected(ResolvedCapability, RejectedCandidateReason),
}

struct StateEmitter {
    execution_id: String,
    request_id: String,
    next_second: u32,
    next_event_index: u32,
    current_state: RuntimeState,
    events: Vec<RuntimeStateEvent>,
    transitions: Vec<RuntimeTransitionRecord>,
    violations: Vec<Value>,
}

struct FinishedStateMachineArtifacts {
    events: Vec<RuntimeStateEvent>,
    transitions: Vec<RuntimeTransitionRecord>,
    validation: RuntimeStateMachineValidationEvidence,
}

fn start_selected_execution(
    emitter: &mut StateEmitter,
    selected: &ResolvedCapability,
    placement: PlacementDecisionRecord,
) -> StartedExecution {
    let started_at = emitter.next_timestamp();
    emitter.push(
        RuntimeState::Executing,
        RuntimeTransitionReasonCode::CandidateSelected,
        json!({
            "capability_id": selected.record.id,
            "capability_version": selected.record.version,
            "artifact_ref": selected.record.artifact_ref,
            "requested_target": placement.requested_target,
            "selected_target": placement.selected_target,
            "placement_status": placement.status,
            "placement_reason": placement.reason,
        }),
    );
    StartedExecution {
        started_at,
        placement,
    }
}

impl StateEmitter {
    fn new(execution_id: &str, request_id: &str) -> Self {
        Self {
            execution_id: execution_id.to_string(),
            request_id: request_id.to_string(),
            next_second: 0,
            next_event_index: 0,
            current_state: RuntimeState::Idle,
            events: Vec::new(),
            transitions: Vec::new(),
            violations: Vec::new(),
        }
    }

    fn push(&mut self, state: RuntimeState, reason: RuntimeTransitionReasonCode, details: Value) {
        let transitioned = self.try_push(state, reason, details);
        debug_assert!(transitioned, "runtime state transition must be spec-valid");
    }

    fn try_push(
        &mut self,
        state: RuntimeState,
        reason: RuntimeTransitionReasonCode,
        details: Value,
    ) -> bool {
        let from_state = self.current_state;
        if !is_allowed_transition(from_state, state, reason) {
            self.violations.push(json!({
                "from_state": from_state,
                "to_state": state,
                "reason_code": reason,
                "message": "unexpected runtime state transition"
            }));
            return false;
        }
        let entered_at = self.next_timestamp();
        let mut event_details = detail_object(details);
        event_details.insert(
            "transition_reason".to_string(),
            serde_json::to_value(reason)
                .unwrap_or_else(|_| Value::String("serialization_failed".to_string())),
        );
        let event = RuntimeStateEvent {
            kind: RUNTIME_STATE_EVENT_KIND.to_string(),
            schema_version: SUPPORTED_SCHEMA_VERSION.to_string(),
            event_id: format!("rse_{}_{:04}", self.execution_id, self.next_event_index),
            execution_id: self.execution_id.clone(),
            request_id: self.request_id.clone(),
            state,
            entered_at: entered_at.clone(),
            details: Value::Object(event_details.clone()),
        };
        self.next_event_index += 1;
        self.events.push(event);
        self.transitions.push(RuntimeTransitionRecord {
            from_state,
            to_state: state,
            reason_code: reason,
            occurred_at: entered_at,
            request_id: Some(self.request_id.clone()),
            execution_id: Some(self.execution_id.clone()),
            details: Some(Value::Object(event_details)),
        });
        self.current_state = state;
        true
    }

    fn next_timestamp(&mut self) -> String {
        let timestamp = format!("1970-01-01T00:00:{:02}Z", self.next_second);
        self.next_second += 1;
        timestamp
    }

    fn finish(self) -> FinishedStateMachineArtifacts {
        let checked_states = vec![
            RuntimeState::Idle,
            RuntimeState::LoadingRegistry,
            RuntimeState::Ready,
            RuntimeState::Discovering,
            RuntimeState::EvaluatingConstraints,
            RuntimeState::Selecting,
            RuntimeState::Executing,
            RuntimeState::EmittingEvents,
            RuntimeState::Completed,
            RuntimeState::Error,
        ];
        let checked_transitions = self
            .transitions
            .iter()
            .map(|transition| {
                format!(
                    "{}->{}",
                    runtime_state_name(transition.from_state),
                    runtime_state_name(transition.to_state)
                )
            })
            .collect();
        let validation = RuntimeStateMachineValidationEvidence {
            kind: RUNTIME_STATE_MACHINE_VALIDATION_KIND.to_string(),
            schema_version: SUPPORTED_SCHEMA_VERSION.to_string(),
            governing_spec: STATE_MACHINE_GOVERNING_SPEC.to_string(),
            validated_at: format!(
                "1970-01-01T00:00:{:02}Z",
                self.next_second.saturating_sub(1)
            ),
            status: if self.violations.is_empty() {
                RuntimeStateMachineValidationStatus::Passed
            } else {
                RuntimeStateMachineValidationStatus::Failed
            },
            checked_states,
            checked_transitions,
            violations: self.violations,
        };
        FinishedStateMachineArtifacts {
            events: self.events,
            transitions: self.transitions,
            validation,
        }
    }
}

fn is_allowed_transition(
    from: RuntimeState,
    to: RuntimeState,
    reason: RuntimeTransitionReasonCode,
) -> bool {
    matches!(
        (from, to, reason),
        (
            RuntimeState::Idle,
            RuntimeState::LoadingRegistry,
            RuntimeTransitionReasonCode::RuntimeInitializationStarted
        ) | (
            RuntimeState::LoadingRegistry,
            RuntimeState::Ready,
            RuntimeTransitionReasonCode::RegistryLoaded
        ) | (
            RuntimeState::LoadingRegistry,
            RuntimeState::Error,
            RuntimeTransitionReasonCode::RegistryLoadFailed
        ) | (
            RuntimeState::Ready,
            RuntimeState::Discovering,
            RuntimeTransitionReasonCode::RequestStarted
        ) | (
            RuntimeState::Discovering,
            RuntimeState::EvaluatingConstraints,
            RuntimeTransitionReasonCode::CandidatesCollected
        ) | (
            RuntimeState::Discovering,
            RuntimeState::Error,
            RuntimeTransitionReasonCode::NoMatch
        ) | (
            RuntimeState::EvaluatingConstraints,
            RuntimeState::Selecting,
            RuntimeTransitionReasonCode::ConstraintsEvaluated
        ) | (
            RuntimeState::EvaluatingConstraints,
            RuntimeState::Error,
            RuntimeTransitionReasonCode::ConstraintValidationFailed
        ) | (
            RuntimeState::Selecting,
            RuntimeState::Executing,
            RuntimeTransitionReasonCode::CandidateSelected
        ) | (
            RuntimeState::Selecting,
            RuntimeState::Error,
            RuntimeTransitionReasonCode::SelectionFailed
        ) | (
            RuntimeState::Executing,
            RuntimeState::EmittingEvents,
            RuntimeTransitionReasonCode::ExecutionSucceededWithEvents
        ) | (
            RuntimeState::Executing,
            RuntimeState::Completed,
            RuntimeTransitionReasonCode::ExecutionSucceeded
        ) | (
            RuntimeState::Executing,
            RuntimeState::Error,
            RuntimeTransitionReasonCode::ExecutionFailed
        ) | (
            RuntimeState::EmittingEvents,
            RuntimeState::Completed,
            RuntimeTransitionReasonCode::EventsEmitted
        ) | (
            RuntimeState::EmittingEvents,
            RuntimeState::Error,
            RuntimeTransitionReasonCode::EventEmissionFailed
        ) | (
            RuntimeState::Completed | RuntimeState::Error,
            RuntimeState::Ready,
            RuntimeTransitionReasonCode::ExecutionClosed
        )
    )
}

fn detail_object(details: Value) -> Map<String, Value> {
    match details {
        Value::Object(map) => map,
        other => {
            let mut map = Map::new();
            map.insert("value".to_string(), other);
            map
        }
    }
}

fn runtime_state_name(state: RuntimeState) -> &'static str {
    match state {
        RuntimeState::Idle => "idle",
        RuntimeState::LoadingRegistry => "loading_registry",
        RuntimeState::Ready => "ready",
        RuntimeState::Discovering => "discovering",
        RuntimeState::EvaluatingConstraints => "evaluating_constraints",
        RuntimeState::Selecting => "selecting",
        RuntimeState::Executing => "executing",
        RuntimeState::EmittingEvents => "emitting_events",
        RuntimeState::Completed => "completed",
        RuntimeState::Error => "error",
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BrowserRuntimeSubscriptionErrorCode, BrowserRuntimeSubscriptionMessage,
        BrowserRuntimeSubscriptionRequest, CandidateEvaluation, CandidateReason, LocalExecutor,
        PlacementTarget, RejectedCandidateReason, Runtime, RuntimeContext, RuntimeIntent,
        RuntimeLookup, RuntimeLookupScope, RuntimeLookupScope::*, RuntimeRequest,
        RuntimeResultStatus, RuntimeState, RuntimeTransitionReasonCode,
        browser_subscription_messages, evaluate_candidate, map_implementation_kind, map_lifecycle,
        map_registry_scope, parse_runtime_request, runtime_candidate, subscription_targets_outcome,
        validate_browser_subscription_request, validate_payload_against_contract, validate_request,
    };
    use serde_json::json;
    use traverse_contracts::{
        BinaryFormat as ContractBinaryFormat, Entrypoint, EntrypointKind, Execution,
        ExecutionConstraints, ExecutionTarget, FilesystemAccess, HostApiAccess, Lifecycle,
        NetworkAccess, Owner, Provenance, ProvenanceSource, SchemaContainer, ServiceType,
    };
    use traverse_registry::{
        ArtifactDigests, BinaryFormat, BinaryReference, CapabilityArtifactRecord,
        CapabilityRegistration, CapabilityRegistry, CapabilityRegistryRecord,
        ComposabilityMetadata, CompositionKind, CompositionPattern, DiscoveryIndexEntry,
        ImplementationKind, RegistryProvenance, RegistryScope, ResolvedCapability, SourceKind,
        SourceReference,
    };

    #[test]
    fn missing_binary_metadata_is_rejected_as_artifact_missing() {
        let capability = resolved_capability(None, Lifecycle::Active);

        let evaluation = evaluate_candidate(capability);

        assert!(matches!(
            evaluation,
            CandidateEvaluation::Rejected(_, RejectedCandidateReason::ArtifactMissing)
        ));
    }

    #[test]
    fn invalid_json_request_reports_parse_error_text() {
        let error = parse_runtime_request("{invalid").err();

        assert!(error.is_some());
        let message = error.map(|item| item.to_string()).unwrap_or_default();
        assert!(!message.is_empty());
    }

    #[test]
    fn request_validation_rejects_all_invalid_request_guards() {
        let mut request = valid_request();
        request.kind = "wrong".to_string();
        assert_eq!(
            validate_request(&request).map(|error| error.code),
            Some(super::RuntimeErrorCode::RequestInvalid)
        );

        let mut request = valid_request();
        request.schema_version = "9.9.9".to_string();
        assert_eq!(
            validate_request(&request).map(|error| error.code),
            Some(super::RuntimeErrorCode::RequestInvalid)
        );

        let mut request = valid_request();
        request.governing_spec = "wrong-spec".to_string();
        assert_eq!(
            validate_request(&request).map(|error| error.code),
            Some(super::RuntimeErrorCode::RequestInvalid)
        );

        let mut request = valid_request();
        request.request_id.clear();
        assert_eq!(
            validate_request(&request).map(|error| error.code),
            Some(super::RuntimeErrorCode::RequestInvalid)
        );

        let mut request = valid_request();
        request.lookup.allow_ambiguity = true;
        assert_eq!(
            validate_request(&request).map(|error| error.code),
            Some(super::RuntimeErrorCode::RequestInvalid)
        );

        let mut request = valid_request();
        request.context.requested_target = PlacementTarget::Local;
        request.intent.capability_version = Some("bad".to_string());
        assert_eq!(
            validate_request(&request).map(|error| error.code),
            Some(super::RuntimeErrorCode::RequestInvalid)
        );

        let mut request = valid_request();
        request.intent.capability_id = None;
        request.intent.intent_key = None;
        request.intent.capability_version = None;
        assert_eq!(
            validate_request(&request).map(|error| error.code),
            Some(super::RuntimeErrorCode::RequestInvalid)
        );

        let mut request = valid_request();
        request.intent.capability_id = None;
        request.intent.capability_version = Some("1.0.0".to_string());
        assert_eq!(
            validate_request(&request).map(|error| error.code),
            Some(super::RuntimeErrorCode::RequestInvalid)
        );
    }

    #[test]
    fn candidate_evaluation_covers_local_runnability_branches() {
        let mut capability = resolved_capability(
            Some(traverse_registry::BinaryReference {
                format: traverse_registry::BinaryFormat::Wasm,
                location: "artifact.wasm".to_string(),
            }),
            Lifecycle::Active,
        );
        capability.record.implementation_kind = ImplementationKind::Workflow;
        assert!(matches!(
            evaluate_candidate(capability.clone()),
            CandidateEvaluation::Rejected(_, RejectedCandidateReason::ArtifactMissing)
        ));
        capability.artifact.workflow_ref = Some(traverse_registry::WorkflowReference {
            workflow_id: "workflow".to_string(),
            workflow_version: "1.0.0".to_string(),
        });
        assert!(matches!(
            evaluate_candidate(capability),
            CandidateEvaluation::Eligible(_)
        ));

        let capability = resolved_capability(
            Some(traverse_registry::BinaryReference {
                format: traverse_registry::BinaryFormat::Wasm,
                location: String::new(),
            }),
            Lifecycle::Active,
        );
        assert!(matches!(
            evaluate_candidate(capability),
            CandidateEvaluation::Rejected(_, RejectedCandidateReason::ArtifactMissing)
        ));

        let mut capability = resolved_capability(
            Some(traverse_registry::BinaryReference {
                format: traverse_registry::BinaryFormat::Wasm,
                location: "artifact.wasm".to_string(),
            }),
            Lifecycle::Active,
        );
        capability.contract.execution.preferred_targets = vec![ExecutionTarget::Cloud];
        assert!(matches!(
            evaluate_candidate(capability),
            CandidateEvaluation::Rejected(_, RejectedCandidateReason::NotRunnableLocally)
        ));

        let mut capability = resolved_capability(
            Some(traverse_registry::BinaryReference {
                format: traverse_registry::BinaryFormat::Wasm,
                location: "artifact.wasm".to_string(),
            }),
            Lifecycle::Active,
        );
        capability.contract.execution.constraints.host_api_access =
            HostApiAccess::ExceptionRequired;
        assert!(matches!(
            evaluate_candidate(capability),
            CandidateEvaluation::Rejected(_, RejectedCandidateReason::NotRunnableLocally)
        ));

        let mut capability = resolved_capability(
            Some(traverse_registry::BinaryReference {
                format: traverse_registry::BinaryFormat::Wasm,
                location: "artifact.wasm".to_string(),
            }),
            Lifecycle::Active,
        );
        capability.contract.execution.constraints.network_access = NetworkAccess::Required;
        assert!(matches!(
            evaluate_candidate(capability),
            CandidateEvaluation::Rejected(_, RejectedCandidateReason::NotRunnableLocally)
        ));

        let capability = resolved_capability(
            Some(traverse_registry::BinaryReference {
                format: traverse_registry::BinaryFormat::Wasm,
                location: "artifact.wasm".to_string(),
            }),
            Lifecycle::Active,
        );
        assert!(matches!(
            evaluate_candidate(capability),
            CandidateEvaluation::Eligible(_)
        ));
    }

    #[test]
    fn payload_validation_covers_schema_branches() {
        let invalid_schema = validate_payload_against_contract(
            &json!({"field": "value"}),
            &json!("bad-schema"),
            super::RuntimeErrorCode::RequestInvalid,
            "invalid schema",
        );
        assert!(invalid_schema.is_err());

        let wrong_object = validate_payload_against_contract(
            &json!("value"),
            &json!({"type": "object"}),
            super::RuntimeErrorCode::RequestInvalid,
            "wrong object",
        );
        assert!(wrong_object.is_err());

        let wrong_array = validate_payload_against_contract(
            &json!("value"),
            &json!({"type": "array"}),
            super::RuntimeErrorCode::RequestInvalid,
            "wrong array",
        );
        assert!(wrong_array.is_err());

        let typed_array = validate_payload_against_contract(
            &json!(["value", 2]),
            &json!({"type": "array", "items": {"type": "string"}}),
            super::RuntimeErrorCode::RequestInvalid,
            "typed array",
        );
        assert!(typed_array.is_err());

        for (value, schema) in [
            (json!("value"), json!({"type": "integer"})),
            (json!("value"), json!({"type": "number"})),
            (json!("value"), json!({"type": "boolean"})),
            (json!("value"), json!({"type": "null"})),
        ] {
            let result = validate_payload_against_contract(
                &value,
                &schema,
                super::RuntimeErrorCode::RequestInvalid,
                "typed validation",
            );
            assert!(result.is_err());
        }

        let missing_required = validate_payload_against_contract(
            &json!({}),
            &json!({"type": "object", "required": ["draft_id"]}),
            super::RuntimeErrorCode::RequestInvalid,
            "required field",
        );
        assert!(missing_required.is_err());

        let property_mismatch = validate_payload_against_contract(
            &json!({"draft_id": 3}),
            &json!({"type": "object", "properties": {"draft_id": {"type": "string"}}}),
            super::RuntimeErrorCode::RequestInvalid,
            "property mismatch",
        );
        assert!(property_mismatch.is_err());

        let array_without_item_schema = validate_payload_against_contract(
            &json!(["draft-1"]),
            &json!({"type": "array"}),
            super::RuntimeErrorCode::RequestInvalid,
            "array without item schema",
        );
        assert!(array_without_item_schema.is_ok());

        let object_without_type = validate_payload_against_contract(
            &json!({"draft_id": "draft-1"}),
            &json!({}),
            super::RuntimeErrorCode::RequestInvalid,
            "object without type",
        );
        assert!(object_without_type.is_ok());
    }

    #[test]
    fn runtime_mapping_helpers_cover_all_variants() {
        assert_eq!(
            map_registry_scope(RegistryScope::Public),
            super::RuntimeRegistryScope::Public
        );
        assert_eq!(
            map_registry_scope(RegistryScope::Private),
            super::RuntimeRegistryScope::Private
        );
        assert_eq!(
            map_implementation_kind(ImplementationKind::Executable),
            super::RuntimeImplementationKind::Executable
        );
        assert_eq!(
            map_implementation_kind(ImplementationKind::Workflow),
            super::RuntimeImplementationKind::Workflow
        );
        assert_eq!(
            map_lifecycle(&Lifecycle::Draft),
            super::RuntimeLifecycle::Draft
        );
        assert_eq!(
            map_lifecycle(&Lifecycle::Active),
            super::RuntimeLifecycle::Active
        );
        assert_eq!(
            map_lifecycle(&Lifecycle::Deprecated),
            super::RuntimeLifecycle::Deprecated
        );
        assert_eq!(
            map_lifecycle(&Lifecycle::Retired),
            super::RuntimeLifecycle::Retired
        );
        assert_eq!(
            map_lifecycle(&Lifecycle::Archived),
            super::RuntimeLifecycle::Archived
        );
    }

    #[test]
    fn runtime_candidate_helper_copies_registry_shape() {
        let capability = resolved_capability(
            Some(traverse_registry::BinaryReference {
                format: traverse_registry::BinaryFormat::Wasm,
                location: "artifact.wasm".to_string(),
            }),
            Lifecycle::Deprecated,
        );

        let candidate = runtime_candidate(&capability, CandidateReason::IntentMatch);

        assert_eq!(candidate.reason, CandidateReason::IntentMatch);
        assert_eq!(candidate.lifecycle, super::RuntimeLifecycle::Deprecated);
        assert_eq!(
            candidate.implementation_kind,
            super::RuntimeImplementationKind::Executable
        );
    }

    #[test]
    fn successful_runtime_execution_reports_completed_result_status() {
        let mut events = super::StateEmitter::new("exec_1", "req_1");
        events.push(
            RuntimeState::LoadingRegistry,
            RuntimeTransitionReasonCode::RuntimeInitializationStarted,
            json!({}),
        );
        events.push(
            RuntimeState::Ready,
            RuntimeTransitionReasonCode::RegistryLoaded,
            json!({}),
        );
        events.push(
            RuntimeState::Discovering,
            RuntimeTransitionReasonCode::RequestStarted,
            json!({}),
        );
        events.push(
            RuntimeState::EvaluatingConstraints,
            RuntimeTransitionReasonCode::CandidatesCollected,
            json!({"candidate_count": 1}),
        );
        events.push(
            RuntimeState::Selecting,
            RuntimeTransitionReasonCode::ConstraintsEvaluated,
            json!({"eligible_candidates": 1}),
        );
        events.push(
            RuntimeState::Executing,
            RuntimeTransitionReasonCode::CandidateSelected,
            json!({"capability_id": "content.comments.create-comment-draft"}),
        );
        let attempt = super::AttemptContext {
            request: valid_request(),
            execution_id: "exec_1".to_string(),
            trace_id: "trace_exec_1".to_string(),
            observability: super::RuntimeObservabilityConfig::default(),
        };
        let capability = resolved_capability(
            Some(traverse_registry::BinaryReference {
                format: traverse_registry::BinaryFormat::Wasm,
                location: "artifact.wasm".to_string(),
            }),
            Lifecycle::Active,
        );

        let outcome = super::successful_execution_outcome(
            super::ExecutionContext {
                attempt,
                emitter: events,
                candidate_collection: super::CandidateCollectionRecord {
                    lookup_scope: PreferPrivate,
                    candidates: vec![runtime_candidate(&capability, CandidateReason::ExactMatch)],
                    rejected_candidates: Vec::new(),
                },
                selection: super::SelectionRecord {
                    status: super::SelectionStatus::Selected,
                    selected_capability_id: Some(capability.record.id.clone()),
                    selected_capability_version: Some(capability.record.version.clone()),
                    failure_reason: None,
                    remaining_candidates: Vec::new(),
                },
            },
            &capability,
            super::StartedExecution {
                started_at: "1970-01-01T00:00:00Z".to_string(),
                placement: super::resolve_placement(PlacementTarget::Local)
                    .unwrap_or_else(|_| unreachable!("local placement should resolve")),
            },
            json!({"draft_id": "draft-1"}),
            capability.contract.emits.clone(),
            None,
        );

        assert_eq!(outcome.result.status, RuntimeResultStatus::Completed);
        assert_eq!(
            outcome.state_events.last().map(|event| event.state),
            Some(RuntimeState::Ready)
        );
        assert_eq!(
            outcome.trace.decision_evidence.selection.status,
            super::SelectionStatus::Selected
        );
        assert_eq!(
            outcome.trace.state_progression.state_events,
            outcome.state_events
        );
        assert_eq!(
            outcome.trace.terminal_outcome.runtime_status,
            RuntimeResultStatus::Completed
        );
        assert_eq!(outcome.trace.emitted_events, capability.contract.emits);
        assert_eq!(
            outcome.trace.state_machine_validation.status,
            super::RuntimeStateMachineValidationStatus::Passed
        );
    }

    #[test]
    fn runtime_execution_produces_otel_phase_spans() {
        let mut registry = CapabilityRegistry::new();
        assert!(registry.register(public_registration()).is_ok());
        let runtime = Runtime::new(registry, NoopExecutor);
        let outcome = runtime.execute(valid_request());
        let spans = &outcome.trace.otel_trace.spans;
        let names: Vec<&str> = spans.iter().map(|span| span.name.as_str()).collect();

        assert_eq!(spans.len(), 6);
        assert!(names.contains(&"traverse.runtime.request"));
        assert!(names.contains(&"traverse.request.intake"));
        assert!(names.contains(&"traverse.registry.lookup"));
        assert!(names.contains(&"traverse.contract.validation"));
        assert!(names.contains(&"traverse.capability.execution"));
        assert!(names.contains(&"traverse.trace.assembly"));
        assert!(
            spans
                .iter()
                .all(|span| span.status == super::OTelSpanStatus::Ok)
        );
        assert!(spans.iter().all(|span| {
            span.attributes
                .iter()
                .all(|attr| attr.key.starts_with("traverse.") || attr.key == "service.name")
        }));
    }

    #[test]
    fn runtime_otel_trace_propagates_w3c_context_and_exporter_config() {
        let mut registry = CapabilityRegistry::new();
        assert!(registry.register(public_registration()).is_ok());
        let runtime = Runtime::new(registry, NoopExecutor).with_observability_config(
            super::RuntimeObservabilityConfig {
                exporter: super::OTelExporterConfig {
                    endpoint: Some("http://collector:4318".to_string()),
                    protocol: super::OtlpProtocol::Http,
                },
                ..super::RuntimeObservabilityConfig::deterministic_test("seed-1")
            },
        );
        let mut request = valid_request();
        request.context.traceparent =
            Some("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01".to_string());
        request.context.tracestate = Some("vendor=value".to_string());

        let first = runtime.execute(request.clone()).trace.otel_trace;
        let second = runtime.execute(request).trace.otel_trace;

        assert_eq!(first.trace_id, second.trace_id);
        assert_eq!(first.spans[0].span_id, second.spans[0].span_id);
        assert_eq!(
            first.parent_traceparent.as_deref(),
            Some("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01")
        );
        assert_eq!(first.tracestate.as_deref(), Some("vendor=value"));
        assert!(first.exporter.enabled);
        assert_eq!(
            first.exporter.endpoint.as_deref(),
            Some("http://collector:4318")
        );
        assert_eq!(
            runtime.observability_config().exporter.endpoint.as_deref(),
            Some("http://collector:4318")
        );
    }

    #[test]
    fn otel_timestamp_helpers_default_without_transitions() {
        let transitions = Vec::new();

        assert_eq!(
            super::first_transition_time(&transitions),
            "1970-01-01T00:00:00Z"
        );
        assert_eq!(
            super::last_transition_time(&transitions),
            "1970-01-01T00:00:00Z"
        );
        assert_eq!(
            super::phase_started_at(&transitions, 0),
            "1970-01-01T00:00:00Z"
        );
        assert_eq!(
            super::phase_ended_at(&transitions, 0),
            "1970-01-01T00:00:00Z"
        );
    }

    #[test]
    fn state_emitter_records_transition_validation_and_rejects_invalid_moves() {
        let mut events = super::StateEmitter::new("exec_1", "req_1");

        assert!(events.try_push(
            RuntimeState::LoadingRegistry,
            RuntimeTransitionReasonCode::RuntimeInitializationStarted,
            json!({})
        ));
        assert!(!events.try_push(
            RuntimeState::Completed,
            RuntimeTransitionReasonCode::ExecutionSucceeded,
            json!({})
        ));

        let finished = events.finish();

        assert_eq!(finished.events.len(), 1);
        assert_eq!(finished.transitions.len(), 1);
        assert_eq!(
            finished.validation.status,
            super::RuntimeStateMachineValidationStatus::Failed
        );
        assert_eq!(finished.validation.violations.len(), 1);
    }

    #[test]
    fn pre_execution_failure_from_constraint_phase_uses_constraint_reason() {
        let mut events = super::StateEmitter::new("exec_1", "req_1");
        events.push(
            RuntimeState::LoadingRegistry,
            RuntimeTransitionReasonCode::RuntimeInitializationStarted,
            json!({}),
        );
        events.push(
            RuntimeState::Ready,
            RuntimeTransitionReasonCode::RegistryLoaded,
            json!({}),
        );
        events.push(
            RuntimeState::Discovering,
            RuntimeTransitionReasonCode::RequestStarted,
            json!({}),
        );
        events.push(
            RuntimeState::EvaluatingConstraints,
            RuntimeTransitionReasonCode::CandidatesCollected,
            json!({"candidate_count": 1}),
        );

        let outcome = super::pre_execution_failure_outcome(
            super::ExecutionContext {
                attempt: super::AttemptContext {
                    request: valid_request(),
                    execution_id: "exec_1".to_string(),
                    trace_id: "trace_exec_1".to_string(),
                    observability: super::RuntimeObservabilityConfig::default(),
                },
                emitter: events,
                candidate_collection: super::CandidateCollectionRecord {
                    lookup_scope: PreferPrivate,
                    candidates: Vec::new(),
                    rejected_candidates: Vec::new(),
                },
                selection: super::SelectionRecord {
                    status: super::SelectionStatus::NoMatch,
                    selected_capability_id: None,
                    selected_capability_version: None,
                    failure_reason: Some(super::SelectionFailureReason::NotRunnable),
                    remaining_candidates: Vec::new(),
                },
            },
            super::PreExecutionFailure {
                artifact_ref: None,
                failure_reason: super::ExecutionFailureReason::ArtifactMissing,
                placement: super::placement_not_attempted(
                    PlacementTarget::Local,
                    super::PlacementDecisionReason::SelectionNotReached,
                ),
                error: super::runtime_error(
                    super::RuntimeErrorCode::CapabilityNotRunnable,
                    "not runnable",
                    json!({}),
                ),
            },
        );

        assert_eq!(
            outcome.trace.state_transitions[4].reason_code,
            RuntimeTransitionReasonCode::ConstraintValidationFailed
        );
    }

    #[test]
    fn detail_object_wraps_non_object_values() {
        let wrapped = super::detail_object(json!("value"));

        assert_eq!(wrapped.get("value"), Some(&json!("value")));
    }

    #[test]
    fn collect_candidates_handles_missing_target_and_public_discovery() {
        let runtime = super::Runtime::new(CapabilityRegistry::new(), NoopExecutor);
        let mut request = valid_request();
        request.intent.capability_id = None;
        request.intent.capability_version = None;
        request.intent.intent_key = None;

        assert!(
            runtime
                .collect_candidates(&request, CandidateReason::IntentMatch)
                .is_empty()
        );

        let mut registry = CapabilityRegistry::new();
        let outcome = registry.register(public_registration());
        assert!(outcome.is_ok());

        let runtime = super::Runtime::new(registry, NoopExecutor);
        let mut request = valid_request();
        request.lookup.scope = PublicOnly;
        request.intent.capability_id = None;
        request.intent.capability_version = None;
        request.intent.intent_key = Some("content.comments.create-comment-draft".to_string());

        let candidates = runtime.collect_candidates(&request, CandidateReason::IntentMatch);

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].record.scope, RegistryScope::Public);
    }

    #[test]
    fn noop_executor_returns_structured_output() {
        let executor = NoopExecutor;
        let capability = resolved_capability(
            Some(BinaryReference {
                format: BinaryFormat::Wasm,
                location: "artifact.wasm".to_string(),
            }),
            Lifecycle::Active,
        );

        let result = executor.execute(&capability, &json!({}));

        assert_eq!(result, Ok(json!({"draft_id": "draft"})));
    }

    #[test]
    fn browser_subscription_validation_covers_guard_branches() {
        let mut request = valid_browser_subscription_request();
        request.kind = "wrong".to_string();
        assert_eq!(
            validate_browser_subscription_request(&request).map(|error| error.code),
            Some(BrowserRuntimeSubscriptionErrorCode::InvalidRequest)
        );

        let mut request = valid_browser_subscription_request();
        request.schema_version = "9.9.9".to_string();
        assert_eq!(
            validate_browser_subscription_request(&request).map(|error| error.code),
            Some(BrowserRuntimeSubscriptionErrorCode::InvalidRequest)
        );

        let mut request = valid_browser_subscription_request();
        request.governing_spec = "wrong-spec".to_string();
        assert_eq!(
            validate_browser_subscription_request(&request).map(|error| error.code),
            Some(BrowserRuntimeSubscriptionErrorCode::InvalidRequest)
        );
    }

    #[test]
    fn browser_subscription_reports_not_found_for_mismatched_target() {
        let outcome = runtime_outcome_for_browser_subscription();
        let request = BrowserRuntimeSubscriptionRequest {
            request_id: Some("req_other".to_string()),
            execution_id: None,
            ..valid_browser_subscription_request()
        };

        let messages = browser_subscription_messages(&request, &outcome);
        assert_eq!(
            messages,
            vec![BrowserRuntimeSubscriptionMessage::Error(
                super::BrowserRuntimeSubscriptionErrorMessage {
                    kind: "browser_runtime_subscription_error".to_string(),
                    schema_version: "1.0.0".to_string(),
                    sequence: 0,
                    code: BrowserRuntimeSubscriptionErrorCode::NotFound,
                    message: "subscription target did not match the supplied execution outcome"
                        .to_string(),
                }
            )]
        );
    }

    #[test]
    fn browser_subscription_target_helper_covers_fallback_branch() {
        let outcome = runtime_outcome_for_browser_subscription();
        let invalid_request = BrowserRuntimeSubscriptionRequest {
            request_id: Some("req_123".to_string()),
            execution_id: Some(outcome.result.execution_id.clone()),
            ..valid_browser_subscription_request()
        };

        assert!(!subscription_targets_outcome(&invalid_request, &outcome));
    }

    fn valid_request() -> RuntimeRequest {
        RuntimeRequest {
            kind: "runtime_request".to_string(),
            schema_version: "1.0.0".to_string(),
            request_id: "req_123".to_string(),
            intent: RuntimeIntent {
                capability_id: Some("content.comments.create-comment-draft".to_string()),
                capability_version: Some("1.0.0".to_string()),
                version_range: None,
                intent_key: Some("content.comments.create-comment-draft".to_string()),
            },
            input: json!({"comment_text": "Hello", "resource_id": "res-1"}),
            lookup: RuntimeLookup {
                scope: RuntimeLookupScope::PreferPrivate,
                allow_ambiguity: false,
            },
            context: RuntimeContext {
                requested_target: PlacementTarget::Local,
                correlation_id: None,
                caller: None,
                traceparent: None,
                tracestate: None,
                metadata: None,
            },
            governing_spec: "006-runtime-request-execution".to_string(),
        }
    }

    fn valid_browser_subscription_request() -> BrowserRuntimeSubscriptionRequest {
        BrowserRuntimeSubscriptionRequest {
            kind: "browser_runtime_subscription_request".to_string(),
            schema_version: "1.0.0".to_string(),
            governing_spec: "013-browser-runtime-subscription".to_string(),
            request_id: Some("req_123".to_string()),
            execution_id: None,
        }
    }

    fn runtime_outcome_for_browser_subscription() -> super::RuntimeExecutionOutcome {
        let mut registry = CapabilityRegistry::new();
        assert!(registry.register(public_registration()).is_ok());
        let runtime = Runtime::new(registry, NoopExecutor);
        runtime.execute(valid_request())
    }

    fn public_registration() -> CapabilityRegistration {
        CapabilityRegistration {
            scope: RegistryScope::Public,
            contract: test_contract(Lifecycle::Active),
            contract_path: "registry/contract.json".to_string(),
            artifact: test_artifact(Some(BinaryReference {
                format: BinaryFormat::Wasm,
                location: "artifact.wasm".to_string(),
            })),
            registered_at: "2026-03-27T00:00:00Z".to_string(),
            tags: vec!["comments".to_string()],
            composability: ComposabilityMetadata {
                kind: CompositionKind::Atomic,
                patterns: vec![CompositionPattern::Sequential],
                provides: vec!["draft".to_string()],
                requires: vec!["authenticated-user".to_string()],
            },
            governing_spec: "005-capability-registry".to_string(),
            validator_version: "0.1.0".to_string(),
        }
    }

    fn resolved_capability(
        binary: Option<traverse_registry::BinaryReference>,
        lifecycle: Lifecycle,
    ) -> ResolvedCapability {
        ResolvedCapability {
            contract: test_contract(lifecycle.clone()),
            record: test_record(lifecycle.clone()),
            artifact: test_artifact(binary),
            index_entry: test_index_entry(lifecycle),
        }
    }

    fn test_contract(lifecycle: Lifecycle) -> traverse_contracts::CapabilityContract {
        traverse_contracts::CapabilityContract {
            kind: "capability_contract".to_string(),
            schema_version: "1.0.0".to_string(),
            id: "content.comments.create-comment-draft".to_string(),
            namespace: "content.comments".to_string(),
            name: "create-comment-draft".to_string(),
            version: "1.0.0".to_string(),
            lifecycle,
            owner: Owner {
                team: "comments".to_string(),
                contact: "comments@example.com".to_string(),
            },
            summary: "Create a comment draft for a resource".to_string(),
            description: "Creates a draft comment and returns the generated draft identifier."
                .to_string(),
            inputs: SchemaContainer {
                schema: json!({"type": "object"}),
            },
            outputs: SchemaContainer {
                schema: json!({"type": "object"}),
            },
            preconditions: Vec::new(),
            postconditions: Vec::new(),
            side_effects: vec![traverse_contracts::SideEffect {
                kind: traverse_contracts::SideEffectKind::MemoryOnly,
                description: "Produces a draft representation in memory.".to_string(),
            }],
            emits: Vec::new(),
            consumes: Vec::new(),
            permissions: Vec::new(),
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
                author: "Enrico Piovesan".to_string(),
                created_at: "2026-03-27T00:00:00Z".to_string(),
                spec_ref: Some("006-runtime-request-execution".to_string()),
                adr_refs: Vec::new(),
                exception_refs: Vec::new(),
            },
            evidence: Vec::new(),
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

    fn test_record(lifecycle: Lifecycle) -> CapabilityRegistryRecord {
        CapabilityRegistryRecord {
            scope: RegistryScope::Private,
            id: "content.comments.create-comment-draft".to_string(),
            version: "1.0.0".to_string(),
            lifecycle,
            owner: Owner {
                team: "comments".to_string(),
                contact: "comments@example.com".to_string(),
            },
            contract_path: "registry/contract.json".to_string(),
            contract_digest: "digest".to_string(),
            implementation_kind: ImplementationKind::Executable,
            artifact_ref: "artifact:content.comments.create-comment-draft:1.0.0".to_string(),
            registered_at: "2026-03-27T00:00:00Z".to_string(),
            provenance: RegistryProvenance {
                source: "test".to_string(),
                author: "Enrico Piovesan".to_string(),
                created_at: "2026-03-27T00:00:00Z".to_string(),
            },
            evidence: traverse_registry::RegistrationEvidence {
                evidence_id: "evidence".to_string(),
                artifact_ref: "artifact:content.comments.create-comment-draft:1.0.0".to_string(),
                capability_id: "content.comments.create-comment-draft".to_string(),
                capability_version: "1.0.0".to_string(),
                scope: RegistryScope::Private,
                governing_spec: "005-capability-registry".to_string(),
                validator_version: "0.1.0".to_string(),
                produced_at: "2026-03-27T00:00:00Z".to_string(),
                result: traverse_registry::RegistrationResult::Passed,
            },
        }
    }

    fn test_artifact(
        binary: Option<traverse_registry::BinaryReference>,
    ) -> CapabilityArtifactRecord {
        CapabilityArtifactRecord {
            artifact_ref: "artifact:content.comments.create-comment-draft:1.0.0".to_string(),
            implementation_kind: ImplementationKind::Executable,
            source: SourceReference {
                kind: SourceKind::Git,
                location: "https://github.com/enricopiovesan/cogolo".to_string(),
            },
            binary,
            workflow_ref: None,
            digests: ArtifactDigests {
                source_digest: "src-digest".to_string(),
                binary_digest: Some("bin-digest".to_string()),
            },
            provenance: RegistryProvenance {
                source: "test".to_string(),
                author: "Enrico Piovesan".to_string(),
                created_at: "2026-03-27T00:00:00Z".to_string(),
            },
        }
    }

    fn test_index_entry(lifecycle: Lifecycle) -> DiscoveryIndexEntry {
        DiscoveryIndexEntry {
            scope: RegistryScope::Private,
            id: "content.comments.create-comment-draft".to_string(),
            version: "1.0.0".to_string(),
            lifecycle,
            owner: Owner {
                team: "comments".to_string(),
                contact: "comments@example.com".to_string(),
            },
            summary: "Create a comment draft for a resource".to_string(),
            tags: vec!["comments".to_string()],
            permissions: Vec::new(),
            emits: Vec::new(),
            consumes: Vec::new(),
            implementation_kind: ImplementationKind::Executable,
            composability: traverse_registry::ComposabilityMetadata {
                kind: traverse_registry::CompositionKind::Atomic,
                patterns: vec![traverse_registry::CompositionPattern::Sequential],
                provides: vec!["draft".to_string()],
                requires: vec!["authenticated-user".to_string()],
            },
            artifact_ref: "artifact:content.comments.create-comment-draft:1.0.0".to_string(),
            registered_at: "2026-03-27T00:00:00Z".to_string(),
        }
    }

    struct NoopExecutor;

    impl super::LocalExecutor for NoopExecutor {
        fn execute(
            &self,
            _capability: &ResolvedCapability,
            _input: &serde_json::Value,
        ) -> Result<serde_json::Value, super::LocalExecutionFailure> {
            Ok(json!({"draft_id": "draft"}))
        }
    }

    struct FailingExecutor;

    impl super::LocalExecutor for FailingExecutor {
        fn execute(
            &self,
            _capability: &ResolvedCapability,
            _input: &serde_json::Value,
        ) -> Result<serde_json::Value, super::LocalExecutionFailure> {
            Err(super::LocalExecutionFailure {
                code: super::LocalExecutionFailureCode::ExecutionFailed,
                message: "forced failure".to_string(),
            })
        }
    }

    fn successful_trace() -> super::RuntimeTrace {
        let mut registry = CapabilityRegistry::new();
        assert!(registry.register(public_registration()).is_ok());
        let runtime = Runtime::new(registry, NoopExecutor);
        runtime.execute(valid_request()).trace
    }

    fn failed_trace() -> super::RuntimeTrace {
        let mut registry = CapabilityRegistry::new();
        assert!(registry.register(public_registration()).is_ok());
        let runtime = Runtime::new(registry, FailingExecutor);
        runtime.execute(valid_request()).trace
    }

    #[test]
    fn selected_capability_id_returns_id_on_success() {
        let trace = successful_trace();
        assert_eq!(
            trace.selected_capability_id(),
            Some("content.comments.create-comment-draft")
        );
    }

    #[test]
    fn selected_capability_id_returns_none_when_no_selection() {
        let registry = CapabilityRegistry::new();
        // empty registry — no capability matches
        let runtime = Runtime::new(registry, NoopExecutor);
        let trace = runtime.execute(valid_request()).trace;
        assert!(trace.selected_capability_id().is_none());
    }

    #[test]
    fn errors_returns_none_on_success() {
        let trace = successful_trace();
        assert!(trace.errors().is_none());
    }

    #[test]
    fn errors_returns_error_on_failure() {
        let trace = failed_trace();
        assert!(trace.errors().is_some());
    }

    #[test]
    fn emitted_events_returns_slice() {
        let trace = successful_trace();
        // NoopExecutor emits no events; method must not panic and slice is valid
        let _ = trace.emitted_events();
    }

    #[test]
    fn output_returns_value_on_success() {
        let trace = successful_trace();
        assert_eq!(trace.output(), Some(&json!({"draft_id": "draft"})));
    }

    #[test]
    fn output_returns_none_on_failure() {
        let trace = failed_trace();
        assert!(trace.output().is_none());
    }

    #[test]
    fn is_success_true_on_completed() {
        let trace = successful_trace();
        assert!(trace.is_success());
    }

    #[test]
    fn is_success_false_on_error() {
        let trace = failed_trace();
        assert!(!trace.is_success());
    }
}
