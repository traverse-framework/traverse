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

## Decision 29: Require Supported Swift Resource Controls Before Certification

- **Date**: 2026-07-18
- **Status**: Accepted
- **Governing spec**: `074-swift-native-resource-control-certification`
- **Related issues**: `#761`, `#762`, `#647`, `#750`, `#758`

### Context

Decision 27 selected WasmKit 0.3.1 based on a false premise: its official
source still exposes `Store.resourceLimiter` only through `@_spi(Fuzzing)` and
does not expose a supported fuel, epoch, deadline, or interruption API.
Raising the Swift and platform floors alone would leave untrusted execution
without the required supported resource controls.

### Decision

Do not certify the Swift package or a cross-platform Native Embedder Baseline
until its runtime profile proves bounded memory growth and deterministic
execution interruption through supported public APIs on physical iOS and
macOS. Prohibit SPI and watchdogs that cannot stop untrusted execution.
Evaluate supported options in #762 before changing engines. A replacement
engine needs its own approved ADR, security/license review, Apple distribution
evidence, and full bridge conformance.

### Alternatives Considered

- Upgrade to WasmKit 0.3.1 alone — rejected because it does not expose the
  required supported controls.
- Use the existing SPI — rejected because unsupported APIs cannot justify a
  production certification claim.
- Adopt an alternative engine immediately — rejected pending device-level
  feasibility, packaging, security, and conformance evidence.

### Outcome

Decision 27 is superseded. #647 remains blocked on a certified Swift profile;
#761 records the governing requirements and #762 evaluates the smallest
supported path. Kotlin and .NET work may continue without calling the release
cross-platform.

## Decision 30: Approve the Native Runtime Distribution Contract

- **Date**: 2026-07-19
- **Status**: Accepted
- **Governing spec**: `075-native-runtime-distribution-contract`
- **Related issues**: `#755`, `#750`, `#756`, `#757`, `#758`

### Context

Spec 071 defines the immutable bridge ABI and Spec 073 defines the release
compatibility baseline, but neither states how the one canonical
`runtime.wasm` build becomes an identified, digest-pinned, host-certified
release that Swift, Kotlin, and .NET packages actually acquire and resolve.
Traverse #755 drafted spec 075 (originally numbered 074, renumbered after a
concurrent numbering collision with #761's
`074-swift-native-resource-control-certification`) and ADR-0012 (originally
drafted as ADR-0011, renumbered for the same reason) to close that gap, with
its Definition of Done requiring explicit human approval rather than the
default post-brainstorm auto-approval.

### Decision

Approve spec `075-native-runtime-distribution-contract` and ADR-0012 as
drafted: runtime artifact releases are identified by an immutable
`runtime_version` + certified `bridge_version` + SHA-256 digest tuple,
resolution deterministically rejects tampered, incompatible, or uncertified
artifacts before instantiation, releases remain independently resolvable
after upgrade, and the distribution metadata schema is host-agnostic across
Swift, Kotlin, and .NET. Distribution is implemented through Traverse's
existing registry publish/resolve infrastructure (Spec 051) rather than a
bespoke channel.

### Alternatives Considered

- Leave the spec in Draft and let #756/#757/#758 proceed against an
  unapproved contract — rejected because those tickets are explicitly
  blocked on this spec's approval and an unapproved contract cannot govern
  new code paths under the spec-alignment gate.
- Fold this contract into Spec 073 as an amendment — rejected because Spec
  073 is immutable and already approved; a distribution layer beneath it is
  additive, not a revision.

### Outcome

`crates/traverse-native-bridge/` (already introduced by #756's in-progress
work) and `docs/adr/0012-native-runtime-distribution-channel.md` are now
governed by spec 075 in `specs/governance/approved-specs.json`. #756, #757,
and #758 may proceed and declare `075-native-runtime-distribution-contract`
as their governing spec.

## Decision 31: Reconcile Spec 037's Approval Record

- **Date**: 2026-07-21
- **Status**: Accepted
- **Governing spec**: `037-semver-range-resolution`
- **Related pull requests**: `#358`, `#794`

### Context

PR #355 introduced Spec 037 with a Draft header. PR #358 formally registered
the same immutable `037-semver-range-resolution` specification as approved on
2026-04-19. The registry is the repository's canonical approval record, but
the source header was never reconciled. That stale header incorrectly made the
targeted registry-lookup performance work appear to be blocked on a new
architecture decision.

### Decision

Record Spec 037 as approved as of its original 2026-04-19 registry approval.
This is a metadata correction only: Spec 037 remains version `1.0.0` and its
functional requirements are unchanged. In particular, NFR-003 continues to
require range evaluation in `O(n log n)` time or better for the registered
versions of the requested capability id.

The targeted lookup implementation must preserve existing exact-version and
range-resolution compatibility, produce deterministic results for identical
registry state and requests, and validate that unrelated capability entries
cannot affect lookup results. Its regression evidence must cover equivalence
with the prior candidate set and a large unrelated-entry case.

### Alternatives Considered

- Create a successor specification — rejected because no requirement or
  contract changed; a successor would falsely imply a new API decision.
- Treat the registry entry as erroneous and re-open approval — rejected
  because PR #358 explicitly registered Spec 037 as approved and immutable.
- Leave the mismatch in place — rejected because it creates avoidable tracker
  and implementation blockage while obscuring the actual approved contract.

### Outcome

The approval record is internally consistent. The targeted lookup work may
proceed under Spec 037, provided it supplies the stated compatibility,
determinism, and regression evidence.

## Decision 32: Approve the Companion Public Embedded Trace API

- **Date**: 2026-07-21
- **Status**: Accepted
- **Governing spec**: `517-embedded-trace-api`
- **Related pull request**: `#797`
- **Related Project tickets**: `embedded-trace-api-decision`, `embedded-trace-api`

### Context

Trace Explorer requires a production embedded path for browsing the current
local runtime session, but the existing public embedder API has no trace
operations. Exposing `RuntimeTrace` directly would disclose unsafe request and
result data, and adding required operations to the existing public Rust
embedder trait would break external implementations. The older `TraceStore`
and MCP tools are not the source used by the current embedded runtime path and
do not form a portable consumer contract.

### Decision

Approve Spec 517 and ADR-0016 as drafted. Traverse publishes the additive,
versioned `embedded-trace-api/1.0.0` companion surface. It provides only
public `trace.list` and `trace.get` operations for the owning application and
the current embedded session. Results are deterministic, cursor-paged, and
bounded by documented process-local retention.

The public projection includes only safe runtime-owned diagnostic evidence.
Raw inputs, outputs, caller and correlation metadata, private trace entries
and hashes, raw telemetry attributes, and unfiltered error details are
prohibited. The API clears its retained history at shutdown or
reinitialization, makes no HTTP or sidecar fallback, and does not promise
durable or cross-restart history. A separate extension capability preserves
all existing `embedder-api/1.0.0` consumers and external Rust implementers.

### Alternatives Considered

- Extend the baseline embedder trait directly — rejected because required
  trait methods would be a breaking public API change.
- Expose `RuntimeTrace` or `TraceStore` directly — rejected because they are
  not a portable safe consumer contract.
- Reuse the HTTP trace endpoint — rejected because it preserves the sidecar
  exception that this decision removes.
- Require durable trace storage first — rejected because the current
  no-sidecar diagnostic use case is independently valuable.

### Outcome

Spec 517 is immutable and governs the runtime, embedder, Web embedder package,
ADR-0016, and its own artifacts. The `embedded-trace-api` implementation ticket
may move from Blocked to Ready. Its first delivery must prove that a
Trace Explorer-equivalent Web consumer can browse local traces without HTTP
and that baseline embedder conformance remains compatible.

## Decision 33: Land Two Orphaned Governing Specs to Fix a Pre-Existing Stale-Spec-ID Bug

- **Date**: 2026-07-21
- **Status**: Accepted
- **Governing specs**: `077-metadata-graph`, `078-federation-registry-routing`
- **Related pull request**: (this change)
- **Related repo**: flagged from `traverse-framework/registry`'s
  `specs/014-extraction-compatibility` decision-log entry 33, which found
  this bug while auditing `traverse-registry` ahead of extraction but left
  it unfixed as out of that repo's scope (registry-scope-only).

### Context

Two literal spec-ID strings embedded in `crates/traverse-registry/src/`
did not correspond to any spec in `specs/governance/approved-specs.json`:
`"015-metadata-graph"` (a `const` in `graph.rs`, used as every projected
metadata-graph snapshot's `governing_spec`) and
`"026-federation-registry-routing"` (used only in `federation.rs` and
`federation_operator.rs` test fixtures, as sample `TrustRecord.
approved_spec_refs` / `ApprovalChainEntry.spec_ref` values). Both slots
(`015`, `026`) were reassigned to unrelated specs (`015-capability-
discovery-mcp`, `026-event-broker`) during the v0.2.0 governance batch
(issue #209, issue #207), so `approved_spec_registry_contains()` has
returned `false` for both original strings since that batch landed —
independent of the registry-extraction work that surfaced it.

Investigation (issue #37 comment history, git history of `graph.rs` via
issue #62, and the abandoned `origin/022-mcp-wasm-server` branch at commit
`b81a17b`) found that in both cases a real, complete spec document had been
written and, in the federation case, even self-marked "Status: Approved" —
but neither was ever merged into `approved-specs.json` before its numeric
slot was taken by different work. The implementations (`graph.rs` via PR
#97, `federation.rs` via PR #240 and follow-ons) shipped anyway, each
assuming its governing spec had been formally approved when it had not.

### Decision

Rather than point the two stale constants at an existing-but-unrelated
approved spec (which would repeat the same category of error in a new
form), land the two already-written spec documents under fresh IDs:
`077-metadata-graph` (written retroactively against the shipped
`graph.rs` behavior, since no committed draft of the original
`015-metadata-graph` could be found in any branch) and
`078-federation-registry-routing` (the original `026-federation-registry-
routing` document, recovered from `b81a17b`, reviewed against the shipped
`federation.rs`/`federation_operator.rs` behavior and found still
accurate, with its own stale cross-references to two other still-
unapproved draft specs removed rather than carried forward). Both new
`approved-specs.json` entries govern the specific source files their spec
actually describes (`graph.rs`; `federation.rs` and
`federation_operator.rs`) rather than the whole crate. `graph.rs`'s
`METADATA_GRAPH_GOVERNING_SPEC` constant and all `federation.rs`/
`federation_operator.rs` test-fixture literals are updated to the new IDs.

### Alternatives Considered

- Point the constants at `015-capability-discovery-mcp` /
  `026-event-broker` (today's real owners of those numeric slots) —
  rejected, since neither actually governs metadata-graph projection or
  federation routing; this would fabricate a false governance claim
  identical in kind to the bug being fixed.
- Point the constants at the closest broad existing spec that already
  governs `traverse-registry/` (e.g. `007-workflow-registry-traversal`) —
  rejected; `007` explicitly excludes "full metadata graph query model"
  from its own scope, and no approved spec describes federation routing.
- Leave the constants unfixed, matching `traverse-framework/registry`'s
  deliberate choice not to paper over this with passthrough entries —
  appropriate for that repo (out of its scope, and a passthrough there
  would mask rather than fix), but this repo owns the actual bug and can
  close it properly instead of leaving it permanently broken.

### Outcome

`077-metadata-graph` and `078-federation-registry-routing` are approved and
immutable, each governing exactly the source file(s) it describes.
`approved_spec_registry_contains()` now returns `true` for the spec IDs
actually embedded in `traverse-registry`'s shipped code. All existing
`traverse-registry` and `traverse-cli` tests pass unchanged in behavior —
only the literal spec-ID strings changed.

## Decision 34: Persist Auditable Traces in a Separate Durable Journal

- **Date**: 2026-07-28
- **Status**: Accepted
- **Governing spec**: `079-durable-trace-journal`
- **Related ADR**: ADR-0017

### Decision

Persist auditable execution traces through the append-only event journal, not
the host-owned DataStore. The journal may use the same host-selected storage
infrastructure, but trace privacy, retention, recovery, and failure semantics
remain independent. Audited execution fails before success is returned when
its durable trace cannot be committed; private trace payloads remain outside
this slice.

### Outcome

The durable trace journal is an approved, separately governed product surface.
It does not assign a DataStore root to the runtime or expand DataStore format
migration scope.

## Decision 35: Prepare Registry Dependencies in a Host-Owned Offline Cache

- **Date**: 2026-07-28
- **Status**: Accepted
- **Governing spec**: `080-embedded-registry-cache`
- **Related issue**: #826

### Decision

Production embedders prepare `registry_ref` dependencies explicitly using a
host-provided network source and content-addressed cache. Initialization and
execution consume only verified local cache entries and never use a CLI
sidecar, an App-References manifest rewrite, or runtime network fallback.

### Outcome

Host-native resolution is the sole supported production path for
`registry_ref`; application wrappers may automate preparation but do not own a
separate materialization architecture.

## Decision 36: Make Synced Registry Discovery Local and Offline-First

- **Date**: 2026-07-28
- **Status**: Accepted
- **Governing spec**: `081-registry-browse-search`
- **Related issue**: #814

### Decision

`traverse-cli registry list` and `search` operate only on the locally synced
public index. Contract summaries are fetched and cached only through an
explicit action; valid stale local state remains discoverable with provenance,
and runtime execution never performs discovery network access.

### Outcome

Registry discovery becomes a deterministic, offline-capable CLI feature
without changing the thin contract-first registry index.

## Decision 37: Keep DataStore Format Migration Explicit and Host-Owned

- **Date**: 2026-07-28
- **Status**: Accepted
- **Governing spec**: `082-datastore-format-migration`
- **Related ADR**: ADR-0020
- **Related pull request**: #839

### Decision

Only the host owning a durable DataStore root may request a named migration.
Every migration validates its source, preserves a verified backup, verifies
the target before atomic commit, and exposes explicit verified restore. The
runtime and ordinary CLI commands never discover a root or perform implicit
migration, backup, restore, or downgrade.

### Outcome

The safety and ownership policy is approved. No format transition or
implementation is authorized until a successor specification names the exact
source-to-target format, backup representation, stable errors, and host API.

## Decision 38: Establish the Production App Readiness Baseline

- **Date**: 2026-07-28
- **Status**: Accepted
- **Planning spec**: `524-production-app-readiness`
- **Extends**: Decisions 34–37 and Specs 079–082

### Decision

Traverse v1's production bar is an embedded multi-platform app that consumes
verified registry capabilities offline, retains safe audit traces across
restart, and upgrades host-owned local state without data loss. The registry
has Certified, Community, and Kit/example tiers; discovery defaults to
Certified; production bundles use committed exact lockfiles and host-prepared
verified cache generations. Updates are explicit and reversible, and security
yanks are enforced through locally known deadlines or minimum-safe versions.

Certified capability admission requires signed provenance, validation,
conformance evidence, and maintainer support policy. Certified platform status
requires equal conformance across Web, Linux/Rust, Apple, Android, and
Windows/.NET. Trace and state ownership, encryption keys, tenancy, and user
authorization remain host-owned. The durable journal is separate from
DataStore; `local-datastore/2` is the first explicit file-backed migration
target; roots are single-writer in v1.

### Outcome

This decision is decomposed into bounded successor specifications and tickets;
it is not itself an implementation authorization.

## Decision 39: Host-Explicit DataStore Retention and Verified Zip Backup/Restore

- **Date**: 2026-07-29
- **Status**: Accepted
- **Governing spec**: `083-datastore-retention-backup` (`specs/526-datastore-retention-backup`)
- **Related ADR**: ADR-0021
- **Related Project 1**: Specify DataStore retention compaction backup and restore policy

### Decision

Retention prune and backup/restore are host-explicit via a separate
`DataStoreMaintenance` port sharing the DataStore root and exclusive lock.
v1 retention knobs are count and age with host-supplied `as_of` (no OS clock).
Prune is interruptible with partial evidence. Backups are zip+manifest;
restore verifies then atomically replaces the root. Compaction is Future.

### Outcome

Unlocks drafting/approval of Spec 083 and a bounded Implement ticket after
approval. Does not authorize compaction or auto-prune.

## Decision 40: Encrypt Only Private DataStore Records via KeyProvider

- **Date**: 2026-07-29
- **Status**: Accepted
- **Governing spec**: `084-datastore-encryption-at-rest` (`specs/527-datastore-encryption-at-rest`)
- **Related ADR**: ADR-0022
- **Related Project 1**: Specify DataStore encryption at rest and key lifecycle

### Decision

Private records use AES-256-GCM at rest with host `KeyProvider` (v1
callback/in-memory). Public remains integrity-only. No provider fails private
ops closed. Classification is immutable. No in-place rotation in v1; re-key via
Spec 083 backup/restore. OS/KMS providers are Future.

### Outcome

Unlocks Spec 084 approval path. Browser private waits on a KeyProvider
follow-on after IndexedDB CRUD.

## Decision 41: IndexedDB as Same-Port Public DataStore Backend

- **Date**: 2026-07-29
- **Status**: Accepted
- **Governing spec**: `085-datastore-indexeddb` (`specs/528-datastore-indexeddb`)
- **Related ADR**: ADR-0023
- **Related Project 1**: Specify browser IndexedDB DataStore adapter contract

### Decision

IndexedDB implements the same DataStore port/envelopes for public CRUD with
Web Locks exclusive ownership and typed quota errors. Private encryption and
maintenance are unsupported in v1.

### Outcome

Unlocks Spec 085 approval path for web permanence without blocking on crypto
or zip-backup parity.

## Decision 42: Opt-In, Anonymous Runtime Usage Telemetry Behind a Provider-Neutral Port

- **Date**: 2026-08-04
- **Status**: Accepted
- **Governing spec**: `088-runtime-usage-telemetry` (`specs/536-runtime-usage-telemetry`)
- **Related ADR**: ADR-0030
- **Related Project 1**: Runtime usage telemetry for capability resolve/execute
- **Origin**: `traverse-framework/registry`'s `docs/decision-log.md` Decision 47
  (registry `/brainstorm`, closing registry#134), handed off here per that
  entry's own execution boundary since the actual instrumentation touches
  `traverse-cli` and the shared `traverse-contracts` port, both owned by this
  repo (`crates/traverse-registry`, where capability *resolution* actually
  happens, was extracted to `traverse-framework/registry` by Spec 051 and is
  governed there under its own `013-inherited-registry-governance`/FR-002 —
  this decision does not re-litigate that boundary).

### Decision

Runtime usage telemetry answers "is a published capability actually being
resolved/executed," distinct from this repo's OTel integrated-observability
work (Spec 029, operator-facing traces/logs/metrics for a single deployment).
It is a separate, deliberately minimal, opt-in-only signal reported to the
Traverse maintainers, not an operator-facing signal.

- Two counters, not one: a `resolve` event (registry version lookup) and an
  `execute` event (`capability execute`/`serve`, actual WASM invocation),
  tracked per exact `namespace/id@version`.
- **Opt-in, off by default.** No prompts, ever. A persistent CLI config
  command (`traverse-cli telemetry enable`/`disable`) is the only way to turn
  it on.
- Each event carries `namespace/id@version`, event type, timestamp, and an
  anonymous random install ID (a UUID generated once locally on first
  opt-in) — enough to distinguish one automated pipeline from many distinct
  users, nothing that identifies a real person or machine.
- Collected by a purpose-built hosted product-analytics tool (e.g. PostHog),
  not this repo's website-analytics account (a different, cookie-based-web
  shaped tool) and not new self-hosted infrastructure.
- Sent fire-and-forget with a short timeout; any failure is swallowed
  silently and must never delay or fail the real CLI command it's attached
  to.
- Architecturally: a `UsageTelemetrySink` port trait lives in
  `traverse-contracts` (the existing pattern for provider-neutral ports, see
  ADR-0029's transport-port precedent) with a no-op default. `traverse-cli`
  owns the only real adapter (config, install ID, PostHog HTTP client).
  `crates/traverse-registry` (external, `traverse-framework/registry`-owned)
  calls the port at its resolution call site but never depends on the
  concrete adapter, network code, or opt-in state directly — that keeps the
  registry crate portable and testable without a live collector, matching
  how the hosted-transport port (Spec 087) keeps DataStore sync provider-blind.

### Outcome

Unlocks Spec 088 approval path. `crates/traverse-registry`'s own resolve-side
hook is out of this repo's governance — tracked as its own spec
(`traverse-framework/registry`'s Spec 015) and ticket in that repo's Project 3,
sequenced behind this repo publishing the new `traverse-contracts` trait.

## Decision 43: Provision the Real Collector as a Hardcoded PostHog Cloud Key in the Published Crate

- **Date**: 2026-08-04
- **Status**: Accepted (provisioning itself deferred — see Outcome)
- **Governing spec**: `088-runtime-usage-telemetry`
- **Related issues**: `#928`
- **Origin**: `/brainstorm 928`, closing the one open question left in Decision 42
  — *which* hosted collector, and how its endpoint/API key actually reaches a
  running `traverse-cli`.

### Context

Decision 42 named "a purpose-built hosted product-analytics tool (e.g.
PostHog)" but left the concrete provider and delivery mechanism open. #927
(port trait) and #928 (config commands, install ID, real HTTP sink) both
shipped fully coded and tested against that open slot — `wire_usage_telemetry_sink()`
reads `TRAVERSE_TELEMETRY_ENDPOINT`/`TRAVERSE_TELEMETRY_API_KEY` from the
process environment, falling back to the no-op sink when either is unset —
but #928's Definition of Done also requires "a real hosted PostHog (or
equivalent) project is provisioned and its endpoint/key wired into the
adapter," which is account/infrastructure setup, not code, and stayed
unresolved.

This surfaced a second, non-obvious question once the provider was picked:
this repo's only release channel is `cargo publish` to crates.io on a `v*`
tag (`scripts/ci/publish_crates.sh`) — there is no separate compiled-binary
release pipeline. crates.io distributes source, compiled by `cargo install`
on each user's own machine. An env-var-only design (the current shipped
code) means telemetry only ever activates for whoever manually exports both
variables in their own shell — in practice nobody but the maintainers
testing locally — which defeats the "real adoption signal" this feature
exists for (Decision 42, registry Decision 47).

### Decision

- **Provider: PostHog Cloud** (free tier), not self-hosted and not a
  different tool. `build_event_payload()` in `crates/traverse-cli/src/telemetry.rs`
  already emits PostHog's exact capture-API shape (`api_key`, `event`,
  `distinct_id`, `properties`), so this needs no code rework.
- **Delivery: baked into the published crate as a hardcoded constant**, not
  an env-var-only runtime lookup and not a build.rs-generated secret
  injected only at publish time. A PostHog *project* API key (as opposed to
  PostHog's secret *personal* API key) is a write-only capture token,
  designed to be publicly embeddable — the same trust model as putting it in
  client-side JS, and no different from what `strings` would recover from a
  compiled binary. Being visible in git history and on crates.io is expected
  and not a leak for this token type. The existing env-var path
  (`TRAVERSE_TELEMETRY_ENDPOINT`/`TRAVERSE_TELEMETRY_API_KEY`) stays as a
  dev-only override for testing against a different collector, rather than
  being removed.
- **A real, separate compiled-binary release pipeline (e.g. GitHub Releases,
  cargo-dist) was explicitly considered and rejected for this ticket** — it
  would let the key stay out of the published crate source entirely, but is
  a substantially larger, unscoped project belonging to its own future
  ticket, not #928.

### Outcome

The PostHog project itself has **not** been created — creating third-party
accounts is outside what Claude Code performs on the user's behalf under any
instruction. #928 stays open/Blocked exactly as-is: fully coded, tested, and
merged (#932, #933), with the no-op sink wired whenever the two config
values are absent, and #929's execute-path wiring (#934) already shipped
against the same port. Whenever the user creates the PostHog project and
hands over its endpoint/key, the remaining work is a one-line hardcode into
`telemetry.rs` plus a PR — no further design decisions.

## Decision 44: Wire `PlacementRouter` Into the Live Execution Path Is Not a New Decision — It's an Unimplemented Approved Requirement

- **Date**: 2026-08-05
- **Status**: Accepted
- **Governing spec**: `210-runtime-placement-router` (already approved; FR-006, Assumptions)
- **Related issues**: `#963`
- **Origin**: `/brainstorm` session closing the eventing-architecture gap analysis (UMA white paper comparison)

### Context

A full read of the UMA white paper's eventing model (capability↔runtime,
runtime↔runtime, runtime↔UI) against the current codebase found that
`PlacementRouter::execute` — which evaluates placement, selects an executor,
writes trace entries, and (Step 5) publishes a `Subscribable` capability's
declared `emitted_events` to `EventBroker` — is fully built and tested
(`router/mod.rs`, `router_tests.rs`, `expedition_wasm_tests.rs`,
`thread_pool_integration.rs`) but never constructed or called from the live
`Runtime::execute` path in `traverse-runtime/src/lib.rs`. That path instead
only emits one hardcoded system lifecycle event
(`RUNTIME_EXECUTION_EVENT_TYPE`) per run via `emit_execution_lifecycle_event`
— no capability-declared business event ever reaches `EventBroker` in
production today.

Before treating this as an open brainstorm question, `210-runtime-placement-router`
was checked directly: **FR-006** already states *"PlacementRouter is the
single public entry point for all capability execution in traverse-runtime,"*
and its Assumptions section already states *"PlacementRouter replaces any
ad-hoc execution wiring currently in traverse-runtime."*

### Decision

This is not a design decision to brainstorm — it is a gap between an already
Approved, immutable spec and the current implementation. It goes straight to
a ticket ("wire `PlacementRouter` into `Runtime::execute`, retire the
ad-hoc lifecycle-only emission path") with no new spec, ADR, or brainstorm
question required.

### Alternatives Considered

None — the governing spec already forecloses alternatives (single entry
point, replaces ad-hoc wiring).

### Outcome

Filed as the first, prerequisite ticket in the eventing sequence. Every
other decision in this session assumes this lands first, since it's what
makes `EventBroker` carry real capability events instead of only lifecycle
telemetry.

## Decision 45: Governance Vehicle for the Runtime-Event-to-Transport Gap Is New, Narrow Specs — Not a `534` Addendum

- **Date**: 2026-08-05
- **Status**: Accepted
- **Governing specs**: extends `207-event-broker`, `534-ecca-event-products`
- **Related issues**: `#964`, `#965`
- **Origin**: `/brainstorm` session, question 1

### Context

`534-ecca-event-products` (approved 2026-07-29, the newest eventing spec)
explicitly scopes in "runtime behavior" and "host adapters" as still-open
work — which is exactly where "how does a domain event reach a UI
transport" belongs. But `.specify/memory/constitution.md` and
`approved-specs.json` (`"immutable": true` on every entry) both state specs
are treated as immutable once approved for implementation, so `534` cannot
be edited in place. The real choice was between one new combined spec
covering the whole bridge, or several narrow specs matching this repo's
existing convention (every other spec here — `003`, `013`, `018`, `207`,
`534`, etc. — is single-purpose).

### Decision

Close the gap with separate, narrow specs per concern rather than one
combined spec: one spec for "production SSE reads from `EventBroker`"
(extends `207` + `534`), and downstream north-star specs (below) for
transport and capability ABI, each extending their own governing chain.

### Alternatives Considered

- Edit `534` in place — rejected, specs are immutable once approved.
- One combined spec covering SSE migration + `browser_adapter.rs` fate +
  transport — rejected, couples a "must do" (governed plumbing) to a
  "policy call" (dev-tool disposition) and has no precedent in this repo's
  spec granularity.

### Outcome

Unlocks the SSE-migration spec as its own ticket, independent of the
`browser_adapter.rs` and transport decisions below.

## Decision 46: Keep `browser_adapter.rs` As-Is for Now; Track Its Eventual Merge as a North-Star Item

- **Date**: 2026-08-05
- **Status**: Accepted
- **Governing specs**: `013-browser-runtime-subscription`, `019-local-browser-adapter-transport` (unchanged)
- **Related issues**: `#973`
- **Origin**: `/brainstorm` session, question 2

### Context

`browser_adapter.rs` (`traverse-cli browser-adapter serve`) is a standalone,
single-connection, local-dev-only server implementing spec `013`'s governed
browser-subscription message contract, replaying one hardcoded canonical
outcome. It is not the production HTTP API server and is not backed by
`EventBroker`. Three options existed: retire it, merge it into the
production HTTP API (making spec `013`'s ordered message contract real in
production), or leave it unchanged.

### Decision

Leave it unchanged for now — it doesn't block the SSE-on-`EventBroker`
migration. The user explicitly flagged that this should not be forgotten:
merging it into the production server (so spec `013`'s ordered
`subscription_established → state → trace → terminal_result →
stream_completed` contract becomes real, `EventBroker`-backed production
behavior) is tracked as a north-star ticket, gated on the WebSocket
transport (Decision 47) shipping first.

### Alternatives Considered

- Retire it entirely — rejected for now: loses a low-friction local-testing
  workflow with no replacement ready yet.
- Merge into production HTTP API now — rejected for now: the production
  endpoint doesn't yet implement spec `013`'s ordered message contract;
  doing this before the transport decision (Decision 47) would mean
  redoing it once WebSocket lands anyway.

### Outcome

No ticket for `browser_adapter.rs` in the near-term batch. One north-star
ticket filed: "revisit merging `browser_adapter.rs` into the production
HTTP API," explicitly blocked on the WebSocket transport ticket.

## Decision 47: North-Star Runtime Event Transport Is WebSocket + gRPC, Decided Now, Replacing SSE

- **Date**: 2026-08-05
- **Status**: Accepted
- **Governing specs**: new ADR + spec (to be authored), extends `013-browser-runtime-subscription`, `207-event-broker`, `534-ecca-event-products`
- **Related issues**: `#966`, `#967`, `#968`
- **Origin**: `/brainstorm` session, questions 3–6

### Context

`534-ecca-event-products` explicitly scopes out "selecting a broker vendor
or transport topology" — meaning no existing spec picks a wire protocol for
runtime→UI (or cross-boundary runtime→runtime) event delivery. A repo-wide
search found zero WebSocket or gRPC dependencies (no `tonic`/`prost`,
no websocket library) anywhere in the workspace; only SSE exists, and only
on the ungoverned `AppStateEventRecord` channel. The UMA white paper
(§4.4.2) treats WebSocket and gRPC as the common event-management interface,
with SSE framed only as a comparison option, not the target state.

Four sub-questions were resolved in sequence:

1. **Decide now vs. defer**: decide now, rather than leaving it an open
   "TBD" — UMA is unambiguous here and Traverse already has real capability
   contracts declaring publishers/subscribers with nowhere governed for
   those events to reach a live client.
2. **Relationship to the near-term SSE work**: WebSocket replaces SSE
   outright once it ships (not "SSE stays as a fallback") — one transport,
   one code path, matching the "no back-compat tax while pre-production"
   principle established in Decision 48.
3. **Sequencing**: still migrate the existing SSE endpoint onto
   `EventBroker` first (Decision 45's spec), then replace it with WebSocket
   later — the SSE step is small, bounded, and already scoped; it proves
   "`EventBroker` actually reaches a transport" before taking on WebSocket's
   larger lift (server framework, connection lifecycle, auth-over-socket).
4. **gRPC scope**: decide gRPC now too, alongside WebSocket, rather than
   deferring it — per UMA's own guidance (§4.3 Platform Considerations),
   WebSocket and gRPC are peers a client picks between per platform/workload
   (`Starscream`/`OkHttp` for WebSocket, `grpc-swift`/`grpc-java` for gRPC on
   mobile), not a primary/secondary pair, so scoping only one now would
   under-specify the interface UMA actually describes.

### Decision

Author an ADR + spec (Decision 50) committing to WebSocket and gRPC as the
governed runtime-event transport pair, both reading from the same
`EventBroker`/`TraverseEvent` source, replacing the SSE endpoint outright
once shipped. Implementation is staged: SSE-on-`EventBroker` ships first
(Decision 45), WebSocket and gRPC ship after, each retiring their
predecessor rather than running in parallel indefinitely.

### Alternatives Considered

- Defer the transport choice entirely — rejected, UMA and the existing
  capability contracts already assume this exists.
- WebSocket only, gRPC deferred until a native/mobile client exists —
  rejected: the user chose to decide both together rather than
  speculatively defer gRPC.
- Keep SSE as a permanent fallback alongside WebSocket — rejected in favor
  of a clean replacement, consistent with the pre-production fix-fast
  principle (Decision 48).

### Outcome

Two north-star tickets: author the transport ADR+spec, then implement
WebSocket (retiring SSE) and gRPC. Both are downstream of the near-term
SSE-on-`EventBroker` ticket (Decision 45), not blocking it.

## Decision 48: No Backward-Compatibility Tax While Traverse Has No Production Users — General Principle

- **Date**: 2026-08-05
- **Status**: Accepted
- **Governing specs**: none (cross-cutting principle, applies to governance choices generally, not one spec)
- **Related issues**: none yet
- **Origin**: `/brainstorm` session, stated by the user when resolving the capability-ABI compatibility question (Decision 49)

### Context

Deciding whether the north-star capability-side event ABI should be
additive (alongside the existing "declare events in output JSON"
convention) or a breaking replacement of it raised a broader question: how
much should any current Traverse design decision weigh compatibility with
existing capabilities, contracts, or runtime behavior, given there are no
production users or production capabilities yet.

### Decision

**General rule for now**: prefer fixing the architecture correctly over
preserving compatibility with pre-production code, contracts, or
conventions. This is not scoped to the capability ABI alone — it applies to
future governance decisions in this repo until stated otherwise (e.g. once
real external users or production capabilities exist, this default should
be revisited).

### Alternatives Considered

- Default to additive/backward-compatible changes as a standing rule —
  rejected by the user: there is no installed base to protect yet, and
  compatibility shims add permanent surface area for a constraint that
  doesn't currently exist.

### Outcome

Directly determined Decision 49 (breaking ABI replacement) and Decision 47
(WebSocket replaces SSE outright rather than SSE staying a fallback).
Should be cited by name in future brainstorms/ADRs where a
compatibility-vs-correctness fork comes up, until the user says otherwise.

## Decision 49: Capability-Side WASM Host ABI Is a Breaking Replacement of the Output-JSON Event Convention

- **Date**: 2026-08-05
- **Status**: Accepted
- **Governing specs**: new ADR + spec (to be authored), extends `002-capability-contracts`, `003-event-contracts`, `207-event-broker`
- **Related issues**: `#969`, `#970`
- **Origin**: `/brainstorm` session, question 6

### Context

No WASM host-function ABI exists today for a capability to imperatively
publish or subscribe to events (confirmed: zero hits across
`traverse-native-bridge`, `executor/wasm.rs`, `executor/native.rs`).
Capabilities instead declare an `emitted_events` array inside their own JSON
output, which `PlacementRouter` Step 3.5 validates *after* execution
completes against the contract's `emits` list, rejecting undeclared
emissions as a `ContractViolation`. This is the piece furthest from UMA's
model, where a microservice calls the runtime's abstraction layer directly
(e.g. `this.eventDispatcher.dispatch(...)`, §5.1.2.2).

### Decision

The north-star ABI is a **breaking replacement**, not an additive option:
capabilities will be required to call a new host function to emit events
imperatively; the output-JSON declaration convention is deprecated and
removed rather than kept as a second supported path. This follows directly
from Decision 48 (no back-compat tax pre-production) — with no production
capabilities depending on the current convention, there's no cost to
replacing it outright, and a single canonical path avoids permanently
maintaining two ways to do the same thing plus the post-hoc violation-check
complexity (the host function can reject an undeclared event synchronously
at call time instead of after the fact).

### Alternatives Considered

- Additive/optional host function alongside the existing convention —
  rejected per Decision 48; would create permanent dual-path complexity for
  a compatibility constraint that doesn't exist yet.

### Outcome

Two north-star tickets: author the capability-ABI ADR+spec, then implement
the host function (native-bridge + WASM executor + contract validation
changes) and migrate existing capability fixtures/tests off the
output-JSON convention.

## Decision 50: Unify Workflow Event-Driven Edges With `EventBroker`, Superseding `018`'s No-External-Broker Scope Cut

- **Date**: 2026-08-05
- **Status**: Accepted
- **Governing specs**: new ADR + spec (to be authored), extends `018-event-driven-composition`, `207-event-broker`
- **Related issues**: `#971`, `#972`
- **Origin**: `/brainstorm` session, question 7

### Context

Spec `018-event-driven-composition` deliberately scoped workflow
event-driven edges to avoid "external brokers, event-created executions,
direct-capability waiting semantics" — a considered design choice, not a
placeholder. In the current implementation
(`workflows.rs::evaluate_event_driven_edges`), a waiting workflow edge can
only advance from an event extracted from the *same* workflow execution's
own node output — no cross-workflow, cross-process, or durable delivery,
and no connection to `EventBroker` at all. This is a third, separate
event mechanism alongside the broker and the (soon-to-be-replaced) ABI
convention.

### Decision

Unify workflow-edge advancement with the real `EventBroker`, so a waiting
edge can advance from any governed event — including ones from other
workflows, other capabilities, or external publishers — not just events
declared in the same execution's own output. This reverses `018`'s explicit
scope cut, justified by Decision 48 (no back-compat tax pre-production) and
by this being the one place in the runtime where UMA's cross-system
event-driven composition model was structurally impossible under the
current design.

### Alternatives Considered

- Keep `018`'s synchronous, single-execution-scoped model as deliberately
  separate — rejected: while it was a legitimate scope cut at the time
  (avoids delivery/ordering/durability concerns), it's the one remaining
  place event-driven composition can't cross a workflow or process
  boundary, which is core to UMA's model and to what capability authors
  will expect once the ABI (Decision 49) exists.

### Outcome

Two north-star tickets: author the workflow-edge-unification ADR+spec,
then implement cross-workflow/cross-process event-driven edge advancement
via `EventBroker`.

## Decision 51: ADR + Spec Pairing for All Three North-Star Decisions

- **Date**: 2026-08-05
- **Status**: Accepted
- **Governing specs**: n/a (meta-decision about how Decisions 47, 49, and 50 get formalized)
- **Related issues**: none yet
- **Origin**: `/brainstorm` session, question 8

### Context

This repo pairs an ADR with a spec for major technology/pattern pivots
(ADR-0001 + `001-foundation-v0-1`; ADR-0024 + `530-remote-key-value-datastore`;
ADR-0029 + `535-s3-compatible-remote-datastore`; ADR-0033 +
`539-datastore-multiprocess-coordination`). The question was whether all
three north-star decisions (transport, capability ABI, workflow-edge
unification) warrant that pairing, or only the two that introduce genuinely
new technology/patterns (transport, ABI), with workflow-edge unification
covered by a spec alone since it applies an already-ADR'd pattern
(`EventBroker`, governed by `207`/`534`) somewhere it wasn't used yet.

### Decision

All three get an ADR + spec pair: transport (Decision 47), capability ABI
(Decision 49), and workflow-edge unification (Decision 50).

### Alternatives Considered

- ADR+spec for transport and ABI only, spec-alone for workflow-edge
  unification — this was the recommended option (workflow-edge unification
  arguably just extends an existing pattern rather than introducing a new
  one), but the user chose full ADR+spec coverage for all three instead.

### Outcome

Six specs total to author across the near-term and north-star batches (one
near-term SSE spec, three north-star ADR+spec pairs), each producing its
own "author the spec" ticket ahead of its "implement" ticket.

## Decision 52: Amend Specs 096–099 to v1.1.0 to Close Happy/Unhappy-Path Gaps Found Before Implementation

- **Date**: 2026-08-06
- **Status**: Accepted
- **Governing specs**: `096-runtime-event-sse-transport`, `097-websocket-grpc-event-transport`, `098-capability-event-host-abi`, `099-workflow-event-broker-unification` (all amended v1.0.0 -> v1.1.0)
- **Related issues**: `#963`–`#973`
- **Origin**: Pre-implementation audit requested by the user ("make sure we have all the tickets... specs and ADRs defined and approved... identify happy and unhappy paths") before any of the eventing-sequence tickets began implementation.

### Context

A systematic pass through the four specs merged in PR #974, checking each
against a happy/unhappy-path checklist (success, auth failure, malformed
input, dependency-unavailable, concurrency/replay edge cases), found seven
requirement gaps that were genuinely undefined behavior — not just missing
illustrative Acceptance Scenarios for already-stated requirements. Left
unresolved, each implementer would have had to invent the behavior ad hoc:

- `096`: no defined response for a malformed/expired `Last-Event-ID`
  (despite `EventBroker` already having typed `InvalidCursor`/`CursorExpired`
  errors for exactly this), and no defined response for an internal
  `EventBroker` failure during poll.
- `097`: FR-008 only covered failures *before* a stream starts, leaving
  mid-stream broker failure undefined; no reconnect/resume story for a
  dropped WebSocket connection; no bound on malformed/oversized incoming
  client messages.
- `098`: no requirement for memory-bounds validation at the WASM guest/host
  boundary for the new host function, despite the guest supplying the
  payload pointer/length the host reads.
- `099`: FR-007 only covered "event type not registered" — `EventBroker`
  being unreachable at subscription-registration time was a distinct,
  undefined failure mode.

Per `.specify/memory/constitution.md` and `approved-specs.json`
(`"immutable": true`), none of these four specs could be edited in place;
closing the gaps required a formal, versioned amendment, following the
precedent already established in `traverse-framework/registry`'s own
`016-ecca-event-product-adoption` spec (v1.0.0 -> v2.0.0 -> v2.1.0, each
amendment explicitly owner-approved and logged).

### Decision

Amend all four specs to v1.1.0, adding one new FR (two for `097`, none
removed or changed) per gap, each proposing a resolution grounded in a
pattern already established elsewhere in this codebase rather than a novel
design (typed `EventError` variants -> structured HTTP status codes;
`browser_adapter.rs`'s existing bounded-input constants; `traverse-swift-host`'s
existing WASM-boundary bounds-checking discipline). No existing FR text
changed in any of the four specs — this is purely additive.

### Alternatives Considered

- Defer explicitly and let each ticket's implementer propose the exact
  behavior during implementation, reviewed in PR rather than pre-specified —
  this was offered as the deferral option but not chosen; the user chose to
  close the gaps now, before implementation starts.

### Outcome

`specs/governance/approved-specs.json` updated to `"version": "1.1.0"` for
all four spec ids. Each spec file carries an `## Amendment` note (matching
the registry repo's own amendment-note convention) documenting what changed
and why. Tracked for codification in a follow-up PR alongside the DoD
strengthening pass on issues `#963`–`#973`.

## Decision 53: Retire `browser_adapter.rs` Now That Production WebSocket Serves Spec 013's Contract

- **Date**: 2026-08-07
- **Status**: Accepted
- **Governing specs**: `013-browser-runtime-subscription` (unchanged); no spec currently registers `crates/traverse-cli/src/browser_adapter.rs` specifically (see Context)
- **Related issues**: `#973`, `#967`
- **Origin**: `/traverse-ops 973`, escalated to `/brainstorm` per the ticket's own text ("decide retire vs. merge... both were live options... deferred rather than ruled out")

### Context

Issue `#973` was filed as a north-star placeholder by Decision 46, explicitly
gated on the WebSocket transport (Decision 47) shipping first, to revisit
whether `browser_adapter.rs` — a standalone, single-connection, dev-only
`traverse-cli browser-adapter serve` binary replaying one hardcoded canonical
outcome — should be retired or merged into the production HTTP API so spec
`013`'s ordered `subscription_established → state → trace → terminal_result
→ stream_completed` message contract becomes real, `EventBroker`-backed
production behavior.

`#967` merged (PR #979) before this brainstorm started. Inspecting its
implementation (`crates/traverse-cli/src/app_events_websocket.rs`) found
that its scope already included a `browser_subscription` WebSocket mode
alongside `app_events`: `serve_browser_subscription()` calls the same
`traverse_runtime::browser_subscription_messages()` function that generates
spec `013`'s ordered contract, sourced from a real trace/execution record in
the workspace, not a hardcoded outcome. The "merge" side of `#973`'s
retire-vs-merge choice had therefore already been substantially done as a
side effect of `#967`, without a dedicated ticket or spec authored for it.

Separately, checking `specs/governance/approved-specs.json` found that no
approved spec entry governs `crates/traverse-cli/src/browser_adapter.rs`
specifically — `013-browser-runtime-subscription`'s `governs` list only
covers `crates/cogolo-runtime/` and `crates/traverse-runtime/`, not
`crates/traverse-cli/`. The `specs/019-local-browser-adapter-transport/`
folder referenced by `#973`'s own body exists on disk but was never
registered in the approved-specs registry — it never became an immutable
governing spec in the enforced sense. Precedent from
`096-runtime-event-sse-transport` (still `"status": "approved"` in the
registry even though `#967` retired the SSE code path it governs, per
`docs/adr/0034-websocket-grpc-event-transport.md`) confirmed this repo does
not use a "retired" registry status — an approved spec stays as a historical
record even after its governed code is removed. The unregistered `019` spec
folder has no such historical-record status to preserve.

### Decision

Retire `browser_adapter.rs` outright: delete the file, its `main.rs` CLI
wiring (`browser-adapter serve` subcommand, help text), and the unregistered
`specs/019-local-browser-adapter-transport/` folder. Update the ~9
doc references found across `docs/` and `specs/` that mention
`browser-adapter`/`browser_adapter`. No new spec or ADR is required — this
is pure removal of now-duplicate surface area, declared under an existing
broad-covering spec (`097-websocket-grpc-event-transport`, whose `governs`
list already includes `crates/traverse-cli/`) for spec-alignment purposes.

### Alternatives Considered

- Keep it as a lightweight dev tool (no full workspace/app/execution setup
  needed to exercise spec 013's message shape) — considered, but rejected:
  production's `browser_subscription` mode already serves the same contract
  with real data; a second, hardcoded-outcome implementation is now
  duplicate surface area with no clear purpose, and Decision 48 (no
  back-compat tax pre-production) favors removing it over maintaining two
  parallel implementations.
- Verify request_id-selector parity before retiring (the `browser_subscription`
  code path inspected only handles the `execution_id` selector explicitly) —
  raised as a caution during the brainstorm; folded into the retirement PR's
  own validation rather than blocking the decision itself.

  **Amendment (2026-08-07, during execution)**: verification found this was
  a real gap, not a hypothetical one — `SubscribeRequest`/`serve_browser_subscription`
  in `crates/traverse-cli/src/app_events_websocket.rs` have no `request_id`
  field or code path at all, only `execution_id`, while spec `013` FR-001
  requires supporting either selector and `browser_adapter.rs` does support
  `request_id` (tested). Retiring `browser_adapter.rs` as originally decided
  would have left production non-compliant with FR-001. Re-raised to the user
  mid-execution rather than silently expanding scope or silently accepting
  the gap; decided to add `request_id` support to
  `serve_browser_subscription` first (small, mirrors the existing
  `execution_id` path), verify against spec 013, and only then retire —
  folded into the same `#973` PR rather than filed as a separate ticket,
  since it's a small, tightly-scoped addition directly gating the retirement
  this ticket already decided on.
- Leave the unregistered `019-local-browser-adapter-transport` spec folder in
  place as an unofficial historical record — rejected: it was never approved,
  so keeping it alongside deleted code it describes is just confusing dead
  documentation, unlike an actually-approved spec.

### Outcome

`#973` proceeds as a normal `TRAVERSE OPS` execution ticket: claim, branch,
delete `browser_adapter.rs` + CLI wiring + the unregistered spec folder,
update doc references, verify `serve_browser_subscription`'s selector
coverage against spec 013 (request_id XOR execution_id) before/while
removing the fallback tool, open a PR declaring `097-websocket-grpc-event-transport`
as the governing spec, and close `#973` on merge.

## Decision 54: Canonical Capability Create Path Is `capability new` (Option A)

- **Date**: 2026-08-07
- **Status**: Accepted
- **Governing spec**: `100-capability-package-authoring` (new; approval via #989)
- **Related issues**: `#988` (umbrella), `#989` (spec), `#990` (implement), `#991` (docs); adjacent Ready bugs `#986`, `#987`
- **Origin**: E2E capability-authoring probe + `/brainstorm` create-path question

### Context

A developer/LLM persona walkthrough (registry → CLI create → skill path →
inspect/execute) found that Traverse already has a working production package
model (`kind: capability_package` + Host ABI / no-std guest profile), but the
advertised or discoverable create paths do not emit it:

- `traverse-cli component new` (governed by `044` FR-015) creates an empty
  `lib.rs`, empty I/O schemas, draft-oriented contract fields, and a
  non-`capability_package` manifest shape.
- `scripts/scaffold/new-capability.sh` emits stale contract fields and a
  draft/WASI-oriented layout.
- Working knowledge of the ABI-clean guest profile lived primarily in the
  `traverse-app-builder` skill, not in the CLI scaffold.

Adjacent CLI bugs (`capability inspect` advertised but unwired; 
`capability-package execute` hardcoding version and allowlisting demo output)
are implementation gaps under already-approved `017` / `516` and do not need
this decision.

### Decision

**Option A**: Add `traverse-cli capability new <capability-id>` as the
canonical create command. It MUST scaffold a skill-correct
`capability_package` (manifest, authorable contract I/O, no-std-oriented
guest stub, artifacts + sample request, next-step messaging that does not
claim executability early). `component new` and the bash scaffold MUST
redirect or fail toward that command rather than remaining silent success
paths for the pre-Spec-100 empty layout.

No new ADR: this is CLI/scaffold authority, not a new runtime or Host ABI
boundary. Guest constraints remain governed by `091` / `090`.

### Alternatives Considered

- **Option B** — Fix only `component new` in place to emit `capability_package`:
  smaller command surface, but keeps “component” naming while the product
  language and package kind are “capability.”
- **Option C** — Keep both commands forever with different jobs: preserves
  `044` wording literally, but leaves two overlapping scaffolds that LLMs
  and humans keep confusing.

### Outcome

- Spec `100-capability-package-authoring` authored for owner approval (#989).
- Implementation (#990) and docs/skill alignment (#991) blocked on approval.
- `#986` / `#987` remain independently Ready under existing specs.
- Decision 48 (no pre-production backward-compatibility tax) applies: do not
  maintain a long dual-scaffold era.

## Decision 55: New Spec + ADR for `LocalExecutor` Event Emission, Extending `LocalExecutor`'s Trait Signature Rather Than Patching Around It

- **Date**: 2026-08-07
- **Status**: Accepted
- **Governing specs**: [ADR-0037](adr/0037-local-executor-event-emission.md) + `101-local-executor-event-emission` (authored in #995), extends `098-capability-event-host-abi`, `207-event-broker`; touches but does not amend `099-workflow-event-broker-unification` (that spec's boundary explicitly excludes "how a capability emits an event")
- **Related issues**: follows up on `#970` (098's implementation); `#995` (spec, complete), `#996` (implementation, Ready)
- **Origin**: `/brainstorm` session auditing `#970`'s follow-through

### Context

`#970` implemented spec `098`'s `traverse_host::emit_event` WASM ABI and
threaded it through `CapabilityExecutor::execute() -> ExecutorOutput` and
`PlacementRouter` Step 5, but only for the `CapabilityExecutor` trait. A
separate, older trait, `LocalExecutor::execute() -> Result<Value,
LocalExecutionFailure>` (`crates/traverse-runtime/src/lib.rs`), has no
event channel at all and is used by two real production paths:
`BoundLocalExecutor` (bridges a host-provided native `LocalExecutor` into
`Runtime::execute()`'s live path) and `ArtifactRouter` (the `LocalExecutor`
used for workflow-internal node execution in `workflows.rs`, and — since
`traverse-cli`'s `main.rs` constructs `Runtime::new(registry,
ArtifactRouter::new()?)` — the *same* underlying executor as the live path).
`ArtifactRouter` calls `WasmExecutor` internally for WASM capabilities and
already receives real, ABI-validated `ExecutorOutput.emitted_events`, but
discards them (`.map(|output| output.value)`) before returning. Both gaps
were already flagged in-repo as known issues (tests documenting the drop in
`lib.rs` and `tests/placement_router_live_wiring.rs`).

A related, closely coupled gap surfaced during investigation: spec `098`'s
FR-004 required removing the old output-JSON `emitted_events` convention
"once this ABI exists — not kept as a second supported path," but
`workflows.rs`'s `emitted_events(&output: &Value)` JSON-parsing convention
is still the only event-emission mechanism available to native
(non-WASM) capabilities inside workflows, and was therefore never actually
removed. It's also narrower than `EventBroker`-backed emission: a
workflow node's emitted events today only ever satisfy waiting edges
within the *same* workflow execution — they never reach `EventBroker` for
other workflows, capabilities, or external subscribers.

### Decision

Six sub-decisions, worked through in sequence:

1. **Fix shape**: extend `LocalExecutor::execute()`'s return type (e.g. a
   new `LocalExecutionOutput { value: Value, emitted_events:
   Vec<TraverseEvent> }`, mirroring `ExecutorOutput`) rather than a
   narrower fix scoped to `ArtifactRouter` alone. Accepted as a breaking
   change to a public, embedder-facing trait (~12 call sites across
   `traverse-runtime`, `traverse-cli`, `traverse-mcp`) because it's the
   only shape that gives native `LocalExecutor` implementors (host
   closures, `ArtifactRouter`'s native handlers) an actual, structural way
   to emit events at all — a narrower `ArtifactRouter`-only fix would have
   left native capabilities with no channel, and full unification into
   `CapabilityExecutor` was rejected as disproportionate to this bug.
2. **Old JSON convention**: removed outright, migrating `workflows.rs`'s
   node-execution and Pass-1 event-driven edge matching onto the new
   structured `emitted_events` field. This finally satisfies `098`'s
   FR-004 across the full codebase (it previously only covered the
   `executor`/`router` slice), consistent with Decision 48 (no
   back-compat tax pre-production).
3. **External publish**: a workflow node's emitted events now also publish
   to `EventBroker` (in addition to satisfying same-execution waiting
   edges), closing the "workflow events are invisible outside their own
   execution" gap while the same code path is already being touched.
4. **Governance vehicle**: a new ADR + spec, not an amendment to `098`.
   `098`'s capability boundary explicitly scoped itself to
   `executor`/`router`/`traverse-contracts`/`traverse-native-bridge`, and
   this is a materially different mechanism (a trait signature change,
   not a WASM host import) serving a related but distinct purpose —
   matching the Decision 51 precedent of one ADR+spec per distinct
   capability boundary.
5. **Native event validation**: events populated directly by native
   `LocalExecutor` implementors must be validated against the capability
   contract's `emits` list and `service_type == Subscribable` before
   publish, mirroring the WASM ABI's FR-002/FR-003 synchronous validation.
   Without this, native code (unsandboxed, unlike WASM) could emit
   undeclared events straight to `EventBroker`, since the existing
   `PlacementRouter` Step 5 check only gates on `service_type`, not on
   `emits` content.
6. **Failure mode on invalid native event**: an undeclared/invalid native
   event fails the whole capability/node execution (same severity as a
   WASM ABI rejection), even though native validation necessarily happens
   *after* the closure has already returned (it can't be rejected
   mid-call the way the synchronous WASM host function can). Chosen over
   silently dropping the event with a warning, to keep "emitted events are
   always declared" a real guarantee on the native path too, not just WASM.

An architectural consequence of (1) that isn't a preference but a forced
correctness constraint: `ArtifactRouter` must not hold its own
`EventBroker` reference or publish internally, since it is used both
directly by `workflows.rs` (bypassing `PlacementRouter`) and, wrapped in
`BoundLocalExecutor`, by `PlacementRouter` Step 5 for the live
`Runtime::execute()` path. Publishing from within `ArtifactRouter` itself
would double-publish on the live path. The single publish point per path
stays `PlacementRouter` Step 5 (already correct once `BoundLocalExecutor`
threads real `emitted_events` through) for the live path, plus a new,
analogous publish step inside `workflows.rs`'s
`execute_workflow_capability` for the workflow-internal path.

### Alternatives Considered

- Narrow fix scoped to `ArtifactRouter` only (inject `EventBroker`
  directly, no trait signature change) — smaller and non-breaking, but
  leaves native `LocalExecutor` implementors with no event-emission
  channel at all, and would have needed reverting once decision 5's native
  closures could populate real events. Not chosen.
- Collapse `LocalExecutor` into `CapabilityExecutor` entirely — removes
  the dual-trait split at its root, but rewrites `BoundLocalExecutor`,
  `ArtifactRouter`'s trait impl, every workflow test double, and both
  embedders (`traverse-cli`, `traverse-mcp`) simultaneously; disproportionate
  to this bug. Not chosen.
- Keep the old JSON `emitted_events` convention as a documented fallback
  alongside the new structured field — less migration work, but directly
  contradicts `098` FR-004 and Decision 48's no-back-compat-tax principle.
  Not chosen.
- Drop invalid native-emitted events with a warning instead of failing
  execution — avoids punishing an otherwise-successful capability result
  for an unrelated event-declaration bug, but weakens the "emitted events
  are always declared" guarantee to something easy to silently miss. Not
  chosen.

### Outcome

Two tickets, following the repo's spec-then-implement convention: `#995`
authored ADR-0037 + spec `101-local-executor-event-emission` v1.0.0
(extending `098`'s emission model to the `LocalExecutor` surface,
including the `workflows.rs` publish step and native-event
validation/failure semantics) directly in-session, registered `approved`
in `specs/governance/approved-specs.json` per the auto-approval policy
(aligned with this decision log entry); `#996` tracks the implementation —
the trait signature change and all ~12 call-site migrations, the
`workflows.rs` JSON-convention removal and structured-field migration, and
updates to the tests that currently document the gap as expected behavior
(`lib.rs`'s `bound_local_executor_never_publishes_events_through_placement_router`,
`tests/placement_router_live_wiring.rs`'s
`live_native_execution_completes_and_writes_trace_without_publishing_events`).
Both filed on org Project 1 (`#995` In Progress pending PR merge, `#996`
Ready). Spec canonical id renumbered from `100` to `101` during this
session's rebase after discovering `origin/main` had concurrently claimed
`100-capability-package-authoring` (Decision 54, #989).

## Decision 56: Fix the Placeholder Ed25519 Signature in `supply_chain_check.sh` Under Existing Spec 031 — Not a New Decision

- **Date**: 2026-08-07
- **Status**: Accepted
- **Governing spec**: `031-supply-chain-hardening` (already approved; FR-009, SC-001)
- **Related issues**: `#985`
- **Origin**: Discovered while checking `main`'s CI health after the eventing sequence (#963–#973) finished; unrelated to eventing.

### Context

`main`'s `Supply Chain` GitHub Actions workflow was failing on every recent
push (confirmed on 3 consecutive commits) with `"ed25519 signature does not
verify the artifact bytes"`. Root cause:
`scripts/ci/supply_chain_check.sh` has hardcoded an all-zero placeholder
`public_key_hex`/`signature_hex` into the release-artifact manifest since it
was added in #431 — it never actually signed anything.
`crates/traverse-cli/src/supply_chain.rs::verify_signature`'s Ed25519 check
is genuinely cryptographic (confirmed by reading it directly) and correctly
rejects an all-zero signature, so this had silently never worked. It went
unnoticed because this workflow only triggers on `push`/`schedule`/
`workflow_dispatch` (no `pull_request` trigger), so it never blocked a PR
merge.

Before treating this as a design decision, `031-supply-chain-hardening` was
checked directly: FR-009 already requires "Ed25519 keypair as the required
baseline" for artifact signing, and SC-001 already requires
`artifact verify` to return `overall_status: passed` for a valid, *signed*
artifact — both already presuppose a real signature exists to check. This
is a gap between an already-approved spec and the implementation, the same
pattern as Decision 44 (`PlacementRouter` wiring), not a new decision.

### Decision

Add `traverse-cli artifact sign <path>` (the natural counterpart to the
existing `artifact verify`) that signs an artifact with a freshly derived,
single-use Ed25519 keypair, and have `supply_chain_check.sh` call it instead
of hand-writing a placeholder manifest. The signing key is derived
deterministically from the artifact's own checksum and the current time —
not a persistent, publicly trusted release key. This is a deliberate scope
choice, not an oversight: Traverse's only real distribution channel is
`cargo publish` to crates.io (source, not this compiled binary — Decision
43), so no persistent binary-signing key exists anywhere in this repo's
governance to use instead, and provisioning one (a GitHub Actions secret)
is exactly the kind of credential/account action Decision 43 already
established Claude does not perform on the user's behalf. An ephemeral key
fully satisfies what this specific CI self-check needs: proving the
sign/verify round trip is internally consistent, not asserting a publicly
verifiable release signature.

### Alternatives Considered

- Provision a persistent signing key as a GitHub Actions secret for real
  release-artifact signing — rejected for this ticket: no such concept
  exists elsewhere in this repo's governance (the actual release channel is
  source-only via crates.io), and creating the secret is account/credential
  provisioning outside what Claude performs unprompted, matching Decision
  43's precedent exactly.
- Sign with a fixed, hardcoded (but non-zero) keypair committed to the repo
  — rejected: this would look like a real key without being one, inviting
  exactly the false confidence the original all-zero placeholder created,
  just with extra steps.

### Outcome

`crates/traverse-cli/src/supply_chain.rs` gains `sign_artifact`,
`ArtifactSigningReport`, and `SigningError`; `main.rs` gains the
`artifact sign` subcommand mirroring `artifact verify`. Verified locally
end-to-end: `bash scripts/ci/supply_chain_check.sh` now reports
`overall_status: passed` with zero warnings. Tracked as `#985`, no new spec
or ADR required.

## Decision 57: Contract Surface Coverage — Schema ⊆ Use Cases ⊆ Smoke

- **Date**: 2026-08-08
- **Status**: Accepted (honesty path); Spec 102 remains Draft until owner registers it
- **Governing spec**: `102-contract-surface-coverage` (Draft), ADR-0038 (Proposed)
- **Related issues**: `#1014`, `#1015`, `#1016`; registry `#192`, `#193`
- **Origin**: Post-ship review of `core.process-comment@1.0.0` overclaim (enum/description beyond use-case matrix).

### Context

Publish and registry validation treated `description` and broad `action` enums as unchecked claims. Only use cases and package smoke were executable promises, so an overclaiming contract could merge.

### Decision

1. **Process**: Govern discriminator-enum coverage (start with `action`) via Spec 102 / ADR-0038; implement publish dry-run failure and a registry mirror check after approval.
2. **Capability honesty**: Ship `core.process-comment@1.0.1` that narrows the declared surface to the tested 8-case matrix; deprecate `1.0.0` with an explicit overclaim reason. Full resolve/pin/markup/allow-list mention work is a separate product enhancement, not required to restore honesty.

### Alternatives Considered

- Block all capability publishes until NLP description linting exists — rejected (too heavy; use cases are the right boundary).
- Implement the entire original marketing surface before any honesty bump — rejected as the default; narrowing is a valid fix.

### Outcome

Tickets filed on Project 1 (`#1014`–`#1016`) and Project 3 (`#192`–`#193`). Spec/ADR drafted. Honesty bump proceeds under existing `516` while Spec 102 awaits approval.

