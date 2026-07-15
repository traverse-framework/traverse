# Local Traverse Runtime Home

Traverse uses one predictable local runtime home for generated helper artifacts and adapter overlays:

- default path: `.traverse/local/`
- optional override: `TRAVERSE_RUNTIME_HOME=/absolute/path`

This is a local runtime workspace, not a source-of-truth location for governed artifacts.

## Directory Layout

The default local runtime home is expected to contain:

```text
.traverse/local/
  bin/
  fixtures/
  overlays/
```

### `bin/`

Runtime-owned generated helper binaries and package-local executable copies that are produced for local execution.

Examples:

- adapter helper binaries downloaded or generated for local development
- copied executable package artifacts that the local runtime needs to invoke deterministically

### `fixtures/`

Runtime-owned generated fixtures and transient execution byproducts that are helpful for local smoke paths or deterministic inspection.

Examples:

- generated fixture outputs derived from checked-in templates
- copied runtime traces or local execution snapshots that should not become governed source artifacts

### `overlays/`

Runtime-owned adapter configuration overlays for local development.

Examples:

- browser adapter local settings
- MCP adapter local settings
- device or sidecar-style local integration overlays

## What Stays Outside The Runtime Home

The local runtime home is intentionally not the place for user-authored governed artifacts.

These stay in checked-in source locations:

- contracts: `contracts/`
- workflows: `workflows/`
- registry bundles and runtime requests: `examples/`
- shared checked-in demo fixtures: `examples/fixtures/`

Executable capability packages may keep deterministic build outputs in package-local `./artifacts/` directories when those artifacts are part of the checked-in package shape. The local runtime home is for runtime-owned copies, caches, and overlays, not for replacing package-local layout.

## Ownership Rule

Treat the local runtime home as runtime-owned.

- the runtime may create, replace, or clean up files under `.traverse/local/`
- developers may inspect it for debugging
- developers should not treat files there as governed source artifacts

If a file is source-of-truth and should be reviewed, versioned, or merge-gated, it belongs in the repo’s governed artifact locations, not in the runtime home.

## Why This Exists

This keeps local execution reproducible:

- generated helpers are not scattered across arbitrary directories
- local adapter overlays have one predictable home
- smoke paths can reference one known runtime-owned location
- checked-in governed artifacts remain clearly separate from generated local runtime state

## Validation

Verify the documented runtime-home layout with:

```bash
bash scripts/ci/runtime_home_smoke.sh
```
