# Workflow Authoring Guide

Thin product pointer for how Traverse expects DEVs and coding agents to
author and run workflows. Deep ritual lives in the skill; this page states
the defaults and where to go next.

**Decision evidence**: [Decision 80](decision-log.md#decision-80-dual-path-workflow-authoring--plan-then-seal-default-skill-front-door)
(2026-09-10).

## Defaults

| Mode | When | Authority |
|---|---|---|
| **Sealed workflow (default)** | What ships and runs in production | Reviewed, pinned `workflow.json` referenced from the app/package manifest (`known_compositions` / workflow refs). Deterministic traversal. |
| **Plan-then-seal (authoring)** | Building or changing a workflow | Planner/MCP proposes candidates; human confirms seal; CLI validates before trust. |
| **Adaptive composition (opt-in)** | Explicit app/request flag only | Live plan at execution time under governed proposal rules. Not the default; never silently persisted as the sealed app workflow. |

Sealed workflows are the production path. Runtime planning is not.

## Plan-then-seal (authoring)

1. Clarify the goal and structured target.
2. Discover capabilities (registry / MCP).
3. Ask the declarative planner for candidate graphs (planner remains an
   untrusted proposer — see ADR-0043 / specs `108`–`113`).
4. If capabilities are missing, author the gaps, then re-plan.
5. **Pause for explicit seal confirmation** before writing artifacts.
6. Write sealed `workflow.json` and manifest workflow refs.
7. **Required DoD**: CLI validate green (including digest fixes).
8. Optional follow-on: register and one smoke execute when the workspace is ready.

Do not auto-seal on a single proposal. Do not seal placeholder or partial
workflows.

## Where to work

| Surface | Role |
|---|---|
| [`.agents/skills/traverse-app-builder/`](../.agents/skills/traverse-app-builder/) | Canonical deep authoring ritual (plan → gap-author → confirm seal → validate). Prefer this in-repo copy over any personal `~/.claude/skills/traverse-app-builder`. |
| `traverse-cli` `app validate` / `workflow` validate & register | Verifier and registrar — see [cli-reference.md](cli-reference.md). |
| Future `workflow plan` / `promote` ([#1346](https://github.com/traverse-framework/traverse/issues/1346)) | CLI wrappers over planner (113) and promotion (112) once shipped; prefer them from the skill when available. |
| Adaptive runtime opt-in ([#1345](https://github.com/traverse-framework/traverse/issues/1345)) | Explicit sealed-default vs adaptive knob — follow-on implementation. |

Hand-composed workflows remain valid; start from
[workflow-composition-guide.md](workflow-composition-guide.md) when you already
know the graph. Prefer the skill when discovery or gap-authoring is part of the
session.

## Governing specs and ADRs

- Workflow registry / traversal: `007`, `041`
- Planner as untrusted proposer; sealed workflow as execution authority:
  `108`–`113`, `109`–`112`
- [ADR-0043](adr/0043-declarative-workflow-planning-boundary.md) — planning boundary
- [ADR-0050](adr/0050-governed-runtime-workflow-proposal-authority.md) —
  proposals as untrusted, manifest-bounded snapshots
- Proposal lifecycle: [workflow-proposal-lifecycle.md](workflow-proposal-lifecycle.md)
- Promotion path detail: [governed-workflow-promotion.md](governed-workflow-promotion.md)

This guide intentionally does **not** copy the full skill ritual.
