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

## Decision 13: Materialize Public Registrations from Verified Contract Artifacts

- **Date**: 2026-07-12
- **Status**: Accepted
- **Governing spec**: `063-registry-contract-materialization`
- **Related issues**: `#551`, `#552`

### Decision

Public records will publish immutable contract URL/digest metadata alongside
artifact metadata. Consumers will verify both, cache by digest, register
atomically, reject local `public` scope, and permit private shadows with
machine-readable evidence.

## Decision 14: Use a Runtime-Owned Production Artifact Router

- **Date**: 2026-07-12
- **Status**: Accepted
- **Governing spec**: `064-production-artifact-execution`
- **Related issue**: `#583`

### Decision

The runtime will route resolved WASM and explicitly host-registered native
artifacts through one production executor boundary. The production server uses
that router by default; the example executor is explicit-only.

## Decision 15: Verify Sigstore Bundles Offline Against Pinned Trust Policy

- **Date**: 2026-07-12
- **Status**: Accepted
- **Governing spec**: `065-sigstore-bundle-verification`
- **Related issue**: `#589`

### Decision

Traverse will use a narrow Rust Sigstore verifier interface. Production
verification consumes self-contained bundles offline, validates pinned trust
roots and publisher identity, and never accepts a string-prefix placeholder as
verification evidence.

## Decision 16: Emit Identity-Aware Events into a Durable Journal

- **Date**: 2026-07-12
- **Status**: Accepted
- **Governing spec**: `066-durable-identity-event-delivery`
- **Related issues**: `#591`, `#593`

### Decision

The runtime will emit identity-bearing events through a canonical sink. The
first durable store uses fsynced append-only journals, opaque persisted cursors,
and bounded retention; future tickets will evaluate its measured limits and
evolution path.

## Decision 17: Bound Durable Journal Retention and Write-Path Stalls

- **Date**: 2026-07-12
- **Status**: Accepted
- **Governing spec**: `067-durable-journal-retention-and-write-limits`
- **Related issue**: `#593`

### Decision

Retention reclaims space by deleting whole segments once every event in a
segment ages out, with segments rolling over on a configured max size or max
duration (default 64 MB or 10 minutes) to bound how long one old event can pin
a segment. A durable write that stalls past a configured timeout (default 2
seconds) rejects the event with a distinct `journal_write_timeout` error and
audit event, rather than blocking indefinitely or silently degrading to
in-memory-only delivery. This closes the remaining gap in issue #593's
Definition of Done left open by Decision 16.

## Decision 18: Deliver Traverse as Consumable Platform Embedder Packages

- **Date**: 2026-07-13
- **Status**: Accepted
- **Governing spec**: `068-public-platform-embedder-packages`
- **Related issues**: `#645`, `#646`, `#647`, `#648`, `#649`, `#650`; App
  References `#113`–`#117`

### Context

The approved embedder model and #553's implementation establish manifest
validation, an IDL, and CLI conformance, but do not give a Web, Swift, Android,
WinUI, or Linux app an SDK it can import to host a bundled Traverse runtime.

### Decision

Traverse will publish versioned, public platform packages that implement the
complete `embedder-api/1.0.0` lifecycle. They load application-owned runtime
and capability bundles, preserve runtime-owned workflow/output semantics, pass
the shared conformance corpus, and emit digest-backed release evidence. The
five platform slices are tracked separately so each downstream reference app
can become Ready only when its usable SDK exists.

### Outcome

The decision log is the authoritative design record. Spec 068 and its tickets
are derived traceability artifacts; they must not reopen this accepted
direction for a second design review.

## Decision 19: Keep Doc Approval on the Existing Analyze Contract

- **Date**: 2026-07-14
- **Status**: Accepted
- **Governing spec**: `069-doc-approval-pipeline-canonicalization`
- **Related issues**: `#538`, `#555`; App References `#111`, `#112`

### Context

Traverse already ships the deterministic `doc-approval.analyze@1.0.0` contract,
agent, manifest, and runtime request path. #538 proposed a distinct
`doc-approval.extract` capability with an incompatible nested output schema,
which would duplicate the public surface and require a migration without a
separate product need.

### Decision

Use `doc-approval.analyze` as the canonical first step of the doc-approval
pipeline. Implement only `doc-approval.recommend` as the second step and make
the pipeline `analyze -> recommend`. Do not introduce `doc-approval.extract`
or a migration from the established analysis contract.

### Outcome

#538 is superseded. #555 can implement the deterministic recommendation step;
the App Reference pipeline and manifests then follow that canonical two-step
contract.

## Decision 20: Make Runtime the Owner of Identity-Aware Event Envelopes

- **Date**: 2026-07-14
- **Status**: Accepted
- **Governing spec**: `070-runtime-event-sink-boundary`
- **Related issues**: `#591`, `#659`

### Decision

Runtime constructs complete identity-aware lifecycle event envelopes and emits
them through a narrow injected event-sink interface. The broker is a sink
adapter, not a concrete runtime dependency. Existing embedders retain a
compatible default no-op/in-memory sink. Live delivery and durable replay share
the same envelope and subject-filter semantics.

### Outcome

#591 can resume once Spec 070 lands; #659 then builds durable replay on the
same identity/filter boundary rather than inventing a second path.

## Decision 21: Retain the Durable Journal After Operational Evaluation

- **Date**: 2026-07-17
- **Status**: Accepted
- **Governing specs**: `066-durable-identity-event-delivery`,
  `067-durable-journal-retention-and-write-limits`
- **Related issues**: `#629`, `#630`

### Decision

Retain the initial append-only journal. The completed #713 matrix measured
Linux, macOS, Windows, and a Linux `fsync-pressure` profile using the
checked-in #629 harness. Host-local append p99 remained 0.524-6.160 ms,
recovery 2.732-5.366 ms, and replay 202k-312k events/s. The pressure profile
reached 41.557 ms p99, above the 25 ms investigation threshold but far below
the two-second fail-closed write timeout; it is a single constrained-profile
signal, so it does not justify a storage migration.

Do not add SQLite or a storage-provider boundary now. Preserve the existing
cursor and replay semantics, keep the weekly/manual measurement workflow, and
revisit this decision only after a comparable threshold breach occurs on two
consecutive runs or reproduces on the affected storage class. ADR-0009 records
the evidence and alternatives.

## Decision 22: Keep Application Source Out of the Traverse Runtime Repository

- **Date**: 2026-07-15
- **Status**: Accepted
- **Related issues**: `#703`, `#704`; App References `#151`

### Decision

Checked-in application UI, platform client demos, and starter/reference source
belong in `traverse-framework/App-References`. Traverse owns only runtime
conformance inputs: manifests, fixture agents, and deterministic test fixtures.
Those artifacts live under `examples/`, never `apps/`.

### Migration inventory

| Current path | Owner | Destination |
| --- | --- | --- |
| `https://github.com/traverse-framework/App-References/tree/main/apps/android-demo/` | Reference Apps | `reference-https://github.com/traverse-framework/App-References/tree/main/apps/android-demo/` |
| `https://github.com/traverse-framework/App-References/tree/main/apps/browser-consumer/` | Reference Apps | `reference-https://github.com/traverse-framework/App-References/tree/main/apps/browser-consumer/` |
| `https://github.com/traverse-framework/App-References/tree/main/apps/macos-demo/` | Reference Apps | `reference-https://github.com/traverse-framework/App-References/tree/main/apps/macos-demo/` |
| `https://github.com/traverse-framework/App-References/tree/main/apps/react-demo/` | Reference Apps | `reference-https://github.com/traverse-framework/App-References/tree/main/apps/react-demo/` |
| `https://github.com/traverse-framework/App-References/tree/main/apps/youaskm3-starter-kit/` | Reference Apps | `reference-https://github.com/traverse-framework/App-References/tree/main/apps/youaskm3-starter-kit/` |
| `apps/demo-fixtures/expedition-runtime-session.json` | Traverse fixture | `examples/fixtures/expedition-runtime-session.json` |
| `apps/meeting-notes/` | Traverse fixture | `examples/applications/meeting-notes/` |
| `apps/traverse-starter/` | Traverse fixture | `examples/applications/traverse-starter/` |

### Outcome

The Reference Apps migration preserves app validation against public Traverse
surfaces. Traverse follows with removal of the now-obsolete `apps/` directory
and a repository check that prevents application source from returning.

## Decision 23: Standardize Native Embedders on One Runtime-WASM Bridge

- **Date**: 2026-07-15
- **Status**: Accepted
- **Governing spec**: `071-native-runtime-wasm-bridge`
- **Related issues**: `#712`, `#647`, `#648`, `#649`

### Context

The Swift, Kotlin/Android, and .NET packages have deterministic API harnesses,
but no production runtime artifact or shared host boundary. Choosing a native
library or a platform-specific ABI per package would duplicate runtime
semantics and make conformance depend on three unrelated implementations.

### Decision

Ship one digest-addressed core WebAssembly orchestrator module implementing
`runtime-wasm-bridge/1.0.0`. The module owns lifecycle, submission, ordered
event production, compatibility decisions, cancellation, resource limits, and
structured errors. Platform packages only verify the bundle, instantiate the
module, marshal UTF-8 JSON through the governed memory ABI, and adapt event
delivery to idiomatic callbacks or streams.

Use WasmKit for Swift, Chicory for Kotlin/Android, and the Bytecode Alliance
Wasmtime .NET package for WinUI. Dependencies are exact-version pinned for a
release, reviewed for license and security status, and recorded in release
evidence. A host change is allowed only when the replacement passes the same
bridge and embedder conformance suites.

### Outcome

Spec 071 and ADR-0007 define the bridge. Native package tickets may implement
independently without changing runtime behavior or introducing a sidecar.

## Decision 24: Carry Compatible Lifecycle Through Bridge 1.1

- **Date**: 2026-07-16
- **Status**: Accepted
- **Governing spec**: `072-native-bridge-compatible-lifecycle`
- **Related issues**: `#716`, `#647`, `#648`, `#649`

### Context

Bridge 1.0 defined runtime initialization, submission, events, cancellation,
and shutdown, but omitted the compatible-capability start, stop, and kill
operations required by `embedder-api/1.0.0`. Implementing them in each native
package would move lifecycle ownership out of the runtime.

### Decision

Bridge 1.1 adds `traverse_compatible_start`, `traverse_compatible_stop`, and
`traverse_compatible_kill` using the existing UTF-8 JSON and output-descriptor
ownership rules. The runtime owns instance identifiers, state validation,
ordered lifecycle events, and shutdown cleanup. Bridge 1.1 is an additive ABI
version, but native packages requiring the complete embedder API must reject a
1.0 runtime artifact as incomplete.

### Outcome

All three native hosts implement one lifecycle contract and can resume without
inventing platform-specific compatible-capability semantics.

## Decision 25: Archive Stale April-2026 Spec Drafts with No Implementation

- **Date**: 2026-07-18
- **Status**: Accepted
- **Related issues**: none — repository/spec-hygiene decision, no implementation ticket

### Context

Five spec directories from April 2026 (`019-local-browser-adapter-transport`,
`020-downstream-integration-validation`, `021-app-facing-operational-constraints`,
`022-mcp-wasm-server`, `023-downstream-publication-strategy`) exist on `main`,
still `Status: Draft`, and were never added to
`specs/governance/approved-specs.json`. No commit in the repository's history
references any of their spec IDs. Two of them (`019`, `023`) share a spec
number with a different, later spec that was approved and implemented instead,
suggesting these were early exploratory drafts superseded before the real
scope was specified.

### Decision

Treat "older than ~60 days, zero implementation commits, never approved" as
sufficient signal on its own — no per-spec review needed. Move all five to
`Status: Superseded` in their own `spec.md`, with a one-line note pointing to
whatever superseded it where known (`019` → `019-downstream-consumer-contract`,
`023` → `023-browser-hosted-mcp-consumer-model`; `020`/`021`/`022` noted as
superseded with no specific direct successor identified).

### Alternatives Considered

- Review each of the five individually before deciding — more thorough, but
  the batch signal (age + zero implementation + never approved) was judged
  strong enough on its own.
- Leave them untouched — avoids any risk of archiving something still wanted,
  but leaves the spec directory permanently cluttered with dead drafts.

### Outcome

`specs/` no longer carries unapproved, unreferenced drafts alongside the real
governing-spec history. The one-line successor notes preserve the "why" for
anyone who finds the archived draft later.

## Decision 26: Retroactively Approve Specs for Already-Completed Governance/Docs Work

- **Date**: 2026-07-18
- **Status**: Accepted
- **Related issues**: `#188`, `#190`

### Context

`188-codex-agent-coordination` and `190-readme-rewrite` were both left
`Status: Draft` and never added to `approved-specs.json`, but the work they
describe was independently verified complete: `AGENTS.md` and
`docs/multi-thread-workflow.md` already implement 188's exact pre-flight/claim
rules (FR-001 through FR-004), and `README.md` already has every badge and
section 190 required, including the GitHub repository description and topics
188 asked for.

### Decision

Add both specs to `specs/governance/approved-specs.json` with `status:
approved` and `immutable: true`, noting they were approved retroactively after
independent verification that the implementation already satisfies every
functional requirement — no code changes needed. `188-codex-agent-coordination`
governs `AGENTS.md` and `docs/multi-thread-workflow.md`; `190-readme-rewrite`
governs `README.md`.

### Alternatives Considered

- Archive both as superseded, on the reasoning that formal approval doesn't
  matter once the goal is met — rejected because the spec content is still an
  accurate description of the current, real behavior, unlike the five drafts
  in Decision 25.
- Leave them unapproved indefinitely — leaves a governance gap where real,
  load-bearing behavior (agent coordination rules, README requirements) has no
  approved spec backing it.

### Outcome

The spec-alignment gate can now correctly attribute `AGENTS.md`,
`docs/multi-thread-workflow.md`, and `README.md` changes to an approved spec
instead of leaving them ungoverned.

## Decision 27: Raise the Swift Embedder's WasmKit Floor to 0.3.1 for Public Resource Controls

- **Date**: 2026-07-18
- **Status**: Accepted
- **Governing spec**: `071-native-runtime-wasm-bridge`
- **Related issues**: `#740`, `#647`

### Context

`packages/swift/TraverseEmbedder` pins WasmKit 0.2.2, which exposes no public
fuel/epoch/deadline interruption hooks and no public memory-growth limiter —
only an `@_spi(Fuzzing) Store.resourceLimiter`, which is not a supported
production API (documented in the package's own `dependency-review.json`,
reviewed 2026-07-16). WasmKit 0.3.1 has the public hooks needed, but requires
Swift tools 6.3, macOS 15, and iOS 18 — newer than the package's current Swift
6.0 / macOS 14 / iOS 17 floor.

### Decision

Bump `packages/swift/TraverseEmbedder/Package.swift` to WasmKit 0.3.1 and the
corresponding Swift 6.3 / macOS 15 / iOS 18 minimums. Same engine, same
integration code, no new dependency-review risk — the tradeoff is a narrower
supported-device matrix (drops macOS 14 / iOS 17) in exchange for genuine
production-grade resource controls.

### Alternatives Considered

- Track upstream WasmKit for a 0.2.x-compatible public-hook release, or
  contribute a backport — keeps the wider device floor, but the timeline isn't
  in Traverse's control.
- Swap to a different Swift WASM engine entirely — preserves both the device
  matrix and gets real safety, but means a full re-integration (bridge, ABI
  validation, digest verification, tests) against an unproven alternative,
  for no confirmed benefit over just bumping WasmKit.

### Outcome

Issue `#740` tracks the version bump. `packages/swift/TraverseEmbedder`'s
`dependency-review.json` `known_limitations` entry should be updated once the
bump lands, and its resolution unblocks `#647`'s remaining Spec 071
release-evidence item.

## Decision 28: Define Native Embedder Baseline 1

- **Date**: 2026-07-18
- **Status**: Accepted
- **Governing spec**: `073-native-embedder-release-baseline`
- **Related issues**: `#752`, `#750`, `#751`, `#647`

### Context

Spec 071 defines the immutable 1.0 core-Wasm bridge base. Spec 072 adds the
runtime-owned compatible-capability lifecycle and states that a complete
`embedder-api/1.0.0` package needs bridge 1.1 or later within major version 1.
Without a release-level composition, a package version and a runtime digest do
not tell a downstream consumer whether all public embedder operations are
available or which host profile certified them.

### Decision

Define Native Embedder Baseline 1 as `embedder-api/1.0.0` plus
`runtime-wasm-bridge >=1.1.0,<2.0.0`. Native package releases must record the
supported bridge range, exact certified bridge/runtime/engine/conformance
inputs, and their host resource-control profile. They validate the mandatory
bridge 1.1 exports as well as the version range. The bridge module remains
import-free core Wasm; bounded capability-host services remain governed by
Spec 057.

### Alternatives Considered

- Keep bridge 1.0 as the release baseline — rejected because it cannot
  implement compatible lifecycle operations inside the runtime-owned boundary.
- Require exactly 1.1.0 — rejected because it blocks compatible 1.1 patch
  releases without a semantic reason.
- Rewrite Specs 071 or 072 — rejected because both approved artifacts are
  immutable and accurately preserve the additive ABI history.

### Outcome

Spec 073 and ADR-0010 record the release baseline. #750 delivers the real
artifact and evidence, #751 completes public native event parity, and #647
resolves the Swift production resource-control prerequisite.
