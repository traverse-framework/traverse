//! End-to-end tests for exporting a completed runtime workflow proposal
//! into a candidate workflow artifact and promoting it through the
//! existing, unchanged workflow registration path (spec
//! `112-governed-workflow-promotion`, P4, ADR-0042).
//!
//! The key end-to-end scenario the spec's own Definition of Done names is
//! "export -> review fixture -> published workflow discovery" — this file
//! proves exactly that using the real `traverse_registry::WorkflowRegistry`,
//! not a mock.

use serde_json::json;

use traverse_contracts::{
    BinaryFormat as ContractBinaryFormat, CanonicalProposal, CapabilityContract, DataFlowPolicy,
    DeterminismClass, EffectClass, Entrypoint, EntrypointKind, Execution, ExecutionConstraints,
    ExecutionTarget, FilesystemAccess, HostApiAccess, Lifecycle, ManifestReference, MappingSource,
    NetworkAccess, Owner, ProposalEdge, ProposalLimits, ProposalMapping, ProposalNode,
    ReliabilityMetadata, RiskMetadata, SchemaContainer, ServiceType, SideEffect, SideEffectKind,
    WorkflowProposal, canonicalize_proposal, proposal_digest,
};
use traverse_mcp::tools::workflow_promotion::{
    PromotedWorkflowIdentity, export_workflow_candidate, finalize_candidate_into_definition,
};
use traverse_registry::{
    ApplicationBundleManifest, ApplicationComponent, ApplicationComponentRef,
    ApplicationEffectiveConfig, ArtifactDigests, BinaryFormat as RegistryBinaryFormat,
    BinaryReference, CapabilityArtifactRecord, CapabilityRegistration, CapabilityRegistry,
    ComponentExecutionMode, ComposabilityMetadata, CompositionKind, CompositionPattern,
    ImplementationKind, LookupScope, RegistryProvenance, RegistryScope, SourceKind,
    SourceReference, WasmComponentManifest, WorkflowRegistration, WorkflowRegistry,
};
use traverse_runtime::proposal::{
    AuthorizationSummary, ProposalTerminalState, execute_proposal,
    validate_proposal_against_host_state,
};
use traverse_runtime::security::RuntimeSecurityConfig;
use traverse_runtime::{LocalExecutionFailure, LocalExecutionOutput, LocalExecutor, Runtime};

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

fn contract(id: &str) -> CapabilityContract {
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
        summary: "Test capability for workflow promotion coverage.".to_string(),
        description: "Portable test capability used to exercise workflow promotion export."
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
        risk: risk(EffectClass::PureRead),
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

fn manifest_declaring(components: &[&str]) -> ApplicationBundleManifest {
    ApplicationBundleManifest {
        app_id: "test-app".to_string(),
        version: "1.0.0".to_string(),
        schema_version: "1.0.0".to_string(),
        workspace_defaults: json!({}),
        components: components
            .iter()
            .map(|capability_id| ApplicationComponent {
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
                contract: contract(capability_id),
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

fn node(node_id: &str, capability_id: &str) -> ProposalNode {
    ProposalNode {
        node_id: node_id.to_string(),
        capability_id: capability_id.to_string(),
        capability_version: "1.0.0".to_string(),
        artifact_digest: format!("digest-{node_id}"),
    }
}

/// A strictly linear a -> b proposal, mapping `/value` to `/value` — the
/// shared field name lets the export cleanly reduce the mapping to a
/// shared state key.
fn linear_proposal() -> WorkflowProposal {
    WorkflowProposal {
        kind: "workflow_proposal".to_string(),
        schema_version: "1.0.0".to_string(),
        proposal_id: "proposal-promo-001".to_string(),
        workspace_id: "workspace-001".to_string(),
        app_manifest: ManifestReference {
            app_id: "test-app".to_string(),
            app_version: "1.0.0".to_string(),
            manifest_digest: "sha256:manifest-digest".to_string(),
        },
        nodes: vec![node("a", "test.a"), node("b", "test.b")],
        edges: vec![ProposalEdge {
            from_node_id: "a".to_string(),
            to_node_id: "b".to_string(),
        }],
        mappings: vec![ProposalMapping {
            source: MappingSource::Node {
                node_id: "a".to_string(),
            },
            source_path: "/value".to_string(),
            target_node_id: "b".to_string(),
            target_path: "/value".to_string(),
        }],
        initial_input: json!({}),
    }
}

/// Same shape as `linear_proposal`, but the mapping renames the field
/// (`/draft_id` -> `/value`), which cannot be reduced to a shared state key.
fn linear_proposal_with_a_rename() -> WorkflowProposal {
    let mut proposal = linear_proposal();
    proposal.mappings[0].source_path = "/draft_id".to_string();
    proposal
}

/// a fans out to b and c — more than one direct outgoing edge from `a`,
/// which the existing `WorkflowRegistry` registration validation rejects.
fn diamond_proposal() -> WorkflowProposal {
    let mut proposal = linear_proposal();
    let mut c = node("c", "test.a");
    c.artifact_digest = "digest-a".to_string(); // shares capability test.a with node "a"
    proposal.nodes.push(c);
    proposal.edges.push(ProposalEdge {
        from_node_id: "a".to_string(),
        to_node_id: "c".to_string(),
    });
    proposal.mappings.clear();
    proposal
}

struct EchoExecutor;

impl LocalExecutor for EchoExecutor {
    fn execute(
        &self,
        _capability: &traverse_registry::ResolvedCapability,
        input: &serde_json::Value,
    ) -> Result<LocalExecutionOutput, LocalExecutionFailure> {
        Ok(LocalExecutionOutput {
            value: input.clone(),
            emitted_events: Vec::new(),
        })
    }
}

fn two_capability_registry() -> CapabilityRegistry {
    registry_with(vec![
        (contract("test.a"), artifact("digest-a")),
        (contract("test.b"), artifact("digest-b")),
    ])
}

fn run_to_success(
    proposal: WorkflowProposal,
    manifest: &ApplicationBundleManifest,
    registry: &CapabilityRegistry,
) -> Result<(CanonicalProposal, traverse_runtime::proposal::ProposalTrace), String> {
    let canonical = canonicalize_proposal(proposal, &ProposalLimits::default())
        .map_err(|e| format!("{e:?}"))?;
    let resolved = validate_proposal_against_host_state(&canonical, manifest, registry)
        .map_err(|e| format!("{e:?}"))?;
    let runtime = Runtime::new(registry.clone(), EchoExecutor)
        .with_security_config(RuntimeSecurityConfig::development());
    let digest = proposal_digest(&canonical.proposal);
    let trace = execute_proposal(
        &runtime,
        &canonical,
        &resolved,
        AuthorizationSummary {
            automatic: true,
            approval_token_id: None,
        },
        &digest,
        "snapshot-digest",
    );
    if trace.terminal_state != ProposalTerminalState::Succeeded {
        return Err(format!(
            "expected Succeeded, got {:?}",
            trace.terminal_state
        ));
    }
    Ok((canonical, trace))
}

fn reviewer_identity() -> PromotedWorkflowIdentity {
    PromotedWorkflowIdentity {
        id: "promoted-workflow-001".to_string(),
        name: "Promoted Test Workflow".to_string(),
        version: "1.0.0".to_string(),
        owner: Owner {
            team: "traverse-core".to_string(),
            contact: "enrico.piovesan10@gmail.com".to_string(),
        },
        lifecycle: Lifecycle::Active,
        summary: "A promoted workflow reviewed and published from a proposal.".to_string(),
        tags: vec!["promoted".to_string()],
    }
}

#[test]
fn export_then_promote_then_discover_end_to_end() -> Result<(), String> {
    let manifest = manifest_declaring(&["test.a", "test.b"]);
    let registry = two_capability_registry();
    let (canonical, trace) = run_to_success(linear_proposal(), &manifest, &registry)?;
    let resolved = validate_proposal_against_host_state(&canonical, &manifest, &registry)
        .map_err(|e| format!("{e:?}"))?;

    let candidate =
        export_workflow_candidate(&canonical, &trace, &resolved).map_err(|e| format!("{e:?}"))?;
    assert!(candidate.unconfirmed_mappings.is_empty());
    assert_eq!(candidate.start_node, "a");
    assert_eq!(candidate.terminal_nodes, vec!["b".to_string()]);
    assert!(
        candidate
            .excluded_fields
            .iter()
            .any(|f| f == "initial_input")
    );

    let definition = finalize_candidate_into_definition(&candidate, reviewer_identity());

    let mut workflow_registry = WorkflowRegistry::new();
    let outcome = workflow_registry
        .register(
            &registry,
            WorkflowRegistration {
                scope: RegistryScope::Public,
                definition,
                workflow_path: "workflows/promoted-workflow-001.json".to_string(),
                registered_at: "2026-08-23T00:00:00Z".to_string(),
                validator_version: "0.1.0".to_string(),
            },
        )
        .map_err(|e| format!("registration must succeed: {e:?}"))?;
    assert_eq!(outcome.record.id, "promoted-workflow-001");

    let resolved_workflow = workflow_registry
        .find_exact(LookupScope::PublicOnly, "promoted-workflow-001", "1.0.0")
        .ok_or_else(|| "promoted workflow must be discoverable by exact lookup".to_string())?;
    assert_eq!(resolved_workflow.definition.nodes.len(), 2);

    let discovered = workflow_registry.discover(LookupScope::PublicOnly);
    assert!(
        discovered
            .iter()
            .any(|entry| entry.id == "promoted-workflow-001"),
        "promoted workflow must appear in discovery listing"
    );
    Ok(())
}

#[test]
fn export_rejects_a_proposal_that_did_not_succeed() -> Result<(), String> {
    let manifest = manifest_declaring(&["test.a", "test.b"]);
    let registry = two_capability_registry();
    let canonical = canonicalize_proposal(linear_proposal(), &ProposalLimits::default())
        .map_err(|e| format!("{e:?}"))?;
    let resolved = validate_proposal_against_host_state(&canonical, &manifest, &registry)
        .map_err(|e| format!("{e:?}"))?;

    let runtime = Runtime::new(registry, FailingExecutor)
        .with_security_config(RuntimeSecurityConfig::development());
    let digest = proposal_digest(&canonical.proposal);
    let trace = execute_proposal(
        &runtime,
        &canonical,
        &resolved,
        AuthorizationSummary {
            automatic: true,
            approval_token_id: None,
        },
        &digest,
        "snapshot-digest",
    );
    assert_eq!(trace.terminal_state, ProposalTerminalState::Failed);

    let result = export_workflow_candidate(&canonical, &trace, &resolved);
    assert!(result.is_err());
    Ok(())
}

struct FailingExecutor;

impl LocalExecutor for FailingExecutor {
    fn execute(
        &self,
        _capability: &traverse_registry::ResolvedCapability,
        _input: &serde_json::Value,
    ) -> Result<LocalExecutionOutput, LocalExecutionFailure> {
        Err(LocalExecutionFailure {
            code: traverse_runtime::LocalExecutionFailureCode::ExecutionFailed,
            message: "always fails".to_string(),
        })
    }
}

#[test]
fn export_flags_a_renamed_mapping_as_unconfirmed() -> Result<(), String> {
    let manifest = manifest_declaring(&["test.a", "test.b"]);
    let registry = two_capability_registry();
    let (canonical, trace) = run_to_success(linear_proposal_with_a_rename(), &manifest, &registry)?;
    let resolved = validate_proposal_against_host_state(&canonical, &manifest, &registry)
        .map_err(|e| format!("{e:?}"))?;

    let candidate =
        export_workflow_candidate(&canonical, &trace, &resolved).map_err(|e| format!("{e:?}"))?;
    assert_eq!(candidate.unconfirmed_mappings.len(), 1);
    assert_eq!(candidate.unconfirmed_mappings[0].source_path, "/draft_id");
    // Since the mapping did not reduce, neither node gets a state-key wire.
    assert!(candidate.nodes[0].to_workflow_state.is_empty());
    assert!(candidate.nodes[1].from_workflow_input.is_empty());
    Ok(())
}

#[test]
fn export_preserves_each_nodes_declared_risk_for_review() -> Result<(), String> {
    let manifest = manifest_declaring(&["test.a", "test.b"]);
    let registry = two_capability_registry();
    let (canonical, trace) = run_to_success(linear_proposal(), &manifest, &registry)?;
    let resolved = validate_proposal_against_host_state(&canonical, &manifest, &registry)
        .map_err(|e| format!("{e:?}"))?;

    let candidate =
        export_workflow_candidate(&canonical, &trace, &resolved).map_err(|e| format!("{e:?}"))?;
    for candidate_node in &candidate.nodes {
        let risk = candidate_node
            .risk
            .as_ref()
            .ok_or_else(|| format!("node '{}' must have resolved risk", candidate_node.node_id))?;
        assert_eq!(risk.effect_class, EffectClass::PureRead);
    }
    Ok(())
}

#[test]
fn a_branching_proposals_candidate_is_honestly_unregistrable() -> Result<(), String> {
    // Documents the real, existing WorkflowDefinition v0.1 constraint (at
    // most one direct outgoing edge per node) rather than hiding it: this
    // export is not "buggy," the *target format* cannot yet express fan-out.
    let manifest = manifest_declaring(&["test.a", "test.b"]);
    let registry = two_capability_registry();
    let (canonical, trace) = run_to_success(diamond_proposal(), &manifest, &registry)?;
    let resolved = validate_proposal_against_host_state(&canonical, &manifest, &registry)
        .map_err(|e| format!("{e:?}"))?;

    let candidate =
        export_workflow_candidate(&canonical, &trace, &resolved).map_err(|e| format!("{e:?}"))?;
    assert_eq!(candidate.edges.len(), 2); // a->b and a->c, honestly preserved

    let definition = finalize_candidate_into_definition(&candidate, reviewer_identity());
    let mut workflow_registry = WorkflowRegistry::new();
    let result = workflow_registry.register(
        &registry,
        WorkflowRegistration {
            scope: RegistryScope::Public,
            definition,
            workflow_path: "workflows/branching.json".to_string(),
            registered_at: "2026-08-23T00:00:00Z".to_string(),
            validator_version: "0.1.0".to_string(),
        },
    );
    assert!(
        result.is_err(),
        "a branching candidate must be rejected by the existing single-outgoing-edge constraint"
    );
    Ok(())
}
