//! Capability contract parsing and validation for Traverse.

use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeSet, HashSet};

pub mod usage_telemetry;
pub mod violations;
pub use usage_telemetry::{NoOpUsageTelemetrySink, UsageEvent, UsageEventKind, UsageTelemetrySink};
pub use violations::ViolationRecord;

const CAPABILITY_CONTRACT_KIND: &str = "capability_contract";
const EVENT_CONTRACT_KIND: &str = "event_contract";
const CONNECTOR_CONTRACT_KIND: &str = "connector_contract";
const SUPPORTED_SCHEMA_VERSION: &str = "1.0.0";
const GOVERNED_CONTENT_VERSION: &str = "0.1.0";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityContract {
    pub kind: String,
    pub schema_version: String,
    pub id: String,
    pub namespace: String,
    pub name: String,
    pub version: String,
    pub lifecycle: Lifecycle,
    pub owner: Owner,
    pub summary: String,
    pub description: String,
    pub inputs: SchemaContainer,
    pub outputs: SchemaContainer,
    pub preconditions: Vec<Condition>,
    pub postconditions: Vec<Condition>,
    pub side_effects: Vec<SideEffect>,
    pub emits: Vec<EventReference>,
    pub consumes: Vec<EventReference>,
    pub permissions: Vec<IdReference>,
    pub execution: Execution,
    pub policies: Vec<IdReference>,
    pub dependencies: Vec<DependencyReference>,
    pub provenance: Provenance,
    pub evidence: Vec<ValidationEvidence>,
    /// UMA service type — governs placement and event routing. Defaults to `Stateless`.
    #[serde(default)]
    pub service_type: ServiceType,
    /// Placement targets this capability may run on. Defaults to all targets.
    #[serde(default = "default_permitted_targets")]
    pub permitted_targets: Vec<ExecutionTarget>,
    /// Required for `Subscribable` capabilities: the event type that triggers this capability.
    #[serde(default)]
    pub event_trigger: Option<String>,
    /// External resource connectors required before this capability can be registered or executed.
    #[serde(default)]
    pub connector_requirements: Vec<ConnectorRequirement>,
    /// Typed JSON schema for capability state values written through the runtime `DataStore`.
    #[serde(default)]
    pub state_schema: Option<Value>,
    /// Executable surface examples (spec 102). Preserved through publish; not cleared by validate.
    #[serde(default)]
    pub use_cases: Vec<UseCase>,
    /// Immutable capability risk classification (spec 109 FR-005). Contracts published
    /// before this field existed default to the most conservative classification so
    /// they are never silently treated as automatic-eligible.
    #[serde(default = "default_risk_metadata")]
    pub risk: RiskMetadata,
}

/// Portable, immutable capability authority metadata across four independent
/// dimensions (ADR-0041). A capability's own contract is the only place these
/// values may be declared; an application manifest may narrow how a capability
/// is actually wired (for example connector selection) but can never override
/// or weaken these classifications.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RiskMetadata {
    pub effect_class: EffectClass,
    pub determinism_class: DeterminismClass,
    #[serde(default)]
    pub data_flow: DataFlowPolicy,
    pub reliability: ReliabilityMetadata,
}

/// What kind of effect invoking this capability has on the world.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectClass {
    /// Reads only; no observable effect on any state.
    PureRead,
    /// Writes to Traverse-owned state (`DataStore`, trace).
    StateWrite,
    /// Calls an external system that is not owned/reversible by Traverse.
    ExternalEffect,
    /// An external effect that cannot be undone or compensated.
    IrreversibleEffect,
}

/// Whether repeated invocation with the same inputs is guaranteed to agree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeterminismClass {
    Deterministic,
    ExternallyVariable,
    ModelDerived,
}

/// Field-level data classification and egress policy for this capability's
/// declared inputs/outputs (spec 109 FR-005, FR-011). Schema compatibility
/// alone never authorizes disclosure of a classified field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataFlowPolicy {
    #[serde(default)]
    pub accepted_data_classifications: Vec<FieldDataClassification>,
    #[serde(default)]
    pub produced_data_classifications: Vec<FieldDataClassification>,
    #[serde(default = "default_egress_policy")]
    pub egress_policy: EgressPolicy,
}

impl Default for DataFlowPolicy {
    fn default() -> Self {
        Self {
            accepted_data_classifications: Vec::new(),
            produced_data_classifications: Vec::new(),
            egress_policy: default_egress_policy(),
        }
    }
}

/// Declares the classification of one field, addressed by a JSON Pointer
/// (RFC 6901) into the capability's `inputs.schema` or `outputs.schema`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldDataClassification {
    pub field_path: String,
    pub classification: DataClassification,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataClassification {
    Public,
    Internal,
    Confidential,
    Restricted,
}

/// Which connectors classified data produced/accepted by this capability may
/// legally flow to. `Denied` means no external connector egress is permitted
/// regardless of what connectors the capability is otherwise wired to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EgressPolicy {
    Denied,
    AllowedConnectors(Vec<String>),
}

fn default_egress_policy() -> EgressPolicy {
    EgressPolicy::Denied
}

/// Reliability semantics a caller MUST honor when invoking this capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReliabilityMetadata {
    pub idempotency_required: bool,
    pub retryable: bool,
    pub compensation_available: bool,
}

/// Conservative migration default for contracts published before spec 109:
/// the most restrictive classification on every dimension, so a capability
/// never becomes silently automatic-eligible just because it predates risk
/// metadata.
#[must_use]
pub fn default_risk_metadata() -> RiskMetadata {
    RiskMetadata {
        effect_class: EffectClass::IrreversibleEffect,
        determinism_class: DeterminismClass::ModelDerived,
        data_flow: DataFlowPolicy::default(),
        reliability: ReliabilityMetadata {
            idempotency_required: true,
            retryable: false,
            compensation_available: false,
        },
    }
}

/// Spec 109 FR-006: whether a proposal using only this capability's declared
/// risk classes is eligible to run without an authorization token. Every
/// caller that gates automatic execution MUST consume this single function
/// rather than re-deriving the rule from individual fields.
#[must_use]
pub fn is_automatic_eligible(risk: &RiskMetadata) -> bool {
    risk.effect_class == EffectClass::PureRead
        && risk.determinism_class == DeterminismClass::Deterministic
        && risk.data_flow.egress_policy == EgressPolicy::Denied
        && !risk.reliability.idempotency_required
}

/// An application manifest's declared narrowing of a capability's egress
/// surface (spec 109 FR-005: "a manifest may only tighten these
/// requirements"). Every other risk dimension is an immutable fact about the
/// capability's own behavior and has no manifest-side override.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ManifestRiskPolicy {
    /// When present, the manifest restricts egress to this connector id
    /// subset. `Some(vec![])` tightens an `AllowedConnectors` capability down
    /// to no egress for this deployment.
    #[serde(default)]
    pub egress_allowed_connectors: Option<Vec<String>>,
}

/// Validates that a manifest's declared risk policy only narrows the
/// capability's immutable, contract-declared egress surface — it MUST NOT
/// permit a connector the contract does not already allow.
///
/// # Errors
///
/// Returns [`ValidationFailure`] when the manifest policy would widen egress
/// beyond what the capability's `RiskMetadata` allows.
pub fn validate_manifest_risk_policy(
    risk: &RiskMetadata,
    policy: &ManifestRiskPolicy,
) -> Result<(), ValidationFailure> {
    let mut errors = Vec::new();

    if let Some(declared) = &policy.egress_allowed_connectors {
        match &risk.data_flow.egress_policy {
            EgressPolicy::Denied => {
                if !declared.is_empty() {
                    errors.push(error(
                        ValidationErrorCode::RiskPolicyWeakened,
                        "$.risk_policy.egress_allowed_connectors",
                        "manifest cannot allow egress for a capability whose contract denies all egress",
                    ));
                }
            }
            EgressPolicy::AllowedConnectors(allowed) => {
                for connector_id in declared {
                    if !allowed.contains(connector_id) {
                        errors.push(error(
                            ValidationErrorCode::RiskPolicyWeakened,
                            "$.risk_policy.egress_allowed_connectors",
                            &format!(
                                "manifest allows connector '{connector_id}' the contract's \
                                 egress policy does not permit"
                            ),
                        ));
                    }
                }
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(ValidationFailure { errors })
    }
}

/// One authored use case that demonstrates a concrete input/output path for a capability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UseCase {
    pub scenario: String,
    pub input_example: Value,
    pub output_example: Value,
    pub happy: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persona_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectorContract {
    pub kind: String,
    pub schema_version: String,
    pub connector_id: String,
    pub version: String,
    pub capabilities_provided: Vec<String>,
    pub required_config_schema: Value,
    pub operation_envelopes: Vec<ConnectorOperationEnvelope>,
    #[serde(default = "default_connector_targets")]
    pub supported_placement_targets: Vec<ExecutionTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectorOperationEnvelope {
    pub operation_id: String,
    pub request_schema: Value,
    pub success_schema: Value,
    pub failure_classes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectorRequirement {
    pub connector_id: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectorInvocation {
    pub capability_id: String,
    pub connector_id: String,
    pub config: Value,
    pub input: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectorOutput {
    pub output: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectorError {
    pub code: String,
    pub message: String,
}

pub trait ConnectorPlugin: Send + Sync {
    fn connector_id(&self) -> &str;
    fn version(&self) -> &str;
    fn capabilities_provided(&self) -> &[String];
    /// Invoke the connector with runtime-injected config and input.
    ///
    /// # Errors
    ///
    /// Returns [`ConnectorError`] when the connector cannot satisfy the invocation.
    fn invoke(&self, invocation: ConnectorInvocation) -> Result<ConnectorOutput, ConnectorError>;
}

#[must_use]
#[allow(clippy::too_many_lines)]
pub fn reference_connector_contracts() -> Vec<ConnectorContract> {
    vec![
        reference_connector_contract(
            "traverse.http",
            vec!["traverse.http.outbound".to_string()],
            serde_json::json!({
                "type": "object",
                "required": ["base_url"],
                "properties": {
                    "base_url": {"type": "string"}
                },
                "additionalProperties": false
            }),
            vec![connector_operation_envelope(
                "request",
                serde_json::json!({
                    "type": "object",
                    "required": ["method", "resource_ref", "idempotency_key"],
                    "properties": {
                        "method": {"type": "string"},
                        "resource_ref": {"type": "string"},
                        "headers_ref": {"type": "string"},
                        "body_ref": {"type": "string"},
                        "idempotency_key": {"type": "string"}
                    },
                    "additionalProperties": false
                }),
                serde_json::json!({
                    "type": "object",
                    "required": ["status", "body_ref", "result_class"],
                    "properties": {
                        "status": {"type": "integer"},
                        "body_ref": {"type": "string"},
                        "result_class": {"type": "string"}
                    },
                    "additionalProperties": false
                }),
                vec!["configuration", "transport", "authorization", "timeout"],
            )],
        ),
        reference_connector_contract(
            "traverse.fs.read",
            vec!["traverse.fs.read".to_string()],
            serde_json::json!({
                "type": "object",
                "required": ["root"],
                "properties": {
                    "root": {"type": "string"}
                },
                "additionalProperties": false
            }),
            vec![connector_operation_envelope(
                "read",
                serde_json::json!({
                    "type": "object",
                    "required": ["resource_ref", "idempotency_key"],
                    "properties": {
                        "resource_ref": {"type": "string"},
                        "max_bytes": {"type": "integer"},
                        "idempotency_key": {"type": "string"}
                    },
                    "additionalProperties": false
                }),
                serde_json::json!({
                    "type": "object",
                    "required": ["content_ref", "content_digest", "size", "result_class"],
                    "properties": {
                        "content_ref": {"type": "string"},
                        "content_digest": {"type": "string"},
                        "size": {"type": "integer"},
                        "result_class": {"type": "string"}
                    },
                    "additionalProperties": false
                }),
                vec!["configuration", "not_found", "authorization", "too_large"],
            )],
        ),
        reference_connector_contract(
            "traverse.env",
            vec!["traverse.env.read".to_string()],
            serde_json::json!({
                "type": "object",
                "required": ["allowed_keys"],
                "properties": {
                    "allowed_keys": {
                        "type": "array",
                        "items": {"type": "string"}
                    }
                },
                "additionalProperties": false
            }),
            vec![connector_operation_envelope(
                "read",
                serde_json::json!({
                    "type": "object",
                    "required": ["key_ref", "idempotency_key"],
                    "properties": {
                        "key_ref": {"type": "string"},
                        "idempotency_key": {"type": "string"}
                    },
                    "additionalProperties": false
                }),
                serde_json::json!({
                    "type": "object",
                    "required": ["value_ref", "result_class"],
                    "properties": {
                        "value_ref": {"type": "string"},
                        "result_class": {"type": "string"}
                    },
                    "additionalProperties": false
                }),
                vec!["configuration", "not_found", "authorization"],
            )],
        ),
        reference_connector_contract(
            "traverse.object-store",
            vec!["traverse.object_store.put".to_string()],
            serde_json::json!({
                "type": "object",
                "required": ["authority_ref"],
                "properties": {
                    "authority_ref": {"type": "string"},
                    "retention_classes": {
                        "type": "array",
                        "items": {"type": "string"}
                    }
                },
                "additionalProperties": false
            }),
            vec![connector_operation_envelope(
                "put_immutable",
                serde_json::json!({
                    "type": "object",
                    "required": ["content_ref", "media_type", "idempotency_key"],
                    "properties": {
                        "content_ref": {"type": "string"},
                        "media_type": {"type": "string"},
                        "retention_class": {"type": "string"},
                        "idempotency_key": {"type": "string"}
                    },
                    "additionalProperties": false
                }),
                serde_json::json!({
                    "type": "object",
                    "required": ["asset_ref", "content_digest", "size", "result_class"],
                    "properties": {
                        "asset_ref": {"type": "string"},
                        "content_digest": {"type": "string"},
                        "size": {"type": "integer"},
                        "result_class": {"type": "string"}
                    },
                    "additionalProperties": false
                }),
                vec!["configuration", "authorization", "quota", "integrity"],
            )],
        ),
        reference_connector_contract(
            "traverse.state-store",
            vec!["traverse.state_store.append".to_string()],
            serde_json::json!({
                "type": "object",
                "required": ["authority_ref"],
                "properties": {
                    "authority_ref": {"type": "string"},
                    "record_type_namespace": {"type": "string"}
                },
                "additionalProperties": false
            }),
            vec![connector_operation_envelope(
                "append_transition",
                serde_json::json!({
                    "type": "object",
                    "required": ["record_refs", "transition", "idempotency_key"],
                    "properties": {
                        "record_refs": {
                            "type": "array",
                            "items": {"type": "string"}
                        },
                        "transition": {"type": "object"},
                        "expected_version": {"type": "integer"},
                        "idempotency_key": {"type": "string"}
                    },
                    "additionalProperties": false
                }),
                serde_json::json!({
                    "type": "object",
                    "required": ["result_ref", "version", "replay", "result_class"],
                    "properties": {
                        "result_ref": {"type": "string"},
                        "version": {"type": "integer"},
                        "replay": {"type": "boolean"},
                        "result_class": {"type": "string"}
                    },
                    "additionalProperties": false
                }),
                vec!["configuration", "authorization", "conflict", "integrity"],
            )],
        ),
        reference_connector_contract(
            "traverse.scheduler",
            vec!["traverse.scheduler.schedule".to_string()],
            serde_json::json!({
                "type": "object",
                "required": ["authority_ref"],
                "properties": {
                    "authority_ref": {"type": "string"},
                    "allowed_job_kinds": {
                        "type": "array",
                        "items": {"type": "string"}
                    }
                },
                "additionalProperties": false
            }),
            vec![connector_operation_envelope(
                "schedule_invocation",
                serde_json::json!({
                    "type": "object",
                    "required": ["job_kind", "calendar_policy_ref", "logical_deadline", "idempotency_key"],
                    "properties": {
                        "job_kind": {"type": "string"},
                        "calendar_policy_ref": {"type": "string"},
                        "logical_deadline": {"type": "string"},
                        "cancellation_ref": {"type": "string"},
                        "idempotency_key": {"type": "string"}
                    },
                    "additionalProperties": false
                }),
                serde_json::json!({
                    "type": "object",
                    "required": ["invocation_ref", "idempotency_key", "result_class"],
                    "properties": {
                        "invocation_ref": {"type": "string"},
                        "idempotency_key": {"type": "string"},
                        "result_class": {"type": "string"}
                    },
                    "additionalProperties": false
                }),
                vec!["configuration", "authorization", "deadline", "quota"],
            )],
        ),
    ]
}

fn reference_connector_contract(
    connector_id: &str,
    capabilities_provided: Vec<String>,
    required_config_schema: Value,
    operation_envelopes: Vec<ConnectorOperationEnvelope>,
) -> ConnectorContract {
    ConnectorContract {
        kind: CONNECTOR_CONTRACT_KIND.to_string(),
        schema_version: SUPPORTED_SCHEMA_VERSION.to_string(),
        connector_id: connector_id.to_string(),
        version: "1.0.0".to_string(),
        capabilities_provided,
        required_config_schema,
        operation_envelopes,
        supported_placement_targets: default_connector_targets(),
    }
}

fn connector_operation_envelope(
    operation_id: &str,
    request_schema: Value,
    success_schema: Value,
    failure_classes: Vec<&str>,
) -> ConnectorOperationEnvelope {
    ConnectorOperationEnvelope {
        operation_id: operation_id.to_string(),
        request_schema,
        success_schema,
        failure_classes: failure_classes
            .into_iter()
            .map(std::string::ToString::to_string)
            .collect(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventContract {
    pub kind: String,
    pub schema_version: String,
    pub id: String,
    pub namespace: String,
    pub name: String,
    pub version: String,
    pub lifecycle: Lifecycle,
    pub owner: Owner,
    pub summary: String,
    pub description: String,
    pub payload: EventPayload,
    pub classification: EventClassification,
    pub publishers: Vec<CapabilityReference>,
    pub subscribers: Vec<CapabilityReference>,
    pub policies: Vec<IdReference>,
    pub tags: Vec<String>,
    pub provenance: EventProvenance,
    pub evidence: Vec<EventValidationEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventPayload {
    pub schema: Value,
    pub compatibility: PayloadCompatibility,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PayloadCompatibility {
    BackwardCompatible,
    ForwardCompatible,
    Breaking,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventClassification {
    pub domain: String,
    pub bounded_context: String,
    pub event_type: EventType,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    Domain,
    Integration,
    System,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityReference {
    pub capability_id: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventProvenance {
    pub source: EventProvenanceSource,
    pub author: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EventProvenanceSource {
    Greenfield,
    Brownfield,
    AiGenerated,
    Extracted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventValidationEvidence {
    pub kind: String,
    pub r#ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lifecycle {
    Draft,
    Active,
    Deprecated,
    Retired,
    Archived,
}

impl Lifecycle {
    #[must_use]
    pub fn is_runtime_eligible(&self) -> bool {
        matches!(self, Self::Active | Self::Deprecated)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Owner {
    pub team: String,
    pub contact: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemaContainer {
    pub schema: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Condition {
    pub id: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SideEffect {
    pub kind: SideEffectKind,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SideEffectKind {
    None,
    MemoryOnly,
    EventEmission,
    ExternalCall,
    StateChange,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventReference {
    pub event_id: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdReference {
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Execution {
    pub binary_format: BinaryFormat,
    pub entrypoint: Entrypoint,
    pub preferred_targets: Vec<ExecutionTarget>,
    pub constraints: ExecutionConstraints,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BinaryFormat {
    Wasm,
}

/// UMA service type classification — governs placement routing and event routing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ServiceType {
    /// Runs anywhere; no persistent state required. Default for backward compatibility.
    #[default]
    Stateless,
    /// Activated by an incoming event; requires a non-empty `event_trigger`.
    Subscribable,
    /// Requires managed persistence; cannot be placed in Browser environments.
    Stateful,
}

fn default_permitted_targets() -> Vec<ExecutionTarget> {
    vec![
        ExecutionTarget::Local,
        ExecutionTarget::Browser,
        ExecutionTarget::Edge,
        ExecutionTarget::Cloud,
        ExecutionTarget::Worker,
        ExecutionTarget::Device,
    ]
}

fn default_connector_targets() -> Vec<ExecutionTarget> {
    vec![ExecutionTarget::Local, ExecutionTarget::Cloud]
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entrypoint {
    pub kind: EntrypointKind,
    pub command: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EntrypointKind {
    WasiCommand,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Ord, PartialOrd)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionTarget {
    Local,
    Browser,
    Edge,
    Cloud,
    Worker,
    Device,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionConstraints {
    pub host_api_access: HostApiAccess,
    pub network_access: NetworkAccess,
    pub filesystem_access: FilesystemAccess,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostApiAccess {
    None,
    ExceptionRequired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkAccess {
    Forbidden,
    Required,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilesystemAccess {
    None,
    SandboxOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DependencyReference {
    pub artifact_type: DependencyArtifactType,
    pub id: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DependencyArtifactType {
    Capability,
    Event,
    Policy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    pub source: ProvenanceSource,
    pub author: String,
    pub created_at: String,
    #[serde(default)]
    pub spec_ref: Option<String>,
    #[serde(default)]
    pub adr_refs: Vec<String>,
    #[serde(default)]
    pub exception_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProvenanceSource {
    Greenfield,
    BrownfieldExtracted,
    AiGenerated,
    AiAssisted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationEvidence {
    pub evidence_id: String,
    #[serde(rename = "type")]
    pub evidence_type: EvidenceType,
    pub status: EvidenceStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceType {
    SpecAlignment,
    ContractValidation,
    Compatibility,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceStatus {
    Passed,
    Failed,
    Superseded,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedContractRecord {
    pub id: String,
    pub version: String,
    pub governed_content_digest: String,
    pub lifecycle: Lifecycle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedEventRecord {
    pub id: String,
    pub version: String,
    pub governed_content_digest: String,
    pub lifecycle: Lifecycle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationContext<'a> {
    pub governing_spec: &'a str,
    pub validator_version: &'a str,
    pub existing_published: Option<&'a PublishedContractRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventValidationContext<'a> {
    pub governing_spec: &'a str,
    pub validator_version: &'a str,
    pub existing_published: Option<&'a PublishedEventRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationResult {
    pub normalized: CapabilityContract,
    pub evidence: ProducedValidationEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventValidationResult {
    pub normalized: EventContract,
    pub evidence: ProducedValidationEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProducedValidationEvidence {
    pub artifact_id: String,
    pub artifact_version: String,
    pub governing_spec: String,
    pub validator_version: String,
    pub status: EvidenceStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationFailure {
    pub errors: Vec<ValidationError>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    pub code: ValidationErrorCode,
    pub message: String,
    pub path: String,
    pub severity: ErrorSeverity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationErrorCode {
    MissingRequiredField,
    InvalidLiteral,
    InvalidFormat,
    InvalidSemver,
    InconsistentIdentity,
    DuplicateItem,
    InvalidCapabilityBoundary,
    InvalidEventBoundary,
    UnsupportedBinaryFormat,
    UnsupportedEntrypoint,
    PortabilityExceptionRequired,
    ImmutableVersionConflict,
    InvalidDependencyRef,
    /// `service_type: stateful` combined with `Browser` in `permitted_targets`.
    InvalidPlacementConstraint,
    /// `service_type: subscribable` without a non-empty `event_trigger`.
    MissingEventTrigger,
    InvalidConnectorContract,
    InvalidConnectorRequirement,
    /// A manifest-declared risk policy would widen egress beyond what the
    /// capability's immutable `RiskMetadata` allows.
    RiskPolicyWeakened,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorSeverity {
    Error,
}

/// Parses a capability contract from raw JSON text.
///
/// # Errors
///
/// Returns [`ValidationFailure`] when the JSON payload cannot be deserialized
/// into the capability contract model.
pub fn parse_contract(json: &str) -> Result<CapabilityContract, ValidationFailure> {
    serde_json::from_str::<CapabilityContract>(json).map_err(|error| ValidationFailure {
        errors: vec![ValidationError {
            code: ValidationErrorCode::InvalidFormat,
            message: error.to_string(),
            path: "$".to_string(),
            severity: ErrorSeverity::Error,
        }],
    })
}

/// Parses an event contract from raw JSON text.
///
/// # Errors
///
/// Returns [`ValidationFailure`] when the JSON payload cannot be deserialized
/// into the event contract model.
pub fn parse_event_contract(json: &str) -> Result<EventContract, ValidationFailure> {
    serde_json::from_str::<EventContract>(json).map_err(|error| ValidationFailure {
        errors: vec![ValidationError {
            code: ValidationErrorCode::InvalidFormat,
            message: error.to_string(),
            path: "$".to_string(),
            severity: ErrorSeverity::Error,
        }],
    })
}

/// Parses a connector contract from raw JSON text.
///
/// # Errors
///
/// Returns [`ValidationFailure`] when the JSON payload cannot be deserialized
/// into the connector contract model.
pub fn parse_connector_contract(json: &str) -> Result<ConnectorContract, ValidationFailure> {
    serde_json::from_str::<ConnectorContract>(json).map_err(|error| ValidationFailure {
        errors: vec![ValidationError {
            code: ValidationErrorCode::InvalidFormat,
            message: error.to_string(),
            path: "$".to_string(),
            severity: ErrorSeverity::Error,
        }],
    })
}

/// Validates a parsed capability contract against the governed `v0.1` rules.
///
/// # Errors
///
/// Returns [`ValidationFailure`] when structural or semantic validation fails.
pub fn validate_contract(
    mut contract: CapabilityContract,
    context: &ValidationContext<'_>,
) -> Result<ValidationResult, ValidationFailure> {
    let mut errors = Vec::new();

    validate_kind(&contract, &mut errors);
    validate_schema_version(&contract, &mut errors);
    validate_identity(&contract, &mut errors);
    validate_semver(&contract.version, "$.version", &mut errors);
    validate_owner(&contract.owner, &mut errors);
    validate_summary(&contract.summary, "$.summary", &mut errors);
    validate_description(&contract.description, "$.description", &mut errors);
    validate_schema_container(&contract.inputs, "$.inputs.schema", &mut errors);
    validate_schema_container(&contract.outputs, "$.outputs.schema", &mut errors);
    validate_conditions(&contract.preconditions, "$.preconditions", &mut errors);
    validate_conditions(&contract.postconditions, "$.postconditions", &mut errors);
    validate_side_effects(&contract.side_effects, &mut errors);
    validate_event_references(&contract.emits, "$.emits", &mut errors);
    validate_event_references(&contract.consumes, "$.consumes", &mut errors);
    validate_id_references(&contract.permissions, "$.permissions", &mut errors);
    validate_execution(&contract.execution, &contract.provenance, &mut errors);
    validate_id_references(&contract.policies, "$.policies", &mut errors);
    validate_dependencies(&contract.dependencies, &mut errors);
    validate_connector_requirements(&contract.connector_requirements, &mut errors);
    validate_provenance(&contract.provenance, &mut errors);
    validate_evidence(&contract.evidence, &mut errors);
    validate_boundary(&contract, &mut errors);
    validate_placement_constraints(&contract, &mut errors);
    validate_risk_metadata(&contract.risk, &mut errors);
    validate_published_record(&contract, context.existing_published, &mut errors);

    if !errors.is_empty() {
        return Err(ValidationFailure { errors });
    }

    contract.evidence.clear();

    Ok(ValidationResult {
        evidence: ProducedValidationEvidence {
            artifact_id: contract.id.clone(),
            artifact_version: contract.version.clone(),
            governing_spec: context.governing_spec.to_string(),
            validator_version: context.validator_version.to_string(),
            status: EvidenceStatus::Passed,
        },
        normalized: contract,
    })
}

/// Validates a parsed connector contract.
///
/// # Errors
///
/// Returns [`ValidationFailure`] when structural or semantic validation fails.
pub fn validate_connector_contract(
    contract: ConnectorContract,
) -> Result<ConnectorContract, ValidationFailure> {
    let mut errors = Vec::new();

    if contract.kind != CONNECTOR_CONTRACT_KIND {
        errors.push(error(
            ValidationErrorCode::InvalidLiteral,
            "$.kind",
            "kind must equal connector_contract",
        ));
    }
    if contract.schema_version != SUPPORTED_SCHEMA_VERSION {
        errors.push(error(
            ValidationErrorCode::InvalidLiteral,
            "$.schema_version",
            "schema_version must equal 1.0.0",
        ));
    }
    validate_non_empty(&contract.connector_id, "$.connector_id", &mut errors);
    validate_semver(&contract.version, "$.version", &mut errors);
    validate_unique_strings(
        &contract.capabilities_provided,
        "$.capabilities_provided",
        "capabilities_provided must be unique",
        &mut errors,
    );
    if contract.capabilities_provided.is_empty() {
        errors.push(error(
            ValidationErrorCode::MissingRequiredField,
            "$.capabilities_provided",
            "capabilities_provided must contain at least one capability id",
        ));
    }
    validate_schema_value(
        &contract.required_config_schema,
        "$.required_config_schema",
        &mut errors,
    );
    validate_connector_operation_envelopes(&contract.operation_envelopes, &mut errors);
    if contract.supported_placement_targets.is_empty() {
        errors.push(error(
            ValidationErrorCode::MissingRequiredField,
            "$.supported_placement_targets",
            "supported_placement_targets must contain at least one target",
        ));
    }
    let unique_targets: BTreeSet<_> = contract
        .supported_placement_targets
        .iter()
        .cloned()
        .collect();
    if unique_targets.len() != contract.supported_placement_targets.len() {
        errors.push(error(
            ValidationErrorCode::DuplicateItem,
            "$.supported_placement_targets",
            "supported_placement_targets must be unique",
        ));
    }

    if !errors.is_empty() {
        return Err(ValidationFailure { errors });
    }

    Ok(contract)
}

fn validate_connector_operation_envelopes(
    operation_envelopes: &[ConnectorOperationEnvelope],
    errors: &mut Vec<ValidationError>,
) {
    if operation_envelopes.is_empty() {
        errors.push(error(
            ValidationErrorCode::MissingRequiredField,
            "$.operation_envelopes",
            "operation_envelopes must contain at least one operation",
        ));
        return;
    }

    let mut seen = HashSet::new();
    for (index, envelope) in operation_envelopes.iter().enumerate() {
        let operation_id_path = format!("$.operation_envelopes[{index}].operation_id");
        validate_non_empty(&envelope.operation_id, &operation_id_path, errors);
        if !seen.insert(envelope.operation_id.clone()) {
            errors.push(error(
                ValidationErrorCode::DuplicateItem,
                &operation_id_path,
                "operation_id values must be unique",
            ));
        }

        validate_schema_value(
            &envelope.request_schema,
            &format!("$.operation_envelopes[{index}].request_schema"),
            errors,
        );
        validate_schema_value(
            &envelope.success_schema,
            &format!("$.operation_envelopes[{index}].success_schema"),
            errors,
        );

        let failure_classes_path = format!("$.operation_envelopes[{index}].failure_classes");
        if envelope.failure_classes.is_empty() {
            errors.push(error(
                ValidationErrorCode::MissingRequiredField,
                &failure_classes_path,
                "failure_classes must contain at least one stable failure class",
            ));
        }
        validate_unique_strings(
            &envelope.failure_classes,
            &failure_classes_path,
            "failure_classes must be unique",
            errors,
        );
        for (failure_index, failure_class) in envelope.failure_classes.iter().enumerate() {
            validate_non_empty(
                failure_class,
                &format!("$.operation_envelopes[{index}].failure_classes[{failure_index}]"),
                errors,
            );
        }
    }
}

/// Validates a parsed event contract against the governed `v0.1` rules.
///
/// # Errors
///
/// Returns [`ValidationFailure`] when structural or semantic validation fails.
pub fn validate_event_contract(
    mut contract: EventContract,
    context: &EventValidationContext<'_>,
) -> Result<EventValidationResult, ValidationFailure> {
    let mut errors = Vec::new();

    validate_event_kind(&contract, &mut errors);
    validate_event_schema_version(&contract, &mut errors);
    validate_event_identity(&contract, &mut errors);
    validate_semver(&contract.version, "$.version", &mut errors);
    validate_owner(&contract.owner, &mut errors);
    validate_summary(&contract.summary, "$.summary", &mut errors);
    validate_description(&contract.description, "$.description", &mut errors);
    validate_event_payload(&contract.payload, &mut errors);
    validate_event_classification(&contract.classification, &mut errors);
    validate_capability_references(&contract.publishers, "$.publishers", true, &mut errors);
    validate_capability_references(&contract.subscribers, "$.subscribers", false, &mut errors);
    validate_id_references(&contract.policies, "$.policies", &mut errors);
    validate_tags(&contract.tags, "$.tags", true, &mut errors);
    validate_event_provenance(&contract.provenance, &mut errors);
    validate_event_evidence(&contract.evidence, &mut errors);
    validate_event_boundary(&contract, &mut errors);
    validate_published_event_record(&contract, context.existing_published, &mut errors);

    if !errors.is_empty() {
        return Err(ValidationFailure { errors });
    }

    contract.evidence.clear();

    Ok(EventValidationResult {
        evidence: ProducedValidationEvidence {
            artifact_id: contract.id.clone(),
            artifact_version: contract.version.clone(),
            governing_spec: context.governing_spec.to_string(),
            validator_version: context.validator_version.to_string(),
            status: EvidenceStatus::Passed,
        },
        normalized: contract,
    })
}

#[must_use]
pub fn governed_content_digest(contract: &CapabilityContract) -> String {
    let mut clone = contract.clone();
    clone.evidence.clear();
    let json = format!("{clone:?}");
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in json.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0001_0000_01b3);
    }
    format!("{GOVERNED_CONTENT_VERSION}:{hash:016x}")
}

#[must_use]
pub fn governed_event_content_digest(contract: &EventContract) -> String {
    let mut clone = contract.clone();
    clone.evidence.clear();
    let json = format!("{clone:?}");
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in json.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0001_0000_01b3);
    }
    format!("{GOVERNED_CONTENT_VERSION}:{hash:016x}")
}

fn validate_kind(contract: &CapabilityContract, errors: &mut Vec<ValidationError>) {
    if contract.kind != CAPABILITY_CONTRACT_KIND {
        errors.push(error(
            ValidationErrorCode::InvalidLiteral,
            "$.kind",
            "kind must equal capability_contract",
        ));
    }
}

fn validate_event_kind(contract: &EventContract, errors: &mut Vec<ValidationError>) {
    if contract.kind != EVENT_CONTRACT_KIND {
        errors.push(error(
            ValidationErrorCode::InvalidLiteral,
            "$.kind",
            "kind must equal event_contract",
        ));
    }
}

fn validate_schema_version(contract: &CapabilityContract, errors: &mut Vec<ValidationError>) {
    if contract.schema_version != SUPPORTED_SCHEMA_VERSION {
        errors.push(error(
            ValidationErrorCode::InvalidLiteral,
            "$.schema_version",
            "schema_version must equal 1.0.0",
        ));
    }
}

fn validate_event_schema_version(contract: &EventContract, errors: &mut Vec<ValidationError>) {
    if contract.schema_version != SUPPORTED_SCHEMA_VERSION {
        errors.push(error(
            ValidationErrorCode::InvalidLiteral,
            "$.schema_version",
            "schema_version must equal 1.0.0",
        ));
    }
}

fn validate_identity(contract: &CapabilityContract, errors: &mut Vec<ValidationError>) {
    if !is_valid_namespace(&contract.namespace) {
        errors.push(error(
            ValidationErrorCode::InvalidFormat,
            "$.namespace",
            "namespace must be dot-separated lowercase kebab-case segments",
        ));
    }

    if !is_valid_name(&contract.name) {
        errors.push(error(
            ValidationErrorCode::InvalidFormat,
            "$.name",
            "name must be lowercase kebab-case",
        ));
    }

    let expected_id = format!("{}.{}", contract.namespace, contract.name);
    if contract.id != expected_id {
        errors.push(error(
            ValidationErrorCode::InconsistentIdentity,
            "$.id",
            "id must equal namespace.name",
        ));
    }
}

fn validate_event_identity(contract: &EventContract, errors: &mut Vec<ValidationError>) {
    if !is_valid_namespace(&contract.namespace) {
        errors.push(error(
            ValidationErrorCode::InvalidFormat,
            "$.namespace",
            "namespace must be dot-separated lowercase kebab-case segments",
        ));
    }

    if !is_valid_name(&contract.name) {
        errors.push(error(
            ValidationErrorCode::InvalidFormat,
            "$.name",
            "name must be lowercase kebab-case",
        ));
    }

    let expected_id = format!("{}.{}", contract.namespace, contract.name);
    if contract.id != expected_id {
        errors.push(error(
            ValidationErrorCode::InconsistentIdentity,
            "$.id",
            "id must equal namespace.name",
        ));
    }
}

fn validate_semver(value: &str, path: &str, errors: &mut Vec<ValidationError>) {
    if Version::parse(value).is_err() {
        errors.push(error(
            ValidationErrorCode::InvalidSemver,
            path,
            "version must match MAJOR.MINOR.PATCH",
        ));
    }
}

fn validate_owner(owner: &Owner, errors: &mut Vec<ValidationError>) {
    validate_non_empty(&owner.team, "$.owner.team", errors);
    validate_non_empty(&owner.contact, "$.owner.contact", errors);
}

fn validate_summary(summary: &str, path: &str, errors: &mut Vec<ValidationError>) {
    if summary.trim().len() < 10 || summary.len() > 200 {
        errors.push(error(
            ValidationErrorCode::InvalidFormat,
            path,
            "summary length must be between 10 and 200 characters",
        ));
    }
}

fn validate_description(description: &str, path: &str, errors: &mut Vec<ValidationError>) {
    if description.trim().len() < 20 {
        errors.push(error(
            ValidationErrorCode::InvalidFormat,
            path,
            "description must be at least 20 characters",
        ));
    }
}

fn validate_schema_container(
    container: &SchemaContainer,
    path: &str,
    errors: &mut Vec<ValidationError>,
) {
    validate_schema_value(&container.schema, path, errors);
}

fn validate_schema_value(schema: &Value, path: &str, errors: &mut Vec<ValidationError>) {
    if !schema.is_object() {
        errors.push(error(
            ValidationErrorCode::InvalidFormat,
            path,
            "schema must be a JSON object",
        ));
    }
}

fn validate_event_payload(payload: &EventPayload, errors: &mut Vec<ValidationError>) {
    if !payload.schema.is_object() {
        errors.push(error(
            ValidationErrorCode::InvalidFormat,
            "$.payload.schema",
            "schema must be a JSON object",
        ));
    }
}

fn validate_event_classification(
    classification: &EventClassification,
    errors: &mut Vec<ValidationError>,
) {
    validate_min_length(
        &classification.domain,
        "$.classification.domain",
        2,
        "domain must be at least 2 characters",
        errors,
    );
    validate_min_length(
        &classification.bounded_context,
        "$.classification.bounded_context",
        2,
        "bounded_context must be at least 2 characters",
        errors,
    );
    validate_tags(&classification.tags, "$.classification.tags", true, errors);
}

fn validate_capability_references(
    references: &[CapabilityReference],
    path: &str,
    require_one: bool,
    errors: &mut Vec<ValidationError>,
) {
    if require_one && references.is_empty() {
        errors.push(error(
            ValidationErrorCode::MissingRequiredField,
            path,
            "array must contain at least one item",
        ));
    }

    let mut seen = HashSet::new();
    for (index, item) in references.iter().enumerate() {
        let id_path = format!("{path}[{index}].capability_id");
        let version_path = format!("{path}[{index}].version");
        validate_non_empty(&item.capability_id, &id_path, errors);
        validate_semver(&item.version, &version_path, errors);
        if !seen.insert((item.capability_id.clone(), item.version.clone())) {
            errors.push(error(
                ValidationErrorCode::DuplicateItem,
                &id_path,
                "capability references must be unique by id and version",
            ));
        }
    }
}

fn validate_tags(
    tags: &[String],
    path: &str,
    require_one: bool,
    errors: &mut Vec<ValidationError>,
) {
    if require_one && tags.is_empty() {
        errors.push(error(
            ValidationErrorCode::MissingRequiredField,
            path,
            "array must contain at least one item",
        ));
    }
    for (index, tag) in tags.iter().enumerate() {
        validate_non_empty(tag, &format!("{path}[{index}]"), errors);
    }
    validate_unique_strings(tags, path, "values must be unique", errors);
}

fn validate_event_provenance(provenance: &EventProvenance, errors: &mut Vec<ValidationError>) {
    validate_non_empty(&provenance.author, "$.provenance.author", errors);
    validate_non_empty(&provenance.created_at, "$.provenance.created_at", errors);
}

fn validate_event_evidence(
    evidence: &[EventValidationEvidence],
    errors: &mut Vec<ValidationError>,
) {
    let mut seen = HashSet::new();
    for (index, item) in evidence.iter().enumerate() {
        let kind_path = format!("$.evidence[{index}].kind");
        let ref_path = format!("$.evidence[{index}].ref");
        validate_non_empty(&item.kind, &kind_path, errors);
        validate_non_empty(&item.r#ref, &ref_path, errors);
        if !seen.insert((item.kind.clone(), item.r#ref.clone())) {
            errors.push(error(
                ValidationErrorCode::DuplicateItem,
                &kind_path,
                "evidence entries must be unique by kind and ref",
            ));
        }
    }
}

fn validate_conditions(conditions: &[Condition], path: &str, errors: &mut Vec<ValidationError>) {
    let mut seen = HashSet::new();
    for (index, condition) in conditions.iter().enumerate() {
        let id_path = format!("{path}[{index}].id");
        let description_path = format!("{path}[{index}].description");
        validate_non_empty(&condition.id, &id_path, errors);
        validate_non_empty(&condition.description, &description_path, errors);
        if !seen.insert(condition.id.clone()) {
            errors.push(error(
                ValidationErrorCode::DuplicateItem,
                &id_path,
                "condition ids must be unique",
            ));
        }
    }
}

fn validate_side_effects(side_effects: &[SideEffect], errors: &mut Vec<ValidationError>) {
    if side_effects.is_empty() {
        errors.push(error(
            ValidationErrorCode::MissingRequiredField,
            "$.side_effects",
            "side_effects must contain at least one item",
        ));
    }

    for (index, side_effect) in side_effects.iter().enumerate() {
        validate_non_empty(
            &side_effect.description,
            &format!("$.side_effects[{index}].description"),
            errors,
        );
    }
}

fn validate_event_references(
    references: &[EventReference],
    path: &str,
    errors: &mut Vec<ValidationError>,
) {
    let mut seen = HashSet::new();
    for (index, item) in references.iter().enumerate() {
        let event_path = format!("{path}[{index}].event_id");
        let version_path = format!("{path}[{index}].version");
        validate_non_empty(&item.event_id, &event_path, errors);
        validate_semver(&item.version, &version_path, errors);
        if !seen.insert((item.event_id.clone(), item.version.clone())) {
            errors.push(error(
                ValidationErrorCode::DuplicateItem,
                &event_path,
                "event references must be unique by id and version",
            ));
        }
    }
}

fn validate_id_references(items: &[IdReference], path: &str, errors: &mut Vec<ValidationError>) {
    let mut seen = HashSet::new();
    for (index, item) in items.iter().enumerate() {
        let item_path = format!("{path}[{index}].id");
        validate_non_empty(&item.id, &item_path, errors);
        if !seen.insert(item.id.clone()) {
            errors.push(error(
                ValidationErrorCode::DuplicateItem,
                &item_path,
                "ids must be unique",
            ));
        }
    }
}

fn validate_execution(
    execution: &Execution,
    provenance: &Provenance,
    errors: &mut Vec<ValidationError>,
) {
    match execution.binary_format {
        BinaryFormat::Wasm => {}
    }

    match execution.entrypoint.kind {
        EntrypointKind::WasiCommand => {}
    }

    validate_non_empty(
        &execution.entrypoint.command,
        "$.execution.entrypoint.command",
        errors,
    );

    if execution.preferred_targets.is_empty() {
        errors.push(error(
            ValidationErrorCode::MissingRequiredField,
            "$.execution.preferred_targets",
            "preferred_targets must contain at least one item",
        ));
    }

    let unique_targets: BTreeSet<_> = execution.preferred_targets.iter().cloned().collect();
    if unique_targets.len() != execution.preferred_targets.len() {
        errors.push(error(
            ValidationErrorCode::DuplicateItem,
            "$.execution.preferred_targets",
            "preferred_targets must be unique",
        ));
    }

    if matches!(
        execution.constraints.host_api_access,
        HostApiAccess::ExceptionRequired
    ) && provenance.exception_refs.is_empty()
    {
        errors.push(error(
            ValidationErrorCode::PortabilityExceptionRequired,
            "$.execution.constraints.host_api_access",
            "host_api_access=exception_required requires provenance.exception_refs",
        ));
    }
}

fn validate_dependencies(dependencies: &[DependencyReference], errors: &mut Vec<ValidationError>) {
    let mut seen = HashSet::new();
    for (index, dependency) in dependencies.iter().enumerate() {
        let id_path = format!("$.dependencies[{index}].id");
        let version_path = format!("$.dependencies[{index}].version");
        validate_non_empty(&dependency.id, &id_path, errors);
        validate_semver(&dependency.version, &version_path, errors);
        if !seen.insert((
            dependency.artifact_type.clone(),
            dependency.id.clone(),
            dependency.version.clone(),
        )) {
            errors.push(error(
                ValidationErrorCode::DuplicateItem,
                &id_path,
                "dependencies must be unique by artifact_type, id, and version",
            ));
        }
    }
}

fn validate_connector_requirements(
    requirements: &[ConnectorRequirement],
    errors: &mut Vec<ValidationError>,
) {
    let mut seen = HashSet::new();
    for (index, requirement) in requirements.iter().enumerate() {
        let id_path = format!("$.connector_requirements[{index}].connector_id");
        let version_path = format!("$.connector_requirements[{index}].version");
        validate_non_empty(&requirement.connector_id, &id_path, errors);
        if VersionReq::parse(&requirement.version).is_err() {
            errors.push(error(
                ValidationErrorCode::InvalidConnectorRequirement,
                &version_path,
                "connector requirement version must be a valid semver range",
            ));
        }
        if !seen.insert((
            requirement.connector_id.clone(),
            requirement.version.clone(),
        )) {
            errors.push(error(
                ValidationErrorCode::DuplicateItem,
                &id_path,
                "connector_requirements must be unique by connector_id and version",
            ));
        }
    }
}

fn validate_provenance(provenance: &Provenance, errors: &mut Vec<ValidationError>) {
    validate_non_empty(&provenance.author, "$.provenance.author", errors);
    validate_non_empty(&provenance.created_at, "$.provenance.created_at", errors);

    if let Some(spec_ref) = &provenance.spec_ref {
        validate_non_empty(spec_ref, "$.provenance.spec_ref", errors);
    }

    validate_unique_strings(
        &provenance.adr_refs,
        "$.provenance.adr_refs",
        "adr_refs must be unique",
        errors,
    );
    validate_unique_strings(
        &provenance.exception_refs,
        "$.provenance.exception_refs",
        "exception_refs must be unique",
        errors,
    );
}

fn validate_evidence(evidence: &[ValidationEvidence], errors: &mut Vec<ValidationError>) {
    let mut seen = HashSet::new();
    for (index, item) in evidence.iter().enumerate() {
        let id_path = format!("$.evidence[{index}].evidence_id");
        validate_non_empty(&item.evidence_id, &id_path, errors);
        if !seen.insert(item.evidence_id.clone()) {
            errors.push(error(
                ValidationErrorCode::DuplicateItem,
                &id_path,
                "evidence_id values must be unique",
            ));
        }
    }
}

fn validate_boundary(contract: &CapabilityContract, errors: &mut Vec<ValidationError>) {
    let summary = contract.summary.to_ascii_lowercase();
    let description = contract.description.to_ascii_lowercase();
    let combined = format!("{summary} {description}");
    let banned_terms = [
        "utility function",
        "helper function",
        "crud wrapper",
        "transport handler",
        "database insert",
        "full application",
        "subsystem",
    ];

    if banned_terms.iter().any(|term| combined.contains(term)) {
        errors.push(error(
            ValidationErrorCode::InvalidCapabilityBoundary,
            "$.summary",
            "capability must represent one meaningful business action",
        ));
    }
}

fn validate_event_boundary(contract: &EventContract, errors: &mut Vec<ValidationError>) {
    let summary = contract.summary.to_ascii_lowercase();
    let description = contract.description.to_ascii_lowercase();
    let combined = format!("{summary} {description}");
    let banned_terms = [
        "kafka topic",
        "transport topic",
        "websocket channel",
        "queue binding",
        "broker partition",
        "payload wrapper",
    ];

    if banned_terms.iter().any(|term| combined.contains(term)) {
        errors.push(error(
            ValidationErrorCode::InvalidEventBoundary,
            "$.summary",
            "event must describe one governed business event boundary",
        ));
    }
}

fn validate_placement_constraints(
    contract: &CapabilityContract,
    errors: &mut Vec<ValidationError>,
) {
    if contract.service_type == ServiceType::Stateful
        && contract
            .permitted_targets
            .contains(&ExecutionTarget::Browser)
    {
        errors.push(ValidationError {
            code: ValidationErrorCode::InvalidPlacementConstraint,
            message: "Stateful capabilities cannot target Browser environments; browsers cannot \
                      provide managed persistence guarantees."
                .to_string(),
            path: "$.permitted_targets".to_string(),
            severity: ErrorSeverity::Error,
        });
    }
    if contract.service_type == ServiceType::Subscribable
        && match contract.event_trigger.as_deref() {
            None => true,
            Some(event_trigger) => event_trigger.is_empty(),
        }
    {
        errors.push(ValidationError {
            code: ValidationErrorCode::MissingEventTrigger,
            message: "Subscribable capabilities must declare a non-empty event_trigger field."
                .to_string(),
            path: "$.event_trigger".to_string(),
            severity: ErrorSeverity::Error,
        });
    }
}

fn validate_risk_metadata(risk: &RiskMetadata, errors: &mut Vec<ValidationError>) {
    validate_field_classifications(
        &risk.data_flow.accepted_data_classifications,
        "$.risk.data_flow.accepted_data_classifications",
        errors,
    );
    validate_field_classifications(
        &risk.data_flow.produced_data_classifications,
        "$.risk.data_flow.produced_data_classifications",
        errors,
    );
}

fn validate_field_classifications(
    classifications: &[FieldDataClassification],
    path: &str,
    errors: &mut Vec<ValidationError>,
) {
    let mut seen = HashSet::new();
    for (index, item) in classifications.iter().enumerate() {
        let field_path = format!("{path}[{index}].field_path");
        validate_non_empty(&item.field_path, &field_path, errors);
        if !seen.insert(item.field_path.clone()) {
            errors.push(error(
                ValidationErrorCode::DuplicateItem,
                &field_path,
                "field_path values must be unique",
            ));
        }
    }
}

fn validate_published_record(
    contract: &CapabilityContract,
    published: Option<&PublishedContractRecord>,
    errors: &mut Vec<ValidationError>,
) {
    let Some(published) = published else {
        return;
    };

    if published.id != contract.id || published.version != contract.version {
        return;
    }

    let digest = governed_content_digest(contract);
    if published.governed_content_digest != digest {
        errors.push(error(
            ValidationErrorCode::ImmutableVersionConflict,
            "$.version",
            "published contract versions are immutable",
        ));
    }
}

fn validate_published_event_record(
    contract: &EventContract,
    published: Option<&PublishedEventRecord>,
    errors: &mut Vec<ValidationError>,
) {
    let Some(published) = published else {
        return;
    };

    if published.id != contract.id || published.version != contract.version {
        return;
    }

    let digest = governed_event_content_digest(contract);
    if published.governed_content_digest != digest {
        errors.push(error(
            ValidationErrorCode::ImmutableVersionConflict,
            "$.version",
            "published contract versions are immutable",
        ));
    }
}

fn validate_non_empty(value: &str, path: &str, errors: &mut Vec<ValidationError>) {
    if value.trim().is_empty() {
        errors.push(error(
            ValidationErrorCode::MissingRequiredField,
            path,
            "value must be non-empty",
        ));
    }
}

fn validate_min_length(
    value: &str,
    path: &str,
    min_length: usize,
    message: &str,
    errors: &mut Vec<ValidationError>,
) {
    if value.trim().len() < min_length {
        errors.push(error(ValidationErrorCode::InvalidFormat, path, message));
    }
}

fn validate_unique_strings(
    values: &[String],
    path: &str,
    message: &str,
    errors: &mut Vec<ValidationError>,
) {
    let mut seen = HashSet::new();
    for value in values {
        if !seen.insert(value.clone()) {
            errors.push(error(ValidationErrorCode::DuplicateItem, path, message));
            break;
        }
    }
}

fn error(code: ValidationErrorCode, path: &str, message: &str) -> ValidationError {
    ValidationError {
        code,
        message: message.to_string(),
        path: path.to_string(),
        severity: ErrorSeverity::Error,
    }
}

fn is_valid_name(name: &str) -> bool {
    let mut parts = name.split('-');
    let first = parts.next().unwrap_or_default();
    is_valid_segment(first) && parts.all(is_valid_segment)
}

fn is_valid_namespace(namespace: &str) -> bool {
    let mut parts = namespace.split('.');
    let first = parts.next().unwrap_or_default();
    is_valid_name(first) && parts.all(is_valid_name)
}

fn is_valid_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment
            .chars()
            .all(|character| character.is_ascii_lowercase() || character.is_ascii_digit())
}
