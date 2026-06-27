# Feature Specification: Public CLI App Registration

**Feature Branch**: `046-public-cli-app-registration`
**Created**: 2026-06-25
**Status**: Approved
**Input**: Downstream `youaskm3` MVP blocker in Traverse issue #475. The downstream app can produce real WASM component artifacts and non-placeholder digests, but it needs a public Traverse CLI setup surface to validate and register its app manifest into durable local workspace state without depending on private Traverse crate internals.

## Purpose

This spec defines the public CLI registration surface for downstream application manifests.

The feature completes the local setup boundary promised by `044-application-bundle-manifest`: a downstream app can hand Traverse an application manifest, validate it, register the app bundle into a named local workspace, and receive machine-readable setup evidence. The registered workspace state must then be usable by Traverse runtime processes that load the same workspace.

This is a local development and setup surface. It does not make Traverse runtime responsible for HTTP app registration, remote administration, deployment orchestration, UI hosting, or service registry discovery. UI clients and other Traverse runtimes communicate with Traverse through governed runtime and eventing surfaces, not by owning registration.

## User Scenarios and Testing

### User Story 1 - Validate A Downstream App Manifest From CLI (Priority: P1)

As a downstream app developer, I want a public `traverse-cli app validate` command so that my app setup script can prove its application manifest, component manifests, workflow definitions, WASM digests, model dependencies, runtime constraints, and public surface declarations are valid before registration.

**Why this priority**: Downstream apps must not rely on private Traverse crates or ad hoc test harnesses to determine whether an app bundle is valid.

**Independent Test**: Run the command against a checked-in downstream app manifest with real component manifests and WASM digests. Verify that the command exits successfully and returns deterministic JSON evidence with app id, app version, component ids, workflow ids, digest status, model readiness summary, and validation status.

**Acceptance Scenarios**:

1. **Given** a valid downstream app manifest, component manifests, workflow definitions, contracts, and WASM binaries, **When** `traverse-cli app validate --manifest <path> --json` runs, **Then** Traverse returns machine-readable validation evidence with status `validated`.
2. **Given** a missing component manifest, **When** validation runs, **Then** Traverse fails with a stable machine-readable error code and identifies the missing path.
3. **Given** a placeholder or all-zero WASM digest, **When** validation runs, **Then** Traverse rejects the bundle before registration evidence can be produced.

### User Story 2 - Register An App Bundle Into Durable Local Workspace State (Priority: P1)

As a downstream setup script, I want a public `traverse-cli app register` command so that a valid app bundle is recorded in a named local workspace and later loadable by Traverse runtime processes.

**Why this priority**: `youaskm3` cannot complete its local Traverse-backed happy path unless registration survives the CLI process and becomes visible to runtime execution.

**Independent Test**: Register a valid app bundle into workspace `local`, restart or separately invoke the runtime against the same workspace state, and verify that the registered workflow and component-backed capabilities are discoverable without re-reading private downstream implementation details.

**Acceptance Scenarios**:

1. **Given** a valid app manifest, **When** `traverse-cli app register --manifest <path> --workspace local --json` runs, **Then** Traverse writes durable local workspace registration state and returns status `registered`.
2. **Given** the same unchanged app bundle is registered again, **When** registration repeats, **Then** Traverse returns an idempotent success with status `already_registered` or equivalent evidence and no duplicate artifacts.
3. **Given** any artifact in the bundle is invalid, **When** registration is attempted, **Then** Traverse fails atomically and leaves no partial app, component, capability, event, workflow, or model readiness state in the workspace.

### User Story 3 - Load Registered Workspace State For Runtime Use (Priority: P1)

As a local Traverse operator, I want runtime processes to load the durable workspace state produced by CLI registration so that UI clients can communicate with the runtime through governed eventing and execution surfaces after setup.

**Why this priority**: CLI registration is only useful if the registered workspace state can be consumed by the runtime that executes app workflows.

**Independent Test**: Register a downstream app bundle with the CLI, start or invoke a runtime process configured for the same workspace, and verify the registered workflow can be addressed without calling private registration internals.

**Acceptance Scenarios**:

1. **Given** a workspace contains a registered downstream app bundle, **When** Traverse runtime loads that workspace, **Then** the app workflows and component-backed capabilities are discoverable through public runtime surfaces.
2. **Given** a UI client communicates through the approved eventing path, **When** it submits a query event for a registered workflow, **Then** Traverse can route execution using registered workspace artifacts rather than downstream harness shortcuts.
3. **Given** workspace state is missing or corrupt, **When** runtime loading occurs, **Then** Traverse fails predictably with stable diagnostic evidence.

### User Story 4 - Provide Stable Setup Evidence For Downstream Automation (Priority: P2)

As a downstream CI or local setup script, I want validation and registration outputs to be machine-readable and stable so that blocked downstream issues can distinguish real setup failures from missing Traverse surface failures.

**Why this priority**: `youaskm3` needs reliable setup evidence for its MVP gates and should not parse human-only text.

**Independent Test**: Run validate/register with `--json` for success, validation failure, digest failure, unsupported version, and partial-registration failure cases. Verify all outputs contain stable codes and predictable fields.

**Acceptance Scenarios**:

1. **Given** validation succeeds, **When** JSON output is requested, **Then** the response includes app id/version, workspace id when applicable, component ids, workflow ids, digest verification results, model readiness summary, and trace/discovery references.
2. **Given** validation fails, **When** JSON output is requested, **Then** the response includes `status: failed`, stable error codes, artifact paths, and actionable messages without leaking secrets.
3. **Given** workspace-local config includes secret values, **When** validation or registration evidence is produced, **Then** public output redacts or omits secret values.

## Edge Cases

- App manifest path is missing or unreadable: fail with a stable manifest read error before any workspace state changes.
- App manifest declares a minimum Traverse version greater than the running CLI version: fail with `unsupported_traverse_version`.
- App manifest contains unsupported private/internal fields: fail clearly rather than ignoring them.
- Component ids are duplicated: fail validation before registration.
- Component manifest and app manifest disagree on component id, version, capability id, or digest: fail validation before registration.
- Capability contract id/version does not match the component manifest: fail validation before registration.
- WASM binary is missing, unreadable, digest-mismatched, placeholder, or all-zero: fail validation before registration.
- Workflow nodes reference missing or unregistered capabilities: fail validation before registration.
- Model dependency declarations are malformed or unsupported: fail validation through the delegated `045` readiness path.
- Workspace-local config attempts to override immutable manifest fields: fail with the existing config override error.
- Registration is interrupted: retry must be safe and must not observe partial committed workspace state.
- Runtime starts before registration: it must report missing app/workflow state rather than inventing fallback behavior.
- Downstream app wants HTTP registration or service discovery: create a separate future spec; this slice does not own it.

## Requirements

### Functional Requirements

- **FR-001**: Traverse MUST expose `traverse-cli app validate --manifest <path> --json` as a public command for application bundle validation.
- **FR-002**: Traverse MUST expose `traverse-cli app register --manifest <path> --workspace <workspace-id> --json` as a public command for local workspace app registration.
- **FR-003**: CLI validation MUST reuse the governed application manifest, component manifest, workflow, digest, workspace config, and delegated model readiness rules from specs `044` and `045`.
- **FR-004**: CLI registration MUST validate before writing workspace state.
- **FR-005**: CLI registration MUST write durable local workspace state that can be loaded by a separate Traverse runtime process configured for the same workspace.
- **FR-006**: CLI registration MUST be atomic; validation or write failures MUST leave no partial committed app registration state.
- **FR-007**: CLI registration MUST be idempotent for unchanged app id, app version, manifest digest, component digests, workflow digests, config schema, and model dependency declarations.
- **FR-008**: CLI validation and registration JSON success output MUST include app id, app version, workspace id when applicable, component ids/versions, workflow ids/versions, digest verification results, model readiness summary, non-sensitive effective config summary, and public runtime/eventing references needed by downstream setup tools.
- **FR-009**: CLI validation and registration JSON failure output MUST include `status`, stable error codes, paths or artifact ids, severity, and actionable messages.
- **FR-010**: CLI validation and registration MUST reject placeholder, all-zero, malformed, or mismatched WASM digests.
- **FR-011**: CLI validation and registration MUST reject unsupported private/internal app manifest fields clearly.
- **FR-012**: Runtime workspace loading MUST consume the durable app registration state produced by CLI registration without requiring downstream apps to call private Traverse crate internals.
- **FR-013**: Runtime workspace loading MUST make registered app workflows and component-backed capabilities discoverable through existing public runtime, workflow, trace, MCP, and eventing surfaces where those surfaces already exist.
- **FR-014**: Public output MUST NOT expose secret workspace-local config values.
- **FR-015**: The CLI help and docs MUST identify validate/register as public setup surfaces and MUST describe their relationship to event-driven UI/runtime communication.
- **FR-016**: The `youaskm3` downstream registration script MUST be able to replace `MISSING_PUBLIC_APP_REGISTRATION_SURFACE` with real validation or registration evidence when pointed at this Traverse surface.

### Non-Functional Requirements

- **NFR-001 Determinism**: Given the same app bundle files, workspace state, and workspace-local config, validation and registration evidence MUST be stable in ordering and content except for explicit timestamps or generated ids.
- **NFR-002 Locality**: This feature MUST work for local development and local workspace setup without requiring a remote service, service registry, or HTTP app registration endpoint.
- **NFR-003 Portability**: Registration MUST keep deployment, UI hosting, and transport choices outside the app manifest registration responsibility.
- **NFR-004 Traceability**: Registration evidence MUST identify the app manifest, component manifests, workflows, digests, validator version, workspace id, and governing specs used.
- **NFR-005 Security**: Secret workspace config values MUST be omitted or redacted in all public validation, registration, trace, and setup evidence.
- **NFR-006 Compatibility**: The public CLI surface MUST evolve compatibly within the `0.x` release line; breaking shape changes require explicit release notes and a successor spec or version.
- **NFR-007 Testability**: The feature MUST include deterministic tests for success, idempotency, atomic failure, digest rejection, unsupported version, runtime load, and downstream conformance.

### Non-Negotiable Quality Standards

- **QG-001**: No downstream app may be required to depend on private Traverse crate internals for app registration.
- **QG-002**: No HTTP app registration endpoint is part of this slice.
- **QG-003**: No Traverse runtime-owned admin/control-plane registration API is part of this slice.
- **QG-004**: No service registry, remote runtime discovery, deployment orchestration, or UI hosting responsibility is part of this slice.
- **QG-005**: CLI registration that writes only process-local in-memory state is insufficient.
- **QG-006**: Runtime execution must use registered workspace artifacts and must not fall back to fake workflows, placeholder manifests, browser demo paths, or downstream runtime shortcuts.

### Key Entities

- **Public CLI App Validation Surface**: The stable `traverse-cli app validate` command that verifies a downstream app bundle and emits readiness evidence.
- **Public CLI App Registration Surface**: The stable `traverse-cli app register` command that validates and records a downstream app bundle into durable local workspace state.
- **Durable Local Workspace State**: The local state store or artifact set that records app, component, capability, workflow, model readiness, and config evidence for later runtime loading.
- **Runtime Workspace Loader**: The runtime-facing loading behavior that consumes durable workspace state and makes registered artifacts available for execution and discovery.
- **Registration Evidence**: Machine-readable output that records validation status, registration status, artifact ids, digests, workspace id, model readiness, and public runtime/eventing references.
- **Downstream Setup Script**: A downstream-owned CLI automation path, such as `youaskm3/scripts/register-traverse-app.sh`, that invokes Traverse public CLI commands.

## Success Criteria

- **SC-001**: A downstream app can validate a complete app manifest from a clean checkout using only a public Traverse CLI command and receives deterministic JSON evidence.
- **SC-002**: A downstream app can register a complete app manifest into workspace `local` using only a public Traverse CLI command and receives deterministic JSON registration evidence.
- **SC-003**: Registered app workspace state survives the CLI process and can be loaded by a separate Traverse runtime process configured for the same workspace.
- **SC-004**: Re-registering an unchanged app bundle is idempotent and does not duplicate app, component, capability, workflow, or model readiness records.
- **SC-005**: Invalid manifests, missing artifacts, placeholder digests, digest mismatches, invalid workflows, unsupported versions, and malformed model dependencies fail before any partial registration state is committed.
- **SC-006**: `youaskm3` can replace `MISSING_PUBLIC_APP_REGISTRATION_SURFACE` with real validation or registration evidence while keeping UI/runtime communication event-driven.

## Assumptions

- The downstream app owns UI deployment, document ingestion, artifact preparation, product copy, and browser/PWA behavior.
- Traverse owns governed validation, local workspace registration, runtime loading, workflow execution, model readiness evidence, traces, and eventing surfaces.
- The first public app registration surface is CLI/local setup only.
- A future service registry may be considered separately, but it is not implied or required by this spec.
- Existing HTTP/JSON execution and trace surfaces may remain available for local development, but they are not app registration endpoints and are not the architecture boundary for UI communication.

## Issue Mapping

- [#475](https://github.com/enricopiovesan/Traverse/issues/475) - Add public app registration surface for downstream app manifests.

## Out of Scope

- HTTP app registration endpoint.
- Runtime-owned admin API or control-plane registration surface.
- Service registry, remote runtime discovery, and cross-runtime registration propagation.
- Deployment orchestration, hosting, installers, or package manager distribution.
- UI communication design changes beyond preserving event-driven runtime communication.
- Downstream app implementation, document ingestion, markdown conversion, source artifact generation, UI rendering, or product fixtures.
- Provider-specific inference product logic in downstream apps.
