# Traverse Browser Adapter Explainer

The Traverse browser adapter is the local host-facing bridge that exposes governed runtime subscription behavior to a browser app.

It is not a second runtime and it does not redefine Traverse semantics. It is an adapter layer over already-governed runtime behavior.

## What The Browser Adapter Is For

Use the browser adapter when you want:

- a browser-hosted app to start from the approved app-consumable path
- governed runtime state updates delivered over a concrete local transport
- a live local bridge between the browser consumer and the Traverse runtime

The current approved path is the local adapter used by the app-consumable quickstart and React demo.

Relevant docs:

- [quickstart.md](../quickstart.md)
- [docs/youaskm3-canonical-app-http-path.md](youaskm3-canonical-app-http-path.md)
- [docs/app-consumable-entry-path.md](app-consumable-entry-path.md)
- [https://github.com/traverse-framework/App-References/tree/main/apps/browser-consumer/README.md](https://github.com/traverse-framework/App-References/tree/main/apps/browser-consumer/README.md)
- [docs/adapter-boundaries.md](adapter-boundaries.md)

## What The Browser Adapter Is Not

The browser adapter is not:

- a separate execution model
- a replacement for the core runtime
- a place to redefine subscription message meaning
- a generic deployment abstraction for every host target

The runtime still owns:

- request validation
- execution
- state progression
- trace artifacts
- subscription payload meaning and ordering

The adapter only owns how those governed surfaces are exposed to a browser-capable host path.

## Responsibilities

In the current supported path, the browser adapter is responsible for:

- binding a local HTTP-facing subscription surface
- accepting the approved browser subscription request shape
- creating the concrete stream for browser consumers
- relaying the governed ordered runtime messages
- surfacing setup or stream errors through the documented local adapter path

It should not invent new runtime states, custom message formats, or host-only execution semantics.

## When To Use It

Use the browser adapter when:

- your app is browser-hosted
- you want a live local consumer path rather than an offline preview
- you need ordered runtime updates, trace visibility, and terminal results in the UI

Examples:

- the checked-in React demo
- the browser-consumer package
- a downstream shell such as `youaskm3`

## Host-Target Comparison

### Browser Adapter

Use when:

- the consumer is a browser-hosted app
- you need live subscription updates in the UI
- you are following the approved app-consumable flow

Primary docs:

- [quickstart.md](../quickstart.md)
- [docs/app-consumable-entry-path.md](app-consumable-entry-path.md)
- [docs/youaskm3-canonical-app-http-path.md](youaskm3-canonical-app-http-path.md)

### MCP Stdio Server

Use when:

- the consumer is an MCP client or agent
- discovery and execution should happen through the governed MCP surface
- you do not need the browser subscription transport

Primary docs:

- [docs/mcp-stdio-server.md](mcp-stdio-server.md)
- [docs/mcp-consumption-validation.md](mcp-consumption-validation.md)

### Direct CLI And Authoring Paths

Use when:

- you are developing contracts, workflows, examples, or executable packages
- you need inspection, registration, or local validation flows
- you are not building a browser host surface yet

Primary docs:

- [docs/getting-started.md](getting-started.md)
- [docs/cli-reference.md](cli-reference.md)
- [docs/expedition-example-authoring.md](expedition-example-authoring.md)

### Packaged Runtime And Consumer Bundle

Use when:

- you are integrating Traverse as a release-facing downstream dependency
- you need the published runtime and MCP artifact story, not just source checkout instructions

Primary docs:

- [docs/app-consumable-consumer-bundle.md](app-consumable-consumer-bundle.md)
- [docs/packaged-traverse-runtime-artifact.md](packaged-traverse-runtime-artifact.md)
- [docs/packaged-traverse-mcp-server-artifact.md](packaged-traverse-mcp-server-artifact.md)

## Practical Rule

If your question is “how does a browser app receive governed live runtime updates?”, the browser adapter is the right document to start with.

If your question is “how does Traverse execute or govern the behavior underneath that stream?”, start with the runtime, app-consumable, or adapter-boundary docs instead.
