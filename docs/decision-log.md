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
  ADR-0049's transport-port precedent, renumbered from ADR-0029 on
  2026-08-24) with a no-op default. `traverse-cli`
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
- **Status**: Accepted; Spec 102 Approved and registered 2026-08-08
- **Governing spec**: `102-contract-surface-coverage`, ADR-0038
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


## Decision 58: Full Capability Surface Coverage via Use Cases (Not Minimum Counts)

- **Date**: 2026-08-10
- **Status**: Accepted; Spec 102 v1.1.0 Approved 2026-08-10
- **Governing spec**: `102-contract-surface-coverage` (v1.1.0), ADR-0038 (amended), registry `001` FR-011
- **Related issues**: traverse `#1040`; registry `#215`
- **Origin**: Post-Loop-batch audit — most registry `core.*` contracts had `use_cases` stripped by publish; owner directed that use cases must cover the entire capability, as a non-negotiable gate.

### Context

Decision 57 / Spec 102 v1.0.0 gated only `inputs.schema.properties.action.enum`. Loop capability publish validated use cases from raw JSON, then serialized `CapabilityContract` (which has no `use_cases` field), so registry copies lost them. Registry CI explicitly allowed missing use cases (`test_contract_without_use_cases_is_not_flagged`). Local examples often had use cases and smokes, but the catalog of record did not.

### Decision

1. **Coverage target**: the declared schema surface — every input schema string enum value; every `inputs.schema.required` property at least once; every `outputs.schema.properties.reason_code` / `status` enum value. Not a minimum use-case count. No cartesian product of required fields.
2. **Enums for checkable outcomes**: `reason_code` / `status` MUST be schema enums when authors need those outcomes covered; free-string fields are not coverage-checkable.
3. **Smoke linkage**: every `use_cases[]` entry MUST have a matching executable smoke fixture that asserts its `reason_code` / key outputs.
4. **Enforcement**: fail closed in both `capability publish` / `--dry-run` and registry CI for newly ADDED or CHANGED contracts. Publish MUST preserve `use_cases` (and author evidence) into the registry-bound JSON.
5. **History**: do not edit immutable stripped versions in place; honesty patch-bump them under FR-010.
6. **Governance vehicle**: amend Spec 102 (and registry `001` FR-011 from MAY→MUST for new/changed contracts); do not create a parallel coverage law.

### Alternatives Considered

- Minimum happy+unhappy counts only — rejected; owner clarified coverage of the whole capability matters, not N.
- Cartesian required-field matrices — rejected as an impractical publish gate.
- NLP description coverage as a blocking gate — deferred; Known limitations remain the honesty path for prose.
- Forward-only gate without republishing stripped caps — rejected; catalog would stay dishonest.
- CLI-only or registry-only enforcement — rejected; both are required.

### Outcome

- Spec 102 drafted at v1.1.0 (Draft) and ADR-0038 amended.
- Implementation tracked by traverse `#1040` and registry `#215`.
- Honesty patch-bumps for stripped Loop caps follow once the gate lands.


## Decision 59: Declarative Workflow Planner (P0) — Data-Dependency-Only, Fail-Closed, Bounded

- **Date**: 2026-08-22
- **Status**: Accepted; Spec 113 Approved 2026-08-22
- **Governing spec**: `113-declarative-workflow-planning` (P0 of `108-governed-runtime-workflow-composition`), ADR-0043
- **Related issues**: `#1098`; registry `#304`, `#305`; related `#865`, `#1089`–`#1094`
- **Origin**: Investigation into why a live demo at traverse-framework.com/discover.html only simulates workflow composition instead of doing it for real (traverse `#1097`, `#1098`, `#1100`; registry `#304`, `#305`), which surfaced that `109`'s own scope explicitly excluded "planner implementation" — the actual remaining gap was narrower and different than first filed.

### Context

`109-runtime-workflow-proposals` (P1) governs how a *submitted* proposal is
validated, authorized, and executed, and is Approved and shipping. It
explicitly named "planner implementation" as out of scope. Nothing in this
repo generates the content of a proposal from declared capability metadata
— composing a multi-step workflow has only ever been possible by hand
(`workflow.json`) or via the explicit `WorkflowBuilder` API (`#309`/`#367`),
where the caller already knows and names the exact chain.

### Decision

Ship a first-party, deterministic planner in `traverse-runtime`, governed as
phase P0 of the existing `108` north star (not a standalone spec, since
`109` already named this exact boundary):

1. Accepts a structured target only — no natural-language goal
   interpretation, no hosted-model dependency.
2. Plans only from capabilities with real, declared `consumes`/`emits`
   linkage; no namespace/verb-name fallback when that data is missing.
3. Never auto-resolves ambiguity between multiple viable capabilities —
   enumerates every complete candidate plan and hands the choice to the
   caller/reviewer.
4. Bounds search to a small, fixed, non-configurable limit in v1 (5
   candidate plans, 8 nodes deep).
5. Proposes best-effort field-level JSON-path mappings per edge, always
   flagged `mapping_unconfirmed` until a human/reviewer clears it.
6. Is exposed as a new public MCP tool alongside the existing P1 tools, and
   produces plans structurally submittable to the P1 surface as-is once
   reviewed — the planner itself never submits or executes anything.

### Alternatives Considered

- Namespace/verb-name heuristic fallback when `consumes`/`emits` is empty
  (what the discover.html demo already does client-side) — rejected for
  the governed planner: mixing a real, data-grounded candidate with a
  guessed one on the same proposal surface undermines exactly the honesty
  line this org holds elsewhere (registry decision-log entry 61).
- Auto-pick via a scoring heuristic (coverage %, version recency) —
  rejected; a silent automatic pick is the unreviewed runtime decision
  `109`'s explicit-mapping requirement exists to prevent.
- Standalone new spec instead of a phase of `108` — rejected; `109`
  explicitly deferred this exact scope to a later phase, and a parallel
  spec covering the same feature invites drift (cf. Decision 58's "do not
  create a parallel coverage law").
- Accept and interpret natural-language goals in-runtime — rejected;
  reintroduces the hosted-model dependency `109` FR-001 and ADR-0050
  (renumbered from ADR-0041 on 2026-08-24) explicitly excluded.

### Outcome

Spec `113` drafted and Approved same-day; ADR-0043 recorded. `traverse#1098`
retitled and rescoped to match (see issue history for the two prior
corrections). Real unblock condition made explicit: this planner will
return almost no candidates until registry `#305` backfills real
`consumes`/`emits` data — that dependency is recorded as a formal
`blocked by` relationship on `#1098`, not just prose.

## Decision 61: Break the #1098 / Registry #305 Circular Wait by Reopening #305, Scoped

- **Date**: 2026-08-23
- **Status**: Accepted
- **Governing spec**: `113-declarative-workflow-planning` (no change — this decision only acts on an already-approved requirement); related registry `100-`series namespace/discovery umbrella
- **Related issues**: `traverse#1098`; registry `#305`, `#302`
- **Origin**: User asked which registry ticket was actually blocking traverse work, surfaced from the registry repo's own Project board (which showed `#103`/`#302` as Blocked, neither of which is the real blocker).

### Context

Registry `#305` documented the empirical gap Decision 59 already leaned
on: 116/116 published capability versions have empty `emits`, 0/116 have
non-empty `consumes`. It was closed as `COMPLETED`, but its own closing
comment says the investigation-only close was deliberate: *"the
traverse-framework/traverse runtime team is evaluating this gap
themselves... before this repo backfills."* That evaluation is exactly
what Decision 59 / spec `113` FR-002 did same-day ("plans only from
capabilities with real, declared consumes/emits linkage; no
namespace/verb-name fallback") — but nothing had gone back to `#305` to
say so, leaving both sides waiting on each other. Separately, registry
`#302` documents that 18 of the 116 capabilities (`owner.team:
"callweave"`) have no `capability-src/` in-repo at all, only a compiled
`.wasm` release artifact — those can't be backfilled without external
contribution regardless of `#305`'s outcome.

### Decision

1. **Reopen `#305`** rather than filing a fresh issue or only commenting on
   the closed one — keeps the existing empirical data (116/116 measured)
   and investigation history in one thread, with a new comment stating
   spec `113` is the runtime-side decision `#305` was waiting on.
2. **Scope the reopened ask to the 98 capabilities with real
   `capability-src/` in this repo**, explicitly excluding the 18
   Callweave-owned, sourceless capabilities from `#302` — asking for all
   116 at once would make the ticket blocked on an unrelated external-
   contribution issue before it could ever fully close.
3. **File and stop** — this traverse-repo session has `gh` API access to
   `traverse-framework/registry` for issue/comment/label operations but no
   cloned worktree there, so it reopens and scopes the ask but does not
   attempt the backfill itself.
4. **Comment on `traverse#1098` only** (not umbrella `#1099`) linking the
   reopened `#305` and its scope, so anyone reading `#1098` later sees
   exactly what unblocking it now depends on, without taking on upkeep of
   a second, higher-level umbrella thread that tends to go stale regardless.

### Alternatives Considered

- File a brand-new registry issue instead of reopening `#305` — rejected;
  fragments the empirical 116/116 measurement and prior triage history
  across two issues for no real benefit.
- Ask for a full 116-capability backfill in one pass — rejected; couples
  a resolvable request to `#302`'s unrelated external-contribution
  dependency, which could stall `#305` indefinitely.
- Attempt the backfill directly from this session — rejected; out of
  this worktree's actual reach (registry access here is `gh`-API-only,
  no local clone to edit contract files in).
- Update umbrella `#1099` too — deferred, not rejected; `#1098`'s own
  comment is the operative record, and `#1099` can be refreshed in a
  dedicated backlog-gardening pass instead of piecemeal per sub-issue.

### Outcome

`traverse-framework/registry#305` reopened with a comment linking spec
`113`'s FR-002 and scoping the ask to the 98 in-source capabilities (18
Callweave-owned ones deferred to `#302`). `traverse#1098` commented with
the same scoping so its `blocked by #305` relationship (recorded in
Decision 59) now points at a live, correctly-scoped ticket instead of a
closed one.

## Decision 60: Single-Capability Browser Execution Routes Through `execute_entrypoint`, Not an Extended BundleEmbedder

- **Date**: 2026-08-22
- **Status**: Accepted
- **Governing spec**: `023-browser-hosted-mcp-consumer-model` (existing scope confirmed sufficient); ADR-0044; related `006-runtime-request-execution`, `010-runtime-state-machine`, `068-public-platform-embedder-packages`
- **Related issues**: `#1100`; related `#1097`, `#1098`, `#865`
- **Origin**: Same discover.html investigation as Decision 59. `#1100` proposed extending `BundleEmbedder` (spec `068`) to construct a `manifest.json`/FNV-1a digest client-side so a browser could run an arbitrary, live-fetched registry capability directly — surfaced while approving specs/ADRs for the remaining `needs-spec` tickets from that investigation.

### Context

Real capability execution requires a `manifest.json` (`agent_package`,
FNV-1a `expected_digest`) and a `runtime-request.json`, with a digest match
enforced before anything runs. `#1100` found that a browser fetching a
capability straight from `catalog.json` has neither, and proposed teaching
`BundleEmbedder` (or a new browser-side helper) to build both client-side.
`traverse-mcp`'s existing `execute_entrypoint` tool already performs this
exact resolution server-side and returns a public trace summary — the
ceremony `#1100` was proposing to duplicate is already solved once.

### Decision

Do not extend `BundleEmbedder` to build manifests or digests client-side.
Single-capability execution from a browser-hosted client goes through the
existing `execute_entrypoint` MCP tool, over whatever non-stdio transport
spec `023` already defines for browser-hosted consumers (FR-006) — the
manifest/digest/binary-resolution ceremony stays server-side. `BundleEmbedder`
keeps its narrower, existing job: executing a pre-vetted application bundle
(spec `044`) shipped with the app itself, not arbitrary live registry
content chosen at runtime. No new governing spec is required: spec `023`
already defines browser-hosted transport access in general terms, with no
tool-level allowlist excluding `execute_entrypoint`. What remains is
implementation and validation — confirming a browser-hosted client can
actually reach `execute_entrypoint` over spec `023`'s transport and that
the trace-summary response is sufficient for a real consumer.

### Alternatives Considered

- Client-side manifest/digest construction in `BundleEmbedder` — rejected;
  duplicates server-enforced ceremony in the browser and needs a registry
  digest field that doesn't exist today.
- Registry publishes a ready-made `manifest.json` per version, keeping
  `BundleEmbedder`'s existing flow — deferred, not rejected; a registry-side
  change with its own review, only worth it if `execute_entrypoint` proves
  insufficient.
- A new spec formally governing browser-hosted `execute_entrypoint` access
  — rejected; spec `023` already covers this generically, and a parallel
  spec re-covering the same surface risks the drift Decision 58 warned
  against.

### Outcome

ADR-0044 recorded. `traverse#1100` rescoped from an execution-ceremony
implementation gap to an integration/validation ticket: confirm the
already-approved browser-hosted MCP execute path works end-to-end; `needs-spec`
removed since no new governing document is required.

## Decision 62: Declarative Workflow Planner Chains on Schema-Shape Compatibility, Not `consumes`/`emits`

- **Date**: 2026-08-23
- **Status**: Accepted; Spec `113` amended to v0.2.0, ADR-0043 amended, both approved same-day
- **Governing spec**: `113-declarative-workflow-planning` (v0.2.0); ADR-0043 (amended)
- **Related issues**: `traverse#1098`; registry `#305`
- **Origin**: A registry-ops session working registry `#305` (Decision 61's reopened, scoped backfill ask) found that registry's own already-governed FR-020 capability inventory directly contradicts the premise that a real `consumes`/`emits` backfill exists to do, and raised it as a live, owner-participated brainstorm spanning both repos rather than either fabricating the backfill or leaving `#1098` blocked indefinitely.

### Context

Decision 59 / spec `113` FR-002 required the planner to chain on declared
`consumes`/`emits` (`EventReference`) linkage. Decision 61 reopened
registry `#305` to get that data populated for the 98 in-source
capabilities. Before attempting the backfill, registry's own
`contracts/governance/ecca-capability-inventory.json` (registry decision
57, CI-enforced) was checked directly rather than assumed: it classifies
45 of registry's 46 unique published capability IDs as `no-event-required`
— reviewed, merged content stating they are synchronous, direct-return
capabilities with no asynchronous domain fact to broadcast. This holds even
for the five capabilities in two already-published, human-reviewed
`workflows/*.json` chains (`doc-approval.analyze` -> `doc-approval.recommend`;
`traverse-starter.validate` -> `.process` -> `.summarize`) — real,
structurally-composable data dependencies that are nonetheless not
asynchronous events. Registry's own `graph.rs` composition-graph builder
confirmed this isn't just a labeling question: a declared `emits`/`consumes`
`EventReference` only produces a graph edge when it resolves against a real,
registered `events/**/product.json`; an unbacked reference is functionally
inert there, not a working shortcut.

### Decision

Change the planner's chain-discovery signal (spec `113` FR-002/FR-003) from
declared `consumes`/`emits` linkage to structural `inputs.schema`/
`outputs.schema` compatibility: a candidate producer's `outputs.schema`
structurally satisfies a candidate consumer's `inputs.schema` when every
property named in the consumer's `inputs.schema.required` exists in the
producer's `outputs.schema.properties` with a matching JSON type. This is
not new computation — the planner already derives this exact relationship
for its FR-005 field-mapping step. A capability that separately declares
real `consumes`/`emits` linkage to a governed event may still be selected
on that basis too; the two signals are independent, not a replacement of
one by the other. `consumes`/`emits` keeps meaning what it means everywhere
else in the org (a real, governed, decoupled-subscriber event); nothing
about registry's FR-020 classification needed to change.

False-positive risk (two unrelated capabilities whose schemas happen to
overlap, e.g. both taking `{id: string}`) is accepted rather than solved
here: FR-005/FR-006's `mapping_unconfirmed` flag and mandatory human review
before any submission already gate exactly this risk, and nothing a planner
proposes ever executes unreviewed.

Both the spec and ADR amendments are approved on creation, per this org's
existing precedent that a decision tracing directly to a live,
owner-participated brainstorm — not an agent-invented draft — already has
its sign-off.

### Alternatives Considered

- Loosen registry's FR-020 inventory criterion instead, keep `consumes`/
  `emits` as the sole signal — rejected; would require ~26 capabilities to
  carry full governed-event ceremony (privacy field classification,
  retention policy, CloudEvents mapping, spec `534`'s full descriptor) for
  relationships that were never really asynchronous events, and blurs the
  event-vs-workflow-composition distinction registry's decision 57
  deliberately drew.
- Backfill `consumes`/`emits` anyway, treating a bare `EventReference` with
  no matching `events/**/product.json` as sufficient — rejected; produces
  zero graph edges in registry's own tooling (functionally inert) and
  repeats the overclaiming pattern registry decision-log entries 61/64
  exist to prevent.
- Exact full-schema match instead of required-property overlap — rejected;
  independently-authored capabilities' schemas are rarely exactly equal or
  a strict superset even when genuinely composable, so this would find very
  few real chains.
- A new named/tagged data-shape identifier capabilities opt into (e.g. an
  `x-data-shape` tag) — deferred, not rejected; avoids both false positives
  and event ceremony, but is itself a new schema convention needing its own
  spec and per-capability adoption. Worth reconsidering if structural
  matching's false-positive rate proves too high in practice.
- Do nothing, accept the planner finds almost no candidates until genuinely
  event-driven capabilities grow organically — rejected; leaves `#1098`
  effectively blocked indefinitely and doesn't address why registry `#305`
  was reopened in the first place.

### Outcome

Spec `113` amended to v0.2.0 (FR-002/FR-003, Purpose, and Acceptance
scenarios 1-3 updated; no other requirement changed). ADR-0043 amended in
place (status line notes the amendment; Decision/Consequences/Alternatives
rewritten to the new signal). Registry `#305` closed as resolved-by-redirect
rather than by backfill (registry decision-log entry 68, cross-referencing
this entry). `traverse#1098`'s `blocked by registry#305` relationship is
removed — it is now unblocked by this spec amendment landing, not by any
registry-side work.

## Decision 63: Approve ADR-0045 and Spec 527 (Host-Owned Durable Trace and Audit Persistence)

- **Date**: 2026-08-24
- **Status**: Accepted
- **Governing spec**: `527-durable-trace-productization` (newly approved), ADR-0045 (Accepted)
- **Related issues**: `#1093` (partial unblock — see Outcome), `#847` (spec origin)
- **Origin**: A backlog-validity sweep across traverse + registry (this session) confirmed `#1093`'s stated P1/P2 half of its blocker is resolved (both approved and implemented this session) but its "approved durable host-owned state/trace substrate" half was still real: ADR-0045 and its governing draft spec `527-durable-trace-productization` were sitting at `Proposed`/`Draft`, dated 2026-07-30, with no open questions left unresolved in either document. Owner explicitly requested approval.

### Context

Spec `527` (authored from issue `#847`) defines the host-authorized durable
trace-journal policy for production embeds: safe evidence persists across
restart, is retained within deterministic limits (30 days or 10,000 records
per workspace, oldest-first), and is exported only as explicitly redacted
evidence. It is additive to the already-approved `079-durable-trace-journal`,
scoped narrowly to `crates/traverse-runtime/src/trace_journal.rs` and a new
export schema, and explicitly excludes encryption/key lifecycle, remote
sync, and DataStore ownership changes. ADR-0045 records the same decision at
the architecture level: production hosts, not Traverse, own the journal
root, authorization, keys, tenancy, retention, and deletion authority.

### Decision

Approve both documents as written, no changes: ADR-0045 status
`Proposed` -> `Accepted`; spec `527` status `Draft` -> `Approved` and
registered in `specs/governance/approved-specs.json` (version `1.0.0`,
`immutable: true`, governs the four paths its own "Compatibility and
Governed Files" section already named). Neither document needed a
`/brainstorm` pass first — both were already complete, self-consistent, and
narrowly scoped when written; the only missing step was the owner's
sign-off itself, which is not something an agent grants on its own
judgment regardless of how well-formed a draft looks.

### Alternatives Considered

- Defer approval pending a fresh `/brainstorm` review — rejected; neither
  document had an open question, an unresolved alternative, or a scope gap
  in its own text, and the owner asked directly for approval, not review.
- Approve only the ADR, leave spec `527` in Draft — rejected; the ADR
  explicitly names `527` as its "governing draft" and states "this ADR and
  its draft require maintainer approval before implementation" as one
  combined gate. Approving one without the other leaves nothing
  independently implementable.

### Outcome

ADR-0045 and spec `527-durable-trace-productization` are both Approved.
This resolves the trace half of `#1093`'s stated blocker
("approved durable host-owned state/trace substrate"); the checkpoint/state
half (P3's own "authenticated secret-free checkpoint binds all governing
snapshots" requirement) is a distinct concern from a trace journal and is
not addressed by this approval — `#1093` is not yet fully unblocked, and no
existing Proposed ADR was confirmed to cover checkpoint/state
specifically. `#1093` left `Blocked` pending that remaining gap.

**Renumbering note**: this ADR was originally numbered 0027, which collided
with the unrelated, already-Accepted `docs/adr/0027-datastore-synchronization.md`
("Keep DataStore Synchronization Host-Owned and Deterministic", spec
`089-datastore-synchronization`). Renumbered to 0045 (the next free number)
as part of approving it, rather than leaving a second "ADR-0027" in the
tree. The same live sweep found five more pre-existing duplicate ADR
numbers (0011, 0018, 0024, 0029, 0041), each shared by two unrelated,
already-Accepted decisions — those are deliberately left as-is here (both
sides are established and possibly cross-referenced elsewhere; renumbering
them safely needs its own pass, not a rider on this approval) and filed as
a separate follow-up ticket.

## Decision 64: Approve ADR-0025 and Spec 531 (DataStore Synchronization Protocol)

- **Date**: 2026-08-24
- **Status**: Superseded same day — see Amendment below
- **Governing spec**: `531-datastore-synchronization-protocol` (approved, then superseded and removed from the registry same day), ADR-0025 (Accepted, then marked Superseded)
- **Related issues**: `#877` (spec origin), `#883` (closed, completed — see Amendment)
- **Origin**: Same backlog-validity sweep as Decision 63, extended per the owner's follow-up request to review the two other Proposed ADRs found (`/brainstorm`, 2026-08-24).

**Amendment (2026-08-24, same day)**: this approval was made without checking
whether the DataStore synchronization protocol question had already been
answered elsewhere. It had — issue `#883` (closed, completed) shows the exact
same design (mutation envelopes, Lamport-clock-then-writer-ID conflict
resolution, opaque per-peer cursors, in-process test-double transport) already
shipped under `089-datastore-synchronization` / the pre-existing
`docs/adr/0027-datastore-synchronization.md` (unrelated to the ADR-0027 that
was renumbered to ADR-0045 in Decision 63 — this is the other, already-Accepted
ADR-0027). ADR-0025 and spec `531` are now marked **Superseded by
ADR-0027/spec 089** rather than Accepted/Approved, and spec `531`'s entry was
removed from `specs/governance/approved-specs.json`, consistent with how spec
`532` was handled in Decision 65's amendment. Found via the same due-diligence
check applied retroactively after the 532/093 conflict surfaced — caught and
corrected within the same session, before any implementation could reference
the redundant protocol.

### Context

Spec `531` (from issue `#877`) defines a provider-neutral protocol for an
explicit host-requested synchronization attempt between two DataStore
replicas: deterministic Lamport-clock-then-writer-ID conflict resolution,
idempotent replay, interruption and integrity-failure handling, and
secret-free evidence — extending specs `518`/`519` without choosing a
transport or provider. ADR-0025 records the same decision architecturally:
synchronization is explicit and host-requested, never a background runtime
service, with v1 scoped to a local-peer/test-double transport only.

### Decision

Approve both as written, no changes: ADR-0025 status `Proposed` ->
`Accepted`; spec `531` registered in `specs/governance/approved-specs.json`
(version `1.0.0`, `immutable: true`). Both documents were complete and
self-consistent when drafted (2026-07-29); the only missing step was
owner sign-off, given directly this session. The spec's own "Adding this
draft to the approved registry without maintainer approval" out-of-scope
line is removed as now-satisfied rather than left as a stale guardrail.

### Alternatives Considered

Same as Decision 63: a fresh `/brainstorm` review was considered and
rejected as unnecessary (no open question in either document), and holding
for further owner review was offered but not chosen.

### Outcome

Superseded same day (see Amendment above). ADR-0025 and spec `531` are marked
Superseded by ADR-0027/spec `089-datastore-synchronization` and kept only as
historical record of a design that was independently re-derived and then found
redundant; spec `531` is removed from `specs/governance/approved-specs.json`.
No open ticket depends on this decision — `#883` was already closed (completed)
against the pre-existing spec, before this approval happened.

## Decision 65: Approve ADR-0026 and Spec 532 (Multi-Process DataStore Coordination)

- **Date**: 2026-08-24
- **Status**: Superseded same day — see Amendment below
- **Governing spec**: `532-multiprocess-datastore-coordination` (approved, then superseded and removed from the registry same day), ADR-0026 (Accepted, then marked Superseded)
- **Related issues**: `#878` (spec origin), `#879` (closed 2026-08-05 — see Amendment)
- **Origin**: Same sweep as Decision 64.

**Amendment (2026-08-24, same day)**: this approval was made without checking
whether the multi-process coordination question had already been answered
elsewhere. It had — issue `#879`'s own comment history shows the team
explicitly pivoted away from this plain-advisory-lock model on 2026-08-05 to
a host-owned-coordinator-with-lease-fencing model
(`093-datastore-multiprocess-coordination` / ADR-0033), which is what
actually shipped; `#879` closed against that spec, not this one. ADR-0026
and spec `532` are now marked **Superseded by ADR-0033/spec 093** rather
than Accepted/Approved, and spec `532`'s entry was removed from
`specs/governance/approved-specs.json` (a spec registry only ever lists
currently-active approved specs; a superseded one is dropped from it, not
given a different status value). Caught and corrected within the same
session, before any implementation could reference the wrong model.

### Context

Spec `532` (from issue `#878`) defines the coordination-model boundary for
a host that deliberately permits multiple processes to access one
DataStore root: one OS-backed exclusive advisory lock per root, contenders
receive `store_busy` and retry only on host instruction, crash recovery
relies solely on OS lock release (no leases, heartbeats, fencing, or
coordinator daemons), and unsupported filesystems fail closed. Single-
process exclusive ownership remains the default; a host must explicitly
opt in. ADR-0026 records the same decision architecturally.

### Decision

Approve both as written, no changes: ADR-0026 status `Proposed` ->
`Accepted`; spec `532` registered in `specs/governance/approved-specs.json`
(version `1.0.0`, `immutable: true`). Same rationale as Decision 64 — both
documents were complete since 2026-07-29, only owner sign-off was missing.
The same now-satisfied "without maintainer approval" out-of-scope line is
removed. Note spec `532`'s own FR-010 states implementation itself still
requires the separately-approved ticket `#879` — this decision approves
the architecture/spec, not a claim to implement it.

### Alternatives Considered

Same as Decision 64.

### Outcome

Superseded same day (see Amendment above). ADR-0026 and spec `532` are
marked Superseded by ADR-0033/spec `093-datastore-multiprocess-coordination`
and kept only as historical record of a design that was considered and
abandoned; spec `532` is removed from `specs/governance/approved-specs.json`.
No open ticket depends on this decision — `#879` was already closed
(2026-08-05) against the superseding spec, before this approval happened.

## Decision 66: Host-Owned Artifact Preparation as a Separate, Cross-Referenced Manifest (Spec 120)

- **Date**: 2026-08-26
- **Status**: Accepted; Spec 120 Approved 2026-08-26
- **Governing spec**: `120-host-owned-artifact-preparation`; related `115-browser-verified-entrypoint-execution`, `118-host-supplied-serve-registry-state`; ADR-0051
- **Related issues**: `#1153`
- **Origin**: `#1153` (found while investigating a real, publicly-hosted `serve` for traverse-framework.com/discover.html — same lineage as `#1097`-`#1100`, `#1105`, specs 113-119) proposed fixing `serve`'s fabricated `bundled://` artifact locations by having `serve` fetch and verify the real artifact itself. A review comment on the ticket established that violates spec 115 FR-004 and spec 118 FR-004/FR-006 (`serve` MUST NOT fetch/sync/mutate during startup or execution), moving the ticket to `needs-spec`/Blocked and requesting this brainstorm.

### Context

`serve --registry-state` (spec 118) registers capabilities from a
registry-bundle manifest whose entries carry only a `contract.json` path —
never a real WASM location. `build_capability_artifact` therefore always
synthesizes `bundled://{id}/{version}/module.wasm`, which `ArtifactRouter`
can never resolve, so no capability loaded this way can actually execute.
Fixing this by having `serve` fetch the artifact itself was ruled out as a
governing-spec violation before this brainstorm started.

### Decision

Real artifact preparation becomes a separate, host-run step, governed by a
new spec (`120`) rather than a modification to the already-Approved,
immutable spec 118:

1. **Manifest shape**: a new, separate `artifact-state.json` (new spec),
   not new fields on spec 118's own manifest.
2. **Discovery**: a new, independent `serve --artifact-state <path>` flag —
   never embedded in spec 118's schema, never a default/conventional
   location.
3. **Materialized-binary location**: an explicit per-entry `path` field in
   `artifact-state.json`, not a directory convention.
4. **Who prepares it**: a new, first-party `traverse-cli registry
   materialize` subcommand — not an external host script — since digest
   verification is a real security boundary, not just metadata reshaping.
5. **Startup verification**: `serve` re-verifies every on-disk artifact's
   digest against the declared digest before listening; it does not trust
   `materialize`'s prior verification alone.
6. **Failure semantics**: whole-manifest fail-closed — any missing or
   mismatched artifact refuses the entire `serve` startup, never a
   silently reduced subset.
7. **Scope**: capabilities only; workflows carry no artifact of their own.

### Alternatives Considered

- Extend spec 118's manifest schema directly — rejected; spec 118 is
  Approved/immutable, and this org's own precedent (Decision 59/ADR-0043
  adding a new phase to `108` rather than modifying `109`) favors a new
  governing artifact over amending an approved one. Spec 118 FR-002's
  strict unknown-field rejection means even one additive field would
  require a formal successor version regardless.
- Fixed-name sibling file discovered by convention, no new flag —
  rejected; spec 118's own Ownership section already forbids `serve`
  inventing a default state location.
- Keep artifact fetch+verification an external, host-authored script
  (matching how the registry-state manifest generator itself stayed
  external) — rejected; unlike manifest generation, digest verification is
  the actual security boundary stopping a tampered/wrong binary from
  executing, and per-host reimplementation risks a subtly wrong
  verification nobody audits.
- Trust `materialize`'s verification without startup re-verification —
  rejected; no protection against anything mutating the artifacts
  directory between `materialize` running and `serve` starting.

### Outcome

Spec `120` drafted and Approved same-day; ADR-0051 recorded. `#1153` can
be rescoped from a design question to "implement `registry materialize`
and `serve --artifact-state` per spec 120." Hosting a real, live `serve`
instance for discover.html still needs someone to actually run
`materialize` and operate the resulting artifacts directory on an ongoing
basis — an operational decision this spec deliberately leaves open.

## Decision 67: Rescope #1235 In Place to Implementation of Spec 129 — Spec Work Is Done

- **Date**: 2026-09-06
- **Status**: Accepted; no spec or ADR change (Spec `129-governed-workflow-reliability` and ADR-0058 already approved via PR #1245)
- **Governing spec**: `129-governed-workflow-reliability`; ADR-0058; related `109-runtime-workflow-proposals`, `111-durable-dynamic-orchestration`
- **Related issues**: `#1235`; related `#1239` (spec ticket the approving PR #1245 was filed against)
- **Origin**: `/brainstorm` on `#1235`. Four hours after Spec 129 + ADR-0058 were approved, the issue was still OPEN, still in project status **Ready**, still labeled `needs-spec` with every DoD box unchecked. PR #1245's body said "Unblocks #1235" but carried no closing reference, so nothing had actually moved the ticket. Owner-participated brainstorm; Enrico deferred each sub-call to the recommendation.

### Context

`#1235` is a spec ticket: its stated DoD is a governing spec plus an ADR, with
"no runtime implementation starts until the spec and ADR are approved." Both
now exist and are approved — Spec `129` (immutable, in `approved-specs.json`,
governing `crates/traverse-runtime/` and `crates/traverse-mcp/`) and ADR-0058
("Explicit Sequential Workflow Recovery"). A Decision record is already posted
as a comment on the issue. Five of the seven DoD checkboxes are cleanly
satisfied by Spec 129's FRs and ADR-0058. Two are soft: checkbox 2 wants
*placement* named among the per-automatic-action requirements (Spec 129 leans
on `109`/`111`, which it extends, and does not use the word), and checkbox 4
wants an explicit *failure matrix* (Spec 129 covers the cases across FR-005,
FR-006, and Acceptance Scenarios 1-4, but not as a labeled matrix). Spec 129
is immutable, so neither gap can be closed by editing it. `#1235` also carries
`future` and `priority:p4` — it is explicitly not the current active slice.

### Decision

1. **Rescope `#1235` in place** into an implementation ticket rather than
   closing it and opening a fresh one. This follows the repository's own
   established pattern — Decision 60 rescoped `#1153` from a design question to
   "implement per spec 120" the moment that spec was approved, and Decision 62 /
   `#1098` was "retitled and rescoped to match." Keeping one thread keeps the
   Decision record comment and the full history together.
2. **Close the two soft DoD checkboxes with a traceability note**, not a spec
   edit or a successor slice: add
   `specs/129-governed-workflow-reliability/dod-traceability.md` mapping each of
   the seven DoD items to its FR/ADR location and stating explicitly that
   placement is governed by `109`/`111` (which `129` extends) and that the
   failure matrix is enumerated by FR-005 + FR-006 + Acceptance Scenarios 1-4.
   This matches the repo's traceability-heavy style and makes the closure
   auditable outside a GitHub comment.
3. **Labels and board**: remove `needs-spec` (blocker cleared) and `spec` (no
   longer a spec ticket); keep `enhancement`, `workflow`, `runtime`, `future`,
   `priority:p4`; move the project card off **Ready** to the backlog/later
   column, because `future` + `priority:p4` means it must not sit in a
   "pull this next" column.
4. **Implementation DoD points at Spec 129's own gates**: QG-001..QG-003 pass
   (FR-005 rejection-category tests; integration tests for success-after-retry,
   reverse-order compensation, compensation failure, interruption recovery;
   coverage + lint + spec-alignment declaring `129`), Acceptance Scenarios 1-4
   covered, scoped to `crates/traverse-runtime/` and `crates/traverse-mcp/` per
   the spec's `governs` list. No bespoke checklist, no split into sub-tickets
   while the work is `p4` and unscheduled.

### Alternatives Considered

- **Close `#1235` as done, open a separate implementation ticket** — rejected;
  cuts against the repo's own recent rescope precedent (Decisions 60, 62) and
  scatters continuity across two tickets for no real gain.
- **Close `#1235`, open no implementation ticket yet** — rejected as the
  default, though its point stands: the rescoped ticket keeps `future`/`p4` and
  moves off Ready, so it does not manufacture active WIP.
- **Accept the two soft checkboxes as substantively met, recorded only in the
  closing comment** — rejected; defensible reading, but a future auditor would
  have to reconstruct it from a comment rather than a spec-adjacent doc.
- **Draft a Spec 129 successor slice adding an explicit failure-matrix table
  and a placement clause** — rejected; heavy governance process for two
  checkboxes already covered in substance, and `priority:p4` does not justify a
  new governing slice.
- **Keep the `spec` label / leave the card on Ready** — rejected; `spec` does
  not describe an implementation ticket and Ready would contradict `future`/`p4`.

### Outcome

The spec work `#1235` asked for is complete. Remaining actions, all on the
maintainer (none block on the runtime or another ticket):

1. Author `specs/129-governed-workflow-reliability/dod-traceability.md` (the
   DoD-to-FR/ADR map) and merge it.
2. On `#1235`: retitle to an implementation framing, replace the DoD body with
   the Spec 129 QG / Acceptance-Scenario bar from point 4 above, tick the
   original seven boxes referencing the traceability note, remove `needs-spec`
   and `spec`, and post a closing-of-spec-phase comment linking Spec 129,
   ADR-0058, and PR #1245.
3. Move the `#1235` project card from **Ready** to the backlog/later column.

## Decision 68: Capability-Side WASM Host ABI for Stateful Persistence — New Spec, Key-Value Trio, Host-Isolated Partitions

- **Date**: 2026-09-08
- **Status**: Accepted; new governing spec + ADR to be drafted from this log
- **Governing spec**: new spec (working title "Capability-Side WASM Host ABI for
  Stateful Persistence"), extends `002-capability-contracts`,
  `208-service-type-taxonomy` (canonical `014`), `518-durable-local-datastore`;
  paired new ADR. Precedent: `098-capability-event-host-abi` / ADR-0035 /
  Decisions 48–49.
- **Related issues**: `#1285`
- **Origin**: `/brainstorm` on `#1285` ("Host storage/lifecycle ABI needed for
  UMA Stateful capabilities"), filed 2026-09-08 with no governing spec, no ADR,
  no labels, and not on Project 1. Registry decision-log entry 92 (registry
  repo) mapped ≥10 real Stateful capabilities (cart, ticket workspace, approval
  packet store, challenge session, …) whose publication is blocked because
  `host_abi_v1.json` exposes no managed-persistence import — only WASI stdio,
  `traverse_host` environment/metadata queries, `emit_event`, and
  `connector_invoke`. Owner-participated brainstorm; Enrico deferred each
  sub-call to the recommendation.

### Context

`service_type: Stateful` already exists on `CapabilityContract` (spec
`208`/`014`) and the placement evaluator already excludes `Browser` for it
(`208` FR-005). What is missing is the runtime surface: a Stateful capability
has no way to actually persist or read state during execution. The closest
precedent is the event-publish ABI (`098`): one whitelisted `traverse_host`
import, a call-time `service_type` gate, synchronous validation, guest-memory
bounds safety (`098` FR-008), and no back-compat tax (Decision 48). Separately,
DataStore v2 (`518` + `519`/`528`) is a mature host-side storage subsystem, but
`518` is explicitly *embedder*-facing and states it does not add a capability
contract or a global runtime storage policy.

### Decision

1. **Governance vehicle**: a new standalone spec + new ADR, mirroring how
   `098`/ADR-0035 landed the event ABI. Not an amendment of `098` (tightly
   scoped to event pub/sub, Approved v1.1.0) and not an addendum to `518`
   (whose stated boundary explicitly excludes a capability-facing contract).
2. **ABI surface**: a key-value trio of flat `traverse_host` imports —
   `state_get`, `state_put`, `state_delete` — whitelisted and documented the
   same way as existing `traverse_*` imports. No `list`, no batch, no
   multiplexed single-import. Maps 1:1 onto DataStore v2 and `530` remote-KV.
   List/batch may be added later as explicit named imports if a real capability
   needs them.
3. **Key scoping**: the host always prefixes storage with the calling
   `capability_id` (guest cannot escape it or read another capability's state).
   Within that, the guest passes an opaque `partition` (user id, session id, …)
   per call and the host composes `capability_id / partition / key`. Call
   signatures are `state_get(partition, key)`, `state_put(partition, key,
   value)`, `state_delete(partition, key)`. The guest never sees raw storage
   paths. This lets one `commerce.cart` deployment serve every shopper without
   per-user activation; a guest partition bug can cross users *within its own
   namespace* but never outside it.
4. **Teardown**: per-key `state_delete` only in v1. Partition-wide clear is not
   in the ABI — wholesale teardown stays with the embedder (retention spec
   `526`), which is accountable for retention/backup. Roster's real "forget"
   case (`identity.challenge-session`) writes few keys per partition. A
   `state_clear(partition)` import may be added later if N-call cleanup proves a
   problem.
5. **Backing store**: the spec defines an abstract `StatefulStore`-shaped host
   trait with a written guarantee floor — integrity-checked, atomic write,
   read-your-writes within a partition, durable across restart. The embedded
   host binds it to DataStore v2 by default; a host may substitute (in-memory
   for tests, `530` remote-KV, Redis) provided the substitute meets the floor.
   Matches `connector_invoke` mediation and `519`'s embedder-owned framing.
6. **Bounds & quota**: the spec fixes conservative caps — max key length, max
   partition length, max value size (≈1 MiB) — and the host validates the
   guest-supplied pointer/length is within linear memory, rejects oversized
   input with a stable error code, and never traps or panics (mirrors `098`
   FR-008). No per-capability key-count or total-bytes quota in v1; an
   embedder-set quota is a clean follow-up once real workloads exist to size
   against (same reasoning spec `050` used).
7. **Browser placement**: `208` FR-005's `Stateful` + `Browser` →
   `InvalidPlacementConstraint` rule is left untouched in this slice. The
   IndexedDB-backed relaxation (`Stateful` on `Browser` when a `528` DataStore
   is bound) is recorded as an explicit follow-up, not done here — the entire
   `#1285` roster is server/edge-side.
8. **Concurrency**: the host takes a short-lived per-`(capability_id,
   partition)` lock around each get/put/delete; the guest sees a plain
   sequential model and never sees contention. Matches DataStore's single-writer
   design (`518` US4). No compare-and-swap / version tokens in v1 — a
   read-modify-write split across two executions still races unless done within
   one execution; CAS is a follow-up if a capability needs cross-execution
   optimistic concurrency.
9. **Trace journal**: each `state_put` / `state_delete` appends a trace entry
   carrying `capability_id`, digested `partition`, `key`, value `digest`, and
   `execution_id` — never the value itself. `state_get` is not traced. Gives the
   audit-sensitive roster items (`doc-approval.packet-store`,
   `doc-approval.policy-store`) a real record without copying business data into
   the append-only journal or bringing it under `527` retention/encryption
   scope.
10. **Stateful + events (multi-role)**: explicitly out of scope. `service_type`
    is a single enum and `098` FR-003 gates `emit_event` to `Subscribable`
    only, so a Stateful capability cannot emit/receive events. The spec gates
    `state_*` to `service_type: Stateful` and records the multi-role gap
    (session-flavored capabilities wanting both) as a known limitation deferred
    to its own decision. The bulk of the roster is pure-Stateful.

Inherited from the `098` precedent and baked into the spec without a separate
sub-decision: call-time `service_type: Stateful` gate with synchronous
rejection; stable, secret-free error codes returned to the guest; host never
traps/panics; whitelist entries documented alongside the existing native-bridge
ABI whitelist; folded into ABI v1 with no back-compat tax (Decision 48).

### Alternatives Considered

- **Amend `098` to cover storage too** — rejected; `098` is Approved v1.1.0 and
  tightly scoped to event pub/sub, and widening it mixes two concerns in one
  amendment of an approved spec.
- **Fold into `518` as a capability-facing addendum** — rejected; `518`
  explicitly does not add a capability contract or a global runtime storage
  policy and is embedder-owned — a direct boundary contradiction.
- **Handle-based ABI** (`state_open`/`read`/`write`/`close`/`dispose`) —
  rejected; 5 imports vs 3, a per-execution handle table for the host to
  manage, and more failure modes (stale/double-close), heavier than any
  existing capability ABI for no roster need.
- **Single multiplexed `state_op(request_json)` import** — rejected; opaque to
  static ABI inspection, which defeats the per-function whitelist that
  `host_abi_v1.json` exists to provide.
- **`capability_id` namespace only, no partition** — rejected; no structural
  per-user boundary, so a capability that forgets to namespace keys silently
  shares one user's cart with all, and per-user retention/dispose has nothing
  to target.
- **Host-pinned scope from `runtime_config`** — rejected; one activation = one
  scope, so N concurrent users need N activations or per-request rebinding;
  does not fit a long-lived embedded host.
- **Add `state_clear(partition)` now** (plain or contract-opt-in) — deferred;
  overlaps embedder retention responsibility and adds ABI/contract surface the
  roster does not yet need.
- **Mandate DataStore v2 as the backing store** — rejected; couples the ABI to
  the `518`+`528`+`522` surface, leaves no seam for tests or alternative
  stores, and contradicts the mediation pattern used elsewhere in the runtime.
- **Leave persistence entirely unspecified in v1** — rejected; no durability /
  integrity floor means "Stateful" promises nothing portable and registry
  decision-92's "real managed persistence" bar goes unenforced.
- **`runtime_config`-driven quota in v1** — deferred; real Stateful workloads
  do not exist yet to size a quota against.
- **Relax the `Stateful` + `Browser` ban in this slice** — deferred; widens the
  slice into an amendment of Approved spec `208`/`014` for a capability class
  nothing in the roster needs.
- **Expose contention / add CAS** — deferred; every Stateful capability would
  have to implement retry/backoff, and CAS widens get/put with version tokens.
- **Full value in the trace journal** — rejected; doubles storage, brings
  business data under `527` retention/encryption scope, and cuts against the
  journal's metadata-not-payload intent.
- **Gate `state_*` on a contract `uses_storage` flag instead of the enum** —
  rejected; adds a second axis beside `service_type`, and `208`'s placement
  constraints are written against `service_type`, so a `Subscribable`+storage
  capability would not inherit Stateful target rules — a real placement hole.
- **Address multi-role (Stateful + events) here** — rejected; turns a focused
  storage-ABI slice into a `service_type` taxonomy change touching Approved
  specs `208`/`014` and `098`.

### Outcome

The design `#1285` asked for is settled. Remaining actions, all on the
maintainer (none block on the runtime or another ticket):

1. Draft the new governing spec from points 1–10 above (FRs for the three
   imports, the `StatefulStore` guarantee floor, bounds, the call-time gate,
   the trace-entry type) and its paired ADR; assign the canonical governing ID
   at draft time.
2. Triage `#1285` into a proper spec ticket: add it to Project 1, label it
   `spec` / `runtime` / `security`, and give it a real Definition of Done
   (spec + ADR approved, then the three imports implemented and whitelisted,
   per-partition serialization, trace entries, negative fixtures for the gate /
   bounds / wrong-`service_type` / missing-store cases).
3. File follow-up tickets for the deferred items: `state_clear`, embedder
   quota, `Stateful`+`Browser` via IndexedDB, cross-execution CAS, and the
   multi-role (Stateful + events) taxonomy question.

## Decision 69: Approve Specs 131, 130, and 1256; Resolve the ADR-0059 Numbering Collision

- **Date**: 2026-09-08
- **Status**: Accepted
- **Governing specs**: `131-stateful-persistence-host-abi` (new, Approved);
  `130-mixed-registry-reference-activation` (Draft → Approved 1.0.0);
  `1256-registry-genericity-policy` (Draft → Approved 0.1.0); `004-spec-alignment-gate`
- **ADRs**: `0063-stateful-persistence-host-abi` (new, Accepted); `0059-mixed-registry-reference-activation` (Proposed → Accepted); `0064-registry-genericity-and-configuration-policy` (renumbered from a second `0059`, Proposed → Accepted)
- **Related issues**: `#1285` (Project Item), `#1258`, `#1256`
- **Origin**: Maintainer instruction immediately after the Decision 68
  `/brainstorm`: author the `#1285` spec + ADR and merge them rather than leave
  drafts, and approve the outstanding Draft specs and their ADRs in the same
  pass. Surfaced by the `status-check` run earlier in the same session, which
  flagged `130` and `1256` as Draft-pending-approval and the duplicate ADR
  number as an untracked fix.

### Context

Three governance items were open at once:

1. `#1285` needed a governing spec + ADR; Decision 68 settled the full design
   but left authoring and approval as maintainer to-dos.
2. `130-mixed-registry-reference-activation` (proposed via `#1258`, ADR-0059)
   was `Draft` and is the named governing surface in `#1275`'s Definition of
   Done, which blocks `#1275` → `#1276`.
3. `1256-registry-genericity-policy` (drafted via `#1263`) was `Draft` with a
   paired `Proposed` ADR.

Two ADR files both carried the number `0059`
(`0059-mixed-registry-reference-activation` and
`0059-registry-genericity-and-configuration-policy`), a real collision on the
sequential ADR index.

### Decision

1. **Author and approve `131-stateful-persistence-host-abi` (v0.1.0)** directly
   from Decision 68's ten points, with new ADR `0063-stateful-persistence-host-abi`
   (Accepted). Recorded in `approved-specs.json` as immutable, governing
   `crates/traverse-runtime/src/executor/`,
   `crates/traverse-runtime/src/trace_journal.rs`, `crates/traverse-contracts/`,
   the spec directory, and the ADR.
2. **Approve `130-mixed-registry-reference-activation`** as-is at v1.0.0
   (`1.0.0-draft` → `1.0.0`); flip ADR-0059 (mixed) `Proposed` → `Accepted`
   with an approval-evidence section. The spec is additive, extends the already
   approved `106`/`107`, and matches its ADR; no content change on approval.
3. **Approve `1256-registry-genericity-policy`** as-is at v0.1.0; flip its ADR
   `Proposed` → `Accepted`. The spec is an additive Registry admission policy
   with no Traverse-repo code surface, scoped in the manifest to its own spec
   directory and ADR (the same shape as `1259-portable-authority-contracts`).
4. **Resolve the ADR-0059 collision** by keeping `0059` for the
   earlier-committed mixed-registry ADR (`#1258`, commit `26f7d33`, before the
   genericity draft `86a74ff`) and renumbering the genericity ADR to the next
   free index, `0064` (0060–0062 already taken). A note in the renumbered file
   records the original number.
5. **Bundle all of the above into one PR** rather than three. Every change is a
   spec header, an ADR status line, a manifest entry, or a decision-log entry;
   one squashed commit keeps the CI cost and the revert surface to a single
   unit. The PR declares `004-spec-alignment-gate` plus the three approved spec
   IDs in its `## Governing Spec` section (the manifest file is governed by
   `004`).

### Alternatives Considered

- **Leave `131` as an unmerged draft** — rejected; the maintainer explicitly
  asked for create-and-merge, and Decision 68 already fixed every design point a
  draft-review would surface.
- **Three separate PRs (one per spec)** — rejected; each would trip the full CI
  matrix (the manifest lives under `specs/governance/`, which forces
  `full_ci=true`), tripling the wall-clock and rate-limit cost for changes that
  are entirely documentation and a JSON registry.
- **Renumber the mixed-registry ADR instead of the genericity one** — rejected;
  the mixed-registry ADR was committed first and is already referenced by name
  in `130`'s header, so moving it churns more references.
- **Approve `130`/`1256` without recording a decision-log entry** — rejected;
  neither had a prior decision-log entry (only issue-comment decision records),
  so this entry is where their approval and coherence review is captured, in the
  style of Decisions 63–65.
- **Hold `130`/`1256` for a fuller re-review** — rejected; both are additive,
  internally coherent with their ADRs and cited decision records, and `130` is
  actively blocking `#1275`.

### Outcome

Specs `131`, `130`, and `1256` are Approved and immutable in
`approved-specs.json`; ADRs `0063`, `0059` (mixed), and `0064` are Accepted; the
ADR index no longer has a duplicate `0059`. Follow-ups:

1. `#1275` can proceed on its `130` governing-surface Definition-of-Done item;
   `#1276` unblocks once `#1274`/`#1275` implementation lands.
2. `#1285` still needs the three imports implemented and whitelisted, the
   per-partition serialization, the trace-entry type, and negative fixtures —
   plus triage onto Project 1 with a `spec`/`runtime`/`security` label set and
   the deferred-item follow-up tickets from Decision 68.
3. Registry genericity enforcement (`1256`) and its audit of existing records
   are downstream registry-repo work.

## Decision 70: Locked-Ticket Triage After the Spec 130 / 996 Approvals — Park the Registry-Reference Cluster on Registry #387

- **Date**: 2026-09-08
- **Status**: Accepted
- **Governing specs**: `130-mixed-registry-reference-activation`, `996-registry-app-preparation` (both approved via Decision 69 / PRs #1286, #1284); `108-governed-runtime-workflow-composition` (declared to cover this `docs/decision-log.md` entry)
- **Related issues**: `#1272`, `#1274`, `#1275`, `#1276`, `#1168`; `traverse-framework/registry#387`
- **Origin**: `/brainstorm locked tickets` after Decision 69 approved the last
  Traverse-side spec blockers. A live re-check found the entire exact-version
  registry-reference cluster now waits on one thing: `traverse-framework/registry#387`
  (expose the versioned preparation contract, `traverse-registry 0.18.0 -> 0.19.0`,
  publish on the `v0.19.0` tag), which registry PR #392 moved `Blocked -> Ready`.
  Nothing in the chain is blocked on the maintainer.

### Context

Post-Decision-69 dependency state:

- `registry#387` — **Ready**, `agent:claude`, sequenced after `registry#384`
  (done). Pure implementation + crate publish. No governance left.
- `#1274` (verified preparation slice) — Blocked; Spec 996 accepted in both
  repos; waits only on `registry#387` releasing a pinnable `traverse-registry
  v0.19.0`.
- `#1275` (offline activation slice) — Blocked on `#1274`; Spec 130 now
  approved, clearing its only governance blocker.
- `#1276` (conformance fixtures) — Blocked on `#1274` + `#1275`.
- `#1168` (two-app reuse proof, `priority:p2`, parent `#1152`) — Blocked on the
  whole chain plus prerequisites that are not ticketed (published capabilities,
  two independently scoped apps).
- `#1272` — the umbrella bug ticket; `#1273` (diagnosis) already merged.

### Decision

1. **Do not start Traverse-side work in parallel.** Keep `#1274`/`#1275`/`#1276`
   parked until `traverse-registry v0.19.0` is pinnable. `#1274`'s own analysis
   is that implementing before the crate API exists violates the cache-only
   resolver boundary and creates ad-hoc public error semantics, and Spec 996
   deliberately leaves the Rust type/error/evidence shapes to a Plan+implement
   session. Parking is the honest state; the lever is `registry#387`, not
   pre-building against it.
2. **Make the cross-repo dependency legible on the board.** Add an explicit
   "blocked by `traverse-framework/registry#387` (needs `traverse-registry
   v0.19.0`)" line to `#1274` and a one-line dependency pointer to the `#1272`
   umbrella. Leave `#1275`/`#1276` pointing at the Traverse ticket above them
   (already legible via their Parent sections).
3. **Move `#1168` to `future`.** It is not actionable until a four-ticket
   cross-repo chain completes and needs prerequisites that do not yet exist.
   `future` matches its actual readiness and its parent `#1152` / sibling
   `#1150`, and keeps the `Blocked` column meaningful.
4. **Keep `#1272` as the `Blocked` tracking umbrella** (with the dependency
   note), distinct from leaf tickets. Its acceptance criteria remain the
   end goal that `#1274` + `#1275` + `#1276` collectively satisfy; closing it is
   the "cluster done" signal.
5. **Tick the now-satisfied governance DoD boxes.** On `#1274`, check
   "Governing spec/contract references ... explicit and accepted" (Spec 996 —
   Traverse #1284, registry #391). On `#1275`, check "Spec 130 ... is the
   documented governing surface" (PR #1286). Add a comment on each noting that
   only implementation work remains. The checkbox tracks whether the governing
   spec is accepted, not whether code is written; leaving it unticked
   misrepresents remaining scope.

### Alternatives Considered

- **Start `#1275`/`#1276` scaffolding now against Spec 130** (stub the registry
  crate behind a local trait) — rejected; the stubbed boundary may not match
  `registry#387`'s real contract, `#1275`'s DoD needs `#1274`'s evidence format
  which does not exist yet, and it risks half-wired merges.
- **One combined implementation arc after `#387` ships** — folded into
  Decision 1 (wait); this is how the work should be picked up, not a separate
  choice.
- **Propagate the `registry#387` dependency onto every ticket in the chain** —
  rejected; four tickets to keep in sync, redundant with each body's
  Parent/Depends-on section.
- **Leave the dependency only in `#1274`'s comment thread** — rejected; buried
  in seven comments, and the board shows only "Blocked" with no "on what",
  forcing repeated re-derivation.
- **Move `#1272` to `future` alongside `#1168`** — rejected; `#1274` is the
  genuine "next up once `#387` ships", so marking its umbrella deferred is
  misleading.
- **Close `#1272` now** — rejected; loses the consolidated acceptance criteria
  and the end-to-end "is the bug fixed" checkpoint.
- **Leave all DoD boxes for the implementation PR** — rejected; the "governing
  spec accepted" box was the actual blocker for weeks and is objectively done.

### Outcome

The registry-reference cluster is correctly parked with one visible external
dependency. Remaining actions (none on the maintainer):

1. `registry#387` — implement the preparation API in `crates/traverse-registry/`
   and publish `traverse-registry v0.19.0`. This is the critical path for the
   entire cluster and needs an ops/implementation session.
2. Once `v0.19.0` is published: pick up `#1274` -> `#1275` -> `#1276` as one
   implementation arc against Specs 996 and 130.
3. Board changes from Decisions 2-5 applied to `#1272`, `#1274`, `#1275`,
   `#1168` directly on GitHub.

## Decision 71: Implement Spec 1285 State Host ABI (Relative-Key v1)

**Date**: 2026-09-08  
**Issue**: #1285  
**Spec**: `specs/1285-capability-state-host-abi/spec.md` (Draft)
**Supersedes / refines**: Decision 68 partition field deferred in v1

### Context

Registry Wave 2 needs honest UMA Stateful capabilities. Host ABI 1.0.0 had
`emit_event` for Subscribable but no guest import for managed persistence,
despite `DataStore` / `RuntimeDataStore` already existing in-tree.

### Decisions (owner `/brainstorm`, recommended option each time)

1. Wire existing DataStore into `traverse_host` (not connector-only, not a parallel session ABI).
2. Three imports only: `state_get`, `state_put`, `state_delete`.
3. Host prefixes keys with `{capability_id}/`.
4. Missing injected store → hard fail (`data_store_not_configured`); explicit in-memory inject for tests.
5. Host stamps lamport/writer; guest sends `{key,value}` only.
6. Fixed `state_schema.properties` keys; resource ids inside values.
7. Stateful contracts require non-empty `state_schema`; Browser still forbidden.
8. Ship Traverse ABI before registry Wave 2 publishes.
9. New focused spec (098 playbook), not taxonomy-only amend.
10. Extend `host_abi_v1` / ABI `1.0.0` whitelist (optional imports).

### Outcome

Landed Spec 1285 + `state_*` in `traverse-runtime` with Decision 68's trio and
capability namespacing. v1 guest envelopes use relative keys only (resource ids
inside values); an explicit `partition` parameter from Decision 68 remains a
follow-up if multi-tenant key fan-out needs it at the ABI layer. Unblocks
registry Wave 2 Stateful publishes.

## Decision 72: Stateful Browser Placement — Contract Allow, Activation Attest (Spec 132)

**Date**: 2026-09-09  
**Issues**: #1305 (specification), #1289 (implementation)  
**Governing artifacts**: Spec `132-stateful-browser-placement` (Proposed), ADR-0065 (Proposed)  
**Related**: Decision 68 (deferred Browser placement); Specs `014`/`208`, `085`, `131`

### Context

Future tickets from Decision 68 included relaxing `Stateful` + `Browser` now
that Spec `085` IndexedDB exists. A `/brainstorm` drained that cluster's first
governing package.

### Decisions (owner `/brainstorm`, recommended option each time)

1. Prioritize the Stateful/Browser future cluster over Mode B MCP, #1150
   children, or waiting only on the #1272 registry proof.
2. Open #1289's governing package before taxonomy (#1291) or ABI extensions
   (#1287/#1288/#1290).
3. Allow Stateful+Browser at contract validation; enforce durable-store proof
   only at activation (`stateful_browser_store_unavailable` on failure).
4. Attestation = runtime-verifiable open Spec `085` IndexedDB DataStore (not
   an embedder flag; not a per-activation conformance certificate).
5. File dedicated specification issue #1305; keep #1289 as implementation with
   `needs-spec` until approval.

### Outcome

#1305 owned Proposed Spec `132` + ADR-0065 on PR #1307. Decision 73 approved
the package the same day; #1289 then moved to `spec-complete` / Ready.
Encryption (#1294), maintenance (#1295), ABI extensions, and #1291 remain
`future`.

## Decision 73: Approve Spec 132 and ADR-0065 (Stateful Browser Placement)

- **Date**: 2026-09-09
- **Status**: Accepted
- **Governing specs**: `132-stateful-browser-placement` (Proposed → Approved 0.1.0);
  `014-service-type-taxonomy` (FR-005 superseded for placement only);
  `085-datastore-indexeddb`; `131-stateful-persistence-host-abi`; `004-spec-alignment-gate`
- **ADR**: `0065-stateful-browser-placement` (Proposed → Accepted)
- **Related issues**: `#1305` (specification), `#1289` (implementation)
- **Origin**: Maintainer `/brainstorm` approval (Option A) after Decision 72 locked the design and PR #1307 landed Proposed artifacts.

### Context

Decision 72 produced Proposed Spec `132` + ADR-0065. #1289 remained
`needs-spec` until approval evidence existed.

### Decision

Approve Spec `132-stateful-browser-placement` v0.1.0 as written and accept
ADR-0065. Register the immutable entry in `approved-specs.json`. Mark #1305
Done and move #1289 to `spec-complete` / Ready for implementation.

### Outcome

Contract-time Stateful+Browser ban is superseded; activation attestation under
Spec `085` is the governed fail-closed rule. Implementation proceeds on #1289.

## Decision 74: npm publish path for `traverse-embedder-web` (issue #1314)

**Date**: 2026-09-09  
**Issue**: #1314 (parent #1308; downstream traverse-framework/website#68)  
**Spec**: `048-semver-publishing-pipeline` (amended → v1.2.0 by PR #1317)

### Context

`traverse-embedder-web` is at `0.8.0` in `main` (PR #1312, `5181f1b`) but npm
still serves `0.7.0`; `npm whoami` in the release environment returns E401. It
is the repo's only npm artifact, published under the sole personal maintainer
account `enricopiovesan`, with no CI automation — publishing had always been a
manual, undocumented step. Spec 048 governed "the complete semver lifecycle" but
was entirely crates.io/Cargo. #1314 asked not just to ship `0.8.0` but to make
the path durable so `0.9.0` (#1313) does not hit E401 again.

### Decisions (owner `/brainstorm`, recommended option taken each time)

1. **Go-forward mechanism: npm Trusted Publishing (OIDC).** A GitHub Actions
   workflow publishes via a short-lived OIDC token; npmjs.com is configured to
   trust the repo + workflow. No stored secret, nothing to expire — structurally
   removes the E401 failure mode; provenance automatic.
   - Rejected: *CI + stored npm automation token* — standard, but a long-lived
     secret to own/rotate; only a never-expiring classic token fully avoids
     repeat E401.
   - Rejected: *documented manual publish with a granular token* — zero CI work,
     but bus-factor of one, no provenance, and granular tokens expire (≤1yr) →
     the same trap #1314 exists to kill, just written down.

2. **`0.8.0` bridge: one-time manual publish now.** *(Superseded by Decision 75
   — `0.8.0` instead ships as the first OIDC run.)*
   - Rejected: *wait, make `0.8.0` the first OIDC release* — later adopted.
   - Rejected: *publish `0.8.0` and `0.9.0` manually* — two more chances to
     re-hit E401.

3. **Workflow trigger: dedicated `web-v<version>` tag.** Push `web-v0.9.0` from
   `main` → workflow publishes that version. Matches the existing "push a tag to
   release" model; stays clear of the `v*` crates namespace and its
   `version-guard` job.
   - Rejected: *`workflow_dispatch` with a version input* — simplest and easy to
     rerun, but no git artifact marking the release.
   - Rejected: *auto-publish on version bump merged to `main`* — most moving
     parts; publish timing tied to PR merge, not choice.

4. **Pre-publish checks: version-guard + build + `npm test`.** Assert the
   `web-v<version>` tag matches `packages/web/TraverseEmbedder/package.json`, run
   the build, run the package's own suite, then publish. A mis-cut tag or a tag
   on a non-green commit cannot publish.
   - Rejected: *version-guard + build only* — nothing catches a tag on a commit
     whose web tests were never green.
   - Rejected: *reuse `web_package.sh` as the gate* — more than a release needs.

5. **Governance: amend spec `048-semver-publishing-pipeline` → v1.2.0.** Add npm
   publishing of `traverse-embedder-web` as a second governed target alongside
   crates, with acceptance scenarios for the `web-v<version>` tag, version-guard,
   OIDC + provenance, and idempotent rerun. Amendment rides in the
   implementation PR per standing spec-approval policy.
   - Rejected: *new dedicated JS-publishing spec* — spec sprawl for one package.
   - Rejected: *ADR only* — a release gate belongs in a spec per constitution III.

6. **Ticketing: two tickets.** #1314 = release execution; a new ticket (#1316) =
   spec 048 v1.2.0 amendment + `web-v*` workflow + docs.
   - Rejected: *one re-scoped #1314 on Project 1* — `needs-enrico` vs
     `agent:claude` on one ticket, which the ticket standard warns against.
   - Rejected: *three tickets* — the spec amendment rides in the impl PR.

### Outcome

#1316 was implemented by **PR #1317** (merged 2026-09-09): spec 048 → v1.2.0,
`.github/workflows/web-embedder-publish.yml` (trigger `web-v*`, `id-token: write`,
`npm ci` → tag/version guard → `npm run build` → `npm test` →
`npm publish --provenance --access public`, idempotent skip when the version is
already on npm), `packages/web/TraverseEmbedder/.npmrc` with
`tag-version-prefix=web-v`, and the rewritten
`docs/web-embedder-npm-publish-runbook.md`. #1316 closed.

Remaining, one-time, on the maintainer: on npmjs.com, add
`traverse-framework/traverse` + `web-embedder-publish.yml` as a trusted publisher
for `traverse-embedder-web`. No tokens anywhere.

## Decision 75: Make #1314 ops-loop-workable — `0.8.0` ships via the OIDC workflow

**Date**: 2026-09-09  
**Issue**: #1314 (and #1316 / PR #1317)  
**Supersedes**: Decision 74 §2 (manual `0.8.0` bridge); refines Decision 74 §6

### Context

Decision 74 left #1314 as human-only npm-account work (mint token,
`npm publish` `0.8.0`, configure trusted publisher). The maintainer needs #1314
to be executable by the ops-loop, not a manual task. npm Trusted Publishing
works for personal-account packages too, so `0.8.0` can be the workflow's first
run rather than a hand publish.

### Decisions (owner `/brainstorm`, recommended option taken each time)

1. **`0.8.0` ships as the first OIDC run**, not a manual bridge publish. With
   #1317 merged, configure the npmjs.com trusted publisher once (no token), then
   cut `web-v0.8.0` and let the workflow publish with provenance. Reverses
   Decision 74 §2.
   - Rejected: *move the package to a `traverse-framework` npm org first* — fixes
     the personal-account root cause but is the largest one-time human effort
     (org creation, package transfer, possibly a paid org) and the slowest path
     to `0.8.0`. Can still happen later as its own ticket.
   - Rejected: *keep #1314 human-only, tighten to a checklist* — smallest change
     but does not deliver a workable #1314.

2. **Re-slice: #1316 = infra, #1314 = release execution blocked-by #1316.** #1316
   (now merged via #1317) is spec 048 v1.2.0 + workflow + docs, self-contained
   and PR-verifiable. #1314 becomes agent-workable — cut `web-v0.8.0`, verify npm
   + provenance, attach evidence to #1308, close #1308 — blocked only by the
   one-time npmjs.com trusted-publisher config (`needs-enrico`).
   - Rejected: *#1316 owns everything, #1314 shrinks to the config* — #1316's DoD
     would span a mergeable workflow plus a release gated on an external human
     step; two-phase, not PR-verifiable.
   - Rejected: *merge #1314 into #1316* — mixed-owner DoD; `needs-enrico` +
     `agent:claude` on one ticket.

### Outcome

- Decision 74 §2 no longer applies: no manual `npm publish` of `0.8.0`.
- Infra (#1316) landed via PR #1317; #1314's `#1316-merged` blocker is cleared.
- #1314's only remaining gate is the npmjs.com trusted-publisher config
  (`needs-enrico`, ~5 min, no token). Once live, #1314 flips to Project 1
  **Ready** and the ops-loop cuts `web-v0.8.0`; the workflow publishes `0.8.0`
  with provenance, and #1314 attaches evidence to #1308 and closes it.
- Follow-up nit: `docs/web-embedder-npm-publish-runbook.md` (from #1317) still
  describes `0.8.0` as a "manual exception that predates Trusted Publishing";
  per this decision `0.8.0` goes through the OIDC workflow like every other
  release. Correct the wording opportunistically.
