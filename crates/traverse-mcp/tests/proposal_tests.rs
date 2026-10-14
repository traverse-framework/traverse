//! End-to-end MCP tests for the runtime workflow proposal lifecycle.
//!
//! Governed by spec `109-runtime-workflow-proposals`. Exercises the full
//! submit -> validate -> authorization-state -> execute -> observe -> export
//! path through `traverse_mcp::tools::proposals`, covering accepted, denied,
//! invalid, exhausted, and failed outcomes end to end.

use ed25519_dalek::{Signer, SigningKey};
use serde_json::{Value, json};
use std::collections::HashMap;

use traverse_contracts::{
    BinaryFormat as ContractBinaryFormat, CapabilityContract, DataFlowPolicy, DeterminismClass,
    EffectClass, Entrypoint, EntrypointKind, Execution, ExecutionConstraints, ExecutionTarget,
    FilesystemAccess, HostApiAccess, Lifecycle, NetworkAccess, Owner, ProposalLimits,
    ReliabilityMetadata, RiskMetadata, SchemaContainer, ServiceType, SideEffect, SideEffectKind,
};
use traverse_mcp::tools::proposals::{
    AuthorizationState, ProposalExecutionRequest, ProposalExecutionResponse, authorization_state,
    execute_proposal_via_mcp, export_proposal, observe_proposal, submit_proposal,
    validate_proposal,
};
use traverse_registry::{
    ApplicationBundleManifest, ApplicationComponent, ApplicationComponentRef,
    ApplicationEffectiveConfig, ArtifactDigests, BinaryFormat as RegistryBinaryFormat,
    BinaryReference, CapabilityArtifactRecord, CapabilityRegistration, CapabilityRegistry,
    ComponentExecutionMode, ComposabilityMetadata, CompositionKind, CompositionPattern,
    ImplementationKind, RegistryProvenance, RegistryScope, SourceKind, SourceReference,
    WasmComponentManifest,
};
use traverse_runtime::proposal::{
    ApprovalTokenStore, ProposalNodeStatus, ProposalTerminalState, QuotaLimits, QuotaTracker,
};
use traverse_runtime::security::RuntimeSecurityConfig;
use traverse_runtime::{
    LocalExecutionFailure, LocalExecutionFailureCode, LocalExecutionOutput, LocalExecutor, Runtime,
};

fn automatic_risk() -> RiskMetadata {
    RiskMetadata {
        effect_class: EffectClass::PureRead,
        determinism_class: DeterminismClass::Deterministic,
        data_flow: DataFlowPolicy::default(),
        reliability: ReliabilityMetadata {
            idempotency_required: false,
            retryable: true,
            compensation_available: false,
        },
    }
}

fn non_automatic_risk() -> RiskMetadata {
    let mut risk = automatic_risk();
    risk.effect_class = EffectClass::ExternalEffect;
    risk
}

fn contract(id: &str, version: &str, risk: RiskMetadata) -> CapabilityContract {
    let (namespace, name) = id.rsplit_once('.').unwrap_or(("test", id));
    CapabilityContract {
        kind: "capability_contract".to_string(),
        schema_version: "1.0.0".to_string(),
        id: id.to_string(),
        namespace: namespace.to_string(),
        name: name.to_string(),
        version: version.to_string(),
        lifecycle: Lifecycle::Active,
        owner: Owner {
            team: "traverse-core".to_string(),
            contact: "enrico.piovesan10@gmail.com".to_string(),
        },
        summary: "Test capability for proposal MCP end-to-end coverage.".to_string(),
        description: "Portable test capability used to exercise the proposal MCP surface."
            .to_string(),
        inputs: SchemaContainer {
            schema: json!({"type": "object"}),
        },
        outputs: SchemaContainer {
            schema: json!({"type": "object"}),
        },
        preconditions: Vec::new(),
        postconditions: Vec::new(),
        side_effects: vec![SideEffect {
            kind: SideEffectKind::MemoryOnly,
            description: "No durable side effect.".to_string(),
        }],
        emits: Vec::new(),
        consumes: Vec::new(),
        permissions: Vec::new(),
        execution: Execution {
            binary_format: ContractBinaryFormat::Wasm,
            entrypoint: Entrypoint {
                kind: EntrypointKind::WasiCommand,
                command: "run".to_string(),
            },
            preferred_targets: vec![ExecutionTarget::Local],
            constraints: ExecutionConstraints {
                host_api_access: HostApiAccess::None,
                network_access: NetworkAccess::Forbidden,
                filesystem_access: FilesystemAccess::None,
            },
            resources: None,
        },
        policies: Vec::new(),
        dependencies: Vec::new(),
        provenance: traverse_contracts::Provenance {
            source: traverse_contracts::ProvenanceSource::Greenfield,
            author: "test".to_string(),
            created_at: "2026-08-23T00:00:00Z".to_string(),
            spec_ref: None,
            adr_refs: Vec::new(),
            exception_refs: Vec::new(),
        },
        evidence: Vec::new(),
        service_type: ServiceType::Stateless,
        permitted_targets: vec![ExecutionTarget::Local],
        event_trigger: None,
        connector_requirements: Vec::new(),
        state_schema: None,
        use_cases: Vec::new(),
        risk,
    }
}

fn artifact(digest: &str) -> CapabilityArtifactRecord {
    CapabilityArtifactRecord {
        artifact_ref: format!("artifact:{digest}"),
        implementation_kind: ImplementationKind::Executable,
        source: SourceReference {
            kind: SourceKind::Git,
            location: "https://example.invalid/repo".to_string(),
        },
        binary: Some(BinaryReference {
            format: RegistryBinaryFormat::Wasm,
            location: format!("artifacts/{digest}/capability.wasm"),
            signature: None,
        }),
        workflow_ref: None,
        digests: ArtifactDigests {
            source_digest: format!("src-{digest}"),
            binary_digest: Some(digest.to_string()),
        },
        provenance: RegistryProvenance {
            source: "test".to_string(),
            author: "test".to_string(),
            created_at: "2026-08-23T00:00:00Z".to_string(),
        },
    }
}

fn registry_with(
    entries: Vec<(CapabilityContract, CapabilityArtifactRecord)>,
) -> CapabilityRegistry {
    let mut registry = CapabilityRegistry::new();
    for (contract, artifact) in entries {
        let outcome = registry.register(CapabilityRegistration {
            scope: RegistryScope::Public,
            contract,
            contract_path: "registry/test/contract.json".to_string(),
            artifact,
            registered_at: "2026-08-23T00:00:00Z".to_string(),
            tags: Vec::new(),
            composability: ComposabilityMetadata {
                kind: CompositionKind::Atomic,
                patterns: vec![CompositionPattern::Sequential],
                provides: Vec::new(),
                requires: Vec::new(),
            },
            governing_spec: "005-capability-registry".to_string(),
            validator_version: "0.1.0".to_string(),
        });
        assert!(outcome.is_ok(), "registration must succeed: {outcome:?}");
    }
    registry
}

fn manifest_declaring(
    components: &[(&str, &str)],
    risk: &RiskMetadata,
) -> ApplicationBundleManifest {
    ApplicationBundleManifest {
        app_id: "test-app".to_string(),
        version: "1.0.0".to_string(),
        schema_version: "1.0.0".to_string(),
        workspace_defaults: json!({}),
        components: components
            .iter()
            .map(|(capability_id, capability_version)| ApplicationComponent {
                reference: ApplicationComponentRef {
                    component_id: (*capability_id).to_string(),
                    version: (*capability_version).to_string(),
                    digest: "sha256:component-digest".to_string(),
                    manifest_path: "component.manifest.json".to_string(),
                },
                manifest_path: "component.manifest.json".into(),
                manifest: WasmComponentManifest {
                    component_id: (*capability_id).to_string(),
                    version: (*capability_version).to_string(),
                    schema_version: "1.0.0".to_string(),
                    execution_mode: ComponentExecutionMode::Wasm,
                    capability_id: (*capability_id).to_string(),
                    capability_version: (*capability_version).to_string(),
                    contract_path: None,
                    registry_ref: None,
                    wasm_binary_path: None,
                    wasm_digest: None,
                    platforms: vec!["local".to_string()],
                    wrapper_path: None,
                    runtime_constraints: json!({}),
                    permitted_targets: vec![ExecutionTarget::Local],
                    dependencies: Vec::new(),
                    connector_requirements: Vec::new(),
                    validation_evidence: Vec::new(),
                    executable_pin: None,
                },
                contract_path: "contract.json".into(),
                contract: contract(capability_id, capability_version, risk.clone()),
                wasm_binary_path: None,
                verified_wasm_digest: None,
            })
            .collect(),
        workflows: Vec::new(),
        connector_bindings: Vec::new(),
        model_dependencies: Vec::new(),
        config_schema: json!({}),
        default_config: json!({}),
        effective_config: ApplicationEffectiveConfig {
            values: json!({}),
            redacted_secret_keys: Vec::new(),
        },
        placement_policy: json!({}),
        public_surfaces: Vec::new(),
        state_machine: None,
    }
}

fn linear_proposal_json(proposal_id: &str) -> String {
    json!({
        "kind": "workflow_proposal",
        "schema_version": "1.0.0",
        "proposal_id": proposal_id,
        "workspace_id": "workspace-001",
        "app_manifest": {
            "app_id": "test-app",
            "app_version": "1.0.0",
            "manifest_digest": "sha256:manifest-digest"
        },
        "nodes": [
            {
                "node_id": "a",
                "capability_id": "test.single",
                "capability_version": "1.0.0",
                "artifact_digest": "digest-a"
            }
        ],
        "edges": [],
        "mappings": [],
        "initial_input": {}
    })
    .to_string()
}

/// Fails `canonicalize_proposal` (wrong `kind`), exercising the structural-
/// validation-failure branch of every MCP tool function.
fn structurally_invalid_proposal_json(proposal_id: &str) -> String {
    json!({
        "kind": "not_a_workflow_proposal",
        "schema_version": "1.0.0",
        "proposal_id": proposal_id,
        "workspace_id": "workspace-001",
        "app_manifest": {
            "app_id": "test-app",
            "app_version": "1.0.0",
            "manifest_digest": "sha256:manifest-digest"
        },
        "nodes": [],
        "edges": [],
        "mappings": [],
        "initial_input": {}
    })
    .to_string()
}

struct EchoExecutor;

impl LocalExecutor for EchoExecutor {
    fn execute(
        &self,
        _capability: &traverse_registry::ResolvedCapability,
        _input: &Value,
    ) -> Result<LocalExecutionOutput, LocalExecutionFailure> {
        Ok(LocalExecutionOutput {
            value: json!({"status": "ok"}),
            emitted_events: Vec::new(),
        })
    }
}

struct AlwaysFailingExecutor;

impl LocalExecutor for AlwaysFailingExecutor {
    fn execute(
        &self,
        _capability: &traverse_registry::ResolvedCapability,
        _input: &Value,
    ) -> Result<LocalExecutionOutput, LocalExecutionFailure> {
        Err(LocalExecutionFailure {
            code: LocalExecutionFailureCode::ExecutionFailed,
            message: "always fails".to_string(),
        })
    }
}

// -- validate_proposal --------------------------------------------------------

#[test]
fn validate_proposal_accepts_a_well_formed_automatic_proposal() -> Result<(), String> {
    let manifest = manifest_declaring(&[("test.single", "1.0.0")], &automatic_risk());
    let registry = registry_with(vec![(
        contract("test.single", "1.0.0", automatic_risk()),
        artifact("digest-a"),
    )]);

    let response = validate_proposal(
        &linear_proposal_json("proposal-accept"),
        &manifest,
        &registry,
        &ProposalLimits::default(),
    )
    .map_err(|e| format!("{e:?}"))?;

    assert!(response.valid, "errors: {:?}", response.errors);
    assert!(response.errors.is_empty());
    assert!(!response.proposal_digest.is_empty());
    Ok(())
}

#[test]
fn validate_proposal_rejects_a_structurally_invalid_proposal() -> Result<(), String> {
    let manifest = manifest_declaring(&[("test.single", "1.0.0")], &automatic_risk());
    let registry = registry_with(vec![(
        contract("test.single", "1.0.0", automatic_risk()),
        artifact("digest-a"),
    )]);

    let response = validate_proposal(
        &structurally_invalid_proposal_json("proposal-bad-kind"),
        &manifest,
        &registry,
        &ProposalLimits::default(),
    )
    .map_err(|e| format!("{e:?}"))?;

    assert!(!response.valid);
    assert!(!response.errors.is_empty());
    Ok(())
}

#[test]
fn validate_proposal_reports_invalid_json_as_an_mcp_error() {
    let manifest = manifest_declaring(&[("test.single", "1.0.0")], &automatic_risk());
    let registry = registry_with(vec![(
        contract("test.single", "1.0.0", automatic_risk()),
        artifact("digest-a"),
    )]);

    let result = validate_proposal("not json", &manifest, &registry, &ProposalLimits::default());
    assert!(result.is_err());
}

#[test]
fn validate_proposal_rejects_a_capability_undeclared_in_the_manifest() -> Result<(), String> {
    let manifest = manifest_declaring(&[("test.other", "1.0.0")], &automatic_risk()); // does not declare test.single
    let registry = registry_with(vec![(
        contract("test.single", "1.0.0", automatic_risk()),
        artifact("digest-a"),
    )]);

    let response = validate_proposal(
        &linear_proposal_json("proposal-undeclared"),
        &manifest,
        &registry,
        &ProposalLimits::default(),
    )
    .map_err(|e| format!("{e:?}"))?;

    assert!(!response.valid);
    assert!(
        response
            .errors
            .iter()
            .any(|e| e.code == "undeclared_capability")
    );
    Ok(())
}

// -- submit_proposal / authorization_state --------------------------------------------------------

fn snapshots() -> traverse_contracts::SnapshotDigests {
    traverse_contracts::SnapshotDigests {
        manifest_digest: "manifest-1".to_string(),
        registry_digest: "registry-1".to_string(),
        binding_digest: "binding-1".to_string(),
        policy_digest: "policy-1".to_string(),
        budget_digest: "budget-1".to_string(),
    }
}

#[test]
fn submit_proposal_reports_automatic_eligibility_and_a_bound_snapshot_digest() -> Result<(), String>
{
    let manifest = manifest_declaring(&[("test.single", "1.0.0")], &automatic_risk());
    let registry = registry_with(vec![(
        contract("test.single", "1.0.0", automatic_risk()),
        artifact("digest-a"),
    )]);

    let response = submit_proposal(
        &linear_proposal_json("proposal-submit"),
        &manifest,
        &registry,
        &ProposalLimits::default(),
        &snapshots(),
    )
    .map_err(|e| format!("{e:?}"))?;

    assert!(response.valid);
    assert!(response.automatic_eligible);
    assert!(!response.snapshot_digest.is_empty());
    assert_ne!(response.snapshot_digest, response.proposal_digest);
    Ok(())
}

#[test]
fn submit_proposal_reports_a_structurally_invalid_proposal() -> Result<(), String> {
    let manifest = manifest_declaring(&[("test.single", "1.0.0")], &automatic_risk());
    let registry = registry_with(vec![(
        contract("test.single", "1.0.0", automatic_risk()),
        artifact("digest-a"),
    )]);

    let response = submit_proposal(
        &structurally_invalid_proposal_json("proposal-submit-bad-kind"),
        &manifest,
        &registry,
        &ProposalLimits::default(),
        &snapshots(),
    )
    .map_err(|e| format!("{e:?}"))?;

    assert!(!response.valid);
    assert!(!response.automatic_eligible);
    assert!(!response.errors.is_empty());
    Ok(())
}

#[test]
fn submit_proposal_reports_a_cross_validation_invalid_proposal() -> Result<(), String> {
    let manifest = manifest_declaring(&[("test.other", "1.0.0")], &automatic_risk()); // does not declare test.single
    let registry = registry_with(vec![(
        contract("test.single", "1.0.0", automatic_risk()),
        artifact("digest-a"),
    )]);

    let response = submit_proposal(
        &linear_proposal_json("proposal-submit-undeclared"),
        &manifest,
        &registry,
        &ProposalLimits::default(),
        &snapshots(),
    )
    .map_err(|e| format!("{e:?}"))?;

    assert!(!response.valid);
    assert!(!response.automatic_eligible);
    assert!(
        response
            .errors
            .iter()
            .any(|e| e.code == "undeclared_capability")
    );
    Ok(())
}

#[test]
fn authorization_state_is_automatic_for_an_automatic_eligible_proposal() -> Result<(), String> {
    let manifest = manifest_declaring(&[("test.single", "1.0.0")], &automatic_risk());
    let registry = registry_with(vec![(
        contract("test.single", "1.0.0", automatic_risk()),
        artifact("digest-a"),
    )]);

    let state = authorization_state(
        &linear_proposal_json("proposal-auth-auto"),
        &manifest,
        &registry,
        &ProposalLimits::default(),
    )
    .map_err(|e| format!("{e:?}"))?;

    assert_eq!(state, AuthorizationState::Automatic);
    Ok(())
}

#[test]
fn authorization_state_requires_approval_for_a_non_automatic_proposal() -> Result<(), String> {
    let manifest = manifest_declaring(&[("test.single", "1.0.0")], &non_automatic_risk());
    let registry = registry_with(vec![(
        contract("test.single", "1.0.0", non_automatic_risk()),
        artifact("digest-a"),
    )]);

    let state = authorization_state(
        &linear_proposal_json("proposal-auth-required"),
        &manifest,
        &registry,
        &ProposalLimits::default(),
    )
    .map_err(|e| format!("{e:?}"))?;

    assert_eq!(state, AuthorizationState::RequiresApprovalToken);
    Ok(())
}

#[test]
fn authorization_state_is_invalid_for_a_structurally_invalid_proposal() -> Result<(), String> {
    let manifest = manifest_declaring(&[("test.single", "1.0.0")], &automatic_risk());
    let registry = registry_with(vec![(
        contract("test.single", "1.0.0", automatic_risk()),
        artifact("digest-a"),
    )]);

    let state = authorization_state(
        &structurally_invalid_proposal_json("proposal-auth-bad-kind"),
        &manifest,
        &registry,
        &ProposalLimits::default(),
    )
    .map_err(|e| format!("{e:?}"))?;

    let AuthorizationState::Invalid { errors } = state else {
        return Err(format!("expected Invalid, got {state:?}"));
    };
    assert!(!errors.is_empty());
    Ok(())
}

#[test]
fn authorization_state_is_invalid_for_a_cross_validation_invalid_proposal() -> Result<(), String> {
    let manifest = manifest_declaring(&[("test.other", "1.0.0")], &automatic_risk()); // does not declare test.single
    let registry = registry_with(vec![(
        contract("test.single", "1.0.0", automatic_risk()),
        artifact("digest-a"),
    )]);

    let state = authorization_state(
        &linear_proposal_json("proposal-auth-undeclared"),
        &manifest,
        &registry,
        &ProposalLimits::default(),
    )
    .map_err(|e| format!("{e:?}"))?;

    let AuthorizationState::Invalid { errors } = state else {
        return Err(format!("expected Invalid, got {state:?}"));
    };
    assert!(errors.iter().any(|e| e.code == "undeclared_capability"));
    Ok(())
}

// -- execute_proposal_via_mcp --------------------------------------------------------

fn execution_context<'a>(
    proposal_json: &'a str,
    manifest: &'a ApplicationBundleManifest,
    registry: &'a CapabilityRegistry,
    limits: &'a ProposalLimits,
    snapshots: &'a traverse_contracts::SnapshotDigests,
    approval_token: Option<&'a str>,
    keys: &'a HashMap<String, ed25519_dalek::VerifyingKey>,
) -> ProposalExecutionRequest<'a> {
    ProposalExecutionRequest {
        proposal_json,
        manifest,
        registry,
        limits,
        snapshots,
        approval_token,
        expected_token_issuer: "traverse-approval-service",
        expected_token_audience: "traverse-runtime",
        token_verifying_keys_by_key_id: keys,
        principal: "principal-001",
        app_id: "test-app",
    }
}

#[test]
fn execute_proposal_via_mcp_succeeds_for_an_automatic_eligible_proposal() -> Result<(), String> {
    let manifest = manifest_declaring(&[("test.single", "1.0.0")], &automatic_risk());
    let registry = registry_with(vec![(
        contract("test.single", "1.0.0", automatic_risk()),
        artifact("digest-a"),
    )]);
    let runtime = Runtime::new(registry.clone(), EchoExecutor)
        .with_security_config(RuntimeSecurityConfig::development());
    let token_store = ApprovalTokenStore::new();
    let quota_tracker = QuotaTracker::new();
    let keys = HashMap::new();
    let proposal_json = linear_proposal_json("proposal-exec-auto");

    let response = execute_proposal_via_mcp(
        &runtime,
        &execution_context(
            &proposal_json,
            &manifest,
            &registry,
            &ProposalLimits::default(),
            &snapshots(),
            None,
            &keys,
        ),
        &token_store,
        &quota_tracker,
        &QuotaLimits::default(),
    )
    .map_err(|e| format!("{e:?}"))?;

    let ProposalExecutionResponse::Trace(trace) = response else {
        return Err(format!("expected success, got {response:?}"));
    };
    assert_eq!(trace.terminal_state, ProposalTerminalState::Succeeded);
    assert_eq!(trace.node_outcomes[0].status, ProposalNodeStatus::Succeeded);
    assert!(trace.authorization.automatic);

    let observed = observe_proposal(&trace);
    assert_eq!(observed["terminal_state"], json!("succeeded"));
    // The redacted trace must never carry raw payloads or secrets.
    assert!(observed.get("initial_input").is_none());
    assert!(observed.get("output").is_none());
    Ok(())
}

#[test]
fn execute_proposal_via_mcp_denies_a_structurally_invalid_proposal() -> Result<(), String> {
    let manifest = manifest_declaring(&[("test.single", "1.0.0")], &automatic_risk());
    let registry = registry_with(vec![(
        contract("test.single", "1.0.0", automatic_risk()),
        artifact("digest-a"),
    )]);
    let runtime = Runtime::new(registry.clone(), EchoExecutor)
        .with_security_config(RuntimeSecurityConfig::development());
    let token_store = ApprovalTokenStore::new();
    let quota_tracker = QuotaTracker::new();
    let keys = HashMap::new();
    let proposal_json = structurally_invalid_proposal_json("proposal-exec-bad-kind");

    let response = execute_proposal_via_mcp(
        &runtime,
        &execution_context(
            &proposal_json,
            &manifest,
            &registry,
            &ProposalLimits::default(),
            &snapshots(),
            None,
            &keys,
        ),
        &token_store,
        &quota_tracker,
        &QuotaLimits::default(),
    )
    .map_err(|e| format!("{e:?}"))?;

    let ProposalExecutionResponse::Denied { code, .. } = response else {
        return Err(format!("expected a denial, got {response:?}"));
    };
    assert_eq!(code, "invalid_proposal");
    Ok(())
}

#[test]
fn execute_proposal_via_mcp_denies_a_cross_validation_invalid_proposal() -> Result<(), String> {
    let manifest = manifest_declaring(&[("test.other", "1.0.0")], &automatic_risk()); // does not declare test.single
    let registry = registry_with(vec![(
        contract("test.single", "1.0.0", automatic_risk()),
        artifact("digest-a"),
    )]);
    let runtime = Runtime::new(registry.clone(), EchoExecutor)
        .with_security_config(RuntimeSecurityConfig::development());
    let token_store = ApprovalTokenStore::new();
    let quota_tracker = QuotaTracker::new();
    let keys = HashMap::new();
    let proposal_json = linear_proposal_json("proposal-exec-undeclared");

    let response = execute_proposal_via_mcp(
        &runtime,
        &execution_context(
            &proposal_json,
            &manifest,
            &registry,
            &ProposalLimits::default(),
            &snapshots(),
            None,
            &keys,
        ),
        &token_store,
        &quota_tracker,
        &QuotaLimits::default(),
    )
    .map_err(|e| format!("{e:?}"))?;

    let ProposalExecutionResponse::Denied { code, .. } = response else {
        return Err(format!("expected a denial, got {response:?}"));
    };
    assert_eq!(code, "invalid_proposal");
    Ok(())
}

#[test]
fn corrected_reproposal_requires_a_new_explicit_execution_after_rejection() -> Result<(), String> {
    let manifest = manifest_declaring(&[("test.single", "1.0.0")], &automatic_risk());
    let registry = registry_with(vec![(
        contract("test.single", "1.0.0", automatic_risk()),
        artifact("digest-a"),
    )]);
    let runtime = Runtime::new(registry.clone(), EchoExecutor)
        .with_security_config(RuntimeSecurityConfig::development());
    let token_store = ApprovalTokenStore::new();
    let quota_tracker = QuotaTracker::new();
    let keys = HashMap::new();
    let rejected_json = structurally_invalid_proposal_json("proposal-recovery-rejected");
    let corrected_json = linear_proposal_json("proposal-recovery-corrected");

    let rejected = execute_proposal_via_mcp(
        &runtime,
        &execution_context(
            &rejected_json,
            &manifest,
            &registry,
            &ProposalLimits::default(),
            &snapshots(),
            None,
            &keys,
        ),
        &token_store,
        &quota_tracker,
        &QuotaLimits::default(),
    )
    .map_err(|error| format!("{error:?}"))?;
    let ProposalExecutionResponse::Denied { code, .. } = rejected else {
        return Err("expected rejected proposal to be denied".to_string());
    };
    assert_eq!(code, "invalid_proposal");

    let corrected = execute_proposal_via_mcp(
        &runtime,
        &execution_context(
            &corrected_json,
            &manifest,
            &registry,
            &ProposalLimits::default(),
            &snapshots(),
            None,
            &keys,
        ),
        &token_store,
        &quota_tracker,
        &QuotaLimits::default(),
    )
    .map_err(|error| format!("{error:?}"))?;
    let ProposalExecutionResponse::Trace(trace) = corrected else {
        return Err("expected corrected re-proposal to execute".to_string());
    };
    assert_eq!(trace.terminal_state, ProposalTerminalState::Succeeded);
    assert_eq!(trace.node_outcomes[0].status, ProposalNodeStatus::Succeeded);
    Ok(())
}

#[test]
fn execute_proposal_via_mcp_denies_when_the_approval_token_use_count_is_already_exhausted()
-> Result<(), String> {
    let manifest = manifest_declaring(&[("test.single", "1.0.0")], &non_automatic_risk());
    let registry = registry_with(vec![(
        contract("test.single", "1.0.0", non_automatic_risk()),
        artifact("digest-a"),
    )]);
    let runtime = Runtime::new(registry.clone(), EchoExecutor)
        .with_security_config(RuntimeSecurityConfig::development());
    let token_store = ApprovalTokenStore::new();
    let quota_tracker = QuotaTracker::new();
    let signing_key = SigningKey::from_bytes(&[13_u8; 32]);
    let mut keys = HashMap::new();
    keys.insert("key-1".to_string(), signing_key.verifying_key());

    let proposal_json = linear_proposal_json("proposal-exec-use-count");
    let submission = submit_proposal(
        &proposal_json,
        &manifest,
        &registry,
        &ProposalLimits::default(),
        &snapshots(),
    )
    .map_err(|e| format!("{e:?}"))?;

    // max_use_count: 1 — the token store must reject the second execute call
    // with the *same* token even though its signature and digest bindings
    // are still perfectly valid (spec 109 FR-006a use-count enforcement).
    let token = sign_token(
        &json!({
            "jti": "token-single-use-001",
            "iss": "traverse-approval-service",
            "aud": "traverse-runtime",
            "sub": "principal-001",
            "workspace_id": "workspace-001",
            "proposal_digest": submission.proposal_digest,
            "snapshot_digest": submission.snapshot_digest,
            "permitted_effects": ["external_effect"],
            "permitted_connectors": [],
            "max_use_count": 1,
            "exp": 4_102_444_800_i64,
        }),
        &signing_key,
        "key-1",
    );

    let first = execute_proposal_via_mcp(
        &runtime,
        &execution_context(
            &proposal_json,
            &manifest,
            &registry,
            &ProposalLimits::default(),
            &snapshots(),
            Some(&token),
            &keys,
        ),
        &token_store,
        &quota_tracker,
        &QuotaLimits::default(),
    )
    .map_err(|e| format!("{e:?}"))?;
    let ProposalExecutionResponse::Trace(_) = first else {
        return Err(format!("expected the first use to succeed, got {first:?}"));
    };

    let second = execute_proposal_via_mcp(
        &runtime,
        &execution_context(
            &proposal_json,
            &manifest,
            &registry,
            &ProposalLimits::default(),
            &snapshots(),
            Some(&token),
            &keys,
        ),
        &token_store,
        &quota_tracker,
        &QuotaLimits::default(),
    )
    .map_err(|e| format!("{e:?}"))?;

    let ProposalExecutionResponse::Denied { code, .. } = second else {
        return Err(format!(
            "expected the second use to be denied, got {second:?}"
        ));
    };
    assert_eq!(code, "use_count_exhausted");
    Ok(())
}

#[test]
fn execute_proposal_via_mcp_denies_execution_without_a_required_approval_token()
-> Result<(), String> {
    let manifest = manifest_declaring(&[("test.single", "1.0.0")], &non_automatic_risk());
    let registry = registry_with(vec![(
        contract("test.single", "1.0.0", non_automatic_risk()),
        artifact("digest-a"),
    )]);
    let runtime = Runtime::new(registry.clone(), EchoExecutor)
        .with_security_config(RuntimeSecurityConfig::development());
    let token_store = ApprovalTokenStore::new();
    let quota_tracker = QuotaTracker::new();
    let keys = HashMap::new();
    let proposal_json = linear_proposal_json("proposal-exec-no-token");

    let response = execute_proposal_via_mcp(
        &runtime,
        &execution_context(
            &proposal_json,
            &manifest,
            &registry,
            &ProposalLimits::default(),
            &snapshots(),
            None,
            &keys,
        ),
        &token_store,
        &quota_tracker,
        &QuotaLimits::default(),
    )
    .map_err(|e| format!("{e:?}"))?;

    let ProposalExecutionResponse::Denied { code, .. } = response else {
        return Err(format!("expected a denial, got {response:?}"));
    };
    assert_eq!(code, "approval_token_required");
    Ok(())
}

fn base64url_encode(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    let mut i = 0;
    while i + 3 <= input.len() {
        let n =
            (u32::from(input[i]) << 16) | (u32::from(input[i + 1]) << 8) | u32::from(input[i + 2]);
        out.push(ALPHABET[((n >> 18) & 63) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 63) as usize] as char);
        out.push(ALPHABET[((n >> 6) & 63) as usize] as char);
        out.push(ALPHABET[(n & 63) as usize] as char);
        i += 3;
    }
    let remainder = input.len() - i;
    if remainder == 1 {
        let n = u32::from(input[i]) << 16;
        out.push(ALPHABET[((n >> 18) & 63) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 63) as usize] as char);
    } else if remainder == 2 {
        let n = (u32::from(input[i]) << 16) | (u32::from(input[i + 1]) << 8);
        out.push(ALPHABET[((n >> 18) & 63) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 63) as usize] as char);
        out.push(ALPHABET[((n >> 6) & 63) as usize] as char);
    }
    out
}

fn sign_token(payload: &Value, key: &SigningKey, key_id: &str) -> String {
    let header = base64url_encode(format!(r#"{{"alg":"EdDSA","kid":"{key_id}"}}"#).as_bytes());
    let payload_b64 = base64url_encode(payload.to_string().as_bytes());
    let signing_input = format!("{header}.{payload_b64}");
    let signature = key.sign(signing_input.as_bytes());
    let signature_b64 = base64url_encode(&signature.to_bytes());
    format!("{header}.{payload_b64}.{signature_b64}")
}

#[test]
fn execute_proposal_via_mcp_succeeds_with_a_valid_approval_token() -> Result<(), String> {
    let manifest = manifest_declaring(&[("test.single", "1.0.0")], &non_automatic_risk());
    let registry = registry_with(vec![(
        contract("test.single", "1.0.0", non_automatic_risk()),
        artifact("digest-a"),
    )]);
    let runtime = Runtime::new(registry.clone(), EchoExecutor)
        .with_security_config(RuntimeSecurityConfig::development());
    let token_store = ApprovalTokenStore::new();
    let quota_tracker = QuotaTracker::new();
    let signing_key = SigningKey::from_bytes(&[11_u8; 32]);
    let mut keys = HashMap::new();
    keys.insert("key-1".to_string(), signing_key.verifying_key());

    let proposal_json = linear_proposal_json("proposal-exec-with-token");
    let submission = submit_proposal(
        &proposal_json,
        &manifest,
        &registry,
        &ProposalLimits::default(),
        &snapshots(),
    )
    .map_err(|e| format!("{e:?}"))?;
    assert!(!submission.automatic_eligible);

    let token = sign_token(
        &json!({
            "jti": "token-exec-001",
            "iss": "traverse-approval-service",
            "aud": "traverse-runtime",
            "sub": "principal-001",
            "workspace_id": "workspace-001",
            "proposal_digest": submission.proposal_digest,
            "snapshot_digest": submission.snapshot_digest,
            "permitted_effects": ["external_effect"],
            "permitted_connectors": [],
            "max_use_count": 1,
            "exp": 4_102_444_800_i64,
        }),
        &signing_key,
        "key-1",
    );

    let response = execute_proposal_via_mcp(
        &runtime,
        &execution_context(
            &proposal_json,
            &manifest,
            &registry,
            &ProposalLimits::default(),
            &snapshots(),
            Some(&token),
            &keys,
        ),
        &token_store,
        &quota_tracker,
        &QuotaLimits::default(),
    )
    .map_err(|e| format!("{e:?}"))?;

    let ProposalExecutionResponse::Trace(trace) = response else {
        return Err(format!("expected success, got {response:?}"));
    };
    assert_eq!(trace.terminal_state, ProposalTerminalState::Succeeded);
    assert!(!trace.authorization.automatic);
    assert_eq!(
        trace.authorization.approval_token_id.as_deref(),
        Some("token-exec-001")
    );
    Ok(())
}

#[test]
fn execute_proposal_via_mcp_denies_an_invalid_approval_token() -> Result<(), String> {
    let manifest = manifest_declaring(&[("test.single", "1.0.0")], &non_automatic_risk());
    let registry = registry_with(vec![(
        contract("test.single", "1.0.0", non_automatic_risk()),
        artifact("digest-a"),
    )]);
    let runtime = Runtime::new(registry.clone(), EchoExecutor)
        .with_security_config(RuntimeSecurityConfig::development());
    let token_store = ApprovalTokenStore::new();
    let quota_tracker = QuotaTracker::new();
    let signing_key = SigningKey::from_bytes(&[11_u8; 32]);
    let wrong_key = SigningKey::from_bytes(&[12_u8; 32]);
    let mut keys = HashMap::new();
    keys.insert("key-1".to_string(), signing_key.verifying_key());

    let proposal_json = linear_proposal_json("proposal-exec-bad-token");
    let submission = submit_proposal(
        &proposal_json,
        &manifest,
        &registry,
        &ProposalLimits::default(),
        &snapshots(),
    )
    .map_err(|e| format!("{e:?}"))?;

    // Signed with the wrong key.
    let token = sign_token(
        &json!({
            "jti": "token-bad-001",
            "iss": "traverse-approval-service",
            "aud": "traverse-runtime",
            "sub": "principal-001",
            "workspace_id": "workspace-001",
            "proposal_digest": submission.proposal_digest,
            "snapshot_digest": submission.snapshot_digest,
            "permitted_effects": ["external_effect"],
            "permitted_connectors": [],
            "max_use_count": 1,
            "exp": 4_102_444_800_i64,
        }),
        &wrong_key,
        "key-1",
    );

    let response = execute_proposal_via_mcp(
        &runtime,
        &execution_context(
            &proposal_json,
            &manifest,
            &registry,
            &ProposalLimits::default(),
            &snapshots(),
            Some(&token),
            &keys,
        ),
        &token_store,
        &quota_tracker,
        &QuotaLimits::default(),
    )
    .map_err(|e| format!("{e:?}"))?;

    let ProposalExecutionResponse::Denied { code, .. } = response else {
        return Err(format!("expected a denial, got {response:?}"));
    };
    assert_eq!(code, "signature_verification_failed");
    Ok(())
}

#[test]
fn execute_proposal_via_mcp_denies_when_quota_is_exhausted() -> Result<(), String> {
    let manifest = manifest_declaring(&[("test.single", "1.0.0")], &automatic_risk());
    let registry = registry_with(vec![(
        contract("test.single", "1.0.0", automatic_risk()),
        artifact("digest-a"),
    )]);
    let runtime = Runtime::new(registry.clone(), EchoExecutor)
        .with_security_config(RuntimeSecurityConfig::development());
    let token_store = ApprovalTokenStore::new();
    let quota_tracker = QuotaTracker::new();
    let keys = HashMap::new();
    let exhausted_limits = QuotaLimits {
        max_concurrent_per_principal: 0,
        max_concurrent_per_app: 10,
        max_concurrent_per_workspace: 10,
    };
    let proposal_json = linear_proposal_json("proposal-exec-quota");

    let response = execute_proposal_via_mcp(
        &runtime,
        &execution_context(
            &proposal_json,
            &manifest,
            &registry,
            &ProposalLimits::default(),
            &snapshots(),
            None,
            &keys,
        ),
        &token_store,
        &quota_tracker,
        &exhausted_limits,
    )
    .map_err(|e| format!("{e:?}"))?;

    let ProposalExecutionResponse::Denied { code, .. } = response else {
        return Err(format!("expected a denial, got {response:?}"));
    };
    assert_eq!(code, "quota_exhausted_principal");
    Ok(())
}

#[test]
fn execute_proposal_via_mcp_reports_a_failed_terminal_state_when_the_executor_fails()
-> Result<(), String> {
    let manifest = manifest_declaring(&[("test.single", "1.0.0")], &automatic_risk());
    let registry = registry_with(vec![(
        contract("test.single", "1.0.0", automatic_risk()),
        artifact("digest-a"),
    )]);
    let runtime = Runtime::new(registry.clone(), AlwaysFailingExecutor)
        .with_security_config(RuntimeSecurityConfig::development());
    let token_store = ApprovalTokenStore::new();
    let quota_tracker = QuotaTracker::new();
    let keys = HashMap::new();
    let proposal_json = linear_proposal_json("proposal-exec-failed");

    let response = execute_proposal_via_mcp(
        &runtime,
        &execution_context(
            &proposal_json,
            &manifest,
            &registry,
            &ProposalLimits::default(),
            &snapshots(),
            None,
            &keys,
        ),
        &token_store,
        &quota_tracker,
        &QuotaLimits::default(),
    )
    .map_err(|e| format!("{e:?}"))?;

    let ProposalExecutionResponse::Trace(trace) = response else {
        return Err(format!(
            "expected a trace with a failed node, got {response:?}"
        ));
    };
    assert_eq!(trace.terminal_state, ProposalTerminalState::Failed);
    assert_eq!(trace.node_outcomes[0].status, ProposalNodeStatus::Failed);
    Ok(())
}

// -- export_proposal --------------------------------------------------------

#[test]
fn export_proposal_digest_matches_submit_proposal_digest() -> Result<(), String> {
    let manifest = manifest_declaring(&[("test.single", "1.0.0")], &automatic_risk());
    let registry = registry_with(vec![(
        contract("test.single", "1.0.0", automatic_risk()),
        artifact("digest-a"),
    )]);
    let proposal_json = linear_proposal_json("proposal-export");

    let submission = submit_proposal(
        &proposal_json,
        &manifest,
        &registry,
        &ProposalLimits::default(),
        &snapshots(),
    )
    .map_err(|e| format!("{e:?}"))?;
    let export = export_proposal(&proposal_json, &ProposalLimits::default())
        .map_err(|e| format!("{e:?}"))?;

    assert_eq!(export.proposal_digest, submission.proposal_digest);
    assert_eq!(export.execution_order, vec!["a".to_string()]);
    Ok(())
}

#[test]
fn export_proposal_rejects_a_structurally_invalid_proposal() {
    let result = export_proposal(
        &structurally_invalid_proposal_json("proposal-export-bad-kind"),
        &ProposalLimits::default(),
    );
    assert!(result.is_err());
}
