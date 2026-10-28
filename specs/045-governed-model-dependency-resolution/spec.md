# Feature Specification: Governed Model Dependency Resolution

**Feature Branch**: `045-governed-model-dependency-resolution`
**Created**: 2026-06-12
**Status**: Approved — `exact-ref.wasm-cpu` candidate kind amend approved 2026-09-17; v1.1.0 registry rights and revocation policy amend (#1728, registry spec 026) approved 2026-10-27
**Input**: Downstream knowledge-app MVP requirements and Traverse team approval decisions from 2026-06-12. Model inference must be real, not placeholder behavior. Model dependencies are selectable candidates declared by the app manifest, resolved by Traverse using availability, basic model fit, and app priority.

## Purpose

This spec promotes model dependencies from documentation-only declarations into governed runtime dependencies for real local inference.

Pure business capabilities remain concrete WASM components governed by `044-application-bundle-manifest`. Model dependencies are different: the app manifest may provide multiple concrete model/provider candidates for one abstract inference interface, and Traverse decides which candidate to use using governed, traceable heuristics.

The first real provider path is local inference through an Ollama-backed capability implementation unless a later approved spec replaces that provider. The downstream app must not hardcode Ollama, llama.cpp, WebLLM, a cloud API, or any provider-specific path in product logic. Provider-specific behavior belongs behind Traverse-governed inference capability implementations and connectors.

No deterministic fake, placeholder, or stub inference implementation satisfies this spec.

## User Scenarios and Testing

### User Story 1 - Resolve a Real Local Model Candidate (Priority: P1)

As a downstream app, I want to declare multiple concrete model candidates for an inference interface so that Traverse can select a real available local model without the app owning provider-specific logic.

**Why this priority**: Real model selection is the central MVP gap. Without it, the app cannot honestly say Traverse owns inference execution.

**Independent Test**: Provide an app manifest with an inference dependency containing multiple Ollama-backed candidates. With one candidate installed and available locally, validate that Traverse selects that candidate and records the decision.

**Acceptance Scenarios**:

1. **Given** an app manifest declares `traverse.inference.generate` with multiple concrete candidates, **When** Traverse validates model readiness, **Then** it checks candidate availability and model fit and selects the highest-priority passing candidate.
2. **Given** the highest-priority candidate is unavailable but a lower-priority candidate is available and fits, **When** Traverse resolves the model dependency, **Then** it selects the lower-priority passing candidate and records why the first candidate was rejected.
3. **Given** no candidate is available or fit, **When** Traverse resolves the dependency, **Then** it fails with `model_dependency_unsatisfied` before user-facing execution begins.

### User Story 2 - Validate Model Readiness During App Setup (Priority: P1)

As a local-first app operator, I want app bundle registration/setup validation to report model provider and model readiness so that users discover missing local model setup before asking a question.

**Why this priority**: Readiness evidence is necessary for a practical local MVP. Setup must reveal provider/model gaps early.

**Independent Test**: Validate an app bundle when the local provider is not running, when the provider is running but the model is missing, and when the model is installed. Verify distinct readiness outcomes.

**Acceptance Scenarios**:

1. **Given** the local provider is not reachable, **When** setup validation runs, **Then** readiness evidence reports `model_provider_unavailable`.
2. **Given** the provider is reachable but a declared model is not installed, **When** setup validation runs, **Then** readiness evidence reports `model_candidate_unavailable`.
3. **Given** a declared model is reachable and satisfies basic fit requirements, **When** setup validation runs, **Then** readiness evidence reports the candidate as ready.

### User Story 3 - Revalidate Model Availability at Execution Time (Priority: P1)

As a runtime user, I want Traverse to re-check model availability immediately before inference so that a model removed or stopped after registration cannot cause ambiguous failures.

**Why this priority**: Registration-time readiness is not enough. Local model availability can change between setup and execution.

**Independent Test**: Validate an app bundle successfully, then make the selected model unavailable before execution. Verify execution fails with a stable model dependency error and a trace-ready failure classification.

**Acceptance Scenarios**:

1. **Given** app setup validation previously succeeded, **When** the selected provider is stopped before execution, **Then** Traverse revalidates and fails with `model_provider_unavailable`.
2. **Given** the selected model is removed before execution but another candidate is available and fits, **When** execution begins, **Then** Traverse may select the available passing candidate according to policy and records the switch in trace evidence.
3. **Given** no candidate passes execution-time revalidation, **When** inference would begin, **Then** Traverse fails with `model_dependency_unsatisfied` before invoking downstream product logic that depends on inference.

### User Story 4 - Record Public Trace Evidence for Model Selection (Priority: P2)

As a UI or auditor, I want public trace evidence to show which model/provider was selected and why so that answer production remains explainable without exposing private prompts or secret config.

**Why this priority**: Model selection is a runtime decision. It must be visible enough for downstream UI and audit views without leaking sensitive content.

**Independent Test**: Execute a workflow that resolves an inference dependency. Fetch the public trace and verify it includes interface requested, candidates evaluated, selected candidate, placement, and non-sensitive rejection reasons.

**Acceptance Scenarios**:

1. **Given** model resolution succeeds, **When** the public trace is fetched, **Then** it includes requested inference interface, evaluated candidates, selected provider, selected model, selected placement, and selection reason.
2. **Given** model resolution fails, **When** the public trace or failure envelope is inspected, **Then** it includes stable failure code and non-sensitive candidate rejection reasons.
3. **Given** workspace-local provider config contains sensitive values, **When** trace evidence is produced, **Then** secret values are omitted from public trace output.

## Edge Cases

- Provider endpoint is configured but unreachable - setup readiness reports `model_provider_unavailable`; execution revalidation fails with the same code if no alternate candidate passes.
- Provider is reachable but model is not installed - report `model_candidate_unavailable`.
- Model is available but does not satisfy required context window - reject candidate with `model_context_window_insufficient`.
- Model is available but does not support the requested interface - reject candidate with `model_interface_unsupported`.
- App manifest lists duplicate candidate ids for the same interface and model - fail readiness validation with `duplicate_model_candidate`.
- Candidate has invalid provider-specific configuration - fail with `model_candidate_config_invalid`.
- Candidate priority ties after filtering - resolve deterministically by manifest order and record the tie in trace evidence.
- Setup validation succeeds but execution-time model availability changes - re-run resolution and record whether selection changed.
- Workspace-local provider config includes secrets - use them only in runtime-local execution and never emit them in public trace/readiness evidence.
- A fake or placeholder inference capability is registered - it cannot satisfy this spec unless it invokes a real model provider and produces real inference output.

## Requirements

### Functional Requirements

- **FR-001**: Traverse MUST define a governed model dependency declaration for app manifests containing at minimum: `interface_id`, `version_range`, `selection_policy`, `required_capabilities`, `minimum_context_window`, and `candidates`.
- **FR-002**: Each model candidate MUST declare at minimum: candidate id, provider capability id, provider implementation id, model identifier, placement target, priority, required provider config keys, and non-sensitive model metadata.
- **FR-002a** *(Decision 95 amend)*: A candidate MAY declare `provider_implementation_id: "exact-ref.wasm-cpu"` instead of a native provider id. Such a candidate MUST reference an existing `044-application-bundle-manifest` `exact_model_dependencies` pin by `model_id` and `version` in place of its own model identifier/digest — it MUST NOT declare an independent digest. Resolution MUST fail closed with `model_dependency_unsatisfied` if the referenced pin's declared `traverse.model-runtime` connector binding is not activated. Execution of an `exact-ref.wasm-cpu` candidate is delegated entirely to `138-governed-exact-model-execution`'s existing `model.execute` path; this spec governs only candidate declaration and selection, never the verified-execution mechanism itself.
- **FR-003**: Model dependencies MUST be satisfied by real governed inference capability implementations; fake, placeholder, or documentation-only implementations MUST NOT satisfy readiness or execution.
- **FR-004**: The first approved provider path MUST support a real local Ollama-backed inference implementation behind a Traverse-governed inference capability interface.
- **FR-005**: Downstream apps MUST depend on the abstract inference interface and candidate set; downstream app product code MUST NOT hardcode provider-specific inference calls.
- **FR-006**: Traverse MUST evaluate model candidates using this MVP heuristic order: provider/model availability, requested inference interface support, placement policy, minimum context window, then app-declared priority.
- **FR-006a** *(Decision 95 amend)*: Candidate availability evaluation MUST be platform-aware: a candidate whose provider implementation cannot execute on the current placement target (for example, an `ollama.local.generate` candidate under a wasm32/browser placement target) MUST be treated as unavailable and skipped in favor of the next passing candidate, rather than surfacing as an execution-time failure.
- **FR-007**: If all filtered candidates fail, Traverse MUST return `model_dependency_unsatisfied` with non-sensitive rejection reasons for each candidate.
- **FR-008**: Traverse MUST perform model readiness checks during app setup or bundle validation and include readiness results in app readiness evidence.
- **FR-009**: Traverse MUST revalidate model availability and basic fit at execution time before invoking inference.
- **FR-010**: Execution-time revalidation MAY select a different passing candidate than setup validation when the original candidate is unavailable and policy permits fallback; the trace MUST record the change.
- **FR-011**: Candidate selection MUST be deterministic for the same app manifest, workspace-local config, provider state, and model metadata.
- **FR-012**: Public trace evidence MUST include requested interface, evaluated candidates, rejected candidate reasons, selected provider, selected model, selected placement, and selection reason.
- **FR-012a** *(Decision 95 amend)*: Public trace evidence (and `traverse.inference.generate` response evidence) MUST include a `trust_class` value of `verified_signed_package` (for `exact-ref.wasm-cpu` candidates) or `local_unverified_daemon` (for `ollama.local.generate` candidates), populated regardless of which candidate kind was selected, so callers can distinguish signed-and-verified execution from local-daemon execution without inspecting provider-implementation-specific strings. This is a breaking change to the previously-published evidence shape; `traverse.inference.generate` bumps `1.0.0` → `2.0.0`.
- **FR-013**: Public trace evidence MUST NOT include private prompts, private source text, secret provider configuration, or raw model credentials.
- **FR-014**: Missing provider, missing model, unsupported interface, insufficient context window, invalid candidate config, and unsatisfied model dependency failures MUST each have stable machine-readable error codes.
- **FR-015**: Model readiness evidence MUST distinguish setup validation failures from execution-time availability failures.
- **FR-016**: Model candidate metadata MUST be versioned and validated as part of the app manifest and workspace-local config merge governed by `044-application-bundle-manifest`.
- **FR-017**: The selected inference implementation and placement MUST be visible through both HTTP/JSON execution trace retrieval and MCP execution report surfaces when the workflow is exposed through both.
- **FR-018**: Model dependency resolution MUST not require downstream app code changes when a new compatible provider implementation is added to the candidate set and app policy permits it.
- **FR-019**: Inference provider HTTP requests to an operator-configured (and possibly untrusted or remote) endpoint MUST bound the **entire** exchange, not only the TCP connect: read and write timeouts MUST be applied for the connected socket so that a peer which accepts and then stalls cannot block the calling thread past the configured request timeout. A timed-out read or write MUST surface as `model_provider_unavailable`.
- **FR-020**: The provider response read MUST be bounded by a configurable maximum size with a safe default; a response exceeding the limit MUST fail with a stable error code rather than buffering unbounded data into memory.
- **FR-021** *(v1.1.0, #1728)*: Range or `selection_policy` resolution of a public registry dependency MUST skip any version whose lifecycle status is `revoked` (the index `status` from registry spec 026, read through `PublicRegistryCapabilityRecord::lifecycle_status`), the same way it skips deprecated versions. If every version matching the range is revoked, resolution MUST fail with `registry_dependency_revoked`, and the message MUST list the revoked versions.
- **FR-022** *(v1.1.0, #1728)*: An exact pin (`=X.Y.Z`) to a revoked version MUST still resolve to that record (registry decision-log entry 127 Q11; registry spec 005 SC-005, pins never break). Before the record is prepared, registered, or activated, the consumer MUST then refuse it with the do-not-run code `registry_dependency_revoked`, carrying the revocation record (reason, `revoked_at`, evidence URL). The default policy is fail closed, and no host option re-enables a revoked version.
- **FR-023** *(v1.1.0, #1728)*: If any object-shaped `ai.models` entry on the resolved record declares `conditional` for `commercial_use`, `redistribution`, or `derivatives`, the consumer MUST deny the record by default with `model_rights_conditional_denied`. A host MAY allow such records only through an explicit, host-owned policy opt-in: `ModelRightsPolicy { allow_conditional: true }` in the embedder and runtime API, or `--allow-conditional-model-rights` on `traverse-cli app validate|register|activate`. There is no implicit default and no network lookup. A declared `unknown` right MUST always be denied with `model_rights_unknown_denied`. `allowed` and `forbidden` values are not denied by this policy, because their use limits are the app's responsibility. Model references that predate spec 026 (legacy strings, or objects without rights fields) are grandfathered and pass.
- **FR-024** *(v1.1.0, #1728)*: Deprecated versions keep their existing behavior. The policy decision MUST be deterministic: the same record and policy give the same code and message. Checks run in a fixed order: revocation first, then each model in declaration order, checking `commercial_use`, `redistribution`, then `derivatives`. The decision is reported through the existing structured error evidence of the surface that applied it: the CLI app JSON errors, `RegistryCacheError` in the embedder, and MCP prepare.

### Non-Functional Requirements

- **NFR-001 Determinism**: Candidate filtering and priority selection MUST produce the same result for the same manifest, config, provider state, and model metadata.
- **NFR-002 Explainability**: Every selected or rejected candidate MUST have a non-sensitive reason suitable for UI readiness and public trace views.
- **NFR-003 Local-First Operation**: The first approved model provider path MUST work without paid external services.
- **NFR-004 Privacy**: Public traces and readiness evidence MUST not reveal private prompts, private source text, secrets, or credentials.
- **NFR-005 Portability**: Provider-specific implementation details MUST remain behind Traverse-governed inference capability implementations and not leak into downstream app product logic.
- **NFR-006 Testability**: Availability checks, candidate filtering, context-window filtering, priority selection, fallback behavior, and trace evidence generation MUST be independently testable.
- **NFR-007 Compatibility**: Inference interface and candidate schema changes MUST follow semver-compatible evolution.

### Non-Negotiable Quality Standards

- **QG-001**: No fake, placeholder, or documentation-only inference implementation may satisfy this spec.
- **QG-002**: The downstream app MUST NOT own provider-specific model invocation logic.
- **QG-003**: Model availability MUST be checked during setup/readiness validation and rechecked at execution time.
- **QG-004**: Selection trace evidence MUST identify selected provider/model/placement and rejected candidate reasons without leaking private data.
- **QG-005**: Missing or unsuitable model dependencies MUST fail before user-facing answer execution begins.

### Key Entities

- **Model Dependency Declaration**: App manifest requirement for an abstract inference interface plus candidate set and selection policy.
- **Inference Interface**: A stable governed capability contract such as `traverse.inference.generate` that downstream workflows depend on.
- **Model Candidate**: One concrete provider/model option that can satisfy an inference interface if available and fit.
- **Provider Implementation**: A governed inference capability implementation, such as an Ollama-backed local provider.
- **Model Readiness Evidence**: Setup-time validation output showing provider availability, model availability, interface support, context fit, and candidate selection readiness.
- **Model Resolution Trace**: Execution-time trace evidence showing evaluated candidates, selected candidate, placement, and reasons.
- **Basic Model Fit**: MVP candidate filters for requested interface, placement policy, and minimum context window.
- **Exact-Ref Candidate** *(Decision 95 amend)*: A model candidate whose `provider_implementation_id` is `exact-ref.wasm-cpu`. It references an existing Spec 044 `exact_model_dependencies` pin instead of declaring its own model identity, and its execution is delegated entirely to Spec 138's `model.execute`. It carries `trust_class: verified_signed_package` in evidence, versus `local_unverified_daemon` for Ollama-backed candidates.

## Success Criteria

- **SC-001**: A downstream app manifest can declare multiple real local model candidates for an inference interface, and Traverse selects the highest-priority available candidate that satisfies basic fit.
- **SC-002**: Setup validation distinguishes provider unavailable, model unavailable, unsupported interface, insufficient context, and ready candidates with stable machine-readable codes.
- **SC-003**: Execution revalidates model availability and either selects a passing candidate or fails before inference-dependent product behavior begins.
- **SC-004**: Public traces and MCP execution reports identify selected model/provider/placement and non-sensitive rejection reasons.
- **SC-005**: Downstream app product logic remains provider-neutral while Traverse invokes a real local inference implementation.
- **SC-006**: No fake or placeholder inference implementation can pass readiness or execution validation for this spec.

## Assumptions

- Application bundle manifest structure, workspace-local config merge, and concrete WASM component dependencies are governed by `044-application-bundle-manifest`.
- The first real local provider path is Ollama-backed because it provides the shortest source-build developer path to real local inference.
- Later providers such as llama.cpp, WebLLM, or cloud APIs may be added as compatible implementations of the same inference interface under future approved specs.
- Model quality ranking and benchmark-based routing are not required for the first MVP heuristic.
- Downstream source ingestion, artifact construction, retrieval indexes, and UI rendering remain owned by the downstream app unless wrapped as governed Traverse components.

## Issue Mapping

- [#450](https://github.com/enricopiovesan/Traverse/issues/450) - Define governed inference interface and model candidate schema.
- [#451](https://github.com/enricopiovesan/Traverse/issues/451) - Implement real local Ollama inference provider capability.
- [#452](https://github.com/enricopiovesan/Traverse/issues/452) - Resolve model dependencies at setup and execution time.
- [#453](https://github.com/enricopiovesan/Traverse/issues/453) - Expose model selection evidence in traces and MCP reports.
- [#454](https://github.com/enricopiovesan/Traverse/issues/454) - Add downstream app MVP conformance suite across specs 044 and 045.
- [#1454](https://github.com/traverse-framework/traverse/issues/1454) - Bridge Spec 045↔138: `exact-ref.wasm-cpu` candidate kind, `trust_class` evidence, platform-aware resolution (Decision 95 amend).
- [#1728](https://github.com/traverse-framework/traverse/issues/1728) - Fail closed on revoked registry versions; deny `conditional` model rights by default (FR-021–FR-024).
- [#1455](https://github.com/traverse-framework/traverse/issues/1455) - Text-generating exact-ref fixture proving the bridge end-to-end.

## Out of Scope

- Fake, placeholder, stub, or documentation-only inference behavior.
- Full benchmark-based model routing and historical performance learning.
- Cloud/server model execution as a first provider path.
- Browser-native WebLLM provider execution.
- Model weight distribution, native installers, or package-manager installation.
- Embedding-specific interfaces unless added by a later approved spec.
- Product-level answer quality scoring beyond model selection and traceability.
- Exact-ref signed WASM model package execution, host-staged tensor I/O,
  digest verification, and CPU-WASM conformance itself — those remain
  entirely governed by `138-governed-exact-model-execution` (Decision 91)
  and are never satisfied by this spec. The `exact-ref.wasm-cpu` candidate
  kind (Decision 95, FR-002a) only lets an app *declare and select* such a
  pin as one candidate among others through this spec's resolution
  machinery; it does not reopen or duplicate Spec 138's execution DoD.
