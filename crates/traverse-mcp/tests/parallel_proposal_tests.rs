//! End-to-end MCP tests for bounded parallel proposal scheduling and
//! execution (spec `110-bounded-parallel-workflow-scheduling`, P2,
//! ADR-0042).
//!
//! Exercises `traverse_mcp::tools::parallel_proposals` — schedule
//! computation, FR-004a `pure_read`-only authorization, and full
//! execute-via-mcp — reusing the same wire format and manifest/registry
//! fixtures as spec 109's P1 surface. Authorization-token and quota
//! mechanics themselves are already covered end to end by
//! `proposal_tests.rs`; these tests focus on what P2 adds on top.

use ed25519_dalek::{Signer, SigningKey};
use serde_json::{Value, json};
use std::collections::HashMap;

use traverse_contracts::{
    BinaryFormat as ContractBinaryFormat, CapabilityContract, DataFlowPolicy, DeterminismClass,
    EffectClass, Entrypoint, EntrypointKind, Execution, ExecutionConstraints, ExecutionTarget,
    FilesystemAccess, HostApiAccess, Lifecycle, NetworkAccess, Owner, ParallelScheduleLimits,
    ProposalLimits, ReliabilityMetadata, RiskMetadata, SchemaContainer, ServiceType, SideEffect,
    SideEffectKind,
};
use traverse_mcp::tools::parallel_proposals::{
    ParallelProposalExecutionRequest, compute_schedule_for_proposal,
    execute_parallel_proposal_via_mcp,
};
use traverse_mcp::tools::proposals::ProposalExecutionResponse;
use traverse_registry::{
    ApplicationBundleManifest, ApplicationComponent, ApplicationComponentRef,
    ApplicationEffectiveConfig, ArtifactDigests, BinaryFormat as RegistryBinaryFormat,
    BinaryReference, CapabilityArtifactRecord, CapabilityRegistration, CapabilityRegistry,
    ComponentExecutionMode, ComposabilityMetadata, CompositionKind, CompositionPattern,
    ImplementationKind, RegistryProvenance, RegistryScope, SourceKind, SourceReference,
    WasmComponentManifest,
};
use traverse_runtime::parallel_proposal::ParallelExecutionLimits;
use traverse_runtime::proposal::{
    ApprovalTokenStore, ProposalTerminalState, QuotaLimits, QuotaTracker,
};
use traverse_runtime::security::RuntimeSecurityConfig;
use traverse_runtime::{
    LocalExecutionFailure, LocalExecutionFailureCode, LocalExecutionOutput, LocalExecutor, Runtime,
};

fn risk(effect_class: EffectClass) -> RiskMetadata {
    RiskMetadata {
        effect_class,
        determinism_class: DeterminismClass::Deterministic,
        data_flow: DataFlowPolicy::default(),
        reliability: ReliabilityMetadata {
            idempotency_required: false,
            retryable: true,
            compensation_available: false,
        },
    }
}

fn contract(id: &str, risk: RiskMetadata) -> CapabilityContract {
    let (namespace, name) = id.rsplit_once('.').unwrap_or(("test", id));
    CapabilityContract {
        kind: "capability_contract".to_string(),
        schema_version: "1.0.0".to_string(),
        id: id.to_string(),
        namespace: namespace.to_string(),
        name: name.to_string(),
        version: "1.0.0".to_string(),
        lifecycle: Lifecycle::Active,
        owner: Owner {
            team: "traverse-core".to_string(),
            contact: "enrico.piovesan10@gmail.com".to_string(),
        },
        summary: "Test capability for parallel proposal MCP coverage.".to_string(),
        description: "Portable test capability used to exercise the parallel proposal MCP surface."
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

fn manifest_declaring(components: &[(&str, EffectClass)]) -> ApplicationBundleManifest {
    ApplicationBundleManifest {
        app_id: "test-app".to_string(),
        version: "1.0.0".to_string(),
        schema_version: "1.0.0".to_string(),
        workspace_defaults: json!({}),
        components: components
            .iter()
            .map(|(capability_id, effect_class)| ApplicationComponent {
                reference: ApplicationComponentRef {
                    component_id: (*capability_id).to_string(),
                    version: "1.0.0".to_string(),
                    digest: "sha256:component-digest".to_string(),
                    manifest_path: "component.manifest.json".to_string(),
                },
                manifest_path: "component.manifest.json".into(),
                manifest: WasmComponentManifest {
                    component_id: (*capability_id).to_string(),
                    version: "1.0.0".to_string(),
                    schema_version: "1.0.0".to_string(),
                    execution_mode: ComponentExecutionMode::Wasm,
                    capability_id: (*capability_id).to_string(),
                    capability_version: "1.0.0".to_string(),
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
                contract: contract(capability_id, risk(*effect_class)),
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

fn diamond_registry(non_read_node: Option<&str>) -> CapabilityRegistry {
    registry_with(
        ["a", "b", "c", "d"]
            .iter()
            .map(|id| {
                let effect_class = if Some(*id) == non_read_node {
                    EffectClass::ExternalEffect
                } else {
                    EffectClass::PureRead
                };
                (
                    contract(&format!("test.{id}"), risk(effect_class)),
                    artifact(&format!("digest-{id}")),
                )
            })
            .collect(),
    )
}

fn diamond_manifest(non_read_node: Option<&str>) -> ApplicationBundleManifest {
    let effect_for = |id: &str| -> EffectClass {
        if Some(id) == non_read_node {
            EffectClass::ExternalEffect
        } else {
            EffectClass::PureRead
        }
    };
    manifest_declaring(&[
        ("test.a", effect_for("a")),
        ("test.b", effect_for("b")),
        ("test.c", effect_for("c")),
        ("test.d", effect_for("d")),
    ])
}

/// a fans out to b and c, which both feed the join node d.
fn diamond_proposal_json(proposal_id: &str) -> String {
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
            {"node_id": "a", "capability_id": "test.a", "capability_version": "1.0.0", "artifact_digest": "digest-a"},
            {"node_id": "b", "capability_id": "test.b", "capability_version": "1.0.0", "artifact_digest": "digest-b"},
            {"node_id": "c", "capability_id": "test.c", "capability_version": "1.0.0", "artifact_digest": "digest-c"},
            {"node_id": "d", "capability_id": "test.d", "capability_version": "1.0.0", "artifact_digest": "digest-d"}
        ],
        "edges": [
            {"from_node_id": "a", "to_node_id": "b"},
            {"from_node_id": "a", "to_node_id": "c"},
            {"from_node_id": "b", "to_node_id": "d"},
            {"from_node_id": "c", "to_node_id": "d"}
        ],
        "mappings": [],
        "initial_input": {}
    })
    .to_string()
}

fn structurally_invalid_proposal_json(proposal_id: &str) -> String {
    json!({
        "kind": "not_a_workflow_proposal",
        "schema_version": "1.0.0",
        "proposal_id": proposal_id,
        "workspace_id": "workspace-001",
        "app_manifest": {"app_id": "test-app", "app_version": "1.0.0", "manifest_digest": "sha256:manifest-digest"},
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

struct FailsOneCapabilityExecutor {
    failing_capability_id: String,
}

impl LocalExecutor for FailsOneCapabilityExecutor {
    fn execute(
        &self,
        capability: &traverse_registry::ResolvedCapability,
        _input: &Value,
    ) -> Result<LocalExecutionOutput, LocalExecutionFailure> {
        if capability.contract.id == self.failing_capability_id {
            Err(LocalExecutionFailure {
                code: LocalExecutionFailureCode::ExecutionFailed,
                message: "scripted failure".to_string(),
            })
        } else {
            Ok(LocalExecutionOutput {
                value: json!({"status": "ok"}),
                emitted_events: Vec::new(),
            })
        }
    }
}

fn keys() -> HashMap<String, ed25519_dalek::VerifyingKey> {
    HashMap::new()
}

// -- compute_schedule_for_proposal -------------------------------------------

#[test]
fn compute_schedule_accepts_a_well_formed_diamond_proposal() -> Result<(), String> {
    let manifest = diamond_manifest(None);
    let registry = diamond_registry(None);

    let response = compute_schedule_for_proposal(
        &diamond_proposal_json("proposal-schedule-accept"),
        &manifest,
        &registry,
        &ProposalLimits::default(),
        &ParallelScheduleLimits::default(),
    )
    .map_err(|e| format!("{e:?}"))?;

    assert!(response.valid, "errors: {:?}", response.errors);
    assert!(response.automatic_eligible);
    assert_eq!(
        response.waves,
        vec![
            vec!["a".to_string()],
            vec!["b".to_string(), "c".to_string()],
            vec!["d".to_string()],
        ]
    );
    Ok(())
}

#[test]
fn compute_schedule_rejects_a_structurally_invalid_proposal() -> Result<(), String> {
    let manifest = diamond_manifest(None);
    let registry = diamond_registry(None);

    let response = compute_schedule_for_proposal(
        &structurally_invalid_proposal_json("proposal-schedule-bad-kind"),
        &manifest,
        &registry,
        &ProposalLimits::default(),
        &ParallelScheduleLimits::default(),
    )
    .map_err(|e| format!("{e:?}"))?;

    assert!(!response.valid);
    assert!(response.waves.is_empty());
    Ok(())
}

#[test]
fn compute_schedule_rejects_a_cross_validation_invalid_proposal() -> Result<(), String> {
    let mut manifest = diamond_manifest(None);
    manifest
        .components
        .retain(|c| c.reference.component_id != "test.d"); // missing test.d
    let registry = diamond_registry(None);

    let response = compute_schedule_for_proposal(
        &diamond_proposal_json("proposal-schedule-undeclared"),
        &manifest,
        &registry,
        &ProposalLimits::default(),
        &ParallelScheduleLimits::default(),
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

#[test]
fn compute_schedule_rejects_a_fan_out_over_the_configured_limit() -> Result<(), String> {
    let manifest = diamond_manifest(None);
    let registry = diamond_registry(None);
    let limits = ParallelScheduleLimits {
        max_fan_out: 1,
        ..ParallelScheduleLimits::default()
    };

    let response = compute_schedule_for_proposal(
        &diamond_proposal_json("proposal-schedule-fan-out"),
        &manifest,
        &registry,
        &ProposalLimits::default(),
        &limits,
    )
    .map_err(|e| format!("{e:?}"))?;

    assert!(!response.valid);
    assert!(response.errors.iter().any(|e| e.code == "fan_out_exceeded"));
    Ok(())
}

#[test]
fn compute_schedule_denies_a_concurrent_wave_with_a_non_pure_read_node() -> Result<(), String> {
    let manifest = diamond_manifest(Some("c"));
    let registry = diamond_registry(Some("c"));

    let response = compute_schedule_for_proposal(
        &diamond_proposal_json("proposal-schedule-side-effect"),
        &manifest,
        &registry,
        &ProposalLimits::default(),
        &ParallelScheduleLimits::default(),
    )
    .map_err(|e| format!("{e:?}"))?;

    assert!(!response.valid);
    assert!(
        response
            .errors
            .iter()
            .any(|e| e.code == "concurrent_side_effect_denied")
    );
    Ok(())
}

#[test]
fn compute_schedule_reports_invalid_json_as_an_mcp_error() {
    let manifest = diamond_manifest(None);
    let registry = diamond_registry(None);

    let result = compute_schedule_for_proposal(
        "not json",
        &manifest,
        &registry,
        &ProposalLimits::default(),
        &ParallelScheduleLimits::default(),
    );
    assert!(result.is_err());
}

// -- execute_parallel_proposal_via_mcp ---------------------------------------

/// A struct literal so temporary fields (`ProposalLimits::default()`, the
/// inline `SnapshotDigests`) get Rust's `let`-initializer lifetime
/// extension, matching `request`'s scope — a plain function call would drop
/// them at the end of the call expression instead.
macro_rules! execution_request {
    ($proposal_json:expr, $manifest:expr, $registry:expr, $schedule_limits:expr, $execution_limits:expr, $approval_token:expr, $keys:expr $(,)?) => {
        ParallelProposalExecutionRequest {
            proposal_json: $proposal_json,
            manifest: $manifest,
            registry: $registry,
            proposal_limits: &ProposalLimits::default(),
            schedule_limits: $schedule_limits,
            execution_limits: $execution_limits,
            snapshots: &traverse_contracts::SnapshotDigests {
                manifest_digest: "manifest-1".to_string(),
                registry_digest: "registry-1".to_string(),
                binding_digest: "binding-1".to_string(),
                policy_digest: "policy-1".to_string(),
                budget_digest: "budget-1".to_string(),
            },
            approval_token: $approval_token,
            expected_token_issuer: "traverse-approval-service",
            expected_token_audience: "traverse-runtime",
            token_verifying_keys_by_key_id: $keys,
            principal: "principal-001",
            app_id: "test-app",
        }
    };
}

#[test]
fn execute_parallel_proposal_via_mcp_succeeds_for_an_automatic_eligible_diamond()
-> Result<(), String> {
    let manifest = diamond_manifest(None);
    let registry = diamond_registry(None);
    let runtime = Runtime::new(registry.clone(), EchoExecutor)
        .with_security_config(RuntimeSecurityConfig::development());
    let token_store = ApprovalTokenStore::new();
    let quota_tracker = QuotaTracker::new();
    let keys = keys();

    let proposal_json = diamond_proposal_json("proposal-exec-accept");
    let schedule_limits = ParallelScheduleLimits::default();
    let execution_limits = ParallelExecutionLimits::default();
    let request = execution_request!(
        &proposal_json,
        &manifest,
        &registry,
        &schedule_limits,
        &execution_limits,
        None,
        &keys,
    );

    let response = execute_parallel_proposal_via_mcp(
        &runtime,
        &request,
        &token_store,
        &quota_tracker,
        &QuotaLimits::default(),
    )
    .map_err(|e| format!("{e:?}"))?;

    let ProposalExecutionResponse::Trace(trace) = response else {
        return Err(format!("expected Trace, got {response:?}"));
    };
    assert_eq!(trace.terminal_state, ProposalTerminalState::Succeeded);
    assert_eq!(trace.node_outcomes.len(), 4);
    Ok(())
}

#[test]
fn execute_parallel_proposal_via_mcp_denies_a_structurally_invalid_proposal() -> Result<(), String>
{
    let manifest = diamond_manifest(None);
    let registry = diamond_registry(None);
    let runtime = Runtime::new(registry.clone(), EchoExecutor)
        .with_security_config(RuntimeSecurityConfig::development());
    let token_store = ApprovalTokenStore::new();
    let quota_tracker = QuotaTracker::new();
    let keys = keys();

    let proposal_json = structurally_invalid_proposal_json("proposal-exec-bad-kind");
    let schedule_limits = ParallelScheduleLimits::default();
    let execution_limits = ParallelExecutionLimits::default();
    let request = execution_request!(
        &proposal_json,
        &manifest,
        &registry,
        &schedule_limits,
        &execution_limits,
        None,
        &keys,
    );

    let response = execute_parallel_proposal_via_mcp(
        &runtime,
        &request,
        &token_store,
        &quota_tracker,
        &QuotaLimits::default(),
    )
    .map_err(|e| format!("{e:?}"))?;

    let ProposalExecutionResponse::Denied { code, .. } = response else {
        return Err(format!("expected Denied, got {response:?}"));
    };
    assert_eq!(code, "invalid_proposal");
    Ok(())
}

#[test]
fn execute_parallel_proposal_via_mcp_denies_a_cross_validation_invalid_proposal()
-> Result<(), String> {
    let mut manifest = diamond_manifest(None);
    manifest
        .components
        .retain(|c| c.reference.component_id != "test.d"); // missing test.d
    let registry = diamond_registry(None);
    let runtime = Runtime::new(registry.clone(), EchoExecutor)
        .with_security_config(RuntimeSecurityConfig::development());
    let token_store = ApprovalTokenStore::new();
    let quota_tracker = QuotaTracker::new();
    let keys = keys();

    let proposal_json = diamond_proposal_json("proposal-exec-undeclared");
    let schedule_limits = ParallelScheduleLimits::default();
    let execution_limits = ParallelExecutionLimits::default();
    let request = execution_request!(
        &proposal_json,
        &manifest,
        &registry,
        &schedule_limits,
        &execution_limits,
        None,
        &keys,
    );

    let response = execute_parallel_proposal_via_mcp(
        &runtime,
        &request,
        &token_store,
        &quota_tracker,
        &QuotaLimits::default(),
    )
    .map_err(|e| format!("{e:?}"))?;

    let ProposalExecutionResponse::Denied { code, .. } = response else {
        return Err(format!("expected Denied, got {response:?}"));
    };
    assert_eq!(code, "invalid_proposal");
    Ok(())
}

#[test]
fn execute_parallel_proposal_via_mcp_denies_a_schedule_bound_violation() -> Result<(), String> {
    let manifest = diamond_manifest(None);
    let registry = diamond_registry(None);
    let runtime = Runtime::new(registry.clone(), EchoExecutor)
        .with_security_config(RuntimeSecurityConfig::development());
    let token_store = ApprovalTokenStore::new();
    let quota_tracker = QuotaTracker::new();
    let keys = keys();
    let schedule_limits = ParallelScheduleLimits {
        max_fan_out: 1,
        ..ParallelScheduleLimits::default()
    };

    let proposal_json = diamond_proposal_json("proposal-exec-fan-out");
    let execution_limits = ParallelExecutionLimits::default();
    let request = execution_request!(
        &proposal_json,
        &manifest,
        &registry,
        &schedule_limits,
        &execution_limits,
        None,
        &keys,
    );

    let response = execute_parallel_proposal_via_mcp(
        &runtime,
        &request,
        &token_store,
        &quota_tracker,
        &QuotaLimits::default(),
    )
    .map_err(|e| format!("{e:?}"))?;

    let ProposalExecutionResponse::Denied { code, .. } = response else {
        return Err(format!("expected Denied, got {response:?}"));
    };
    assert_eq!(code, "invalid_parallel_schedule");
    Ok(())
}

#[test]
fn execute_parallel_proposal_via_mcp_denies_a_concurrent_side_effect() -> Result<(), String> {
    let manifest = diamond_manifest(Some("b"));
    let registry = diamond_registry(Some("b"));
    let runtime = Runtime::new(registry.clone(), EchoExecutor)
        .with_security_config(RuntimeSecurityConfig::development());
    let token_store = ApprovalTokenStore::new();
    let quota_tracker = QuotaTracker::new();
    let keys = keys();

    let proposal_json = diamond_proposal_json("proposal-exec-side-effect");
    let schedule_limits = ParallelScheduleLimits::default();
    let execution_limits = ParallelExecutionLimits::default();
    let request = execution_request!(
        &proposal_json,
        &manifest,
        &registry,
        &schedule_limits,
        &execution_limits,
        None,
        &keys,
    );

    let response = execute_parallel_proposal_via_mcp(
        &runtime,
        &request,
        &token_store,
        &quota_tracker,
        &QuotaLimits::default(),
    )
    .map_err(|e| format!("{e:?}"))?;

    let ProposalExecutionResponse::Denied { code, .. } = response else {
        return Err(format!("expected Denied, got {response:?}"));
    };
    assert_eq!(code, "concurrent_side_effect_denied");
    Ok(())
}

#[test]
fn execute_parallel_proposal_via_mcp_denies_execution_without_a_required_approval_token()
-> Result<(), String> {
    // d has an external effect but sits alone in the final wave, so FR-004a
    // does not deny it — it still requires a token via FR-006 (P1 carryover).
    let manifest = diamond_manifest(Some("d"));
    let registry = diamond_registry(Some("d"));
    let runtime = Runtime::new(registry.clone(), EchoExecutor)
        .with_security_config(RuntimeSecurityConfig::development());
    let token_store = ApprovalTokenStore::new();
    let quota_tracker = QuotaTracker::new();
    let keys = keys();

    let proposal_json = diamond_proposal_json("proposal-exec-needs-token");
    let schedule_limits = ParallelScheduleLimits::default();
    let execution_limits = ParallelExecutionLimits::default();
    let request = execution_request!(
        &proposal_json,
        &manifest,
        &registry,
        &schedule_limits,
        &execution_limits,
        None,
        &keys,
    );

    let response = execute_parallel_proposal_via_mcp(
        &runtime,
        &request,
        &token_store,
        &quota_tracker,
        &QuotaLimits::default(),
    )
    .map_err(|e| format!("{e:?}"))?;

    let ProposalExecutionResponse::Denied { code, .. } = response else {
        return Err(format!("expected Denied, got {response:?}"));
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
fn execute_parallel_proposal_via_mcp_succeeds_with_a_valid_approval_token() -> Result<(), String> {
    let manifest = diamond_manifest(Some("d"));
    let registry = diamond_registry(Some("d"));
    let runtime = Runtime::new(registry.clone(), EchoExecutor)
        .with_security_config(RuntimeSecurityConfig::development());
    let token_store = ApprovalTokenStore::new();
    let quota_tracker = QuotaTracker::new();

    let signing_key = SigningKey::from_bytes(&[9_u8; 32]);
    let mut keys = HashMap::new();
    keys.insert("key-1".to_string(), signing_key.verifying_key());

    let proposal_json = diamond_proposal_json("proposal-exec-with-token");
    let proposal_digest = traverse_contracts::proposal_digest(
        &serde_json::from_str::<traverse_contracts::WorkflowProposal>(&proposal_json)
            .map_err(|e| e.to_string())?,
    );
    let snapshots = traverse_contracts::SnapshotDigests {
        manifest_digest: "manifest-1".to_string(),
        registry_digest: "registry-1".to_string(),
        binding_digest: "binding-1".to_string(),
        policy_digest: "policy-1".to_string(),
        budget_digest: "budget-1".to_string(),
    };
    let snapshot_digest =
        traverse_contracts::proposal_snapshot_digest(&proposal_digest, &snapshots);

    let payload = json!({
        "jti": "token-001",
        "iss": "traverse-approval-service",
        "aud": "traverse-runtime",
        "sub": "principal-001",
        "workspace_id": "workspace-001",
        "proposal_digest": proposal_digest,
        "snapshot_digest": snapshot_digest,
        "permitted_effects": ["external_effect"],
        "permitted_connectors": [],
        "max_use_count": 1,
        "exp": 4_102_444_800_i64,
    });
    let token = sign_token(&payload, &signing_key, "key-1");

    let request = ParallelProposalExecutionRequest {
        proposal_json: &proposal_json,
        manifest: &manifest,
        registry: &registry,
        proposal_limits: &ProposalLimits::default(),
        schedule_limits: &ParallelScheduleLimits::default(),
        execution_limits: &ParallelExecutionLimits::default(),
        snapshots: &snapshots,
        approval_token: Some(&token),
        expected_token_issuer: "traverse-approval-service",
        expected_token_audience: "traverse-runtime",
        token_verifying_keys_by_key_id: &keys,
        principal: "principal-001",
        app_id: "test-app",
    };

    let response = execute_parallel_proposal_via_mcp(
        &runtime,
        &request,
        &token_store,
        &quota_tracker,
        &QuotaLimits::default(),
    )
    .map_err(|e| format!("{e:?}"))?;

    let ProposalExecutionResponse::Trace(trace) = response else {
        return Err(format!("expected Trace, got {response:?}"));
    };
    assert_eq!(trace.terminal_state, ProposalTerminalState::Succeeded);
    Ok(())
}

#[test]
fn execute_parallel_proposal_via_mcp_denies_when_quota_is_exhausted() -> Result<(), String> {
    let manifest = diamond_manifest(None);
    let registry = diamond_registry(None);
    let runtime = Runtime::new(registry.clone(), EchoExecutor)
        .with_security_config(RuntimeSecurityConfig::development());
    let token_store = ApprovalTokenStore::new();
    let quota_tracker = QuotaTracker::new();
    let keys = keys();
    let quota_limits = QuotaLimits {
        max_concurrent_per_principal: 0,
        max_concurrent_per_app: 10,
        max_concurrent_per_workspace: 10,
    };

    let proposal_json = diamond_proposal_json("proposal-exec-quota");
    let schedule_limits = ParallelScheduleLimits::default();
    let execution_limits = ParallelExecutionLimits::default();
    let request = execution_request!(
        &proposal_json,
        &manifest,
        &registry,
        &schedule_limits,
        &execution_limits,
        None,
        &keys,
    );

    let response = execute_parallel_proposal_via_mcp(
        &runtime,
        &request,
        &token_store,
        &quota_tracker,
        &quota_limits,
    )
    .map_err(|e| format!("{e:?}"))?;

    let ProposalExecutionResponse::Denied { code, .. } = response else {
        return Err(format!("expected Denied, got {response:?}"));
    };
    assert_eq!(code, "quota_exhausted_principal");
    Ok(())
}

#[test]
fn execute_parallel_proposal_via_mcp_reports_a_failed_terminal_state_when_a_node_fails()
-> Result<(), String> {
    let manifest = diamond_manifest(None);
    let registry = diamond_registry(None);
    let executor = FailsOneCapabilityExecutor {
        failing_capability_id: "test.c".to_string(),
    };
    let runtime = Runtime::new(registry.clone(), executor)
        .with_security_config(RuntimeSecurityConfig::development());
    let token_store = ApprovalTokenStore::new();
    let quota_tracker = QuotaTracker::new();
    let keys = keys();

    let proposal_json = diamond_proposal_json("proposal-exec-node-fails");
    let schedule_limits = ParallelScheduleLimits::default();
    let execution_limits = ParallelExecutionLimits::default();
    let request = execution_request!(
        &proposal_json,
        &manifest,
        &registry,
        &schedule_limits,
        &execution_limits,
        None,
        &keys,
    );

    let response = execute_parallel_proposal_via_mcp(
        &runtime,
        &request,
        &token_store,
        &quota_tracker,
        &QuotaLimits::default(),
    )
    .map_err(|e| format!("{e:?}"))?;

    let ProposalExecutionResponse::Trace(trace) = response else {
        return Err(format!("expected Trace, got {response:?}"));
    };
    assert_eq!(trace.terminal_state, ProposalTerminalState::Failed);
    Ok(())
}

#[test]
fn execute_parallel_proposal_via_mcp_reports_invalid_json_as_an_mcp_error() {
    let manifest = diamond_manifest(None);
    let registry = diamond_registry(None);
    let runtime = Runtime::new(registry.clone(), EchoExecutor)
        .with_security_config(RuntimeSecurityConfig::development());
    let token_store = ApprovalTokenStore::new();
    let quota_tracker = QuotaTracker::new();
    let keys = keys();

    let proposal_json = "not json";
    let schedule_limits = ParallelScheduleLimits::default();
    let execution_limits = ParallelExecutionLimits::default();
    let request = execution_request!(
        &proposal_json,
        &manifest,
        &registry,
        &schedule_limits,
        &execution_limits,
        None,
        &keys,
    );

    let result = execute_parallel_proposal_via_mcp(
        &runtime,
        &request,
        &token_store,
        &quota_tracker,
        &QuotaLimits::default(),
    );
    assert!(result.is_err());
}
