# Registry Consumer Golden App

This application composes the published `traverse-starter.validate`,
`traverse-starter.process`, and `traverse-starter.summarize` capabilities. Every
component uses `registry_ref`; no component points at a checked-in contract or
WASM binary.

From the repository root, sync the public registry before validating or
registering the app:

```bash
cargo run -p traverse-cli-rs -- registry sync --workspace local-default --json
cargo run -p traverse-cli-rs -- app validate \
  --manifest examples/applications/registry-consumer/app.manifest.json \
  --workspace local-default \
  --json
cargo run -p traverse-cli-rs -- app register \
  --manifest examples/applications/registry-consumer/app.manifest.json \
  --workspace local-default \
  --json
```

The component versions and digests are pinned to the active `1.1.0` public
records. Deprecated `1.0.x` records are intentionally excluded.

`traverse-cli serve` dispatches app commands from the persisted
`registration.json` written by `app register`. It does not reopen the source
manifest or re-resolve `registry_ref` at startup. After register, prepare, and
optional activate:

```bash
cargo run -p traverse-cli-rs -- serve
```

If a command returns `app_registration_requires_refresh`, the workspace has a
legacy registration without a persisted state-machine declaration. Re-run
`app register` against the current manifest. There is no source-manifest
fallback or automatic migration.
