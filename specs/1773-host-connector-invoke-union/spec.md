# Feature Specification: Canonical Host-Connector Invoke Union

**Feature Branch**: `codex/issue-1773-host-connector-invoke-spec`
**Created**: 2026-11-02
**Status**: Approved (2026-11-03; maintainer approval recorded in #1773)
**Canonical governing ID**: `1773-host-connector-invoke-union`
**Version**: 1.0.0
**Amends on approval**: `044-application-bundle-manifest`,
`046-public-cli-app-registration`, and
`139-embedder-app-state-machine-execution`
**Input**: #1772 and the owner decision recorded there on 2026-11-02.

## Purpose

Define the one canonical typed manifest representation for a state-machine
invoke. It removes the mismatch in which raw validation/runtime accepts a
host-connector invoke while the public CLI typed parser only accepts a
capability invoke.

The change is additive for existing capability manifests and does not add a
new authority, connector, guest ABI, or host adapter.

## Canonical manifest representation

Every state that has `invoke` MUST contain exactly one of these variants:

```json
{
  "invoke": {
    "capability_id": "com.example.transcribe",
    "input_from": "host_connector_result.artifact_ref"
  }
}
```

```json
{
  "invoke": {
    "host_connector": "audio.permission.request"
  }
}
```

`input_from` remains meaningful only for the capability variant and retains
the bounds and staged-artifact rules of Spec 139. `host_connector` is the
declared connector command name consumed by Spec 137; it is not a credential,
configuration reference, endpoint, device identifier, or arbitrary adapter
method.

## Requirements

- **FR-001 Exact-one union**: The application-manifest parser, raw
  validator, CLI, workspace materializer, runtime, and every supported
  embedder MUST accept precisely one variant above. They MUST reject no
  variant, both variants, an empty identifier, an empty object, or unknown
  invoke fields with stable secret-free diagnostics.
- **FR-002 Capability compatibility**: A previously valid capability-only
  invoke retains its normalized meaning and serialized field names. No
  capability declaration or component validation rule changes in this slice.
- **FR-003 Connector governance**: A host-connector invoke MUST name a
  connector command declared by the application and MUST resolve through
  Specs 104 and 137. Binding presence, version compatibility, explicit host
  activation, payload ceilings, policy, cancellation, and redaction remain
  governed there and fail closed.
- **FR-004 CLI parity**: `traverse-cli app validate`, activation, and
  registration MUST use the same normalized union and must not reject a valid
  host-connector invoke for a missing `capability_id`. Their JSON diagnostics
  MUST be deterministic, stable, and secret-free.
- **FR-005 Runtime and embedder parity**: Rust, web, Swift, and Kotlin MUST
  consume equivalent normalized invoke semantics. A platform MUST NOT invent
  a target-specific reinterpretation of raw manifest fields.
- **FR-006 Terminal routing**: Host-connector waits route only the Spec 139
  terminal outcomes `host_connector_succeeded`, `host_connector_failed`,
  `host_connector_timeout`, and `host_connector_cancelled`. Capability
  outcomes remain unchanged.
- **FR-007 Offline and persistence boundary**: After required Registry/cache
  preparation, validation, activation, and registration of the union require
  no network access. Persisted registration state and diagnostics MUST omit
  credentials, private connector configuration, filesystem paths,
  authorization headers, device identities, and native error text.

## Acceptance scenarios

1. A manifest with a declared `audio.permission.request` host connector
   validates, activates, and registers without requiring `capability_id`.
2. Existing capability-only manifests validate and serialize as before.
3. Both variants, neither variant, an empty value, and an unknown invoke field
   fail at manifest validation with the documented stable redacted code.
4. An undeclared connector command or incompatible connector version fails
   before dispatch; an unactivated binding fails closed under Spec 137.
5. A host-connector invocation produces distinct success, failure, timeout,
   and cancellation terminal events on Rust, web, Swift, and Kotlin.
6. An offline prepared cache exercises validate, activate, and register with
   zero network calls.

## Verification and Definition of Done

- Parser unit fixtures cover both variants and every invalid combination.
- CLI fixtures cover validate, activate, register, offline operation,
  undeclared connector, and incompatible version.
- Runtime fixtures cover all four host-connector terminal outcomes.
- Web, Swift, and Kotlin manifest-loading/conformance suites prove the same
  normalized semantics.
- Regression evidence covers Specs 044, 046, 104, 137, and 139.
- The governing amendments and this draft have maintainer approval before
  #1772 or any implementation child becomes Ready.
- `approved-specs.json` is updated only after that approval is evidenced.

## Non-goals

- Generic model connector implementation (#1719).
- Production signing-key custody (#1567).
- Session recovery or interruption semantics (#1470 and #1582).
- New device adapters or browser emulation of native-only authorities.
- Changes to the Spec 104 guest ABI or Spec 137 host dispatch policy.

## Governing relationships

| Specification | Relationship |
| --- | --- |
| 044 | Adds the app-manifest schema and fail-closed union validation. |
| 046 | Adds public CLI validation, activation, registration, and diagnostic parity. |
| 139 | Makes this the canonical state-machine invoke representation and terminal-routing contract. |
| 104 | Remains the unchanged mediated guest-connector authority boundary. |
| 137 | Remains the unchanged host command dispatch, binding, activation, and policy boundary. |

## Issue mapping

- #1773 — approve this governing specification.
- #1772 — implement the approved union in bounded parser, runtime, and
  platform-conformance slices.
