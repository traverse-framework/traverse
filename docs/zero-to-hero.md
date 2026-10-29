# Zero To Hero (Under 5 Minutes)

This is the fastest path to a first end-to-end success with Traverse.

Golden path (executive decision): **local CLI + browser host**.

You will prove two things:

1. The local CLI can build and execute one governed hello-world capability.
2. A browser app can consume Traverse through the live browser subscription path and receive governed runtime updates.

This guide is intentionally narrow. If you want more context, start with:

- [quickstart.md](../quickstart.md) (browser app-consumable flow)
- [docs/getting-started.md](./getting-started.md) (capability/registry layer)

## Prerequisites

- Rust 1.95+
- Node.js 20+

From the repository root:

```bash
bash scripts/validate-setup.sh
```

## Step 1: Local Hello World (CLI)

Run the governed hello-world agent example:

```bash
bash scripts/ci/hello_world_example_smoke.sh
```

What success looks like:

- status is `completed`
- the output JSON contains `"greeting": "Hello, Traverse!"`

## Step 2: Browser Host (App-Consumable)

Run the first supported browser-consumption path end-to-end:

```bash
bash scripts/ci/react_demo_live_adapter_smoke.sh
```

What success looks like:

- the production server starts and reports a listening address
- the React demo starts
- the smoke test sees a completed request and a trace artifact

## One-Command Validation (CI Equivalent)

To validate the full zero-to-hero path the same way CI does:

```bash
bash scripts/ci/zero_to_hero_acceptance.sh
```

## Scaffold A New Hello World Capability

If you want to author your own new hello-world capability package, use:

```bash
bash scripts/scaffold/hello_world_agent_scaffold.sh
```

That script creates a copy of the checked-in template in a temporary directory and prints the next steps.

