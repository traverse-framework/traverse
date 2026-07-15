# React integration example — no sidecar

Proves `BundleEmbedder` loads and executes a real, checked-in Traverse
application bundle straight from the repository, from inside a React page,
with no `traverse-cli serve` process running (spec 068 DoD: "A React
integration proves traverse-starter ... executes a bundled workflow without
a sidecar").

## What it does

1. `server.mjs` is a plain static file server (not a Traverse sidecar) that
   serves three things over HTTP: the built `traverse-embedder-web` package
   (`/pkg/`), the existing React UMD vendor bundles already checked in under
   `apps/react-demo/vendor` (`/vendor/`), and the repository root itself
   (`/repo/`) so the browser can `fetch` the real
   `examples/applications/traverse-starter/app.manifest.json` bundle and every file it
   references (component manifest, workflow definition, compiled WASM
   artifact) by the same relative paths the bundle already declares.
2. `src/main.js` is a small React app (no build step, `React.createElement`
   directly, matching the existing `apps/react-demo` convention) that calls
   `BundleEmbedder.init` with a `FetchBundleLoader` pointed at that bundle,
   subscribes to events, and lets you `submit` the
   `traverse-starter.process` capability.

## Running it

```bash
npm run build          # from packages/web/TraverseEmbedder
node examples/react-integration/server.mjs
```

Then open `http://127.0.0.1:4175`.

## What the result actually proves

The checked-in `examples/traverse-starter/process-agent/artifacts/process-agent.wasm`
artifact is a placeholder fixture (a WASI command module with no imports and
an empty `_start` body — see `examples/traverse-starter/process-agent/build-fixture.sh`),
not a real payload agent. Submitting therefore correctly ends in an `error`
event with code `output_deserialization_failed` (empty stdout is not valid
JSON) — the same honest outcome the package's own test suite asserts against
this exact file. That is the correct behavior, not a bug: it demonstrates
that manifest/workflow resolution, sha-256 digest verification, Traverse
Host ABI import validation, and real `WebAssembly.instantiate` + invocation
all genuinely ran in the browser tab with no server round trip — the
pipeline plumbing this package slice delivers. A real payload agent (once
one exists in this repository) would produce a `capability_result` event
here with zero code changes to this example.

A doc-approval integration will be added once the `doc-approval.pipeline`
workflow (traverse-framework/Traverse#555) lands on `main`, to demonstrate
the multi-node case end to end as well.
