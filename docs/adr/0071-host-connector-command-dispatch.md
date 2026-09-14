# ADR-0071: App-Command Dispatch for Manifest-Selected Host Connectors

- Status: Accepted
- Date: 2026-09-14
- Governing spec: `137-host-connector-command-dispatch` (Approved)
- Related issue: #1384
- Extends: ADR-0039; ADR-0060; ADR-0068

## Context

Callweave's recording-to-analysis workflow needs to start capture from an
application state-machine command. Spec 104 already mediates guest WASM
connector calls, and Spec 135 already fakes a Component WIT recording host.
Neither is the app-runtime port: guests must not own microphone or model
authority, and the WIT fake is a different execution profile.

Applications already declare Spec 103 connector bindings. What was missing
was a typed, target-neutral command envelope that resolves those bindings
and calls an explicitly activated host adapter.

## Decision

Add a public `dispatch_host_connector_command` port on the app runtime.
Commands name a state-machine transition; the runtime maps them to a
manifest-selected connector operation (`audio.capture` first,
`model.execute` on the same envelopes). Host adapters implement capture or
local model execution. Browser and macOS share the wire contract; a
native-only audio binding is rejected on `browser` before invocation.

Do not route this surface through `traverse_host.connector_invoke` or the
Spec 135 WIT fake. Keep opaque artifact references, limits, cancellation,
idempotency, and redacted evidence on the public envelopes.

## Consequences

Embedders can dispatch capture without giving WASM guests host authority.
Production microphone and model drivers stay out of this slice; fake hosts
prove the contract. Downstream TypeScript and Swift packages consume the
same schema `1.0.0` types.

## Alternatives considered

- Reuse guest `connector_invoke` from HTTP/app commands: rejected because
  that ABI is guest-authorized and capability-declared, not app-command
  owned.
- Reuse the Spec 135 WIT fake as the app capture port: rejected because
  WIT is a Component profile, not the Spec 1259 connector contract.
- Let commands carry device, codec, or provider fields: rejected because
  it breaks portability and redaction.
- Separate audio and model public APIs: rejected because the issue
  requires one port with `local-model-runtime` following `audio.capture`.
