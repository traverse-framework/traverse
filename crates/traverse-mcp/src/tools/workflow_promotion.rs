//! Non-mutating export of a completed runtime workflow proposal into a
//! human-reviewable candidate workflow artifact (spec
//! `112-governed-workflow-promotion`, P4, ADR-0042).
//!
//! Export never touches the registry or an app manifest (FR-001) — the
//! candidate is a plain, serializable value with no identity of its own.
//! Promotion into a reusable, versioned workflow definition still requires
//! the same human review and existing registry/bundle publication gates
//! (`traverse_registry::WorkflowRegistry::register`, `traverse-cli workflow
//! register`) as any hand-authored workflow (FR-002) — this module never
//! calls them directly, and [`finalize_candidate_into_definition`] only
//! assembles the `WorkflowDefinition` value a reviewer would still submit
//! through that unchanged path.
//!
//! ## Why the candidate isn't always registrable as-is
//!
//! `WorkflowDefinition` predates proposals and only supports a single
//! direct outgoing edge per node (`traverse_registry`'s own registration
//! validation rejects more than one with `DuplicateItem`) — a real,
//! existing v0.1 constraint this module does not relax or work around. A
//! branching proposal (fan-out/fan-in, which P1/P2 fully support) exports
//! honestly into a candidate that reflects its real edge structure, and
//! correctly fails registration if promoted as-is; only proposals whose
//! nodes each have at most one outgoing edge can currently become a
//! registrable workflow. This is surfaced, not hidden — the candidate's
//! `edges` always mirror the proposal exactly.
//!
//! `WorkflowNode`'s `from_workflow_input`/`to_workflow_state` model shares
//! data through named top-level keys in accumulated workflow state, unlike
//! a proposal's arbitrary-JSON-Pointer field mappings. A mapping whose
//! source and target path share the same final pointer segment (e.g.
//! `/value` → `/value`, or `/a/value` → `/b/value`) translates cleanly;
//! anything else — a rename, or a nested path with a different leaf name —
//! is listed in `unconfirmed_mappings` for the reviewer to resolve by hand
//! rather than guessed at.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

use traverse_contracts::{
    CanonicalProposal, Lifecycle, MappingSource, Owner, RiskMetadata, SchemaContainer,
};
use traverse_registry::{
    WorkflowDefinition, WorkflowEdge, WorkflowEdgeTrigger, WorkflowNode, WorkflowNodeInput,
    WorkflowNodeOutput,
};
use traverse_runtime::proposal::{ProposalTerminalState, ProposalTrace, ResolvedProposalNode};

use crate::{McpError, McpErrorCode};

const WORKFLOW_KIND: &str = "workflow_definition";
const WORKFLOW_SCHEMA_VERSION: &str = "1.0.0";
const WORKFLOW_GOVERNING_SPEC: &str = "007-workflow-registry-traversal";

/// One candidate node's declared capability and best-effort state-key
/// wiring, carried for reviewer visibility (spec 112 FR-002a: preserve
/// reviewed effect/determinism/data-flow/reliability declarations).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandidateNode {
    pub node_id: String,
    pub capability_id: String,
    pub capability_version: String,
    /// `None` only if `resolved_nodes` (a caller-supplied slice) did not
    /// include this node id — otherwise always the node's declared risk.
    pub risk: Option<RiskMetadata>,
    pub from_workflow_input: Vec<String>,
    pub to_workflow_state: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateEdge {
    pub from: String,
    pub to: String,
}

/// A mapping that could not be reduced to a shared top-level state key
/// unambiguously and needs reviewer correction before promotion — mirrors
/// spec 113's `mapping_unconfirmed` convention.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnconfirmedMapping {
    pub source_path: String,
    pub target_node_id: String,
    pub target_path: String,
    pub reason: String,
}

/// A non-mutating, secret-free candidate workflow artifact exported from a
/// successfully completed runtime workflow proposal (spec 112 FR-001,
/// FR-002a). Never registered directly — see the module documentation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowCandidateArtifact {
    pub source_proposal_id: String,
    pub source_proposal_digest: String,
    pub source_snapshot_digest: String,
    pub nodes: Vec<CandidateNode>,
    pub edges: Vec<CandidateEdge>,
    pub start_node: String,
    pub terminal_nodes: Vec<String>,
    pub unconfirmed_mappings: Vec<UnconfirmedMapping>,
    /// Proposal fields intentionally never carried into this candidate
    /// (spec 112 FR-001/FR-002a: no raw inputs, approval material, or
    /// private bindings) — recorded for audit, mirroring
    /// `ApplicationEffectiveConfig::redacted_secret_keys`.
    pub excluded_fields: Vec<String>,
}

/// Exports a completed proposal's execution as a candidate workflow
/// artifact (spec 112 FR-001). Only a proposal whose trace reached
/// `succeeded` may be exported — acceptance scenario 1 requires exporting
/// a *completed* proposal, and an incomplete or failed execution has no
/// proven reusable shape to offer a reviewer.
///
/// # Errors
///
/// Returns [`McpError`] when the trace did not reach `succeeded`.
#[allow(clippy::too_many_lines)]
pub fn export_workflow_candidate(
    canonical: &CanonicalProposal,
    trace: &ProposalTrace,
    resolved_nodes: &[ResolvedProposalNode],
) -> Result<WorkflowCandidateArtifact, McpError> {
    if trace.terminal_state != ProposalTerminalState::Succeeded {
        return Err(McpError {
            code: McpErrorCode::ValidationFailed,
            message: format!(
                "only a successfully completed proposal may be exported; terminal state was {:?}",
                trace.terminal_state
            ),
        });
    }

    let risk_by_node: BTreeMap<&str, &RiskMetadata> = resolved_nodes
        .iter()
        .map(|node| (node.node_id.as_str(), &node.contract.risk))
        .collect();

    let mut from_workflow_input: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();
    let mut to_workflow_state: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();
    let mut unconfirmed_mappings = Vec::new();

    for mapping in &canonical.proposal.mappings {
        let target_key = pointer_leaf(&mapping.target_path);
        let source_key = pointer_leaf(&mapping.source_path);
        if source_key.is_empty() || target_key.is_empty() || source_key != target_key {
            unconfirmed_mappings.push(UnconfirmedMapping {
                source_path: mapping.source_path.clone(),
                target_node_id: mapping.target_node_id.clone(),
                target_path: mapping.target_path.clone(),
                reason: "source and target paths do not share a common field name; \
                         the shared-state model has no rename step"
                    .to_string(),
            });
            continue;
        }

        from_workflow_input
            .entry(mapping.target_node_id.as_str())
            .or_default()
            .insert(target_key.to_string());
        if let MappingSource::Node { node_id } = &mapping.source {
            to_workflow_state
                .entry(node_id.as_str())
                .or_default()
                .insert(source_key.to_string());
        }
    }

    let nodes = canonical
        .proposal
        .nodes
        .iter()
        .map(|node| CandidateNode {
            node_id: node.node_id.clone(),
            capability_id: node.capability_id.clone(),
            capability_version: node.capability_version.clone(),
            risk: risk_by_node
                .get(node.node_id.as_str())
                .map(|risk| (*risk).clone()),
            from_workflow_input: from_workflow_input
                .get(node.node_id.as_str())
                .map(|keys| keys.iter().cloned().collect())
                .unwrap_or_default(),
            to_workflow_state: to_workflow_state
                .get(node.node_id.as_str())
                .map(|keys| keys.iter().cloned().collect())
                .unwrap_or_default(),
        })
        .collect();

    let edges = canonical
        .proposal
        .edges
        .iter()
        .map(|edge| CandidateEdge {
            from: edge.from_node_id.clone(),
            to: edge.to_node_id.clone(),
        })
        .collect();

    let has_incoming: BTreeSet<&str> = canonical
        .proposal
        .edges
        .iter()
        .map(|edge| edge.to_node_id.as_str())
        .collect();
    let has_outgoing: BTreeSet<&str> = canonical
        .proposal
        .edges
        .iter()
        .map(|edge| edge.from_node_id.as_str())
        .collect();
    let start_node = canonical
        .proposal
        .nodes
        .iter()
        .find(|node| !has_incoming.contains(node.node_id.as_str()))
        .map(|node| node.node_id.clone())
        .unwrap_or_default();
    let terminal_nodes = canonical
        .proposal
        .nodes
        .iter()
        .filter(|node| !has_outgoing.contains(node.node_id.as_str()))
        .map(|node| node.node_id.clone())
        .collect();

    Ok(WorkflowCandidateArtifact {
        source_proposal_id: canonical.proposal.proposal_id.clone(),
        source_proposal_digest: trace.proposal_digest.clone(),
        source_snapshot_digest: trace.snapshot_digest.clone(),
        nodes,
        edges,
        start_node,
        terminal_nodes,
        unconfirmed_mappings,
        excluded_fields: vec![
            "initial_input".to_string(),
            "authorization.approval_token_id".to_string(),
        ],
    })
}

/// The final `unconfirmed_mappings` entries are for reviewer information,
/// not for callers of this function; returns the leaf (final) JSON Pointer
/// segment of `pointer`, or an empty string for the root pointer.
fn pointer_leaf(pointer: &str) -> &str {
    pointer.rsplit('/').next().unwrap_or("")
}

/// The identity and metadata a human reviewer assigns at promotion time —
/// never inferred from the source proposal (spec 112 FR-003).
pub struct PromotedWorkflowIdentity {
    pub id: String,
    pub name: String,
    pub version: String,
    pub owner: Owner,
    pub lifecycle: Lifecycle,
    pub summary: String,
    pub tags: Vec<String>,
}

/// Assembles a registrable `WorkflowDefinition` from a candidate plus the
/// identity a human reviewer assigns at promotion time (spec 112 FR-002,
/// FR-003: a promoted workflow has its own immutable identity, decided at
/// promotion — never inferred from the source proposal). This performs no
/// registration itself; the caller still submits the result through the
/// existing `WorkflowRegistry::register` / `traverse-cli workflow
/// register` path.
#[must_use]
pub fn finalize_candidate_into_definition(
    candidate: &WorkflowCandidateArtifact,
    identity: PromotedWorkflowIdentity,
) -> WorkflowDefinition {
    let PromotedWorkflowIdentity {
        id,
        name,
        version,
        owner,
        lifecycle,
        summary,
        tags,
    } = identity;
    WorkflowDefinition {
        kind: WORKFLOW_KIND.to_string(),
        schema_version: WORKFLOW_SCHEMA_VERSION.to_string(),
        id,
        name,
        version,
        lifecycle,
        owner,
        summary,
        inputs: SchemaContainer {
            schema: serde_json::json!({"type": "object"}),
        },
        outputs: SchemaContainer {
            schema: serde_json::json!({"type": "object"}),
        },
        nodes: candidate
            .nodes
            .iter()
            .map(|node| WorkflowNode {
                node_id: node.node_id.clone(),
                capability_id: node.capability_id.clone(),
                capability_version: node.capability_version.clone(),
                input: WorkflowNodeInput {
                    from_workflow_input: node.from_workflow_input.clone(),
                },
                output: WorkflowNodeOutput {
                    to_workflow_state: node.to_workflow_state.clone(),
                    publish_to_state_as: None,
                },
            })
            .collect(),
        edges: candidate
            .edges
            .iter()
            .enumerate()
            .map(|(index, edge)| WorkflowEdge {
                edge_id: format!("edge-{index}"),
                from: edge.from.clone(),
                to: edge.to.clone(),
                trigger: WorkflowEdgeTrigger::Direct,
                event: None,
                predicate: None,
            })
            .collect(),
        start_node: candidate.start_node.clone(),
        terminal_nodes: candidate.terminal_nodes.clone(),
        output_projection: Vec::new(),
        tags,
        governing_spec: WORKFLOW_GOVERNING_SPEC.to_string(),
    }
}
