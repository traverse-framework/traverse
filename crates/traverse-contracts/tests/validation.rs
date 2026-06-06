use std::{fs, path::Path};

use traverse_contracts::{
    BinaryFormat, CapabilityContract, CapabilityReference, Condition, ConnectorRequirement,
    DependencyArtifactType, DependencyReference, Entrypoint, EntrypointKind, EventClassification,
    EventContract, EventPayload, EventProvenance, EventProvenanceSource, EventReference, EventType,
    EventValidationContext, EventValidationEvidence, EvidenceStatus, EvidenceType, Execution,
    ExecutionConstraints, ExecutionTarget, FilesystemAccess, HostApiAccess, IdReference, Lifecycle,
    NetworkAccess, Owner, PayloadCompatibility, ProducedValidationEvidence, Provenance,
    ProvenanceSource, PublishedContractRecord, PublishedEventRecord, SchemaContainer, ServiceType,
    SideEffect, SideEffectKind, ValidationContext, ValidationErrorCode, ValidationEvidence,
    ValidationFailure, ValidationResult, governed_content_digest, governed_event_content_digest,
    parse_connector_contract, parse_contract, parse_event_contract, reference_connector_contracts,
    validate_connector_contract, validate_contract, validate_event_contract,
};

const GOVERNING_SPEC: &str = "002-capability-contracts@0.1.0";
const EVENT_GOVERNING_SPEC: &str = "003-event-contracts@1.0.0";
const VALIDATOR_VERSION: &str = "0.1.0";

#[test]
fn parses_and_validates_a_contract() -> Result<(), String> {
    let parsed = parse_contract(&valid_contract_json()).map_err(|error| format!("{error:?}"))?;
    let result = validate_contract(
        parsed.clone(),
        &ValidationContext {
            governing_spec: GOVERNING_SPEC,
            validator_version: VALIDATOR_VERSION,
            existing_published: None,
        },
    )
    .map_err(|error| format!("{error:?}"))?;

    assert_eq!(
        result.normalized.id,
        "content.comments.create-comment-draft"
    );
    assert_eq!(
        result.normalized.execution.binary_format,
        BinaryFormat::Wasm
    );
    assert_eq!(result.evidence.governing_spec, GOVERNING_SPEC);
    assert_eq!(result.evidence.status, EvidenceStatus::Passed);
    assert_eq!(parsed.lifecycle, Lifecycle::Draft);
    Ok(())
}

#[test]
fn rejects_invalid_identity_and_semver() {
    let mut contract = valid_contract();
    contract.id = "wrong.id".to_string();
    contract.version = "not-semver".to_string();

    let errors: Vec<_> = expect_validation_failure(validate_contract(
        contract,
        &ValidationContext {
            governing_spec: GOVERNING_SPEC,
            validator_version: VALIDATOR_VERSION,
            existing_published: None,
        },
    ))
    .into_iter()
    .collect();
    assert_eq!(errors.len(), 1);
    let error = errors[0].clone();

    assert!(
        error
            .errors
            .iter()
            .any(|item| item.code == ValidationErrorCode::InconsistentIdentity)
    );
    assert!(
        error
            .errors
            .iter()
            .any(|item| item.code == ValidationErrorCode::InvalidSemver)
    );
}

#[test]
fn parses_and_validates_connector_contract() -> Result<(), String> {
    let contract = reference_connector_contracts()
        .into_iter()
        .find(|connector| connector.connector_id == "traverse.http")
        .ok_or_else(|| "missing traverse.http reference connector".to_string())?;
    let json = serde_json::to_string(&contract).map_err(|error| error.to_string())?;
    let parsed = parse_connector_contract(&json).map_err(|error| format!("{error:?}"))?;
    let validated =
        validate_connector_contract(parsed.clone()).map_err(|error| format!("{error:?}"))?;

    assert_eq!(validated, parsed);
    assert_eq!(validated.version, "1.0.0");
    assert_eq!(
        validated.capabilities_provided,
        vec!["traverse.http.outbound".to_string()]
    );
    assert!(validated.required_config_schema.is_object());
    Ok(())
}

#[test]
fn rejects_invalid_connector_json() {
    let errors: Vec<_> = parse_connector_contract("{").err().into_iter().collect();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].errors[0].code, ValidationErrorCode::InvalidFormat);
}

#[test]
fn rejects_invalid_connector_contract_shape() -> Result<(), String> {
    let mut contract = reference_connector_contracts()
        .into_iter()
        .next()
        .ok_or_else(|| "reference connector should exist".to_string())?;
    contract.kind = "wrong".to_string();
    contract.schema_version = "2.0.0".to_string();
    contract.version = "nope".to_string();
    contract.capabilities_provided.clear();
    contract.required_config_schema = serde_json::json!("not-object");
    contract.supported_placement_targets = vec![ExecutionTarget::Local, ExecutionTarget::Local];

    let errors: Vec<_> = expect_validation_failure(validate_connector_contract(contract))
        .into_iter()
        .collect();
    let failure = &errors[0];

    assert!(failure.errors.iter().any(|error| {
        error.code == ValidationErrorCode::InvalidLiteral && error.path == "$.kind"
    }));
    assert!(failure.errors.iter().any(|error| {
        error.code == ValidationErrorCode::InvalidLiteral && error.path == "$.schema_version"
    }));
    assert!(failure.errors.iter().any(|error| {
        error.code == ValidationErrorCode::InvalidSemver && error.path == "$.version"
    }));
    assert!(failure.errors.iter().any(|error| {
        error.code == ValidationErrorCode::MissingRequiredField
            && error.path == "$.capabilities_provided"
    }));
    assert!(failure.errors.iter().any(|error| {
        error.code == ValidationErrorCode::DuplicateItem
            && error.path == "$.supported_placement_targets"
    }));
    Ok(())
}

#[test]
fn rejects_connector_contract_without_targets() -> Result<(), String> {
    let mut contract = reference_connector_contracts()
        .into_iter()
        .next()
        .ok_or_else(|| "reference connector should exist".to_string())?;
    contract.supported_placement_targets.clear();

    let errors: Vec<_> = expect_validation_failure(validate_connector_contract(contract))
        .into_iter()
        .collect();

    assert!(errors[0].errors.iter().any(|error| {
        error.code == ValidationErrorCode::MissingRequiredField
            && error.path == "$.supported_placement_targets"
    }));
    Ok(())
}

#[test]
fn validates_connector_requirements_on_capability_contracts() {
    let mut contract = valid_contract();
    contract.connector_requirements.push(ConnectorRequirement {
        connector_id: "traverse.http".to_string(),
        version: "^1.0.0".to_string(),
    });

    let result = validate_contract(
        contract,
        &ValidationContext {
            governing_spec: GOVERNING_SPEC,
            validator_version: VALIDATOR_VERSION,
            existing_published: None,
        },
    );

    assert!(result.is_ok());
}

#[test]
fn rejects_invalid_connector_requirements_on_capability_contracts() {
    let mut contract = valid_contract();
    contract.connector_requirements = vec![
        ConnectorRequirement {
            connector_id: "traverse.http".to_string(),
            version: "not a range".to_string(),
        },
        ConnectorRequirement {
            connector_id: "traverse.http".to_string(),
            version: "not a range".to_string(),
        },
    ];

    let errors: Vec<_> = expect_validation_failure(validate_contract(
        contract,
        &ValidationContext {
            governing_spec: GOVERNING_SPEC,
            validator_version: VALIDATOR_VERSION,
            existing_published: None,
        },
    ))
    .into_iter()
    .collect();
    let failure = &errors[0];

    assert!(failure.errors.iter().any(|error| {
        error.code == ValidationErrorCode::InvalidConnectorRequirement
            && error.path == "$.connector_requirements[0].version"
    }));
    assert!(failure.errors.iter().any(|error| {
        error.code == ValidationErrorCode::DuplicateItem
            && error.path == "$.connector_requirements[1].connector_id"
    }));
}

#[test]
fn rejects_duplicate_references_and_missing_exception_metadata() {
    let mut contract = valid_contract();
    contract.permissions.push(IdReference {
        id: "comments.create".to_string(),
    });
    contract
        .execution
        .preferred_targets
        .push(ExecutionTarget::Local);
    contract.execution.constraints.host_api_access = HostApiAccess::ExceptionRequired;

    let errors: Vec<_> = expect_validation_failure(validate_contract(
        contract,
        &ValidationContext {
            governing_spec: GOVERNING_SPEC,
            validator_version: VALIDATOR_VERSION,
            existing_published: None,
        },
    ))
    .into_iter()
    .collect();
    assert_eq!(errors.len(), 1);
    let error = errors[0].clone();

    assert!(
        error
            .errors
            .iter()
            .any(|item| item.code == ValidationErrorCode::DuplicateItem)
    );
    assert!(
        error
            .errors
            .iter()
            .any(|item| item.code == ValidationErrorCode::PortabilityExceptionRequired)
    );
}

#[test]
fn rejects_invalid_boundary_language() {
    let mut contract = valid_contract();
    contract.summary = "utility function for JSON reshaping".to_string();

    let errors: Vec<_> = expect_validation_failure(validate_contract(
        contract,
        &ValidationContext {
            governing_spec: GOVERNING_SPEC,
            validator_version: VALIDATOR_VERSION,
            existing_published: None,
        },
    ))
    .into_iter()
    .collect();
    assert_eq!(errors.len(), 1);
    let error = errors[0].clone();

    assert!(
        error
            .errors
            .iter()
            .any(|item| item.code == ValidationErrorCode::InvalidCapabilityBoundary)
    );
}

#[test]
fn rejects_immutable_published_version_conflicts() {
    let contract = valid_contract();
    let published = PublishedContractRecord {
        id: contract.id.clone(),
        version: contract.version.clone(),
        governed_content_digest: "0.1.0:deadbeefdeadbeef".to_string(),
        lifecycle: Lifecycle::Active,
    };

    let errors: Vec<_> = expect_validation_failure(validate_contract(
        contract,
        &ValidationContext {
            governing_spec: GOVERNING_SPEC,
            validator_version: VALIDATOR_VERSION,
            existing_published: Some(&published),
        },
    ))
    .into_iter()
    .collect();
    assert_eq!(errors.len(), 1);
    let error = errors[0].clone();

    assert!(
        error
            .errors
            .iter()
            .any(|item| item.code == ValidationErrorCode::ImmutableVersionConflict)
    );
}

#[test]
fn governed_content_digest_ignores_evidence() {
    let contract = valid_contract();
    let mut with_evidence = contract.clone();
    with_evidence.evidence.push(ValidationEvidence {
        evidence_id: "evd_123".to_string(),
        evidence_type: EvidenceType::ContractValidation,
        status: EvidenceStatus::Passed,
    });

    assert_eq!(
        governed_content_digest(&contract),
        governed_content_digest(&with_evidence)
    );
}

#[test]
fn lifecycle_runtime_eligibility_matches_spec() {
    assert!(!Lifecycle::Draft.is_runtime_eligible());
    assert!(Lifecycle::Active.is_runtime_eligible());
    assert!(Lifecycle::Deprecated.is_runtime_eligible());
    assert!(!Lifecycle::Retired.is_runtime_eligible());
    assert!(!Lifecycle::Archived.is_runtime_eligible());
}

#[test]
fn rejects_invalid_json() {
    let errors: Vec<_> = parse_contract("{").err().into_iter().collect();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].errors[0].code, ValidationErrorCode::InvalidFormat);
}

#[test]
fn rejects_invalid_metadata_and_structure() {
    let mut contract = valid_contract();
    contract.kind = "wrong".to_string();
    contract.schema_version = "9.9.9".to_string();
    contract.namespace = "Content.Comments".to_string();
    contract.name = "Invalid Name".to_string();
    contract.summary = "short".to_string();
    contract.description = "too short".to_string();
    contract.inputs.schema = serde_json::json!(["not", "an", "object"]);
    contract.outputs.schema = serde_json::json!(["still", "wrong"]);
    contract.owner.team.clear();
    contract.owner.contact.clear();

    let errors: Vec<_> = expect_validation_failure(validate_contract(
        contract,
        &ValidationContext {
            governing_spec: GOVERNING_SPEC,
            validator_version: VALIDATOR_VERSION,
            existing_published: None,
        },
    ))
    .into_iter()
    .collect();
    assert_eq!(errors.len(), 1);
    let error = errors[0].clone();

    assert!(
        error
            .errors
            .iter()
            .any(|item| item.code == ValidationErrorCode::InvalidLiteral)
    );
    assert!(
        error
            .errors
            .iter()
            .any(|item| item.code == ValidationErrorCode::InconsistentIdentity)
    );
    assert!(
        error
            .errors
            .iter()
            .any(|item| item.code == ValidationErrorCode::MissingRequiredField)
    );
    assert!(
        error
            .errors
            .iter()
            .any(|item| item.path == "$.inputs.schema")
    );
    assert!(
        error
            .errors
            .iter()
            .any(|item| item.path == "$.outputs.schema")
    );
}

#[test]
fn rejects_invalid_conditions_side_effects_and_events() {
    let mut contract = valid_contract();
    contract.preconditions.push(Condition {
        id: "request-authenticated".to_string(),
        description: String::new(),
    });
    contract.postconditions.push(Condition {
        id: String::new(),
        description: "duplicate and empty".to_string(),
    });
    contract.side_effects.clear();
    contract.emits.push(EventReference {
        event_id: "content.comments.comment-draft-created".to_string(),
        version: "0.1.0".to_string(),
    });
    contract.consumes.push(EventReference {
        event_id: String::new(),
        version: "bad".to_string(),
    });

    let errors: Vec<_> = expect_validation_failure(validate_contract(
        contract,
        &ValidationContext {
            governing_spec: GOVERNING_SPEC,
            validator_version: VALIDATOR_VERSION,
            existing_published: None,
        },
    ))
    .into_iter()
    .collect();
    assert_eq!(errors.len(), 1);
    let error = errors[0].clone();

    assert!(
        error
            .errors
            .iter()
            .any(|item| item.path == "$.preconditions[1].id")
    );
    assert!(
        error
            .errors
            .iter()
            .any(|item| item.path == "$.preconditions[1].description")
    );
    assert!(
        error
            .errors
            .iter()
            .any(|item| item.path == "$.postconditions[1].id")
    );
    assert!(
        error
            .errors
            .iter()
            .any(|item| item.path == "$.side_effects")
    );
    assert!(
        error
            .errors
            .iter()
            .any(|item| item.path == "$.emits[1].event_id")
    );
    assert!(
        error
            .errors
            .iter()
            .any(|item| item.path == "$.consumes[0].version")
    );
}

#[test]
fn rejects_invalid_execution_dependencies_and_evidence() {
    let mut contract = valid_contract();
    contract.execution.entrypoint.command.clear();
    contract.execution.preferred_targets.clear();
    contract.dependencies = vec![
        DependencyReference {
            artifact_type: DependencyArtifactType::Capability,
            id: "content.comments.create-comment-draft".to_string(),
            version: "0.1.0".to_string(),
        },
        DependencyReference {
            artifact_type: DependencyArtifactType::Capability,
            id: "content.comments.create-comment-draft".to_string(),
            version: "0.1.0".to_string(),
        },
    ];
    contract.evidence = vec![
        ValidationEvidence {
            evidence_id: "evd_dup".to_string(),
            evidence_type: EvidenceType::ContractValidation,
            status: EvidenceStatus::Passed,
        },
        ValidationEvidence {
            evidence_id: "evd_dup".to_string(),
            evidence_type: EvidenceType::Compatibility,
            status: EvidenceStatus::Superseded,
        },
    ];

    let errors: Vec<_> = expect_validation_failure(validate_contract(
        contract,
        &ValidationContext {
            governing_spec: GOVERNING_SPEC,
            validator_version: VALIDATOR_VERSION,
            existing_published: None,
        },
    ))
    .into_iter()
    .collect();
    assert_eq!(errors.len(), 1);
    let error = errors[0].clone();

    assert!(
        error
            .errors
            .iter()
            .any(|item| item.path == "$.execution.entrypoint.command")
    );
    assert!(
        error
            .errors
            .iter()
            .any(|item| item.path == "$.execution.preferred_targets")
    );
    assert!(
        error
            .errors
            .iter()
            .any(|item| item.path == "$.dependencies[1].id")
    );
    assert!(
        error
            .errors
            .iter()
            .any(|item| item.path == "$.evidence[1].evidence_id")
    );
}

#[test]
fn rejects_duplicate_provenance_references() {
    let mut contract = valid_contract();
    contract.provenance.spec_ref = Some(String::new());
    contract.provenance.adr_refs = vec!["adr-1".to_string(), "adr-1".to_string()];
    contract.provenance.exception_refs = vec!["ex-1".to_string(), "ex-1".to_string()];

    let errors: Vec<_> = expect_validation_failure(validate_contract(
        contract,
        &ValidationContext {
            governing_spec: GOVERNING_SPEC,
            validator_version: VALIDATOR_VERSION,
            existing_published: None,
        },
    ))
    .into_iter()
    .collect();
    assert_eq!(errors.len(), 1);
    let error = errors[0].clone();

    assert!(
        error
            .errors
            .iter()
            .any(|item| item.path == "$.provenance.spec_ref")
    );
    assert!(
        error
            .errors
            .iter()
            .any(|item| item.path == "$.provenance.adr_refs")
    );
    assert!(
        error
            .errors
            .iter()
            .any(|item| item.path == "$.provenance.exception_refs")
    );
}

#[test]
fn allows_published_records_for_other_versions() {
    let contract = valid_contract();
    let published = PublishedContractRecord {
        id: contract.id.clone(),
        version: "9.9.9".to_string(),
        governed_content_digest: "different".to_string(),
        lifecycle: Lifecycle::Active,
    };

    let result = validate_contract(
        contract,
        &ValidationContext {
            governing_spec: GOVERNING_SPEC,
            validator_version: VALIDATOR_VERSION,
            existing_published: Some(&published),
        },
    );

    assert!(result.is_ok());
}

#[test]
fn expect_validation_failure_rejects_success_results() {
    let result = expect_validation_failure(Ok(ValidationResult {
        normalized: valid_contract(),
        evidence: ProducedValidationEvidence {
            artifact_id: "x".to_string(),
            artifact_version: "0.1.0".to_string(),
            governing_spec: GOVERNING_SPEC.to_string(),
            validator_version: VALIDATOR_VERSION.to_string(),
            status: EvidenceStatus::Passed,
        },
    }));

    assert!(result.is_err());
}

fn valid_contract_json() -> String {
    serde_json::to_string(&valid_contract()).unwrap_or_default()
}

fn expect_validation_failure<T>(
    result: Result<T, ValidationFailure>,
) -> Result<ValidationFailure, String> {
    match result {
        Ok(_) => Err("validation unexpectedly succeeded".to_string()),
        Err(error) => Ok(error),
    }
}

fn valid_contract() -> CapabilityContract {
    CapabilityContract {
        kind: "capability_contract".to_string(),
        schema_version: "1.0.0".to_string(),
        id: "content.comments.create-comment-draft".to_string(),
        namespace: "content.comments".to_string(),
        name: "create-comment-draft".to_string(),
        version: "0.1.0".to_string(),
        lifecycle: Lifecycle::Draft,
        owner: Owner {
            team: "traverse-core".to_string(),
            contact: "enrico.piovesan10@gmail.com".to_string(),
        },
        summary: "Create a draft comment from validated request input.".to_string(),
        description: "Portable capability for creating a comment draft before persistence."
            .to_string(),
        inputs: SchemaContainer {
            schema: serde_json::json!({ "type": "object" }),
        },
        outputs: SchemaContainer {
            schema: serde_json::json!({ "type": "object" }),
        },
        preconditions: vec![Condition {
            id: "request-authenticated".to_string(),
            description: "Caller identity is already established.".to_string(),
        }],
        postconditions: vec![Condition {
            id: "draft-created".to_string(),
            description: "A draft payload is returned.".to_string(),
        }],
        side_effects: vec![SideEffect {
            kind: SideEffectKind::MemoryOnly,
            description: "No durable side effect occurs in this capability.".to_string(),
        }],
        emits: vec![EventReference {
            event_id: "content.comments.comment-draft-created".to_string(),
            version: "0.1.0".to_string(),
        }],
        consumes: Vec::new(),
        permissions: vec![IdReference {
            id: "comments.create".to_string(),
        }],
        execution: Execution {
            binary_format: BinaryFormat::Wasm,
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
        dependencies: Vec::new(),
        provenance: Provenance {
            source: ProvenanceSource::Greenfield,
            author: "enricopiovesan".to_string(),
            created_at: "2026-03-26T00:00:00Z".to_string(),
            spec_ref: Some(GOVERNING_SPEC.to_string()),
            adr_refs: vec!["0001-rust-wasm-foundation".to_string()],
            exception_refs: Vec::new(),
        },
        evidence: Vec::new(),
        service_type: ServiceType::Stateless,
        permitted_targets: vec![
            ExecutionTarget::Local,
            ExecutionTarget::Cloud,
            ExecutionTarget::Edge,
        ],
        event_trigger: None,
        connector_requirements: Vec::new(),
    }
}

#[test]
fn stateless_contract_defaults_parse_from_json_without_new_fields() -> Result<(), String> {
    // Contracts without service_type/permitted_targets/event_trigger must still parse —
    // backward-compatible defaults apply.
    let mut v: serde_json::Value =
        serde_json::from_str(&serde_json::to_string(&valid_contract()).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    if let Some(m) = v.as_object_mut() {
        m.remove("service_type");
        m.remove("permitted_targets");
        m.remove("event_trigger");
    }
    let json = serde_json::to_string(&v).map_err(|e| e.to_string())?;

    let parsed = parse_contract(&json).map_err(|e| format!("{e:?}"))?;
    assert_eq!(parsed.service_type, ServiceType::Stateless);
    assert!(
        !parsed.permitted_targets.is_empty(),
        "permitted_targets defaults to all targets"
    );
    assert_eq!(parsed.event_trigger, None);
    Ok(())
}

#[test]
fn stateful_with_browser_target_is_rejected() -> Result<(), String> {
    let mut contract = valid_contract();
    contract.service_type = ServiceType::Stateful;
    contract.permitted_targets = vec![ExecutionTarget::Browser, ExecutionTarget::Cloud];

    let failure = expect_validation_failure(validate_contract(
        contract,
        &ValidationContext {
            governing_spec: GOVERNING_SPEC,
            validator_version: VALIDATOR_VERSION,
            existing_published: None,
        },
    ))?;

    let codes: Vec<_> = failure.errors.iter().map(|e| &e.code).collect();
    assert!(
        codes.contains(&&ValidationErrorCode::InvalidPlacementConstraint),
        "expected InvalidPlacementConstraint, got {codes:?}"
    );
    Ok(())
}

#[test]
fn subscribable_without_event_trigger_is_rejected() -> Result<(), String> {
    let mut contract = valid_contract();
    contract.service_type = ServiceType::Subscribable;
    contract.event_trigger = None;

    let failure = expect_validation_failure(validate_contract(
        contract,
        &ValidationContext {
            governing_spec: GOVERNING_SPEC,
            validator_version: VALIDATOR_VERSION,
            existing_published: None,
        },
    ))?;

    let codes: Vec<_> = failure.errors.iter().map(|e| &e.code).collect();
    assert!(
        codes.contains(&&ValidationErrorCode::MissingEventTrigger),
        "expected MissingEventTrigger, got {codes:?}"
    );
    Ok(())
}

#[test]
fn subscribable_with_event_trigger_passes() -> Result<(), String> {
    let mut contract = valid_contract();
    contract.service_type = ServiceType::Subscribable;
    contract.event_trigger = Some("content.comments.comment-draft-created".to_string());

    validate_contract(
        contract,
        &ValidationContext {
            governing_spec: GOVERNING_SPEC,
            validator_version: VALIDATOR_VERSION,
            existing_published: None,
        },
    )
    .map_err(|e| format!("{e:?}"))?;
    Ok(())
}

#[test]
fn stateful_without_browser_passes() -> Result<(), String> {
    let mut contract = valid_contract();
    contract.service_type = ServiceType::Stateful;
    contract.permitted_targets = vec![ExecutionTarget::Cloud, ExecutionTarget::Edge];

    validate_contract(
        contract,
        &ValidationContext {
            governing_spec: GOVERNING_SPEC,
            validator_version: VALIDATOR_VERSION,
            existing_published: None,
        },
    )
    .map_err(|e| format!("{e:?}"))?;
    Ok(())
}

#[test]
fn parses_and_validates_an_event_contract() -> Result<(), String> {
    let parsed =
        parse_event_contract(&valid_event_contract_json()).map_err(|error| format!("{error:?}"))?;
    let result = validate_event_contract(
        parsed.clone(),
        &EventValidationContext {
            governing_spec: EVENT_GOVERNING_SPEC,
            validator_version: VALIDATOR_VERSION,
            existing_published: None,
        },
    )
    .map_err(|error| format!("{error:?}"))?;

    assert_eq!(result.normalized.id, "content.comments.comment-created");
    assert_eq!(
        result.normalized.payload.compatibility,
        PayloadCompatibility::BackwardCompatible
    );
    assert_eq!(result.evidence.governing_spec, EVENT_GOVERNING_SPEC);
    assert_eq!(result.evidence.status, EvidenceStatus::Passed);
    assert_eq!(parsed.lifecycle, Lifecycle::Draft);
    Ok(())
}

#[test]
fn validates_checked_in_expedition_capability_contract_examples() -> Result<(), String> {
    let examples_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../contracts/examples/expedition/capabilities");

    for entry in fs::read_dir(&examples_dir).map_err(|error| format!("{error}"))? {
        let entry = entry.map_err(|error| format!("{error}"))?;
        let contract_path = entry.path().join("contract.json");
        let contract_json = fs::read_to_string(&contract_path)
            .map_err(|error| format!("{}: {error}", contract_path.display()))?;
        let parsed = parse_contract(&contract_json)
            .map_err(|error| format!("{}: {error:?}", contract_path.display()))?;

        validate_contract(
            parsed,
            &ValidationContext {
                governing_spec: "009-expedition-example-artifacts@1.0.0",
                validator_version: VALIDATOR_VERSION,
                existing_published: None,
            },
        )
        .map_err(|error| format!("{}: {error:?}", contract_path.display()))?;
    }

    Ok(())
}

#[test]
fn rejects_invalid_event_identity_and_metadata() {
    let mut contract = valid_event_contract();
    contract.kind = "wrong".to_string();
    contract.schema_version = "9.9.9".to_string();
    contract.id = "wrong.id".to_string();
    contract.namespace = "Content.Comments".to_string();
    contract.name = "Invalid Name".to_string();
    contract.version = "bad".to_string();
    contract.owner.team.clear();
    contract.owner.contact.clear();
    contract.summary = "short".to_string();
    contract.description = "too short".to_string();
    contract.payload.schema = serde_json::json!(["bad"]);
    contract.classification.domain = "x".to_string();
    contract.classification.bounded_context = "y".to_string();
    contract.classification.tags.clear();
    contract.tags.clear();
    contract.publishers.clear();
    contract.provenance.author.clear();
    contract.provenance.created_at.clear();

    let errors: Vec<_> = expect_validation_failure(validate_event_contract(
        contract,
        &EventValidationContext {
            governing_spec: EVENT_GOVERNING_SPEC,
            validator_version: VALIDATOR_VERSION,
            existing_published: None,
        },
    ))
    .into_iter()
    .collect();
    let error = errors[0].clone();

    assert!(
        error
            .errors
            .iter()
            .any(|item| item.code == ValidationErrorCode::InvalidLiteral)
    );
    assert!(
        error
            .errors
            .iter()
            .any(|item| item.code == ValidationErrorCode::InconsistentIdentity)
    );
    assert!(
        error
            .errors
            .iter()
            .any(|item| item.code == ValidationErrorCode::InvalidSemver)
    );
    assert!(
        error
            .errors
            .iter()
            .any(|item| item.path == "$.payload.schema")
    );
    assert!(
        error
            .errors
            .iter()
            .any(|item| item.path == "$.classification.tags")
    );
}

#[test]
fn rejects_duplicate_event_references_and_evidence() {
    let mut contract = valid_event_contract();
    contract.publishers.push(CapabilityReference {
        capability_id: "content.comments.persist-comment".to_string(),
        version: "0.1.0".to_string(),
    });
    contract.subscribers.push(CapabilityReference {
        capability_id: "content.comments.send-notification".to_string(),
        version: "0.1.0".to_string(),
    });
    contract.policies.push(IdReference {
        id: "default-comment-publication".to_string(),
    });
    contract.tags.push("created".to_string());
    contract
        .classification
        .tags
        .push("notifications".to_string());
    contract.evidence.push(EventValidationEvidence {
        kind: "contract_validation".to_string(),
        r#ref: "ref-1".to_string(),
    });
    contract.evidence.push(EventValidationEvidence {
        kind: "contract_validation".to_string(),
        r#ref: "ref-1".to_string(),
    });

    let errors: Vec<_> = expect_validation_failure(validate_event_contract(
        contract,
        &EventValidationContext {
            governing_spec: EVENT_GOVERNING_SPEC,
            validator_version: VALIDATOR_VERSION,
            existing_published: None,
        },
    ))
    .into_iter()
    .collect();
    let error = errors[0].clone();

    assert!(
        error
            .errors
            .iter()
            .any(|item| item.path == "$.publishers[1].capability_id")
    );
    assert!(
        error
            .errors
            .iter()
            .any(|item| item.path == "$.subscribers[1].capability_id")
    );
    assert!(
        error
            .errors
            .iter()
            .any(|item| item.path == "$.policies[1].id")
    );
    assert!(error.errors.iter().any(|item| item.path == "$.tags"));
    assert!(
        error
            .errors
            .iter()
            .any(|item| item.path == "$.classification.tags")
    );
    assert!(
        error
            .errors
            .iter()
            .any(|item| item.path == "$.evidence[1].kind")
    );
}

#[test]
fn validates_checked_in_reference_connector_contracts() -> Result<(), String> {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for relative_path in [
        "contracts/connectors/traverse.http/connector_contract.json",
        "contracts/connectors/traverse.fs.read/connector_contract.json",
        "contracts/connectors/traverse.env/connector_contract.json",
    ] {
        let path = repo_root.join(relative_path);
        let contents = fs::read_to_string(&path)
            .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
        let parsed = parse_connector_contract(&contents).map_err(|error| format!("{error:?}"))?;
        validate_connector_contract(parsed).map_err(|error| format!("{error:?}"))?;
    }

    Ok(())
}

#[test]
fn rejects_invalid_event_boundary_and_published_conflicts() {
    let mut contract = valid_event_contract();
    contract.summary = "Kafka topic for comment created transport".to_string();

    let published = PublishedEventRecord {
        id: contract.id.clone(),
        version: contract.version.clone(),
        governed_content_digest: "0.1.0:deadbeefdeadbeef".to_string(),
        lifecycle: Lifecycle::Active,
    };

    let errors: Vec<_> = expect_validation_failure(validate_event_contract(
        contract,
        &EventValidationContext {
            governing_spec: EVENT_GOVERNING_SPEC,
            validator_version: VALIDATOR_VERSION,
            existing_published: Some(&published),
        },
    ))
    .into_iter()
    .collect();
    let error = errors[0].clone();

    assert!(
        error
            .errors
            .iter()
            .any(|item| item.code == ValidationErrorCode::InvalidEventBoundary)
    );
    assert!(
        error
            .errors
            .iter()
            .any(|item| item.code == ValidationErrorCode::ImmutableVersionConflict)
    );
}

#[test]
fn governed_event_content_digest_ignores_evidence() {
    let contract = valid_event_contract();
    let mut with_evidence = contract.clone();
    with_evidence.evidence.push(EventValidationEvidence {
        kind: "contract_validation".to_string(),
        r#ref: "ref-1".to_string(),
    });

    assert_eq!(
        governed_event_content_digest(&contract),
        governed_event_content_digest(&with_evidence)
    );
}

#[test]
fn rejects_invalid_event_json() {
    let errors: Vec<_> = parse_event_contract("{").err().into_iter().collect();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].errors[0].code, ValidationErrorCode::InvalidFormat);
}

#[test]
fn allows_published_event_records_for_other_versions() {
    let contract = valid_event_contract();
    let published = PublishedEventRecord {
        id: contract.id.clone(),
        version: "9.9.9".to_string(),
        governed_content_digest: "different".to_string(),
        lifecycle: Lifecycle::Active,
    };

    let result = validate_event_contract(
        contract,
        &EventValidationContext {
            governing_spec: EVENT_GOVERNING_SPEC,
            validator_version: VALIDATOR_VERSION,
            existing_published: Some(&published),
        },
    );

    assert!(result.is_ok());
}

fn valid_event_contract_json() -> String {
    serde_json::to_string(&valid_event_contract()).unwrap_or_default()
}

fn valid_event_contract() -> EventContract {
    EventContract {
        kind: "event_contract".to_string(),
        schema_version: "1.0.0".to_string(),
        id: "content.comments.comment-created".to_string(),
        namespace: "content.comments".to_string(),
        name: "comment-created".to_string(),
        version: "0.1.0".to_string(),
        lifecycle: Lifecycle::Draft,
        owner: Owner {
            team: "traverse-core".to_string(),
            contact: "enrico.piovesan10@gmail.com".to_string(),
        },
        summary: "A comment has been created and is ready for downstream processing.".to_string(),
        description:
            "Governed event contract for newly created comments within the comments domain."
                .to_string(),
        payload: EventPayload {
            schema: serde_json::json!({ "type": "object" }),
            compatibility: PayloadCompatibility::BackwardCompatible,
        },
        classification: EventClassification {
            domain: "content".to_string(),
            bounded_context: "comments".to_string(),
            event_type: EventType::Domain,
            tags: vec!["comments".to_string(), "notifications".to_string()],
        },
        publishers: vec![CapabilityReference {
            capability_id: "content.comments.persist-comment".to_string(),
            version: "0.1.0".to_string(),
        }],
        subscribers: vec![CapabilityReference {
            capability_id: "content.comments.send-notification".to_string(),
            version: "0.1.0".to_string(),
        }],
        policies: vec![IdReference {
            id: "default-comment-publication".to_string(),
        }],
        tags: vec!["comments".to_string(), "created".to_string()],
        provenance: EventProvenance {
            source: EventProvenanceSource::Greenfield,
            author: "enricopiovesan".to_string(),
            created_at: "2026-03-26T00:00:00Z".to_string(),
        },
        evidence: Vec::new(),
    }
}
