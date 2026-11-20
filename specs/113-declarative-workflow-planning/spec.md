# Declarative Workflow Planning — P0 of Governed Runtime Workflow Composition

**Status**: Approved
**Canonical governing ID**: `113-declarative-workflow-planning`
**Version**: 1.2.0
**Implements phase**: P0 of `108-governed-runtime-workflow-composition` (precedes the P1 proposal-submission surface)
**Input**: Decision 59 (`docs/decision-log.md`); traverse #1098; registry #305.

**Amendment (approved 2026-11-20, version 1.1.0 -> 1.2.0, Decision 136 / #1896):**
a failed approved workflow may be replanned only for its remaining work. Its
completed outputs enter planning as bounded, schema-validated references bound
to trace digests; hosts resolve authorized values locally and raw prior output
does not enter MCP or the planner.

**Amendment (2026-08-23, version 0.1.0 -> 0.2.0, approved 2026-08-23)**: implementing this
spec's real unblock condition (registry `#305`'s backfill) surfaced a direct conflict
with registry's own already-governed FR-020 capability inventory
(`contracts/governance/ecca-capability-inventory.json`, registry decision-log entry 57):
45 of registry's 46 unique published capability IDs — including every capability
involved in an already-published, human-reviewed `workflows/*.json` chain — are
classified `no-event-required`, with reviewed evidence that they are synchronous,
direct-return capabilities with no asynchronous domain fact to broadcast. FR-002's
original text asked the planner to chain on `consumes`/`emits` (`EventReference`)
linkage specifically, which — per registry's own `graph.rs` composition-graph builder
and its FR-020 classification criterion — has consistently meant a real, governed,
decoupled-subscriber domain event elsewhere in that repo, not "capability A's output
happens to feed capability B's input" (which `workflows/*.json` already models
separately, via explicit node edges and field-level mappings, with no `EventReference`
involved). Backfilling `consumes`/`emits` for capabilities registry's own governance
already, deliberately, classified as not needing an event would have repeated the
overclaiming pattern registry decision-log entries 61/64 exist to prevent.

Resolved via a live, owner-participated brainstorm (2026-08-23, cross-posted to
registry decision-log entry 68): the planner's chain-discovery signal changes from
declared `consumes`/`emits` linkage to structural `inputs.schema`/`outputs.schema`
compatibility — a signal the planner already computes for its per-edge field-mapping
step (FR-005), not a new one. This keeps `consumes`/`emits` meaning what it means
everywhere else in the org (a real governed async event) and removes this spec's
dependency on registry backfilling anything. FR-002 and FR-003 below reflect the
amended signal; Acceptance scenarios 1-3 updated to match. No other requirement
changed. Approved on creation per this org's existing precedent for a decision that
traces directly to a live, owner-participated brainstorm rather than an
agent-invented draft.

## Purpose

Define a deterministic planner that, given a structured target and the published
`inputs.schema`/`outputs.schema` of published capabilities, derives one
or more complete candidate workflow proposals — each already shaped to
satisfy `109-runtime-workflow-proposals` FR-002/FR-004/FR-011 (explicit
capability IDs/versions, explicit edges, explicit field-level JSON-path
mappings) — for a caller to review and submit through the existing P1 MCP
surface. This spec governs what generates a proposal's *content*; it does
not change how proposals are validated, authorized, or executed, which
remain entirely owned by `109`–`112`.

## Requirements

- **FR-001**: The planner MUST accept a structured target (a declared event
  type or capability ID the caller needs produced) and a starting fact set.
  It MUST NOT accept or interpret open-ended natural-language goals, and
  MUST NOT require or embed a hosted language model, matching FR-001 of
  spec `109`.
- **FR-002**: The planner MUST derive candidate chains only from capability pairs
  where the candidate producer's `outputs.schema` structurally satisfies the
  candidate consumer's `inputs.schema` — every property named in the consumer's
  `inputs.schema.required` MUST exist in the producer's `outputs.schema.properties`
  with a matching JSON type. A capability pair with no such structural match MUST
  NOT be selected, even if its name or namespace suggests a plausible fit. A
  capability whose contract separately declares real `consumes`/`emits`
  (`EventReference`) linkage to a governed event MAY also be selected on that
  basis — the two signals are independent, and a match on either is sufficient.
- **FR-003**: When more than one published capability's `outputs.schema` could
  structurally satisfy the same downstream `inputs.schema` need, the planner MUST
  NOT pick one automatically. It MUST enumerate each resulting complete
  candidate plan separately and return the full set to the caller.
- **FR-004**: The planner MUST bound its search: at most 5 complete
  candidate plans, each at most 8 nodes deep, per planning call. A call
  that would exceed either bound MUST return the bounded subset it found
  plus a stable `plan_search_truncated` indicator, never fail silently or
  search unbounded.
- **FR-005**: Each returned candidate plan MUST include a best-effort,
  per-edge field-level JSON-path mapping derived from the source
  capability's `outputs.schema` and the target capability's `inputs.schema`.
  A candidate plan MUST be flagged `mapping_unconfirmed` until a human or
  caller-side reviewer approves it; this flag MUST NOT be cleared by the
  planner itself.
- **FR-006**: A candidate plan MUST be structurally submittable as-is to the
  `109-runtime-workflow-proposals` MCP surface (identical capability-id/
  version, edge, and mapping shape) once its `mapping_unconfirmed` flag is
  cleared by review. The planner MUST NOT submit or execute a plan itself.
- **FR-007**: The planner MUST be exposed as a public MCP tool alongside the
  existing P1 proposal tools (e.g. `workflow.plan`), discoverable without
  special credentials, per FR-001 of spec `109`.
- **FR-008**: Planning MUST be a pure, read-only operation over the current
  registry/manifest snapshot. It MUST NOT mutate the registry, the workflow
  catalog, or any manifest, and MUST NOT persist a candidate plan as a
  reusable workflow.
- **FR-009**: A remainder replan MUST accept only completed-step output
  references that are bounded, schema-validated, and bound to the completed
  trace digest. The host resolves authorized values locally; the planner and
  MCP surface MUST NOT receive raw secrets, credentials, or unrestricted prior
  outputs.
- **FR-010**: A remainder replan MUST exclude the failed capability version,
  propose only not-yet-completed work, and require fresh out-of-band approval
  before execution. Missing, tampered, expired, unbound, or schema-invalid
  references MUST fail closed.

## Acceptance scenarios

1. A caller requests a plan targeting a capability whose `inputs.schema` only
   one other published capability's `outputs.schema` structurally satisfies;
   the planner returns exactly one candidate plan with field mappings flagged
   `mapping_unconfirmed`.
2. A caller requests a plan where two published capabilities' `outputs.schema`
   both structurally satisfy the same downstream `inputs.schema` need; the
   planner returns two complete candidate plans, one per choice, rather than
   picking one.
3. A caller requests a plan for a target no published capability's
   `outputs.schema` structurally satisfies (and that declares no real
   `consumes`/`emits` linkage either); the planner returns zero candidates
   rather than falling back to a name/namespace guess.
4. A candidate plan's field mapping is reviewed and corrected by a human
   before submission; the corrected mapping, not the planner's original
   guess, is what gets submitted to and validated by the P1 surface.
5. A goal whose valid chain space exceeds the bound returns the bounded
   subset plus `plan_search_truncated: true`, never an unbounded result or
   a silent failure.
6. After an approved step fails, a replan receives only trace-bound completed
   output references, excludes the failed capability version, and cannot run
   until a new approval token is minted.

## Out of scope

Natural-language goal interpretation, embedding a hosted planner model,
automatic proposal submission or execution, ambiguity auto-resolution,
unbounded plan search, registry or workflow-catalog mutation, and any
change to how `109`–`112` validate, authorize, or execute a submitted
proposal.
