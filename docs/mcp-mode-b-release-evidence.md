# Traverse MCP Mode B — Release & Provenance Evidence

Governed by spec [`080-embedded-registry-cache`](../specs/520-embedded-registry-cache/spec.md)
(Spec 520 lineage). Mode B is the embedded-cache host track: the shipped
`traverse-mcp` binary **prepares** a host-owned verified cache from public
registry refs, then serves discover/validate/execute/report from that cache
only. Consumers do not rewrite App-References trees via `registry materialize`.

Mode A (Spec 119) remains the default Claude Desktop / Cursor path that
**consumes** an already-prepared cache. Mode B adds the prepare host CLI.

## Release form

Mode B ships in the same versioned `traverse-mcp` binary as Mode A. Pin and
verify that binary through the packaged MCP server artifact path in
[docs/packaged-traverse-mcp-server-artifact.md](packaged-traverse-mcp-server-artifact.md)
and [docs/mcp-mode-a-release-evidence.md](mcp-mode-a-release-evidence.md):

1. Record the pinned version, e.g. `traverse-mcp 0.11.0`.
2. Download the binary for the host target and its published `.sha256`.
3. Recompute and compare: `shasum -a 256 traverse-mcp` must equal the published
   digest.
4. Confirm the provenance attestation names the same version tag and the
   `cargo build --locked` build invocation.

App-References `apps/llm-mcp-reference/mode-b/serve.sh` should invoke this
pinned binary — not a source checkout and not a materialize rewrite.

## Documented prepare → serve path

Preparation is the only network-capable step. Serving is offline and
fail-closed when the cache is missing or invalid.

```bash
traverse-mcp prepare-cache \
  --synced-state /path/to/synced-public-registry-state.json \
  --cache /path/to/verified-registry-cache \
  --ref core/core.normalize-participants@=1.1.0 \
  --json

traverse-mcp stdio --cache /path/to/verified-registry-cache
```

Equivalent serve form (Mode A env, same verified-cache host):

```bash
TRAVERSE_MCP_REGISTRY_CACHE=/path/to/verified-registry-cache \
  traverse-mcp stdio
```

`--ref` may be repeated. When omitted, every non-deprecated capability in the
synced snapshot is prepared at its exact published version.

See [docs/mcp-stdio-server.md](mcp-stdio-server.md) for the command surface.

## Verification

```bash
bash scripts/ci/mcp_stdio_server_mode_b_smoke.sh
```

That smoke prepares a fixture cache from public-registry refs, drives stdio
MCP execute against one digest-pinned capability, and asserts an unprepared
cache fails closed.
