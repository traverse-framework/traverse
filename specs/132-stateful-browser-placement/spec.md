# Feature Specification: Stateful Browser Placement via IndexedDB Attestation

**Feature Branch**: `codex/issue-1305-stateful-browser-placement`
**Created**: 2026-09-09
**Status**: Approved (2026-09-09)
**Canonical governing ID**: `132-stateful-browser-placement`
**Version**: 0.1.0
**Supersedes**: `014-service-type-taxonomy` / `208-service-type-taxonomy` **FR-005**, User Story 2, SC-002, and the assumption that Browser cannot provide managed persistence (only those placement rules; all other `014`/`208` requirements remain in force)
**Extends**: `085-datastore-indexeddb`, `131-stateful-persistence-host-abi`, `518-durable-local-datastore`, `519-embedder-owned-datastore-integration`
**Input**: Issue #1305; product decisions recorded on #1289 (2026-09-09 `/brainstorm`); ADR-0065

## Purpose

Relax the contract-time ban on `service_type: Stateful` with `Browser` in
`permitted_targets`, now that Spec `085` defines an IndexedDB DataStore backend
with durable public integrity and exclusive ownership. Move the durable-store
proof to **activation**, where the runtime can verify the bound store — not to
the portable capability contract.

## Capability Boundary

| Concern | Owner |
| --- | --- |
| Contract validation of `service_type` + `permitted_targets` (accept Stateful+Browser) | This spec / `traverse-contracts` |
| Activation-time proof that the bound DataStore is Spec `085` IndexedDB and open under its guarantees | This spec / runtime activation |
| IndexedDB backend semantics (lock, integrity, public CRUD, fail-closed private) | Spec `085` |
| `state_*` host ABI | Spec `131` |
| Private-record encryption via browser KeyProvider | Out of scope (#1294) |
| IndexedDB maintenance parity | Out of scope (#1295) |

## User Scenarios & Testing

### User Story 1 — Portable Stateful+Browser contract validates (Priority: P1)

1. **Given** a capability contract with `service_type: Stateful` and
   `permitted_targets` containing `Browser` (alone or with other targets),
   **When** contract validation runs, **Then** it succeeds with respect to the
   former FR-005 rule (no `InvalidPlacementConstraint` solely for
   Stateful+Browser).

### User Story 2 — Browser activation succeeds with qualifying IndexedDB store (Priority: P1)

1. **Given** a Stateful capability permitted on Browser and a bound DataStore
   that is the IndexedDB backend and has opened under Spec `085` (exclusive
   ownership/lock acquired; public integrity path available),
   **When** Browser activation runs, **Then** activation succeeds for the
   durable-store check governed by this spec.

### User Story 3 — Browser activation fails closed without attestation (Priority: P1)

1. **Given** a Stateful capability permitted on Browser and any of: no bound
   DataStore, a non-IndexedDB DataStore backend, or an IndexedDB store that did
   not open under Spec `085` guarantees (missing exclusive lock / unavailable
   persistence),
   **When** Browser activation runs, **Then** activation fails closed with
   stable code `stateful_browser_store_unavailable` and does not execute the
   capability.

### User Story 4 — Non-Browser Stateful activation unchanged (Priority: P1)

1. **Given** a Stateful capability activating on Edge, Cloud, or Device with a
   host store meeting Spec `131`'s guarantee floor,
   **When** activation runs, **Then** this spec imposes no additional
   IndexedDB attestation requirement.

## Requirements

- **FR-001**: Contract validation MUST NOT reject a capability solely because
  `service_type` is `Stateful` and `permitted_targets` includes `Browser`.
  This requirement supersedes `014`/`208` FR-005, User Story 2, and SC-002.
- **FR-002**: For activation of a `Stateful` capability on `Browser`, the
  runtime MUST require a bound DataStore that is the IndexedDB backend defined
  by Spec `085` and that has opened under Spec `085` guarantees: exclusive
  ownership/lock acquired and the public integrity path available.
- **FR-003**: When FR-002 is not satisfied, Browser activation of a Stateful
  capability MUST fail closed with stable error code
  `stateful_browser_store_unavailable` before guest execution begins.
- **FR-004**: Attestation MUST be runtime-verifiable from the bound open store
  (backend identity + open guarantees). It MUST NOT be satisfied by an
  embedder honor-system flag alone, and MUST NOT require a separate Spec `085`
  conformance certificate artifact on every activation.
- **FR-005**: Activation and telemetry evidence for this check MUST be
  secret-free and MUST NOT include record payloads, IndexedDB database names,
  or host-private filesystem/DOM paths.
- **FR-006**: This spec MUST NOT change Spec `131` import signatures, Spec
  `085` backend behavior, or non-Browser Stateful activation rules.

## Success Criteria

- **SC-001**: Automated contract tests accept Stateful+Browser contracts that
  previously failed FR-005.
- **SC-002**: Browser activation conformance covers success with a qualifying
  open IndexedDB store and fail-closed outcomes for missing, non-IndexedDB,
  and non-qualifying open failures, asserting
  `stateful_browser_store_unavailable`.
- **SC-003**: Evidence fixtures for those paths contain no payloads, DB names,
  or host-private paths.

## Assumptions

- Spec `085` remains the definition of a qualifying browser durable store for
  this placement rule.
- Private DataStore classification on IndexedDB remains fail-closed until #1294;
  FR-002 only requires the public integrity path to be available.
- Capability contracts stay backend-agnostic; hosts choose and prove storage.

## Out of Scope

- Runtime/contracts implementation (tracked by #1289).
- `#1294` private encryption, `#1295` maintenance parity.
- `#1287` / `#1288` / `#1290` Stateful ABI extensions.
- `#1291` multi-role service_type taxonomy.
- Editing the immutable text of `014`/`208` in place (successor supersession
  only).

## Validation

```bash
# After implementation of #1289 (not required to approve this Proposed spec):
cargo test -p traverse-contracts
cargo test -p traverse-runtime
```

Approved by maintainer decision on 2026-09-09 (brainstorm Option A on
#1305 / PR #1307). Recorded in `specs/governance/approved-specs.json`.
