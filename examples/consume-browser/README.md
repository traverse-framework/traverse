# consume-browser

Minimal browser example for `traverse-embedder-web`: no React, no bundler, and no
`traverse-cli serve` sidecar.

## What it does

`index.html` maps the bare `traverse-embedder-web` import to the package's local
`dist/index.js`. `main.js` then uses `FetchBundleLoader` to load the checked-in
`traverse-starter` application bundle over the same static HTTP origin, subscribes
to embedder events, and submits `traverse-starter.process`.

The package is published to npm, but this checkout example deliberately uses the
local build so the SDK and checked-in bundle come from the same revision.

## Build and run

Build the web embedder first:

```bash
cd packages/web/TraverseEmbedder
npm install
npm run build
```
From the repository root, serve the checkout over HTTP:

```bash
python3 -m http.server 4177 --bind 127.0.0.1
```

Open `http://127.0.0.1:4177/examples/consume-browser/`.

## Expected output

The status line should become `submit status: accepted`. The event panel should
then show `capability_invoked` followed by `capability_result`; with the current
checked-in `traverse-starter` fixture, the result completes successfully and
includes the starter note metadata.

This example proves the browser-hosted WASM path against the repository fixture.
It does not demonstrate an npm-installed production app or a remote bundle, and
its exact result payload may evolve when the checked-in starter fixture changes.
