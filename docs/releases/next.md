# Next Release Notes

Unreleased changes since `v0.16.2` will be recorded here.

- **Breaking:** Swift, Kotlin and .NET guests now receive only the submitted
  input, not the `{target_id, input}` envelope, and a `target_id` that doesn't
  match the capability is rejected (Decision 124, `#1794`).
- Web capability input now reaches the guest as sent, even when it holds members
  like `kind: "app_command"` or `command` (Decision 125, `#1802`).
- `BundleEmbedder` apps can now get `runtime.wasm` from a release. Every cut
  publishes `runtime-wasm-vX.Y.Z`, and v0.16.1 and v0.16.2 are backfilled
  (Decision 123, `#1784`).
