# Data Model: Federated Traverse registry, routing, and trust model (spec 078)

## FederationPeer

Represents a trusted Traverse instance participating in federation.

### Fields

- `peer_id`: stable peer identifier
- `display_name`: human-readable peer label
- `trust_state`: trusted, blocked, pending, or revoked
- `identity_fingerprint`: certificate or trust-anchor fingerprint
- `sync_enabled`: whether the peer participates in sync
- `last_sync_at`: timestamp of the most recent sync
- `last_sync_status`: success, partial, failed, or unknown
- `visible_registry_scopes`: which registry scopes this peer may see

## TrustRecord

Represents the approved relationship that allows a peer to participate in federation.

### Fields

- `peer_id`: referenced FederationPeer
- `trust_model`: how the peer is trusted
- `allowed_scopes`: scopes or registry classes the peer may access
- `approved_spec_refs`: governing spec references that authorize trust
- `approved_at`: timestamp of approval
- `revoked_at`: optional timestamp if trust is revoked

## FederationSyncSession

Represents one manual sync attempt.

### Fields

- `session_id`: unique sync session identifier
- `peer_id`: peer being synced
- `started_at`: sync start time
- `finished_at`: sync end time
- `status`: success, partial, failed
- `registry_types`: capability, event, workflow, or a subset
- `validated_entries`: number of accepted entries
- `rejected_entries`: number of rejected entries
- `conflict_count`: number of conflicts detected
- `evidence_ref`: pointer to sync evidence or audit artifact

## PeerRegistrySnapshot

Represents the remote registry state accepted from a peer.

### Fields

- `peer_id`: source peer
- `registry_type`: capability, event, or workflow
- `entry_id`: stable registry entry id
- `version`: semver or equivalent version identifier
- `scope`: public or private
- `approval_state`: approved, draft, deprecated, or rejected
- `contract_ref`: reference to the governing contract artifact
- `provenance_ref`: origin metadata for the remote entry

## FederatedInvocation

Represents a routed request sent to the owning peer.

### Fields

- `invocation_id`: stable routed invocation id
- `origin_peer_id`: peer initiating the invocation
- `target_peer_id`: owning peer
- `capability_id`: requested capability
- `request_ref`: governing request or payload reference
- `status`: success, failure, retryable_failure
- `response_ref`: response or failure payload reference
- `trace_provenance_ref`: provenance for the routed trace

## ConflictRecord

Represents a detected divergence between peers.

### Fields

- `conflict_id`: stable conflict identifier
- `peer_ids`: peers involved in the divergence
- `registry_type`: capability, event, or workflow
- `entry_key`: conflicting scope/id/version key
- `conflict_reason`: human-readable explanation
- `resolution_state`: open, resolved, escalated
- `audit_ref`: evidence pointer for review

## CrossPeerTraceProvenance

Represents the audit trail for a routed invocation across peers.

### Fields

- `trace_id`: governing trace id
- `origin_peer_id`: peer that initiated the request
- `owning_peer_id`: peer that executed the request
- `route_reason`: why the owning peer was selected
- `sync_session_ref`: federation sync evidence ref if relevant
- `response_status`: success or failure outcome
- `evidence_ref`: reviewable evidence pointer

## FederationStatusSummary

Represents the operator-facing summary for federation health.

### Fields

- `peer_count`: number of trusted peers
- `last_sync_outcome`: latest sync result
- `sync_age`: age since last successful sync
- `conflict_count`: current unresolved conflicts
- `blocked_entries`: number of entries rejected by trust or spec validation
- `route_failures`: number of failed cross-peer invocations

## Relationships

- A `FederationPeer` can have many `TrustRecord` entries.
- A `FederationPeer` can have many `FederationSyncSession` records.
- A `FederationSyncSession` can produce many `PeerRegistrySnapshot` entries.
- A `FederatedInvocation` references exactly one origin peer and one owning peer.
- A `ConflictRecord` can reference many peers and many rejected registry entries.
- A `CrossPeerTraceProvenance` record can be attached to a `FederatedInvocation`.

## Validation Rules

- A peer without a valid `TrustRecord` cannot participate in federation.
- A `PeerRegistrySnapshot` must match an approved spec or be rejected.
- A `FederatedInvocation` must always identify the owning peer.
- A `ConflictRecord` must retain an audit reference.
- A `CrossPeerTraceProvenance` record must not omit origin and owning peer ids.
