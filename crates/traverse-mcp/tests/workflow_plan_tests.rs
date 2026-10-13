//! End-to-end MCP tests for the declarative workflow planner (P0).
//!
//! Governed by spec `113-declarative-workflow-planning`. Exercises
//! `traverse_mcp::tools::workflow_plan::plan_workflow` against the five
//! acceptance scenarios in the spec: a unique candidate, enumerated
//! ambiguous candidates, zero candidates for a structurally unreachable
//! target, a reviewer-corrected mapping round-tripping through the real P1
//! `submit_proposal` surface unchanged, and search-bound truncation (both
//! the candidate-count and node-depth bounds).

use serde_json::{Value, json};

use traverse_contracts::{
    BinaryFormat as ContractBinaryFormat, CapabilityContract, DataFlowPolicy, DeterminismClass,
    EffectClass, Entrypoint, EntrypointKind, EventReference, Execution, ExecutionConstraints,
    ExecutionTarget, FilesystemAccess, HostApiAccess, Lifecycle, ManifestReference,
    MappingSource as ContractMappingSource, NetworkAccess, Owner, ReliabilityMetadata,
    RiskMetadata, SchemaContainer, ServiceType, SideEffect, SideEffectKind,
};
use traverse_mcp::tools::proposals::submit_proposal;
use traverse_mcp::tools::workflow_plan::{PlanRequest, PlanTarget, plan_workflow};
use traverse_registry::{
    ApplicationBundleManifest, ApplicationComponent, ApplicationComponentRef,
    ApplicationEffectiveConfig, ArtifactDigests, BinaryFormat as RegistryBinaryFormat,
    BinaryReference, CapabilityArtifactRecord, CapabilityRegistration, CapabilityRegistry,
    ComponentExecutionMode, ComposabilityMetadata, CompositionKind, CompositionPattern,
    ImplementationKind, RegistryProvenance, RegistryScope, SourceKind, SourceReference,
    WasmComponentManifest,
};

fn risk() -> RiskMetadata {
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

fn contract_with_schema(
    id: &str,
    version: &str,
    inputs_schema: Value,
    outputs_schema: Value,
    emits: Vec<EventReference>,
) -> CapabilityContract {
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
        summary: "Test capability for workflow planner MCP end-to-end coverage.".to_string(),
        description: "Portable test capability used to exercise the planner MCP surface."
            .to_string(),
        inputs: SchemaContainer {
            schema: inputs_schema,
        },
        outputs: SchemaContainer {
            schema: outputs_schema,
        },
        preconditions: Vec::new(),
        postconditions: Vec::new(),
        side_effects: vec![SideEffect {
            kind: SideEffectKind::MemoryOnly,
            description: "No durable side effect.".to_string(),
        }],
        emits,
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
        risk: risk(),
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

fn registry_with(entries: Vec<CapabilityContract>) -> CapabilityRegistry {
    let mut registry = CapabilityRegistry::new();
    for contract in entries {
        let digest = format!("{}-{}", contract.id, contract.version);
        let outcome = registry.register(CapabilityRegistration {
            scope: RegistryScope::Public,
            contract,
            contract_path: "registry/test/contract.json".to_string(),
            artifact: artifact(&digest),
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

fn manifest_declaring(contracts: &[CapabilityContract]) -> ApplicationBundleManifest {
    ApplicationBundleManifest {
        app_id: "test-app".to_string(),
        version: "1.0.0".to_string(),
        schema_version: "1.0.0".to_string(),
        workspace_defaults: json!({}),
        components: contracts
            .iter()
            .map(|contract| ApplicationComponent {
                reference: ApplicationComponentRef {
                    component_id: contract.id.clone(),
                    version: contract.version.clone(),
                    digest: "sha256:component-digest".to_string(),
                    manifest_path: "component.manifest.json".to_string(),
                },
                manifest_path: "component.manifest.json".into(),
                manifest: WasmComponentManifest {
                    component_id: contract.id.clone(),
                    version: contract.version.clone(),
                    schema_version: "1.0.0".to_string(),
                    execution_mode: ComponentExecutionMode::Wasm,
                    capability_id: contract.id.clone(),
                    capability_version: contract.version.clone(),
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
                contract: contract.clone(),
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

fn app_manifest_reference() -> ManifestReference {
    ManifestReference {
        app_id: "test-app".to_string(),
        app_version: "1.0.0".to_string(),
        manifest_digest: "sha256:manifest-digest".to_string(),
    }
}

fn object_schema(properties: &[(&str, &str)], required: &[&str]) -> Value {
    let mut props = serde_json::Map::new();
    for (name, json_type) in properties {
        props.insert((*name).to_string(), json!({"type": json_type}));
    }
    json!({
        "type": "object",
        "properties": Value::Object(props),
        "required": required,
    })
}

fn empty_schema() -> Value {
    json!({"type": "object", "properties": {}, "required": []})
}

/// Acceptance scenario 1: exactly one declared capability's `outputs.schema`
/// structurally satisfies the target's `inputs.schema.required` -- the
/// planner returns exactly one candidate plan, mapping flagged unconfirmed.
#[test]
fn plan_workflow_returns_unique_candidate_when_exactly_one_producer_matches() {
    let producer = contract_with_schema(
        "test.producer",
        "1.0.0",
        empty_schema(),
        object_schema(&[("result", "string")], &[]),
        Vec::new(),
    );
    let consumer = contract_with_schema(
        "test.consumer",
        "1.0.0",
        object_schema(&[("result", "string")], &["result"]),
        empty_schema(),
        Vec::new(),
    );
    let registry = registry_with(vec![producer.clone(), consumer.clone()]);
    let manifest = manifest_declaring(&[producer, consumer]);
    let app_manifest = app_manifest_reference();
    let target = PlanTarget::Capability {
        capability_id: "test.consumer".to_string(),
        capability_version: "1.0.0".to_string(),
    };
    let starting_facts = json!({});

    let response = plan_workflow(&PlanRequest {
        target: &target,
        starting_facts: &starting_facts,
        manifest: &manifest,
        app_manifest: &app_manifest,
        registry: &registry,
        workspace_id: "workspace-001",
    });

    assert!(!response.plan_search_truncated);
    assert_eq!(response.candidates.len(), 1);
    let candidate = &response.candidates[0];
    assert!(candidate.mapping_unconfirmed);
    assert_eq!(candidate.proposal.nodes.len(), 2);
    assert_eq!(candidate.proposal.nodes[0].capability_id, "test.producer");
    assert_eq!(candidate.proposal.nodes[1].capability_id, "test.consumer");
    assert_eq!(candidate.proposal.edges.len(), 1);
    assert_eq!(candidate.proposal.mappings.len(), 1);
    assert_eq!(
        candidate.proposal.mappings[0].source,
        ContractMappingSource::Node {
            node_id: "n0".to_string()
        }
    );
    assert_eq!(candidate.proposal.mappings[0].source_path, "/result");
    assert_eq!(candidate.proposal.mappings[0].target_path, "/result");
}

/// Acceptance scenario 2: two declared capabilities' `outputs.schema` both
/// structurally satisfy the same downstream need -- the planner enumerates
/// both complete candidate plans rather than picking one.
#[test]
fn plan_workflow_enumerates_ambiguous_candidates_instead_of_choosing() {
    let producer_a = contract_with_schema(
        "test.producer-a",
        "1.0.0",
        empty_schema(),
        object_schema(&[("result", "string")], &[]),
        Vec::new(),
    );
    let producer_b = contract_with_schema(
        "test.producer-b",
        "1.0.0",
        empty_schema(),
        object_schema(&[("result", "string")], &[]),
        Vec::new(),
    );
    let consumer = contract_with_schema(
        "test.consumer",
        "1.0.0",
        object_schema(&[("result", "string")], &["result"]),
        empty_schema(),
        Vec::new(),
    );
    let registry = registry_with(vec![
        producer_a.clone(),
        producer_b.clone(),
        consumer.clone(),
    ]);
    let manifest = manifest_declaring(&[producer_a, producer_b, consumer]);
    let app_manifest = app_manifest_reference();
    let target = PlanTarget::Capability {
        capability_id: "test.consumer".to_string(),
        capability_version: "1.0.0".to_string(),
    };
    let starting_facts = json!({});

    let response = plan_workflow(&PlanRequest {
        target: &target,
        starting_facts: &starting_facts,
        manifest: &manifest,
        app_manifest: &app_manifest,
        registry: &registry,
        workspace_id: "workspace-001",
    });

    assert!(!response.plan_search_truncated);
    assert_eq!(response.candidates.len(), 2);
    let mut producer_ids: Vec<&str> = response
        .candidates
        .iter()
        .map(|candidate| candidate.proposal.nodes[0].capability_id.as_str())
        .collect();
    producer_ids.sort_unstable();
    assert_eq!(producer_ids, ["test.producer-a", "test.producer-b"]);
}

/// Acceptance scenario 3: no declared capability's `outputs.schema`
/// structurally satisfies the target -- the planner returns zero candidates
/// rather than a namespace/verb-name guess.
#[test]
fn plan_workflow_returns_zero_candidates_when_nothing_structurally_matches() {
    let unrelated = contract_with_schema(
        "test.unrelated",
        "1.0.0",
        empty_schema(),
        object_schema(&[("other_field", "string")], &[]),
        Vec::new(),
    );
    let consumer = contract_with_schema(
        "test.consumer",
        "1.0.0",
        object_schema(&[("result", "string")], &["result"]),
        empty_schema(),
        Vec::new(),
    );
    let registry = registry_with(vec![unrelated.clone(), consumer.clone()]);
    let manifest = manifest_declaring(&[unrelated, consumer]);
    let app_manifest = app_manifest_reference();
    let target = PlanTarget::Capability {
        capability_id: "test.consumer".to_string(),
        capability_version: "1.0.0".to_string(),
    };
    let starting_facts = json!({});

    let response = plan_workflow(&PlanRequest {
        target: &target,
        starting_facts: &starting_facts,
        manifest: &manifest,
        app_manifest: &app_manifest,
        registry: &registry,
        workspace_id: "workspace-001",
    });

    assert!(!response.plan_search_truncated);
    assert!(response.candidates.is_empty());
}

/// Acceptance scenario 4: a reviewer corrects the planner's field mapping
/// before submission -- the corrected mapping, not the planner's guess, is
/// what actually gets submitted to and validated by the real P1 surface.
#[test]
fn plan_workflow_candidate_mapping_can_be_corrected_and_submitted_through_p1() -> Result<(), String>
{
    let producer = contract_with_schema(
        "test.producer",
        "1.0.0",
        empty_schema(),
        object_schema(&[("payload", "string")], &[]),
        Vec::new(),
    );
    let consumer = contract_with_schema(
        "test.consumer",
        "1.0.0",
        object_schema(&[("payload", "string")], &["payload"]),
        empty_schema(),
        Vec::new(),
    );
    let registry = registry_with(vec![producer.clone(), consumer.clone()]);
    let manifest = manifest_declaring(&[producer, consumer]);
    let app_manifest = app_manifest_reference();
    let target = PlanTarget::Capability {
        capability_id: "test.consumer".to_string(),
        capability_version: "1.0.0".to_string(),
    };
    let starting_facts = json!({});

    let response = plan_workflow(&PlanRequest {
        target: &target,
        starting_facts: &starting_facts,
        manifest: &manifest,
        app_manifest: &app_manifest,
        registry: &registry,
        workspace_id: "workspace-001",
    });
    assert_eq!(response.candidates.len(), 1);

    // The planner guessed source_path "/payload" -- a reviewer corrects it
    // to a (still valid) explicit path before submission.
    let mut corrected = response.candidates[0].proposal.clone();
    corrected.mappings[0].source_path = "/payload".to_string();
    corrected.mappings[0].target_path = "/payload".to_string();
    let corrected_json = serde_json::to_string(&corrected).map_err(|error| format!("{error:?}"))?;

    let submission = submit_proposal(
        &corrected_json,
        &manifest,
        &registry,
        &traverse_contracts::ProposalLimits::default(),
        &traverse_contracts::SnapshotDigests {
            manifest_digest: "sha256:manifest-digest".to_string(),
            registry_digest: "sha256:registry-digest".to_string(),
            binding_digest: "sha256:binding-digest".to_string(),
            policy_digest: "sha256:policy-digest".to_string(),
            budget_digest: "sha256:budget-digest".to_string(),
        },
    )
    .map_err(|error| format!("{error:?}"))?;

    assert!(
        submission.valid,
        "corrected planner candidate must validate cleanly against the real P1 surface: {:?}",
        submission.errors
    );
    Ok(())
}

/// Acceptance scenario 5 (candidate-count bound): more than 5 declared
/// capabilities independently satisfy the same downstream need -- the
/// planner returns only 5 and flags `plan_search_truncated: true`.
#[test]
fn plan_workflow_truncates_when_more_than_five_candidates_are_valid() {
    let producers: Vec<CapabilityContract> = (0..6)
        .map(|index| {
            contract_with_schema(
                &format!("test.producer-{index}"),
                "1.0.0",
                empty_schema(),
                object_schema(&[("result", "string")], &[]),
                Vec::new(),
            )
        })
        .collect();
    let consumer = contract_with_schema(
        "test.consumer",
        "1.0.0",
        object_schema(&[("result", "string")], &["result"]),
        empty_schema(),
        Vec::new(),
    );
    let mut all_contracts = producers.clone();
    all_contracts.push(consumer.clone());
    let registry = registry_with(all_contracts.clone());
    let manifest = manifest_declaring(&all_contracts);
    let app_manifest = app_manifest_reference();
    let target = PlanTarget::Capability {
        capability_id: "test.consumer".to_string(),
        capability_version: "1.0.0".to_string(),
    };
    let starting_facts = json!({});

    let response = plan_workflow(&PlanRequest {
        target: &target,
        starting_facts: &starting_facts,
        manifest: &manifest,
        app_manifest: &app_manifest,
        registry: &registry,
        workspace_id: "workspace-001",
    });

    assert!(response.plan_search_truncated);
    assert_eq!(response.candidates.len(), 5);
}

/// Acceptance scenario 5 (node-depth bound): the only structurally valid
/// chain to the target needs 9 nodes, one past the 8-node-deep bound -- the
/// planner returns the bounded (empty) subset plus `plan_search_truncated:
/// true`, never an unbounded result or a silent failure.
#[test]
fn plan_workflow_truncates_when_the_only_valid_chain_exceeds_the_node_depth_bound() {
    // A 9-node linear chain: p0 (root, satisfied by starting facts) -> p1
    // -> ... -> p8 (the target). Each link needs exactly the prior node's
    // single output field.
    let mut contracts = Vec::new();
    for index in 0..9 {
        let field = format!("field{index}");
        let next_field = format!("field{}", index + 1);
        let inputs = object_schema(&[(&field, "string")], &[&field]);
        let outputs = object_schema(&[(&next_field, "string")], &[]);
        contracts.push(contract_with_schema(
            &format!("test.chain-{index}"),
            "1.0.0",
            inputs,
            outputs,
            Vec::new(),
        ));
    }
    let registry = registry_with(contracts.clone());
    let manifest = manifest_declaring(&contracts);
    let app_manifest = app_manifest_reference();
    let target = PlanTarget::Capability {
        capability_id: "test.chain-8".to_string(),
        capability_version: "1.0.0".to_string(),
    };
    let starting_facts = json!({"field0": "seed"});

    let response = plan_workflow(&PlanRequest {
        target: &target,
        starting_facts: &starting_facts,
        manifest: &manifest,
        app_manifest: &app_manifest,
        registry: &registry,
        workspace_id: "workspace-001",
    });

    assert!(response.plan_search_truncated);
    assert!(response.candidates.is_empty());
}

/// The `EmitsEvent` target form (spec 113 FR-001): the planner selects the
/// declared capability whose contract emits the requested event type.
#[test]
fn plan_workflow_matches_target_by_emitted_event_type() {
    let emitter = contract_with_schema(
        "test.emitter",
        "1.0.0",
        empty_schema(),
        empty_schema(),
        vec![EventReference {
            event_id: "test.something-happened".to_string(),
            version: "1.0.0".to_string(),
        }],
    );
    let registry = registry_with(vec![emitter.clone()]);
    let manifest = manifest_declaring(&[emitter]);
    let app_manifest = app_manifest_reference();
    let target = PlanTarget::EmitsEvent {
        event_type: "test.something-happened".to_string(),
    };
    let starting_facts = json!({});

    let response = plan_workflow(&PlanRequest {
        target: &target,
        starting_facts: &starting_facts,
        manifest: &manifest,
        app_manifest: &app_manifest,
        registry: &registry,
        workspace_id: "workspace-001",
    });

    assert!(!response.plan_search_truncated);
    assert_eq!(response.candidates.len(), 1);
    assert_eq!(
        response.candidates[0].proposal.nodes[0].capability_id,
        "test.emitter"
    );
}

/// A 0-hop candidate: the target's own required inputs are all satisfiable
/// directly from starting facts, producing `InitialInput` mappings at the
/// chain's root. Also exercises every JSON-Schema `type` the starting-facts
/// synthesis maps a value to (null/boolean/integer/number/string/array/
/// object), since it types every key in `starting_facts`, not only the ones
/// a required property actually names.
#[test]
fn plan_workflow_root_node_maps_directly_from_richly_typed_starting_facts() {
    let target_capability = contract_with_schema(
        "test.direct",
        "1.0.0",
        object_schema(&[("text_field", "string")], &["text_field"]),
        empty_schema(),
        Vec::new(),
    );
    let registry = registry_with(vec![target_capability.clone()]);
    let manifest = manifest_declaring(&[target_capability]);
    let app_manifest = app_manifest_reference();
    let target = PlanTarget::Capability {
        capability_id: "test.direct".to_string(),
        capability_version: "1.0.0".to_string(),
    };
    let starting_facts = json!({
        "text_field": "value",
        "flag_field": true,
        "int_field": 5,
        "float_field": 1.5,
        "array_field": [1, 2],
        "object_field": {},
        "null_field": null,
    });

    let response = plan_workflow(&PlanRequest {
        target: &target,
        starting_facts: &starting_facts,
        manifest: &manifest,
        app_manifest: &app_manifest,
        registry: &registry,
        workspace_id: "workspace-001",
    });

    assert!(!response.plan_search_truncated);
    assert_eq!(response.candidates.len(), 1);
    let candidate = &response.candidates[0];
    assert_eq!(candidate.proposal.nodes.len(), 1);
    assert!(candidate.proposal.edges.is_empty());
    assert_eq!(candidate.proposal.mappings.len(), 1);
    assert_eq!(
        candidate.proposal.mappings[0].source,
        ContractMappingSource::InitialInput
    );
    assert_eq!(candidate.proposal.mappings[0].source_path, "/text_field");
}

/// A manifest-declared component that was never actually registered in the
/// capability registry must be silently excluded from the candidate universe
/// rather than crashing the planner or being placed in a node.
#[test]
fn plan_workflow_skips_a_declared_component_that_is_not_registered() {
    let producer = contract_with_schema(
        "test.producer",
        "1.0.0",
        empty_schema(),
        object_schema(&[("result", "string")], &[]),
        Vec::new(),
    );
    let consumer = contract_with_schema(
        "test.consumer",
        "1.0.0",
        object_schema(&[("result", "string")], &["result"]),
        empty_schema(),
        Vec::new(),
    );
    // Registered in the registry, so both resolve.
    let registry = registry_with(vec![producer.clone(), consumer.clone()]);
    // Declared in the manifest too, plus one extra component the registry
    // has never seen.
    let ghost = contract_with_schema(
        "test.ghost",
        "9.9.9",
        empty_schema(),
        empty_schema(),
        Vec::new(),
    );
    let manifest = manifest_declaring(&[producer, consumer, ghost]);
    let app_manifest = app_manifest_reference();
    let target = PlanTarget::Capability {
        capability_id: "test.consumer".to_string(),
        capability_version: "1.0.0".to_string(),
    };
    let starting_facts = json!({});

    let response = plan_workflow(&PlanRequest {
        target: &target,
        starting_facts: &starting_facts,
        manifest: &manifest,
        app_manifest: &app_manifest,
        registry: &registry,
        workspace_id: "workspace-001",
    });

    assert!(!response.plan_search_truncated);
    assert_eq!(response.candidates.len(), 1);
    assert_eq!(response.candidates[0].proposal.nodes.len(), 2);
    assert!(
        response.candidates.iter().all(|candidate| candidate
            .proposal
            .nodes
            .iter()
            .all(|node| node.capability_id != "test.ghost")),
        "an unregistered manifest component must never appear in a candidate node"
    );
}

/// Spec 113 FR-008: planning must be pure/read-only -- calling it twice with
/// identical inputs must produce byte-identical candidates (no registry,
/// manifest, or catalog mutation, no hidden nondeterminism).
#[test]
fn plan_workflow_is_deterministic_across_repeated_calls() -> Result<(), String> {
    let producer = contract_with_schema(
        "test.producer",
        "1.0.0",
        empty_schema(),
        object_schema(&[("result", "string")], &[]),
        Vec::new(),
    );
    let consumer = contract_with_schema(
        "test.consumer",
        "1.0.0",
        object_schema(&[("result", "string")], &["result"]),
        empty_schema(),
        Vec::new(),
    );
    let registry = registry_with(vec![producer.clone(), consumer.clone()]);
    let manifest = manifest_declaring(&[producer, consumer]);
    let app_manifest = app_manifest_reference();
    let target = PlanTarget::Capability {
        capability_id: "test.consumer".to_string(),
        capability_version: "1.0.0".to_string(),
    };
    let starting_facts = json!({});
    let request = PlanRequest {
        target: &target,
        starting_facts: &starting_facts,
        manifest: &manifest,
        app_manifest: &app_manifest,
        registry: &registry,
        workspace_id: "workspace-001",
    };

    let first = plan_workflow(&request);
    let second = plan_workflow(&request);

    let first_json =
        serde_json::to_string(&first.candidates).map_err(|error| format!("{error:?}"))?;
    let second_json =
        serde_json::to_string(&second.candidates).map_err(|error| format!("{error:?}"))?;
    assert_eq!(first_json, second_json);
    assert_eq!(first.plan_search_truncated, second.plan_search_truncated);
    Ok(())
}
