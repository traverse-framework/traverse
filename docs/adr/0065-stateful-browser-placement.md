# ADR-0065: Stateful Browser Placement via IndexedDB Host Attestation

- Status: Proposed
- Date: 2026-09-09
- Governing spec: `132-stateful-browser-placement` (Proposed)
- Extends / supersedes placement rule in: `014-service-type-taxonomy` (`208`) FR-005
- Depends on: `085-datastore-indexeddb`, `131-stateful-persistence-host-abi`
- Related issues: #1305 (specification), #1289 (implementation)
- Related: ADR-0063; `docs/decision-log.md` Decision 72

## Context

Spec `014` / `208` FR-005 rejects `service_type: Stateful` when
`permitted_targets` includes `Browser`, on the assumption that browsers cannot
offer managed persistence. Spec `085` since approved an IndexedDB DataStore
backend with exclusive ownership and public integrity. Spec `131` defines the
`state_*` host ABI serviced by an abstract host store. Decision 68 deferred
Browser placement so the first Stateful slice stayed server/edge-side.

#1289 asks to permit Stateful on Browser when a qualifying IndexedDB store is
bound. A 2026-09-09 `/brainstorm` recorded the product decisions on #1289 and
opened #1305 as the governing vehicle.

## Decision

1. **Supersede contract-time FR-005.** Contract validation MUST accept
   Stateful+Browser. Portable contracts MUST NOT declare IndexedDB or any
   concrete backend.
2. **Enforce at activation with runtime backend proof.** Browser activation of
   a Stateful capability succeeds only when the bound DataStore is the
   IndexedDB backend and has opened under Spec `085` guarantees (exclusive
   lock/ownership acquired; public integrity path available). Otherwise fail
   closed with `stateful_browser_store_unavailable` before guest execution.
3. **No honor-system flag; no per-activation conformance certificate.**
   Attestation is in-process verification of the bound open store.
4. **Secret-free evidence.** Activation/telemetry evidence MUST NOT include
   payloads, DB names, or host-private paths.
5. **Keep follow-ons separate.** Private encryption (#1294), maintenance
   parity (#1295), ABI extensions (#1287/#1288/#1290), and multi-role taxonomy
   (#1291) remain out of scope.

## Consequences

- `traverse-contracts` must stop applying the FR-005 hard reject once Spec
  `132` is approved and #1289 implements it.
- Runtime Browser activation gains an IndexedDB attestation gate and a stable
  error code.
- Immutable `014`/`208` text is not edited in place; Spec `132` is the
  successor for the superseded clauses only.
- #1289 stays `needs-spec` until Spec `132` and this ADR are approved.

## Alternatives Considered

- **Keep the Browser ban** — rejected: Spec `085` would remain unavailable to
  Stateful capabilities.
- **Declare IndexedDB in each capability contract** — rejected: couples
  portable contracts to one host backend.
- **Embedder attestation flag only** — rejected: not runtime-verifiable.
- **Require Spec `085` conformance evidence artifact on every activation** —
  rejected: operationally too heavy for normal app start.
- **Soft contract warning + hard activation fail** — rejected: warnings are
  not a governed fail-closed boundary.

## Approval evidence

Pending maintainer approval of Spec `132-stateful-browser-placement` and
acceptance of this ADR. Proposed artifacts and a merged PR without approval
evidence do not count as approved.
