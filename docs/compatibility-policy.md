# Compatibility Policy

The shared, org-wide compatibility policy lives in [`traverse-framework/.github`](https://github.com/traverse-framework/.github)'s `docs/compatibility-policy.md`. This repo has adopted **governance version 1.0.0**.

## Repo-Specific Compatibility Statements

The boundary between governed core runtime responsibilities and optional adapters is documented in:

- `docs/adapter-boundaries.md`

The release-facing downstream compatibility statement for the current `youaskm3` baseline is:

- `docs/v0.3.0-public-surface-compatibility.md`

The current Traverse release notes are:

- `docs/releases/v0.8.0.md`

## v1 Stability Statement

At `v1.0.0`, Traverse makes the following stability commitments:

**Public API (stable — semver guarantees apply):**
- `traverse-runtime`: `CapabilityExecutor`, `ExecutorCapability`, `ExecutorError`, `ArtifactType`, `PlacementRouter`, `RouterRequest`, `RouterResponse`, `TraceStore`, `TraceOutcome`
- `traverse-contracts`: all public contract types (`CapabilityContract`, `EventContract`, etc.)
- `traverse-registry`: `CapabilityRegistry`, `EventRegistry`, `WorkflowRegistry`
- `traverse-cli`: all documented CLI subcommands and their `--json` output shapes
- `traverse-mcp`: `discover_capabilities()`, `execute_capability()` library surface

**Internal (may change between minor versions):**
- Anything in `mod.rs` private submodules not re-exported at the crate root
- `TraceStore` internal storage format (inspect via public API only)
- `InProcessBroker` internal state

**Semver at v1:**
- PATCH: bug fixes, no API changes
- MINOR: new public API additions, backward-compatible
- MAJOR: breaking public API changes (requires new v2.x series)

This v1 stability statement is a gate condition for the v1.0.0 release. See `docs/v1-milestone.md` for the full gate list.
