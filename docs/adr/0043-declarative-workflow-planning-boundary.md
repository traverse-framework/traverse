# ADR-0043: Plan Workflows as a Deterministic, Data-Dependency-Only Reader, Never an Auto-Chooser

- Status: Accepted (amended 2026-08-23 for Decision 62; amended and approved
  2026-11-20 for Decision 136 / #1896)
- Governing specs: `108-governed-runtime-workflow-composition`, `113-declarative-workflow-planning` (v0.2.0)
- Related issues: #865, #1089, #1098; registry #305

## Context

`109-runtime-workflow-proposals` (P1) deliberately excluded "planner
implementation" from its scope, leaving open how a concrete, submittable
proposal actually gets generated in the first place. A live website demo
(traverse-framework.com/discover.html) exposed this gap concretely: without
real declared data-dependency metadata or a real planner, composing a
plausible workflow from live registry data required a client-side
namespace/verb-name heuristic — a guess, not something grounded in the
contracts themselves.

Implementing this spec's real unblock condition (registry `#305`'s
backfill) surfaced that `consumes`/`emits` (`EventReference`) is not the
right signal for most capability pairs: registry's own governed FR-020
capability inventory classifies 45 of 46 unique published capability IDs —
including every capability in an already-published, human-reviewed
`workflows/*.json` chain — as `no-event-required`, synchronous,
direct-return capabilities with no asynchronous domain fact to broadcast.
`consumes`/`emits` has consistently meant a real, governed, decoupled-
subscriber event elsewhere in that repo; "A's output feeds B's input" is
the *workflow-composition* concept, which `workflows/*.json` already models
via explicit node edges and field mappings, not the event concept.

## Decision

The planner derives candidate chains from structural
`inputs.schema`/`outputs.schema` compatibility — every property named in a
candidate consumer's `inputs.schema.required` MUST exist in a candidate
producer's `outputs.schema.properties` with a matching JSON type. This is
not a new computation: the planner already derives this same structural
relationship for its per-edge field-mapping step. A capability that
separately declares real `consumes`/`emits` linkage to a governed event MAY
also be selected on that basis — the two signals are independent extension
points, not a replacement of one by the other. The planner never infers a
chain from capability names, namespaces, or any other signal, and never
operates on a capability pair with no structural match. When more than one
capability could satisfy a need, the planner enumerates every resulting
complete candidate plan rather than choosing one — ambiguity resolution is
a caller/reviewer decision, never a runtime one, consistent with ADR-0050
(renumbered from ADR-0041 on 2026-08-24) treating the planner-adjacent
surface as untrusted and this runtime's
everywhere-else stance against silent inference. Search is bounded by
fixed, small, non-configurable limits in v1. Field-level JSON-path mappings
are proposed but always marked unconfirmed until reviewed — the planner
produces a draft, not an authority. The planner is a new phase (P0) of the
`108` north star rather than a standalone spec, since `109` already named
this exact gap as its own boundary.

## Consequences

### Proposed Decision 136 amendment

For agent-originated workflows, the agent supplies a structured target and
reads deterministic candidates but cannot approve execution. The host renders
the selected proposal in a local CLI prompt and issues a signed, single-use,
digest-bound token after human confirmation. A failed execution may replan
only the uncompleted remainder from bounded trace-bound output references;
the host resolves authorized values locally, excludes the failed capability
version, and requires fresh approval.

A planner call against today's registry finds candidates wherever a
capability's `outputs.schema` structurally satisfies another's
`inputs.schema.required` — which, unlike `consumes`/`emits`, is already
true for many already-published capability pairs (e.g. the `doc-approval`
and `traverse-starter` reference chains) without requiring any registry-side
backfill. Registry `#305` is resolved by this redirect, not by a backfill;
`traverse#1098` is unblocked by this spec amendment rather than by
registry-side work. A pair whose schemas happen to overlap without being
genuinely composable (e.g. two unrelated capabilities that both take
`{id: string}`) can still surface as a false-positive candidate — this is
accepted, not solved here, because FR-005/FR-006's `mapping_unconfirmed`
flag and mandatory human review before any submission already gate exactly
this risk; nothing a planner proposes ever executes unreviewed.

## Alternatives considered

- Fall back to a namespace/verb heuristic when there is no structural match
  (matching the discover.html demo): rejected — mixing a real, data-grounded
  candidate with a guessed one on the same governed proposal surface blurs
  exactly the honesty line this org has held elsewhere (registry
  decision-log entry 61).
- Auto-resolve ambiguity via a scoring heuristic (coverage %, version
  recency): rejected — a silent automatic pick is the kind of unreviewed
  runtime decision `109`'s explicit-mapping requirement exists to prevent.
- Accept natural-language goals and interpret them in-runtime: rejected —
  reintroduces exactly the hosted-model dependency `109` FR-001 and
  ADR-0050 (renumbered from ADR-0041 on 2026-08-24) explicitly excluded.
- Loosen registry's FR-020 inventory criterion instead, keep `consumes`/
  `emits` as the sole signal: rejected — would require ~26 capabilities to
  carry full governed-event ceremony (privacy field classification,
  retention policy, CloudEvents mapping) for relationships that were never
  really asynchronous events, and blurs a distinction registry's decision
  57 deliberately drew.
- Exact full-schema match (not just required-property overlap): rejected —
  independently-authored capabilities' schemas are rarely exactly equal or
  superset even when genuinely composable; would find very few real chains.
- A new named/tagged data-shape identifier capabilities opt into: deferred,
  not rejected — avoids both false positives and event ceremony, but is
  itself a new schema convention needing its own spec and per-capability
  adoption; worth reconsidering if structural matching's false-positive
  rate proves too high in practice.
