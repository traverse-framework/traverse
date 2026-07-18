# Traverse Hello World Example

This is the smallest governed example in the repo.

Use it when you want one command path that proves:

- a capability contract exists
- a packaged executable artifact exists
- a runtime request exists
- Traverse can inspect and execute the package locally

## Files

- contract:
  - `contracts/examples/hello-world/capabilities/say-hello/contract.json`
- workflow reference:
  - `workflows/examples/hello-world/say-hello/workflow.json`
- package:
  - `examples/hello-world/say-hello-agent/manifest.json`
- request:
  - `examples/hello-world/runtime-requests/say-hello.json`

## Run It

Build the deterministic local fixture:

```bash
bash examples/hello-world/say-hello-agent/build-fixture.sh
```

Inspect the package:

```bash
cargo run -p traverse-cli-rs -- agent inspect \
  examples/hello-world/say-hello-agent/manifest.json
```

Execute the package:

```bash
cargo run -p traverse-cli-rs -- agent execute \
  examples/hello-world/say-hello-agent/manifest.json \
  examples/hello-world/runtime-requests/say-hello.json
```

What good output looks like:

- `package_id: hello.world.say-hello-agent`
- `capability_id: hello.world.say-hello`
- `status: completed`
- `name: Traverse`
- `greeting: Hello, Traverse!`

## Validation

Run the example smoke path:

```bash
bash scripts/ci/hello_world_example_smoke.sh
```
