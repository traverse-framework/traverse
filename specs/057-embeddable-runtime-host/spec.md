# Feature Specification: Embeddable In-App Runtime Host

**Feature Branch**: `057-embeddable-runtime-host`
**Created**: 2026-07-06
**Status**: Approved
**Version**: 1.0.0
**Input**: App-References Phase 3 architecture, Traverse issue #553, product decisions recorded 2026-07-06.

## Purpose

This spec defines the **production deployment model** for Traverse downstream apps:

1. The **Traverse orchestrator ships as a WASM module** embedded in every app binary or web bundle.
2. **~90% of business capabilities** ship as portable **WASM + WASI** components (no direct OS APIs).
3. **~10% platform-specific logic** ships as **`compatible` mode** capabilities with thin event wrappers; the **platform embedder** runs and kills them.
4. The **platform embedder** is **thin and uniform**: same API version and operations on iOS, Android, web, Windows, Linux, and CLI.

This spec **does not replace** approved specs; it **scopes** and **extends** them:

| Spec | Relationship |
|------|----------------|
| **033-http-json-api** | **Dev sidecar only** (`traverse-cli serve`). Not the production client path. |
| **038-wasi-host-insulation** | Governs WASM capability sandbox and Host ABI. Unchanged. |
| **044-application-bundle-manifest** | Extended by this spec with `execution_mode` on components (see FR-020–FR-024). |
| **046-public-cli-app-registration** | Dev/CI registration into sidecar workspace. Production apps bundle manifests at build time. |
| **052-app-state-machine** | State machines run inside embedded runtime WASM; embedder forwards events only. |
| **058-workflow-pipeline-execution** | Multi-step workflow invoke surface used by embedded runtime and dev sidecar. |

## Architecture

```text
┌─────────────────────────────────────────────────────────────┐
│  Native UI shell (platform toolkit — render only)           │
└───────────────────────────┬─────────────────────────────────┘
                            │ embedder events (JSON)
┌───────────────────────────▼─────────────────────────────────┐
│  Platform embedder v1 (thin — same on all platforms)        │
│    init / shutdown / submit / subscribe                       │
│    compatible.start | compatible.stop | compatible.kill     │
└───────────────────────────┬─────────────────────────────────┘
                            │ loads
┌───────────────────────────▼─────────────────────────────────┐
│  Traverse runtime (WASM module — orchestrator)                │
│    workflows, state machine, WASM capability execution      │
└───────────────┬─────────────────────────────┬─────────────────┘
                │ WASI                        │ compatible bridge
                ▼                             ▼
         WASM capabilities              compatible capabilities
         (~90%, portable)               (~10%, OS-specific logic)
```

The embedder **MUST NOT** re-implement WASI. WASI is provided by the WASM engine hosting capability modules.

The embedder **MUST NOT** contain business logic, workflow transitions, or output field computation.

## Platform Embedder Contract

Governing IDL: [`embedder-api-1.0.0.json`](embedder-api-1.0.0.json)

All platforms implement **`embedder-api/1.0.0`** with identical semantics. Version bumps require a new IDL file and conformance suite revision.

### Required operations

| Operation | Purpose |
|-----------|---------|
| `runtime.init(manifest_bundle)` | Load embedded runtime WASM + bundled app manifest/WASM artifacts |
| `runtime.shutdown()` | Tear down runtime and all compatible capabilities |
| `runtime.submit(workflow_or_capability_id, input_json)` | Start execution (see 058 for pipeline ids) |
| `runtime.subscribe(callback)` | Deliver runtime state/output/error events to UI shell |
| `compatible.start(capability_id, input_json)` | Start a compatible-mode capability instance |
| `compatible.stop(capability_id)` | Graceful stop |
| `compatible.kill(capability_id)` | Force terminate (timeout/cancel/shutdown) |

JSON is the wire format for all operation payloads unless a later embedder version adds optional typed codecs.

### Lifecycle isolation (compatible capabilities)

Embedder exposes uniform `start` / `stop` / `kill`. Internal isolation mechanism is platform-defined (Worker, XPC, subprocess, in-process task) but **MUST NOT** change the public embedder API per platform.

## Component Execution Modes

This spec extends **044** component manifests with:

```json
{
  "component_id": "traverse-starter.process-component",
  "execution_mode": "wasm",
  "wasm_binary_path": "...",
  "wasm_digest": "sha256:..."
}
```

```json
{
  "component_id": "ios.metal-inference",
  "execution_mode": "compatible",
  "platforms": ["ios"],
  "wrapper_path": "wrappers/metal-inference/",
  "contract_path": "contracts/.../metal-inference.json"
}
```

### WASM mode (`execution_mode: wasm`)

- Business logic in `.wasm` only.
- **MUST** use WASI + Traverse Host ABI per **038**.
- **MUST NOT** import raw OS APIs.
- Default for ~90% of capabilities.

### Compatible mode (`execution_mode: compatible`)

- Business logic in platform module (Metal, DOM, etc.).
- **MUST** ship with a **thin wrapper** that maps runtime events ↔ capability only (no business rules in wrapper).
- Embedder **runs and kills** the compatible capability on runtime request.
- **MUST** declare `platforms[]` allowlist.
- **MUST NOT** add platform-specific methods to the embedder API; invocation goes through runtime → embedder lifecycle + JSON bridge defined in wrapper contract.

## Functional Requirements

- **FR-001**: Traverse MUST ship a WASM build of the orchestrator suitable for embedding in downstream apps.
- **FR-002**: Downstream apps MUST embed the orchestrator WASM; they MUST NOT require a user-started `traverse-cli serve` process in production.
- **FR-003**: Platform embedders MUST implement `embedder-api/1.0.0` without platform-specific operation additions.
- **FR-004**: Embedder implementations MUST pass the cross-platform conformance suite defined in FR-015.
- **FR-005**: WASM capabilities MUST execute under WASI sandbox via the embedded engine; embedder MUST NOT substitute custom file/network APIs for WASM modules.
- **FR-006**: Compatible capabilities MUST be declared in app manifests with `execution_mode: compatible` and `platforms[]`.
- **FR-007**: Compatible capabilities MUST include a wrapper that performs event wiring only.
- **FR-008**: Embedder MUST implement `compatible.start/stop/kill` for compatible capabilities.
- **FR-009**: Runtime MUST reject compatible capability invocation on platforms not listed in `platforms[]` with a deterministic error surfaced to the UI via `runtime.subscribe`.
- **FR-010**: UI shells MUST render runtime-provided output only; embedder and wrapper MUST NOT compute business fields.
- **FR-011**: Downstream apps MUST NOT import private Traverse internal crates; public embedder + runtime WASM artifacts only.
- **FR-012**: Build tooling MUST bundle manifests, WASM capability modules, and runtime WASM with content digests verified at build time.
- **FR-013**: Dev sidecar per **033** MAY be used for development and CI smoke tests; production builds MUST NOT depend on `.traverse/server.json` discovery.
- **FR-014**: `traverse-cli app validate` MUST validate `execution_mode`, `platforms`, wrapper paths, and compatible contracts in addition to existing **044** rules.
- **FR-015**: Traverse MUST provide `scripts/ci/embedder_conformance/` tests that every platform embedder MUST pass for a pinned embedder API version.
- **FR-016**: Embedded runtime MUST support app state machines per **052** and pipeline workflows per **058** without client-side polling state machines.
- **FR-017**: Embedded runtime event delivery to embedder `runtime.subscribe` MUST use the same JSON event shape as **033** SSE `data:` payloads where applicable, so UI clients can share parsing logic.
- **FR-018**: Compatible wrapper contracts MUST define input/output JSON schemas identical in shape to WASM capability contracts for UI rendering consistency.
- **FR-019**: On `runtime.shutdown`, embedder MUST kill all active compatible capabilities.

### Manifest extensions (044 additive)

- **FR-020**: Component manifest `execution_mode` MUST be `wasm` or `compatible`; default `wasm` when omitted for backward compatibility.
- **FR-021**: `execution_mode: wasm` REQUIRES `wasm_binary_path` and `wasm_digest`.
- **FR-022**: `execution_mode: compatible` REQUIRES `platforms`, `wrapper_path`, and `contract_path`; MUST NOT include `wasm_binary_path`.
- **FR-023**: App manifest workflows MAY reference capabilities from both execution modes in the same pipeline (058).
- **FR-024**: Validation MUST fail if a compatible component lacks a wrapper path or declares empty `platforms`.

## Non-Functional Requirements

- **NFR-001**: Embedder reference implementation per platform SHOULD target ≤500 LOC excluding generated bindings.
- **NFR-002**: Same bundled manifest + WASM inputs MUST produce deterministic capability output on the same platform build (same as sidecar).
- **NFR-003**: Embeddable runtime WASM artifact size SHOULD be tracked in release notes.

## Out of Scope

- Replacing **033** dev sidecar (still required for Phase 1/2 CI until embedder conformance lands in CI agents).
- Registry publication workflow (**056**, registry **002/007**).
- SSE over HTTP for production embedded apps (events use embedder `subscribe`; HTTP SSE remains dev sidecar transport per **033**).

## Acceptance Scenarios

1. **Given** a bundled traverse-starter app with embedded runtime, **When** the user submits a note without any sidecar running, **Then** output fields render from runtime events.
2. **Given** a compatible capability declared for iOS only, **When** the same manifest runs on Android, **Then** runtime returns a deterministic platform-unavailable error without crashing.
3. **Given** two platform embedders for the same embedder API version, **When** both run the conformance suite, **Then** both pass identical tests.

## Implementation Tickets

- Traverse **#553** — embeddable runtime SDK (implementation of this spec)
- reference-apps **#113–#117** — platform embedder integrations
