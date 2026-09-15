# consume-python

Minimal example: run a governed Traverse WASM capability from Python via
`traverse-cli` — no server, no SDK.

## Honest limitation

There is **no** native Python SDK for Traverse yet. This example shells out
to the real `traverse-cli` binary using the documented `capability-package
execute` command (see [`docs/cli-reference.md`](../../docs/cli-reference.md));
it does not embed Wasmtime or call into Traverse from Python directly.

## Prerequisites

- A Rust toolchain (`cargo`) able to build `traverse-cli-rs` — see the repo
  root [`README.md`](../../README.md) for the pinned version.
- Python 3. `requirements.txt` is intentionally empty — the script uses only
  the standard library, so there is nothing to `pip install`.

## Run

From the repo root:

```bash
python3 examples/consume-python/run_example.py
```

The first run builds `traverse-cli-rs` via `cargo run` (this can take a few
minutes); subsequent runs reuse the build and are fast.

## What it does

Runs the checked-in `examples/capabilities/expedition-intent-agent`
capability package against
`examples/capabilities/runtime-requests/interpret-expedition-intent.json`,
equivalent to:

```bash
cargo run -p traverse-cli-rs -- capability-package execute \
  examples/capabilities/expedition-intent-agent/manifest.json \
  examples/capabilities/runtime-requests/interpret-expedition-intent.json
```

## Expected output

```
request_id: agent-interpret-expedition-intent-001
execution_id: <generated execution id>
package_id: expedition.planning.interpret-expedition-intent
capability_id: expedition.planning.interpret-expedition-intent
capability_version: 1.0.0
status: completed
trace_ref: <generated trace reference>
output:
{
  ...capability-specific result fields...
}
```

## Out of scope

- Publishing a PyPI package
- Embedding Wasmtime in Python without the runtime
- Changing the CLI surface
