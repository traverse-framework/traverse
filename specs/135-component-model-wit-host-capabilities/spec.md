# Feature Specification: Component Model WIT Host Capabilities

**Status**: Approved (2026-09-10)
**Canonical governing ID**: `135-component-model-wit-host-capabilities`
**Version**: 0.1.0
**Extends**: `038-wasi-host-insulation`, `100-capability-package-authoring`,
`103-application-connector-binding`, `104-mediated-connector-invocation`, and
`1259-portable-authority-contracts`.
**Decision evidence**: Traverse #1340; Decision 79; ADR-0068.

## Purpose and boundary

Define a governed Component Model execution profile through which a portable
Wasm Component may import a narrowly scoped, versioned WIT interface implemented
by an explicitly activated target-local host. This is a composition and
authorization boundary, not a target API or a native adapter implementation.

The existing `core-wasm-v1` Host ABI v1 path remains unchanged. It continues to
validate its fixed WASI/`traverse_host` import whitelist. `component-wit-v1` is
a distinct profile; it does not broaden that whitelist or permit ambient core
Wasm imports.

Portable business logic uses neither interface where possible. Governed WASI is
for standardized sandbox runtime services. WIT is reserved for an unavoidable
host authority such as hardware, OS permission/lifecycle, secure identity or
storage, notifications, or similar target-local services. It MUST NOT be used
as a general-purpose plug-in, filesystem, provider, or application-workflow
escape hatch.

## Model

Traverse publishes a registry of narrow standard interfaces under
`traverse:platform`; it MUST NOT publish one ambient, catch-all `platform` or
`device` interface. An application selects an activated binding; a component
knows only its declared interface identity, never a host implementation,
operating system, provider, device identity, path, credential, or permission
mechanism.

The initial standard interface candidate is
`traverse:platform/recording-host@0.1.0`. Callweave's
`callweave:recording/recording-host@0.1.0` is input to this standardization, not
an automatically equivalent runtime alias. A migration adapter, if ever needed,
MUST be explicit and MUST NOT make package identities interchangeable.

## Requirements

- **FR-001 — Profiles**: The runtime MUST expose `core-wasm-v1` and
  `component-wit-v1` as separately versioned execution profiles. A profile
  mismatch MUST fail before execution with a stable, secret-free error.
- **FR-002 — Component metadata**: For `component-wit-v1`, Traverse MUST read
  the Component Model type metadata and enumerate every imported WIT package,
  interface, version, and exported operation shape. It MUST reject malformed
  metadata and imports not declared by the package manifest.
- **FR-003 — Manifest declaration**: A component package MUST declare each
  required WIT import by package namespace/name, interface name, and exact
  interface version. The declaration set and component import set MUST match
  exactly. Required imports are mandatory; no optional privileged import exists
  in v0.1.0.
- **FR-004 — Pinned compatibility**: Activation MUST require exact package,
  interface, version, and WIT operation-shape compatibility. Semver ranges,
  package aliases, and name-only matching are forbidden in v0.1.0.
- **FR-005 — Registration**: A host implementation registration MUST include a
  stable binding ID, WIT package/interface/version, supported precise target
  families, provenance/trust state, and conformance-evidence reference. A
  registry entry MAY publish registration metadata but MUST NOT activate native
  code or host authority automatically.
- **FR-006 — Activation and target ownership**: Only the application/embedder
  may activate a trusted local implementation and map it to an interface. The
  application owns logical placement (for example local execution); the host
  claims precise governed target families. The component MUST NOT choose a
  target, binding, OS, provider, or device at execution time.
- **FR-007 — Deterministic selection**: Multiple compatible registrations MAY
  exist, but the application MUST select an explicit binding for a capability
  profile/workflow. Missing, incompatible, target-mismatched, untrusted, or
  ambiguous bindings MUST fail before guest execution.
- **FR-008 — Errors and redaction**: Activation failures MUST use stable public
  codes: `component_model_profile_unsupported`, `wit_import_undeclared`,
  `wit_import_unfulfilled`, `wit_import_version_incompatible`,
  `wit_host_target_mismatch`, `wit_host_binding_ambiguous`, and
  `wit_host_activation_failed`. Host calls MUST return only WIT-defined public
  domain outcomes. Neither output nor diagnostics may contain raw audio, device
  identities, paths, endpoints, credentials, native error strings, entitlement
  names, or private configuration.
- **FR-009 — Evidence**: Traverse MUST record the profile, declared import
  identities, selected binding ID, abstract target family, compatibility result,
  and stable outcome code. Evidence MUST omit host-private implementation data.
- **FR-010 — Conformance**: Every registry-published implementation MUST carry
  shared WIT contract-test evidence. Official Traverse implementations also
  require target-specific integration evidence. Application-local hosts MAY be
  marked unverified but MUST be explicitly activated and never represented as a
  verified portable default.

## `traverse:platform/recording-host@0.1.0`

The first standard host capability is foreground recording only. Its initial
interface has `status`, `start`, `stop`, and `events`; `discard` is a required
follow-up before publishing a durable-reference contract unless the adopted WIT
already provides equivalent cancellation semantics.

- `status` is advisory and exposes only portable states.
- `start` is authoritative and may prompt only when the app-authenticated
  execution context is both user-initiated and foregrounded. Otherwise it
  returns `permission-required` without prompting.
- A host supports at most one active recording session per host context and
  returns `recording-busy` for another start.
- `events` is a bounded lifecycle stream only (`started`, `stopped`,
  `interrupted`, and public failures); it carries no audio/content bytes.
- `stop` succeeds only when its opaque recording reference is ready for its
  documented next host-authorized use.
- The host chooses microphone/device and audio format. The component receives
  no device ID, codec negotiation surface, path, URL, provider, or raw bytes.
- Recording references are host-managed, scoped, expiring opaque values. They
  are not transferable by default; cross-capability use requires an explicit
  application/workflow-mediated grant.
- `background-unsupported`, `permission-required`, `recording-busy`, and
  `unavailable` are portable domain outcomes. Background recording, raw audio
  streaming, codec profiles, multiple concurrent tracks, and direct reference
  transfer are out of scope.

## Manifest examples

```json
{
  "execution_profile": "component-wit-v1",
  "required_wit_imports": [{
    "package": "traverse:platform",
    "interface": "recording-host",
    "version": "0.1.0"
  }]
}
```

```json
{
  "wit_bindings": {
    "traverse:platform/recording-host@0.1.0": "default-local-recording"
  }
}
```

## Acceptance scenarios

1. A Component importing the declared `recording-host@0.1.0` validates,
   activates, and invokes `status`, `start`, `stop`, and lifecycle `events`
   through a compatible explicit fake local host.
2. A missing declaration, host, compatible version, matching target, or
   unambiguous explicit binding fails deterministically before guest execution.
3. A fake host returns `permission-required` and `background-unsupported` as
   public typed outcomes, while its private diagnostics never appear in guest
   output or trace evidence.
4. A second active session returns `recording-busy`; event queues are bounded
   and terminal lifecycle behavior is deterministic.
5. Existing `core-wasm-v1`/WASI Host ABI v1 fixtures pass unchanged.
6. A registry-declared host cannot become active without application activation;
   an application-local unverified fake is clearly distinguishable from an
   official conformance-verified host.

## Compatibility and non-goals

This is additive and does not modify Host ABI v1, existing core-Wasm artifacts,
the connector contracts in spec `1259`, or target-specific host implementations.
It does not implement macOS, iOS, Android, browser, filesystem, storage,
permission, microphone, audio byte, or provider APIs. It does not make WIT a
replacement for WASI or a general module-composition mechanism.

Migration: existing capabilities retain `core-wasm-v1`. A recording-dependent
capability is repackaged as a Component, declares the exact standard WIT import,
and runs only where an application activates a compatible host. Callweave may
keep its current package during transition, but it is not a Traverse-standard
default until an explicit adapter or republished interface is approved.
