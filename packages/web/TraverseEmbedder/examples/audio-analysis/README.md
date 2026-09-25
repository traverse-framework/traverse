# Browser audio-analysis example

This is a deliberately thin browser shell for the `audio-analysis` application
bundle. It registers the Spec 140 browser audio adapter, submits Spec 139
`app_command` envelopes, and renders only events emitted by `runtime.wasm`.
It contains no client-side transition table, result routing, or sidecar.

Build and serve it from the web package:

```bash
npm run build
node examples/audio-analysis/server.mjs
```

Open `http://127.0.0.1:4177`, grant microphone permission, capture a bounded
recording, then select Analyze. The host adapter stages capture bytes and
returns an opaque artifact reference; the runtime resolves the reference before
invoking the registered `doc-approval.analyze` capability. The event panel is
the sole UI state source.

The application bundle requires the current certified `runtime/runtime.wasm`.
Run `bash scripts/ci/native_artifact_certification.sh` to build and validate a
production artifact before manually exercising this example.
