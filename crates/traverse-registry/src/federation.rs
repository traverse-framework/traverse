#![allow(
    clippy::expect_used,
    clippy::too_many_lines,
    clippy::too_many_arguments,
    clippy::uninlined_format_args,
    clippy::must_use_candidate,
    clippy::needless_pass_by_value,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::map_unwrap_or
)]

use crate::{
    CapabilityRegistry, EventRegistry, LookupScope, RegistryScope, ResolvedCapability,
    ResolvedEvent, ResolvedWorkflow, WorkflowRegistry,
};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::sync::OnceLock;
use traverse_contracts::{ErrorSeverity, Lifecycle};

const APPROVED_SPECS_REGISTRY_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../specs/governance/approved-specs.json"
);

static APPROVED_SPEC_IDS: OnceLock<BTreeSet<String>> = OnceLock::new();
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FederationRegistryKind {
    Capability,
    Event,
    Workflow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FederationApprovalState {
    Approved,
    Draft,
    Deprecated,
    Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FederationTrustState {
    Trusted,
    Pending,
    Blocked,
    Revoked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FederationTrustLevel {
    LocalOnly,
    PeerTrusted,
    PubliclyTrusted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FederationSyncStatus {
    Unknown,
    Success,
    Partial,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FederationInvocationStatus {
    Success,
    Failure,
    RetryableFailure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FederationConflictResolutionState {
    Open,
    Resolved,
    Escalated,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FederationPeer {
    pub peer_id: String,
    pub display_name: String,
    pub trust_state: FederationTrustState,
    pub identity_fingerprint: String,
    pub sync_enabled: bool,
    pub last_sync_at: Option<String>,
    pub last_sync_status: FederationSyncStatus,
    pub visible_registry_scopes: Vec<RegistryScope>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustRecord {
    pub peer_id: String,
    pub trust_model: String,
    pub allowed_scopes: Vec<RegistryScope>,
    pub approved_spec_refs: Vec<String>,
    pub approved_at: String,
    pub revoked_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalChainEntry {
    pub spec_ref: String,
    pub approved_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FederatedContractProvenance {
    pub origin_peer_id: String,
    pub source_ref: String,
    pub validation_evidence_ref: String,
    pub approval_chain: Vec<ApprovalChainEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FederationPeerExport {
    pub peer: FederationPeer,
    pub trust: TrustRecord,
    pub capabilities: Vec<ResolvedCapability>,
    pub events: Vec<ResolvedEvent>,
    pub workflows: Vec<ResolvedWorkflow>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FederationSyncSession {
    pub session_id: String,
    pub peer_id: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub status: FederationSyncStatus,
    pub registry_types: Vec<FederationRegistryKind>,
    pub validated_entries: usize,
    pub rejected_entries: usize,
    pub conflict_count: usize,
    pub evidence_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerRegistrySnapshot {
    pub peer_id: String,
    pub registry_type: FederationRegistryKind,
    pub entry_id: String,
    pub version: String,
    pub scope: RegistryScope,
    pub trust_level: FederationTrustLevel,
    pub approval_state: FederationApprovalState,
    pub contract_ref: String,
    pub provenance: FederatedContractProvenance,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrossPeerTraceProvenance {
    pub trace_id: String,
    pub origin_peer_id: String,
    pub owning_peer_id: String,
    pub trust_level: FederationTrustLevel,
    pub route_reason: String,
    pub sync_session_ref: Option<String>,
    pub response_status: FederationInvocationStatus,
    pub evidence_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FederatedInvocation {
    pub invocation_id: String,
    pub origin_peer_id: String,
    pub target_peer_id: String,
    pub capability_id: String,
    pub request_ref: String,
    pub status: FederationInvocationStatus,
    pub response_ref: Option<String>,
    pub trace_provenance: CrossPeerTraceProvenance,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictRecord {
    pub conflict_id: String,
    pub peer_ids: Vec<String>,
    pub registry_type: FederationRegistryKind,
    pub entry_key: String,
    pub trust_level: FederationTrustLevel,
    pub conflict_reason: String,
    pub resolution_state: FederationConflictResolutionState,
    pub audit_ref: String,
    pub provenance: Option<FederatedContractProvenance>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GovernanceDecisionKind {
    PeerRegistration,
    SnapshotAcceptance,
    SnapshotRejection,
    InvocationAuthorization,
    InvocationDenial,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GovernanceDecisionOutcome {
    Approved,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GovernanceDecisionRecord {
    pub decision_id: String,
    pub peer_id: String,
    pub registry_type: Option<FederationRegistryKind>,
    pub entry_key: Option<String>,
    pub trust_level: FederationTrustLevel,
    pub decision_kind: GovernanceDecisionKind,
    pub outcome: GovernanceDecisionOutcome,
    pub rationale: String,
    pub evidence_ref: String,
    pub provenance: Option<FederatedContractProvenance>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FederationStatusSummary {
    pub peer_count: usize,
    pub trusted_peer_count: usize,
    pub last_sync_outcome: FederationSyncStatus,
    pub sync_age: Option<String>,
    pub conflict_count: usize,
    pub blocked_entries: usize,
    pub route_failures: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FederationSyncOutcome {
    pub session: FederationSyncSession,
    pub accepted_snapshots: Vec<PeerRegistrySnapshot>,
    pub conflicts: Vec<ConflictRecord>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FederationErrorCode {
    MissingRequiredField,
    DuplicatePeer,
    InvalidTrust,
    PeerNotFound,
    EntryValidationFailed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FederationError {
    pub code: FederationErrorCode,
    pub target: String,
    pub message: String,
    pub severity: ErrorSeverity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FederationFailure {
    pub errors: Vec<FederationError>,
}

#[derive(Debug, Default)]
pub struct FederationRegistry {
    peers: BTreeMap<String, FederationPeer>,
    trust_records: BTreeMap<String, TrustRecord>,
    snapshots: BTreeMap<(String, FederationRegistryKind, String, String), PeerRegistrySnapshot>,
    sync_sessions: Vec<FederationSyncSession>,
    invocations: Vec<FederatedInvocation>,
    conflicts: Vec<ConflictRecord>,
    governance_decisions: Vec<GovernanceDecisionRecord>,
}

impl FederationRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_peer(
        &mut self,
        peer: FederationPeer,
        trust: TrustRecord,
    ) -> Result<(), FederationFailure> {
        let mut errors = Vec::new();
        if peer.peer_id.trim().is_empty() {
            errors.push(federation_error(
                FederationErrorCode::MissingRequiredField,
                "$.peer.peer_id",
                "peer_id must not be empty",
            ));
        }
        if peer.display_name.trim().is_empty() {
            errors.push(federation_error(
                FederationErrorCode::MissingRequiredField,
                "$.peer.display_name",
                "display_name must not be empty",
            ));
        }
        if peer.identity_fingerprint.trim().is_empty() {
            errors.push(federation_error(
                FederationErrorCode::MissingRequiredField,
                "$.peer.identity_fingerprint",
                "identity_fingerprint must not be empty",
            ));
        }
        if peer.peer_id != trust.peer_id {
            errors.push(federation_error(
                FederationErrorCode::InvalidTrust,
                "$.trust.peer_id",
                "trust record must reference the same peer_id as the peer",
            ));
        }
        if !peer.sync_enabled {
            errors.push(federation_error(
                FederationErrorCode::InvalidTrust,
                "$.peer.sync_enabled",
                "sync_enabled must be true for a trusted federation peer",
            ));
        }
        if !matches!(peer.trust_state, FederationTrustState::Trusted) {
            errors.push(federation_error(
                FederationErrorCode::InvalidTrust,
                "$.peer.trust_state",
                "peer trust_state must be trusted before federation registration",
            ));
        }
        if trust.allowed_scopes.is_empty() {
            errors.push(federation_error(
                FederationErrorCode::InvalidTrust,
                "$.trust.allowed_scopes",
                "allowed_scopes must not be empty",
            ));
        }
        if trust.approved_spec_refs.is_empty() {
            errors.push(federation_error(
                FederationErrorCode::InvalidTrust,
                "$.trust.approved_spec_refs",
                "approved_spec_refs must not be empty",
            ));
        }
        if !errors.is_empty() {
            return Err(FederationFailure { errors });
        }

        match self.peers.get(&peer.peer_id) {
            Some(existing)
                if existing == &peer && self.trust_records.get(&peer.peer_id) == Some(&trust) =>
            {
                Ok(())
            }
            Some(_) => Err(FederationFailure {
                errors: vec![federation_error(
                    FederationErrorCode::DuplicatePeer,
                    "$.peer.peer_id",
                    "a different federation peer is already registered with this peer_id",
                )],
            }),
            None => {
                self.trust_records
                    .insert(peer.peer_id.clone(), trust.clone());
                self.peers.insert(peer.peer_id.clone(), peer.clone());
                self.governance_decisions.push(GovernanceDecisionRecord {
                    decision_id: format!("decision_peer_registration_{}", peer.peer_id),
                    peer_id: peer.peer_id.clone(),
                    registry_type: None,
                    entry_key: None,
                    trust_level: highest_authorized_trust_level(&trust),
                    decision_kind: GovernanceDecisionKind::PeerRegistration,
                    outcome: GovernanceDecisionOutcome::Approved,
                    rationale: format!(
                        "peer registered with trust model {} and approved specs {:?}",
                        trust.trust_model, trust.approved_spec_refs
                    ),
                    evidence_ref: format!("trust://{}", peer.peer_id),
                    provenance: None,
                });
                Ok(())
            }
        }
    }

    #[must_use]
    pub fn list_peers(&self) -> Vec<FederationPeer> {
        let mut peers = self.peers.values().cloned().collect::<Vec<_>>();
        peers.sort_by(|left, right| left.peer_id.cmp(&right.peer_id));
        peers
    }

    #[must_use]
    pub fn conflicts(&self) -> &[ConflictRecord] {
        &self.conflicts
    }

    #[must_use]
    pub fn governance_decisions(&self) -> &[GovernanceDecisionRecord] {
        &self.governance_decisions
    }

    #[must_use]
    pub fn sync_sessions(&self) -> &[FederationSyncSession] {
        &self.sync_sessions
    }

    #[must_use]
    pub fn invocations(&self) -> &[FederatedInvocation] {
        &self.invocations
    }

    #[must_use]
    pub fn status_summary(&self) -> FederationStatusSummary {
        let trusted_peer_count = self
            .peers
            .values()
            .filter(|peer| peer.trust_state == FederationTrustState::Trusted)
            .count();
        let last_session = self.sync_sessions.last();
        FederationStatusSummary {
            peer_count: self.peers.len(),
            trusted_peer_count,
            last_sync_outcome: last_session
                .map(|session| session.status)
                .unwrap_or(FederationSyncStatus::Unknown),
            sync_age: last_session.and_then(|session| session.finished_at.clone()),
            conflict_count: self.conflicts.len(),
            blocked_entries: self
                .sync_sessions
                .iter()
                .map(|session| session.rejected_entries)
                .sum(),
            route_failures: self
                .invocations
                .iter()
                .filter(|invocation| is_route_failure(invocation.status))
                .count(),
        }
    }

    pub fn sync_peer(
        &mut self,
        export: FederationPeerExport,
        capabilities: &CapabilityRegistry,
        events: &EventRegistry,
        workflows: &WorkflowRegistry,
        started_at: &str,
        finished_at: &str,
        evidence_ref: &str,
    ) -> Result<FederationSyncOutcome, FederationFailure> {
        let mut errors = Vec::new();
        if started_at.trim().is_empty() {
            errors.push(federation_error(
                FederationErrorCode::MissingRequiredField,
                "$.started_at",
                "started_at must not be empty",
            ));
        }
        if finished_at.trim().is_empty() {
            errors.push(federation_error(
                FederationErrorCode::MissingRequiredField,
                "$.finished_at",
                "finished_at must not be empty",
            ));
        }
        if evidence_ref.trim().is_empty() {
            errors.push(federation_error(
                FederationErrorCode::MissingRequiredField,
                "$.evidence_ref",
                "evidence_ref must not be empty",
            ));
        }
        if export.peer.peer_id != export.trust.peer_id {
            errors.push(federation_error(
                FederationErrorCode::InvalidTrust,
                "$.trust.peer_id",
                "export trust record must match the exporting peer id",
            ));
        }

        let Some(registered_peer) = self.peers.get(&export.peer.peer_id) else {
            errors.push(federation_error(
                FederationErrorCode::PeerNotFound,
                "$.peer.peer_id",
                "peer must be registered before it can be synced",
            ));
            return Err(FederationFailure { errors });
        };
        let Some(registered_trust) = self.trust_records.get(&export.peer.peer_id) else {
            errors.push(federation_error(
                FederationErrorCode::InvalidTrust,
                "$.trust.peer_id",
                "peer is missing its approved trust record",
            ));
            return Err(FederationFailure { errors });
        };

        if registered_peer != &export.peer || registered_trust != &export.trust {
            errors.push(federation_error(
                FederationErrorCode::InvalidTrust,
                "$.peer",
                "exported peer metadata must match the registered trusted peer",
            ));
        }
        if !registered_peer.sync_enabled {
            errors.push(federation_error(
                FederationErrorCode::InvalidTrust,
                "$.peer.sync_enabled",
                "sync is disabled for this peer",
            ));
        }
        if registered_peer.trust_state != FederationTrustState::Trusted {
            errors.push(federation_error(
                FederationErrorCode::InvalidTrust,
                "$.peer.trust_state",
                "only trusted peers can participate in federation sync",
            ));
        }
        if !errors.is_empty() {
            return Err(FederationFailure { errors });
        }

        let mut accepted_snapshots = Vec::new();
        let mut conflict_records = Vec::new();

        for capability in &export.capabilities {
            if let Some(snapshot) = validate_capability_snapshot(
                &export.peer,
                &export.trust,
                capabilities,
                capability,
                evidence_ref,
                &mut conflict_records,
            ) {
                accepted_snapshots.push(snapshot);
            }
        }
        for event in &export.events {
            if let Some(snapshot) = validate_event_snapshot(
                &export.peer,
                &export.trust,
                events,
                event,
                evidence_ref,
                &mut conflict_records,
            ) {
                accepted_snapshots.push(snapshot);
            }
        }
        for workflow in &export.workflows {
            if let Some(snapshot) = validate_workflow_snapshot(
                &export.peer,
                &export.trust,
                workflows,
                workflow,
                evidence_ref,
                &mut conflict_records,
            ) {
                accepted_snapshots.push(snapshot);
            }
        }

        for snapshot in &accepted_snapshots {
            let key = (
                snapshot.peer_id.clone(),
                snapshot.registry_type,
                snapshot.entry_id.clone(),
                snapshot.version.clone(),
            );
            self.snapshots.insert(key, snapshot.clone());
        }
        self.conflicts.extend(conflict_records.clone());
        self.governance_decisions.extend(
            accepted_snapshots
                .iter()
                .map(snapshot_acceptance_decision)
                .chain(conflict_records.iter().map(snapshot_rejection_decision)),
        );

        let status = if accepted_snapshots.is_empty() && conflict_records.is_empty() {
            FederationSyncStatus::Failed
        } else if conflict_records.is_empty() {
            FederationSyncStatus::Success
        } else {
            FederationSyncStatus::Partial
        };

        let session = FederationSyncSession {
            session_id: format!(
                "sync_{}_{}",
                export.peer.peer_id,
                self.sync_sessions.len() + 1
            ),
            peer_id: export.peer.peer_id.clone(),
            started_at: started_at.to_string(),
            finished_at: Some(finished_at.to_string()),
            status,
            registry_types: synced_registry_types(&accepted_snapshots),
            validated_entries: accepted_snapshots.len(),
            rejected_entries: conflict_records.len(),
            conflict_count: conflict_records.len(),
            evidence_ref: evidence_ref.to_string(),
        };

        if let Some(peer) = self.peers.get_mut(&export.peer.peer_id) {
            peer.last_sync_at = Some(finished_at.to_string());
            peer.last_sync_status = status;
        }

        self.sync_sessions.push(session.clone());

        Ok(FederationSyncOutcome {
            session,
            accepted_snapshots,
            conflicts: conflict_records,
        })
    }

    pub fn route_capability_invocation(
        &mut self,
        origin_peer_id: &str,
        capability_id: &str,
        version: &str,
        request_ref: &str,
        available_peer_ids: &BTreeSet<String>,
        routed_at: &str,
        evidence_ref: &str,
    ) -> Result<FederatedInvocation, FederationFailure> {
        let mut errors = Vec::new();
        if origin_peer_id.trim().is_empty() {
            errors.push(federation_error(
                FederationErrorCode::MissingRequiredField,
                "$.origin_peer_id",
                "origin_peer_id must not be empty",
            ));
        }
        if capability_id.trim().is_empty() {
            errors.push(federation_error(
                FederationErrorCode::MissingRequiredField,
                "$.capability_id",
                "capability_id must not be empty",
            ));
        }
        if version.trim().is_empty() {
            errors.push(federation_error(
                FederationErrorCode::MissingRequiredField,
                "$.version",
                "version must not be empty",
            ));
        }
        if request_ref.trim().is_empty() {
            errors.push(federation_error(
                FederationErrorCode::MissingRequiredField,
                "$.request_ref",
                "request_ref must not be empty",
            ));
        }
        if routed_at.trim().is_empty() {
            errors.push(federation_error(
                FederationErrorCode::MissingRequiredField,
                "$.routed_at",
                "routed_at must not be empty",
            ));
        }
        if evidence_ref.trim().is_empty() {
            errors.push(federation_error(
                FederationErrorCode::MissingRequiredField,
                "$.evidence_ref",
                "evidence_ref must not be empty",
            ));
        }
        if !self.peers.contains_key(origin_peer_id) {
            errors.push(federation_error(
                FederationErrorCode::PeerNotFound,
                "$.origin_peer_id",
                "origin peer must be registered before routing",
            ));
        }
        if !errors.is_empty() {
            return Err(FederationFailure { errors });
        }

        let origin_peer = self.peers.get(origin_peer_id).expect("validated above");
        let trust = self
            .trust_records
            .get(origin_peer_id)
            .expect("validated above");

        let exact_matches = self
            .snapshots
            .values()
            .filter(|snapshot| snapshot.registry_type == FederationRegistryKind::Capability)
            .filter(|snapshot| snapshot.entry_id == capability_id && snapshot.version == version)
            .cloned()
            .collect::<Vec<_>>();
        let candidate = exact_matches
            .iter()
            .filter(|snapshot| scope_is_visible(snapshot.scope, trust, origin_peer))
            .min_by(|left, right| left.peer_id.cmp(&right.peer_id))
            .cloned()
            .map(|snapshot| (snapshot.peer_id.clone(), snapshot));

        let Some((target_peer_id, target_snapshot)) = candidate else {
            if let Some(denied_snapshot) = exact_matches
                .iter()
                .min_by(|left, right| left.peer_id.cmp(&right.peer_id))
                .cloned()
            {
                let denial_reason = format!(
                    "requested capability requires {:?} visibility and is not authorized for the origin peer under trust model {}",
                    denied_snapshot.trust_level, trust.trust_model
                );
                self.conflicts.push(build_conflict_record(
                    origin_peer_id,
                    FederationRegistryKind::Capability,
                    capability_id,
                    version,
                    denied_snapshot.trust_level,
                    "private capability invocation was denied because the origin peer is not authorized to view the target snapshot",
                    evidence_ref,
                    Some(denied_snapshot.provenance.clone()),
                ));
                self.governance_decisions.push(GovernanceDecisionRecord {
                    decision_id: format!(
                        "decision_invocation_denial_{}_{}_{}",
                        origin_peer_id, capability_id, version
                    ),
                    peer_id: origin_peer_id.to_string(),
                    registry_type: Some(FederationRegistryKind::Capability),
                    entry_key: Some(denied_snapshot.entry_key()),
                    trust_level: denied_snapshot.trust_level,
                    decision_kind: GovernanceDecisionKind::InvocationDenial,
                    outcome: GovernanceDecisionOutcome::Rejected,
                    rationale: denial_reason.clone(),
                    evidence_ref: evidence_ref.to_string(),
                    provenance: Some(denied_snapshot.provenance.clone()),
                });
                return Err(FederationFailure {
                    errors: vec![federation_error(
                        FederationErrorCode::EntryValidationFailed,
                        "$.capability_id",
                        denial_reason.as_str(),
                    )],
                });
            }
            return Err(FederationFailure {
                errors: vec![federation_error(
                    FederationErrorCode::EntryValidationFailed,
                    "$.capability_id",
                    "no synchronized owning peer was found for the requested capability",
                )],
            });
        };

        let available = available_peer_ids.contains(&target_peer_id);
        let sync_session_ref = self
            .sync_sessions
            .iter()
            .rev()
            .find(|session| session.peer_id == target_peer_id)
            .map(|session| session.evidence_ref.clone());
        let trace_id = format!("trace_{}_{}_{}", origin_peer_id, capability_id, version);
        let invocation_id = format!(
            "invocation_{}_{}_{}",
            origin_peer_id, capability_id, version
        );
        let (status, response_ref, route_reason) = if available {
            (
                FederationInvocationStatus::Success,
                Some(format!(
                    "response://{}/{}/{}",
                    target_peer_id, capability_id, version
                )),
                format!(
                    "routed to owning peer {} for synchronized capability snapshot",
                    target_peer_id
                ),
            )
        } else {
            (
                FederationInvocationStatus::RetryableFailure,
                None,
                format!(
                    "owning peer {} is not currently reachable for invocation",
                    target_peer_id
                ),
            )
        };

        let invocation = FederatedInvocation {
            invocation_id,
            origin_peer_id: origin_peer_id.to_string(),
            target_peer_id: target_peer_id.clone(),
            capability_id: capability_id.to_string(),
            request_ref: request_ref.to_string(),
            status,
            response_ref,
            trace_provenance: CrossPeerTraceProvenance {
                trace_id,
                origin_peer_id: origin_peer_id.to_string(),
                owning_peer_id: target_snapshot.peer_id,
                trust_level: target_snapshot.trust_level,
                route_reason,
                sync_session_ref,
                response_status: status,
                evidence_ref: evidence_ref.to_string(),
            },
        };
        self.invocations.push(invocation.clone());
        self.governance_decisions
            .push(invocation_governance_decision(&invocation));
        Ok(invocation)
    }
}

pub fn export_peer_state(
    peer: FederationPeer,
    trust: TrustRecord,
    capabilities: &CapabilityRegistry,
    events: &EventRegistry,
    workflows: &WorkflowRegistry,
) -> FederationPeerExport {
    FederationPeerExport {
        peer,
        trust,
        capabilities: capabilities.graph_entries(),
        events: events.graph_entries(),
        workflows: workflows.graph_entries(),
    }
}

fn validate_capability_snapshot(
    peer: &FederationPeer,
    trust: &TrustRecord,
    capabilities: &CapabilityRegistry,
    export: &ResolvedCapability,
    evidence_ref: &str,
    conflicts: &mut Vec<ConflictRecord>,
) -> Option<PeerRegistrySnapshot> {
    if !scope_is_allowed(export.record.scope, trust, peer) {
        conflicts.push(build_conflict_record(
            peer.peer_id.as_str(),
            FederationRegistryKind::Capability,
            &export.record.id,
            &export.record.version,
            trust_level_for_scope(export.record.scope),
            "peer trust does not authorize the exported scope",
            evidence_ref,
            Some(build_provenance(
                peer,
                trust,
                &export.record.contract_path,
                &export.record.evidence.evidence_id,
            )),
        ));
        return None;
    }

    if !approved_spec_registry_contains(&export.record.evidence.governing_spec) {
        conflicts.push(build_conflict_record(
            peer.peer_id.as_str(),
            FederationRegistryKind::Capability,
            &export.record.id,
            &export.record.version,
            trust_level_for_scope(export.record.scope),
            "exported capability governing spec is not approved in the local approved spec registry",
            evidence_ref,
            Some(build_provenance(
                peer,
                trust,
                &export.record.contract_path,
                &export.record.evidence.evidence_id,
            )),
        ));
        return None;
    }

    let lookup_scope = lookup_scope_for(export.record.scope);
    let Some(local) =
        capabilities.find_exact(lookup_scope, &export.record.id, &export.record.version)
    else {
        conflicts.push(build_conflict_record(
            peer.peer_id.as_str(),
            FederationRegistryKind::Capability,
            &export.record.id,
            &export.record.version,
            trust_level_for_scope(export.record.scope),
            "local approved registry is missing the exported capability",
            evidence_ref,
            Some(build_provenance(
                peer,
                trust,
                &export.record.contract_path,
                &export.record.evidence.evidence_id,
            )),
        ));
        return None;
    };

    if local != *export {
        conflicts.push(build_conflict_record(
            peer.peer_id.as_str(),
            FederationRegistryKind::Capability,
            &export.record.id,
            &export.record.version,
            trust_level_for_scope(export.record.scope),
            "local capability record differs from the exported peer record",
            evidence_ref,
            Some(build_provenance(
                peer,
                trust,
                &export.record.contract_path,
                &export.record.evidence.evidence_id,
            )),
        ));
        return None;
    }

    Some(build_snapshot(
        peer,
        FederationRegistryKind::Capability,
        &export.record.id,
        &export.record.version,
        export.record.scope,
        export.record.lifecycle.clone(),
        &export.record.contract_path,
        build_provenance(
            peer,
            trust,
            &format!(
                "{:?}:{}:{}",
                export.record.provenance.source,
                export.record.provenance.author,
                export.record.provenance.created_at
            ),
            &export.record.evidence.evidence_id,
        ),
    ))
}

fn validate_event_snapshot(
    peer: &FederationPeer,
    trust: &TrustRecord,
    events: &EventRegistry,
    export: &ResolvedEvent,
    evidence_ref: &str,
    conflicts: &mut Vec<ConflictRecord>,
) -> Option<PeerRegistrySnapshot> {
    if !scope_is_allowed(export.record.scope, trust, peer) {
        conflicts.push(build_conflict_record(
            peer.peer_id.as_str(),
            FederationRegistryKind::Event,
            &export.record.id,
            &export.record.version,
            trust_level_for_scope(export.record.scope),
            "peer trust does not authorize the exported scope",
            evidence_ref,
            Some(build_provenance(
                peer,
                trust,
                &export.record.contract_path,
                &format!(
                    "{}:{}:{}",
                    export.record.validation_evidence.kind,
                    export.record.validation_evidence.governing_spec,
                    export.record.validation_evidence.validated_at
                ),
            )),
        ));
        return None;
    }

    if !approved_spec_registry_contains(&export.record.validation_evidence.governing_spec) {
        conflicts.push(build_conflict_record(
            peer.peer_id.as_str(),
            FederationRegistryKind::Event,
            &export.record.id,
            &export.record.version,
            trust_level_for_scope(export.record.scope),
            "exported event governing spec is not approved in the local approved spec registry",
            evidence_ref,
            Some(build_provenance(
                peer,
                trust,
                &export.record.contract_path,
                &format!(
                    "{}:{}:{}",
                    export.record.validation_evidence.kind,
                    export.record.validation_evidence.governing_spec,
                    export.record.validation_evidence.validated_at
                ),
            )),
        ));
        return None;
    }

    let lookup_scope = lookup_scope_for(export.record.scope);
    let Some(local) = events.find_exact(lookup_scope, &export.record.id, &export.record.version)
    else {
        conflicts.push(build_conflict_record(
            peer.peer_id.as_str(),
            FederationRegistryKind::Event,
            &export.record.id,
            &export.record.version,
            trust_level_for_scope(export.record.scope),
            "local approved registry is missing the exported event",
            evidence_ref,
            Some(build_provenance(
                peer,
                trust,
                &export.record.contract_path,
                &format!(
                    "{}:{}:{}",
                    export.record.validation_evidence.kind,
                    export.record.validation_evidence.governing_spec,
                    export.record.validation_evidence.validated_at
                ),
            )),
        ));
        return None;
    };

    if local != *export {
        conflicts.push(build_conflict_record(
            peer.peer_id.as_str(),
            FederationRegistryKind::Event,
            &export.record.id,
            &export.record.version,
            trust_level_for_scope(export.record.scope),
            "local event record differs from the exported peer record",
            evidence_ref,
            Some(build_provenance(
                peer,
                trust,
                &export.record.contract_path,
                &format!(
                    "{}:{}:{}",
                    export.record.validation_evidence.kind,
                    export.record.validation_evidence.governing_spec,
                    export.record.validation_evidence.validated_at
                ),
            )),
        ));
        return None;
    }

    Some(build_snapshot(
        peer,
        FederationRegistryKind::Event,
        &export.record.id,
        &export.record.version,
        export.record.scope,
        export.record.lifecycle.clone(),
        &export.record.contract_path,
        build_provenance(
            peer,
            trust,
            &format!(
                "{:?}:{}:{}",
                export.record.provenance.source,
                export.record.provenance.author,
                export.record.provenance.created_at
            ),
            &format!(
                "{}:{}:{}",
                export.record.validation_evidence.kind,
                export.record.validation_evidence.governing_spec,
                export.record.validation_evidence.validated_at
            ),
        ),
    ))
}

fn validate_workflow_snapshot(
    peer: &FederationPeer,
    trust: &TrustRecord,
    workflows: &WorkflowRegistry,
    export: &ResolvedWorkflow,
    evidence_ref: &str,
    conflicts: &mut Vec<ConflictRecord>,
) -> Option<PeerRegistrySnapshot> {
    if !scope_is_allowed(export.record.scope, trust, peer) {
        conflicts.push(build_conflict_record(
            peer.peer_id.as_str(),
            FederationRegistryKind::Workflow,
            &export.record.id,
            &export.record.version,
            trust_level_for_scope(export.record.scope),
            "peer trust does not authorize the exported scope",
            evidence_ref,
            Some(build_provenance(
                peer,
                trust,
                &export.record.workflow_path,
                &export.record.evidence.evidence_id,
            )),
        ));
        return None;
    }

    if !approved_spec_registry_contains(&export.record.governing_spec) {
        conflicts.push(build_conflict_record(
            peer.peer_id.as_str(),
            FederationRegistryKind::Workflow,
            &export.record.id,
            &export.record.version,
            trust_level_for_scope(export.record.scope),
            "exported workflow governing spec is not approved in the local approved spec registry",
            evidence_ref,
            Some(build_provenance(
                peer,
                trust,
                &export.record.workflow_path,
                &export.record.evidence.evidence_id,
            )),
        ));
        return None;
    }

    let lookup_scope = lookup_scope_for(export.record.scope);
    let Some(local) = workflows.find_exact(lookup_scope, &export.record.id, &export.record.version)
    else {
        conflicts.push(build_conflict_record(
            peer.peer_id.as_str(),
            FederationRegistryKind::Workflow,
            &export.record.id,
            &export.record.version,
            trust_level_for_scope(export.record.scope),
            "local approved registry is missing the exported workflow",
            evidence_ref,
            Some(build_provenance(
                peer,
                trust,
                &export.record.workflow_path,
                &export.record.evidence.evidence_id,
            )),
        ));
        return None;
    };

    if local != *export {
        conflicts.push(build_conflict_record(
            peer.peer_id.as_str(),
            FederationRegistryKind::Workflow,
            &export.record.id,
            &export.record.version,
            trust_level_for_scope(export.record.scope),
            "local workflow record differs from the exported peer record",
            evidence_ref,
            Some(build_provenance(
                peer,
                trust,
                &export.record.workflow_path,
                &export.record.evidence.evidence_id,
            )),
        ));
        return None;
    }

    Some(build_snapshot(
        peer,
        FederationRegistryKind::Workflow,
        &export.record.id,
        &export.record.version,
        export.record.scope,
        export.record.lifecycle.clone(),
        &export.record.workflow_path,
        build_provenance(
            peer,
            trust,
            &export.record.workflow_path,
            &export.record.evidence.evidence_id,
        ),
    ))
}

fn build_snapshot(
    peer: &FederationPeer,
    registry_type: FederationRegistryKind,
    entry_id: &str,
    version: &str,
    scope: RegistryScope,
    lifecycle: Lifecycle,
    contract_ref: &str,
    provenance: FederatedContractProvenance,
) -> PeerRegistrySnapshot {
    PeerRegistrySnapshot {
        peer_id: peer.peer_id.clone(),
        registry_type,
        entry_id: entry_id.to_string(),
        version: version.to_string(),
        scope,
        trust_level: trust_level_for_scope(scope),
        approval_state: approval_state_from_lifecycle(&lifecycle),
        contract_ref: contract_ref.to_string(),
        provenance,
    }
}

fn build_conflict_record(
    peer_id: &str,
    registry_type: FederationRegistryKind,
    entry_id: &str,
    version: &str,
    trust_level: FederationTrustLevel,
    reason: &str,
    audit_ref: &str,
    provenance: Option<FederatedContractProvenance>,
) -> ConflictRecord {
    ConflictRecord {
        conflict_id: format!("conflict_{}_{}_{}", peer_id, entry_id, version),
        peer_ids: vec![peer_id.to_string()],
        registry_type,
        entry_key: format!("{registry_type:?}:{entry_id}@{version}"),
        trust_level,
        conflict_reason: reason.to_string(),
        resolution_state: FederationConflictResolutionState::Open,
        audit_ref: audit_ref.to_string(),
        provenance,
    }
}

fn build_provenance(
    peer: &FederationPeer,
    trust: &TrustRecord,
    source_ref: &str,
    validation_evidence_ref: &str,
) -> FederatedContractProvenance {
    FederatedContractProvenance {
        origin_peer_id: peer.peer_id.clone(),
        source_ref: source_ref.to_string(),
        validation_evidence_ref: validation_evidence_ref.to_string(),
        approval_chain: trust
            .approved_spec_refs
            .iter()
            .map(|spec_ref| ApprovalChainEntry {
                spec_ref: spec_ref.clone(),
                approved_at: trust.approved_at.clone(),
            })
            .collect(),
    }
}

fn approval_state_from_lifecycle(lifecycle: &Lifecycle) -> FederationApprovalState {
    match lifecycle {
        Lifecycle::Draft => FederationApprovalState::Draft,
        Lifecycle::Active => FederationApprovalState::Approved,
        Lifecycle::Deprecated => FederationApprovalState::Deprecated,
        Lifecycle::Retired | Lifecycle::Archived => FederationApprovalState::Rejected,
    }
}

fn is_route_failure(status: FederationInvocationStatus) -> bool {
    matches!(
        status,
        FederationInvocationStatus::Failure | FederationInvocationStatus::RetryableFailure
    )
}

fn trust_level_for_scope(scope: RegistryScope) -> FederationTrustLevel {
    match scope {
        RegistryScope::Public => FederationTrustLevel::PubliclyTrusted,
        RegistryScope::Private => FederationTrustLevel::PeerTrusted,
    }
}

fn highest_authorized_trust_level(trust: &TrustRecord) -> FederationTrustLevel {
    if trust.allowed_scopes.contains(&RegistryScope::Private) {
        FederationTrustLevel::PeerTrusted
    } else if trust.allowed_scopes.contains(&RegistryScope::Public) {
        FederationTrustLevel::PubliclyTrusted
    } else {
        FederationTrustLevel::LocalOnly
    }
}

fn scope_is_allowed(scope: RegistryScope, trust: &TrustRecord, peer: &FederationPeer) -> bool {
    trust.allowed_scopes.contains(&scope) && peer.visible_registry_scopes.contains(&scope)
}

fn scope_is_visible(scope: RegistryScope, trust: &TrustRecord, peer: &FederationPeer) -> bool {
    scope_is_allowed(scope, trust, peer)
}

fn lookup_scope_for(scope: RegistryScope) -> LookupScope {
    match scope {
        RegistryScope::Public => LookupScope::PublicOnly,
        RegistryScope::Private => LookupScope::PreferPrivate,
    }
}

fn approved_spec_registry_contains(spec_id: &str) -> bool {
    APPROVED_SPEC_IDS
        .get_or_init(load_approved_spec_ids)
        .contains(spec_id)
}

fn load_approved_spec_ids() -> BTreeSet<String> {
    load_approved_spec_ids_from_path(APPROVED_SPECS_REGISTRY_PATH)
}

fn load_approved_spec_ids_from_path(path: &str) -> BTreeSet<String> {
    let Ok(contents) = fs::read_to_string(path) else {
        return BTreeSet::new();
    };

    parse_approved_spec_ids(&contents)
}

fn snapshot_acceptance_decision(snapshot: &PeerRegistrySnapshot) -> GovernanceDecisionRecord {
    GovernanceDecisionRecord {
        decision_id: format!(
            "decision_snapshot_acceptance_{}_{}_{}",
            snapshot.peer_id, snapshot.entry_id, snapshot.version
        ),
        peer_id: snapshot.peer_id.clone(),
        registry_type: Some(snapshot.registry_type),
        entry_key: Some(snapshot.entry_key()),
        trust_level: snapshot.trust_level,
        decision_kind: GovernanceDecisionKind::SnapshotAcceptance,
        outcome: GovernanceDecisionOutcome::Approved,
        rationale:
            "remote entry was accepted after trust, visibility, and approved-spec validation"
                .to_string(),
        evidence_ref: snapshot.provenance.validation_evidence_ref.clone(),
        provenance: Some(snapshot.provenance.clone()),
    }
}

fn snapshot_rejection_decision(conflict: &ConflictRecord) -> GovernanceDecisionRecord {
    GovernanceDecisionRecord {
        decision_id: format!("decision_{}", conflict.conflict_id),
        peer_id: conflict.peer_ids.first().cloned().unwrap_or_default(),
        registry_type: Some(conflict.registry_type),
        entry_key: Some(conflict.entry_key.clone()),
        trust_level: conflict.trust_level,
        decision_kind: GovernanceDecisionKind::SnapshotRejection,
        outcome: GovernanceDecisionOutcome::Rejected,
        rationale: conflict.conflict_reason.clone(),
        evidence_ref: conflict.audit_ref.clone(),
        provenance: conflict.provenance.clone(),
    }
}

fn invocation_governance_decision(invocation: &FederatedInvocation) -> GovernanceDecisionRecord {
    let (decision_kind, outcome) = match invocation.status {
        FederationInvocationStatus::Success => (
            GovernanceDecisionKind::InvocationAuthorization,
            GovernanceDecisionOutcome::Approved,
        ),
        FederationInvocationStatus::Failure | FederationInvocationStatus::RetryableFailure => (
            GovernanceDecisionKind::InvocationDenial,
            GovernanceDecisionOutcome::Rejected,
        ),
    };
    GovernanceDecisionRecord {
        decision_id: format!("decision_{}", invocation.invocation_id),
        peer_id: invocation.origin_peer_id.clone(),
        registry_type: Some(FederationRegistryKind::Capability),
        entry_key: Some(format!(
            "{:?}:{}@{}",
            FederationRegistryKind::Capability,
            invocation.capability_id,
            invocation.trace_provenance.trace_id
        )),
        trust_level: invocation.trace_provenance.trust_level,
        decision_kind,
        outcome,
        rationale: invocation.trace_provenance.route_reason.clone(),
        evidence_ref: invocation.trace_provenance.evidence_ref.clone(),
        provenance: None,
    }
}

impl PeerRegistrySnapshot {
    fn entry_key(&self) -> String {
        format!(
            "{:?}:{}@{}",
            self.registry_type, self.entry_id, self.version
        )
    }
}

fn parse_approved_spec_ids(contents: &str) -> BTreeSet<String> {
    let Ok(payload) = serde_json::from_str::<Value>(contents) else {
        return BTreeSet::new();
    };

    payload
        .get("specs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.get("id").and_then(Value::as_str))
        .map(str::to_string)
        .collect()
}

fn synced_registry_types(snapshots: &[PeerRegistrySnapshot]) -> Vec<FederationRegistryKind> {
    let mut kinds = BTreeSet::new();
    for snapshot in snapshots {
        kinds.insert(snapshot.registry_type);
    }
    kinds.into_iter().collect()
}

fn federation_error(code: FederationErrorCode, target: &str, message: &str) -> FederationError {
    FederationError {
        code,
        target: target.to_string(),
        message: message.to_string(),
        severity: ErrorSeverity::Error,
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::too_many_lines)]
mod tests {
    use super::*;
    use crate::{
        ArtifactDigests, BinaryFormat, BinaryReference, CapabilityArtifactRecord,
        CapabilityRegistration, CapabilityRegistry, ComposabilityMetadata, CompositionKind,
        CompositionPattern, EventRegistry, ImplementationKind, RegistryProvenance, RegistryScope,
        SourceKind, SourceReference, WorkflowDefinition, WorkflowNode, WorkflowNodeInput,
        WorkflowNodeOutput, WorkflowRegistration, WorkflowRegistry, export_peer_state,
    };
    use serde_json::json;
    use traverse_contracts::{
        CapabilityContract, Entrypoint, EntrypointKind, EventClassification, EventContract,
        EventPayload, EventProvenance, EventProvenanceSource, EventReference, EventType, Lifecycle,
        Owner, PayloadCompatibility, SchemaContainer, SideEffect, SideEffectKind,
    };

    #[test]
    fn registers_trusted_peer_and_reports_status() {
        let mut federation = FederationRegistry::new();
        let peer = peer("peer-a", "Peer A");
        let trust = trust(
            "peer-a",
            vec![RegistryScope::Public, RegistryScope::Private],
        );

        federation
            .register_peer(peer.clone(), trust.clone())
            .expect("peer should register");
        federation
            .register_peer(peer.clone(), trust.clone())
            .expect("identical peer registration should be idempotent");

        assert_eq!(federation.list_peers(), vec![peer]);
        assert!(federation.sync_sessions().is_empty());
        assert!(federation.invocations().is_empty());
        let summary = federation.status_summary();
        assert_eq!(summary.peer_count, 1);
        assert_eq!(summary.trusted_peer_count, 1);
        assert_eq!(summary.last_sync_outcome, FederationSyncStatus::Unknown);
    }

    #[test]
    fn syncs_peer_export_and_routes_invocation_to_owner() {
        let mut local_capabilities = CapabilityRegistry::new();
        let mut local_events = EventRegistry::new();
        let mut local_workflows = WorkflowRegistry::new();
        seed_capabilities(&mut local_capabilities);
        seed_events(&mut local_events);
        seed_workflows(&mut local_workflows, &local_capabilities);

        let peer = peer("peer-b", "Peer B");
        let trust = trust(
            "peer-b",
            vec![RegistryScope::Public, RegistryScope::Private],
        );
        let export = export_peer_state(
            peer.clone(),
            trust.clone(),
            &local_capabilities,
            &local_events,
            &local_workflows,
        );

        let mut federation = FederationRegistry::new();
        federation
            .register_peer(peer, trust)
            .expect("peer should register");

        let outcome = federation
            .sync_peer(
                export,
                &local_capabilities,
                &local_events,
                &local_workflows,
                "2026-04-09T20:00:00Z",
                "2026-04-09T20:01:00Z",
                "evidence:sync-001",
            )
            .expect("sync should pass");

        assert_eq!(outcome.session.status, FederationSyncStatus::Success);
        assert!(!outcome.accepted_snapshots.is_empty());
        assert!(outcome.conflicts.is_empty());

        let origin_peer = self::peer("peer-a", "Peer A");
        let origin_trust = self::trust(
            "peer-a",
            vec![RegistryScope::Public, RegistryScope::Private],
        );
        federation
            .register_peer(origin_peer, origin_trust)
            .expect("origin peer should register");
        let available = BTreeSet::from([String::from("peer-b")]);
        let invocation = federation
            .route_capability_invocation(
                "peer-a",
                "federation.capability.echo",
                "1.0.0",
                "request:001",
                &available,
                "2026-04-09T20:02:00Z",
                "evidence:route-001",
            )
            .expect("invocation should route");

        assert_eq!(invocation.status, FederationInvocationStatus::Success);
        assert_eq!(invocation.target_peer_id, "peer-b");
        assert_eq!(invocation.trace_provenance.origin_peer_id, "peer-a");
        assert_eq!(invocation.trace_provenance.owning_peer_id, "peer-b");
        assert_eq!(
            invocation.response_ref.as_deref(),
            Some("response://peer-b/federation.capability.echo/1.0.0")
        );
    }

    #[test]
    fn sync_reports_conflicts_for_divergent_private_entries() {
        let mut local_capabilities = CapabilityRegistry::new();
        let mut local_events = EventRegistry::new();
        let mut local_workflows = WorkflowRegistry::new();
        seed_capabilities(&mut local_capabilities);
        seed_events(&mut local_events);
        seed_workflows(&mut local_workflows, &local_capabilities);

        let mut remote_capabilities = CapabilityRegistry::new();
        let mut altered_contract = capability_contract();
        altered_contract.summary = "divergent export".to_string();
        remote_capabilities
            .register(capability_registration(
                RegistryScope::Private,
                altered_contract,
            ))
            .expect("remote capability should register");
        seed_events(&mut local_events);
        seed_workflows(&mut local_workflows, &local_capabilities);

        let peer = peer("peer-c", "Peer C");
        let trust = trust("peer-c", vec![RegistryScope::Public]);
        let export = export_peer_state(
            peer.clone(),
            trust.clone(),
            &remote_capabilities,
            &local_events,
            &local_workflows,
        );

        let mut federation = FederationRegistry::new();
        federation
            .register_peer(peer, trust)
            .expect("peer should register");

        let outcome = federation
            .sync_peer(
                export,
                &local_capabilities,
                &local_events,
                &local_workflows,
                "2026-04-09T20:10:00Z",
                "2026-04-09T20:11:00Z",
                "evidence:sync-002",
            )
            .expect("sync should report conflicts rather than failing");

        assert_eq!(outcome.session.status, FederationSyncStatus::Partial);
        assert!(!outcome.conflicts.is_empty());
        assert_eq!(federation.conflicts().len(), outcome.conflicts.len());
    }

    #[test]
    fn sync_reports_conflicts_for_permitted_but_divergent_private_capability_entries() {
        let mut local_capabilities = CapabilityRegistry::new();
        seed_capabilities(&mut local_capabilities);

        let mut altered_contract = private_capability_contract();
        altered_contract.summary = "altered private capability".to_string();
        let mut remote_capabilities = CapabilityRegistry::new();
        remote_capabilities
            .register(capability_registration(
                RegistryScope::Private,
                altered_contract,
            ))
            .expect("remote private capability should register");

        let peer = peer("peer-capability-divergent", "Peer Capability Divergent");
        let trust = trust(
            "peer-capability-divergent",
            vec![RegistryScope::Public, RegistryScope::Private],
        );

        let mut federation = FederationRegistry::new();
        federation
            .register_peer(peer.clone(), trust.clone())
            .expect("peer should register");

        let export = export_peer_state(
            peer,
            trust,
            &remote_capabilities,
            &EventRegistry::new(),
            &WorkflowRegistry::new(),
        );
        let outcome = federation
            .sync_peer(
                export,
                &local_capabilities,
                &EventRegistry::new(),
                &WorkflowRegistry::new(),
                "2026-04-09T20:50:00Z",
                "2026-04-09T20:51:00Z",
                "evidence:divergent-private-capability",
            )
            .expect("sync should report permitted capability divergence as conflicts");

        assert!(outcome.conflicts.iter().any(|conflict| {
            conflict
                .conflict_reason
                .contains("local capability record differs")
        }));
    }

    #[test]
    fn sync_peer_rejects_trust_and_peer_state_mismatches() {
        let mut federation = FederationRegistry::new();
        let peer = peer("peer-sync-guard", "Peer Sync Guard");
        let trust = trust("peer-sync-guard", vec![RegistryScope::Public]);

        federation
            .register_peer(peer.clone(), trust.clone())
            .expect("peer should register");

        let export = FederationPeerExport {
            peer: peer.clone(),
            trust: trust.clone(),
            capabilities: Vec::new(),
            events: Vec::new(),
            workflows: Vec::new(),
        };

        let mut mismatched_trust = export.clone();
        mismatched_trust.trust.peer_id = "other-peer".to_string();
        assert!(
            federation
                .sync_peer(
                    mismatched_trust,
                    &CapabilityRegistry::new(),
                    &EventRegistry::new(),
                    &WorkflowRegistry::new(),
                    "2026-04-09T20:20:00Z",
                    "2026-04-09T20:21:00Z",
                    "evidence:sync-mismatch",
                )
                .is_err()
        );

        federation.trust_records.remove("peer-sync-guard");
        assert!(
            federation
                .sync_peer(
                    export.clone(),
                    &CapabilityRegistry::new(),
                    &EventRegistry::new(),
                    &WorkflowRegistry::new(),
                    "2026-04-09T20:22:00Z",
                    "2026-04-09T20:23:00Z",
                    "evidence:sync-missing-trust",
                )
                .is_err()
        );

        federation.peers.remove("peer-sync-guard");
        federation
            .register_peer(peer.clone(), trust.clone())
            .expect("peer should re-register");
        if let Some(registered_peer) = federation.peers.get_mut("peer-sync-guard") {
            registered_peer.sync_enabled = false;
            registered_peer.trust_state = FederationTrustState::Pending;
        }

        assert!(
            federation
                .sync_peer(
                    export,
                    &CapabilityRegistry::new(),
                    &EventRegistry::new(),
                    &WorkflowRegistry::new(),
                    "2026-04-09T20:24:00Z",
                    "2026-04-09T20:25:00Z",
                    "evidence:sync-disabled",
                )
                .is_err()
        );
    }

    #[test]
    fn sync_peer_reports_missing_local_registry_entries() {
        let peer = peer("peer-missing", "Peer Missing");
        let trust = trust(
            "peer-missing",
            vec![RegistryScope::Public, RegistryScope::Private],
        );

        let mut federation = FederationRegistry::new();
        federation
            .register_peer(peer.clone(), trust.clone())
            .expect("peer should register");
        let mut remote_capabilities = CapabilityRegistry::new();
        seed_capabilities(&mut remote_capabilities);
        let local_capabilities = CapabilityRegistry::new();
        let local_events = EventRegistry::new();
        let local_workflows = WorkflowRegistry::new();
        let capability_export = export_peer_state(
            peer.clone(),
            trust.clone(),
            &remote_capabilities,
            &EventRegistry::new(),
            &WorkflowRegistry::new(),
        );
        let capability_outcome = federation
            .sync_peer(
                capability_export,
                &local_capabilities,
                &local_events,
                &local_workflows,
                "2026-04-09T20:30:00Z",
                "2026-04-09T20:31:00Z",
                "evidence:missing-capability",
            )
            .expect("sync should report missing capability as conflict");
        assert!(capability_outcome.conflicts.iter().any(|conflict| {
            conflict
                .conflict_reason
                .contains("missing the exported capability")
        }));

        let mut federation = FederationRegistry::new();
        federation
            .register_peer(peer.clone(), trust.clone())
            .expect("peer should register");
        let mut remote_events = EventRegistry::new();
        seed_events(&mut remote_events);
        let event_export = export_peer_state(
            peer.clone(),
            trust.clone(),
            &CapabilityRegistry::new(),
            &remote_events,
            &WorkflowRegistry::new(),
        );
        let event_outcome = federation
            .sync_peer(
                event_export,
                &CapabilityRegistry::new(),
                &EventRegistry::new(),
                &WorkflowRegistry::new(),
                "2026-04-09T20:32:00Z",
                "2026-04-09T20:33:00Z",
                "evidence:missing-event",
            )
            .expect("sync should report missing event as conflict");
        assert!(event_outcome.conflicts.iter().any(|conflict| {
            conflict
                .conflict_reason
                .contains("missing the exported event")
        }));

        let mut federation = FederationRegistry::new();
        federation
            .register_peer(peer.clone(), trust.clone())
            .expect("peer should register");
        let mut remote_workflows = WorkflowRegistry::new();
        let mut local_capabilities = CapabilityRegistry::new();
        let mut local_events = EventRegistry::new();
        seed_capabilities(&mut local_capabilities);
        seed_events(&mut local_events);
        seed_workflows(&mut remote_workflows, &local_capabilities);
        let workflow_export = export_peer_state(
            peer.clone(),
            trust.clone(),
            &CapabilityRegistry::new(),
            &EventRegistry::new(),
            &remote_workflows,
        );
        let workflow_outcome = federation
            .sync_peer(
                workflow_export,
                &CapabilityRegistry::new(),
                &EventRegistry::new(),
                &WorkflowRegistry::new(),
                "2026-04-09T20:34:00Z",
                "2026-04-09T20:35:00Z",
                "evidence:missing-workflow",
            )
            .expect("sync should report missing workflow as conflict");
        assert!(workflow_outcome.conflicts.iter().any(|conflict| {
            conflict
                .conflict_reason
                .contains("missing the exported workflow")
        }));
    }

    #[test]
    fn sync_peer_marks_empty_exports_as_failed() {
        let mut federation = FederationRegistry::new();
        let peer = peer("peer-empty", "Peer Empty");
        let trust = trust("peer-empty", vec![RegistryScope::Public]);
        federation
            .register_peer(peer.clone(), trust.clone())
            .expect("peer should register");

        let export = export_peer_state(
            peer,
            trust,
            &CapabilityRegistry::new(),
            &EventRegistry::new(),
            &WorkflowRegistry::new(),
        );
        let outcome = federation
            .sync_peer(
                export,
                &CapabilityRegistry::new(),
                &EventRegistry::new(),
                &WorkflowRegistry::new(),
                "2026-04-09T20:40:00Z",
                "2026-04-09T20:41:00Z",
                "evidence:sync-empty",
            )
            .expect("empty export should still be accepted as a failed sync outcome");

        assert_eq!(outcome.session.status, FederationSyncStatus::Failed);
        assert!(outcome.accepted_snapshots.is_empty());
        assert!(outcome.conflicts.is_empty());
    }

    #[test]
    fn sync_peer_rejects_private_exports_without_scope_authority() {
        let mut federation = FederationRegistry::new();
        let peer = peer("peer-private", "Peer Private");
        let trust = trust("peer-private", vec![RegistryScope::Public]);
        federation
            .register_peer(peer.clone(), trust.clone())
            .expect("peer should register");

        let mut remote_capabilities = CapabilityRegistry::new();
        remote_capabilities
            .register(capability_registration(
                RegistryScope::Private,
                private_capability_contract(),
            ))
            .expect("private capability should register");

        let mut remote_events = EventRegistry::new();
        remote_events
            .register(event_registration(RegistryScope::Private, event_contract()))
            .expect("private event should register");

        let mut workflow_capabilities = CapabilityRegistry::new();
        seed_capabilities(&mut workflow_capabilities);
        let mut remote_workflows = WorkflowRegistry::new();
        remote_workflows
            .register(
                &workflow_capabilities,
                workflow_registration(RegistryScope::Private, workflow_definition()),
            )
            .expect("private workflow should register");

        let export = export_peer_state(
            peer,
            trust,
            &remote_capabilities,
            &remote_events,
            &remote_workflows,
        );
        let outcome = federation
            .sync_peer(
                export,
                &CapabilityRegistry::new(),
                &EventRegistry::new(),
                &WorkflowRegistry::new(),
                "2026-04-09T20:42:00Z",
                "2026-04-09T20:43:00Z",
                "evidence:sync-private",
            )
            .expect("sync should report private-scope rejection as conflicts");

        assert_eq!(outcome.session.status, FederationSyncStatus::Partial);
        assert!(
            outcome
                .conflicts
                .iter()
                .all(|conflict| conflict.conflict_reason.contains("does not authorize"))
        );
    }

    #[test]
    fn sync_reports_conflicts_for_divergent_private_event_and_workflow_entries() {
        let mut federation = FederationRegistry::new();
        let peer = peer("peer-divergent", "Peer Divergent");
        let trust = trust(
            "peer-divergent",
            vec![RegistryScope::Public, RegistryScope::Private],
        );
        federation
            .register_peer(peer.clone(), trust.clone())
            .expect("peer should register");

        let mut local_events = EventRegistry::new();
        let mut remote_events = EventRegistry::new();
        let mut local_event_contract = event_contract();
        local_event_contract.summary = "local event".to_string();
        local_events
            .register(event_registration(
                RegistryScope::Private,
                local_event_contract.clone(),
            ))
            .expect("local private event should register");
        let mut remote_event_contract = local_event_contract.clone();
        remote_event_contract.summary = "remote event".to_string();
        remote_events
            .register(event_registration(
                RegistryScope::Private,
                remote_event_contract,
            ))
            .expect("remote private event should register");

        let event_export = export_peer_state(
            peer.clone(),
            trust.clone(),
            &CapabilityRegistry::new(),
            &remote_events,
            &WorkflowRegistry::new(),
        );
        let event_outcome = federation
            .sync_peer(
                event_export,
                &CapabilityRegistry::new(),
                &local_events,
                &WorkflowRegistry::new(),
                "2026-04-09T20:44:00Z",
                "2026-04-09T20:45:00Z",
                "evidence:divergent-event",
            )
            .expect("event divergence should report conflicts");
        assert!(event_outcome.conflicts.iter().any(|conflict| {
            conflict
                .conflict_reason
                .contains("local event record differs")
        }));

        let mut federation = FederationRegistry::new();
        federation
            .register_peer(peer.clone(), trust.clone())
            .expect("peer should register");
        let mut local_capabilities = CapabilityRegistry::new();
        let mut remote_capabilities = CapabilityRegistry::new();
        seed_capabilities(&mut local_capabilities);
        seed_capabilities(&mut remote_capabilities);

        let mut local_workflows = WorkflowRegistry::new();
        let mut remote_workflows = WorkflowRegistry::new();
        local_workflows
            .register(
                &local_capabilities,
                workflow_registration(RegistryScope::Private, workflow_definition()),
            )
            .expect("local private workflow should register");
        let mut remote_workflow_definition = workflow_definition();
        remote_workflow_definition.summary = "remote workflow".to_string();
        remote_workflows
            .register(
                &remote_capabilities,
                workflow_registration(RegistryScope::Private, remote_workflow_definition),
            )
            .expect("remote private workflow should register");

        let workflow_export = export_peer_state(
            peer,
            trust,
            &remote_capabilities,
            &EventRegistry::new(),
            &remote_workflows,
        );
        let workflow_outcome = federation
            .sync_peer(
                workflow_export,
                &local_capabilities,
                &EventRegistry::new(),
                &local_workflows,
                "2026-04-09T20:46:00Z",
                "2026-04-09T20:47:00Z",
                "evidence:divergent-workflow",
            )
            .expect("workflow divergence should report conflicts");
        assert!(workflow_outcome.conflicts.iter().any(|conflict| {
            conflict
                .conflict_reason
                .contains("local workflow record differs")
        }));
    }

    #[test]
    fn sync_rejects_unapproved_governing_specs_with_audit_evidence() {
        let capability_outcome = sync_with_unapproved_capability_spec();
        assert!(
            capability_outcome
                .conflicts
                .iter()
                .any(|conflict| { conflict.conflict_reason.contains("approved spec registry") })
        );

        let event_outcome = sync_with_unapproved_event_spec();
        assert!(
            event_outcome
                .conflicts
                .iter()
                .any(|conflict| { conflict.conflict_reason.contains("approved spec registry") })
        );

        let workflow_outcome = sync_with_unapproved_workflow_spec();
        assert!(
            workflow_outcome
                .conflicts
                .iter()
                .any(|conflict| { conflict.conflict_reason.contains("approved spec registry") })
        );
    }

    #[test]
    fn approved_spec_loader_returns_empty_for_missing_or_invalid_inputs() {
        assert!(
            load_approved_spec_ids_from_path("/definitely-missing/approved-specs.json").is_empty()
        );
        assert!(parse_approved_spec_ids("{not-json").is_empty());
    }

    #[test]
    fn route_capability_invocation_returns_error_without_matching_snapshot() {
        let mut federation = FederationRegistry::new();
        let origin_peer = peer("peer-route-empty", "Peer Route Empty");
        let origin_trust = trust("peer-route-empty", vec![RegistryScope::Public]);
        federation
            .register_peer(origin_peer, origin_trust)
            .expect("origin peer should register");

        let failure = federation
            .route_capability_invocation(
                "peer-route-empty",
                "federation.capability.missing",
                "9.9.9",
                "request:missing",
                &BTreeSet::from([String::from("peer-route-empty")]),
                "2026-04-09T20:48:00Z",
                "evidence:missing-route",
            )
            .expect_err("missing snapshot should fail closed");

        assert_eq!(
            failure.errors[0].code,
            FederationErrorCode::EntryValidationFailed
        );
    }

    #[test]
    fn register_peer_rejects_invalid_and_duplicate_peers() {
        let mut federation = FederationRegistry::new();

        let mut invalid_peer = peer("peer-invalid", "Peer Invalid");
        invalid_peer.peer_id.clear();
        invalid_peer.display_name.clear();
        invalid_peer.identity_fingerprint.clear();
        invalid_peer.sync_enabled = false;
        invalid_peer.trust_state = FederationTrustState::Pending;

        let invalid_trust = TrustRecord {
            peer_id: "other-peer".to_string(),
            trust_model: "allow-list".to_string(),
            allowed_scopes: vec![],
            approved_spec_refs: vec![],
            approved_at: "2026-04-09T00:00:00Z".to_string(),
            revoked_at: None,
        };

        assert!(
            federation
                .register_peer(invalid_peer, invalid_trust)
                .is_err()
        );

        let peer = peer("peer-dup", "Peer Dup");
        let trust = trust("peer-dup", vec![RegistryScope::Public]);
        federation
            .register_peer(peer.clone(), trust.clone())
            .expect("peer should register");

        let mut changed_peer = peer.clone();
        changed_peer.display_name = "Peer Dup Updated".to_string();
        assert!(federation.register_peer(changed_peer, trust).is_err());
    }

    #[test]
    fn sync_peer_rejects_unregistered_and_invalid_export_paths() {
        let mut federation = FederationRegistry::new();
        let registered_peer = peer("peer-sync", "Peer Sync");
        let trust = trust("peer-sync", vec![RegistryScope::Public]);
        federation
            .register_peer(registered_peer.clone(), trust.clone())
            .expect("peer should register");

        let local_capabilities = CapabilityRegistry::new();
        let local_events = EventRegistry::new();
        let local_workflows = WorkflowRegistry::new();
        let export = FederationPeerExport {
            peer: registered_peer.clone(),
            trust: trust.clone(),
            capabilities: Vec::new(),
            events: Vec::new(),
            workflows: Vec::new(),
        };

        assert!(
            federation
                .sync_peer(
                    export.clone(),
                    &local_capabilities,
                    &local_events,
                    &local_workflows,
                    "",
                    "",
                    "",
                )
                .is_err()
        );

        let mut bad_peer = peer("peer-sync-bad", "Peer Sync Bad");
        bad_peer.sync_enabled = false;
        let bad_export = FederationPeerExport {
            peer: bad_peer,
            trust: TrustRecord {
                peer_id: "peer-sync-bad".to_string(),
                trust_model: "allow-list".to_string(),
                allowed_scopes: vec![RegistryScope::Public],
                approved_spec_refs: vec!["005-capability-registry".to_string()],
                approved_at: "2026-04-09T00:00:00Z".to_string(),
                revoked_at: None,
            },
            capabilities: Vec::new(),
            events: Vec::new(),
            workflows: Vec::new(),
        };

        assert!(
            federation
                .sync_peer(
                    bad_export,
                    &local_capabilities,
                    &local_events,
                    &local_workflows,
                    "2026-04-09T20:00:00Z",
                    "2026-04-09T20:01:00Z",
                    "evidence:sync-invalid",
                )
                .is_err()
        );
    }

    #[test]
    fn route_capability_invocation_covers_missing_and_unavailable_paths() {
        let mut federation = FederationRegistry::new();
        let origin_peer = peer("peer-route", "Peer Route");
        let origin_trust = trust("peer-route", vec![RegistryScope::Public]);
        federation
            .register_peer(origin_peer.clone(), origin_trust)
            .expect("origin peer should register");

        assert!(
            federation
                .route_capability_invocation("", "", "", "", &BTreeSet::new(), "", "",)
                .is_err()
        );

        let mut local_capabilities = CapabilityRegistry::new();
        let mut local_events = EventRegistry::new();
        let mut local_workflows = WorkflowRegistry::new();
        seed_capabilities(&mut local_capabilities);
        seed_events(&mut local_events);
        seed_workflows(&mut local_workflows, &local_capabilities);

        let target_peer = peer("peer-target", "Peer Target");
        let target_trust = trust(
            "peer-target",
            vec![RegistryScope::Public, RegistryScope::Private],
        );
        let export = export_peer_state(
            target_peer.clone(),
            target_trust.clone(),
            &local_capabilities,
            &local_events,
            &local_workflows,
        );
        federation
            .register_peer(target_peer, target_trust)
            .expect("target peer should register");
        federation
            .sync_peer(
                export,
                &local_capabilities,
                &local_events,
                &local_workflows,
                "2026-04-09T21:00:00Z",
                "2026-04-09T21:01:00Z",
                "evidence:sync-route",
            )
            .expect("sync should succeed");

        let unavailable = BTreeSet::new();
        let invocation = federation
            .route_capability_invocation(
                "peer-route",
                "federation.capability.echo",
                "1.0.0",
                "request:route-unavailable",
                &unavailable,
                "2026-04-09T21:02:00Z",
                "evidence:route-unavailable",
            )
            .expect("route should return retryable failure rather than error");

        assert_eq!(
            invocation.status,
            FederationInvocationStatus::RetryableFailure
        );
        assert!(invocation.response_ref.is_none());
        assert!(federation.governance_decisions().iter().any(|decision| {
            decision.decision_kind == GovernanceDecisionKind::InvocationDenial
                && decision.evidence_ref == "evidence:route-unavailable"
        }));
    }

    #[test]
    fn sync_records_structured_provenance_and_governance_decisions() {
        let mut local_capabilities = CapabilityRegistry::new();
        let mut local_events = EventRegistry::new();
        let mut local_workflows = WorkflowRegistry::new();
        seed_capabilities(&mut local_capabilities);
        seed_events(&mut local_events);
        seed_workflows(&mut local_workflows, &local_capabilities);

        let peer = peer("peer-governed", "Peer Governed");
        let trust = trust(
            "peer-governed",
            vec![RegistryScope::Public, RegistryScope::Private],
        );

        let mut federation = FederationRegistry::new();
        federation
            .register_peer(peer.clone(), trust.clone())
            .expect("peer should register");
        let export = export_peer_state(
            peer,
            trust,
            &local_capabilities,
            &local_events,
            &local_workflows,
        );
        let outcome = federation
            .sync_peer(
                export,
                &local_capabilities,
                &local_events,
                &local_workflows,
                "2026-04-09T21:10:00Z",
                "2026-04-09T21:11:00Z",
                "evidence:governed-sync",
            )
            .expect("sync should succeed");

        let private_snapshot = outcome
            .accepted_snapshots
            .iter()
            .find(|snapshot| snapshot.scope == RegistryScope::Private)
            .expect("private snapshot should exist");
        assert_eq!(
            private_snapshot.trust_level,
            FederationTrustLevel::PeerTrusted
        );
        assert_eq!(private_snapshot.provenance.origin_peer_id, "peer-governed");
        assert_eq!(
            private_snapshot.provenance.approval_chain[0].spec_ref,
            "026-federation-registry-routing"
        );
        assert!(federation.governance_decisions().iter().any(|decision| {
            decision.decision_kind == GovernanceDecisionKind::SnapshotAcceptance
                && decision.provenance.is_some()
                && decision.peer_id == "peer-governed"
        }));
    }

    #[test]
    fn route_denies_private_snapshot_without_private_scope_authority() {
        let mut local_capabilities = CapabilityRegistry::new();
        seed_capabilities(&mut local_capabilities);

        let mut local_events = EventRegistry::new();
        seed_events(&mut local_events);
        let mut local_workflows = WorkflowRegistry::new();
        seed_workflows(&mut local_workflows, &local_capabilities);

        let target_peer = peer("peer-private-target", "Peer Private Target");
        let target_trust = trust(
            "peer-private-target",
            vec![RegistryScope::Public, RegistryScope::Private],
        );
        let export = export_peer_state(
            target_peer.clone(),
            target_trust.clone(),
            &local_capabilities,
            &local_events,
            &local_workflows,
        );

        let mut federation = FederationRegistry::new();
        federation
            .register_peer(target_peer, target_trust)
            .expect("target peer should register");
        federation
            .sync_peer(
                export,
                &local_capabilities,
                &local_events,
                &local_workflows,
                "2026-04-09T21:20:00Z",
                "2026-04-09T21:21:00Z",
                "evidence:private-sync",
            )
            .expect("sync should succeed");

        let origin_peer = peer("peer-public-origin", "Peer Public Origin");
        let origin_trust = trust("peer-public-origin", vec![RegistryScope::Public]);
        federation
            .register_peer(origin_peer, origin_trust)
            .expect("origin peer should register");

        let failure = federation
            .route_capability_invocation(
                "peer-public-origin",
                "federation.capability.private-echo",
                "1.0.0",
                "request:private-denial",
                &BTreeSet::from([String::from("peer-private-target")]),
                "2026-04-09T21:22:00Z",
                "evidence:private-denial",
            )
            .expect_err("private capability should be denied for public-only origin");

        assert!(failure.errors[0].message.contains("PeerTrusted visibility"));
        assert!(federation.governance_decisions().iter().any(|decision| {
            decision.decision_kind == GovernanceDecisionKind::InvocationDenial
                && decision.trust_level == FederationTrustLevel::PeerTrusted
                && decision.evidence_ref == "evidence:private-denial"
                && decision.provenance.is_some()
        }));
    }

    #[test]
    fn route_capability_invocation_denies_private_snapshots_with_explicit_evidence() {
        let mut federation = FederationRegistry::new();
        let origin_peer = peer("peer-private-route", "Peer Private Route");
        let origin_trust = trust("peer-private-route", vec![RegistryScope::Public]);
        federation
            .register_peer(origin_peer, origin_trust)
            .expect("origin peer should register");

        let target_peer = peer("peer-private-owner", "Peer Private Owner");
        let target_trust = trust(
            "peer-private-owner",
            vec![RegistryScope::Public, RegistryScope::Private],
        );
        federation
            .register_peer(target_peer.clone(), target_trust.clone())
            .expect("target peer should register");

        let mut local_capabilities = CapabilityRegistry::new();
        let mut local_events = EventRegistry::new();
        let mut local_workflows = WorkflowRegistry::new();
        seed_capabilities(&mut local_capabilities);
        seed_events(&mut local_events);
        seed_workflows(&mut local_workflows, &local_capabilities);

        let export = export_peer_state(
            target_peer,
            target_trust,
            &local_capabilities,
            &local_events,
            &local_workflows,
        );
        federation
            .sync_peer(
                export,
                &local_capabilities,
                &local_events,
                &local_workflows,
                "2026-04-16T21:30:00Z",
                "2026-04-16T21:31:00Z",
                "evidence:sync-private-route",
            )
            .expect("sync should succeed");

        let failure = federation
            .route_capability_invocation(
                "peer-private-route",
                "federation.capability.private-echo",
                "1.0.0",
                "request:private-route",
                &BTreeSet::from([String::from("peer-private-owner")]),
                "2026-04-16T21:32:00Z",
                "evidence:private-route-denied",
            )
            .expect_err("private snapshot should be denied for unauthorized origin peers");

        assert!(
            failure.errors[0]
                .message
                .contains("requested capability requires PeerTrusted visibility")
        );
        assert!(federation.conflicts().iter().any(|conflict| {
            conflict.entry_key == "Capability:federation.capability.private-echo@1.0.0"
                && conflict.audit_ref == "evidence:private-route-denied"
                && conflict
                    .conflict_reason
                    .contains("not authorized to view the target snapshot")
        }));
    }

    #[test]
    fn status_summary_counts_sync_and_route_failures() {
        let mut federation = FederationRegistry::new();
        federation.peers.insert(
            "peer-summary".to_string(),
            peer("peer-summary", "Peer Summary"),
        );
        federation.sync_sessions.push(FederationSyncSession {
            session_id: "sync_peer-summary_1".to_string(),
            peer_id: "peer-summary".to_string(),
            started_at: "2026-04-09T22:00:00Z".to_string(),
            finished_at: Some("2026-04-09T22:01:00Z".to_string()),
            status: FederationSyncStatus::Partial,
            registry_types: vec![FederationRegistryKind::Capability],
            validated_entries: 1,
            rejected_entries: 2,
            conflict_count: 2,
            evidence_ref: "evidence:summary".to_string(),
        });
        federation.invocations.push(FederatedInvocation {
            invocation_id: "invocation_peer-summary_federation.capability.echo_1.0.0".to_string(),
            origin_peer_id: "peer-summary".to_string(),
            target_peer_id: "peer-target".to_string(),
            capability_id: "federation.capability.echo".to_string(),
            request_ref: "request:summary".to_string(),
            status: FederationInvocationStatus::Failure,
            response_ref: None,
            trace_provenance: CrossPeerTraceProvenance {
                trace_id: "trace_peer-summary_federation.capability.echo_1.0.0".to_string(),
                origin_peer_id: "peer-summary".to_string(),
                owning_peer_id: "peer-target".to_string(),
                trust_level: FederationTrustLevel::PubliclyTrusted,
                route_reason: "test route".to_string(),
                sync_session_ref: None,
                response_status: FederationInvocationStatus::Failure,
                evidence_ref: "evidence:summary-route".to_string(),
            },
        });
        federation.invocations.push(FederatedInvocation {
            invocation_id: "invocation_peer-summary_federation.capability.echo_1.0.1".to_string(),
            origin_peer_id: "peer-summary".to_string(),
            target_peer_id: "peer-target".to_string(),
            capability_id: "federation.capability.echo".to_string(),
            request_ref: "request:summary-retryable".to_string(),
            status: FederationInvocationStatus::RetryableFailure,
            response_ref: None,
            trace_provenance: CrossPeerTraceProvenance {
                trace_id: "trace_peer-summary_federation.capability.echo_1.0.1".to_string(),
                origin_peer_id: "peer-summary".to_string(),
                owning_peer_id: "peer-target".to_string(),
                trust_level: FederationTrustLevel::PubliclyTrusted,
                route_reason: "retryable route".to_string(),
                sync_session_ref: None,
                response_status: FederationInvocationStatus::RetryableFailure,
                evidence_ref: "evidence:summary-route-retryable".to_string(),
            },
        });

        let summary = federation.status_summary();
        assert_eq!(summary.peer_count, 1);
        assert_eq!(summary.trusted_peer_count, 1);
        assert_eq!(summary.last_sync_outcome, FederationSyncStatus::Partial);
        assert_eq!(summary.blocked_entries, 2);
        assert_eq!(summary.route_failures, 2);
    }

    #[test]
    fn approval_state_from_lifecycle_covers_all_states() {
        assert_eq!(
            approval_state_from_lifecycle(&Lifecycle::Draft),
            FederationApprovalState::Draft
        );
        assert_eq!(
            approval_state_from_lifecycle(&Lifecycle::Active),
            FederationApprovalState::Approved
        );
        assert_eq!(
            approval_state_from_lifecycle(&Lifecycle::Deprecated),
            FederationApprovalState::Deprecated
        );
        assert_eq!(
            approval_state_from_lifecycle(&Lifecycle::Retired),
            FederationApprovalState::Rejected
        );
        assert_eq!(
            approval_state_from_lifecycle(&Lifecycle::Archived),
            FederationApprovalState::Rejected
        );
    }

    #[test]
    fn highest_authorized_trust_level_covers_all_paths() {
        assert_eq!(
            highest_authorized_trust_level(&trust("peer-private", vec![RegistryScope::Private])),
            FederationTrustLevel::PeerTrusted
        );
        assert_eq!(
            highest_authorized_trust_level(&trust("peer-public", vec![RegistryScope::Public])),
            FederationTrustLevel::PubliclyTrusted
        );
        assert_eq!(
            highest_authorized_trust_level(&TrustRecord {
                peer_id: "peer-local".to_string(),
                trust_model: "local-only".to_string(),
                allowed_scopes: Vec::new(),
                approved_spec_refs: vec!["026-federation-registry-routing".to_string()],
                approved_at: "2026-04-09T19:30:00Z".to_string(),
                revoked_at: None,
            }),
            FederationTrustLevel::LocalOnly
        );
    }

    #[test]
    fn is_route_failure_covers_failure_variants() {
        assert!(!is_route_failure(FederationInvocationStatus::Success));
        assert!(is_route_failure(FederationInvocationStatus::Failure));
        assert!(is_route_failure(
            FederationInvocationStatus::RetryableFailure
        ));
    }

    fn peer(peer_id: &str, display_name: &str) -> FederationPeer {
        FederationPeer {
            peer_id: peer_id.to_string(),
            display_name: display_name.to_string(),
            trust_state: FederationTrustState::Trusted,
            identity_fingerprint: format!("fingerprint:{peer_id}"),
            sync_enabled: true,
            last_sync_at: None,
            last_sync_status: FederationSyncStatus::Unknown,
            visible_registry_scopes: vec![RegistryScope::Public, RegistryScope::Private],
        }
    }

    fn trust(peer_id: &str, scopes: Vec<RegistryScope>) -> TrustRecord {
        TrustRecord {
            peer_id: peer_id.to_string(),
            trust_model: "shared-api-token".to_string(),
            allowed_scopes: scopes,
            approved_spec_refs: vec!["026-federation-registry-routing".to_string()],
            approved_at: "2026-04-09T19:30:00Z".to_string(),
            revoked_at: None,
        }
    }

    fn seed_capabilities(registry: &mut CapabilityRegistry) {
        registry
            .register(capability_registration(
                RegistryScope::Public,
                capability_contract(),
            ))
            .expect("capability should register");
        registry
            .register(capability_registration(
                RegistryScope::Private,
                private_capability_contract(),
            ))
            .expect("private capability should register");
    }

    fn seed_events(registry: &mut EventRegistry) {
        registry
            .register(event_registration(RegistryScope::Public, event_contract()))
            .expect("event should register");
    }

    fn seed_workflows(registry: &mut WorkflowRegistry, capabilities: &CapabilityRegistry) {
        registry
            .register(
                capabilities,
                workflow_registration(RegistryScope::Public, workflow_definition()),
            )
            .expect("workflow should register");
    }

    fn capability_contract() -> CapabilityContract {
        CapabilityContract {
            kind: "capability_contract".to_string(),
            schema_version: "1.0.0".to_string(),
            id: "federation.capability.echo".to_string(),
            namespace: "federation.capability".to_string(),
            name: "echo".to_string(),
            version: "1.0.0".to_string(),
            lifecycle: Lifecycle::Active,
            owner: Owner {
                team: "platform".to_string(),
                contact: "platform@example.com".to_string(),
            },
            summary: "Echo a federated capability call.".to_string(),
            description: "End-to-end federation test capability.".to_string(),
            inputs: SchemaContainer {
                schema: json!({"type":"object"}),
            },
            outputs: SchemaContainer {
                schema: json!({"type":"object"}),
            },
            preconditions: vec![],
            postconditions: vec![],
            side_effects: vec![SideEffect {
                kind: SideEffectKind::EventEmission,
                description: "Emit routing evidence for federation sync.".to_string(),
            }],
            emits: vec![EventReference {
                event_id: "federation.event.routed".to_string(),
                version: "1.0.0".to_string(),
            }],
            consumes: vec![],
            permissions: vec![],
            execution: traverse_contracts::Execution {
                binary_format: traverse_contracts::BinaryFormat::Wasm,
                entrypoint: Entrypoint {
                    kind: EntrypointKind::WasiCommand,
                    command: "echo".to_string(),
                },
                preferred_targets: vec![traverse_contracts::ExecutionTarget::Local],
                constraints: traverse_contracts::ExecutionConstraints {
                    host_api_access: traverse_contracts::HostApiAccess::None,
                    filesystem_access: traverse_contracts::FilesystemAccess::None,
                    network_access: traverse_contracts::NetworkAccess::Forbidden,
                },
            },
            policies: vec![],
            dependencies: vec![],
            provenance: traverse_contracts::Provenance {
                source: traverse_contracts::ProvenanceSource::Greenfield,
                author: "enricopiovesan".to_string(),
                created_at: "2026-04-09T19:00:00Z".to_string(),
                spec_ref: Some("026-federation-registry-routing".to_string()),
                adr_refs: vec![],
                exception_refs: vec![],
            },
            evidence: vec![],
            service_type: traverse_contracts::ServiceType::Stateless,
            permitted_targets: vec![
                traverse_contracts::ExecutionTarget::Local,
                traverse_contracts::ExecutionTarget::Browser,
                traverse_contracts::ExecutionTarget::Edge,
                traverse_contracts::ExecutionTarget::Cloud,
                traverse_contracts::ExecutionTarget::Worker,
                traverse_contracts::ExecutionTarget::Device,
            ],
            event_trigger: None,
            connector_requirements: Vec::new(),
            state_schema: None,
        }
    }

    fn private_capability_contract() -> CapabilityContract {
        let mut contract = capability_contract();
        contract.id = "federation.capability.private-echo".to_string();
        contract.name = "private-echo".to_string();
        contract.summary = "Private federated echo.".to_string();
        contract
    }

    fn event_contract() -> EventContract {
        EventContract {
            kind: "event_contract".to_string(),
            schema_version: "1.0.0".to_string(),
            id: "federation.event.routed".to_string(),
            namespace: "federation.event".to_string(),
            name: "routed".to_string(),
            version: "1.0.0".to_string(),
            lifecycle: Lifecycle::Active,
            owner: Owner {
                team: "platform".to_string(),
                contact: "platform@example.com".to_string(),
            },
            summary: "A federation routing event.".to_string(),
            description: "End-to-end federation event.".to_string(),
            payload: EventPayload {
                schema: json!({"type":"object"}),
                compatibility: PayloadCompatibility::BackwardCompatible,
            },
            classification: EventClassification {
                domain: "federation".to_string(),
                bounded_context: "registry".to_string(),
                event_type: EventType::System,
                tags: vec!["federation".to_string()],
            },
            publishers: vec![traverse_contracts::CapabilityReference {
                capability_id: "federation.capability.echo".to_string(),
                version: "1.0.0".to_string(),
            }],
            subscribers: vec![traverse_contracts::CapabilityReference {
                capability_id: "federation.capability.private-echo".to_string(),
                version: "1.0.0".to_string(),
            }],
            policies: vec![],
            tags: vec!["federation".to_string()],
            provenance: EventProvenance {
                source: EventProvenanceSource::Greenfield,
                author: "enricopiovesan".to_string(),
                created_at: "2026-04-09T19:00:00Z".to_string(),
            },
            evidence: vec![],
        }
    }

    fn workflow_definition() -> WorkflowDefinition {
        WorkflowDefinition {
            kind: "workflow_definition".to_string(),
            schema_version: "1.0.0".to_string(),
            id: "federation.workflow.route".to_string(),
            name: "route".to_string(),
            version: "1.0.0".to_string(),
            lifecycle: Lifecycle::Active,
            owner: Owner {
                team: "platform".to_string(),
                contact: "platform@example.com".to_string(),
            },
            summary: "A federated routing workflow.".to_string(),
            inputs: SchemaContainer {
                schema: json!({"type":"object"}),
            },
            outputs: SchemaContainer {
                schema: json!({"type":"object"}),
            },
            nodes: vec![WorkflowNode {
                node_id: "route-node".to_string(),
                capability_id: "federation.capability.echo".to_string(),
                capability_version: "1.0.0".to_string(),
                input: WorkflowNodeInput {
                    from_workflow_input: vec!["request".to_string()],
                },
                output: WorkflowNodeOutput {
                    to_workflow_state: vec!["response".to_string()],
                },
            }],
            edges: vec![],
            start_node: "route-node".to_string(),
            terminal_nodes: vec!["route-node".to_string()],
            tags: vec!["federation".to_string()],
            governing_spec: "007-workflow-registry-traversal".to_string(),
        }
    }

    fn capability_registration(
        scope: RegistryScope,
        contract: CapabilityContract,
    ) -> CapabilityRegistration {
        CapabilityRegistration {
            scope,
            contract_path: format!(
                "registry/{}/{}/{}{}",
                scope_name(scope),
                contract.id,
                contract.version,
                "/contract.json"
            ),
            artifact: CapabilityArtifactRecord {
                artifact_ref: format!("artifact:{}:{}", contract.name, contract.version),
                implementation_kind: ImplementationKind::Executable,
                source: SourceReference {
                    kind: SourceKind::Git,
                    location: format!("https://example.invalid/{}", contract.name),
                },
                binary: Some(BinaryReference {
                    format: BinaryFormat::Wasm,
                    location: format!("artifacts/{}/{}.wasm", contract.name, contract.version),
                    signature: None,
                }),
                workflow_ref: None,
                digests: ArtifactDigests {
                    source_digest: format!("source:{}:{}", contract.name, contract.version),
                    binary_digest: Some(format!("binary:{}:{}", contract.name, contract.version)),
                },
                provenance: RegistryProvenance {
                    source: "greenfield".to_string(),
                    author: "enricopiovesan".to_string(),
                    created_at: "2026-04-09T19:00:00Z".to_string(),
                },
            },
            registered_at: "2026-04-09T19:00:00Z".to_string(),
            tags: vec!["federation".to_string()],
            composability: ComposabilityMetadata {
                kind: CompositionKind::Atomic,
                patterns: vec![CompositionPattern::Sequential],
                provides: vec!["federation".to_string()],
                requires: vec!["registry".to_string()],
            },
            governing_spec: "005-capability-registry".to_string(),
            validator_version: "registry-test".to_string(),
            contract,
        }
    }

    fn event_registration(
        scope: RegistryScope,
        contract: EventContract,
    ) -> crate::EventRegistration {
        crate::EventRegistration {
            scope,
            contract,
            contract_path: format!(
                "registry/{}/{}/{}{}",
                scope_name(scope),
                "federation.event.routed",
                "1.0.0",
                "/contract.json"
            ),
            registered_at: "2026-04-09T19:00:00Z".to_string(),
            governing_spec: "011-event-registry".to_string(),
            validator_version: "registry-test".to_string(),
        }
    }

    fn workflow_registration(
        scope: RegistryScope,
        definition: WorkflowDefinition,
    ) -> WorkflowRegistration {
        WorkflowRegistration {
            scope,
            definition,
            workflow_path: "registry/public/federation.workflow.route/1.0.0/workflow.json"
                .to_string(),
            registered_at: "2026-04-09T19:00:00Z".to_string(),
            validator_version: "registry-test".to_string(),
        }
    }

    fn scope_name(scope: RegistryScope) -> &'static str {
        match scope {
            RegistryScope::Public => "public",
            RegistryScope::Private => "private",
        }
    }

    fn sync_with_unapproved_capability_spec() -> FederationSyncOutcome {
        let mut federation = FederationRegistry::new();
        let peer = peer("peer-unapproved-capability", "Peer Unapproved Capability");
        let trust = trust(
            "peer-unapproved-capability",
            vec![RegistryScope::Public, RegistryScope::Private],
        );
        federation
            .register_peer(peer.clone(), trust.clone())
            .expect("peer should register");

        let mut remote_capabilities = CapabilityRegistry::new();
        remote_capabilities
            .register(capability_registration(
                RegistryScope::Public,
                capability_contract(),
            ))
            .expect("capability should register");

        let mut capability_export = export_peer_state(
            peer,
            trust,
            &remote_capabilities,
            &EventRegistry::new(),
            &WorkflowRegistry::new(),
        );
        capability_export.capabilities[0]
            .record
            .evidence
            .governing_spec = "999-unapproved-spec".to_string();

        federation
            .sync_peer(
                capability_export,
                &CapabilityRegistry::new(),
                &EventRegistry::new(),
                &WorkflowRegistry::new(),
                "2026-04-16T14:00:00Z",
                "2026-04-16T14:01:00Z",
                "evidence:unapproved-capability",
            )
            .expect("unapproved capability spec should report a conflict")
    }

    fn sync_with_unapproved_event_spec() -> FederationSyncOutcome {
        let mut federation = FederationRegistry::new();
        let peer = peer("peer-unapproved-event", "Peer Unapproved Event");
        let trust = trust(
            "peer-unapproved-event",
            vec![RegistryScope::Public, RegistryScope::Private],
        );
        federation
            .register_peer(peer.clone(), trust.clone())
            .expect("peer should register");

        let mut remote_events = EventRegistry::new();
        remote_events
            .register(event_registration(RegistryScope::Public, event_contract()))
            .expect("event should register");

        let mut event_export = export_peer_state(
            peer,
            trust,
            &CapabilityRegistry::new(),
            &remote_events,
            &WorkflowRegistry::new(),
        );
        event_export.events[0]
            .record
            .validation_evidence
            .governing_spec = "999-unapproved-spec".to_string();

        federation
            .sync_peer(
                event_export,
                &CapabilityRegistry::new(),
                &EventRegistry::new(),
                &WorkflowRegistry::new(),
                "2026-04-16T14:02:00Z",
                "2026-04-16T14:03:00Z",
                "evidence:unapproved-event",
            )
            .expect("unapproved event spec should report a conflict")
    }

    fn sync_with_unapproved_workflow_spec() -> FederationSyncOutcome {
        let mut federation = FederationRegistry::new();
        let peer = peer("peer-unapproved-workflow", "Peer Unapproved Workflow");
        let trust = trust(
            "peer-unapproved-workflow",
            vec![RegistryScope::Public, RegistryScope::Private],
        );
        federation
            .register_peer(peer.clone(), trust.clone())
            .expect("peer should register");

        let mut workflow_capabilities = CapabilityRegistry::new();
        seed_capabilities(&mut workflow_capabilities);
        let mut remote_workflows = WorkflowRegistry::new();
        remote_workflows
            .register(
                &workflow_capabilities,
                workflow_registration(RegistryScope::Public, workflow_definition()),
            )
            .expect("workflow should register");

        let mut workflow_export = export_peer_state(
            peer,
            trust,
            &workflow_capabilities,
            &EventRegistry::new(),
            &remote_workflows,
        );
        workflow_export.workflows[0].record.governing_spec = "999-unapproved-spec".to_string();

        federation
            .sync_peer(
                workflow_export,
                &workflow_capabilities,
                &EventRegistry::new(),
                &WorkflowRegistry::new(),
                "2026-04-16T14:04:00Z",
                "2026-04-16T14:05:00Z",
                "evidence:unapproved-workflow",
            )
            .expect("unapproved workflow spec should report a conflict")
    }
}
