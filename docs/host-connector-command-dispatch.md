# Host connector command dispatch (Spec 137)

Governed by
[`137-host-connector-command-dispatch`](../specs/137-host-connector-command-dispatch/spec.md)
and [ADR-0071](adr/0071-host-connector-command-dispatch.md). Schema version
**`1.0.0`**.

This is the public **app-runtime** port for invoking a manifest-selected,
explicitly activated host connector from an app state-machine command. It is
not guest WASM `traverse_host.connector_invoke` (Spec 104) and not the
Component WIT recording fake (Spec 135).

## Downstream consumption

Exact consumers of schema `1.0.0`:

| Consumer | Surface |
| --- | --- |
| Rust runtime | `traverse_runtime::host_connector_dispatch::dispatch_host_connector_command` |
| Web/TypeScript embedder | `packages/web/TraverseEmbedder` `hostConnectorCommand.ts` |
| Swift/macOS embedder | `packages/swift/TraverseEmbedder` `HostConnectorCommand.swift` |
| Shared JSON schema | `contracts/app-runtime/host-connector-command-1.0.0.json` |
| Connector contracts | `contracts/connectors/traverse.audio-input/`, `traverse.model-runtime/` |

Native microphone/model drivers and browser permission prompts stay in host
adapters. Those packages only share the command/event types.

## When to use this port

Use it when an application command must start `audio.permission.request`,
`audio.capture`, or `model.execute` (`local-model-runtime`) through an
activated Spec 103 binding. Do not add device, codec, provider, endpoint, or
credential fields to the public payload.

## Command

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
  "payload": { "max_duration_ms": 5000, "max_bytes": 1048576 }
}
```

`target_family` is `macos`, `browser`, or `local`. Bindings declare supported
target families via `placement_targets`; a family the activated binding does
not claim fails with `target_incompatible` before the adapter runs. There is
no native-only connector classification.

## Version / migration

- **1.0.0** is the first public schema. Add fields only with a minor bump
  that existing `1.0.0` readers can ignore; rename or change meaning only
  with a new `schema_version`.
- Existing capability `invoke` commands (Spec 059 / 052) are unchanged.
- Guest `connector_invoke` ABI version is unchanged.
- Callweave `callweave:recording/recording-host@0.1.0` is not an alias for
  this port. Map Callweave recording to `capture_audio` → `audio.capture`
  in the application manifest.

## Verification

```bash
cargo test -p traverse-runtime --lib host_connector_dispatch
```
