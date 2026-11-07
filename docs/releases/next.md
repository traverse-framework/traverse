# Next Release Notes

Unreleased changes since `v0.16.2` will be recorded here.

- **Breaking (web):** compatible capabilities now run one instance at a time
  through `runtime.wasm` on every host, and report the runtime's error codes:
  `compatible_instance_not_active` replaces `instance_not_found` and
  `instance_not_running`. The started `state_changed` event's `input` is the
  canonical input (Decision 128, `#1820`).
- `runtime.wasm` now runs the compatible-capability start, stop and kill
  lifecycle instead of returning `-1`. It enforces the `platforms[]`
  allowlist and returns canonical start input. The hosts switch over in
  `#1820` (Decision 127, `#1819`).
- Swift, Kotlin and .NET embedders now pass capability input to the runtime
  unchanged, so input Spec 1722 can't represent is rejected instead of altered
  (Decision 126, `#1795`).
- Native hosts can now read the model's output frame after `model.execute`,
  through `ExactModelMediatedConnector::activated()` (`#1785`).
- **Breaking:** Swift, Kotlin and .NET guests now receive only the submitted
  input, not the `{target_id, input}` envelope, and a `target_id` that doesn't
  match the capability is rejected (Decision 124, `#1794`).
- Web capability input now reaches the guest as sent, even when it holds members
  like `kind: "app_command"` or `command` (Decision 125, `#1802`).
- `BundleEmbedder` apps can now get `runtime.wasm` from a release. Every cut
  publishes `runtime-wasm-vX.Y.Z`, and v0.16.1 and v0.16.2 are backfilled
  (Decision 123, `#1784`).
