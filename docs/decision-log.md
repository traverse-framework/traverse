# Traverse Decision Log

This log consolidates material product and architecture decisions that shape the current Traverse roadmap. It is intentionally higher level than the governing specs: specs define what must be built, while this log records why the direction was chosen.

All current implementation specs listed in `specs/governance/approved-specs.json` are approved for implementation unless a later approved spec or ADR supersedes them.

## Decision 1: Provide HTTP+JSON as the First App-Consumable Runtime API

- **Date**: 2026-05-27
- **Status**: Accepted
- **Governing specs**: `033-http-json-api`, `035-multi-agent-isolation`, `029-integrated-observability`
- **Related issues**: `#300`, `#387`, `#390`, `#391`, `#392`, `#393`, `#394`, `#395`, `#396`

### Context

Downstream apps such as `youaskm3`, browser clients, local agents, and non-Rust tools need to consume Traverse without shelling out to human-readable CLI commands.

### Decision

Expose `traverse-cli serve` with stable HTTP+JSON endpoints, local discovery through `.traverse/server.json`, structured errors, explicit API versioning, CORS behavior, and synchronous plus asynchronous execution flows.

### Alternatives Considered

- Keep CLI-only execution and add JSON flags later.
- Expose only a Rust SDK.
- Start with WebSocket or Server-Sent Events before a stable request/response API exists.

### Outcome

HTTP+JSON becomes the first stable external runtime surface. CLI remains useful for humans and CI, but applications should target the HTTP API for app integration.

## Decision 2: Use Repo-Local Discovery for Local App and Agent Development

- **Date**: 2026-05-27
- **Status**: Accepted
- **Governing specs**: `033-http-json-api`, `035-multi-agent-isolation`
- **Related issues**: `#387`

### Context

Local browser apps and agents need a deterministic way to find a running Traverse server even when the default port is unavailable.

### Decision

`traverse-cli serve` writes `.traverse/server.json` with `base_url`, `health_url`, `workspace_default`, `pid`, `started_at`, `auth_mode`, and local token metadata when applicable. Clients must verify `GET /healthz` before trusting the file.

### Alternatives Considered

- Require every app to pass the port explicitly.
- Use an OS-level service registry.
- Use a global config file outside the repo.

### Outcome

Local discovery is repo-scoped, testable, and suitable for both humans and coding agents.

## Decision 3: Make MCP Both a Stdio Server and an Embeddable Library Surface

- **Date**: 2026-05-27
- **Status**: Accepted
- **Governing specs**: `042-mcp-library-surface`, `015-capability-discovery-mcp`, `023-browser-hosted-mcp-consumer-model`
- **Related issues**: `#310`, `#366`

### Context

`youaskm3` needs MCP support, and agents should not have to reimplement the MCP wire protocol over stdin/stdout when they are already running in Rust or embedding Traverse.

### Decision

Keep the stdio MCP server path, and expose the core MCP operations as a public Rust library surface with deterministic request/response types.

### Alternatives Considered

- Keep MCP only as a stdio binary.
- Make downstream apps reimplement Traverse MCP behavior.
- Delay MCP library support until after the HTTP API.

### Outcome

Traverse owns MCP execution and discovery behavior. Downstream apps can choose stdio integration or direct library integration without coupling to private crate internals.

## Decision 4: Add Programmatic Registration Instead of CLI-Only Registration

- **Date**: 2026-05-27
- **Status**: Accepted
- **Governing specs**: `034-programmatic-registration`, `035-multi-agent-isolation`, `040-contractual-enforcement-gate`
- **Related issues**: `#302`, `#397`, `#398`, `#399`, `#400`

### Context

Agents and app runtimes need to register capabilities, bundles, manifests, and related artifacts without writing ad hoc files and invoking the CLI as a subprocess.

### Decision

Define a programmatic registration API with stable request models, idempotency behavior, conflict handling, validation evidence, and audit requirements.

### Alternatives Considered

- Keep bundle registration as CLI-only.
- Expose low-level registry structs directly.
- Permit dynamic registration without validation and audit evidence.

### Outcome

Registration becomes app-consumable while preserving contract validation, workspace boundaries, and governance evidence.

## Decision 5: Govern Multi-Agent Use with Workspaces, Bearer Auth, Scopes, and Audit Logs

- **Date**: 2026-05-27
- **Status**: Accepted
- **Governing specs**: `030-security-identity-model`, `035-multi-agent-isolation`, `033-http-json-api`
- **Related issues**: `#303`, `#372`, `#401`, `#402`, `#403`

### Context

Concurrent agents cannot safely share one mutable registry without identity, authorization, workspace boundaries, and auditable operations.

### Decision

Use workspace-scoped registry/runtime operations, bearer auth for non-loopback bindings, operation-specific scopes, dev-loopback local tokens, runtime grants, and workspace-local audit logs.

### Alternatives Considered

- Document Traverse as single-agent-only.
- Add authentication later after exposing mutable APIs.
- Trust caller-supplied identity fields.

### Outcome

Multi-agent behavior is part of the governed runtime model. Local development remains ergonomic through dev-loopback mode, but production and non-loopback access must be authenticated.

## Decision 6: Insulate WASM Modules Behind a Traverse Host ABI

- **Date**: 2026-05-27
- **Status**: Accepted
- **Governing specs**: `038-wasi-host-insulation`, `025-wasm-executor-adapter`, `027-expedition-wasm-port`
- **Related issues**: `#369`

### Context

Traverse modules should not couple directly to a specific WASI implementation or unstable host imports.

### Decision

Introduce a stable Traverse Host ABI v1 as the sanctioned boundary between WASM modules and the runtime host, with load-time import validation and a governed Component Model migration path.

### Alternatives Considered

- Let modules import host/WASI functions directly.
- Treat the current WASI layer as the public contract.
- Delay ABI governance until after more module examples exist.

### Outcome

WASM modules gain a stable portability boundary, and host/runtime upgrades can happen without casually breaking module authors.

## Decision 7: Separate External Resource Access Through Connector Plugins

- **Date**: 2026-05-27
- **Status**: Accepted
- **Governing specs**: `039-connector-plugin-architecture`, `032-universal-data-access`
- **Related issues**: `#370`, `#371`

### Context

Capabilities need external data and resource access, but embedding resource-specific logic into capabilities or runtime core would weaken portability and governance.

### Decision

Use connector plugins as the extension point for external integrations, with declared dependencies, registration validation, governed discovery, and reference connectors for v0.

### Alternatives Considered

- Put all resource access into runtime core.
- Let each capability bundle its own connector logic without governance.
- Treat connectors as informal examples rather than a governed surface.

### Outcome

External integrations can grow without turning the runtime into an integration monolith or coupling capabilities to one host.

## Decision 8: Add Module Dependency Management Before Complex Composition Expands

- **Date**: 2026-05-27
- **Status**: Accepted
- **Governing specs**: `043-module-dependency-management`, `037-semver-range-resolution`, `041-workflow-composition-api`
- **Related issues**: `#338`, `#374`

### Context

As capabilities, agents, and WASM modules compose, dependency drift and unsatisfied version requirements become runtime risks.

### Decision

Govern dependency declaration, semver-compatible resolution, lock evidence, digest checks, and circular dependency rejection before relying on complex inter-capability composition.

### Alternatives Considered

- Resolve dependencies dynamically at execution time only.
- Require exact versions everywhere.
- Defer dependency governance until after app integration.

### Outcome

Registration and execution can produce deterministic dependency evidence, and downstream apps can rely on stable composition behavior.

## Decision 9: Treat Observability as Runtime Evidence, Not Optional Logging

- **Date**: 2026-05-27
- **Status**: Accepted
- **Governing specs**: `029-integrated-observability`, `012-execution-trace-tiered`, `010-runtime-state-machine`
- **Related issues**: `#362`

### Context

Traverse runtime decisions must be explainable to humans, agents, CI, and downstream apps. Plain logs are not enough for deterministic validation or UI presentation.

### Decision

Instrument runtime execution with structured trace evidence, OpenTelemetry-compatible spans, trace context propagation, deterministic test mode, and Traverse-specific semantic attributes.

### Alternatives Considered

- Keep only internal logs.
- Emit ad hoc JSON traces without OTel compatibility.
- Add observability after app integration.

### Outcome

Execution evidence becomes a first-class integration surface for debugging, UI feedback, and release validation.

## Decision 10: Harden Supply Chain Before Publishing Runtime Packages

- **Date**: 2026-05-27
- **Status**: Accepted
- **Governing specs**: `031-supply-chain-hardening`, `030-security-identity-model`, `038-wasi-host-insulation`
- **Related issues**: `#373`

### Context

Downstream consumers such as `youaskm3` need runtime and MCP artifacts they can verify, not just source code they can build locally.

### Decision

Add checksum, SBOM, signature/provenance, and CI verification gates for published artifacts, using Ed25519 as the baseline signing path and Sigstore for published artifacts.

### Alternatives Considered

- Publish packages first and add provenance later.
- Rely only on GitHub release tags.
- Treat SBOM and signatures as enterprise-only follow-up work.

### Outcome

Artifact publication is tied to verifiable provenance and release evidence, which supports real downstream adoption.

## Decision 11: Keep youaskm3 UI Ownership Outside Traverse

- **Date**: 2026-05-27
- **Status**: Accepted
- **Governing specs**: `019-downstream-consumer-contract`, `023-browser-hosted-mcp-consumer-model`, `033-http-json-api`, `042-mcp-library-surface`
- **Related issues**: downstream validation and consumer package tickets

### Context

`youaskm3` should use Traverse for runtime, state, MCP, capability execution, and governed business logic, while keeping product UI and chat experience in its own app.

### Decision

Traverse exposes app-facing runtime and MCP surfaces. It does not own the `youaskm3` UI, chat UX, layout, source presentation, or product behavior outside runtime/MCP execution.

### Alternatives Considered

- Build the `youaskm3` webapp inside Traverse.
- Make `youaskm3` call private Traverse internals.
- Keep Traverse as demos only and let `youaskm3` reimplement runtime/MCP behavior.

### Outcome

Traverse remains a reusable runtime project, and `youaskm3` becomes the first serious downstream consumer rather than a forked product shell.

## Decision 12: Use Semantic Versioning and Keep Release Readiness Explicit

- **Date**: 2026-05-27
- **Status**: Accepted
- **Governing specs**: `019-downstream-consumer-contract`, `028-schema-alignment-gate-v02`, `031-supply-chain-hardening`
- **Related issues**: package and release-readiness tickets

### Context

Traverse has a public v0.1.0 release, but downstream apps need clear expectations for compatibility, package artifacts, and first-release readiness.

### Decision

Follow semantic versioning, keep public surfaces explicitly versioned, and require release checklists plus validation artifacts before declaring app-consumable releases ready.

### Alternatives Considered

- Use informal release labels only.
- Treat release notes as the only compatibility statement.
- Version crates and artifacts independently without a release-readiness checklist.

### Outcome

Release readiness is auditable, and downstream users can reason about compatibility from specs, package artifacts, and release evidence.
