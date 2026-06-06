#![allow(clippy::expect_used)]

use serde_json::json;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use traverse_contracts::{
    BinaryFormat as ContractBinaryFormat, CapabilityContract, Condition, ConnectorRequirement,
    DependencyArtifactType, DependencyReference, Entrypoint, EntrypointKind, EventClassification,
    EventContract, EventPayload, EventProvenance, EventProvenanceSource, EventReference, EventType,
    EvidenceStatus, EvidenceType, Execution, ExecutionConstraints, ExecutionTarget,
    FilesystemAccess, HostApiAccess, IdReference, Lifecycle, NetworkAccess, Owner,
    PayloadCompatibility, Provenance, ProvenanceSource, SchemaContainer, ServiceType, SideEffect,
    SideEffectKind, ValidationEvidence, reference_connector_contracts,
};
use traverse_registry::{
    ArtifactDigests, BinaryFormat, BinaryReference, BundleLoadErrorCode, CapabilityArtifactRecord,
    CapabilityRegistration, CapabilityRegistry, ComposabilityMetadata, CompositionKind,
    CompositionPattern, ConnectorRegistration, DiscoveryQuery, EventRegistration, EventRegistry,
    EventRegistryErrorCode, ImplementationKind, LookupScope, RegistryErrorCode, RegistryProvenance,
    RegistryScope, SourceKind, SourceReference, WorkflowReference, load_registry_bundle,
    resolve_version_range,
};

#[test]
fn registers_and_finds_public_executable_capability() {
    let mut registry = CapabilityRegistry::new();
    let request = executable_registration(
        RegistryScope::Public,
        base_contract("content.comments.create-comment-draft", "1.0.0"),
    );

    let outcome = registry
        .register(request)
        .expect("registration should pass");
    let resolved = registry
        .find_exact(
            LookupScope::PublicOnly,
            "content.comments.create-comment-draft",
            "1.0.0",
        )
        .expect("capability should resolve");

    assert_eq!(resolved.record, outcome.record);
    assert_eq!(resolved.artifact, outcome.artifact);
    assert_eq!(resolved.index_entry, outcome.index_entry);
    assert_eq!(resolved.record.scope, RegistryScope::Public);
}

#[test]
fn duplicate_identical_registration_is_idempotent() {
    let mut registry = CapabilityRegistry::new();
    let request = executable_registration(
        RegistryScope::Public,
        base_contract("content.comments.create-comment-draft", "1.0.0"),
    );

    let first = registry
        .register(request.clone())
        .expect("first registration should pass");
    let second = registry
        .register(request)
        .expect("duplicate identical registration should be a no-op");

    assert_eq!(first.record, second.record);
    assert_eq!(first.index_entry, second.index_entry);
}

#[test]
fn registers_and_discovers_reference_connector() {
    let mut registry = CapabilityRegistry::new();
    let connector = reference_connector_contracts()
        .into_iter()
        .find(|contract| contract.connector_id == "traverse.http")
        .expect("traverse.http reference connector should exist");

    let record = registry
        .register_connector(connector_registration(RegistryScope::Public, connector))
        .expect("connector registration should pass");
    let discovered =
        registry.discover_connectors(LookupScope::PublicOnly, "traverse.http", "^1.0.0");

    assert_eq!(discovered, vec![record]);
}

#[test]
fn connector_registration_rejects_invalid_contract() {
    let mut registry = CapabilityRegistry::new();
    let mut connector = reference_connector_contracts()
        .into_iter()
        .next()
        .expect("reference connector should exist");
    connector.connector_id.clear();

    let failure = registry
        .register_connector(connector_registration(RegistryScope::Public, connector))
        .expect_err("invalid connector contract should fail");

    assert!(failure.errors.iter().any(|error| {
        error.code == RegistryErrorCode::InvalidConnectorContract
            && error.target == "$.connector_id"
    }));
}

#[test]
fn connector_registration_is_idempotent_and_rejects_conflicts() {
    let mut registry = CapabilityRegistry::new();
    let connector = reference_connector_contracts()
        .into_iter()
        .find(|contract| contract.connector_id == "traverse.env")
        .expect("traverse.env reference connector should exist");
    let request = connector_registration(RegistryScope::Public, connector.clone());

    let first = registry
        .register_connector(request.clone())
        .expect("connector registration should pass");
    let second = registry
        .register_connector(request)
        .expect("identical connector registration should be idempotent");
    assert_eq!(first, second);

    let mut conflicting_request = connector_registration(RegistryScope::Public, connector);
    conflicting_request.registered_at = "2026-04-20T00:00:00Z".to_string();
    let failure = registry
        .register_connector(conflicting_request)
        .expect_err("different connector metadata should conflict");
    assert_eq!(
        failure.errors[0].code,
        RegistryErrorCode::ImmutableVersionConflict
    );
}

#[test]
fn connector_discovery_handles_invalid_ranges_and_private_shadowing() {
    let mut registry = CapabilityRegistry::new();
    let connector = reference_connector_contracts()
        .into_iter()
        .find(|contract| contract.connector_id == "traverse.env")
        .expect("traverse.env reference connector should exist");
    registry
        .register_connector(connector_registration(
            RegistryScope::Public,
            connector.clone(),
        ))
        .expect("public connector registration should pass");
    registry
        .register_connector(connector_registration(RegistryScope::Private, connector))
        .expect("private connector registration should pass");

    assert!(
        registry
            .discover_connectors(LookupScope::PreferPrivate, "traverse.env", "not a range")
            .is_empty()
    );
    assert!(
        registry
            .discover_connectors(LookupScope::PreferPrivate, "traverse.http", "^1.0.0")
            .is_empty()
    );

    let discovered =
        registry.discover_connectors(LookupScope::PreferPrivate, "traverse.env", "^1.0.0");
    assert_eq!(discovered.len(), 1);
    assert_eq!(discovered[0].scope, RegistryScope::Private);
}

#[test]
fn capability_registration_rejects_missing_required_connector() {
    let mut registry = CapabilityRegistry::new();
    let mut contract = base_contract("content.comments.create-comment-draft", "1.0.0");
    contract.connector_requirements.push(ConnectorRequirement {
        connector_id: "traverse.http".to_string(),
        version: "^1.0.0".to_string(),
    });

    let failure = registry
        .register(executable_registration(RegistryScope::Public, contract))
        .expect_err("missing connector should reject registration");

    assert!(failure.errors.iter().any(|error| {
        error.code == RegistryErrorCode::MissingRequiredConnector
            && error.message.contains("missing_required_connector")
    }));
}

#[test]
fn capability_registration_rejects_incompatible_connector_version() {
    let mut registry = CapabilityRegistry::new();
    let connector = reference_connector_contracts()
        .into_iter()
        .find(|contract| contract.connector_id == "traverse.http")
        .expect("traverse.http reference connector should exist");
    registry
        .register_connector(connector_registration(RegistryScope::Public, connector))
        .expect("connector registration should pass");

    let mut contract = base_contract("content.comments.create-comment-draft", "1.0.0");
    contract.connector_requirements.push(ConnectorRequirement {
        connector_id: "traverse.http".to_string(),
        version: "^2.0.0".to_string(),
    });

    let failure = registry
        .register(executable_registration(RegistryScope::Public, contract))
        .expect_err("incompatible connector should reject registration");

    assert!(failure.errors.iter().any(|error| {
        error.code == RegistryErrorCode::ConnectorVersionIncompatible
            && error.message.contains("connector_version_incompatible")
    }));
}

#[test]
fn capability_registration_accepts_satisfied_connector_requirement() {
    let mut registry = CapabilityRegistry::new();
    let connector = reference_connector_contracts()
        .into_iter()
        .find(|contract| contract.connector_id == "traverse.http")
        .expect("traverse.http reference connector should exist");
    registry
        .register_connector(connector_registration(RegistryScope::Public, connector))
        .expect("connector registration should pass");

    let mut contract = base_contract("content.comments.create-comment-draft", "1.0.0");
    contract.connector_requirements.push(ConnectorRequirement {
        connector_id: "traverse.http".to_string(),
        version: "^1.0.0".to_string(),
    });

    let outcome = registry
        .register(executable_registration(RegistryScope::Public, contract))
        .expect("satisfied connector requirement should allow registration");

    assert_eq!(outcome.record.id, "content.comments.create-comment-draft");
}

#[test]
fn rejects_immutable_version_conflict_for_changed_contract() {
    let mut registry = CapabilityRegistry::new();
    registry
        .register(executable_registration(
            RegistryScope::Public,
            base_contract("content.comments.create-comment-draft", "1.0.0"),
        ))
        .expect("seed registration should pass");

    let mut changed = base_contract("content.comments.create-comment-draft", "1.0.0");
    changed.summary = "Create a materially different comment draft result.".to_string();

    let failure = registry
        .register(executable_registration(RegistryScope::Public, changed))
        .expect_err("republishing same version with changed content must fail");

    assert_eq!(
        failure.errors[0].code,
        RegistryErrorCode::ContractValidationFailed
    );
}

#[test]
fn private_overlay_takes_precedence_over_public() {
    let mut registry = CapabilityRegistry::new();
    let public = executable_registration(
        RegistryScope::Public,
        base_contract("content.comments.create-comment-draft", "1.0.0"),
    );
    let mut private_contract = base_contract("content.comments.create-comment-draft", "1.0.0");
    private_contract.summary = "Create a private overlay comment draft variant.".to_string();
    let private = executable_registration(RegistryScope::Private, private_contract);

    registry
        .register(public)
        .expect("public registration should pass");
    registry
        .register(private)
        .expect("private registration should pass");

    let resolved = registry
        .find_exact(
            LookupScope::PreferPrivate,
            "content.comments.create-comment-draft",
            "1.0.0",
        )
        .expect("lookup should resolve");

    assert_eq!(resolved.record.scope, RegistryScope::Private);
    assert_eq!(
        resolved.record.contract_path,
        "registry/private/content.comments.create-comment-draft/1.0.0/contract.json"
    );
}

#[test]
fn discover_filters_and_orders_results_deterministically() {
    let mut registry = CapabilityRegistry::new();

    let mut older = executable_registration(
        RegistryScope::Public,
        base_contract("content.comments.create-comment-draft", "1.0.0"),
    );
    older.tags = vec!["comments".to_string(), "draft".to_string()];

    let mut newer = executable_registration(
        RegistryScope::Public,
        additive_contract("content.comments.create-comment-draft", "1.1.0"),
    );
    newer.tags = vec!["comments".to_string(), "draft".to_string()];

    registry
        .register(older)
        .expect("older registration should pass");
    registry
        .register(newer)
        .expect("newer registration should pass");

    let results = registry.discover(
        LookupScope::PreferPrivate,
        &DiscoveryQuery {
            tag: Some("draft".to_string()),
            composition_pattern: Some(CompositionPattern::Sequential),
            ..DiscoveryQuery::default()
        },
    );

    assert_eq!(results.len(), 2);
    assert_eq!(results[0].version, "1.1.0");
    assert_eq!(results[1].version, "1.0.0");
}

#[test]
fn additive_changes_require_at_least_minor_version() {
    let mut registry = CapabilityRegistry::new();
    registry
        .register(executable_registration(
            RegistryScope::Public,
            base_contract("content.comments.create-comment-draft", "1.0.0"),
        ))
        .expect("seed registration should pass");

    let failure = registry
        .register(executable_registration(
            RegistryScope::Public,
            additive_contract("content.comments.create-comment-draft", "1.0.1"),
        ))
        .expect_err("patch bump should be too small for additive change");

    assert_eq!(failure.errors[0].code, RegistryErrorCode::SemverTooSmall);

    registry
        .register(executable_registration(
            RegistryScope::Public,
            additive_contract("content.comments.create-comment-draft", "1.1.0"),
        ))
        .expect("minor bump should pass for additive change");

    assert_eq!(registry.compatibility_records().len(), 1);
}

#[test]
fn breaking_changes_require_major_version() {
    let mut registry = CapabilityRegistry::new();
    registry
        .register(executable_registration(
            RegistryScope::Public,
            base_contract("content.comments.create-comment-draft", "1.0.0"),
        ))
        .expect("seed registration should pass");

    let failure = registry
        .register(executable_registration(
            RegistryScope::Public,
            breaking_contract("content.comments.create-comment-draft", "1.1.0"),
        ))
        .expect_err("minor bump should be too small for breaking change");

    assert_eq!(failure.errors[0].code, RegistryErrorCode::SemverTooSmall);

    registry
        .register(executable_registration(
            RegistryScope::Public,
            breaking_contract("content.comments.create-comment-draft", "2.0.0"),
        ))
        .expect("major bump should pass for breaking change");
}

#[test]
fn unknown_schema_changes_fail_closed() {
    let mut registry = CapabilityRegistry::new();
    registry
        .register(executable_registration(
            RegistryScope::Public,
            base_contract("content.comments.create-comment-draft", "1.0.0"),
        ))
        .expect("seed registration should pass");

    let failure = registry
        .register(executable_registration(
            RegistryScope::Public,
            schema_changed_contract("content.comments.create-comment-draft", "1.1.0"),
        ))
        .expect_err("unknown compatibility should fail closed");

    assert_eq!(
        failure.errors[0].code,
        RegistryErrorCode::UnknownCompatibility
    );
}

#[test]
fn workflow_backed_capabilities_require_composite_metadata() {
    let mut registry = CapabilityRegistry::new();
    let workflow_request = workflow_registration(
        RegistryScope::Private,
        base_contract("content.comments.publish-comment", "1.0.0"),
    );

    let outcome = registry
        .register(workflow_request)
        .expect("workflow-backed capability should register");

    assert_eq!(
        outcome.record.implementation_kind,
        ImplementationKind::Workflow
    );
    assert_eq!(
        outcome.index_entry.composability.kind,
        CompositionKind::Composite
    );
}

#[test]
fn rejects_invalid_registration_metadata() {
    let mut registry = CapabilityRegistry::new();
    let mut request = executable_registration(
        RegistryScope::Public,
        base_contract("content.comments.create-comment-draft", "1.0.0"),
    );
    request.contract_path.clear();
    request.tags.push("comments".to_string());
    request
        .composability
        .patterns
        .push(CompositionPattern::Sequential);

    let failure = registry
        .register(request)
        .expect_err("invalid registration metadata should fail");

    assert_eq!(failure.errors.len(), 3);
    assert_eq!(
        failure.errors[0].code,
        RegistryErrorCode::MissingRequiredField
    );
    assert_eq!(failure.errors[1].code, RegistryErrorCode::DuplicateItem);
    assert_eq!(failure.errors[2].code, RegistryErrorCode::DuplicateItem);
}

#[test]
fn rejects_artifact_conflicts_for_reused_artifact_refs() {
    let mut registry = CapabilityRegistry::new();
    registry
        .register(executable_registration(
            RegistryScope::Public,
            base_contract("content.comments.create-comment-draft", "1.0.0"),
        ))
        .expect("seed registration should pass");

    let mut request = executable_registration(
        RegistryScope::Public,
        additive_contract("content.comments.create-comment-draft", "1.1.0"),
    );
    request.artifact.artifact_ref = "artifact:create-comment-draft:1.0.0".to_string();
    request.artifact.digests.source_digest = "different-source-digest".to_string();

    let failure = registry
        .register(request)
        .expect_err("reusing an artifact_ref for different metadata must fail");

    assert_eq!(failure.errors[0].code, RegistryErrorCode::ArtifactConflict);
}

#[test]
fn registers_and_finds_public_event_contract() {
    let mut registry = EventRegistry::new();
    let request = event_registration(
        RegistryScope::Public,
        base_event_contract("content.comments.comment-draft-created", "1.0.0"),
    );

    let outcome = registry
        .register(request)
        .expect("event registration should pass");
    let resolved = registry
        .find_exact(
            LookupScope::PublicOnly,
            "content.comments.comment-draft-created",
            "1.0.0",
        )
        .expect("event should resolve");

    assert_eq!(resolved.record, outcome.record);
    assert_eq!(resolved.index_record, outcome.index_record);
    assert_eq!(resolved.record.scope, RegistryScope::Public);
}

#[test]
fn duplicate_identical_event_registration_is_idempotent() {
    let mut registry = EventRegistry::new();
    let request = event_registration(
        RegistryScope::Public,
        base_event_contract("content.comments.comment-draft-created", "1.0.0"),
    );

    let first = registry
        .register(request.clone())
        .expect("first event registration should pass");
    let second = registry
        .register(request)
        .expect("duplicate event registration should be idempotent");

    assert_eq!(first.record, second.record);
    assert_eq!(first.index_record, second.index_record);
}

#[test]
fn rejects_immutable_version_conflict_for_changed_event_contract() {
    let mut registry = EventRegistry::new();
    registry
        .register(event_registration(
            RegistryScope::Public,
            base_event_contract("content.comments.comment-draft-created", "1.0.0"),
        ))
        .expect("seed event registration should pass");

    let mut changed = base_event_contract("content.comments.comment-draft-created", "1.0.0");
    changed.summary = "A materially different governed event summary.".to_string();

    let failure = registry
        .register(event_registration(RegistryScope::Public, changed))
        .expect_err("changed event content must fail");

    assert_eq!(
        failure.errors[0].code,
        EventRegistryErrorCode::ImmutableVersionConflict
    );
}

#[test]
fn private_event_overlay_takes_precedence_over_public() {
    let mut registry = EventRegistry::new();
    registry
        .register(event_registration(
            RegistryScope::Public,
            base_event_contract("content.comments.comment-draft-created", "1.0.0"),
        ))
        .expect("public event should register");

    let mut private = base_event_contract("content.comments.comment-draft-created", "1.0.0");
    private.summary = "Private overlay for comment draft creation.".to_string();
    registry
        .register(event_registration(RegistryScope::Private, private))
        .expect("private event should register");

    let resolved = registry
        .find_exact(
            LookupScope::PreferPrivate,
            "content.comments.comment-draft-created",
            "1.0.0",
        )
        .expect("event should resolve");

    assert_eq!(resolved.record.scope, RegistryScope::Private);
}

#[test]
fn event_lineage_orders_versions_ascending() {
    let mut registry = EventRegistry::new();
    registry
        .register(event_registration(
            RegistryScope::Public,
            base_event_contract("content.comments.comment-draft-created", "1.0.0"),
        ))
        .expect("seed event should register");
    registry
        .register(event_registration(
            RegistryScope::Public,
            additive_event_contract("content.comments.comment-draft-created", "1.1.0"),
        ))
        .expect("additive event should register");

    let lineage = registry
        .lineage(
            RegistryScope::Public,
            "content.comments.comment-draft-created",
        )
        .expect("lineage should exist");

    assert_eq!(lineage.versions.len(), 2);
    assert_eq!(lineage.versions[0].version, "1.0.0");
    assert_eq!(lineage.versions[1].version, "1.1.0");
}

#[test]
fn additive_event_payload_changes_require_minor_version() {
    let mut registry = EventRegistry::new();
    registry
        .register(event_registration(
            RegistryScope::Public,
            base_event_contract("content.comments.comment-draft-created", "1.0.0"),
        ))
        .expect("seed event should register");

    let failure = registry
        .register(event_registration(
            RegistryScope::Public,
            additive_event_contract("content.comments.comment-draft-created", "1.0.1"),
        ))
        .expect_err("patch bump should be too small");

    assert_eq!(
        failure.errors[0].code,
        EventRegistryErrorCode::SemverTooSmall
    );

    registry
        .register(event_registration(
            RegistryScope::Public,
            additive_event_contract("content.comments.comment-draft-created", "1.1.0"),
        ))
        .expect("minor bump should pass");

    assert_eq!(registry.compatibility_records().len(), 1);
}

#[test]
fn breaking_event_payload_changes_require_major_version() {
    let mut registry = EventRegistry::new();
    registry
        .register(event_registration(
            RegistryScope::Public,
            base_event_contract("content.comments.comment-draft-created", "1.0.0"),
        ))
        .expect("seed event should register");

    let failure = registry
        .register(event_registration(
            RegistryScope::Public,
            breaking_event_contract("content.comments.comment-draft-created", "1.1.0"),
        ))
        .expect_err("minor bump should be too small");

    assert_eq!(
        failure.errors[0].code,
        EventRegistryErrorCode::SemverTooSmall
    );

    registry
        .register(event_registration(
            RegistryScope::Public,
            breaking_event_contract("content.comments.comment-draft-created", "2.0.0"),
        ))
        .expect("major bump should pass");
}

#[test]
fn rejects_invalid_event_registration_metadata() {
    let mut registry = EventRegistry::new();
    let mut request = event_registration(
        RegistryScope::Public,
        base_event_contract("content.comments.comment-draft-created", "1.0.0"),
    );
    request.contract_path.clear();

    let failure = registry
        .register(request)
        .expect_err("invalid event registration metadata should fail");

    assert_eq!(
        failure.errors[0].code,
        EventRegistryErrorCode::MissingRequiredField
    );
}

fn executable_registration(
    scope: RegistryScope,
    contract: CapabilityContract,
) -> CapabilityRegistration {
    CapabilityRegistration {
        scope,
        contract_path: format!(
            "registry/{}/{}{}/contract.json",
            scope_name(scope),
            "",
            contract.id.replace(':', "")
        )
        .replace(
            &format!("registry/{}/{}", scope_name(scope), contract.id),
            &format!(
                "registry/{}/{}/{}",
                scope_name(scope),
                contract.id,
                contract.version
            ),
        ),
        artifact: CapabilityArtifactRecord {
            artifact_ref: format!("artifact:{}:{}", contract.name, contract.version),
            implementation_kind: ImplementationKind::Executable,
            source: SourceReference {
                kind: SourceKind::Git,
                location: format!("https://github.com/enricopiovesan/cogolo/{}", contract.name),
            },
            binary: Some(BinaryReference {
                format: BinaryFormat::Wasm,
                location: format!("artifacts/{}/{}.wasm", contract.name, contract.version),
                signature: None,
            }),
            workflow_ref: None,
            digests: ArtifactDigests {
                source_digest: format!("source:{}:{}", contract.name, contract.version),
                binary_digest: Some(format!("binary:{}:{}", contract.name, contract.version)),
            },
            provenance: RegistryProvenance {
                source: "greenfield".to_string(),
                author: "enricopiovesan".to_string(),
                created_at: "2026-03-27T00:00:00Z".to_string(),
            },
        },
        registered_at: "2026-03-27T00:00:00Z".to_string(),
        tags: vec!["comments".to_string()],
        composability: ComposabilityMetadata {
            kind: CompositionKind::Atomic,
            patterns: vec![CompositionPattern::Sequential],
            provides: vec!["comment-draft".to_string()],
            requires: vec!["validated-request".to_string()],
        },
        governing_spec: "005-capability-registry".to_string(),
        validator_version: "registry-test".to_string(),
        contract,
    }
}

fn event_registration(scope: RegistryScope, contract: EventContract) -> EventRegistration {
    EventRegistration {
        scope,
        contract_path: format!(
            "registry/{}/{}/{}/contract.json",
            scope_name(scope),
            contract.id,
            contract.version
        ),
        registered_at: "2026-03-30T00:00:00Z".to_string(),
        governing_spec: "011-event-registry".to_string(),
        validator_version: "registry-test".to_string(),
        contract,
    }
}

fn connector_registration(
    scope: RegistryScope,
    contract: traverse_contracts::ConnectorContract,
) -> ConnectorRegistration {
    ConnectorRegistration {
        scope,
        contract_path: format!(
            "registry/{}/connectors/{}/{}.json",
            scope_name(scope),
            contract.connector_id,
            contract.version
        ),
        registered_at: "2026-04-19T00:00:00Z".to_string(),
        governing_spec: "039-connector-plugin-architecture".to_string(),
        validator_version: "registry-test".to_string(),
        contract,
    }
}

fn workflow_registration(
    scope: RegistryScope,
    contract: CapabilityContract,
) -> CapabilityRegistration {
    CapabilityRegistration {
        composability: ComposabilityMetadata {
            kind: CompositionKind::Composite,
            patterns: vec![
                CompositionPattern::EventDriven,
                CompositionPattern::Aggregation,
            ],
            provides: vec!["published-comment".to_string()],
            requires: vec!["comment-draft".to_string()],
        },
        artifact: CapabilityArtifactRecord {
            artifact_ref: format!("artifact:{}:{}", contract.name, contract.version),
            implementation_kind: ImplementationKind::Workflow,
            source: SourceReference {
                kind: SourceKind::Local,
                location: format!("examples/workflows/{}/", contract.name),
            },
            binary: None,
            workflow_ref: Some(WorkflowReference {
                workflow_id: "content.comments.publish-comment-flow".to_string(),
                workflow_version: "1.0.0".to_string(),
            }),
            digests: ArtifactDigests {
                source_digest: format!("source:{}:{}", contract.name, contract.version),
                binary_digest: None,
            },
            provenance: RegistryProvenance {
                source: "greenfield".to_string(),
                author: "enricopiovesan".to_string(),
                created_at: "2026-03-27T00:00:00Z".to_string(),
            },
        },
        tags: vec!["workflow".to_string(), "comments".to_string()],
        ..executable_registration(scope, contract)
    }
}

fn base_contract(id: &str, version: &str) -> CapabilityContract {
    let (namespace, name) = split_id(id);
    CapabilityContract {
        kind: "capability_contract".to_string(),
        schema_version: "1.0.0".to_string(),
        id: id.to_string(),
        namespace,
        name: name.to_string(),
        version: version.to_string(),
        lifecycle: Lifecycle::Active,
        owner: Owner {
            team: "traverse-core".to_string(),
            contact: "enrico.piovesan10@gmail.com".to_string(),
        },
        summary: "Create a validated comment draft for downstream composition.".to_string(),
        description: "Portable capability for creating a validated comment draft before further workflow processing.".to_string(),
        inputs: SchemaContainer {
            schema: json!({"type": "object", "required": ["comment_text"]}),
        },
        outputs: SchemaContainer {
            schema: json!({"type": "object", "required": ["draft_id"]}),
        },
        preconditions: vec![Condition {
            id: "request-authenticated".to_string(),
            description: "Caller identity has already been established.".to_string(),
        }],
        postconditions: vec![Condition {
            id: "draft-created".to_string(),
            description: "A draft payload is produced.".to_string(),
        }],
        side_effects: vec![SideEffect {
            kind: SideEffectKind::MemoryOnly,
            description: "The capability produces in-memory draft state only.".to_string(),
        }],
        emits: vec![EventReference {
            event_id: "content.comments.comment-draft-created".to_string(),
            version: "1.0.0".to_string(),
        }],
        consumes: vec![],
        permissions: vec![IdReference {
            id: "comments.create".to_string(),
        }],
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
        },
        policies: vec![IdReference {
            id: "default-comment-safety".to_string(),
        }],
        dependencies: vec![DependencyReference {
            artifact_type: DependencyArtifactType::Event,
            id: "content.comments.comment-draft-created".to_string(),
            version: "1.0.0".to_string(),
        }],
        provenance: Provenance {
            source: ProvenanceSource::Greenfield,
            author: "enricopiovesan".to_string(),
            created_at: "2026-03-27T00:00:00Z".to_string(),
            spec_ref: Some("002-capability-contracts".to_string()),
            adr_refs: vec![],
            exception_refs: vec![],
        },
        evidence: vec![ValidationEvidence {
            evidence_id: "validation:contract".to_string(),
            evidence_type: EvidenceType::ContractValidation,
            status: EvidenceStatus::Passed,
        }],
        service_type: ServiceType::Stateless,
        permitted_targets: vec![
            ExecutionTarget::Local,
            ExecutionTarget::Cloud,
            ExecutionTarget::Edge,
            ExecutionTarget::Device,
        ],
        event_trigger: None,
        connector_requirements: Vec::new(),
        state_schema: None,
    }
}

fn additive_contract(id: &str, version: &str) -> CapabilityContract {
    let mut contract = base_contract(id, version);
    contract.emits.push(EventReference {
        event_id: "content.comments.comment-draft-indexed".to_string(),
        version: "1.0.0".to_string(),
    });
    contract
}

fn breaking_contract(id: &str, version: &str) -> CapabilityContract {
    let mut contract = base_contract(id, version);
    contract.permissions.clear();
    contract
}

fn schema_changed_contract(id: &str, version: &str) -> CapabilityContract {
    let mut contract = base_contract(id, version);
    contract.inputs = SchemaContainer {
        schema: json!({"type": "object", "required": ["comment_text", "resource_id"]}),
    };
    contract
}

fn base_event_contract(id: &str, version: &str) -> EventContract {
    let (namespace, name) = split_id(id);
    EventContract {
        kind: "event_contract".to_string(),
        schema_version: "1.0.0".to_string(),
        id: id.to_string(),
        namespace,
        name: name.to_string(),
        version: version.to_string(),
        lifecycle: Lifecycle::Active,
        owner: Owner {
            team: "traverse-core".to_string(),
            contact: "enrico.piovesan10@gmail.com".to_string(),
        },
        summary: "Published when a comment draft has been created.".to_string(),
        description:
            "Governed event contract for comment draft creation used by comment workflows."
                .to_string(),
        payload: EventPayload {
            schema: json!({
                "type": "object",
                "required": ["draft_id"],
                "properties": {
                    "draft_id": {"type": "string"}
                }
            }),
            compatibility: PayloadCompatibility::BackwardCompatible,
        },
        classification: EventClassification {
            domain: "content.comments".to_string(),
            bounded_context: "comments".to_string(),
            event_type: EventType::Domain,
            tags: vec!["comments".to_string(), "draft".to_string()],
        },
        publishers: vec![traverse_contracts::CapabilityReference {
            capability_id: "content.comments.create-comment-draft".to_string(),
            version: "1.0.0".to_string(),
        }],
        subscribers: vec![traverse_contracts::CapabilityReference {
            capability_id: "content.comments.publish-comment".to_string(),
            version: "1.0.0".to_string(),
        }],
        policies: vec![IdReference {
            id: "default-comment-safety".to_string(),
        }],
        tags: vec!["comments".to_string(), "draft".to_string()],
        provenance: EventProvenance {
            source: EventProvenanceSource::Greenfield,
            author: "enricopiovesan".to_string(),
            created_at: "2026-03-30T00:00:00Z".to_string(),
        },
        evidence: vec![],
    }
}

fn additive_event_contract(id: &str, version: &str) -> EventContract {
    let mut contract = base_event_contract(id, version);
    contract.payload.schema = json!({
        "type": "object",
        "required": ["draft_id"],
        "properties": {
            "draft_id": {"type": "string"},
            "moderation_hint": {"type": "string"}
        }
    });
    contract.payload.compatibility = PayloadCompatibility::BackwardCompatible;
    contract
}

fn breaking_event_contract(id: &str, version: &str) -> EventContract {
    let mut contract = base_event_contract(id, version);
    contract.payload.schema = json!({
        "type": "object",
        "required": ["draft_id", "author_id"],
        "properties": {
            "draft_id": {"type": "string"},
            "author_id": {"type": "string"}
        }
    });
    contract.payload.compatibility = PayloadCompatibility::Breaking;
    contract
}

fn split_id(id: &str) -> (String, &str) {
    let mut parts = id.rsplitn(2, '.');
    let name = parts.next().expect("id must include a name");
    let namespace = parts.next().expect("id must include a namespace");
    (namespace.to_string(), name)
}

fn scope_name(scope: RegistryScope) -> &'static str {
    match scope {
        RegistryScope::Public => "public",
        RegistryScope::Private => "private",
    }
}

#[test]
fn loads_canonical_registry_bundle_from_manifest() {
    let manifest_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/expedition/registry-bundle/manifest.json");
    let bundle = load_registry_bundle(manifest_path.as_path()).expect("canonical bundle loads");

    assert_eq!(bundle.bundle_id, "expedition.planning.seed-bundle");
    assert_eq!(bundle.scope, RegistryScope::Public);
    assert_eq!(bundle.capabilities.len(), 6);
    assert_eq!(bundle.events.len(), 5);
    assert_eq!(bundle.workflows.len(), 1);
    assert_eq!(
        bundle.capabilities[0].contract.id,
        "expedition.planning.capture-expedition-objective"
    );
    assert_eq!(
        bundle.workflows[0].definition.id,
        "expedition.planning.plan-expedition"
    );
}

#[test]
fn bundle_loader_rejects_duplicate_manifest_entries() {
    let temp_dir = unique_temp_dir();
    assert!(fs::create_dir_all(&temp_dir).is_ok());

    let manifest_path = temp_dir.join("manifest.json");
    assert!(
        fs::write(
            &manifest_path,
            r#"{
  "bundle_id": "expedition.planning.seed-bundle",
  "version": "1.0.0",
  "scope": "public",
  "capabilities": [
    {
      "id": "expedition.planning.capture-expedition-objective",
      "version": "1.0.0",
      "path": "contract-a.json"
    },
    {
      "id": "expedition.planning.capture-expedition-objective",
      "version": "1.0.0",
      "path": "contract-b.json"
    }
  ],
  "events": [],
  "workflows": []
}"#,
        )
        .is_ok()
    );

    let failure = load_registry_bundle(&manifest_path).expect_err("duplicate ids must fail");
    assert_eq!(
        failure.errors[0].code,
        BundleLoadErrorCode::DuplicateArtifactId
    );
}

#[test]
fn bundle_loader_rejects_missing_artifact_files() {
    let temp_dir = unique_temp_dir();
    assert!(fs::create_dir_all(&temp_dir).is_ok());

    let manifest_path = temp_dir.join("manifest.json");
    assert!(
        fs::write(
            &manifest_path,
            r#"{
  "bundle_id": "expedition.planning.seed-bundle",
  "version": "1.0.0",
  "scope": "public",
  "capabilities": [
    {
      "id": "expedition.planning.capture-expedition-objective",
      "version": "1.0.0",
      "path": "missing.json"
    }
  ],
  "events": [],
  "workflows": []
}"#,
        )
        .is_ok()
    );

    let failure = load_registry_bundle(&manifest_path).expect_err("missing file must fail");
    assert_eq!(
        failure.errors[0].code,
        BundleLoadErrorCode::MissingArtifactFile
    );
}

#[test]
fn bundle_loader_rejects_artifact_identity_mismatch() {
    let temp_dir = unique_temp_dir();
    assert!(fs::create_dir_all(&temp_dir).is_ok());

    let contract_path = temp_dir.join("contract.json");
    let contract_json = serde_json::to_string_pretty(&base_contract(
        "expedition.planning.capture-expedition-objective",
        "1.0.0",
    ))
    .expect("contract should serialize");
    assert!(fs::write(&contract_path, contract_json).is_ok());

    let manifest_path = temp_dir.join("manifest.json");
    assert!(
        fs::write(
            &manifest_path,
            r#"{
  "bundle_id": "expedition.planning.seed-bundle",
  "version": "1.0.0",
  "scope": "public",
  "capabilities": [
    {
      "id": "expedition.planning.wrong-id",
      "version": "1.0.0",
      "path": "contract.json"
    }
  ],
  "events": [],
  "workflows": []
}"#,
        )
        .is_ok()
    );

    let failure = load_registry_bundle(&manifest_path).expect_err("id mismatch must fail");
    assert_eq!(
        failure.errors[0].code,
        BundleLoadErrorCode::ArtifactIdMismatch
    );
}

#[test]
fn bundle_loader_rejects_invalid_scope() {
    let temp_dir = unique_temp_dir();
    assert!(fs::create_dir_all(&temp_dir).is_ok());

    let manifest_path = temp_dir.join("manifest.json");
    assert!(
        fs::write(
            &manifest_path,
            r#"{
  "bundle_id": "expedition.planning.seed-bundle",
  "version": "1.0.0",
  "scope": "shared",
  "capabilities": [],
  "events": [],
  "workflows": []
}"#,
        )
        .is_ok()
    );

    let failure = load_registry_bundle(&manifest_path).expect_err("invalid scope must fail");
    assert_eq!(failure.errors[0].code, BundleLoadErrorCode::InvalidScope);
}

#[test]
fn bundle_loader_rejects_missing_manifest_file() {
    let temp_dir = unique_temp_dir();
    let manifest_path = temp_dir.join("missing-manifest.json");

    let failure = load_registry_bundle(&manifest_path)
        .expect_err("missing manifest file should fail to load");

    assert_eq!(
        failure.errors[0].code,
        BundleLoadErrorCode::ManifestReadFailed
    );
}

#[test]
fn bundle_loader_rejects_manifest_path_without_parent_directory() {
    let failure = load_registry_bundle(PathBuf::new().as_path())
        .expect_err("empty manifest path should fail before reading");

    assert_eq!(
        failure.errors[0].code,
        BundleLoadErrorCode::ManifestParentMissing
    );
}

#[test]
fn bundle_loader_rejects_invalid_manifest_json() {
    let temp_dir = unique_temp_dir();
    let manifest_path = temp_dir.join("manifest.json");
    fs::write(&manifest_path, "{ not valid json ").expect("manifest should write");

    let failure =
        load_registry_bundle(&manifest_path).expect_err("invalid manifest json should fail");

    assert_eq!(
        failure.errors[0].code,
        BundleLoadErrorCode::ManifestParseFailed
    );
}

#[test]
fn bundle_loader_rejects_invalid_capability_contract_artifact() {
    let temp_dir = unique_temp_dir();
    let manifest_path = temp_dir.join("manifest.json");
    let capability_path = temp_dir.join("capability.json");
    let event_path = temp_dir.join("event.json");
    let workflow_path = temp_dir.join("workflow.json");

    fs::write(&capability_path, "{\"kind\":\"capability_contract\"}")
        .expect("capability artifact should write");
    write_json(&event_path, &canonical_event_contract_json());
    write_json(&workflow_path, &canonical_workflow_json());
    write_bundle_manifest(
        &manifest_path,
        &bundle_manifest_json(
            "private",
            &[artifact_manifest_json(
                "expedition.planning.capture-expedition-objective",
                "1.0.0",
                "capability.json",
            )],
            &[artifact_manifest_json(
                "expedition.planning.expedition-objective-captured",
                "1.0.0",
                "event.json",
            )],
            &[artifact_manifest_json(
                "expedition.planning.plan-expedition",
                "1.0.0",
                "workflow.json",
            )],
        ),
    );

    let failure = load_registry_bundle(&manifest_path)
        .expect_err("invalid capability artifact should fail to load");

    assert_eq!(
        failure.errors[0].code,
        BundleLoadErrorCode::CapabilityParseFailed
    );
}

#[test]
fn bundle_loader_rejects_invalid_event_contract_artifact() {
    let temp_dir = unique_temp_dir();
    let manifest_path = temp_dir.join("manifest.json");
    let capability_path = temp_dir.join("capability.json");
    let event_path = temp_dir.join("event.json");
    let workflow_path = temp_dir.join("workflow.json");

    copy_example_json(
        "contracts/examples/expedition/capabilities/capture-expedition-objective/contract.json",
        &capability_path,
    );
    fs::write(&event_path, "{\"kind\":\"event_contract\"}").expect("event artifact should write");
    copy_example_json(
        "workflows/examples/expedition/plan-expedition/workflow.json",
        &workflow_path,
    );
    write_bundle_manifest(
        &manifest_path,
        &bundle_manifest_json(
            "private",
            &[artifact_manifest_json(
                "expedition.planning.capture-expedition-objective",
                "1.0.0",
                "capability.json",
            )],
            &[artifact_manifest_json(
                "expedition.planning.expedition-objective-captured",
                "1.0.0",
                "event.json",
            )],
            &[artifact_manifest_json(
                "expedition.planning.plan-expedition",
                "1.0.0",
                "workflow.json",
            )],
        ),
    );

    let failure =
        load_registry_bundle(&manifest_path).expect_err("invalid event artifact should fail");

    assert_eq!(
        failure.errors[0].code,
        BundleLoadErrorCode::EventParseFailed
    );
}

#[test]
fn bundle_loader_rejects_invalid_workflow_artifact() {
    let temp_dir = unique_temp_dir();
    let manifest_path = temp_dir.join("manifest.json");
    let capability_path = temp_dir.join("capability.json");
    let event_path = temp_dir.join("event.json");
    let workflow_path = temp_dir.join("workflow.json");

    copy_example_json(
        "contracts/examples/expedition/capabilities/capture-expedition-objective/contract.json",
        &capability_path,
    );
    copy_example_json(
        "contracts/examples/expedition/events/expedition-objective-captured/contract.json",
        &event_path,
    );
    fs::write(&workflow_path, "{\"id\":true}").expect("workflow artifact should write");
    write_bundle_manifest(
        &manifest_path,
        &bundle_manifest_json(
            "private",
            &[artifact_manifest_json(
                "expedition.planning.capture-expedition-objective",
                "1.0.0",
                "capability.json",
            )],
            &[artifact_manifest_json(
                "expedition.planning.expedition-objective-captured",
                "1.0.0",
                "event.json",
            )],
            &[artifact_manifest_json(
                "expedition.planning.plan-expedition",
                "1.0.0",
                "workflow.json",
            )],
        ),
    );

    let failure =
        load_registry_bundle(&manifest_path).expect_err("invalid workflow artifact should fail");

    assert_eq!(
        failure.errors[0].code,
        BundleLoadErrorCode::WorkflowParseFailed
    );
}

#[test]
fn bundle_loader_rejects_unreadable_artifact_contents() {
    let temp_dir = unique_temp_dir();
    let manifest_path = temp_dir.join("manifest.json");
    let capability_path = temp_dir.join("capability.json");
    let event_path = temp_dir.join("event.json");
    let workflow_path = temp_dir.join("workflow.json");

    fs::write(&capability_path, vec![0xFF, 0xFE, 0xFD]).expect("artifact bytes should write");
    write_json(&event_path, &canonical_event_contract_json());
    write_json(&workflow_path, &canonical_workflow_json());
    write_bundle_manifest(
        &manifest_path,
        &bundle_manifest_json(
            "private",
            &[artifact_manifest_json(
                "expedition.planning.capture-expedition-objective",
                "1.0.0",
                "capability.json",
            )],
            &[artifact_manifest_json(
                "expedition.planning.expedition-objective-captured",
                "1.0.0",
                "event.json",
            )],
            &[artifact_manifest_json(
                "expedition.planning.plan-expedition",
                "1.0.0",
                "workflow.json",
            )],
        ),
    );

    let failure = load_registry_bundle(&manifest_path)
        .expect_err("invalid utf-8 artifact contents should fail to load");

    assert_eq!(
        failure.errors[0].code,
        BundleLoadErrorCode::MissingArtifactFile
    );
}

#[test]
fn bundle_loader_rejects_artifact_version_mismatch() {
    let temp_dir = unique_temp_dir();
    let manifest_path = temp_dir.join("manifest.json");
    let capability_path = temp_dir.join("capability.json");
    let event_path = temp_dir.join("event.json");
    let workflow_path = temp_dir.join("workflow.json");

    let mut capability = load_example_json(
        "contracts/examples/expedition/capabilities/capture-expedition-objective/contract.json",
    );
    capability["version"] = json!("2.0.0");

    write_json(&capability_path, &capability);
    copy_example_json(
        "contracts/examples/expedition/events/expedition-objective-captured/contract.json",
        &event_path,
    );
    copy_example_json(
        "workflows/examples/expedition/plan-expedition/workflow.json",
        &workflow_path,
    );
    write_bundle_manifest(
        &manifest_path,
        &bundle_manifest_json(
            "private",
            &[artifact_manifest_json(
                "expedition.planning.capture-expedition-objective",
                "1.0.0",
                "capability.json",
            )],
            &[artifact_manifest_json(
                "expedition.planning.expedition-objective-captured",
                "1.0.0",
                "event.json",
            )],
            &[artifact_manifest_json(
                "expedition.planning.plan-expedition",
                "1.0.0",
                "workflow.json",
            )],
        ),
    );

    let failure = load_registry_bundle(&manifest_path)
        .expect_err("artifact version mismatch should fail to load");

    assert_eq!(
        failure.errors[0].code,
        BundleLoadErrorCode::ArtifactVersionMismatch
    );
}

// ── semver range resolution (spec 037) ───────────────────────────────────────

#[test]
fn range_resolver_resolves_highest_satisfying_version_from_integration_test() {
    let mut registry = CapabilityRegistry::new();
    for version in &["1.0.0", "1.1.0", "1.2.0"] {
        registry
            .register(executable_registration(
                RegistryScope::Public,
                base_contract("test.integration.range-cap", version),
            ))
            .expect("registration should succeed");
    }
    let resolved = resolve_version_range(
        &registry,
        "test.integration.range-cap",
        "^1.0.0",
        LookupScope::PublicOnly,
    )
    .expect("^1.0.0 should resolve to 1.2.0");
    assert_eq!(resolved.version, "1.2.0");
    assert_eq!(resolved.capability_id, "test.integration.range-cap");
}

#[test]
fn range_resolver_returns_no_version_satisfies_when_range_does_not_match() {
    let mut registry = CapabilityRegistry::new();
    registry
        .register(executable_registration(
            RegistryScope::Public,
            base_contract("test.integration.range-cap", "2.0.0"),
        ))
        .expect("registration should succeed");
    let err = resolve_version_range(
        &registry,
        "test.integration.range-cap",
        "^1.0.0",
        LookupScope::PublicOnly,
    )
    .expect_err("should fail with NoVersionSatisfies");
    assert!(matches!(
        err,
        traverse_registry::RangeResolutionError::NoVersionSatisfies { .. }
    ));
}

#[test]
fn range_resolver_returns_not_found_for_unknown_capability() {
    let registry = CapabilityRegistry::new();
    let err = resolve_version_range(
        &registry,
        "test.integration.nonexistent",
        "^1.0.0",
        LookupScope::PublicOnly,
    )
    .expect_err("should fail with CapabilityNotFound");
    assert!(matches!(
        err,
        traverse_registry::RangeResolutionError::CapabilityNotFound { .. }
    ));
}

fn write_bundle_manifest(path: &PathBuf, value: &serde_json::Value) {
    write_json(path, value);
}

fn write_json(path: &PathBuf, value: &serde_json::Value) {
    fs::write(
        path,
        serde_json::to_string_pretty(value).expect("json should serialize"),
    )
    .expect("json file should write");
}

fn bundle_manifest_json(
    scope: &str,
    capabilities: &[serde_json::Value],
    events: &[serde_json::Value],
    workflows: &[serde_json::Value],
) -> serde_json::Value {
    json!({
        "bundle_id": "expedition.planning.registry-bundle",
        "version": "1.0.0",
        "scope": scope,
        "capabilities": capabilities,
        "events": events,
        "workflows": workflows,
    })
}

fn artifact_manifest_json(id: &str, version: &str, path: &str) -> serde_json::Value {
    json!({
        "id": id,
        "version": version,
        "path": path,
    })
}

fn canonical_event_contract_json() -> serde_json::Value {
    json!({
        "kind": "event_contract",
        "schema_version": "1.0.0",
        "id": "expedition.planning.expedition-objective-captured",
        "namespace": "expedition.planning",
        "name": "expedition-objective-captured",
        "version": "1.0.0",
        "lifecycle": "active",
        "classification": "domain",
        "event_type": "notification",
        "owner": {
            "team": "Traverse",
            "contact": "team@traverse.dev"
        },
        "summary": "Objective capture completed.",
        "description": "Emitted when the expedition objective has been captured.",
        "payload": {
            "schema": {
                "type": "object"
            },
            "compatibility": "backward_compatible"
        },
        "publishers": [
            {
                "capability_id": "expedition.planning.capture-expedition-objective",
                "version": "1.0.0"
            }
        ],
        "subscribers": [],
        "policies": [
            {
                "id": "policy.expedition"
            }
        ],
        "provenance": {
            "source": "greenfield",
            "author": "Traverse",
            "created_at": "2026-03-30T00:00:00Z",
            "updated_at": "2026-03-30T00:00:00Z",
            "spec_ref": "009-expedition-example-artifacts",
            "adr_refs": ["0001-rust-wasm-foundation"],
            "exception_refs": []
        },
        "validation_evidence": [
            {
                "evidence_id": "evidence.expedition-objective-captured",
                "evidence_type": "test_suite",
                "status": "satisfied",
                "details": "Validated against the canonical expedition example."
            }
        ]
    })
}

fn canonical_workflow_json() -> serde_json::Value {
    json!({
        "kind": "workflow_definition",
        "id": "expedition.planning.plan-expedition",
        "version": "1.0.0",
        "summary": "Compose the expedition planning path.",
        "start_node": "capture_objective",
        "nodes": [
            {
                "id": "capture_objective",
                "capability_id": "expedition.planning.capture-expedition-objective",
                "version": "1.0.0",
                "inputs": [
                    {
                        "key": "request",
                        "from": "$request"
                    }
                ],
                "outputs": [
                    {
                        "key": "objective",
                        "to": "$workflow.objective"
                    }
                ]
            }
        ],
        "edges": []
    })
}

fn copy_example_json(relative_path: &str, destination: &PathBuf) {
    let value = load_example_json(relative_path);
    write_json(destination, &value);
}

fn load_example_json(relative_path: &str) -> serde_json::Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative_path);
    serde_json::from_str(
        &fs::read_to_string(path).expect("example artifact should read from the repository"),
    )
    .expect("example artifact should contain valid json")
}

fn unique_temp_dir() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("traverse-registry-bundle-test-{nanos}"));
    fs::create_dir_all(&path).expect("temporary directory should create");
    path
}
