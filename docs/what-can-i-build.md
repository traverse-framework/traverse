# What Can I Build With Traverse Today?

Traverse is already useful as a governed runtime and MCP substrate for several concrete patterns.

This page answers the practical question a new developer asks first:

> What kinds of apps, agents, and systems can I build with Traverse right now?

## Supported Today

### 1. A Browser-Hosted App That Uses Traverse For Runtime And MCP

You can build a web app that owns the UI while Traverse owns:

- runtime execution
- workflow/state progression
- trace generation
- MCP-facing behavior

Relevant docs:

- [quickstart.md](../quickstart.md)
- [docs/app-consumable-entry-path.md](app-consumable-entry-path.md)
- [https://github.com/traverse-framework/reference-apps/tree/main/apps/browser-consumer/README.md](../https://github.com/traverse-framework/reference-apps/tree/main/apps/browser-consumer/README.md)
- [docs/youaskm3-integration-validation.md](youaskm3-integration-validation.md)

Good fit:

- a React or browser-hosted shell like `youaskm3`
- a governed internal tool UI
- a browser consumer that subscribes to runtime state and trace updates

### 2. A Governed MCP Server For Capability Discovery And Execution

You can expose Traverse through the dedicated MCP surface and let a downstream client discover and execute governed content through stdio.

Relevant docs:

- [docs/mcp-stdio-server.md](mcp-stdio-server.md)
- [docs/packaged-traverse-mcp-server-artifact.md](packaged-traverse-mcp-server-artifact.md)
- [docs/mcp-consumption-validation.md](mcp-consumption-validation.md)
- [docs/mcp-real-agent-exercise.md](mcp-real-agent-exercise.md)

Good fit:

- a downstream tool client that needs governed capability discovery
- an agentic shell that should talk to Traverse over MCP instead of private internals
- a local-first MCP integration path for development and validation

### 3. Workflow-Backed Business Capability Execution

You can model business behavior as governed capability and workflow artifacts, register them, and execute them through the Traverse runtime.

Relevant docs:

- [docs/getting-started.md](getting-started.md)
- [docs/expedition-example-authoring.md](expedition-example-authoring.md)
- [docs/expedition-example-smoke.md](expedition-example-smoke.md)
- [docs/cli-reference.md](cli-reference.md)

Good fit:

- business actions that should stay portable across hosts
- deterministic workflow-backed execution with trace output
- contract-first runtime behavior instead of host-specific glue

### 4. Packaged WASM Agents And WASM Microservices

You can package executable WASM-backed capabilities under the Traverse package model and validate them through the existing CLI and smoke paths.

Relevant docs:

- [docs/wasm-agent-authoring-guide.md](wasm-agent-authoring-guide.md)
- [docs/wasm-microservice-authoring-guide.md](wasm-microservice-authoring-guide.md)
- [docs/wasm-io-contract.md](wasm-io-contract.md)
- [docs/wasm-agent-example.md](wasm-agent-example.md)
- [docs/wasm-agent-team-readiness-example.md](wasm-agent-team-readiness-example.md)

Good fit:

- portable agent packages that must execute under runtime governance
- WASM-backed microservice-style packages with explicit boundaries
- deterministic package inspection and execution flows

### 5. A Downstream Consumer Bundle And Release Integration Path

You can consume Traverse as a released dependency surface rather than only as a source checkout, using the governed consumer bundle and release-facing docs.

Relevant docs:

- [docs/app-consumable-consumer-bundle.md](app-consumable-consumer-bundle.md)
- [docs/app-consumable-package-release-pointer.md](app-consumable-package-release-pointer.md)
- [docs/packaged-traverse-runtime-artifact.md](packaged-traverse-runtime-artifact.md)
- [docs/packaged-traverse-mcp-server-artifact.md](packaged-traverse-mcp-server-artifact.md)
- [docs/youaskm3-published-artifact-validation.md](youaskm3-published-artifact-validation.md)

Good fit:

- a downstream app that wants a stable runtime + MCP consumption path
- a release-driven integration workflow instead of repo archaeology
- a first real consumer such as `youaskm3`

## Supported Building Blocks

Even if your exact app is not listed above, Traverse already gives you these reusable pieces:

- governed capability contracts
- governed event contracts
- workflow definitions and deterministic traversal
- runtime traces
- browser-consumer integration surfaces
- CLI inspection and execution flows
- MCP discovery and execution surfaces
- packaged WASM execution paths

## Future Directions, Not Finished Product Paths

Traverse also points toward some directions that are not the main supported path yet:

- deeper federation and multi-peer synchronization work
- more transports beyond the current MCP stdio-first flow
- broader deployment/publication forms beyond the current governed release artifacts
- additional example domains beyond expedition and the current browser-consumer path

If you are deciding what to build today, prefer the supported paths above over the future federation items still marked `Blocked` on the board.
