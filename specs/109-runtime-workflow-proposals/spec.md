# Runtime Workflow Proposals and Authorization (P1)

**Status**: Approved
**Canonical governing ID**: `109-runtime-workflow-proposals`  
**Version**: 1.1.0
**Implements phase**: P1 of `108-governed-runtime-workflow-composition`  
**Input**: Issue #1089; decisions on #865.

**Amendment (approved 2026-11-20, version 1.0.0 -> 1.1.0, Decision 136 / #1896):**
approval for an agent-originated proposal is a host-owned, out-of-band action.
The initial host surface is a local CLI prompt. MCP can submit targets and
read plans/proposals, but cannot mint or approve a token.

## Purpose

Define a provider-neutral MCP lifecycle for an ephemeral, manifest-constrained
workflow proposal that executes as a bounded sequential DAG.

## Requirements

- **FR-001**: The public MCP surface MUST support discovery, compatibility
  inspection, proposal submission, validation feedback, authorization state,
  execution, observation, and proposal export; it MUST NOT require a
  Traverse-hosted LLM or planner credential.
- **FR-002**: A proposal MUST identify its app-manifest version/digest,
  workspace scope, capability IDs and versions, explicit directed edges, and
  explicit source/target JSON paths for every data mapping.
- **FR-003**: The runtime MUST canonicalize a proposal and bind its digest to
  pinned manifest, registry, binding, policy, and budget snapshots.
- **FR-004**: Validation MUST reject undeclared capabilities/connectors,
  incompatible schemas, invalid mappings, unavailable/yanked versions,
  disallowed placement, and any attempt to weaken manifest policy.
- **FR-005**: Capability contracts MUST independently declare effect class
  (`pure_read`, `state_write`, `external_effect`, `irreversible_effect`),
  determinism class (`deterministic`, `externally_variable`,
  `model_derived`), field-level accepted/produced data classifications and
  egress policy, plus reliability metadata (idempotency requirement,
  retryability, and compensation availability). A manifest may only tighten
  these requirements.
- **FR-006**: A valid proposal containing only automatic-eligible effect,
  data-flow, reliability, and budget classes MAY run automatically. Private connectors/secrets,
  side-effecting operations, or policy/budget exceptions MUST require an
  approval token bound to proposal digest, manifest identity/version,
  workspace scope, and expiry.
- **FR-006a**: An approval token MUST have a verified issuer, key identity,
  audience, authenticated approver/principal, tenant/workspace scope, exact
  proposal and snapshot digest, permitted effects/connectors, maximum use
  count, expiry, revocation behavior, and replay prevention.
- **FR-006b**: A host MUST render the proposal in an out-of-band approval
  surface and mint a signed, single-use token only after authenticated human
  confirmation. The initial surface is a local CLI prompt. No MCP command,
  client callback, or agent may mint, approve, or clear an approval token.
- **FR-006c**: Before executing any authorized side effect, the host MUST
  verify the token's signature, proposal and snapshot digests, scope, expiry,
  and permitted authority, then atomically record its nonce as consumed. A
  missing, invalid, expired, revoked, scope-mismatched, digest-mismatched, or
  consumed token MUST fail closed with a stable secret-free code.
- **FR-007**: P1 MUST accept only acyclic graphs below configured node, edge,
  mapping, payload, validation-time, execution-time, and resource limits, and
  execute one node at a time in deterministic topological order.
- **FR-007a**: The proposal format MUST define canonical JSON/hash rules,
  exact resolved capability/artifact versions and digests, initial input
  bindings, unique node IDs, deterministic ready-node tie breaking, no
  ambiguous multi-writer target path, and policy-defined safe defaults.
- **FR-007b**: Validation MUST authenticate the caller and enforce per-
  principal, per-app, and per-workspace quotas/reservations for concurrency,
  wall time, payload/output bytes, and declared external-resource budgets.
- **FR-008**: The runtime MUST stop at the first failed node. It MUST NOT
  perform an implicit retry, compensation, graph mutation, or runtime catalog
  registration. A new execution is required for retry.
- **FR-008a**: P1 terminal states are `succeeded`, `failed`, `cancelled`,
  `expired`, and `authorization_revoked`. Side-effecting nodes MUST use a
  declared idempotency key; P1 provides neither exactly-once execution nor
  distributed transactional rollback.
- **FR-009**: The trace MUST retain a bounded redacted snapshot: graph digest,
  identities/versions/digests, mapping paths, authorization decision, budgets,
  per-node status, and terminal outcome. It MUST exclude prompts, secrets,
  private config, and arbitrary raw payload copies.
- **FR-010**: All denials and failures MUST use stable machine-readable codes
  and secret-free details.
- **FR-011**: Every explicit mapping MUST pass field-level data-flow policy:
  source classification, target acceptance, connector/model egress policy,
  and trace projection policy. Schema compatibility alone MUST NOT authorize
  data disclosure.

## Acceptance scenarios

1. A client proposes two manifest-declared read-only capabilities with a
   schema-valid explicit mapping; validation deterministically accepts and
   sequential execution produces an auditable redacted trace snapshot.
2. A proposal includes an external-side-effect capability; it cannot execute
   without a valid bound approval token.
3. A proposal maps an undeclared/private field or references a capability not
   allowed by the manifest; validation rejects before invocation.
4. A node fails after an earlier state write; no later node, retry, or
   compensation runs, and the trace distinguishes succeeded, failed, and
   not-started nodes.
5. Re-submitting pinned identical inputs produces the same proposal digest and
   validation decision.
6. A schema-compatible mapping of classified customer data into an external
   connector without declared egress authority is rejected before invocation.
7. An expired, revoked, replayed, or differently scoped approval token is
   rejected before side effects occur.
8. An MCP client can read an approval-required proposal but cannot mint a
   token; a local CLI prompt mints one token after human confirmation, and a
   second execution attempt with it is rejected as replayed.

## Out of scope

Parallel execution, cycles, event waits, durable resume, automatic retries,
sagas, direct registry mutation, and planner implementation.
