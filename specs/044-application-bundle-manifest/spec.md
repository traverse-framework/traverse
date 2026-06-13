# Feature Specification: Application Bundle Manifest

**Feature Branch**: `044-application-bundle-manifest`
**Created**: 2026-06-12
**Status**: Approved
**Input**: Downstream knowledge-app MVP requirements and approval decisions from Traverse team review on 2026-06-12. The downstream app provides an app manifest, concrete WASM component manifests, app configuration, and dependency declarations. Traverse validates, registers, executes, and exposes the app through public runtime, HTTP/JSON, and MCP surfaces.

## Purpose

This spec defines the first Traverse application bundle manifest model for downstream apps that ship real WASM business components and depend on Traverse as the governed runtime boundary.

The manifest model has two layers:

1. **Application manifest**: Declares the downstream app identity, workspace defaults, required workflows, concrete WASM component dependencies, selectable model dependency references, governed configuration schema, safe defaults, public surface needs, and placement policy.
2. **WASM component manifest**: Declares one executable component package, including capability identity, contract identity, WASM binary path and digest, runtime constraints, permitted targets, required dependencies, connectors, and validation evidence.

Pure business capabilities are concrete. If an app declares a WASM microservice component, Traverse MUST validate and execute that concrete component; Traverse MUST NOT substitute another implementation unless the app manifest is changed and revalidated.

AI/model dependencies are not owned by this spec. Application manifests MAY reference model dependency requirements, but selectable model candidate semantics, availability checks, and inference selection are governed by `045-governed-model-dependency-resolution`.

## User Scenarios and Testing

### User Story 1 - Validate a Complete App Bundle (Priority: P1)

As a downstream app developer, I want Traverse to validate one app manifest and its referenced WASM component manifests before registration so that invalid app bundles never enter a workspace.

**Why this priority**: Manifest validation is the entry point for all downstream app integration. Without it, Traverse cannot safely distinguish app-owned behavior from runtime-owned governance.

**Independent Test**: Submit an app manifest that references two valid WASM component manifests, capability contracts, and workflow definitions. Verify validation succeeds and returns a machine-readable readiness record.

**Acceptance Scenarios**:

1. **Given** an app manifest references valid component manifests and all referenced files exist, **When** Traverse validates the app bundle, **Then** validation succeeds and returns app id, app version, component ids, workflow ids, digests, and readiness status.
2. **Given** an app manifest references a missing component manifest, **When** Traverse validates the app bundle, **Then** validation fails with `app_component_manifest_missing` and identifies the missing reference.
3. **Given** a component manifest declares a WASM digest that does not match the referenced binary, **When** Traverse validates the app bundle, **Then** validation fails with `component_digest_mismatch` before registration.

### User Story 2 - Register an App Bundle Atomically (Priority: P1)

As a downstream app CLI, I want to register a complete app bundle through public Traverse APIs so that all capabilities, events, workflows, component manifests, and app metadata enter the workspace together or not at all.

**Why this priority**: Downstream apps need one reliable setup operation. Partial app registration would produce confusing runtime failures and break repeatable setup.

**Independent Test**: Register a complete app bundle through the public bundle registration path. Verify the response includes all artifact ids, versions, digests, execution links, and registration evidence. Re-register the same bundle and verify idempotency.

**Acceptance Scenarios**:

1. **Given** a valid app bundle, **When** the bundle is registered into a workspace, **Then** Traverse stores all app, component, capability, event, and workflow artifacts atomically and returns `201`.
2. **Given** the same app bundle is registered again unchanged, **When** registration is repeated, **Then** Traverse returns `200` with the same artifact ids and digests.
3. **Given** any artifact in the bundle is invalid, **When** registration is attempted, **Then** Traverse rejects the entire bundle and writes no partial registry state.

### User Story 3 - Generate App and Component Manifests With CLI (Priority: P1)

As an app author, I want `traverse-cli app new` and `traverse-cli component new` flows so that new Traverse apps and components start with governed manifest structure instead of ad hoc files.

**Why this priority**: Manifest authoring must be repeatable and discoverable. A first-class CLI flow prevents downstream apps from inventing incompatible structures.

**Independent Test**: Run `traverse-cli app new youaskm3` and `traverse-cli component new knowledge.retrieve`. Verify the generated files are schema-valid, contain no fake product behavior, and make registration fail until real executable component metadata is provided.

**Acceptance Scenarios**:

1. **Given** no app bundle exists, **When** a developer runs `traverse-cli app new youaskm3`, **Then** Traverse creates a repo-local app bundle directory with an app manifest, workspace config template, workflow directory, component reference directory, and bundle README.
2. **Given** a component id, **When** a developer runs `traverse-cli component new knowledge.retrieve`, **Then** Traverse creates a buildable component package structure with a component manifest and capability contract template but no fake product logic.
3. **Given** `--register` is passed to `app new`, **When** the generated bundle has no complete executable components, **Then** Traverse does not register the bundle and returns a clear validation result explaining which required executable artifacts are missing.

### User Story 4 - Merge Manifest Defaults With Workspace Config (Priority: P2)

As a local-first app operator, I want app manifests to declare governed defaults while workspace-local config supplies machine-specific values so that one app bundle can run in different local environments without committing private paths or secrets.

**Why this priority**: Downstream apps need portability and repeatable validation, but local paths, provider endpoints, user preferences, and browser origins differ by workspace.

**Independent Test**: Validate an app manifest with a workspace-local config file. Verify Traverse validates both schemas, applies the defined precedence rules, and returns an effective non-sensitive config summary.

**Acceptance Scenarios**:

1. **Given** an app manifest declares config schema and safe defaults, **When** workspace-local config provides allowed overrides, **Then** Traverse validates the merged config and records the effective non-sensitive values in readiness evidence.
2. **Given** workspace-local config tries to override an immutable manifest field, **When** validation runs, **Then** Traverse rejects the config with `app_config_immutable_override`.
3. **Given** workspace-local config contains a value that fails the manifest-declared schema, **When** validation runs, **Then** Traverse rejects the config with `app_config_invalid`.

## Edge Cases

- App manifest references a component id that is declared twice - fail validation with `duplicate_component_reference`.
- Component manifest references a capability contract whose id or version does not match the component declaration - fail with `component_contract_mismatch`.
- App manifest declares a pure WASM component dependency using a version range - fail with `component_dependency_must_be_concrete`.
- App manifest declares a model dependency without any candidates - defer to `045-governed-model-dependency-resolution` for candidate validation and report the delegated failure in app readiness evidence.
- Workspace-local config is absent - apply manifest defaults and fail only for required config values without defaults.
- Workspace-local config includes secrets - accept only in the local runtime config boundary and never include secret values in public traces or readiness summaries.
- App bundle registration is interrupted - either all artifacts are registered or none are registered; retry must be safe.
- App manifest references a workflow that references a missing component capability - fail at bundle validation before registration.
- App scaffold target directory already exists - fail unless an explicit force/update mode is provided by a later spec.
- Component scaffold creates build files but no product logic - generated component is not treated as executable until real implementation metadata and digest are supplied.

## Requirements

### Functional Requirements

- **FR-001**: Traverse MUST define an application manifest schema with at minimum: `app_id`, `version`, `schema_version`, `workspace_defaults`, `components`, `workflows`, `model_dependencies`, `config_schema`, `default_config`, `placement_policy`, and `public_surfaces`.
- **FR-002**: Traverse MUST define a WASM component manifest schema with at minimum: `component_id`, `version`, `capability_id`, `capability_version`, `contract_path`, `wasm_binary_path`, `wasm_digest`, `runtime_constraints`, `permitted_targets`, `dependencies`, `connector_requirements`, and `validation_evidence`.
- **FR-003**: Application manifest component dependencies for pure WASM microservices MUST be concrete references to component id, version, and digest; version ranges are not permitted for pure component substitution in this slice.
- **FR-004**: Traverse MUST validate that every component manifest referenced by an app manifest exists, is schema-valid, and references an existing capability contract.
- **FR-005**: Traverse MUST validate that each component manifest's capability id and version match the referenced capability contract.
- **FR-006**: Traverse MUST verify each referenced WASM binary digest before app bundle registration succeeds.
- **FR-007**: Traverse MUST validate workflow definitions referenced by the app bundle and ensure every workflow node references a concrete component-backed capability present in the bundle or already registered in the workspace.
- **FR-008**: Traverse MUST register app bundles atomically; any validation or registration failure MUST prevent all bundle artifacts from being written.
- **FR-009**: App bundle registration MUST be idempotent when the same app id, app version, component digests, workflow digests, and config schema are resubmitted unchanged.
- **FR-010**: App bundle registration responses MUST include machine-readable artifact ids, versions, digests, workspace id, registration status, readiness status, and execution/inspection links.
- **FR-011**: Traverse MUST support app manifest defaults plus workspace-local config overrides with explicit precedence rules: immutable manifest identity and component references cannot be overridden; environment-specific fields may be overridden only when declared overrideable.
- **FR-012**: Traverse MUST reject workspace-local config that attempts to override immutable manifest fields with `app_config_immutable_override`.
- **FR-013**: Traverse MUST never include secret workspace-local config values in public readiness evidence, public traces, or registration responses.
- **FR-014**: `traverse-cli app new <app-id>` MUST generate a repo-local app bundle structure with schema-valid manifest/config files and no fake product behavior.
- **FR-015**: `traverse-cli component new <component-id>` MUST generate a component package structure with manifest and contract files suitable for real WASM implementation, but the generated component MUST NOT be considered executable until real implementation metadata and digest are supplied.
- **FR-016**: `traverse-cli app new <app-id> --register --workspace <workspace-id>` MUST attempt registration only after validation; incomplete generated bundles MUST fail registration with a clear machine-readable validation result.
- **FR-017**: Application manifests MAY reference model dependencies, but candidate selection, availability checks, inference provider validation, and model placement heuristics MUST be governed by `045-governed-model-dependency-resolution`.
- **FR-018**: App bundle validation MUST produce readiness evidence that separates app manifest validity, component validity, workflow validity, config validity, and delegated model dependency readiness.
- **FR-019**: App bundle validation and registration MUST be exposed through public Traverse surfaces and MUST NOT require downstream apps to call private crate internals.
- **FR-020**: App bundle manifest and component manifest schemas MUST be versioned and governed by the spec-alignment gate.

### Non-Functional Requirements

- **NFR-001 Determinism**: Given the same app bundle files, workspace state, and workspace-local config, validation MUST produce the same result and ordering of validation messages.
- **NFR-002 Atomicity**: Bundle registration MUST never leave partial app, component, capability, event, or workflow state in the workspace.
- **NFR-003 Portability**: App manifests MUST remain portable across local environments by keeping machine-specific paths, provider endpoints, and secrets in workspace-local config.
- **NFR-004 Traceability**: Registration and validation evidence MUST identify the app manifest, component manifests, workflow definitions, digests, and effective non-sensitive config used.
- **NFR-005 Testability**: Manifest schema validation, component digest verification, config merge rules, idempotent registration, and CLI scaffold behavior MUST be independently testable.
- **NFR-006 Security**: Secret workspace-local config values MUST be excluded from public evidence and public traces by default.
- **NFR-007 Compatibility**: Manifest schema changes MUST follow semver-compatible evolution; breaking schema changes require a new major schema version and a governing spec update.

### Non-Negotiable Quality Standards

- **QG-001**: Pure WASM business component dependencies MUST be concrete and must not be silently substituted by Traverse.
- **QG-002**: App bundle registration MUST be atomic; partial registration is a blocking defect.
- **QG-003**: Component digest verification MUST run before registration; unchecked WASM binaries are not executable.
- **QG-004**: CLI scaffolding MUST NOT generate fake product behavior that can pass as a working downstream app capability.
- **QG-005**: Public readiness and trace evidence MUST NOT leak secret workspace-local configuration.

### Key Entities

- **Application Manifest**: The app-owned governed artifact declaring app identity, components, workflows, model dependencies, config schema, defaults, and public surface needs.
- **WASM Component Manifest**: The component-owned governed artifact declaring one concrete executable WASM package and its contract, digest, constraints, dependencies, and validation evidence.
- **Application Bundle**: The complete set of app manifest, component manifests, capability contracts, event contracts, workflow definitions, WASM binaries, and config schema submitted for validation or registration.
- **Workspace-Local Config**: Runtime-local configuration values for one workspace, including paths, local provider endpoints, browser origins, preferences, and secrets.
- **Effective App Config**: The validated merge of manifest defaults and workspace-local overrides, excluding secret values from public evidence.
- **App Readiness Evidence**: Machine-readable validation output describing whether the app bundle, components, workflows, config, and delegated model dependencies are ready.
- **Concrete Component Dependency**: A manifest reference to a specific component id, version, and digest that Traverse must execute without substitution.

## Success Criteria

- **SC-001**: A complete downstream app bundle can be validated from a clean checkout and produces machine-readable readiness evidence for app, component, workflow, config, and model dependency sections.
- **SC-002**: A valid app bundle can be registered atomically through public Traverse APIs; repeating the same registration is idempotent.
- **SC-003**: A bundle with any missing component manifest, invalid contract reference, digest mismatch, or invalid workflow fails before any partial registry state is written.
- **SC-004**: `traverse-cli app new` and `traverse-cli component new` generate schema-valid manifest structures without fake executable product behavior.
- **SC-005**: Workspace-local config can override declared environment-specific fields while immutable app/component identity fields remain protected.
- **SC-006**: Public registration/readiness output excludes secret values and includes all non-sensitive artifact ids, versions, digests, and validation outcomes needed by downstream apps.

## Assumptions

- Downstream apps own their product UI, source ingestion tooling, source content, and product release notes.
- Traverse owns validation, registration, execution, dependency governance, runtime traces, app-facing HTTP/JSON surfaces, and MCP-facing surfaces.
- Pure app business behavior is shipped as real WASM components with real manifests and digests; generated placeholders are not acceptable executable artifacts.
- Workspace-local config is runtime-owned generated or operator-authored state and is not a governed source artifact unless explicitly promoted.
- The first model dependency selection behavior is governed separately by `045-governed-model-dependency-resolution`.

## Issue Mapping

- [#446](https://github.com/enricopiovesan/Traverse/issues/446) - Define application and WASM component manifest schemas.
- [#447](https://github.com/enricopiovesan/Traverse/issues/447) - Implement atomic application bundle registration.
- [#448](https://github.com/enricopiovesan/Traverse/issues/448) - Add `app new` and `component new` CLI flows.
- [#449](https://github.com/enricopiovesan/Traverse/issues/449) - Validate workspace config merge and secret redaction.
- [#454](https://github.com/enricopiovesan/Traverse/issues/454) - Add downstream app MVP conformance suite across specs 044 and 045.

## Out of Scope

- Model candidate selection, model availability checks, and inference provider execution rules, except for references delegated to spec `045`.
- File conversion from PDF, DOCX, PPTX, or other user files into raw artifacts.
- Product UI layout, chat UX, graph visualization, source cards, or downstream release notes.
- Remote/cloud app bundle distribution and native installers.
- Automatic migration of existing ad hoc downstream app manifests into this schema.
- Dynamic substitution of pure WASM business components at runtime.
