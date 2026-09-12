# Traverse MCP Stdio Server Package

The dedicated Traverse MCP WASM server package is the thin, governed host-facing surface for the app-consumable MCP path.

The packaged MCP server artifact is defined in [docs/packaged-traverse-mcp-server-artifact.md](packaged-traverse-mcp-server-artifact.md).

For the first `youaskm3` release-facing client path, use [docs/youaskm3-canonical-mcp-client-path.md](youaskm3-canonical-mcp-client-path.md).

It is intentionally narrow:

- it stays a façade over Traverse runtime authority
- contributor `stdio` without a cache uses the canonical expedition registry bundle as its source of truth
- Mode A / Mode B serve discover, description, validation, execution, and execution-report from a host-owned verified cache
- it is documented and runnable locally

## Supported Bootstrap Path

The supported developer bootstrap path for the dedicated MCP server is:

```bash
cargo run -p traverse-mcp -- stdio
```

Supported host commands:

- `stdio [--cache <dir>] [--simulate-startup-failure]` — serve MCP on stdio
- `prepare-cache --synced-state <path> --cache <dir> [--ref <namespace/id@version_range>]... [--json]` — Mode B Spec 520 cache prepare

Unsupported bootstrap attempts fail loudly:

- omitting the command prints the usage line and exits non-zero
- using any command other than `stdio` or `prepare-cache` prints `Unsupported command: <command>` and exits non-zero

Developers and agents should treat other bootstrap ideas as unsupported unless they are explicitly documented in this page or in the packaged artifact docs.

## Start The Server

From the repository root:

```bash
cargo run -p traverse-mcp -- stdio
```

By default, the stdio server runs in `local_trust` mode. This is intended for
local IDE and agent integrations where the parent process already owns the
local user session and can launch the server command.

For stricter local launchers, require a bearer token for execution commands:

```bash
TRAVERSE_MCP_STDIO_BEARER_TOKEN="replace-with-local-secret" \
  cargo run -p traverse-mcp -- stdio
```

When `TRAVERSE_MCP_STDIO_BEARER_TOKEN` is set, `execute_entrypoint` and
`render_execution_report` reject requests that do not include a matching local
bearer token. Discovery and description commands remain readable so clients can
bootstrap safely. The token is never echoed in startup, error, execution, or
debug output.

Authenticated execution commands can pass the token either as:

```json
{"auth":{"type":"bearer","token":"replace-with-local-secret"}}
```

or as the compatibility field:

```json
{"bearer_token":"replace-with-local-secret"}
```

To simulate a deterministic startup failure for validation:

```bash
cargo run -p traverse-mcp -- stdio --simulate-startup-failure
```

## Verified Public-Registry Mode A

Governed by spec [`119-verified-registry-mcp-mode-a`](../specs/119-verified-registry-mcp-mode-a/spec.md).

When `TRAVERSE_MCP_REGISTRY_CACHE` names a host-owned, digest-verified registry
cache root, the server engages **Mode A**: discovery and execution are served
exclusively from that prepared verified state — never the expedition bundle,
private overrides, in-process registrations, or a network refresh.

```bash
TRAVERSE_MCP_REGISTRY_CACHE=/srv/traverse/verified-registry-cache \
  cargo run -p traverse-mcp -- stdio
```

The versioned binary release and its checksum/provenance pin path for
Claude Desktop and Cursor are documented in
[docs/mcp-mode-a-release-evidence.md](mcp-mode-a-release-evidence.md).

In Mode A:

- `describe_server` / `describe` report `"mode": "verified_public"`, the Spec 119
  governing id, and a `discovery_source` block with the verified `source_release`
  and `index_digest`.
- `list_entrypoints` and `describe_entrypoint` return only capabilities present
  in the verified public-metadata generation. There are no content groups until
  registry-governed grouping metadata exists (FR-007), so `list_content_groups`
  is empty and `describe_content_group` returns `not_found`.
- `validate_entrypoint`, `execute_entrypoint`, and `render_execution_report`
  accept an inline `request` object (a serialized `RuntimeRequest`) that is
  mutually exclusive with the legacy `request_path`. Every response reports
  `request_source` and an `artifact` block with the digest handed to the WASM
  executor and whether it matches the verified public state.
- Execution resolves the exact digest-verified published WASM from the cache and
  runs it through the real runtime WASM executor. A missing, malformed,
  unverified, or unprepared state or artifact fails closed with a stable code
  (`registry_sync_missing`, `registry_metadata_cache_invalid`,
  `registry_cache_entry_missing`) — it never degrades to an empty catalog or a
  demo fallback.

## Supported Commands

The package emits deterministic JSON envelopes for:

- `describe_server`
- `list_content_groups`
- `describe_content_group`
- `list_entrypoints`
- `describe_entrypoint`
- `validate_entrypoint`
- `execute_entrypoint`
- `render_execution_report`
- `shutdown`

The server reports governed content groups, capabilities, events, and workflows from the canonical expedition bundle.

## Trace Redaction

Execution responses and rendered reports return a public trace summary by
default, not the full runtime trace. The response includes `trace_redaction`
metadata and omits private or high-volume trace fields such as the original
runtime request, decision evidence, execution record, result record, and OTEL
span payloads. Observation messages summarize trace events instead of embedding
the full trace object.

Use runtime trace inspection tools for local debugging when full trace evidence
is explicitly needed.

## Content Groups

The first content group exposed through the dedicated server is the neutral core-runtime example group:

- `core-runtime-example`

It points at the checked-in executable capability package template and local runtime documentation, so clients can discover a Traverse-native content family that is not expedition-specific.

## Validation

Run the deterministic smoke test for the package surface:

```bash
bash scripts/ci/mcp_stdio_server_smoke.sh
bash scripts/ci/mcp_stdio_server_discovery_smoke.sh
bash scripts/ci/mcp_stdio_server_execution_report_smoke.sh
bash scripts/ci/mcp_stdio_server_mode_a_smoke.sh
```

The `mcp_stdio_server_mode_a_smoke.sh` script drives the released binary against
the checked-in verified kit fixture at
`crates/traverse-mcp/tests/fixtures/mode-a-cache` (regenerate it with
`cargo test -p traverse-mcp --lib -- --ignored --exact stdio_server::tests::mode_a::regenerate_committed_fixture`)
and asserts Mode A discovery + inline execute succeed and that an unprepared
cache fails closed.

## Mode B Embedded Verified-Cache Host

Governed by spec [`080-embedded-registry-cache`](../specs/520-embedded-registry-cache/spec.md).

Mode B prepares a host-owned Spec 520 verified cache from public registry refs,
then serves MCP from that cache only. App-References consumers point
`mode-b/serve.sh` at the shipped `traverse-mcp` binary — they do not rewrite
trees via `registry materialize`.

```bash
cargo run -p traverse-mcp -- prepare-cache \
  --synced-state /path/to/synced-public-registry-state.json \
  --cache /path/to/verified-registry-cache \
  --ref core/core.normalize-participants@=1.1.0 \
  --json

cargo run -p traverse-mcp -- stdio --cache /path/to/verified-registry-cache
```

Prepare is the only network-capable step. A missing or invalid cache fails
closed with `registry_sync_missing` / `registry_metadata_cache_invalid` /
`registry_cache_entry_missing`. The versioned binary pin path for App-Refs
Mode B is documented in
[docs/mcp-mode-b-release-evidence.md](mcp-mode-b-release-evidence.md).

```bash
bash scripts/ci/mcp_stdio_server_mode_b_smoke.sh
```

The smoke prepares a fixture cache, drives stdio MCP execute against one
digest-pinned capability, and asserts an unprepared `--cache` fails closed.

Run repository checks:

```bash
bash scripts/ci/repository_checks.sh
```

For downstream `youaskm3` release evidence, also run:

```bash
bash scripts/ci/mcp_consumption_validation.sh
bash scripts/ci/mcp_real_agent_exercise_smoke.sh
```
