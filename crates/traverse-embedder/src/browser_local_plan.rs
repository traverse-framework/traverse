//! Browser-local deterministic workflow composition (spec
//! `1277-browser-local-workflow-composition`, issue #1269).
//!
//! A backend-less browser page holds an already synced, digest-verified public
//! registry snapshot (`SyncedPublicRegistryState`) and a set of already
//! prepared, digest-verified `registry_ref` dependencies
//! ([`VerifiedRegistryDependency`], from [`crate::resolve_registry_dependency_offline`]).
//! [`browser_local_plan`] turns a **structured** target plus starting facts
//! into zero or more untrusted, versioned [`BrowserWorkflowProposal`]s that a
//! caller can hand to the local governed runtime (Spec 108/109) for
//! validation, authorization and execution.
//!
//! This module performs **no I/O and no network access** — every input is
//! borrowed and every output is derived purely from it (spec 1277 FR-003,
//! FR-008). Candidate derivation is the exact structural, schema-and-event
//! chaining rule of spec `113-declarative-workflow-planning` FR-002; the
//! helpers below are mirrored from
//! `crates/traverse-mcp/src/tools/workflow_plan.rs` and MUST stay in step with
//! it. The planner never infers a plan from capability names, namespaces,
//! natural-language goals, model output, recency, or a hidden score
//! (FR-003), and never clears a field mapping (FR-004).

use serde_json::{Value, json};
use std::collections::BTreeSet;

use traverse_contracts::{
    CapabilityContract, ManifestReference, MappingSource, ProposalEdge, ProposalMapping,
    ProposalNode, WorkflowProposal,
};
use traverse_registry::SyncedPublicRegistryState;

use crate::VerifiedRegistryDependency;
use crate::registry_cache::index_snapshot_digest;

/// Contract envelope schema version this planner understands (spec 1277
/// FR-001 "supported contract-schema version"). Matches
/// `traverse_contracts` `SUPPORTED_SCHEMA_VERSION`.
pub const SUPPORTED_CONTRACT_SCHEMA_VERSION: &str = "1.0.0";
/// Versioned identity of a [`BrowserWorkflowProposal`] envelope.
pub const BROWSER_WORKFLOW_PROPOSAL_SCHEMA_VERSION: &str = "1.0.0";

/// At most 5 complete candidate proposals per call (spec 113 FR-004, retained
/// by spec 1277 FR-002).
const PLAN_MAX_CANDIDATES: usize = 5;
/// At most 8 nodes deep per candidate (spec 113 FR-004, retained by spec 1277
/// FR-002).
const PLAN_MAX_NODES: usize = 8;
/// Defensive cap on total recursive search calls (spec 113 FR-004: never
/// search unbounded).
const PLAN_MAX_SEARCH_CALLS: usize = 4_000;
/// Bound on serialized starting facts (spec 1277 FR-007).
const MAX_STARTING_FACTS_BYTES: usize = 64 * 1024;
/// Bound on the number of prepared dependencies a single call may consider
/// (spec 1277 FR-007).
const MAX_VERIFIED_DEPENDENCIES: usize = 128;

/// What a plan must end at (spec 113 FR-001): a declared event type or an
/// exact capability id/version. Never a natural-language goal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrowserPlanTarget {
    /// The plan must end at this exact capability.
    Capability {
        capability_id: String,
        capability_version: String,
    },
    /// The plan must end at a capability that declares it emits this event.
    EmitsEvent { event_type: String },
}

/// Verified identity of the registry snapshot a browser plan is derived from
/// (spec 1277 FR-001). Every field is non-secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotIdentity {
    /// `sha256:<hex>` digest of the canonical `SyncedPublicRegistryState`.
    pub registry_snapshot_digest: String,
    /// Release tag the snapshot was synced from.
    pub source_release: String,
    /// Contract envelope schema version the caller prepared against.
    pub contract_schema_version: String,
}

/// A single browser planning result: a versioned envelope binding the source
/// snapshot identity to a Spec-109-compatible [`WorkflowProposal`] whose
/// field mappings are all unconfirmed (spec 1277 FR-004).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct BrowserWorkflowProposal {
    pub kind: &'static str,
    pub schema_version: &'static str,
    pub snapshot_digest: String,
    pub source_release: String,
    pub proposal: WorkflowProposal,
    /// Always `true`: the browser planner never clears field mappings
    /// (spec 1277 FR-004).
    pub mapping_unconfirmed: bool,
}

/// Outcome of [`browser_local_plan`].
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct BrowserPlanResponse {
    pub proposals: Vec<BrowserWorkflowProposal>,
    /// `true` when the search found more than [`PLAN_MAX_CANDIDATES`] valid
    /// plans or hit the node-depth / search-work bound before fully exploring
    /// a branch with a structurally valid next step (spec 113 FR-004). Never a
    /// silent partial result.
    pub plan_search_truncated: bool,
}

/// Stable, secret-free failure classes (spec 1277 FR-001, FR-007).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserPlanErrorCode {
    /// `contract_schema_version` is not [`SUPPORTED_CONTRACT_SCHEMA_VERSION`].
    UnsupportedContractSchemaVersion,
    /// `registry_snapshot_digest` is malformed or does not match the snapshot.
    SnapshotDigestMismatch,
    /// The snapshot has no capability records.
    SnapshotEmpty,
    /// `source_release` does not match the snapshot's release tag.
    SnapshotEvidenceStale,
    /// A prepared dependency's bytes are not a valid capability contract.
    VerifiedDependencyContractInvalid,
    /// A prepared dependency has no matching active record in the snapshot.
    VerifiedDependencyNotInSnapshot,
    /// A prepared dependency's digests disagree with the snapshot record.
    VerifiedDependencyDigestMismatch,
    /// A prepared dependency's prepare evidence points at a different snapshot.
    VerifiedDependencyEvidenceMismatch,
    /// Starting facts exceed the size bound.
    StartingFactsTooLarge,
    /// The prepared dependency set exceeds the count bound.
    VerifiedDependencySetTooLarge,
}

impl BrowserPlanErrorCode {
    /// Stable `snake_case` identifier.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            BrowserPlanErrorCode::UnsupportedContractSchemaVersion => {
                "browser_plan_unsupported_contract_schema_version"
            }
            BrowserPlanErrorCode::SnapshotDigestMismatch => "browser_plan_snapshot_digest_mismatch",
            BrowserPlanErrorCode::SnapshotEmpty => "browser_plan_snapshot_empty",
            BrowserPlanErrorCode::SnapshotEvidenceStale => "browser_plan_snapshot_evidence_stale",
            BrowserPlanErrorCode::VerifiedDependencyContractInvalid => {
                "browser_plan_verified_dependency_contract_invalid"
            }
            BrowserPlanErrorCode::VerifiedDependencyNotInSnapshot => {
                "browser_plan_verified_dependency_not_in_snapshot"
            }
            BrowserPlanErrorCode::VerifiedDependencyDigestMismatch => {
                "browser_plan_verified_dependency_digest_mismatch"
            }
            BrowserPlanErrorCode::VerifiedDependencyEvidenceMismatch => {
                "browser_plan_verified_dependency_evidence_mismatch"
            }
            BrowserPlanErrorCode::StartingFactsTooLarge => "browser_plan_starting_facts_too_large",
            BrowserPlanErrorCode::VerifiedDependencySetTooLarge => {
                "browser_plan_verified_dependency_set_too_large"
            }
        }
    }
}

/// A stable, redacted browser-planning error. `detail` may name a declared
/// capability identity or a digest; it never carries paths, raw values, or
/// bytes (spec 1277 FR-007).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserPlanError {
    pub code: BrowserPlanErrorCode,
    pub detail: String,
}

impl BrowserPlanError {
    fn new(code: BrowserPlanErrorCode, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }

    /// Secret-free JSON projection.
    #[must_use]
    pub fn as_value(&self) -> Value {
        json!({ "code": self.code.as_str(), "detail": self.detail })
    }
}

/// A prepared dependency resolved to the fields the structural planner needs.
struct DeclaredCapability {
    capability_id: String,
    capability_version: String,
    artifact_digest: String,
    contract: CapabilityContract,
}

/// Spec 1277 FR-007: reject an oversized dependency set or starting facts
/// before any work.
fn check_input_bounds(
    verified_dependencies: &[VerifiedRegistryDependency],
    starting_facts: &Value,
) -> Result<(), BrowserPlanError> {
    if verified_dependencies.len() > MAX_VERIFIED_DEPENDENCIES {
        return Err(BrowserPlanError::new(
            BrowserPlanErrorCode::VerifiedDependencySetTooLarge,
            format!(
                "{} prepared dependencies exceeds the {MAX_VERIFIED_DEPENDENCIES} bound",
                verified_dependencies.len()
            ),
        ));
    }
    if serialized_len(starting_facts) > MAX_STARTING_FACTS_BYTES {
        return Err(BrowserPlanError::new(
            BrowserPlanErrorCode::StartingFactsTooLarge,
            format!("starting facts exceed the {MAX_STARTING_FACTS_BYTES}-byte bound"),
        ));
    }
    Ok(())
}

/// Spec 1277 FR-001: the supplied snapshot identity must exactly describe
/// `snapshot`, or planning fails closed with a stable secret-free error.
fn check_snapshot_identity(
    snapshot_identity: &SnapshotIdentity,
    snapshot: &SyncedPublicRegistryState,
) -> Result<(), BrowserPlanError> {
    if snapshot_identity.contract_schema_version != SUPPORTED_CONTRACT_SCHEMA_VERSION {
        return Err(BrowserPlanError::new(
            BrowserPlanErrorCode::UnsupportedContractSchemaVersion,
            format!(
                "contract schema version {} is not supported (expected {SUPPORTED_CONTRACT_SCHEMA_VERSION})",
                snapshot_identity.contract_schema_version
            ),
        ));
    }
    if snapshot.capabilities.is_empty() {
        return Err(BrowserPlanError::new(
            BrowserPlanErrorCode::SnapshotEmpty,
            "the synced registry snapshot contains no capability records",
        ));
    }
    if snapshot_identity.registry_snapshot_digest != index_snapshot_digest(snapshot) {
        return Err(BrowserPlanError::new(
            BrowserPlanErrorCode::SnapshotDigestMismatch,
            "supplied registry_snapshot_digest does not match the snapshot",
        ));
    }
    if snapshot_identity.source_release != snapshot.release_tag {
        return Err(BrowserPlanError::new(
            BrowserPlanErrorCode::SnapshotEvidenceStale,
            "supplied source_release does not match the snapshot release tag",
        ));
    }
    Ok(())
}

/// Spec 1277 FR-002/FR-006: resolve each prepared dependency against the exact
/// snapshot record, failing closed on any drift, and return the structural
/// planner's declared-capability set.
fn bind_verified_dependencies(
    snapshot_identity: &SnapshotIdentity,
    snapshot: &SyncedPublicRegistryState,
    verified_dependencies: &[VerifiedRegistryDependency],
) -> Result<Vec<DeclaredCapability>, BrowserPlanError> {
    let mut declared: Vec<DeclaredCapability> = Vec::with_capacity(verified_dependencies.len());
    for dependency in verified_dependencies {
        let contract: CapabilityContract = serde_json::from_slice(&dependency.contract_bytes)
            .map_err(|_| {
                BrowserPlanError::new(
                    BrowserPlanErrorCode::VerifiedDependencyContractInvalid,
                    format!(
                        "prepared dependency {}:{} contract bytes are not a capability contract",
                        dependency.evidence.namespace, dependency.evidence.id
                    ),
                )
            })?;
        if contract.schema_version != SUPPORTED_CONTRACT_SCHEMA_VERSION {
            return Err(BrowserPlanError::new(
                BrowserPlanErrorCode::UnsupportedContractSchemaVersion,
                format!(
                    "prepared dependency {}:{} contract schema version {} is not supported",
                    dependency.evidence.namespace, dependency.evidence.id, contract.schema_version
                ),
            ));
        }
        if dependency.evidence.index_digest != snapshot_identity.registry_snapshot_digest {
            return Err(BrowserPlanError::new(
                BrowserPlanErrorCode::VerifiedDependencyEvidenceMismatch,
                format!(
                    "prepared dependency {}:{} was prepared against a different snapshot",
                    dependency.evidence.namespace, dependency.evidence.id
                ),
            ));
        }
        let record = snapshot
            .capabilities
            .iter()
            .find(|record| {
                record.namespace == dependency.evidence.namespace
                    && record.id == dependency.evidence.id
                    && record.version == dependency.evidence.selected_version
            })
            .ok_or_else(|| {
                BrowserPlanError::new(
                    BrowserPlanErrorCode::VerifiedDependencyNotInSnapshot,
                    format!(
                        "prepared dependency {}:{}@{} has no record in the snapshot",
                        dependency.evidence.namespace,
                        dependency.evidence.id,
                        dependency.evidence.selected_version
                    ),
                )
            })?;
        if record.deprecated || record.lifecycle != "active" {
            return Err(BrowserPlanError::new(
                BrowserPlanErrorCode::VerifiedDependencyNotInSnapshot,
                format!(
                    "prepared dependency {}:{}@{} is not active in the snapshot",
                    record.namespace, record.id, record.version
                ),
            ));
        }
        if record.digest != dependency.evidence.artifact_digest
            || record.digest != dependency.wasm_digest
        {
            return Err(BrowserPlanError::new(
                BrowserPlanErrorCode::VerifiedDependencyDigestMismatch,
                format!(
                    "prepared dependency {}:{}@{} artifact digest disagrees with the snapshot record",
                    record.namespace, record.id, record.version
                ),
            ));
        }
        declared.push(DeclaredCapability {
            capability_id: record.id.clone(),
            capability_version: record.version.clone(),
            artifact_digest: record.digest.clone(),
            contract,
        });
    }
    Ok(declared)
}

/// Derives bounded candidate workflow proposals for `target` from a verified
/// snapshot and its prepared dependencies. Deterministic and read-only over
/// the exact inputs (spec 1277 FR-002); no network request is made (FR-008).
///
/// # Errors
///
/// Returns a [`BrowserPlanError`] when snapshot identity evidence is missing,
/// altered, stale, or unsupported, when a prepared dependency does not bind to
/// the snapshot, or when a size bound is exceeded (spec 1277 FR-001, FR-007).
/// An empty `proposals` list is a valid success outcome, not an error.
pub fn browser_local_plan(
    snapshot_identity: &SnapshotIdentity,
    snapshot: &SyncedPublicRegistryState,
    verified_dependencies: &[VerifiedRegistryDependency],
    target: &BrowserPlanTarget,
    starting_facts: &Value,
    workspace_id: &str,
    app_manifest: &ManifestReference,
) -> Result<BrowserPlanResponse, BrowserPlanError> {
    check_input_bounds(verified_dependencies, starting_facts)?;
    check_snapshot_identity(snapshot_identity, snapshot)?;

    let mut declared =
        bind_verified_dependencies(snapshot_identity, snapshot, verified_dependencies)?;
    declared.sort_by(|a, b| {
        (&a.capability_id, &a.capability_version).cmp(&(&b.capability_id, &b.capability_version))
    });
    declared.dedup_by(|a, b| {
        a.capability_id == b.capability_id && a.capability_version == b.capability_version
    });

    // --- Structural candidate search (spec 113 FR-002) ----------------
    let starting_outputs = starting_facts_output_schema(starting_facts);
    let target_indices: Vec<usize> = declared
        .iter()
        .enumerate()
        .filter(|(_, capability)| target_matches(target, capability))
        .map(|(index, _)| index)
        .collect();

    let mut truncated = false;
    let mut search_calls_remaining = PLAN_MAX_SEARCH_CALLS;
    let mut all_chains: Vec<Vec<usize>> = Vec::new();
    for &target_index in &target_indices {
        let chains = build_chains(
            &declared,
            &starting_outputs,
            target_index,
            PLAN_MAX_NODES,
            &BTreeSet::new(),
            &mut search_calls_remaining,
            &mut truncated,
        );
        all_chains.extend(chains);
    }

    all_chains.sort_by(|a, b| {
        a.len().cmp(&b.len()).then_with(|| {
            let a_ids: Vec<&str> = a
                .iter()
                .map(|&i| declared[i].capability_id.as_str())
                .collect();
            let b_ids: Vec<&str> = b
                .iter()
                .map(|&i| declared[i].capability_id.as_str())
                .collect();
            a_ids.cmp(&b_ids)
        })
    });
    all_chains.dedup();

    if all_chains.len() > PLAN_MAX_CANDIDATES {
        truncated = true;
    }

    let proposals = all_chains
        .into_iter()
        .take(PLAN_MAX_CANDIDATES)
        .enumerate()
        .map(|(candidate_index, chain)| {
            build_browser_proposal(
                snapshot_identity,
                workspace_id,
                app_manifest,
                starting_facts,
                &declared,
                &chain,
                candidate_index,
            )
        })
        .collect();

    Ok(BrowserPlanResponse {
        proposals,
        plan_search_truncated: truncated,
    })
}

fn serialized_len(value: &Value) -> usize {
    serde_json::to_vec(value).map_or(usize::MAX, |bytes| bytes.len())
}

fn target_matches(target: &BrowserPlanTarget, capability: &DeclaredCapability) -> bool {
    match target {
        BrowserPlanTarget::Capability {
            capability_id,
            capability_version,
        } => {
            capability.capability_id == *capability_id
                && capability.capability_version == *capability_version
        }
        BrowserPlanTarget::EmitsEvent { event_type } => capability
            .contract
            .emits
            .iter()
            .any(|reference| reference.event_id == *event_type),
    }
}

// --- Structural chain search (mirrored from spec 113 workflow_plan.rs) ---

#[allow(clippy::too_many_arguments)]
fn build_chains(
    declared: &[DeclaredCapability],
    starting_outputs: &Value,
    node_index: usize,
    remaining_budget: usize,
    excluded: &BTreeSet<usize>,
    search_calls_remaining: &mut usize,
    truncated: &mut bool,
) -> Vec<Vec<usize>> {
    if *search_calls_remaining == 0 {
        *truncated = true;
        return Vec::new();
    }
    *search_calls_remaining -= 1;

    let node = &declared[node_index];
    let required = schema_required(&node.contract.inputs.schema);
    let node_schema = &node.contract.inputs.schema;

    let mut result = Vec::new();
    if schema_covers_required(starting_outputs, &required, node_schema) {
        result.push(vec![node_index]);
    }

    // A node with no required inputs never gains a predecessor: an empty
    // `required` list is vacuously covered by every candidate's outputs, which
    // would otherwise manufacture meaningless zero-mapping edges.
    if required.is_empty() {
        return result;
    }

    for (candidate_index, candidate) in declared.iter().enumerate() {
        if candidate_index == node_index || excluded.contains(&candidate_index) {
            continue;
        }
        if !schema_covers_required(&candidate.contract.outputs.schema, &required, node_schema) {
            continue;
        }
        if remaining_budget <= 1 {
            *truncated = true;
            continue;
        }
        let mut branch_excluded = excluded.clone();
        branch_excluded.insert(node_index);
        let sub_chains = build_chains(
            declared,
            starting_outputs,
            candidate_index,
            remaining_budget - 1,
            &branch_excluded,
            search_calls_remaining,
            truncated,
        );
        for mut sub_chain in sub_chains {
            sub_chain.push(node_index);
            result.push(sub_chain);
        }
    }
    result
}

fn schema_covers_required(
    source_schema: &Value,
    required: &[String],
    target_schema: &Value,
) -> bool {
    required.iter().all(|property| {
        match (
            schema_property_type(source_schema, property),
            schema_property_type(target_schema, property),
        ) {
            (Some(source_type), Some(target_type)) => source_type == target_type,
            _ => false,
        }
    })
}

fn schema_property_type<'a>(schema: &'a Value, property: &str) -> Option<&'a str> {
    schema
        .get("properties")?
        .get(property)?
        .get("type")?
        .as_str()
}

fn schema_required(schema: &Value) -> Vec<String> {
    schema
        .get("required")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(|value| value.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn starting_facts_output_schema(starting_facts: &Value) -> Value {
    let mut properties = serde_json::Map::new();
    if let Some(object) = starting_facts.as_object() {
        for (key, value) in object {
            properties.insert(key.clone(), json!({ "type": json_type_name(value) }));
        }
    }
    json!({ "type": "object", "properties": Value::Object(properties) })
}

fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(number) if number.is_i64() || number.is_u64() => "integer",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[allow(clippy::too_many_arguments)]
fn build_browser_proposal(
    snapshot_identity: &SnapshotIdentity,
    workspace_id: &str,
    app_manifest: &ManifestReference,
    starting_facts: &Value,
    declared: &[DeclaredCapability],
    chain: &[usize],
    candidate_index: usize,
) -> BrowserWorkflowProposal {
    let mut nodes = Vec::with_capacity(chain.len());
    let mut edges = Vec::new();
    let mut mappings = Vec::new();

    for (position, &node_index) in chain.iter().enumerate() {
        let capability = &declared[node_index];
        let node_id = format!("n{position}");
        nodes.push(ProposalNode {
            node_id: node_id.clone(),
            capability_id: capability.capability_id.clone(),
            capability_version: capability.capability_version.clone(),
            artifact_digest: capability.artifact_digest.clone(),
        });

        let required = schema_required(&capability.contract.inputs.schema);
        if position == 0 {
            for property in &required {
                mappings.push(ProposalMapping {
                    source: MappingSource::InitialInput,
                    source_path: format!("/{property}"),
                    target_node_id: node_id.clone(),
                    target_path: format!("/{property}"),
                });
            }
        } else {
            let predecessor_node_id = format!("n{}", position - 1);
            edges.push(ProposalEdge {
                from_node_id: predecessor_node_id.clone(),
                to_node_id: node_id.clone(),
            });
            for property in &required {
                mappings.push(ProposalMapping {
                    source: MappingSource::Node {
                        node_id: predecessor_node_id.clone(),
                    },
                    source_path: format!("/{property}"),
                    target_node_id: node_id.clone(),
                    target_path: format!("/{property}"),
                });
            }
        }
    }

    let proposal = WorkflowProposal {
        kind: "workflow_proposal".to_string(),
        schema_version: "1.0.0".to_string(),
        proposal_id: format!("browser-plan-candidate-{candidate_index}"),
        workspace_id: workspace_id.to_string(),
        app_manifest: app_manifest.clone(),
        nodes,
        edges,
        mappings,
        initial_input: starting_facts.clone(),
    };

    BrowserWorkflowProposal {
        kind: "browser_workflow_proposal",
        schema_version: BROWSER_WORKFLOW_PROPOSAL_SCHEMA_VERSION,
        snapshot_digest: snapshot_identity.registry_snapshot_digest.clone(),
        source_release: snapshot_identity.source_release.clone(),
        proposal,
        mapping_unconfirmed: true,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;
    use crate::RegistryPrepareEvidence;
    use traverse_contracts::EventReference;
    use traverse_registry::{PublicRegistryCapabilityRecord, SyncedPublicRegistryState};

    const BASE_CONTRACT: &str = include_str!(
        "../../../contracts/examples/meeting-notes/capabilities/process/contract.json"
    );

    fn digest_for(marker: &str) -> String {
        format!("sha256:{marker:0>64}")
    }

    /// A full [`CapabilityContract`] with `id`/`version` and the input/output
    /// property sets overridden. Each `(name, type)` pair becomes a required
    /// property of the given JSON-Schema `type`.
    fn contract(
        id: &str,
        version: &str,
        inputs: &[(&str, &str)],
        outputs: &[(&str, &str)],
        emits: &[&str],
    ) -> CapabilityContract {
        let mut parsed: CapabilityContract =
            serde_json::from_str(BASE_CONTRACT).expect("base contract fixture parses");
        parsed.id = id.to_string();
        parsed.namespace = id.split('.').next().unwrap_or(id).to_string();
        parsed.version = version.to_string();
        parsed.inputs.schema = schema(inputs);
        parsed.outputs.schema = schema(outputs);
        parsed.emits = emits
            .iter()
            .map(|event_id| EventReference {
                event_id: (*event_id).to_string(),
                version: "1.0.0".to_string(),
            })
            .collect();
        parsed
    }

    fn schema(properties: &[(&str, &str)]) -> Value {
        let mut map = serde_json::Map::new();
        let mut required = Vec::new();
        for (name, ty) in properties {
            map.insert((*name).to_string(), json!({ "type": ty }));
            required.push(Value::String((*name).to_string()));
        }
        json!({ "type": "object", "required": required, "properties": Value::Object(map) })
    }

    fn record(contract: &CapabilityContract) -> PublicRegistryCapabilityRecord {
        PublicRegistryCapabilityRecord {
            namespace: contract.namespace.clone(),
            id: contract.id.clone(),
            version: contract.version.clone(),
            digest: digest_for(&format!("a{}", contract.id.replace(['.', '-'], ""))),
            artifact_url: format!("file:///cache/{}.wasm", contract.id),
            contract_digest: digest_for(&format!("c{}", contract.id.replace(['.', '-'], ""))),
            contract_url: format!("file:///cache/{}.json", contract.id),
            deprecated: false,
            summary: String::new(),
            description: String::new(),
            use_cases: Vec::new(),
            service_type: "stateless".to_string(),
            permitted_targets: Vec::new(),
            lifecycle: "active".to_string(),
            provenance: None,
            commercial_use: "unknown".to_string(),
            redistribution: "unknown".to_string(),
            verification_status: "unknown".to_string(),
            license_expression: None,
        }
    }

    fn snapshot(records: &[PublicRegistryCapabilityRecord]) -> SyncedPublicRegistryState {
        SyncedPublicRegistryState {
            schema_version: "1.0.0".to_string(),
            workspace_id: "local".to_string(),
            state_scope: "public_registry_synced".to_string(),
            source_repo: "traverse-framework/registry".to_string(),
            release_tag: "registry-v243".to_string(),
            index_version: 243,
            generated_at: "2026-09-08T00:00:00Z".to_string(),
            source_commit: Some("abc123".to_string()),
            synced_at: "2026-09-08T00:00:00Z".to_string(),
            record_count: records.len(),
            validation_status: "verified".to_string(),
            governing_spec: "055-registry-sync".to_string(),
            capabilities: records.to_vec(),
            events: Vec::new(),
        }
    }

    fn dependency(
        contract: &CapabilityContract,
        snapshot: &SyncedPublicRegistryState,
    ) -> VerifiedRegistryDependency {
        let record = snapshot
            .capabilities
            .iter()
            .find(|record| record.id == contract.id && record.version == contract.version)
            .expect("record for dependency exists in snapshot");
        VerifiedRegistryDependency {
            wasm_binary_path: format!("/cache/sha256/{}", record.digest).into(),
            contract_path: format!("/cache/sha256/{}", record.contract_digest).into(),
            contract_bytes: serde_json::to_vec(contract).expect("contract serializes"),
            wasm_digest: record.digest.clone(),
            evidence: RegistryPrepareEvidence {
                namespace: record.namespace.clone(),
                id: record.id.clone(),
                selected_version: record.version.clone(),
                version_range: format!("={}", record.version),
                source_release: snapshot.release_tag.clone(),
                index_digest: index_snapshot_digest(snapshot),
                artifact_digest: record.digest.clone(),
                verified_at: 1_757_000_000,
                outcome: "prepared",
            },
        }
    }

    fn identity(snapshot: &SyncedPublicRegistryState) -> SnapshotIdentity {
        SnapshotIdentity {
            registry_snapshot_digest: index_snapshot_digest(snapshot),
            source_release: snapshot.release_tag.clone(),
            contract_schema_version: SUPPORTED_CONTRACT_SCHEMA_VERSION.to_string(),
        }
    }

    fn app_manifest() -> ManifestReference {
        ManifestReference {
            app_id: "discover.demo".to_string(),
            app_version: "1.0.0".to_string(),
            manifest_digest: digest_for("m"),
        }
    }

    /// normalize → `transcript` → `summary`; a second normalizer with a
    /// different output shape also exists (ambiguity fixture).
    fn two_step_world() -> (
        SnapshotIdentity,
        SyncedPublicRegistryState,
        Vec<VerifiedRegistryDependency>,
    ) {
        let ingest = contract(
            "evidence.ingest",
            "1.0.1",
            &[("raw", "string")],
            &[("transcript", "string")],
            &[],
        );
        let summarize = contract(
            "evidence.summarize",
            "1.0.0",
            &[("transcript", "string")],
            &[("summary", "string")],
            &["evidence.summary-ready"],
        );
        let records = vec![record(&ingest), record(&summarize)];
        let snap = snapshot(&records);
        let deps = vec![dependency(&ingest, &snap), dependency(&summarize, &snap)];
        (identity(&snap), snap, deps)
    }

    #[test]
    fn deterministic_identical_output_for_the_same_snapshot() {
        let (id, snap, deps) = two_step_world();
        let target = BrowserPlanTarget::EmitsEvent {
            event_type: "evidence.summary-ready".to_string(),
        };
        let facts = json!({ "raw": "hello" });

        let first =
            browser_local_plan(&id, &snap, &deps, &target, &facts, "local", &app_manifest())
                .expect("plan succeeds");
        let second =
            browser_local_plan(&id, &snap, &deps, &target, &facts, "local", &app_manifest())
                .expect("plan succeeds");
        assert_eq!(first, second);
        assert!(!first.plan_search_truncated);
        assert_eq!(first.proposals.len(), 1);
        let proposal = &first.proposals[0];
        assert!(proposal.mapping_unconfirmed);
        assert_eq!(proposal.kind, "browser_workflow_proposal");
        assert_eq!(proposal.snapshot_digest, id.registry_snapshot_digest);
        let node_ids: Vec<&str> = proposal
            .proposal
            .nodes
            .iter()
            .map(|node| node.capability_id.as_str())
            .collect();
        assert_eq!(node_ids, ["evidence.ingest", "evidence.summarize"]);
    }

    #[test]
    fn ambiguous_producers_return_separate_proposals_and_no_winner() {
        // Two capabilities both produce `transcript` from `raw`; the consumer
        // needs `transcript`. The planner returns both chains, picks neither.
        let alpha = contract(
            "evidence.ingest-alpha",
            "1.0.0",
            &[("raw", "string")],
            &[("transcript", "string")],
            &[],
        );
        let beta = contract(
            "evidence.ingest-beta",
            "1.0.0",
            &[("raw", "string")],
            &[("transcript", "string")],
            &[],
        );
        let summarize = contract(
            "evidence.summarize",
            "1.0.0",
            &[("transcript", "string")],
            &[("summary", "string")],
            &[],
        );
        let records = vec![record(&alpha), record(&beta), record(&summarize)];
        let snap = snapshot(&records);
        let deps = vec![
            dependency(&alpha, &snap),
            dependency(&beta, &snap),
            dependency(&summarize, &snap),
        ];
        let response = browser_local_plan(
            &identity(&snap),
            &snap,
            &deps,
            &BrowserPlanTarget::Capability {
                capability_id: "evidence.summarize".to_string(),
                capability_version: "1.0.0".to_string(),
            },
            &json!({ "raw": "x" }),
            "local",
            &app_manifest(),
        )
        .expect("plan succeeds");
        assert_eq!(response.proposals.len(), 2);
        let roots: Vec<&str> = response
            .proposals
            .iter()
            .map(|proposal| proposal.proposal.nodes[0].capability_id.as_str())
            .collect();
        assert_eq!(roots, ["evidence.ingest-alpha", "evidence.ingest-beta"]);
    }

    #[test]
    fn no_candidate_for_a_name_only_apparent_match() {
        // Names imply a pipeline, schemas do not connect: `summarize` needs
        // `transcript`, `ingest` only produces `document`.
        let ingest = contract(
            "evidence.transcript-ingest",
            "1.0.0",
            &[("raw", "string")],
            &[("document", "string")],
            &[],
        );
        let summarize = contract(
            "evidence.transcript-summarize",
            "1.0.0",
            &[("transcript", "string")],
            &[("summary", "string")],
            &[],
        );
        let records = vec![record(&ingest), record(&summarize)];
        let snap = snapshot(&records);
        let deps = vec![dependency(&ingest, &snap), dependency(&summarize, &snap)];
        let response = browser_local_plan(
            &identity(&snap),
            &snap,
            &deps,
            &BrowserPlanTarget::Capability {
                capability_id: "evidence.transcript-summarize".to_string(),
                capability_version: "1.0.0".to_string(),
            },
            &json!({ "raw": "x" }),
            "local",
            &app_manifest(),
        )
        .expect("plan succeeds");
        assert!(response.proposals.is_empty());
        assert!(!response.plan_search_truncated);
    }

    #[test]
    fn snapshot_digest_mismatch_fails_closed() {
        let (mut id, snap, deps) = two_step_world();
        id.registry_snapshot_digest = digest_for("deadbeef");
        let error = browser_local_plan(
            &id,
            &snap,
            &deps,
            &BrowserPlanTarget::Capability {
                capability_id: "evidence.summarize".to_string(),
                capability_version: "1.0.0".to_string(),
            },
            &json!({ "raw": "x" }),
            "local",
            &app_manifest(),
        )
        .expect_err("digest mismatch must fail");
        assert_eq!(error.code, BrowserPlanErrorCode::SnapshotDigestMismatch);
    }

    #[test]
    fn stale_source_release_fails_closed() {
        let (mut id, snap, deps) = two_step_world();
        id.source_release = "registry-v99".to_string();
        let error = browser_local_plan(
            &id,
            &snap,
            &deps,
            &BrowserPlanTarget::EmitsEvent {
                event_type: "evidence.summary-ready".to_string(),
            },
            &json!({ "raw": "x" }),
            "local",
            &app_manifest(),
        )
        .expect_err("stale evidence must fail");
        assert_eq!(error.code, BrowserPlanErrorCode::SnapshotEvidenceStale);
    }

    #[test]
    fn unsupported_contract_schema_version_fails_closed() {
        let (mut id, snap, deps) = two_step_world();
        id.contract_schema_version = "2.0.0".to_string();
        let error = browser_local_plan(
            &id,
            &snap,
            &deps,
            &BrowserPlanTarget::EmitsEvent {
                event_type: "evidence.summary-ready".to_string(),
            },
            &json!({ "raw": "x" }),
            "local",
            &app_manifest(),
        )
        .expect_err("unsupported schema version must fail");
        assert_eq!(
            error.code,
            BrowserPlanErrorCode::UnsupportedContractSchemaVersion
        );
    }

    #[test]
    fn prepared_dependency_from_a_different_snapshot_fails_closed() {
        let (id, snap, mut deps) = two_step_world();
        deps[1].evidence.index_digest = digest_for("otherdigest");
        let error = browser_local_plan(
            &id,
            &snap,
            &deps,
            &BrowserPlanTarget::EmitsEvent {
                event_type: "evidence.summary-ready".to_string(),
            },
            &json!({ "raw": "x" }),
            "local",
            &app_manifest(),
        )
        .expect_err("cross-snapshot dependency must fail");
        assert_eq!(
            error.code,
            BrowserPlanErrorCode::VerifiedDependencyEvidenceMismatch
        );
    }

    #[test]
    fn prepared_dependency_digest_drift_fails_closed() {
        let (id, snap, mut deps) = two_step_world();
        deps[0].wasm_digest = digest_for("drifteddigest");
        let error = browser_local_plan(
            &id,
            &snap,
            &deps,
            &BrowserPlanTarget::EmitsEvent {
                event_type: "evidence.summary-ready".to_string(),
            },
            &json!({ "raw": "x" }),
            "local",
            &app_manifest(),
        )
        .expect_err("digest drift must fail");
        assert_eq!(
            error.code,
            BrowserPlanErrorCode::VerifiedDependencyDigestMismatch
        );
    }

    #[test]
    fn prepared_dependency_missing_from_snapshot_fails_closed() {
        let (id, mut snap, mut deps) = two_step_world();
        snap.capabilities
            .retain(|record| record.id != "evidence.summarize");
        // Re-point identity and every dependency's prepare evidence at the
        // mutated snapshot so the digest and evidence gates pass and the
        // missing-record branch is what fails.
        let id = SnapshotIdentity {
            registry_snapshot_digest: index_snapshot_digest(&snap),
            ..id
        };
        for dependency in &mut deps {
            dependency.evidence.index_digest = id.registry_snapshot_digest.clone();
        }
        let error = browser_local_plan(
            &id,
            &snap,
            &deps,
            &BrowserPlanTarget::EmitsEvent {
                event_type: "evidence.summary-ready".to_string(),
            },
            &json!({ "raw": "x" }),
            "local",
            &app_manifest(),
        )
        .expect_err("missing record must fail");
        assert_eq!(
            error.code,
            BrowserPlanErrorCode::VerifiedDependencyNotInSnapshot
        );
    }

    #[test]
    fn too_many_prepared_dependencies_fails_closed() {
        let (id, snap, deps) = two_step_world();
        let mut oversized = Vec::new();
        for _ in 0..=MAX_VERIFIED_DEPENDENCIES {
            oversized.push(deps[0].clone());
        }
        let error = browser_local_plan(
            &id,
            &snap,
            &oversized,
            &BrowserPlanTarget::EmitsEvent {
                event_type: "evidence.summary-ready".to_string(),
            },
            &json!({ "raw": "x" }),
            "local",
            &app_manifest(),
        )
        .expect_err("oversized dependency set must fail");
        assert_eq!(
            error.code,
            BrowserPlanErrorCode::VerifiedDependencySetTooLarge
        );
    }

    #[test]
    fn oversized_starting_facts_fails_closed() {
        let (id, snap, deps) = two_step_world();
        let big = json!({ "raw": "z".repeat(super::MAX_STARTING_FACTS_BYTES + 1) });
        let error = browser_local_plan(
            &id,
            &snap,
            &deps,
            &BrowserPlanTarget::EmitsEvent {
                event_type: "evidence.summary-ready".to_string(),
            },
            &big,
            "local",
            &app_manifest(),
        )
        .expect_err("oversized starting facts must fail");
        assert_eq!(error.code, BrowserPlanErrorCode::StartingFactsTooLarge);
    }

    #[test]
    fn node_depth_bound_reports_truncation() {
        // A straight chain of 10 capabilities c0..c9; c{n} consumes f{n},
        // produces f{n+1}. Target is c9; the 8-node bound cannot express the
        // full 10-node chain, so truncation is reported and no over-long
        // proposal is emitted.
        let mut contracts = Vec::new();
        for index in 0..10 {
            contracts.push(contract(
                &format!("evidence.step-{index:02}"),
                "1.0.0",
                &[(&format!("f{index}"), "string")],
                &[(&format!("f{}", index + 1), "string")],
                &[],
            ));
        }
        let records: Vec<_> = contracts.iter().map(record).collect();
        let snap = snapshot(&records);
        let deps: Vec<_> = contracts.iter().map(|c| dependency(c, &snap)).collect();
        let response = browser_local_plan(
            &identity(&snap),
            &snap,
            &deps,
            &BrowserPlanTarget::Capability {
                capability_id: "evidence.step-09".to_string(),
                capability_version: "1.0.0".to_string(),
            },
            &json!({ "f0": "seed" }),
            "local",
            &app_manifest(),
        )
        .expect("plan succeeds");
        assert!(response.plan_search_truncated);
        assert!(
            response
                .proposals
                .iter()
                .all(|proposal| proposal.proposal.nodes.len() <= 8)
        );
    }

    #[test]
    fn errors_and_proposals_carry_no_paths_or_bytes() {
        let (mut id, snap, deps) = two_step_world();
        id.registry_snapshot_digest = digest_for("beef");
        let error = browser_local_plan(
            &id,
            &snap,
            &deps,
            &BrowserPlanTarget::EmitsEvent {
                event_type: "evidence.summary-ready".to_string(),
            },
            &json!({ "raw": "x" }),
            "local",
            &app_manifest(),
        )
        .expect_err("must fail");
        let serialized = serde_json::to_string(&error.as_value()).expect("error serializes");
        assert!(
            !serialized.contains('/'),
            "error leaked a path/URL: {serialized}"
        );
        assert!(
            !serialized.contains("cache"),
            "error leaked cache detail: {serialized}"
        );

        let (id, snap, deps) = two_step_world();
        let response = browser_local_plan(
            &id,
            &snap,
            &deps,
            &BrowserPlanTarget::EmitsEvent {
                event_type: "evidence.summary-ready".to_string(),
            },
            &json!({ "raw": "x" }),
            "local",
            &app_manifest(),
        )
        .expect("plan succeeds");
        let serialized = serde_json::to_string(&response).expect("response serializes");
        assert!(
            !serialized.contains("file:///"),
            "proposal leaked a URL: {serialized}"
        );
        assert!(
            !serialized.contains("/cache/"),
            "proposal leaked a cache path: {serialized}"
        );
    }

    #[test]
    fn every_error_code_has_a_distinct_stable_slug() {
        use std::collections::BTreeSet;
        let codes = [
            BrowserPlanErrorCode::UnsupportedContractSchemaVersion,
            BrowserPlanErrorCode::SnapshotDigestMismatch,
            BrowserPlanErrorCode::SnapshotEmpty,
            BrowserPlanErrorCode::SnapshotEvidenceStale,
            BrowserPlanErrorCode::VerifiedDependencyContractInvalid,
            BrowserPlanErrorCode::VerifiedDependencyNotInSnapshot,
            BrowserPlanErrorCode::VerifiedDependencyDigestMismatch,
            BrowserPlanErrorCode::VerifiedDependencyEvidenceMismatch,
            BrowserPlanErrorCode::StartingFactsTooLarge,
            BrowserPlanErrorCode::VerifiedDependencySetTooLarge,
        ];
        let slugs: BTreeSet<&str> = codes.iter().map(|code| code.as_str()).collect();
        assert_eq!(slugs.len(), codes.len());
        for code in codes {
            assert!(code.as_str().starts_with("browser_plan_"));
        }
    }

    #[test]
    fn empty_snapshot_fails_closed() {
        let snap = snapshot(&[]);
        let error = browser_local_plan(
            &identity(&snap),
            &snap,
            &[],
            &BrowserPlanTarget::EmitsEvent {
                event_type: "evidence.summary-ready".to_string(),
            },
            &json!({ "raw": "x" }),
            "local",
            &app_manifest(),
        )
        .expect_err("empty snapshot must fail");
        assert_eq!(error.code, BrowserPlanErrorCode::SnapshotEmpty);
    }

    #[test]
    fn dependency_with_non_contract_bytes_fails_closed() {
        let (id, snap, mut deps) = two_step_world();
        deps[0].contract_bytes = b"this is not a contract".to_vec();
        let error = browser_local_plan(
            &id,
            &snap,
            &deps,
            &BrowserPlanTarget::EmitsEvent {
                event_type: "evidence.summary-ready".to_string(),
            },
            &json!({ "raw": "x" }),
            "local",
            &app_manifest(),
        )
        .expect_err("invalid contract bytes must fail");
        assert_eq!(
            error.code,
            BrowserPlanErrorCode::VerifiedDependencyContractInvalid
        );
    }

    #[test]
    fn dependency_contract_with_unsupported_schema_version_fails_closed() {
        let (id, snap, mut deps) = two_step_world();
        let mut contract: CapabilityContract =
            serde_json::from_slice(&deps[0].contract_bytes).expect("contract parses");
        contract.schema_version = "2.0.0".to_string();
        deps[0].contract_bytes = serde_json::to_vec(&contract).expect("contract serializes");
        let error = browser_local_plan(
            &id,
            &snap,
            &deps,
            &BrowserPlanTarget::EmitsEvent {
                event_type: "evidence.summary-ready".to_string(),
            },
            &json!({ "raw": "x" }),
            "local",
            &app_manifest(),
        )
        .expect_err("unsupported dependency contract schema must fail");
        assert_eq!(
            error.code,
            BrowserPlanErrorCode::UnsupportedContractSchemaVersion
        );
    }

    #[test]
    fn deprecated_snapshot_record_for_a_prepared_dependency_fails_closed() {
        let ingest = contract(
            "evidence.ingest",
            "1.0.1",
            &[("raw", "string")],
            &[("transcript", "string")],
            &[],
        );
        let summarize = contract(
            "evidence.summarize",
            "1.0.0",
            &[("transcript", "string")],
            &[("summary", "string")],
            &["evidence.summary-ready"],
        );
        let mut records = vec![record(&ingest), record(&summarize)];
        records[1].deprecated = true;
        let snap = snapshot(&records);
        let deps = vec![dependency(&ingest, &snap), dependency(&summarize, &snap)];
        let error = browser_local_plan(
            &identity(&snap),
            &snap,
            &deps,
            &BrowserPlanTarget::EmitsEvent {
                event_type: "evidence.summary-ready".to_string(),
            },
            &json!({ "raw": "x" }),
            "local",
            &app_manifest(),
        )
        .expect_err("deprecated record must fail");
        assert_eq!(
            error.code,
            BrowserPlanErrorCode::VerifiedDependencyNotInSnapshot
        );
    }

    #[test]
    fn starting_facts_of_every_json_type_are_projected() {
        // Every `json_type_name` arm is exercised via the starting-facts
        // projection; the target consumes one string field so a proposal is
        // still produced.
        let sink = contract(
            "evidence.sink",
            "1.0.0",
            &[("text", "string")],
            &[("done", "boolean")],
            &["evidence.sink-done"],
        );
        let records = vec![record(&sink)];
        let snap = snapshot(&records);
        let deps = vec![dependency(&sink, &snap)];
        let facts = json!({
            "text": "hello",
            "count": 3,
            "ratio": 1.5,
            "flag": true,
            "nothing": null,
            "list": [1, 2],
            "nested": { "a": 1 }
        });
        let response = browser_local_plan(
            &identity(&snap),
            &snap,
            &deps,
            &BrowserPlanTarget::EmitsEvent {
                event_type: "evidence.sink-done".to_string(),
            },
            &facts,
            "local",
            &app_manifest(),
        )
        .expect("plan succeeds");
        assert_eq!(response.proposals.len(), 1);
    }

    #[test]
    fn non_object_starting_facts_yield_no_candidate() {
        let sink = contract(
            "evidence.sink",
            "1.0.0",
            &[("text", "string")],
            &[("done", "boolean")],
            &[],
        );
        let records = vec![record(&sink)];
        let snap = snapshot(&records);
        let deps = vec![dependency(&sink, &snap)];
        let response = browser_local_plan(
            &identity(&snap),
            &snap,
            &deps,
            &BrowserPlanTarget::Capability {
                capability_id: "evidence.sink".to_string(),
                capability_version: "1.0.0".to_string(),
            },
            &json!("just a string"),
            "local",
            &app_manifest(),
        )
        .expect("plan succeeds");
        assert!(response.proposals.is_empty());
    }

    #[test]
    fn a_capability_with_no_required_inputs_is_a_single_node_plan() {
        let seedless = contract(
            "evidence.seedless",
            "1.0.0",
            &[],
            &[("summary", "string")],
            &["evidence.seedless-done"],
        );
        let records = vec![record(&seedless)];
        let snap = snapshot(&records);
        let deps = vec![dependency(&seedless, &snap)];
        let response = browser_local_plan(
            &identity(&snap),
            &snap,
            &deps,
            &BrowserPlanTarget::EmitsEvent {
                event_type: "evidence.seedless-done".to_string(),
            },
            &json!({}),
            "local",
            &app_manifest(),
        )
        .expect("plan succeeds");
        assert_eq!(response.proposals.len(), 1);
        assert_eq!(response.proposals[0].proposal.nodes.len(), 1);
        assert!(response.proposals[0].proposal.mappings.is_empty());
    }

    #[test]
    fn a_structurally_matching_but_unrooted_predecessor_yields_no_candidate() {
        // `stitch` produces the `bridge` field `finish` needs, but nothing
        // produces `stitch`'s own required `seed` input and the starting
        // facts do not contain it -- so the recursion returns an empty chain
        // set and `finish` has no rooted plan.
        let stitch = contract(
            "evidence.stitch",
            "1.0.0",
            &[("seed", "string")],
            &[("bridge", "string")],
            &[],
        );
        let finish = contract(
            "evidence.finish",
            "1.0.0",
            &[("bridge", "string")],
            &[("done", "string")],
            &["evidence.finish-done"],
        );
        let records = vec![record(&stitch), record(&finish)];
        let snap = snapshot(&records);
        let deps = vec![dependency(&stitch, &snap), dependency(&finish, &snap)];
        let response = browser_local_plan(
            &identity(&snap),
            &snap,
            &deps,
            &BrowserPlanTarget::EmitsEvent {
                event_type: "evidence.finish-done".to_string(),
            },
            &json!({ "unrelated": "x" }),
            "local",
            &app_manifest(),
        )
        .expect("plan succeeds");
        assert!(response.proposals.is_empty());
        assert!(!response.plan_search_truncated);
    }

    #[test]
    fn dense_capability_graph_exhausts_the_search_budget_and_truncates() {
        // Eight capabilities that each consume and produce the same `link`
        // field: every one is a valid predecessor of every other, so chains
        // ending at the target explode past both the 5-candidate cap and the
        // 4 000-search-call budget.
        let mut contracts = Vec::new();
        for index in 0..8 {
            let emits: &[&str] = if index == 7 {
                &["evidence.dense-done"]
            } else {
                &[]
            };
            contracts.push(contract(
                &format!("evidence.dense-{index}"),
                "1.0.0",
                &[("link", "string")],
                &[("link", "string")],
                emits,
            ));
        }
        let records: Vec<_> = contracts.iter().map(record).collect();
        let snap = snapshot(&records);
        let deps: Vec<_> = contracts.iter().map(|c| dependency(c, &snap)).collect();
        let response = browser_local_plan(
            &identity(&snap),
            &snap,
            &deps,
            &BrowserPlanTarget::EmitsEvent {
                event_type: "evidence.dense-done".to_string(),
            },
            &json!({ "link": "seed" }),
            "local",
            &app_manifest(),
        )
        .expect("plan succeeds");
        assert!(response.plan_search_truncated);
        assert!(response.proposals.len() <= 5);
        assert!(
            response
                .proposals
                .iter()
                .all(|proposal| proposal.proposal.nodes.len() <= 8)
        );
    }
}
