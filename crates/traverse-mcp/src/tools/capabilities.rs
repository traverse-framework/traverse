//! MCP tool surfaces for capability discovery.
//!
//! Governed by spec 015-capability-discovery-mcp

use semver::Version;
use serde::{Deserialize, Serialize};
use traverse_contracts::{
    DeterminismClass, EffectClass, EgressPolicy, ExecutionTarget, ReliabilityMetadata, ServiceType,
    is_automatic_eligible,
};
use traverse_registry::{
    CapabilityArtifactRecord, CapabilityRegistry, DiscoveryQuery, LookupScope, WorkflowReference,
};

use crate::{McpError, McpErrorCode};
use traverse_embedder::{HostRegistryCache, PublicCapabilityMetadata, read_public_metadata};

/// Search result from the host-supplied verified public registry cache.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicCapabilitySearchResult {
    pub records: Vec<PublicCapabilityMetadata>,
    pub stale: bool,
    pub source_release: String,
    pub index_digest: String,
}

/// Search public capability descriptions and declared scenario text offline.
///
/// # Errors
///
/// Returns stable errors for invalid queries and unavailable verified cache state.
pub fn search_capabilities(
    cache: &HostRegistryCache,
    query: &str,
) -> Result<PublicCapabilitySearchResult, McpError> {
    let tokens = query
        .split_whitespace()
        .map(str::to_lowercase)
        .collect::<Vec<_>>();
    if tokens.is_empty() {
        return Err(McpError {
            code: McpErrorCode::InvalidRequest,
            message: "invalid_query".to_string(),
        });
    }
    let generation = read_public_metadata(cache).map_err(|error| McpError {
        code: McpErrorCode::InvalidRequest,
        message: error.code.as_str().to_string(),
    })?;
    let mut records = generation.records;
    records.retain(|record| {
        let text = format!("{} {}", record.description, record.scenarios.join(" ")).to_lowercase();
        tokens.iter().all(|token| text.contains(token))
    });
    records.sort_by(|left, right| {
        left.id.cmp(&right.id).then_with(|| {
            match (
                Version::parse(&left.version),
                Version::parse(&right.version),
            ) {
                (Ok(left), Ok(right)) => right.cmp(&left),
                _ => right.version.cmp(&left.version),
            }
        })
    });
    Ok(PublicCapabilitySearchResult {
        records,
        stale: generation.stale,
        source_release: generation.source_release,
        index_digest: generation.index_digest,
    })
}

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

#[cfg(test)]
mod search_tests {
    #![allow(clippy::expect_used)]

    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use traverse_embedder::publish_public_metadata;
    use traverse_registry::{
        PublicRegistryCapabilityRecord, PublicUseCaseSummary, SyncedPublicRegistryState,
    };

    static CACHE_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn cache() -> HostRegistryCache {
        let sequence = CACHE_COUNTER.fetch_add(1, Ordering::Relaxed);
        HostRegistryCache::new(std::env::temp_dir().join(format!(
            "traverse-mcp-search-{}-{sequence}",
            std::process::id()
        )))
    }

    fn record(id: &str, version: &str, description: &str) -> PublicRegistryCapabilityRecord {
        PublicRegistryCapabilityRecord {
            namespace: "demo".to_string(),
            id: id.to_string(),
            version: version.to_string(),
            digest: format!("sha256:{}", "a".repeat(64)),
            artifact_url: format!("https://example.test/{id}.wasm"),
            contract_digest: format!("sha256:{}", "b".repeat(64)),
            contract_url: format!("https://example.test/{id}.json"),
            deprecated: false,
            summary: format!("{id} summary"),
            description: description.to_string(),
            use_cases: vec![PublicUseCaseSummary {
                scenario: "Find a public capability".to_string(),
            }],
            service_type: "stateless".to_string(),
            permitted_targets: vec!["wasm".to_string()],
            lifecycle: "active".to_string(),
            provenance: None,
            commercial_use: "unknown".to_string(),
            redistribution: "unknown".to_string(),
            verification_status: "unknown".to_string(),
            license_expression: None,
        }
    }

    fn publish(cache: &HostRegistryCache) {
        let snapshot = SyncedPublicRegistryState {
            schema_version: "1".to_string(),
            workspace_id: "workspace".to_string(),
            state_scope: "public".to_string(),
            source_repo: "traverse-framework/registry".to_string(),
            release_tag: "index-v200".to_string(),
            index_version: 200,
            generated_at: "2026-08-25T00:00:00Z".to_string(),
            source_commit: None,
            synced_at: "2026-08-25T00:00:00Z".to_string(),
            record_count: 3,
            validation_status: "valid".to_string(),
            governing_spec: "114-capability-search".to_string(),
            capabilities: vec![
                record("search", "2.0.0", "Search public catalog entries"),
                record("search", "10.0.0", "Search public catalog entries"),
                record("alpha", "1.0.0", "Search public catalog entries"),
            ],
            events: Vec::new(),
        };
        publish_public_metadata(cache, &snapshot, true).expect("publish metadata");
    }

    #[test]
    fn search_requires_query_and_fails_closed_without_a_generation() {
        let cache = cache();
        let error = search_capabilities(&cache, "   ").expect_err("empty query");
        assert_eq!(error.message, "invalid_query");
        let error = search_capabilities(&cache, "catalog").expect_err("missing generation");
        assert_eq!(error.message, "registry_sync_missing");
    }

    #[test]
    fn search_matches_public_text_sorts_semver_and_preserves_provenance() {
        let cache = cache();
        publish(&cache);
        let result = search_capabilities(&cache, "PUBLIC catalog").expect("search");
        assert!(result.stale);
        assert_eq!(result.source_release, "index-v200");
        assert!(result.index_digest.starts_with("sha256:"));
        assert_eq!(result.records.len(), 3);
        assert_eq!(result.records[0].id, "alpha");
        assert_eq!(result.records[1].version, "10.0.0");
        assert_eq!(result.records[2].version, "2.0.0");
        assert_eq!(result.records[0].permitted_targets, ["wasm"]);
    }

    #[test]
    fn search_keeps_generation_provenance_when_nothing_matches() {
        let cache = cache();
        publish(&cache);
        let result = search_capabilities(&cache, "not-present").expect("search");
        assert!(result.records.is_empty());
        assert_eq!(result.source_release, "index-v200");
    }
}
