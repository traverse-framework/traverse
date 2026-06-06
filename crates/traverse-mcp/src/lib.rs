//! MCP-facing surfaces for Traverse.
//!
//! Governed by spec 015-capability-discovery-mcp and spec 042-mcp-library-surface

mod stdio_server;

pub mod context;
pub mod error;
pub mod tools;

pub use context::McpContext;
pub use stdio_server::*;

use std::collections::HashMap;
use traverse_registry::{
    CapabilityRegistry, DiscoveryQuery, EventRegistry, LookupScope, RegistryScope,
    ResolvedCapability, ResolvedEvent, ResolvedWorkflow, WorkflowRegistry,
};
use traverse_runtime::{
    LocalExecutor, Runtime, RuntimeErrorCode, RuntimeExecutionOutcome, RuntimeRequest,
    RuntimeResult, RuntimeStateEvent, RuntimeTrace,
};

// ---------------------------------------------------------------------------
// Spec 042-mcp-library-surface: free functions
// ---------------------------------------------------------------------------

/// Returns a JSON array summarising every capability visible in `registry`.
///
/// Uses `LookupScope::PreferPrivate` so private overrides are preferred over
/// public entries when both exist for the same id.
#[must_use]
pub fn discover_capabilities(registry: &CapabilityRegistry) -> serde_json::Value {
    let entries = registry.discover(LookupScope::PreferPrivate, &DiscoveryQuery::default());
    let summaries: Vec<serde_json::Value> = entries
        .into_iter()
        .map(|e| {
            serde_json::json!({
                "id": e.id,
                "version": e.version,
                "lifecycle": lifecycle_name(&e.lifecycle),
                "summary": e.summary,
                "tags": e.tags,
            })
        })
        .collect();
    serde_json::Value::Array(summaries)
}

/// Returns `Some(JSON object)` for the capability with the given `id`, or
/// `None` when the id is not found in `registry`.
///
/// Resolves the latest registered version using `LookupScope::PreferPrivate`.
#[must_use]
pub fn get_capability(registry: &CapabilityRegistry, id: &str) -> Option<serde_json::Value> {
    let entries = registry.discover(LookupScope::PreferPrivate, &DiscoveryQuery::default());
    let entry = entries.into_iter().find(|e| e.id == id)?;
    let resolved = registry.find_exact(LookupScope::PreferPrivate, &entry.id, &entry.version)?;
    serde_json::to_value(&resolved.contract).ok()
}

/// Executes `request` through `runtime` and returns a JSON summary of the
/// outcome, including result status, execution id, and any error.
#[must_use]
pub fn execute_capability<E: LocalExecutor>(
    runtime: &Runtime<E>,
    request: RuntimeRequest,
) -> serde_json::Value {
    let outcome = runtime.execute(request);
    serde_json::json!({
        "status": outcome.result.status,
        "request_id": outcome.result.request_id,
        "execution_id": outcome.result.execution_id,
        "error": outcome.result.error,
    })
}

// ---------------------------------------------------------------------------
// Spec 042-mcp-library-surface: McpToolRegistry
// ---------------------------------------------------------------------------

/// A runtime registry that maps MCP tool names to handler functions.
///
/// Enables third-party MCP servers to embed Traverse tools without taking a
/// direct dependency on `TraverseMcp`.  Each handler receives a
/// `serde_json::Value` argument bag and returns a `serde_json::Value` result.
pub struct McpToolRegistry {
    handlers: HashMap<String, Box<dyn Fn(serde_json::Value) -> serde_json::Value + Send + Sync>>,
}

impl McpToolRegistry {
    /// Create an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            handlers: HashMap::new(),
        }
    }

    /// Register `handler` under `name`.  A second call with the same `name`
    /// replaces the previous handler.
    pub fn register_tool<F>(&mut self, name: impl Into<String>, handler: F)
    where
        F: Fn(serde_json::Value) -> serde_json::Value + Send + Sync + 'static,
    {
        self.handlers.insert(name.into(), Box::new(handler));
    }

    /// Dispatch `args` to the handler registered under `tool_name`.
    ///
    /// Returns `Some(result)` when a handler is found, `None` otherwise.
    #[must_use]
    pub fn dispatch(&self, tool_name: &str, args: serde_json::Value) -> Option<serde_json::Value> {
        self.handlers.get(tool_name).map(|handler| handler(args))
    }
}

impl Default for McpToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for McpToolRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpToolRegistry")
            .field("tool_count", &self.handlers.len())
            .finish()
    }
}

#[derive(Debug)]
pub struct TraverseMcp<'a, E> {
    capability_registry: &'a CapabilityRegistry,
    event_registry: &'a EventRegistry,
    workflow_registry: &'a WorkflowRegistry,
    runtime: &'a Runtime<E>,
}

impl<'a, E> TraverseMcp<'a, E>
where
    E: LocalExecutor,
{
    #[must_use]
    pub fn new(
        capability_registry: &'a CapabilityRegistry,
        event_registry: &'a EventRegistry,
        workflow_registry: &'a WorkflowRegistry,
        runtime: &'a Runtime<E>,
    ) -> Self {
        Self {
            capability_registry,
            event_registry,
            workflow_registry,
            runtime,
        }
    }

    #[must_use]
    pub fn discover_capabilities(
        &self,
        lookup_scope: McpLookupScope,
        query: &DiscoveryQuery,
    ) -> Vec<McpArtifactSummary> {
        self.capability_registry
            .discover(map_lookup_scope(lookup_scope), query)
            .into_iter()
            .map(|entry| McpArtifactSummary {
                artifact_kind: McpArtifactKind::Capability,
                scope: map_registry_scope(entry.scope),
                id: entry.id,
                version: entry.version,
                lifecycle: lifecycle_name(&entry.lifecycle).to_string(),
                summary: entry.summary,
                owner_team: Some(entry.owner.team),
                tags: entry.tags,
                provenance_summary: None,
            })
            .collect()
    }

    #[must_use]
    pub fn discover_events(&self, lookup_scope: McpLookupScope) -> Vec<McpArtifactSummary> {
        self.event_registry
            .discover(map_lookup_scope(lookup_scope))
            .into_iter()
            .map(|entry| McpArtifactSummary {
                artifact_kind: McpArtifactKind::Event,
                scope: map_registry_scope(entry.scope),
                id: entry.id,
                version: entry.version,
                lifecycle: lifecycle_name(&entry.lifecycle).to_string(),
                summary: entry.summary,
                owner_team: None,
                tags: entry.tags,
                provenance_summary: Some(format!("{:?}", entry.classification)),
            })
            .collect()
    }

    #[must_use]
    pub fn discover_workflows(&self, lookup_scope: McpLookupScope) -> Vec<McpArtifactSummary> {
        self.workflow_registry
            .discover(map_lookup_scope(lookup_scope))
            .into_iter()
            .map(|entry| McpArtifactSummary {
                artifact_kind: McpArtifactKind::Workflow,
                scope: map_registry_scope(entry.scope),
                id: entry.id,
                version: entry.version,
                lifecycle: lifecycle_name(&entry.lifecycle).to_string(),
                summary: entry.summary,
                owner_team: Some(entry.owner.team),
                tags: entry.tags,
                provenance_summary: Some(format!(
                    "start={} terminals={}",
                    entry.start_node,
                    entry.terminal_nodes.join(",")
                )),
            })
            .collect()
    }

    /// Resolves one governed capability artifact by exact id and version.
    ///
    /// # Errors
    ///
    /// Returns [`McpError`] when the requested capability does not exist.
    pub fn get_capability(
        &self,
        lookup_scope: McpLookupScope,
        id: &str,
        version: &str,
    ) -> Result<McpArtifactDetail, McpError> {
        self.capability_registry
            .find_exact(map_lookup_scope(lookup_scope), id, version)
            .map(|item| McpArtifactDetail::Capability(Box::new(item)))
            .ok_or_else(|| not_found("capability", id, version))
    }

    /// Resolves one governed event artifact by exact id and version.
    ///
    /// # Errors
    ///
    /// Returns [`McpError`] when the requested event does not exist.
    pub fn get_event(
        &self,
        lookup_scope: McpLookupScope,
        id: &str,
        version: &str,
    ) -> Result<McpArtifactDetail, McpError> {
        self.event_registry
            .find_exact(map_lookup_scope(lookup_scope), id, version)
            .map(|item| McpArtifactDetail::Event(Box::new(item)))
            .ok_or_else(|| not_found("event", id, version))
    }

    /// Resolves one governed workflow artifact by exact id and version.
    ///
    /// # Errors
    ///
    /// Returns [`McpError`] when the requested workflow does not exist.
    pub fn get_workflow(
        &self,
        lookup_scope: McpLookupScope,
        id: &str,
        version: &str,
    ) -> Result<McpArtifactDetail, McpError> {
        self.workflow_registry
            .find_exact(map_lookup_scope(lookup_scope), id, version)
            .map(|item| McpArtifactDetail::Workflow(Box::new(item)))
            .ok_or_else(|| not_found("workflow", id, version))
    }

    /// Executes one governed runtime request through the MCP-facing surface.
    ///
    /// # Errors
    ///
    /// Returns [`McpError`] when runtime validation, resolution, or execution
    /// fails for the supplied request.
    pub fn execute(&self, request: RuntimeRequest) -> Result<McpExecutionResponse, McpError> {
        let outcome = self.runtime.execute(request);
        if let Some(error) = outcome.result.error.as_ref() {
            return Err(map_runtime_error(error.code, &error.message));
        }

        Ok(McpExecutionResponse {
            result: outcome.result.clone(),
            trace: outcome.trace.clone(),
            observation_messages: observation_messages_from_outcome(&outcome),
        })
    }

    #[must_use]
    pub fn observe_execution(
        &self,
        outcome: &RuntimeExecutionOutcome,
    ) -> Vec<McpObservationMessage> {
        observation_messages_from_outcome(outcome)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpConsumptionValidationPath {
    pub consumer_name: &'static str,
    pub validated_flow_id: &'static str,
    pub public_surface_id: &'static str,
    pub governing_specs: &'static [&'static str],
    pub required_public_surfaces: &'static [&'static str],
    pub exposed_tools: &'static [&'static str],
    pub internal_non_goals: &'static [&'static str],
}

#[must_use]
pub fn youaskm3_mcp_consumption_validation_path() -> McpConsumptionValidationPath {
    McpConsumptionValidationPath {
        consumer_name: "youaskm3",
        validated_flow_id: "youaskm3_mcp_validation",
        public_surface_id: "traverse.mcp.downstream-consumer",
        governing_specs: &[
            "019-downstream-consumer-contract",
            "020-downstream-integration-validation",
            "021-app-facing-operational-constraints",
        ],
        required_public_surfaces: &[
            "mcp.capabilities.discover",
            "mcp.capability.get",
            "mcp.runtime.execute",
            "mcp.runtime.observe_execution",
        ],
        exposed_tools: &[
            "mcp.capabilities.discover",
            "mcp.capability.get",
            "mcp.runtime.execute",
            "mcp.runtime.observe_execution",
        ],
        internal_non_goals: &[
            "internal_crate_layout",
            "private_helper_modules",
            "undocumented_message_types",
        ],
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpLookupScope {
    PublicOnly,
    PreferPrivate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpRegistryScope {
    Public,
    Private,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpArtifactKind {
    Capability,
    Event,
    Workflow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpArtifactSummary {
    pub artifact_kind: McpArtifactKind,
    pub scope: McpRegistryScope,
    pub id: String,
    pub version: String,
    pub lifecycle: String,
    pub summary: String,
    pub owner_team: Option<String>,
    pub tags: Vec<String>,
    pub provenance_summary: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpArtifactDetail {
    Capability(Box<ResolvedCapability>),
    Event(Box<ResolvedEvent>),
    Workflow(Box<ResolvedWorkflow>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpExecutionResponse {
    pub result: RuntimeResult,
    pub trace: RuntimeTrace,
    pub observation_messages: Vec<McpObservationMessage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpObservationMessage {
    Lifecycle(McpLifecycleMessage),
    State(McpStateMessage),
    Trace(Box<McpTraceMessage>),
    Terminal(McpTerminalMessage),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpLifecycleMessage {
    pub sequence: u64,
    pub execution_id: String,
    pub request_id: String,
    pub status: McpLifecycleStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpLifecycleStatus {
    StreamStarted,
    StreamCompleted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpStateMessage {
    pub sequence: u64,
    pub state_event: RuntimeStateEvent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpTraceMessage {
    pub sequence: u64,
    pub trace: RuntimeTrace,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpTerminalMessage {
    pub sequence: u64,
    pub result: RuntimeResult,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpErrorCode {
    InvalidRequest,
    NotFound,
    AmbiguousMatch,
    ValidationFailed,
    ExecutionFailed,
    UnsupportedOperation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpError {
    pub code: McpErrorCode,
    pub message: String,
}

#[must_use]
pub fn observation_messages_from_outcome(
    outcome: &RuntimeExecutionOutcome,
) -> Vec<McpObservationMessage> {
    let mut sequence = 0_u64;
    let mut messages = Vec::new();
    messages.push(McpObservationMessage::Lifecycle(McpLifecycleMessage {
        sequence,
        execution_id: outcome.result.execution_id.clone(),
        request_id: outcome.result.request_id.clone(),
        status: McpLifecycleStatus::StreamStarted,
    }));
    sequence += 1;

    for state_event in &outcome.state_events {
        messages.push(McpObservationMessage::State(McpStateMessage {
            sequence,
            state_event: state_event.clone(),
        }));
        sequence += 1;
    }

    messages.push(McpObservationMessage::Trace(Box::new(McpTraceMessage {
        sequence,
        trace: outcome.trace.clone(),
    })));
    sequence += 1;

    messages.push(McpObservationMessage::Terminal(McpTerminalMessage {
        sequence,
        result: outcome.result.clone(),
    }));
    sequence += 1;

    messages.push(McpObservationMessage::Lifecycle(McpLifecycleMessage {
        sequence,
        execution_id: outcome.result.execution_id.clone(),
        request_id: outcome.result.request_id.clone(),
        status: McpLifecycleStatus::StreamCompleted,
    }));
    messages
}

fn map_lookup_scope(scope: McpLookupScope) -> LookupScope {
    match scope {
        McpLookupScope::PublicOnly => LookupScope::PublicOnly,
        McpLookupScope::PreferPrivate => LookupScope::PreferPrivate,
    }
}

fn map_registry_scope(scope: RegistryScope) -> McpRegistryScope {
    match scope {
        RegistryScope::Public => McpRegistryScope::Public,
        RegistryScope::Private => McpRegistryScope::Private,
    }
}

fn not_found(kind: &str, id: &str, version: &str) -> McpError {
    McpError {
        code: McpErrorCode::NotFound,
        message: format!("{kind} {id}@{version} was not found"),
    }
}

fn map_runtime_error(code: RuntimeErrorCode, message: &str) -> McpError {
    let code = match code {
        RuntimeErrorCode::RequestInvalid => McpErrorCode::InvalidRequest,
        RuntimeErrorCode::CapabilityNotFound | RuntimeErrorCode::ArtifactMissing => {
            McpErrorCode::NotFound
        }
        RuntimeErrorCode::CapabilityAmbiguous => McpErrorCode::AmbiguousMatch,
        RuntimeErrorCode::CapabilityNotRunnable
        | RuntimeErrorCode::PlacementUnsupported
        | RuntimeErrorCode::OutputValidationFailed
        | RuntimeErrorCode::ContractViolation => McpErrorCode::ValidationFailed,
        RuntimeErrorCode::ExecutionFailed => McpErrorCode::ExecutionFailed,
    };
    McpError {
        code,
        message: message.to_string(),
    }
}

fn lifecycle_name(lifecycle: &traverse_contracts::Lifecycle) -> &'static str {
    match lifecycle {
        traverse_contracts::Lifecycle::Draft => "draft",
        traverse_contracts::Lifecycle::Active => "active",
        traverse_contracts::Lifecycle::Deprecated => "deprecated",
        traverse_contracts::Lifecycle::Retired => "retired",
        traverse_contracts::Lifecycle::Archived => "archived",
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]
    #![allow(clippy::panic)]

    use super::*;
    use serde_json::{Value, json};
    use traverse_contracts::{
        BinaryFormat as ContractBinaryFormat, Condition, DependencyArtifactType,
        DependencyReference, Entrypoint, EntrypointKind, EventClassification, EventContract,
        EventPayload, EventProvenance, EventProvenanceSource, EventReference, EventType,
        EventValidationEvidence, Execution, ExecutionConstraints, ExecutionTarget,
        FilesystemAccess, HostApiAccess, IdReference, Lifecycle, NetworkAccess, Owner,
        PayloadCompatibility, Provenance, ProvenanceSource, SchemaContainer, SideEffect,
        SideEffectKind,
    };
    use traverse_registry::{
        ArtifactDigests, BinaryFormat, BinaryReference, CapabilityArtifactRecord,
        CapabilityRegistration, ComposabilityMetadata, CompositionKind, CompositionPattern,
        EventRegistration, RegistryProvenance, SourceKind, SourceReference, WorkflowDefinition,
        WorkflowNode, WorkflowNodeInput, WorkflowNodeOutput, WorkflowRegistration,
    };
    use traverse_runtime::{
        LocalExecutionFailure, RuntimeContext, RuntimeIntent, RuntimeLookup, RuntimeLookupScope,
        RuntimeResultStatus,
    };

    #[test]
    fn discovers_capabilities_events_and_workflows() {
        let capability_registry = capability_registry_fixture();
        let event_registry = event_registry_fixture();
        let workflow_registry = workflow_registry_fixture(&capability_registry);
        let runtime = runtime_fixture(&capability_registry, &workflow_registry);
        let mcp = TraverseMcp::new(
            &capability_registry,
            &event_registry,
            &workflow_registry,
            &runtime,
        );

        let capabilities =
            mcp.discover_capabilities(McpLookupScope::PreferPrivate, &DiscoveryQuery::default());
        let events = mcp.discover_events(McpLookupScope::PreferPrivate);
        let workflows = mcp.discover_workflows(McpLookupScope::PreferPrivate);

        assert_eq!(capabilities.len(), 1);
        assert_eq!(capabilities[0].artifact_kind, McpArtifactKind::Capability);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].artifact_kind, McpArtifactKind::Event);
        assert_eq!(workflows.len(), 1);
        assert_eq!(workflows[0].artifact_kind, McpArtifactKind::Workflow);
    }

    #[test]
    fn gets_exact_artifacts_by_id_and_version() {
        let capability_registry = capability_registry_fixture();
        let event_registry = event_registry_fixture();
        let workflow_registry = workflow_registry_fixture(&capability_registry);
        let runtime = runtime_fixture(&capability_registry, &workflow_registry);
        let mcp = TraverseMcp::new(
            &capability_registry,
            &event_registry,
            &workflow_registry,
            &runtime,
        );

        let capability = match mcp.get_capability(
            McpLookupScope::PreferPrivate,
            "content.comments.create-comment-draft",
            "1.0.0",
        ) {
            Ok(capability) => capability,
            Err(error) => panic!("capability should resolve: {error:?}"),
        };
        let event = match mcp.get_event(
            McpLookupScope::PreferPrivate,
            "content.comments.draft-created",
            "1.0.0",
        ) {
            Ok(event) => event,
            Err(error) => panic!("event should resolve: {error:?}"),
        };
        let workflow = match mcp.get_workflow(
            McpLookupScope::PreferPrivate,
            "content.comments.publish-comment",
            "1.0.0",
        ) {
            Ok(workflow) => workflow,
            Err(error) => panic!("workflow should resolve: {error:?}"),
        };

        assert!(matches!(capability, McpArtifactDetail::Capability(_)));
        assert!(matches!(event, McpArtifactDetail::Event(_)));
        assert!(matches!(workflow, McpArtifactDetail::Workflow(_)));
    }

    #[test]
    fn executes_and_emits_transport_agnostic_observation_messages() {
        let capability_registry = capability_registry_fixture();
        let event_registry = event_registry_fixture();
        let workflow_registry = workflow_registry_fixture(&capability_registry);
        let runtime = runtime_fixture(&capability_registry, &workflow_registry);
        let mcp = TraverseMcp::new(
            &capability_registry,
            &event_registry,
            &workflow_registry,
            &runtime,
        );

        let response = match mcp.execute(runtime_request()) {
            Ok(response) => response,
            Err(error) => panic!("execution should pass: {error:?}"),
        };

        assert_eq!(response.result.status, RuntimeResultStatus::Completed);
        assert_eq!(
            response.observation_messages.first(),
            Some(&McpObservationMessage::Lifecycle(McpLifecycleMessage {
                sequence: 0,
                execution_id: response.result.execution_id.clone(),
                request_id: response.result.request_id.clone(),
                status: McpLifecycleStatus::StreamStarted,
            }))
        );
        assert!(matches!(
            response
                .observation_messages
                .get(response.observation_messages.len() - 2),
            Some(McpObservationMessage::Terminal(_))
        ));
        assert!(matches!(
            response.observation_messages.last(),
            Some(McpObservationMessage::Lifecycle(McpLifecycleMessage {
                status: McpLifecycleStatus::StreamCompleted,
                ..
            }))
        ));
        assert_eq!(response.trace.result.status, RuntimeResultStatus::Completed);
    }

    #[test]
    fn maps_runtime_failures_into_mcp_errors() {
        let capability_registry = capability_registry_fixture();
        let event_registry = event_registry_fixture();
        let workflow_registry = workflow_registry_fixture(&capability_registry);
        let runtime = runtime_fixture(&capability_registry, &workflow_registry);
        let mcp = TraverseMcp::new(
            &capability_registry,
            &event_registry,
            &workflow_registry,
            &runtime,
        );

        let mut request = runtime_request();
        request.lookup = RuntimeLookup {
            scope: RuntimeLookupScope::PreferPrivate,
            allow_ambiguity: true,
        };

        let Err(error) = mcp.execute(request) else {
            panic!("invalid request should fail");
        };

        assert_eq!(error.code, McpErrorCode::InvalidRequest);
    }

    #[test]
    fn validates_youaskm3_mcp_consumption_path() {
        let capability_registry_for_runtime = public_capability_registry_fixture();
        let capability_registry_for_mcp = public_capability_registry_fixture();
        let event_registry = EventRegistry::new();
        let workflow_registry = WorkflowRegistry::new();
        let runtime = Runtime::new(capability_registry_for_runtime, EchoExecutor);
        let mcp = TraverseMcp::new(
            &capability_registry_for_mcp,
            &event_registry,
            &workflow_registry,
            &runtime,
        );

        let path = youaskm3_mcp_consumption_validation_path();
        assert_eq!(path.consumer_name, "youaskm3");
        assert_eq!(path.validated_flow_id, "youaskm3_mcp_validation");
        assert_eq!(path.public_surface_id, "traverse.mcp.downstream-consumer");
        assert!(
            path.governing_specs
                .contains(&"020-downstream-integration-validation")
        );
        assert!(
            path.governing_specs
                .contains(&"021-app-facing-operational-constraints")
        );
        assert!(
            path.required_public_surfaces
                .contains(&"mcp.runtime.execute")
        );
        assert!(path.exposed_tools.contains(&"mcp.capability.get"));

        let discovered =
            mcp.discover_capabilities(McpLookupScope::PublicOnly, &DiscoveryQuery::default());
        assert_eq!(discovered.len(), 1);
        assert_eq!(discovered[0].scope, McpRegistryScope::Public);
        assert_eq!(discovered[0].id, "content.comments.create-comment-draft");

        let capability = match mcp.get_capability(
            McpLookupScope::PublicOnly,
            "content.comments.create-comment-draft",
            "1.0.0",
        ) {
            Ok(capability) => capability,
            Err(error) => panic!("public capability should resolve: {error:?}"),
        };
        assert!(matches!(capability, McpArtifactDetail::Capability(_)));

        let response = match mcp.execute(runtime_request()) {
            Ok(response) => response,
            Err(error) => panic!("execution should pass: {error:?}"),
        };
        assert_eq!(response.result.status, RuntimeResultStatus::Completed);
        assert_eq!(
            response.observation_messages.first(),
            Some(&McpObservationMessage::Lifecycle(McpLifecycleMessage {
                sequence: 0,
                execution_id: response.result.execution_id.clone(),
                request_id: response.result.request_id.clone(),
                status: McpLifecycleStatus::StreamStarted,
            }))
        );
        assert!(matches!(
            response.observation_messages.last(),
            Some(McpObservationMessage::Lifecycle(McpLifecycleMessage {
                status: McpLifecycleStatus::StreamCompleted,
                ..
            }))
        ));
    }

    fn runtime_fixture<'a>(
        capability_registry: &'a CapabilityRegistry,
        workflow_registry: &'a WorkflowRegistry,
    ) -> Runtime<EchoExecutor> {
        let _ = capability_registry;
        let _ = workflow_registry;
        let registry = capability_registry_fixture();
        let workflows = workflow_registry_fixture(&registry);
        Runtime::new(registry, EchoExecutor).with_workflow_registry(workflows)
    }

    fn capability_registry_fixture() -> CapabilityRegistry {
        let mut registry = CapabilityRegistry::new();
        let outcome = registry.register(CapabilityRegistration {
            scope: RegistryScope::Private,
            contract: capability_contract(),
            contract_path:
                "registry/private/content.comments.create-comment-draft/1.0.0/contract.json"
                    .to_string(),
            artifact: capability_artifact_record(),
            registered_at: "2026-03-30T00:00:00Z".to_string(),
            tags: vec!["comments".to_string()],
            composability: ComposabilityMetadata {
                kind: CompositionKind::Atomic,
                patterns: vec![CompositionPattern::Sequential],
                provides: vec!["draft".to_string()],
                requires: Vec::new(),
            },
            governing_spec: "005-capability-registry".to_string(),
            validator_version: "validator".to_string(),
        });
        assert!(outcome.is_ok());
        registry
    }

    fn public_capability_registry_fixture() -> CapabilityRegistry {
        let mut registry = CapabilityRegistry::new();
        let outcome = registry.register(CapabilityRegistration {
            scope: RegistryScope::Public,
            contract: capability_contract(),
            contract_path:
                "registry/public/content.comments.create-comment-draft/1.0.0/contract.json"
                    .to_string(),
            artifact: capability_artifact_record(),
            registered_at: "2026-03-30T00:00:00Z".to_string(),
            tags: vec!["comments".to_string()],
            composability: ComposabilityMetadata {
                kind: CompositionKind::Atomic,
                patterns: vec![CompositionPattern::Sequential],
                provides: vec!["draft".to_string()],
                requires: Vec::new(),
            },
            governing_spec: "005-capability-registry".to_string(),
            validator_version: "validator".to_string(),
        });
        assert!(outcome.is_ok());
        registry
    }

    fn event_registry_fixture() -> EventRegistry {
        let mut registry = EventRegistry::new();
        let outcome = registry.register(EventRegistration {
            scope: RegistryScope::Private,
            contract: event_contract(),
            contract_path: "registry/private/content.comments.draft-created/1.0.0/contract.json"
                .to_string(),
            registered_at: "2026-03-30T00:00:00Z".to_string(),
            governing_spec: "011-event-registry".to_string(),
            validator_version: "validator".to_string(),
        });
        assert!(outcome.is_ok());
        registry
    }

    fn workflow_registry_fixture(capabilities: &CapabilityRegistry) -> WorkflowRegistry {
        let mut registry = WorkflowRegistry::new();
        let outcome = registry.register(
            capabilities,
            WorkflowRegistration {
                scope: RegistryScope::Private,
                definition: workflow_definition(),
                workflow_path: "workflows/content.comments.publish-comment.json".to_string(),
                registered_at: "2026-03-30T00:00:00Z".to_string(),
                validator_version: "validator".to_string(),
            },
        );
        assert!(outcome.is_ok());
        registry
    }

    fn capability_contract() -> traverse_contracts::CapabilityContract {
        traverse_contracts::CapabilityContract {
            kind: "capability_contract".to_string(),
            schema_version: "1.0.0".to_string(),
            id: "content.comments.create-comment-draft".to_string(),
            namespace: "content.comments".to_string(),
            name: "create-comment-draft".to_string(),
            version: "1.0.0".to_string(),
            lifecycle: Lifecycle::Active,
            owner: Owner {
                team: "comments".to_string(),
                contact: "comments@example.com".to_string(),
            },
            summary: "Create a comment draft.".to_string(),
            description: "Create a deterministic comment draft.".to_string(),
            inputs: SchemaContainer {
                schema: json!({
                    "type": "object",
                    "required": ["comment_text", "resource_id"],
                    "properties": {
                        "comment_text": {"type": "string"},
                        "resource_id": {"type": "string"}
                    }
                }),
            },
            outputs: SchemaContainer {
                schema: json!({
                    "type": "object",
                    "required": ["draft_id"],
                    "properties": {
                        "draft_id": {"type": "string"}
                    }
                }),
            },
            preconditions: vec![Condition {
                id: "authenticated".to_string(),
                description: "Caller is authenticated.".to_string(),
            }],
            postconditions: vec![Condition {
                id: "draft_created".to_string(),
                description: "Draft id is produced.".to_string(),
            }],
            side_effects: vec![SideEffect {
                kind: SideEffectKind::MemoryOnly,
                description: "Creates draft state.".to_string(),
            }],
            emits: vec![EventReference {
                event_id: "content.comments.draft-created".to_string(),
                version: "1.0.0".to_string(),
            }],
            consumes: Vec::new(),
            permissions: vec![IdReference {
                id: "comments.create".to_string(),
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
            policies: vec![IdReference {
                id: "policy.comments.default".to_string(),
            }],
            dependencies: vec![DependencyReference {
                artifact_type: DependencyArtifactType::Event,
                id: "content.comments.draft-created".to_string(),
                version: "1.0.0".to_string(),
            }],
            provenance: Provenance {
                source: ProvenanceSource::Greenfield,
                author: "Enrico Piovesan".to_string(),
                created_at: "2026-03-30T00:00:00Z".to_string(),
                spec_ref: Some("006-runtime-request-execution".to_string()),
                adr_refs: Vec::new(),
                exception_refs: Vec::new(),
            },
            evidence: Vec::new(),
            service_type: traverse_contracts::ServiceType::Stateless,
            permitted_targets: vec![
                traverse_contracts::ExecutionTarget::Local,
                traverse_contracts::ExecutionTarget::Cloud,
                traverse_contracts::ExecutionTarget::Edge,
                traverse_contracts::ExecutionTarget::Device,
            ],
            event_trigger: None,
            connector_requirements: Vec::new(),
            state_schema: None,
        }
    }

    fn capability_artifact_record() -> CapabilityArtifactRecord {
        CapabilityArtifactRecord {
            artifact_ref: "artifact:content.comments.create-comment-draft:1.0.0".to_string(),
            implementation_kind: traverse_registry::ImplementationKind::Executable,
            source: SourceReference {
                kind: SourceKind::Git,
                location: "https://github.com/enricopiovesan/Traverse".to_string(),
            },
            binary: Some(BinaryReference {
                format: BinaryFormat::Wasm,
                location: "artifacts/create-comment-draft.wasm".to_string(),
                signature: None,
            }),
            workflow_ref: None,
            digests: ArtifactDigests {
                source_digest: "src-1".to_string(),
                binary_digest: Some("bin-1".to_string()),
            },
            provenance: RegistryProvenance {
                source: "test".to_string(),
                author: "Enrico Piovesan".to_string(),
                created_at: "2026-03-30T00:00:00Z".to_string(),
            },
        }
    }

    fn event_contract() -> EventContract {
        EventContract {
            kind: "event_contract".to_string(),
            schema_version: "1.0.0".to_string(),
            id: "content.comments.draft-created".to_string(),
            namespace: "content.comments".to_string(),
            name: "draft-created".to_string(),
            version: "1.0.0".to_string(),
            lifecycle: Lifecycle::Active,
            summary: "Draft was created.".to_string(),
            description: "Signals deterministic draft creation.".to_string(),
            classification: EventClassification {
                domain: "content".to_string(),
                bounded_context: "comments".to_string(),
                event_type: EventType::Domain,
                tags: vec!["comments".to_string()],
            },
            payload: EventPayload {
                schema: json!({
                    "type": "object",
                    "required": ["draft_id"],
                    "properties": {
                        "draft_id": {"type": "string"}
                    }
                }),
                compatibility: PayloadCompatibility::BackwardCompatible,
            },
            owner: Owner {
                team: "comments".to_string(),
                contact: "comments@example.com".to_string(),
            },
            publishers: vec![traverse_contracts::CapabilityReference {
                capability_id: "content.comments.create-comment-draft".to_string(),
                version: "1.0.0".to_string(),
            }],
            subscribers: vec![traverse_contracts::CapabilityReference {
                capability_id: "content.comments.publish-comment".to_string(),
                version: "1.0.0".to_string(),
            }],
            policies: vec![IdReference {
                id: "policy.comments.default".to_string(),
            }],
            tags: vec!["comments".to_string()],
            provenance: EventProvenance {
                source: EventProvenanceSource::Greenfield,
                author: "Enrico Piovesan".to_string(),
                created_at: "2026-03-30T00:00:00Z".to_string(),
            },
            evidence: vec![EventValidationEvidence {
                kind: "spec_alignment".to_string(),
                r#ref: "spec://003-event-contracts".to_string(),
            }],
        }
    }

    fn workflow_definition() -> WorkflowDefinition {
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
            summary: "Publish comment workflow.".to_string(),
            inputs: SchemaContainer {
                schema: json!({
                    "type": "object",
                    "required": ["comment_text", "resource_id"],
                    "properties": {
                        "comment_text": {"type": "string"},
                        "resource_id": {"type": "string"}
                    }
                }),
            },
            outputs: SchemaContainer {
                schema: json!({
                    "type": "object",
                    "required": ["draft_id"],
                    "properties": {
                        "draft_id": {"type": "string"}
                    }
                }),
            },
            nodes: vec![WorkflowNode {
                node_id: "create-draft".to_string(),
                capability_id: "content.comments.create-comment-draft".to_string(),
                capability_version: "1.0.0".to_string(),
                input: WorkflowNodeInput {
                    from_workflow_input: vec![
                        "comment_text".to_string(),
                        "resource_id".to_string(),
                    ],
                },
                output: WorkflowNodeOutput {
                    to_workflow_state: vec!["draft_id".to_string()],
                },
            }],
            edges: Vec::new(),
            start_node: "create-draft".to_string(),
            terminal_nodes: vec!["create-draft".to_string()],
            tags: vec!["comments".to_string()],
            governing_spec: "007-workflow-registry-traversal".to_string(),
        }
    }

    fn runtime_request() -> RuntimeRequest {
        RuntimeRequest {
            kind: "runtime_request".to_string(),
            schema_version: "1.0.0".to_string(),
            request_id: "req-mcp-1".to_string(),
            intent: RuntimeIntent {
                capability_id: Some("content.comments.create-comment-draft".to_string()),
                capability_version: Some("1.0.0".to_string()),
                version_range: None,
                intent_key: Some("content.comments.create-comment-draft".to_string()),
            },
            input: json!({
                "comment_text": "Hello",
                "resource_id": "res-1"
            }),
            lookup: RuntimeLookup {
                scope: RuntimeLookupScope::PreferPrivate,
                allow_ambiguity: false,
            },
            context: RuntimeContext {
                requested_target: traverse_runtime::PlacementTarget::Local,
                correlation_id: Some("corr-1".to_string()),
                caller: Some("mcp".to_string()),
                traceparent: None,
                tracestate: None,
                metadata: None,
                identity: None,
            },
            governing_spec: "006-runtime-request-execution".to_string(),
        }
    }

    #[derive(Debug)]
    struct EchoExecutor;

    impl LocalExecutor for EchoExecutor {
        fn execute(
            &self,
            _capability: &ResolvedCapability,
            _input: &Value,
        ) -> Result<Value, LocalExecutionFailure> {
            Ok(json!({"draft_id": "draft-001"}))
        }
    }

    // -----------------------------------------------------------------------
    // Spec 042-mcp-library-surface: free-function and McpToolRegistry tests
    // -----------------------------------------------------------------------

    #[test]
    fn discover_capabilities_returns_json_array_with_expected_structure() {
        let registry = capability_registry_fixture();
        let result = super::discover_capabilities(&registry);
        let arr = result.as_array().expect("must be a JSON array");
        assert_eq!(arr.len(), 1);
        let entry = &arr[0];
        assert_eq!(
            entry["id"].as_str(),
            Some("content.comments.create-comment-draft")
        );
        assert!(entry.get("version").is_some(), "entry must have version");
        assert!(entry.get("summary").is_some(), "entry must have summary");
        assert!(entry.get("tags").is_some(), "entry must have tags");
    }

    #[test]
    fn discover_capabilities_returns_empty_array_for_empty_registry() {
        let registry = CapabilityRegistry::new();
        let result = super::discover_capabilities(&registry);
        let arr = result.as_array().expect("must be a JSON array");
        assert!(arr.is_empty());
    }

    #[test]
    fn get_capability_returns_some_for_known_id() {
        let registry = capability_registry_fixture();
        let result = super::get_capability(&registry, "content.comments.create-comment-draft");
        assert!(result.is_some(), "expected Some for known capability id");
        let contract = result.expect("guarded by is_some");
        assert_eq!(
            contract["id"].as_str(),
            Some("content.comments.create-comment-draft")
        );
    }

    #[test]
    fn get_capability_returns_none_for_unknown_id() {
        let registry = capability_registry_fixture();
        let result = super::get_capability(&registry, "does.not.exist");
        assert!(result.is_none(), "expected None for unknown capability id");
    }

    #[test]
    fn execute_capability_returns_json_with_status_and_ids() {
        let registry = capability_registry_fixture();
        let workflow_registry = workflow_registry_fixture(&registry);
        let runtime =
            Runtime::new(registry, EchoExecutor).with_workflow_registry(workflow_registry);
        let result = super::execute_capability(&runtime, runtime_request());
        assert!(result.get("status").is_some(), "must include status");
        assert!(
            result.get("execution_id").is_some(),
            "must include execution_id"
        );
        assert!(
            result.get("request_id").is_some(),
            "must include request_id"
        );
        assert_eq!(result["request_id"].as_str(), Some("req-mcp-1"));
    }

    #[test]
    fn mcp_tool_registry_dispatch_routes_to_registered_handler() {
        let mut reg = McpToolRegistry::new();
        reg.register_tool("echo", |args| json!({ "echoed": args }));
        let input = json!({ "msg": "hello" });
        let output = reg.dispatch("echo", input.clone());
        assert!(output.is_some(), "expected Some for registered tool");
        let output = output.expect("guarded by is_some");
        assert_eq!(output["echoed"], input);
    }

    #[test]
    fn mcp_tool_registry_dispatch_returns_none_for_unknown_tool() {
        let reg = McpToolRegistry::new();
        let result = reg.dispatch("no_such_tool", json!({}));
        assert!(result.is_none(), "expected None for unregistered tool");
    }

    #[test]
    fn mcp_tool_registry_second_register_replaces_handler() {
        let mut reg = McpToolRegistry::new();
        reg.register_tool("tool", |_| json!("first"));
        reg.register_tool("tool", |_| json!("second"));
        let result = reg.dispatch("tool", json!({}));
        assert_eq!(result, Some(json!("second")));
    }
}
