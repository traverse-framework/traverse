# Traverse MCP Mode A — Release & Provenance Evidence

Governed by spec [`119-verified-registry-mcp-mode-a`](../specs/119-verified-registry-mcp-mode-a/spec.md) (FR-006).

Mode A is the product-facing local MCP host for LLM façades (Claude Desktop,
Cursor, and compatible stdio clients). It ships as a **versioned, standalone
`traverse-mcp` binary** that a client pins by version and verifies by checksum
and provenance — without a Rust toolchain or source checkout. Source-run
workflows remain contributor-only and must pass `--bundle <manifest-path>`.

## Release form

The Mode A binary is released through the packaged MCP server artifact path
defined in
[docs/packaged-traverse-mcp-server-artifact.md](packaged-traverse-mcp-server-artifact.md).
Each product tag `vX.Y.Z` publishes artifact release `traverse-mcp-vX.Y.Z`
(never Latest) with:

- `traverse-mcp` archives for `x86_64-unknown-linux-gnu` and
  `aarch64-apple-darwin`
- per-archive `.sha256` files and aggregated `SHA256SUMS`
- GitHub Attestations build provenance (`actions/attest-build-provenance`)

`traverse-mcp --version` reports `X.Y.Z` and the tagged commit SHA.

## Client pin and verification

1. Record the pinned artifact tag, e.g. `traverse-mcp-v0.15.0`.
2. Download the archive for the host target and `SHA256SUMS`.
3. `sha256sum -c --ignore-missing SHA256SUMS`.
4. `gh attestation verify traverse-mcp-<target>.tar.gz --repo traverse-framework/traverse`.
5. Extract and confirm `./traverse-mcp --version`.
6. Launch with the verified registry cache the host prepared:

   ```bash
   TRAVERSE_MCP_REGISTRY_CACHE=/path/to/verified-registry-cache \
     ./traverse-mcp stdio
   ```

Discovery and execution then run entirely from the prepared, digest-verified
state — see [docs/mcp-stdio-server.md](mcp-stdio-server.md) for the command
surface.

## Verification

```bash
bash scripts/ci/packaged_traverse_mcp_server_artifact_smoke.sh
bash scripts/ci/mcp_stdio_server_mode_a_smoke.sh
```
