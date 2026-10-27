# Packaged traverse-mcp server artifact (Mode A)

Governed by Spec [`119-verified-registry-mcp-mode-a`](../specs/119-verified-registry-mcp-mode-a/spec.md) (FR-006) and issue #1653.

The Mode A MCP server ships as a **versioned, checksum-pinned binary** for Linux
x86_64 and macOS arm64. Clients download the archive, verify `SHA256SUMS` and
the GitHub build-provenance attestation, then launch `traverse-mcp stdio` —
no Rust toolchain required.

Source-run remains contributor-only and requires an explicit bundle, for
example `cargo run -p traverse-mcp -- stdio --bundle
examples/expedition/registry-bundle/manifest.json`. Packaged Mode A binaries
need only their prepared verified cache; they never infer a source-checkout
bundle.

## Release form

On each product tag `vX.Y.Z`, workflow
[`.github/workflows/mcp-server-publish.yml`](../.github/workflows/mcp-server-publish.yml)
builds release binaries and publishes an artifact-only GitHub Release:

| Field | Value |
| --- | --- |
| Release tag | `traverse-mcp-vX.Y.Z` (optional rebuild `traverse-mcp-vX.Y.Z-N`) |
| Latest | never (`--latest=false`) |
| Assets | `traverse-mcp-<target>.tar.gz`, per-archive `.sha256`, `SHA256SUMS` |
| Provenance | `actions/attest-build-provenance` on the archives |

Targets:

- `x86_64-unknown-linux-gnu`
- `aarch64-apple-darwin`

`traverse-mcp --version` prints `traverse-mcp X.Y.Z (<git-sha>)` for the
tagged commit.

## Pin and verify

```bash
TAG=traverse-mcp-v0.15.0   # replace with the release you pin
TARGET=aarch64-apple-darwin # or x86_64-unknown-linux-gnu
REPO=traverse-framework/traverse

curl -LO "https://github.com/${REPO}/releases/download/${TAG}/traverse-mcp-${TARGET}.tar.gz"
curl -LO "https://github.com/${REPO}/releases/download/${TAG}/SHA256SUMS"
sha256sum -c --ignore-missing SHA256SUMS
gh attestation verify "traverse-mcp-${TARGET}.tar.gz" --repo "${REPO}"
tar -xzf "traverse-mcp-${TARGET}.tar.gz"
./traverse-mcp --version
```

Then point the MCP client at the verified binary (absolute path on Claude
Desktop):

```json
{
  "mcpServers": {
    "traverse": {
      "command": "/absolute/path/to/traverse-mcp",
      "args": ["stdio"]
    }
  }
}
```

Prepare a host-owned verified registry cache as in
[docs/mcp-mode-a-release-evidence.md](mcp-mode-a-release-evidence.md) before
production discovery/execute.

## macOS Gatekeeper

These binaries are not notarized yet (follow-up). On first launch macOS may
quarantine a browser/curl download; clear quarantine only for a binary you
have checksum- and attestation-verified:

```bash
xattr -d com.apple.quarantine ./traverse-mcp
```

## Verification (docs + workflow)

```bash
bash scripts/ci/packaged_traverse_mcp_server_artifact_smoke.sh
```

That check confirms this doc, the publish workflow, and the pin/verify
instructions stay linked from release-facing docs. Downloading a published
asset and running MCP `initialize` / `tools/list` is exercised when
`TRAVERSE_MCP_RELEASE_TAG` is set (post-publish smoke).
