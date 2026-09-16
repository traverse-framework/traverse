# Feature Specification: Host Connector Command Dispatch

**Feature Branch**: `codex/issue-1384-host-connector-command-dispatch`
**Created**: 2026-09-14
**Status**: Approved (2026-09-14); **0.2.0 amend approved** 2026-09-16
  (Decisions 91–92 / Spec 138)
**Canonical governing ID**: `137-host-connector-command-dispatch`
**Version**: 0.2.0
**Extends**: `059-http-command-dispatch`, `103-application-connector-binding`,
`104-mediated-connector-invocation`, `135-component-model-wit-host-capabilities`,
`1259-portable-authority-contracts`, and (for `model.execute` semantics)
`138-governed-exact-model-execution`.
**Decision evidence**: Traverse #1384; Decision 84; ADR-0071; Decisions 91–92;
ADR-0074 (Draft).
**Input**: Callweave recording-to-analysis workflow contract, commit `0acc2e8`;
Callweave exact-ref model-execution slice (Decisions 91–92).

## Purpose and boundary

Define the smallest public Traverse **app-runtime** dispatch surface for
invoking a manifest-selected, explicitly activated host connector from an app
state-machine command. Clients send commands; the runtime resolves the
manifest binding and calls a host-owned adapter. Guests never receive host
private configuration, device identity, raw bytes, or provider endpoints.

This is not the guest WASM path. `traverse_host.connector_invoke` remains the
only WASM guest-to-host connector ABI (Spec 104) and is out of this surface.
The Component Model `component-wit-v1` recording-host fake (Spec 135) is a
distinct profile and MUST NOT be used here.

The first bounded operation is `audio.capture` on `traverse.audio-input`.
`traverse.model-runtime` (`model.execute`, the `local-model-runtime` port)
uses the same request/result/event/error **kinds**. Exact-ref model package
manifests, guest ABI, Spec 526 package binding, host stage/read APIs, and
inference field semantics are governed by Spec 138.

## Relationship to existing specifications

| Specification | Relationship |
| --- | --- |
| 059-http-command-dispatch | Commands remain the client trigger. This spec adds host-connector invoke, not a new HTTP verb. |
| 052-app-state-machine | Command names are state-machine `on` values. Connector invoke is additive to capability `invoke`. |
| 103-application-connector-binding | Dispatch resolves a declared, non-secret binding. Missing or incompatible bindings fail closed. |
| 104-mediated-connector-invocation | Guest ABI is unchanged. This surface is host-side, for app commands. |
| 135-component-model-wit-host-capabilities | WIT profile and its fake host stay separate. No automatic Callweave alias. |
| 1259-portable-authority-contracts | Audio-input is native-only. Model-runtime is vendor-neutral. Envelopes stay generic. |
| 138-governed-exact-model-execution | Owns model package manifest, guest ABI, Spec 526 package binding, staging APIs, and `model.execute` inference fields. |
| 044-application-bundle-manifest | Exact model pins live in `exact_model_dependencies`. |
| 045-governed-model-dependency-resolution | Separate LLM/candidate track; not this port’s exact-ref path. |
| 057 / 068 | Browser and macOS embedders share this command/event contract; adapters stay host implementations. |

## Model

1. The application manifest binds `traverse.audio-input` and/or
   `traverse.model-runtime` by connector id, compatible version, and a
   non-secret `config_ref`, and routes a state-machine command to an
   operation (`audio.capture` or `model.execute`).
2. The embedding host **explicitly activates** a named binding. Registry
   presence never grants authority.
3. `dispatch_host_connector_command` validates the typed command envelope,
   resolves the binding, enforces target/limits/cancellation/idempotency,
   then calls the host adapter.
4. The adapter returns an **opaque artifact reference** plus public events.
   Raw audio, paths, device names, credentials, and native error strings
   never cross the public contract.

## Requirements

- **FR-001 — Public port**: Traverse MUST expose
  `dispatch_host_connector_command` as the public app-runtime API for this
  slice. It MUST NOT call `traverse_host.connector_invoke` and MUST NOT use
  the Spec 135 Component WIT fake.
- **FR-002 — Command envelope**: A command MUST be the typed
  `host_connector_command` document with `schema_version` `1.0.0`,
  `command` (state-machine `on` value), `command_id`, `correlation_id`,
  `idempotency_key`, `target_family`, optional `cancel_requested`, and a
  bounded `payload`. Unknown kinds or schema versions fail closed.
- **FR-003 — Manifest routing**: The manifest MUST map `command` to exactly
  one `connector_id` + `operation`. Unknown commands return
  `unknown_command`. Duplicate command routes fail at dispatch
  construction.
- **FR-004 — Binding resolution**: Dispatch MUST select the unique declared
  binding for that connector id. Missing, duplicate, version-incompatible,
  or unconfigured (`config_ref` empty) bindings fail with `unbound`,
  `incompatible`, or `unconfigured`.
- **FR-005 — Explicit activation**: The selected binding MUST already be
  activated by the host/application. An unactivated binding fails with
  `unbound` before the adapter runs.
- **FR-006 — Target neutrality**: Command, result, and event field names
  are identical for `browser` and `macos`. `traverse.audio-input` is
  native-only: a `browser` target with a local/native binding MUST fail
  with `target_incompatible` before capture. Host adapters remain
  target-specific implementations behind this contract.
- **FR-007 — Limits**: Runtime MUST reject oversized payloads and
  out-of-range duration/size fields with `input_limit_exceeded` without
  invoking the adapter. Audio capture payload MUST declare
  `max_duration_ms` and `max_bytes` within published ceilings.
  `model.execute` payload MUST declare Spec 138 ceilings (including
  `max_output_bytes`) within published, policy, and manifest ceilings.
- **FR-008 — Cancellation**: `cancel_requested: true` MUST return
  `cancelled` without adapter side effects when cancellation is observed
  before invoke. Adapter-observed cancellation uses the same public code.
- **FR-009 — Idempotency**: A repeated `idempotency_key` with an equivalent
  command/operation/payload MUST return the original result without a
  second adapter invoke. A conflicting payload with the same key MUST
  return `idempotency_conflict`.
- **FR-010 — Opaque references**: Success payload MAY include `artifact_ref`
  (audio) or `output_ref` (model) as a host-managed opaque string. It MUST
  NOT include raw audio, tensor bytes, file paths, URLs, device identifiers,
  provider names, or credentials.
- **FR-011 — Events and errors**: Dispatch MUST emit a bounded, ordered
  public event list (`accepted`, `started`, `completed` / `cancelled` /
  `failed`). Errors use stable secret-free codes: `unknown_command`,
  `unbound`, `incompatible`, `unconfigured`, `target_incompatible`,
  `input_limit_exceeded`, `cancelled`, `idempotency_conflict`,
  `policy_denied`, and `unavailable`.
- **FR-012 — Evidence and redaction**: Evidence records connector id and
  version, operation, target family, binding id, `config_ref` **name**,
  correlation id, result class, and outcome code. It MUST omit values,
  credentials, endpoints, paths, device identities, raw audio, native
  error strings, and host-private diagnostics.
- **FR-013 — Same port**: `model.execute` on `traverse.model-runtime` MUST
  use the same command/result/event/error **kinds** as `audio.capture`.
  Provider, endpoint, credential, and free model-selection fields are
  forbidden. Payload MUST include must-match `model_ref`, host-staged
  `input_ref`, execution `policy_ref`, required `data_classification`, and
  Spec 138 schema/limit fields. Tensor bytes MUST NOT appear as unbounded
  JSON base64 on this port.
- **FR-014 — Downstream docs**: API and version/migration documentation
  MUST name the exact public types, schema version, and embedder packages
  that consume this contract.

## Command envelope

```json
{
  "kind": "host_connector_command",
  "schema_version": "1.0.0",
  "command": "capture_audio",
  "command_id": "cmd-00000001",
  "correlation_id": "corr-00000001",
  "idempotency_key": "idem-00000001",
  "target_family": "macos",
  "cancel_requested": false,
  "payload": {
    "max_duration_ms": 5000,
    "max_bytes": 1048576
  }
}
```

## Result envelope

```json
{
  "kind": "host_connector_result",
  "schema_version": "1.0.0",
  "result_class": "succeeded",
  "command_id": "cmd-00000001",
  "correlation_id": "corr-00000001",
  "connector_id": "traverse.audio-input",
  "operation": "audio.capture",
  "binding_id": "default-local-audio",
  "target_family": "macos",
  "artifact_ref": "audio-ref-1",
  "error": null,
  "events": [],
  "evidence": {}
}
```

## Acceptance scenarios

1. An app command `capture_audio` with an activated compatible
   `traverse.audio-input` binding on `macos` dispatches `audio.capture`
   through a fake host and returns an opaque `artifact_ref`.
2. The same command with a missing, incompatible, unconfigured, or
   unactivated binding fails before the fake host runs.
3. A `browser` target with a native-only audio binding fails with
   `target_incompatible`. The command/event field names match macos.
4. Oversized duration/bytes/payload fail with `input_limit_exceeded`.
5. `cancel_requested` and idempotent replay/conflict behave as specified.
6. Structured events and errors contain no host-private leakage.
7. `model.execute` succeeds through the same port with a distinct
   activated `traverse.model-runtime` binding and a must-match `model_ref`
   that equals an app-manifest `exact_model_dependencies` pin (Spec 138).
8. Guest `connector_invoke` and Spec 135 WIT fakes are unused.
9. `model.execute` payloads that omit required Spec 138 fields, mismatch
   the pin, include provider/endpoint/credential fields, or attempt
   unbounded base64 tensors fail closed before the adapter runs.

## Compatibility and non-goals

Additive. Does not modify Host ABI v1, Spec 104 guest mediation, Spec 135
WIT activation, HTTP route shape, or production microphone/model drivers.
Native and browser adapters remain host implementations; this slice ships
the shared contract plus a fake host for tests.

Downstream consumption: `traverse-runtime` public API,
`packages/web/TraverseEmbedder` wire types, and
`packages/swift/TraverseEmbedder` wire types, all at schema `1.0.0`.
