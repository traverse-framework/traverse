# Changelog

## Unreleased

## v0.10.1 — 2026-09-09

### Fixes

- Runtime: treat WASI preview1 `proc_exit(0)` as successful completion, while
  retaining fail-closed handling for nonzero exit codes.
- Web embedder: provide deterministic denied implementations for the approved
  `emit_event` and `connector_invoke` Host ABI imports so ABI-valid modules
  load without ambient connector authority.
- CLI: `app new` now writes the documented `app.manifest.json` filename.

## v0.10.0 — 2026-09-05

### Governed runtime workflow composition

- Declarative workflow planner (spec 113, P0): the MCP surface can plan a
  multi-capability workflow from a goal, chaining steps on schema-shape
  compatibility rather than declared `consumes`/`emits` (Decision 62).
- Runtime workflow proposal lifecycle over MCP (P1), bounded deterministic
  parallel proposal scheduling (P2), and governed export and promotion of
  runtime-generated proposals back into authored workflows (P4).
- Durable orchestration controls with checkpoint/recovery guards, and proven
  recovery of an explicitly abandoned workflow proposal.

### Durable trace journal

- `DurableTraceJournal` persists execution traces through the existing
  append-only, fsync-committed event journal (spec 079, ADR-0045). Traces are
  canonical JSON Lines carrying only non-sensitive metadata and hashes, per
  workspace, with the journal's existing crash-recovery and whole-segment
  retention semantics. Opt-in via `PlacementRouter::with_durable_trace`;
  existing call sites are unaffected. Callers choose fail-open or fail-closed
  on durable-write failure (ADR-0017).

### Browser-hosted execution

- Governed browser proposal host and end-to-end proposal journey.
- Browser execution of a verified single capability that returns an execution
  receipt, along the validated `execute_entrypoint` path (spec 023).

### Verified registry metadata and host-owned artifacts

- `serve` requires host-supplied registry state and exposes a verified
  entrypoint endpoint.
- MCP verified capability search over host-owned, verified public metadata.
- Embedder caches verified public contract metadata.
- CLI materializes verified registry artifacts locally (spec 120, ADR-0051).

### Authoring telemetry

- Privacy-preserving, manifest-governed authoring telemetry: manifest policy
  validation, host-owned aggregation, and host config that is **off by
  default** (specs 184–197).

### Cross-host and native execution

- Bounded native WASI profile (spec 181) with native cross-host fixture
  evidence, a cross-host CLI fixture runner in CI, and a cross-host
  hello-world fixture corpus governed by a fixture comparison contract.

### Connectors

- Universal local connector contracts, runtime mediation of connector invoke
  requests, activation of application connector bindings, and native registry
  cache adapters for the embedder (specs 1054, 1074, 1083).

### Capabilities and authoring

- Immutable capability risk metadata on `CapabilityContract` — effect class,
  determinism class, field-level data-flow/egress policy, reliability
  semantics — with an `is_automatic_eligible()` gate, manifest-side egress
  narrowing validated at app register/validate time, and redacted exposure
  through MCP discovery (spec 109 FR-005/FR-006).
- Publish now fails closed on incomplete `use_case` surface coverage: declared
  schema surface must be covered by use cases, each with `ucNN` smoke fixtures
  (Spec 102 v1.1.0, Decision 58).
- Standalone capability packages are allowed, with a standalone package
  discovery posture exposed over MCP.
- `traverse-embedder-web` prepared for publish (packaging only).
- Loop package: added remaining capabilities and the
  `core.validate-action-item` example for registry publish;
  `core.transition-action-status` honesty-bumped to 1.2.0 and now really
  emits `status-transitioned`.

### CLI and runtime

- CLI resolves executable artifacts at activation time.
- Pinned `traverse-registry` advanced (now `=0.17.0`); the 0.15.0 bump
  published the resolve hook, and the runtime now threads an opt-in
  `UsageTelemetrySink` (`Runtime::with_usage_telemetry_sink`, NoOp by
  default) into semver-range resolution, wired at both production
  Runtime-construction points in the CLI (#930).

### Fixes

- CLI: pinned down and regression-tested the artifact-metadata trust boundary.
- CLI: bound gRPC event streams; use `Duration::from_mins` for
  `MAX_STREAM_DURATION`.
- Runtime: gate native inference on wasm.
- CI: clear stale coverage artifacts before measuring; scope heavy PR checks
  to code changes.

### Upgrade notes

- The binary remains `traverse-cli`.
- **Publishing is stricter.** Packages that publish cleanly on v0.9.1 may now
  be rejected: `use_case` surface coverage is fail-closed (schema ⊆
  use_cases ⊆ smoke), and manifest egress that is broader than the
  capability's declared data-flow/egress policy fails at register/validate
  time. Pre-existing contracts without risk metadata take a conservative
  migration default.
- `traverse-registry` is pinned at `=0.17.0`. Usage/resolve telemetry is
  opt-in and defaults to a no-op sink; no action needed unless you want to
  collect it.
- Durable trace and durable orchestration are opt-in builders; existing
  `PlacementRouter` and `Runtime` construction paths are unchanged.

## v0.9.1 — 2026-08-09

- Capability packages can be scaffolded, inspected, validated, and published through
  the CLI. Publishing now records executable artifacts with registry records and
  rejects unresolved persona references, including during dry runs.
- The runtime now carries real events emitted by executable capabilities through the
  `LocalExecutor`; the retired output-JSON convention is no longer the execution path.
- Added governed, end-to-end-smoked examples for `core.authorize`,
  `core.process-comment`, `core.transition-action-status`,
  `core.assign-ownership`, and `core.validate-action-item`.
- Added contract-surface coverage enforcement and corrected the
  `core.process-comment` published surface with an honesty bump.
- Fixed the CLI release package so its generated protobuf source is included in the
  v0.9.1 crate artifact.

- Kotlin and Swift embedder SDKs: marshal native bridge requests and map
  typed runtime bridge results, with a CI-enforced embedder conformance
  suite for both platforms (specs 071, 072).
- Kotlin embedder SDK: bound runtime execution against the Chicory Wasm
  runtime.
- Hardened durable event revocation to await completion instead of firing
  and forgetting, with test coverage for revocation failure paths.
- Recorded the decision to retain the durable event journal after
  operational evaluation (see `docs/decision-log.md` and
  `docs/adr/durable-journal-after-operational-evaluation.md`).

## v0.8.0 — 2026-07-16

See [docs/releases/v0.8.0.md](docs/releases/v0.8.0.md) for full release notes.
Highlights: public `traverse-embedder` Rust SDK and `traverse-embedder-web`
TypeScript SDK with real bundle execution (spec 068); the durable event
journal, including journal-backed replay through `subscribe`/`poll`; the
deterministic `doc-approval.recommend` capability and canonical
`doc-approval.pipeline` workflow (spec 069); HTTP API connection timeout
hardening, Sigstore placeholder-evidence rejection, and approved-specs-based
governed-artifact classification; and idempotency fixes across the event and
capability registries.
