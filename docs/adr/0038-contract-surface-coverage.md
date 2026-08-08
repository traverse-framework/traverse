# ADR-0038: Contract Surface Must Be Covered by Use Cases

- Status: Proposed (accept with Spec `102-contract-surface-coverage` approval)
- Governing spec: `102-contract-surface-coverage` (Draft)
- Related issues: traverse#1014, #1015, #1016; registry#192, #193

## Context

Capability contracts combine:

1. Free-text `summary` / `description`
2. JSON Schema input surface (including discriminator enums such as `action`)
3. `use_cases[]` with concrete input/output examples
4. An executable WASM artifact verified by package smoke

Only (3) and (4) are mechanically exercised today. `core.process-comment@1.0.0` demonstrated the failure mode: the schema enum and description advertised `resolve` / `pin` / markup sanitisation / allow-list “strict” mentions, while the artifact and eight use cases implemented a narrower matrix. Registry and traverse publish gates accepted the overclaim.

## Decision

Adopt **schema ⊆ use_cases ⊆ smoke** as a governed rule for discriminator enums (starting with `action`):

- Every enum value retained in the contract MUST have at least one use case.
- Publish dry-run MUST fail on gaps once Spec 102 is Approved.
- Description claims beyond use cases MUST be called out under **Known limitations** or removed.
- An enum value MUST NOT be “implemented” solely as an undocumented generic unsupported stub.

For already-published overclaims: do not edit immutable versions; publish an honesty bump (e.g. `core.process-comment@1.0.1`) and deprecate the overclaiming version with an explicit reason.

## Alternatives Considered

- **Description-only linting (NLP)** — rejected for v1: high false-positive risk; use cases are the executable contract.
- **Require implementing every marketing claim immediately** — rejected as the default honesty path: narrowing the declared surface is a valid fix; full feature completion is a separate product ticket.
- **Gate only in registry** — rejected as sole control: authors need fail-fast in `capability publish --dry-run` before opening a registry PR. Registry SHOULD mirror the check for newly ADDED contracts.

## Consequences

- Traverse gains Spec 102 + publish coverage checker (#1016) after approval.
- Registry gains a diff-based mirror check (registry#192) after the traverse spec is Approved (or a thin registry FR that references it).
- `core.process-comment` honesty bump (#1015 / registry#193) can land under existing `516` without waiting for Spec 102 approval, because it reduces claimed surface to already-tested behavior.
