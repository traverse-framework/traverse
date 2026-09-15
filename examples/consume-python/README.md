# consume-python

Minimal example: call Traverse from Python by shelling out to the real CLI —
there is **no** native Python SDK yet.

This matches the documented public surface in [`docs/cli-reference.md`](../../docs/cli-reference.md):
`capability-package execute <manifest> <request.json>`.

## Prerequisites

- A Rust toolchain that can build this workspace (`cargo build -p traverse-cli-rs`), **or**
- An installed `traverse-cli` binary on `PATH` (or set `TRAVERSE_CLI` to its absolute path)

Python 3.9+ with the standard library only (`requirements.txt` is intentionally empty of deps).

## Run

From the repository root:

```bash
# One-time (if you do not already have traverse-cli on PATH)
cargo build -p traverse-cli-rs

python3 examples/consume-python/run_example.py
```

Or from this directory:

```bash
python3 run_example.py
```

The script resolves the CLI as:

1. `$TRAVERSE_CLI` if set
2. else `traverse-cli` on `PATH`
3. else `cargo run -q -p traverse-cli-rs --` from the repo root

## Expected stdout shape

Successful runs print the same fields as the CLI reference example for
`capability-package execute` against
`examples/capabilities/expedition-intent-agent/manifest.json` and
`examples/capabilities/runtime-requests/interpret-expedition-intent.json`:

```text
request_id: agent-interpret-expedition-intent-001
execution_id: exec_agent-interpret-expedition-intent-001
package_id: expedition.planning.interpret-expedition-intent
capability_id: expedition.planning.interpret-expedition-intent
capability_version: 1.0.0
status: completed
trace_ref: trace_exec_agent-interpret-expedition-intent-001
output:
{
  ...
}
```

Exact `output` JSON may evolve with the checked-in expedition fixture; `status: completed`
and the identity fields above are the stable shape to assert against.

## Honesty notes

- This is subprocess integration, not an embedded runtime or PyPI package.
- Do not invent CLI verbs such as `capability run` — use
  `capability-package execute` (or other commands listed in the CLI reference).
- Website FAQ updates that point here can land in a separate follow-up.
