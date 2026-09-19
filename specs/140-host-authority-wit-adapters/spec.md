# Feature Specification: Target-Neutral Host Authorities With WIT Host-Adapter Interfaces

**Feature Branch**: `codex/host-authority-wit-governance`
**Created**: 2026-09-18
**Status**: Approved (2026-09-18)
**Canonical governing ID**: `140-host-authority-wit-adapters`
**Version**: 0.1.0
**Extends**: `137-host-connector-command-dispatch`,
`138-governed-exact-model-execution`,
`139-embedder-app-state-machine-execution`,
`1259-portable-authority-contracts`,
`103-application-connector-binding`
**Amends**: `1259-portable-authority-contracts` (0.1.0 → 0.2.0),
`137-host-connector-command-dispatch` (0.2.0 → 0.3.0),
`138-governed-exact-model-execution` (0.1.0 → 0.2.0)
**Decision evidence**: Decision 97; ADR-0076 (Accepted).
**Approval**: Owner-approved in `/brainstorm` session 2026-09-18 (Decision 97).

## Purpose and boundary

Define the **generic rules** by which any Traverse host authority (connector)
is exposed to applications and implemented by hosts, independent of
application, capability, or target (browser, macOS, iOS, Android, Windows,
server). `traverse.audio-input` is the first authority to apply these rules;
it is an example, not the scope.

This spec governs the *contract shape and conformance* of authorities and
their host adapters. It does not implement production drivers, choose
codecs/devices/vendors, or change the Spec 137 public command/result/event
envelope.

## Non-goals

- Executing host adapters as Wasm Components inside `runtime.wasm`. WIT is the
  **interface definition language** for adapters; an adapter MAY be native
  code (Swift, Kotlin, C#, Rust), JavaScript, or a Component, provided it
  conforms to the WIT-defined semantics.
- Guest (capability) access to host authorities. Spec 104 mediation and
  Spec 135 `component-wit-v1` guest profile are unchanged.
- Durable artifact storage or an object-store authority (see
  `docs/callweave-universal-connector-follow-up.md`; not a dependency).
- Async WIT (Component Model preview 3). Adapter functions complete through
  the Spec 139 host-connector wait; blocking semantics are not required.

## Relationship to existing specifications

| Specification | Relationship |
| --- | --- |
| 1259 | "Native-only" is removed as a contract classification (amended) |
| 137 | Target rule generalized; adds `audio.permission.request` (amended) |
| 138 | Host staging generalized to bounded artifacts, incl. audio (amended) |
| 139 | Runtime-owned state machine drives authorities via host-connector waits |
| 135 | Guest-facing WIT profile; unchanged and separate |
| 103 / 104 | Bindings and guest mediation unchanged |
| 057 / 529 | Cross-platform embedder conformance MUST cover these rules |

## Functional requirements

### Contract identity

- **FR-001**: Each host authority MUST have exactly one target-neutral
  connector ID, one public envelope, and one WIT adapter interface. There MUST
  be no per-OS, per-browser, or per-application variant of the same authority.
- **FR-002**: "Native-only" MUST NOT be a connector contract classification.
  A binding MUST declare the target families it supports. Activating a
  binding on a target family it does not declare, or for which the host
  supplies no adapter, MUST fail before invocation with the stable
  secret-free code `target_incompatible`.
- **FR-003**: Command, result, event, and error field names MUST be identical
  across all target families (Spec 137 FR-006, generalized).

### WIT host-adapter interface

- **FR-004**: Every authority MUST publish a versioned WIT interface that is
  the normative definition of its host-adapter operations, request/result
  records, and failure classes. Adapters on every target MUST implement the
  same interface semantics.
- **FR-005**: Adapter WIT interfaces MUST NOT contain device IDs, paths,
  credentials, endpoints, vendor names, or application workflow fields.
- **FR-006**: The public dispatch surface remains the Spec 137 JSON envelope.
  WIT adapter functions are invoked only by the host-side dispatcher, never by
  guests.
- **FR-007**: The WIT package MUST be committed under
  `contracts/connectors/<connector-id>/wit/` and version-pinned by the
  connector contract. Changing an interface is a versioned contract change.

### Permissioned authorities

- **FR-008**: An authority that requires user or OS permission MUST expose
  `permission-status` and `request-permission` in its WIT interface, returning
  the non-secret enum `granted | denied | prompt-required | unavailable`.
- **FR-009**: The app state machine MAY declare a permission wait via
  `invoke.host_connector` on the authority's permission operation, with its
  own `host_connector_*` completion routes (Spec 139). Target-specific
  gesture or permission tokens MUST NOT appear in the public envelope; the
  adapter owns gesture/prompt handling.
- **FR-010**: A denied or unavailable permission MUST yield a typed
  `host_connector_failed` (`policy_denied` or `unavailable`), never a silent
  hang or a generic capture failure.

### Host-produced artifacts

- **FR-011**: Host-produced binary results MUST be written to host-owned
  staging (Spec 138, generalized) and returned as an opaque `artifact_ref`.
  Public contracts MUST NOT expose raw bytes, paths, or URLs.
- **FR-012**: An authority MUST NOT define its own artifact storage unless a
  separately approved contract does.

### Conformance

- **FR-013**: Each authority MUST ship implementation-independent fixtures
  for: success; missing/incompatible/unconfigured/unactivated binding;
  unsupported target; permission granted/denied/unavailable; limits;
  cancellation; idempotency; redaction; and **identical ordered events across
  target families for the same command sequence**.
- **FR-014**: The Spec 057/529 embedder conformance suite MUST run these
  fixtures against every published embedder for the pinned embedder-api
  version.
- **FR-015**: Work MUST be scoped and shipped by generic contract, not by
  platform. A blocker specific to one target's tooling or evidence MUST be
  tracked as a separate constraint and MUST NOT redefine the contract.

## First authority: `traverse.audio-input` (normative WIT)

```wit
package traverse:audio-input@1.0.0;

interface capture {
  enum permission-state { granted, denied, prompt-required, unavailable }

  enum failure-class {
    unbound, incompatible, unconfigured, policy-denied,
    target-incompatible, limit-exceeded, cancelled, unavailable
  }

  record failure { class: failure-class, code: string }

  record capture-request {
    correlation-id: string,
    max-duration-ms: u32,
    max-bytes: u32,
  }

  record capture-result {
    artifact-ref: string,
    duration-ms: u32,
    size-bytes: u32,
  }

  permission-status: func() -> permission-state;
  request-permission: func() -> result<permission-state, failure>;
  capture: func(request: capture-request) -> result<capture-result, failure>;
  cancel: func(correlation-id: string);
}
```

Operation mapping (Spec 137): `audio.permission.request` →
`request-permission`; `audio.capture` → `capture`. `permission-status` is
host-internal and MAY be used by the dispatcher for fail-fast evidence. The
adapter MUST stage captured audio and return only `artifact-ref`.

## Acceptance scenarios

1. Given an app manifest that binds `traverse.audio-input`, when the same
   command sequence (`request_permission` → `capture_audio`) is submitted on
   any target family that has an activated adapter, then the ordered events
   and result fields are identical.
2. Given a target family with no adapter, when the command is submitted, then
   it fails with `target_incompatible` before any capture.
3. Given permission `denied`, when `capture_audio` follows, then the state
   machine takes the declared failure route with `policy_denied`.
4. Given a captured recording, when a registered capability consumes the
   `artifact_ref`, then it reads bounded bytes only through runtime-mediated
   staging (Spec 138) and never receives a path or URL.
5. Given a new authority proposal, when it defines a per-target ID or a
   native-only classification, then governance review rejects it under
   FR-001/FR-002.

## Out of scope

- Production microphone drivers and OS permission UX copy.
- Object-store, state-store, and scheduler authorities.
- Durable artifact retention.

## Governance

- ADR-0076 records the decision. Implementation tickets MUST NOT be `Ready`
  until this spec and ADR-0076 are on `main`.
