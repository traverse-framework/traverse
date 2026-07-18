# Traverse Architecture: Execution and Consumption Models

Traverse exposes three distinct surfaces for building and consuming capabilities. Understanding how they relate — and when to use each — is essential for designing your integration.

## The Three Models

### 1. WASM Capabilities (Local Execution)

**What**: A capability is a WASM binary compiled from Rust (or any WASM-compatible language) that reads a JSON payload from stdin and writes a JSON result to stdout. It is registered in the capability registry via a bundle manifest and invoked by the runtime's `WasmExecutor`.

**When to use**:
- You are building a new capability (a unit of computation)
- The capability should be portable across execution targets (local, cloud, edge)
- You want governance (spec, contract, digest immutability)

**How it works**:
```
CLI request → PlacementRouter → WasmExecutor → WASM binary (stdin/stdout) → RuntimeTrace
```

**Entry point**: [`docs/wasm-agent-authoring-guide.md`](wasm-agent-authoring-guide.md)

---

### 2. MCP Surface (Agent/LLM Discovery and Invocation)

**What**: The `traverse-mcp` crate exposes a Model Context Protocol server over stdio. It provides tools that LLMs and AI agents can call to discover registered capabilities, inspect their contracts, and execute them — without knowing the CLI.

**MCP tools exposed**:
| Tool | Description |
|------|-------------|
| `discover_capabilities` | List capabilities matching an intent or filter |
| `get_capability` | Inspect a specific capability contract |
| `list_events` | List events in the event catalog |
| `get_event` | Inspect a specific event contract |
| `execute_capability` | Execute a capability by ID with a JSON input |
| `get_trace` | Retrieve a trace by ID |

**When to use**:
- An LLM or AI agent needs to discover what capabilities are available
- You are integrating Traverse with Claude, GPT, or another tool-use enabled model
- You want the model to drive capability selection rather than hard-coding IDs

**How it works**:
```
LLM tool call → MCP stdio server → traverse-mcp → traverse-runtime → WasmExecutor
```

**Entry point**: [`docs/mcp-stdio-server.md`](mcp-stdio-server.md)

**Important**: In v0.1, `traverse-mcp` is a stdio binary server. Agents cannot link it as a library — they must communicate via the MCP wire protocol. See [#310](https://github.com/traverse-framework/traverse/issues/310) for the planned library API.

---

### 3. Browser Adapter (Live Streaming to a Frontend)

**What**: The browser adapter (`traverse-cli browser-adapter serve`) starts a local HTTP server that streams runtime state events and execution traces to a browser client over SSE (Server-Sent Events) or WebSocket. It enables a React or web frontend to display live Traverse execution state.

**When to use**:
- You are building a UI that shows live capability execution status
- You want to stream `RuntimeTrace` updates to a browser in real time
- You are building the `youaskm3` shell or a similar consumer app

**How it works**:
```
Browser client → HTTP/SSE → browser-adapter server → traverse-runtime subscription → state events
```

**Entry point**: [`docs/browser-adapter.md`](browser-adapter.md)

**Important**: The browser adapter delivers events only to actively connected clients. There is no replay for late-connecting clients in v0.1. See [#312](https://github.com/traverse-framework/traverse/issues/312).

---

## How the Three Models Interact

```
┌─────────────────────────────────────────────────────────────┐
│                     Your Application                        │
│                                                             │
│  ┌──────────┐    ┌─────────────┐    ┌──────────────────┐   │
│  │  CLI /   │    │  MCP tools  │    │  Browser UI      │   │
│  │  Scripts │    │  (LLM use)  │    │  (React/Web)     │   │
│  └────┬─────┘    └──────┬──────┘    └────────┬─────────┘   │
│       │                 │                    │              │
└───────┼─────────────────┼────────────────────┼─────────────┘
        │                 │                    │
        ▼                 ▼                    ▼
┌───────────────────────────────────────────────────────────┐
│                  traverse-runtime                         │
│  PlacementRouter → WasmExecutor → RuntimeTrace            │
│  EventBroker → subscriptions                              │
└───────────────────────────────────────────────────────────┘
        │
        ▼
┌─────────────────────┐
│  WASM Capabilities  │
│  (stdin/stdout JSON)│
└─────────────────────┘
```

All three surfaces drive the same runtime. A CLI invocation, an MCP tool call, and a browser-triggered execution all go through `PlacementRouter` and produce a `RuntimeTrace`.

---

## Decision Guide

| If you are building... | Use |
|------------------------|-----|
| A new capability (unit of computation) | WASM capability + contract |
| An LLM integration that needs to discover and call capabilities | MCP surface |
| A web UI that shows live execution status | Browser adapter |
| A CI pipeline or script that invokes capabilities | CLI (`traverse-cli expedition execute`) |
| An autonomous agent that needs to register and invoke capabilities programmatically | CLI with `--json` (planned, [#305](https://github.com/traverse-framework/traverse/issues/305)) or MCP |
| A multi-capability workflow | Workflow contract + registry traversal |

---

## Related Docs

- [`docs/wasm-agent-authoring-guide.md`](wasm-agent-authoring-guide.md) — write a WASM capability
- [`docs/mcp-stdio-server.md`](mcp-stdio-server.md) — MCP server setup
- [`docs/browser-adapter.md`](browser-adapter.md) — browser adapter and streaming
- [`docs/workflow-composition-guide.md`](workflow-composition-guide.md) — chain capabilities
- [`quickstart.md`](../quickstart.md) — first browser-consumption flow
