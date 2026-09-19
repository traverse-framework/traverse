# ADR-0076: Target-Neutral Host Authorities With WIT Host-Adapter Interfaces

- Status: Accepted
- Date: 2026-09-18
- Governing spec: `140-host-authority-wit-adapters` (Approved)
- Related decisions: Decision 97
- Extends: ADR-0060; ADR-0071; ADR-0074; ADR-0075
- Amends: Specs `1259`, `137`, `138`

## Context

Spec 1259 classed `traverse.audio-input` as native-only and Spec 137 rejects a
`browser` target for it. ADR-0060 records no technical limit, only a non-goal
of emulating native authority in the browser. The Spec 137 envelope is already
target-neutral. Keeping a per-target authority forces per-target application
manifests and breaks the shared runtime-owned state machine (ADR-0075).

## Decision

1. **One contract per authority.** One target-neutral connector ID, one
   envelope, one WIT adapter interface. No per-OS, per-browser, or per-app
   variants. Bindings declare supported target families; an unsupported or
   adapter-less target fails closed with `target_incompatible`. "Native-only"
   is no longer a contract classification.
2. **WIT is the adapter interface definition language.** Adapters on every
   target implement the same WIT semantics. The public dispatch contract stays
   the Spec 137 JSON envelope. Adapters need not be Wasm Components; WIT does
   not put Component Model execution inside `runtime.wasm`.
3. **Explicit permission step** (`permission-status`, `request-permission`) for
   permissioned authorities, drivable as a state-machine wait. No gesture or
   permission token enters the public envelope.
4. **Host-produced artifacts use Spec 138 staging**, generalized from
   model-only to bounded artifacts, and referenced by opaque `artifact_ref`.
5. **Scope by generic contract, not by platform.** Target-specific blockers
   are separate constraints.

## Consequences

- Specs 1259, 137, 138 gain surgical amendments; Spec 140 governs the generic
  rules and the first WIT interface (`traverse.audio-input`).
- `crates/traverse-runtime` (`host_connector_dispatch.rs`) and the
  `traverse.audio-input` connector contract must drop the native-only rule in
  implementation tickets.
- Embedder conformance (Spec 057/529) gains cross-target ordered-event
  fixtures.
- Browser microphone capture becomes possible through a host-owned adapter
  without a new authority (supersedes the scope of `#1471`).

## Alternatives considered

- Separate browser connector: rejected; per-target manifests.
- Native-only class with host-injected refs: rejected; splits state-machine
  ownership (ADR-0075).
- Permission handled inside the operation: rejected; non-deterministic denial.
- Gesture token in the envelope: rejected; leaks target concepts.
- Depend on a new object-store authority: rejected; unapproved critical path.
- WIT replacing the Spec 137 envelope: rejected; blast radius, breaks
  embedder-api `1.1.0`.
- Adapters required to be Wasm Components: rejected; forces Component Model
  into every embedder host for no contract benefit.
- Platform-sequenced releases: rejected; contradicts generic scoping.
