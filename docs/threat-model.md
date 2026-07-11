# Traverse Threat Model

**Status:** Living document. Traverse is pre-production; this model tracks the
trust boundaries that exist today and the controls that protect them.

This document maps the major trust boundaries of the Traverse runtime, the
threats at each boundary, and the controls that mitigate them. It is the
reference for security reviews and for [SECURITY.md](../SECURITY.md) reports —
a report that demonstrates a bypass of a control listed here is high value.

## Scope and Assets

The assets Traverse protects:

- **Governed artifacts** — capability contracts, WASM binaries, workflows, and
  app manifests whose integrity and provenance must hold before execution.
- **Runtime execution integrity** — only authorized, verified capabilities run,
  and they run within their declared constraints.
- **Workspace isolation and privilege** — callers act only within workspaces and
  privilege levels they are entitled to; the `__system__` workspace is
  administrator-only.
- **Credential material** — bearer tokens and derived identities.
- **Execution evidence** — traces and events must be truthful and must not leak
  secrets.

## Trust Boundaries

### 1. HTTP + JSON API (`traverse-cli serve`)

The primary network-facing surface (spec `033-http-json-api`).

**Threats:** unauthenticated access; bearer-token forgery / privilege
escalation; cross-origin abuse; denial of service; workspace escape.

**Controls:**

- **Auth modes** gate the surface: `dev-loopback` (loopback only), `dev-any`
  (RFC 1918 private IPv4 only, public callers rejected `403`), and
  `bearer-required` (network-facing).
- **Signed-JWT authentication** in `bearer-required` mode: the server verifies
  the Ed25519 signature over `header.payload` before trusting any claim,
  enforces an `EdDSA`-only `alg` allow-list (rejecting `alg: none`), validates
  `exp`/`nbf`, and **fails closed** when no verification key is configured
  (issue #580, spec `033` FR-033–FR-037).
- **Privilege from verified tokens only**: administrative identity and
  `__system__` workspace access are granted solely from a signature-verified
  token; opaque bearer tokens are accepted only in the dev auth modes and never
  yield admin.
- **RFC 9457 problem responses** with stable `traverse_code`s; `401` for
  unauthenticated, `403` for unauthorized.
- **CORS** is closed by default: loopback origins in dev, exact configured
  origins only for non-loopback bindings.

**Known gaps / open items:** socket read/write timeouts and a concurrent accept
loop to resist slowloris-style DoS are tracked in issue #581. TLS termination is
out of scope for the runtime and is expected at a fronting proxy.

### 2. WASM Execution

Capabilities execute as WASM guests under the runtime's executor.

**Threats:** execution of untrusted or tampered binaries; guest resource
exhaustion (CPU hang, memory OOM); host escape.

**Controls:**

- **Artifact verification before execution** (`verify_artifact`): governed
  artifacts must carry a valid signature (Ed25519 or Sigstore) and a matching
  SHA-256 checksum; a checksum mismatch is rejected with `checksum_mismatch`
  before any bytes execute (issue #590, spec `031` FR-007).
- **Production-by-default security posture**: `RuntimeSecurityConfig::default()`
  is Production, which rejects unsigned artifacts. Development mode (which allows
  unsigned local artifacts with a warning) must be selected explicitly, and
  `serve` derives the posture from its auth mode — network-facing serving is
  Production (issue #588, spec `030` FR-013).
- **Declared execution constraints** in each capability contract
  (`host_api_access`, `network_access`, `filesystem_access`) bound guest
  capability.

**Known gaps / open items:** per-execution resource limits (fuel/epoch timeout,
memory cap) are tracked in issue #584; compiled-module caching that must remain
keyed to the verified checksum is tracked in issue #585; bridging the real
`WasmExecutor` into `serve` (which currently ships an example executor) is
tracked in issue #583.

### 3. Supply Chain / Artifact Provenance

How artifacts enter the registry and are trusted (spec `031-supply-chain-hardening`).

**Threats:** substitution of a compromised binary; unsigned or unprovenanced
artifacts reaching execution; trust decisions on attacker-influenced metadata.

**Controls:**

- SHA-256 checksum verification and signature verification at the
  `verify_artifact` boundary (see WASM Execution).
- Governed-artifact classification distinguishes `PublishedGoverned` from
  `LocalDev` trust levels.
- CI supply-chain checks (SBOM generation, checksum validation, signature
  presence) for governed artifacts.

**Known gaps / open items:** classifying governed artifacts from the
registry / approved-specs registry rather than path heuristics is tracked in
issue #596 (spec `030` FR-008); replacing the Sigstore `verified://` stub with
real Rekor/Fulcio verification is tracked in issue #589.

### 4. MCP Surface

The Model Context Protocol server exposes capability discovery and invocation to
local agents (spec `042-mcp-library-surface`).

**Threats:** unauthenticated local invocation; leakage of secrets through
returned traces.

**Controls:** MCP runs over a local stdio boundary. Execution goes through the
same runtime verification path as other callers.

**Known gaps / open items:** an explicit MCP stdio authentication boundary and
trace redaction are tracked in issue #592.

### 5. Federation

Cross-registry capability routing.

**Threats:** trusting capabilities or events from an unverified peer registry;
routing to a peer that does not satisfy contract constraints.

**Controls:** federated capabilities carry provenance and are subject to the
same artifact verification and contract validation as local capabilities before
execution.

### 6. Development Modes

Convenience modes that intentionally relax controls for local development.

**Threats:** a development posture leaking into a network-facing deployment.

**Controls:** dev modes bind loopback (`dev-loopback`) or private IPv4
(`dev-any`) only; `--allow-unauthenticated` and Development security mode emit
explicit startup warnings; `serve` selects the Production posture and
`bearer-required` auth automatically for non-loopback bindings, so a
network-facing server cannot silently inherit development trust.

### 7. Credential Handling and Evidence

**Threats:** bearer-token material lingering in memory; raw tokens leaking into
logs, events, or traces.

**Controls:** identity attribution stores only derived, non-secret fields
(`subject_id`, optional actor, a token *hash*), never the raw token. Execution
traces are public-tier evidence and must not embed credentials.

**Known gaps / open items:** zeroizing transient JWT/credential buffers after
identity derivation is tracked in issue #597 (spec `030` NFR-001); propagating
`subject_id`/`actor_id` into event envelopes and subscription filters is tracked
in issue #591.

## Out of Scope

- TLS termination (expected at a fronting proxy).
- Physical and host-OS security of the machine running the runtime.
- Compromise of the maintainer's signing keys (key management is an operational
  concern outside the runtime).

## Maintenance

Update this document whenever a trust boundary changes, a listed control lands
or is removed, or a "known gap" ticket is resolved. Link the governing spec and
issue for each control so reviewers can trace it to its source.
