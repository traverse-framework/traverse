# Feature Specification: Federated Traverse registry, routing, and trust model

**Feature Branch**: `078-federation-registry-routing`
**Created**: 2026-04-09
**Approved**: 2026-07-21
**Status**: Approved
**Input**: User description: "Create the governing federation spec from the long-term end-to-end decisions, with future work split into separate tickets."

## Provenance

This spec was originally authored as `026-federation-registry-routing` on
2026-04-09 and marked "Status: Approved" in its own header, but the document
was only ever committed to an abandoned branch
(`origin/022-mcp-wasm-server`, commit `b81a17b`) and was never merged into
`specs/governance/approved-specs.json`. The federation implementation
(`crates/traverse-registry/src/federation.rs`, `crates/traverse-cli/src/
federation_operator.rs`, landed via PR #240 and follow-ons #247–#250)
nonetheless shipped referencing `026-federation-registry-routing` as an
approved spec ID in its test fixtures, on the mistaken assumption that this
document had been formally approved. It had not, and the numeric slot `026`
was later assigned to an unrelated spec (`026-event-broker`, issue #207)
during the v0.2.0 governance batch, permanently orphaning the original ID.

The content below is the original document, reviewed against the shipped
implementation and found to still accurately describe it, renumbered and
landed under `078` to close the governance gap. The original's "Governing
Relationship" section referenced open issue numbers and two sibling specs
(`020-downstream-integration-validation`, `021-app-facing-operational-
constraints`) that are themselves still unapproved drafts in this repo;
those unverifiable references have been removed rather than carried forward
uncritically — see the trimmed "Governing Relationship" section below.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Register and Sync Trusted Peers (Priority: P1)

As a Traverse operator, I want to register trusted peers and manually sync registry state so that distributed Traverse instances can discover each other and share governed registry entries without a central coordinator.

**Why this priority**: Peer registration and sync are the foundation of federation; nothing else in the federation chain is useful if peers cannot establish trusted shared state.

**Independent Test**: A reviewer can verify that a peer can be added, listed, synced on demand, and reported as healthy or unhealthy without needing to inspect implementation code.

**Acceptance Scenarios**:

1. **Given** two trusted Traverse peers, **When** one peer is registered and a manual sync is run, **Then** the local instance records the remote peer, sync outcome, and the discovered registry entries.
2. **Given** a remote entry fails trust or spec validation, **When** sync runs, **Then** the entry is rejected and the sync evidence reports the reason.

### User Story 2 - Discover and Route Cross-Peer Invocations (Priority: P1)

As a downstream consumer or agent, I want a capability discovered on a remote peer to be invoked through the owning peer so that federation is useful as a real end-to-end execution path, not just a registry mirror.

**Why this priority**: Federation must deliver a real cross-peer call path; otherwise it only proves discovery and state replication.

**Independent Test**: A reviewer can verify that a discovered remote capability is routed to its owning peer, that the result or failure is returned, and that trace provenance is preserved.

**Acceptance Scenarios**:

1. **Given** a capability is registered on peer B and synced to peer A, **When** peer A invokes that capability, **Then** the request is routed to peer B and the response is returned with cross-peer provenance.
2. **Given** the owning peer is unavailable, **When** peer A invokes the remote capability, **Then** the failure is explicit, retryable if policy allows, and the trace records the reason.

### User Story 3 - Trust, Visibility, and Auditability Across Peers (Priority: P2)

As a federation steward, I want remote registry entries to be validated against local approved specs and tracked with audit evidence so that federation remains governable and explainable over time.

**Why this priority**: The federation model needs a durable trust story and reviewable evidence so the system can grow without becoming opaque.

**Independent Test**: A reviewer can confirm that remote entries are validated before acceptance, peer visibility is exposed, and conflicts or rejections produce audit evidence.

**Acceptance Scenarios**:

1. **Given** a remote registry entry from a trusted peer, **When** it is synced, **Then** the entry is validated against the local approved spec registry before acceptance.
2. **Given** a sync detects a divergent or conflicting entry, **When** the sync completes, **Then** the conflict is reported with audit evidence instead of being silently hidden.
3. **Given** an operator inspects federation state, **When** they query peers and sync status, **Then** the current peer list, sync outcomes, and trust state are visible through the supported operator surface.

## Scope

In scope:

- peer-to-peer federation with no central coordinator in the first governed slice
- trusted peer registration and peer identity
- manual on-demand synchronization of registry state
- federated discovery for capability, event, and workflow registries
- public/private visibility rules across trusted peers, with explicit authorization for private entries
- routed cross-peer invocation to the owning peer
- explicit trust and validation of remote entries against local approved specs
- peer listing, sync status, and audit evidence for operators and agents
- cross-peer trace provenance for routed invocations
- explicit conflict reporting when sync detects divergence
- a governed first transport path that can be implemented and validated without guessing at the federation boundary

Out of scope:

- a central federation coordinator
- automatic background sync after registration
- streaming sync transport
- gossip-based replication
- load-balanced invocation to any eligible peer
- transport protocols beyond the first governed peer-sync path

## Edge Cases

- A peer presents a registry entry that is valid structurally but not approved locally.
- A remote peer is reachable for discovery but unavailable for invocation.
- Two peers present the same `(scope, id, version)` entry with different provenance.
- A peer is trusted for discovery but not trusted for private capability visibility.
- A sync completes successfully for some registry types but not others.
- An invocation is routed successfully but the remote peer returns a structured failure.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: Traverse MUST support trusted peer registration for federation.
- **FR-002**: Traverse MUST support manual on-demand synchronization of registry state between peers.
- **FR-003**: Traverse MUST expose peer listing and sync status to operators and agents.
- **FR-004**: Traverse MUST validate remote registry entries against the local approved spec registry before accepting them.
- **FR-005**: Traverse MUST federate capability, event, and workflow registry entries through the first governed sync path.
- **FR-006**: Traverse MUST route a discovered remote capability invocation to the owning peer.
- **FR-007**: Traverse MUST preserve cross-peer trace provenance for routed invocations.
- **FR-008**: Traverse MUST produce explicit failure evidence when the owning peer is unavailable.
- **FR-009**: Traverse MUST report conflicts or divergent registry entries with audit evidence.
- **FR-010**: Traverse MUST preserve public/private visibility rules across peers and deny unauthorized access to private registry entries.
- **FR-011**: Approved implementation under this slice MUST be checked against the governing federation spec before merge.
- **FR-012**: Future federation extensions beyond this slice MUST be split into separate tickets rather than expanded in place.

### Key Entities *(include if feature involves data)*

- **FederationPeer**: A trusted Traverse instance participating in federation, identified by peer identity and trust metadata.
- **PeerRegistrySnapshot**: A synchronized view of the capability, event, and workflow registry entries received from a peer.
- **FederationSyncSession**: A single manual sync operation with outcome, timing, and validation evidence.
- **FederatedInvocation**: A routed request sent to the owning peer and the resulting response or failure.
- **TrustRecord**: The approved relationship that allows a peer to participate in federation.
- **ConflictRecord**: Audit evidence that records divergent or conflicting registry entries.
- **CrossPeerTraceProvenance**: Evidence showing which peer owned the invocation and how the result returned.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A reviewer can register a peer, run a manual sync, and verify the peer and sync outcome without reading implementation code.
- **SC-002**: A discovered remote capability can be invoked through the owning peer and return a recorded response or explicit failure.
- **SC-003**: Remote entries are accepted only after local approved-spec validation, and rejected entries produce reviewable evidence.
- **SC-004**: Peer list, sync status, and conflict evidence are visible through the supported operator surface.
- **SC-005**: Cross-peer provenance is present in routed invocation evidence and can be reviewed after the call completes.

## Assumptions

- The first governed federation slice is peer-to-peer and does not require a central coordinator.
- The first supported sync path is manual and on-demand.
- The first supported transport path is explicit and reviewable rather than inferred from internal implementation structure.
- Future improvements such as automatic sync, streaming updates, and central coordination will be tracked as separate tickets.
- The federation model must stay compatible with the approved spec registry and the existing runtime/tracing model.

## Governing Relationship

This specification governs `crates/traverse-registry/src/federation.rs` and
`crates/traverse-cli/src/federation_operator.rs`.

This specification is governed by:

- `001-foundation-v0-1`
- `019-downstream-consumer-contract`

Coordinator behavior, automatic sync, streaming updates, and other
nonessential federation extensions remain future, separately governed work.
