# consume-typescript

Minimal example: consume `traverse-embedder-web` from plain Node.js + TypeScript — no React, no browser.

## Target

Node.js, using `NodeFsBundleLoader`. This does not run in a browser — the browser path uses `FetchBundleLoader` instead (see `packages/web/TraverseEmbedder/examples/react-integration/` for that).

## Prerequisites

This example depends on `packages/web/TraverseEmbedder` via a local `file:` path. That package's `dist/` output is not committed to the repo, so it must be built first.

```bash
cd packages/web/TraverseEmbedder
npm install
npm run build
```

## Install & run

```bash
cd examples/consume-typescript
npm install
npm start
```

## Expected output

Prints the events emitted while loading and executing the checked-in `traverse-starter` bundle (`examples/applications/traverse-starter/`), then the final submit status:

```
[capability_invoked] {...}
[capability_result] {"status":"completed","output":{...}}

submit status: accepted
```
