//! MCP tool surfaces for capability discovery.
//!
//! Governed by spec 015-capability-discovery-mcp

use serde::{Deserialize, Serialize};
use traverse_contracts::{
    DeterminismClass, EffectClass, EgressPolicy, ExecutionTarget, ReliabilityMetadata, ServiceType,
    is_automatic_eligible,
};
use traverse_registry::{
    CapabilityArtifactRecord, CapabilityRegistry, DiscoveryQuery, LookupScope, WorkflowReference,
};

use crate::{McpError, McpErrorCode};

/// Optional filter for [`list_capabilities`].
#[derive(Debug, Clone, Default)]
pub struct CapabilityFilter {
    /// When set, only return capabilities with this service type.
    pub service_type: Option<ServiceType>,
    /// When non-empty, only return capabilities whose `permitted_targets`
    /// include all of the listed targets.
    pub permitted_targets: Vec<ExecutionTarget>,
}

/// Summary record returned by [`list_capabilities`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilitySummary {
    /// Capability identifier.
    pub id: String,
    /// Capability display name.
    pub name: String,
    /// Service type classification.
    pub service_type: ServiceType,
    /// Execution targets this capability may run on.
    pub permitted_targets: Vec<ExecutionTarget>,
    /// Short human-readable description.
    pub description: String,
    /// Whether the executable package is standalone or carries advisory workflow composition metadata.
    pub package_mode: String,
    /// Advisory known workflow compositions; never used as execution authority.
    pub advisory_compositions: Vec<String>,
    /// Host activation eligibility is unknown from registry discovery alone.
    pub activation_eligibility: String,
    /// Stable reason explaining why eligibility requires activation evidence.
    pub activation_eligibility_reason: String,
    /// Immutable effect classification (spec 109 FR-005).
    pub effect_class: EffectClass,
    /// Immutable determinism classification (spec 109 FR-005).
    pub determinism_class: DeterminismClass,
    /// Declared egress surface — connector ids only, never private connector
    /// configuration or secrets.
    pub egress_policy: EgressPolicy,
    /// Declared reliability semantics (spec 109 FR-005).
    pub reliability: ReliabilityMetadata,
    /// Spec 109 FR-006: whether a proposal using only this capability's
    /// declared risk classes may run without an authorization token.
    pub automatic_eligible: bool,
}

/// List all capabilities, optionally filtered by `service_type` or `permitted_targets`.
///
/// Uses `LookupScope::PreferPrivate` internally so private overrides are preferred.
#[must_use]
pub fn list_capabilities(
    registry: &CapabilityRegistry,
    filter: Option<&CapabilityFilter>,
) -> Vec<CapabilitySummary> {
    let entries = registry.discover(LookupScope::PreferPrivate, &DiscoveryQuery::default());

    entries
        .into_iter()
        .filter_map(|entry| {
            registry.find_exact(LookupScope::PreferPrivate, &entry.id, &entry.version)
        })
        .filter(|cap| {
            let Some(f) = filter else { return true };

            let service_type_ok = f
                .service_type
                .as_ref()
                .is_none_or(|st| &cap.contract.service_type == st);

            let targets_ok = f.permitted_targets.is_empty()
                || f.permitted_targets
                    .iter()
                    .all(|t| cap.contract.permitted_targets.contains(t));

            service_type_ok && targets_ok
        })
        .map(|cap| CapabilitySummary {
            id: cap.contract.id.clone(),
            name: cap.contract.name.clone(),
            service_type: cap.contract.service_type.clone(),
            permitted_targets: cap.contract.permitted_targets.clone(),
            description: cap.contract.description.clone(),
            package_mode: capability_package_mode(&cap.artifact).to_string(),
            advisory_compositions: advisory_compositions(cap.artifact.workflow_ref.as_ref()),
            activation_eligibility: "unknown".to_string(),
            activation_eligibility_reason: "requires_host_activation_resolution".to_string(),
            effect_class: cap.contract.risk.effect_class,
            determinism_class: cap.contract.risk.determinism_class,
            egress_policy: cap.contract.risk.data_flow.egress_policy.clone(),
            reliability: cap.contract.risk.reliability,
            automatic_eligible: is_automatic_eligible(&cap.contract.risk),
        })
        .collect()
}

fn capability_package_mode(artifact: &CapabilityArtifactRecord) -> &'static str {
    if artifact.workflow_ref.is_some() {
        "workflow_composed"
    } else {
        "standalone"
    }
}

fn advisory_compositions(workflow_ref: Option<&WorkflowReference>) -> Vec<String> {
    workflow_ref
        .map(|reference| {
            vec![format!(
                "{}@{}",
                reference.workflow_id, reference.workflow_version
            )]
        })
        .unwrap_or_default()
}

/// Return the full contract JSON for a capability identified by `capability_id`.
///
/// Finds the latest registered version for the given id. Uses
/// `LookupScope::PreferPrivate` so private overrides are preferred.
///
/// # Errors
///
/// Returns [`McpError`] with code `NotFound` when no matching capability exists in the registry.
pub fn get_capability(
    registry: &CapabilityRegistry,
    capability_id: &str,
) -> Result<serde_json::Value, McpError> {
    let entries = registry.discover(LookupScope::PreferPrivate, &DiscoveryQuery::default());

    let entry = entries
        .into_iter()
        .find(|e| e.id == capability_id)
        .ok_or_else(|| McpError {
            code: McpErrorCode::NotFound,
            message: format!("capability '{capability_id}' not found"),
        })?;

    let resolved = registry
        .find_exact(LookupScope::PreferPrivate, &entry.id, &entry.version)
        .ok_or_else(|| McpError {
            code: McpErrorCode::NotFound,
            message: format!("capability '{capability_id}' not found"),
        })?;

    serde_json::to_value(&resolved.contract).map_err(|e| McpError {
        code: McpErrorCode::InvalidRequest,
        message: e.to_string(),
    })
}
