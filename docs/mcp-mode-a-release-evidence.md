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

- `traverse-mcp` archives for `x86_64-unknown-linux-gnu`,
  `aarch64-unknown-linux-gnu` and `aarch64-apple-darwin` (`.tar.gz`), and
  `x86_64-pc-windows-msvc` (`.zip`)
- per-archive `.sha256` files and aggregated `SHA256SUMS`
- GitHub Attestations build provenance (`actions/attest-build-provenance`)

`traverse-mcp --version` reports `X.Y.Z` and the tagged commit SHA.

## Client pin and verification

1. Record the pinned artifact tag, e.g. `traverse-mcp-v0.15.0`.
2. Download the archive for the host target and `SHA256SUMS`.
3. `sha256sum -c --ignore-missing SHA256SUMS`.
4. `gh attestation verify traverse-mcp-<target>.tar.gz --repo traverse-framework/traverse`.
5. Extract and confirm `./traverse-mcp --version`.
6. Sync the public registry index and prepare the verified cache. `sync` is
   the only network step; it records the `index-vN` release it used.

   ```bash
   ./traverse-mcp sync --out synced-state.json
   ./traverse-mcp prepare-cache --synced-state synced-state.json \
     --cache /path/to/verified-registry-cache --ref core/core.authorize@^1
   ```

   Pin a registry release with `sync --release index-vN`, and add one
   `--ref <namespace/id@version_range>` per capability the agent may use.
7. Launch with the verified registry cache:

   ```bash
   ./traverse-mcp stdio --cache /path/to/verified-registry-cache
   ```

   `TRAVERSE_MCP_REGISTRY_CACHE=/path/to/verified-registry-cache` is
   equivalent to `--cache`. In a client config, use
   `"args": ["stdio", "--cache", "/absolute/path/to/verified-registry-cache"]`.

Discovery and execution then run entirely from the prepared, digest-verified
state — see [docs/mcp-stdio-server.md](mcp-stdio-server.md) for the command
surface.

## Troubleshooting: invalid metadata names the record and field

`prepare-cache` and server startup run the same verification-binding check
(Spec 119 FR-009), so a cache that prepares is a cache that serves. A
failure keeps the code `registry_metadata_cache_invalid` and names the
failing record and field. For example, a synced state whose
`core.authorize@1.2.0` record has an empty `service_type`:

```json
{"code":"registry_metadata_cache_invalid","field":"service_type","kind":"mcp_mode_b_prepare_cache_error","message":"public metadata cache generation has invalid verification bindings: record `core/core.authorize@1.2.0` field `service_type`","record":"core/core.authorize@1.2.0"}
```

`record` is `null` for a generation-level field (`schema_version`,
`source_release`, `index_digest`). `prepare-cache` checks before writing, so
an existing cache is kept. Re-run `traverse-mcp sync` to replace a
hand-edited or stale synced state.

## Weekly demo

The weekly demo runs Mode A from the release binary and `sync`, never from
a source build or a hand-written synced state:

1. Download and verify the pinned `traverse-mcp-vX.Y.Z` archive (steps 1–5).
2. `./traverse-mcp sync --out synced-state.json` and note the `index-vN`
   it reports.
3. `./traverse-mcp prepare-cache --synced-state synced-state.json --cache demo-cache --ref <the capabilities the demo uses>`.
4. Point the demo agent at `./traverse-mcp stdio --cache demo-cache` and run
   `describe_server`. It reports `discovery_source.source_release` equal to
   the synced `index-vN`.

The path was checked on 2026-10-30 with the `aarch64-apple-darwin` binary
from a `mcp-server-publish.yml` dry run of `cbac84a3`, on a shell with no
Rust toolchain on `PATH`. `sync` wrote `index-v412` (232 capability records),
`prepare-cache --ref core/core.authorize@^1` cached `core.authorize@1.2.0`,
and `stdio --cache` served `describe_server` and `list_entrypoints` from it.

## Verification

```bash
bash scripts/ci/packaged_traverse_mcp_server_artifact_smoke.sh
bash scripts/ci/mcp_stdio_server_mode_a_smoke.sh
```
