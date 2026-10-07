# Feature Specification: Browser-Local Deterministic Workflow Composition

**Status**: Approved (2026-09-08)
**Canonical governing ID**: `1277-browser-local-workflow-composition`
**Version**: 0.2.0
**Amended**: 2026-10-03 (Decision 98 / #1399): deterministic
multi-proposal ordering.
**Extends**: `108-governed-runtime-workflow-composition`,
`109-runtime-workflow-proposals`, `113-declarative-workflow-planning`, and
`1258-offline-cache-activation`.
**Decision evidence**: Traverse #1271 decision record (2026-09-08) and
Decision 98 in Traverse #1399 (2026-10-03).

## Purpose and boundary

Define the backend-less `/discover` path: a browser uses an already synced,
digest-verified public registry snapshot to deterministically produce one or
more untrusted workflow proposals, then hands a selected reviewed proposal to
the local governed runtime for manifest-bounded execution. The browser is not
a runtime authority and this path does not introduce a Traverse-operated
planning service.

## Requirements

- **FR-001**: The browser planner MUST accept only the structured target and
  starting facts defined by Spec 113. It MUST use a supplied snapshot identity
  containing the registry snapshot digest, preparation/verification evidence,
  and supported contract-schema version; it MUST reject missing, altered,
  unverified, unsupported, or stale evidence with a stable secret-free error.
- **FR-002**: Planning MUST be deterministic and read-only over that exact
  snapshot. It MUST derive candidates only from the structural and declared
  event relationships allowed by Spec 113, enumerate ambiguity rather than
  choose it, and retain Spec 113's five-plan/eight-node bounds. When more than
  one valid proposal exists, it MUST order the complete bounded search result
  by ascending node count and then lexicographically by the ordered
  capability-id chain. It MUST apply the five-plan limit only after that
  ordering, set `plan_search_truncated` when valid proposals are omitted, and
  expose a stable no-candidate outcome. Callers MUST NOT supply a preference
  or scoring input that changes this order.
- **FR-003**: The browser planner MUST NOT infer a plan from capability names,
  namespaces, natural-language goals, prompt/model output, recency, or an
  undisclosed scoring rule. It MUST NOT fetch, sync, refresh, mutate, or
  persist registry, manifest, workflow-catalog, or proposal state.
- **FR-004**: Each browser result MUST be a versioned,
  canonicalizable `workflow_proposal` structurally compatible with Spec 109.
  It MUST bind the source snapshot identity, exact capability identities and
  versions, graph, and explicit field mappings; all inferred mappings remain
  `mapping_unconfirmed` until a separate reviewer/caller action clears them.
- **FR-005**: Browser planning is an untrusted proposer. Proposal validation,
  authorization, data-flow/egress policy, connector and placement checks,
  approval-token checks, canonicalization, execution scheduling, and trace
  production MUST remain with the local governed runtime under Specs 108 and
  109. A browser result MUST NOT enlarge manifest authority or bypass a
  runtime denial.
- **FR-006**: The local handoff MUST execute only exact, prepared
  `registry_ref` components whose activation evidence satisfies Spec 1258. It
  MUST fail closed when activation evidence, digests, versions, ABI, target,
  placement, trust lifecycle, or policy inputs do not match; it MUST NOT
  substitute a candidate, re-resolve a range, fetch, sync, refresh, or fall
  back to an expedition bundle during validation or execution.
- **FR-007**: The browser and local handoff MUST enforce bounded input,
  proposal, graph, payload, validation-time, execution-time, and memory
  limits. Failures and public evidence MUST be stable and redacted: they may
  identify declared artifact identity, snapshot digest, and failure class, but
  MUST NOT expose credentials, private configuration, host paths, raw request
  values, or artifact bytes.
- **FR-008**: This path MUST operate without a public remote planning endpoint.
  It MUST NOT require CORS exceptions, anonymous service access, hosted model
  credentials, or a Traverse-operated planning service. A future remote
  endpoint requires a separately approved specification and ADR.

## Acceptance scenarios

1. A browser receives a verified pinned public snapshot and a structured
   target; it deterministically returns all bounded candidate proposals with
   mappings marked unconfirmed, without making a network request.
2. Candidate chains of different lengths structurally satisfy a consumer; the
   browser returns at most five proposals shortest-chain-first, breaks equal
   lengths lexicographically by their ordered capability-id chains, and marks
   the result truncated when another valid proposal exists. It applies no
   caller preference. A name-only apparent match returns no candidate.
3. A changed snapshot digest, absent verification evidence, or unsupported
   schema version fails before planning with a stable redacted error.
4. A reviewed proposal with exact prepared `registry_ref` components passes
   through local Spec-109 validation and executes offline. A missing, drifted,
   incompatible, or unprepared component fails closed without fallback.
5. A proposal that asks for an undeclared capability, prohibited mapping,
   connector, placement, or side effect is rejected by the local runtime even
   though the browser produced it.
6. Oversize inputs, candidate search beyond the fixed bound, or a resource
   limit breach yields the specified stable bounded outcome and no raw private
   data in evidence.

## Compatibility and non-goals

This is additive. Specs 108, 109, 113, and 1258 remain immutable and retain
their authority. It does not implement the browser proposal UI (#1269), the
runtime-composed execution adapter (#1270), a hosted endpoint, natural-
language planning, automatic review/submission, registry mutation, dynamic
manifest mutation, or network activity after preparation.

## Validation

A shared Rust/TypeScript conformance fixture must cover multiple valid chain
lengths, shortest-first ordering, the lexicographic tie-break, post-ordering
candidate limiting, and truncation. Conformance fixtures must also cover
deterministic identical-snapshot output, ambiguity, no name/namespace
inference, invalid snapshot evidence, offline handoff, prepared-component
drift, authorization/policy denial, resource bounds, and evidence redaction.
Run `bash scripts/ci/spec_alignment_check.sh` with the PR body and the
repository documentation checks.
