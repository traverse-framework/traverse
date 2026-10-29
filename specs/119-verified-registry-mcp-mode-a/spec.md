# Verified Public-Registry MCP Mode A Host

**Status**: Approved (2026-08-25)
**Canonical governing ID**: `119-verified-registry-mcp-mode-a`
**Version**: 0.2.0
**Input**: Traverse #865 and #1144.
**Amendment (2026-10-29, version 0.1.0 -> 0.2.0)**: Decision 121 / `#1753`.
FR-006 names the four release targets, the Windows `.zip` archive, and the
per-target smoke run and SHA-256 evidence.

## Purpose

Define the first product-facing local MCP host for LLM façades. It lets a
local client discover and execute public governed entrypoints using the same
host-supplied, digest-verified registry state and artifacts as consumer hosts.

## Scope and boundary

Mode A is a local stdio host. It is not a hosted, browser, multi-tenant, or
gateway service. It consumes prepared verified state and performs no network
refresh during discovery or execution.

## Requirements

- **FR-001**: Mode A MUST discover only public capability and workflow entries
  from host-supplied, digest-verified registry state. It MUST NOT use the
  expedition bundle, private overrides, in-process registrations, or network
  fallback.
- **FR-002**: `validate_entrypoint`, `execute_entrypoint`, and
  `render_execution_report` MUST accept an inline `request` object serialized
  as a `RuntimeRequest`. A legacy `request_path` MAY remain only as a mutually
  exclusive compatibility input.
- **FR-003**: An absent, malformed, stale-unverified, or unprepared state or
  artifact MUST fail closed with stable actionable errors; it MUST NOT become
  an empty catalog or demo fallback.
- **FR-004**: Production execution MUST hand off only the digest-verified
  published WASM artifact selected by verified public state. The expedition
  Rust executor is a separate demo/test path and MUST NOT be a fallback.
- **FR-005**: Responses MUST expose only runtime-owned results and redacted
  public trace/evidence. They MUST NOT expose raw contracts, private records,
  request paths, credentials, or secrets.
- **FR-006**: Mode A MUST ship as a versioned standalone `traverse-mcp`
  binary release with published checksum and provenance evidence. Source-run
  workflows remain contributor-only.
  - (0.2.0) The release MUST carry four targets: `x86_64-unknown-linux-gnu`,
    `aarch64-unknown-linux-gnu`, and `aarch64-apple-darwin` as `.tar.gz`, and
    `x86_64-pc-windows-msvc` as `.zip`.
  - Each target MUST have a SHA-256 checksum, a `SHA256SUMS` line, and build
    provenance. The release evidence MUST list the SHA-256 of each asset.
  - Each target's built binary MUST be smoke-run (`traverse-mcp --version`
    reports the tag version and commit) before upload.
  - Binaries follow product `v*` tags in lockstep as `traverse-mcp-vX.Y.Z`.
    A `workflow_dispatch` dry run builds, smoke-runs, and checksums every
    target without attesting or releasing.
- **FR-007**: The first release MUST expose verified public discovery without
  hard-coded or inferred content groups. Registry-governed grouping metadata
  is required before content groups become a product surface.
- **FR-008**: Existing stdio authentication behavior remains unchanged. This
  spec adds no hosted authentication, tenancy, browser transport, or gateway.

## Acceptance scenarios

1. A local Claude/Cursor-compatible client starts a pinned Mode A binary with
   valid prepared state and discovers public entrypoints without expedition
   records.
2. The client sends an inline runtime request and receives the result from the
   exact verified published WASM artifact.
3. Missing state, invalid verification evidence, an invalid inline request,
   or both `request` and `request_path` returns a stable error without network
   access or filesystem request materialization.
4. Repeated discovery from unchanged verified state is deterministic and
   returns no private or raw-contract data.
5. The published release can be pinned and its checksum/provenance verified
   without installing a Rust toolchain.

## Validation

- Unit and integration coverage for public-only discovery, inline/XOR input,
  exact artifact verification, missing/invalid state, redaction, and no
  fallback.
- Consumer smoke coverage for a released binary with a Claude/Cursor-style
  local configuration and no source checkout.
- Release validation verifies supported artifacts, version, checksums, and
  provenance.

## Dependencies

- Spec 118 supplies verified server-state ownership and manifest semantics.
- Registry #318 must supply the public metadata projection required for safe
  public discovery.

## Non-goals

- Hosted/browser MCP, a remote gateway, tenancy, or new remote auth.
- Static kit content groups, request-time refresh, private fallback, or
  expedition fallback.
