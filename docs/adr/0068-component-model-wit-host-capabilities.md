# ADR-0068: Versioned Component Model WIT Host Capabilities

- Status: Accepted
- Date: 2026-09-10
- Governing spec: `135-component-model-wit-host-capabilities` (Approved)
- Related issue: #1340
- Extends: Decision 6; ADR-0060; ADR-0063

## Context

Traverse Host ABI v1 intentionally validates a fixed core-Wasm/WASI import
surface. A portable capability that needs a microphone, OS permission,
foreground lifecycle, secure native storage, or similar target-local authority
cannot safely express that need through ambient imports, a target-specific
capability, or an ever-growing ABI v1 whitelist. Callweave proposed a
`recording-host` WIT import as a concrete need.

## Decision

Introduce `component-wit-v1` as a profile separate from `core-wasm-v1`; do not
change Host ABI v1. Component imports are validated against exact manifest WIT
declarations and resolved only through application-activated host bindings. The
runtime owns validation, selection, target compatibility, error redaction,
evidence, and conformance policy; hosts own target implementation details.

Traverse standardizes a registry of small `traverse:platform` WIT interfaces.
The application may select a trusted default or named compatible binding, while
the component sees only the interface identity. Standard recording is
foreground-only and lifecycle/reference oriented, never a raw-audio or native
device API.

## Consequences

Component execution requires new type-metadata validation and host-linking
support, plus manifest, registry, embedder, trace, and conformance-test work.
Existing core-Wasm artifacts and the Host ABI v1 whitelist retain their exact
behavior. Native adapter code remains outside this governing slice.

## Alternatives considered

- Add arbitrary WIT imports to Host ABI v1: rejected because it destroys the
  fixed-whitelist core-Wasm security/compatibility boundary.
- One generic `platform`/`device` WIT interface: rejected because it becomes
  ambient authority and makes auditing/versioning incoherent.
- Keep `callweave:recording` as an automatic Traverse-standard alias: rejected
  because package identity and contract governance must be unambiguous.
- Use WASI for recording: rejected because WASI does not model permission,
  microphone, device selection, or mobile/browser lifecycle portably.
- Let components choose targets or hosts at runtime: rejected because guest
  code must not route itself to more privileged native authority.
- Auto-activate registry-discovered native hosts: rejected because installation
  must not silently grant microphone or other native authority.
