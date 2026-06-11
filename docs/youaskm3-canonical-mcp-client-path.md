# Canonical Traverse MCP Client Path for youaskm3

This page is the release-facing MCP integration path that `youaskm3` can cite for its first real release.

Supported Traverse baseline: `v0.3.0`

## What Is Supported

For the first `youaskm3` release, the canonical Traverse MCP path is:

```bash
cargo run -p traverse-mcp -- stdio
```

This starts the governed Traverse MCP stdio server from a source checkout pinned to `v0.3.0`.

The supported client category is any MCP client that can launch a local stdio server command and route tool calls over that process. A real client such as Claude Desktop fits this category when configured to run the local `traverse-mcp` command.

## Recommended Source Checkout

Downstream consumers should pin the released Traverse tag instead of following repository head:

```bash
git clone https://github.com/enricopiovesan/Traverse.git
cd Traverse
git checkout v0.3.0
cargo run -p traverse-mcp -- stdio
```

Requirements:

- Rust 1.94+
- local source checkout of Traverse `v0.3.0`
- an MCP client that supports stdio server commands

For packaging and source-build expectations, see [docs/v0.3.0-source-build-consumer-packaging.md](v0.3.0-source-build-consumer-packaging.md).
For the combined release evidence path that includes this MCP surface, see [docs/v0.3.0-downstream-validation-path.md](v0.3.0-downstream-validation-path.md).

## Example Client Configuration

For a stdio MCP client that accepts JSON server configuration, use this shape and replace `/absolute/path/to/Traverse` with the local Traverse checkout:

```json
{
  "mcpServers": {
    "traverse": {
      "command": "cargo",
      "args": [
        "run",
        "-p",
        "traverse-mcp",
        "--",
        "stdio"
      ],
      "cwd": "/absolute/path/to/Traverse"
    }
  }
}
```

Expected startup behavior:

- the client launches `cargo run -p traverse-mcp -- stdio`
- Traverse emits deterministic MCP stdio envelopes
- the client can discover the governed MCP server description, content groups, entrypoints, validation path, execution path, and execution report path

## Public MCP Surfaces

The first-release public MCP path exposes the governed surfaces documented by [docs/mcp-stdio-server.md](mcp-stdio-server.md):

- `describe_server`
- `list_content_groups`
- `describe_content_group`
- `list_entrypoints`
- `describe_entrypoint`
- `validate_entrypoint`
- `execute_entrypoint`
- `render_execution_report`
- `shutdown`

The lower-level Rust library helpers in `crates/traverse-mcp` remain implementation details unless a release-facing document explicitly names them as public consumer API.

## What This Does Not Promise

This path does not promise:

- every possible MCP client walkthrough
- hosted MCP server operation
- cross-platform binary distribution
- a final `1.0` compatibility guarantee
- access to private Traverse internals

For `v0.3.0`, the honest claim is narrower: `youaskm3` can use Traverse through a released, source-build, stdio MCP path.

## Validation

Run the deterministic validation commands from the Traverse repository root:

```bash
bash scripts/ci/mcp_stdio_server_smoke.sh
bash scripts/ci/mcp_consumption_validation.sh
bash scripts/ci/mcp_real_agent_exercise_smoke.sh
```

These checks prove that the released public MCP path can start, expose the expected server surface, validate the downstream `youaskm3` consumption path, and exercise a real-agent MCP flow without private repository knowledge.

## Related Docs

- [docs/mcp-stdio-server.md](mcp-stdio-server.md)
- [docs/mcp-consumption-validation.md](mcp-consumption-validation.md)
- [docs/mcp-real-agent-exercise.md](mcp-real-agent-exercise.md)
- [docs/v0.3.0-downstream-validation-path.md](v0.3.0-downstream-validation-path.md)
- [docs/youaskm3-integration-validation.md](youaskm3-integration-validation.md)
