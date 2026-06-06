//! Integration tests for the placement constraint evaluator.
//!
//! Governed by spec: 024-placement-constraint-evaluator

use traverse_contracts::{
    BinaryFormat, CapabilityContract, Condition, Entrypoint, EntrypointKind, Execution,
    ExecutionConstraints, ExecutionTarget, FilesystemAccess, HostApiAccess, Lifecycle,
    NetworkAccess, Owner, Provenance, ProvenanceSource, SchemaContainer, ServiceType, SideEffect,
    SideEffectKind,
};
use traverse_runtime::placement::{
    PlacementConfidence, PlacementConstraintEvaluator, PlacementError, PlacementReason,
    PlacementRequest, RuntimeSnapshot,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const GOVERNING_SPEC: &str = "002-capability-contracts@0.1.0";

fn base_contract() -> CapabilityContract {
    CapabilityContract {
        kind: "capability_contract".to_string(),
        schema_version: "1.0.0".to_string(),
        id: "placement.tests.evaluator-subject".to_string(),
        namespace: "placement.tests".to_string(),
        name: "evaluator-subject".to_string(),
        version: "0.1.0".to_string(),
        lifecycle: Lifecycle::Draft,
        owner: Owner {
            team: "traverse-core".to_string(),
            contact: "test@example.com".to_string(),
        },
        summary: "Placement evaluator test subject capability.".to_string(),
        description: "Used only in placement evaluator tests, not a real capability.".to_string(),
        inputs: SchemaContainer {
            schema: serde_json::json!({ "type": "object" }),
        },
        outputs: SchemaContainer {
            schema: serde_json::json!({ "type": "object" }),
        },
        preconditions: vec![Condition {
            id: "always-met".to_string(),
            description: "No preconditions in test.".to_string(),
        }],
        postconditions: vec![Condition {
            id: "always-met".to_string(),
            description: "No postconditions in test.".to_string(),
        }],
        side_effects: vec![SideEffect {
            kind: SideEffectKind::MemoryOnly,
            description: "No durable side effect.".to_string(),
        }],
        emits: Vec::new(),
        consumes: Vec::new(),
        permissions: Vec::new(),
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
        policies: Vec::new(),
        dependencies: Vec::new(),
        provenance: Provenance {
            source: ProvenanceSource::Greenfield,
            author: "test-author".to_string(),
            created_at: "2026-04-01T00:00:00Z".to_string(),
            spec_ref: Some(GOVERNING_SPEC.to_string()),
            adr_refs: Vec::new(),
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
        state_schema: None,
    }
}

fn snapshot_with(pairs: &[(ExecutionTarget, f32)]) -> RuntimeSnapshot {
    RuntimeSnapshot {
        target_loads: pairs.iter().cloned().collect(),
    }
}

fn evaluator() -> PlacementConstraintEvaluator {
    PlacementConstraintEvaluator
}

// ---------------------------------------------------------------------------
// Tier 1 tests
// ---------------------------------------------------------------------------

#[test]
fn tier1_hint_accepted_when_in_permitted_targets() -> Result<(), PlacementError> {
    let contract = base_contract(); // permitted: Local, Cloud, Edge
    let request = PlacementRequest {
        capability_id: "placement.tests.evaluator-subject".to_string(),
        target_hint: Some(ExecutionTarget::Cloud),
        runtime_snapshot: snapshot_with(&[(ExecutionTarget::Cloud, 0.3)]),
    };

    let decision = evaluator().evaluate(&request, &contract)?;

    assert!(
        matches!(decision.target, ExecutionTarget::Cloud),
        "expected Cloud to be selected"
    );
    assert!(
        matches!(decision.reason, PlacementReason::CallerHintAccepted),
        "expected CallerHintAccepted reason"
    );
    assert!(
        matches!(decision.confidence, PlacementConfidence::High),
        "load 0.3 → High confidence"
    );
    Ok(())
}

#[test]
fn tier1_hint_rejected_falls_through_to_tier3() -> Result<(), PlacementError> {
    let contract = base_contract(); // permitted: Local, Cloud, Edge
    // Hint is Browser, which is not in permitted_targets → must fall through.
    let request = PlacementRequest {
        capability_id: "placement.tests.evaluator-subject".to_string(),
        target_hint: Some(ExecutionTarget::Browser),
        runtime_snapshot: snapshot_with(&[
            (ExecutionTarget::Local, 0.2),
            (ExecutionTarget::Cloud, 0.5),
            (ExecutionTarget::Edge, 0.8),
        ]),
    };

    let decision = evaluator().evaluate(&request, &contract)?;

    // Lowest load eligible target is Local (0.2), Edge is Low (0.8 < 0.9 still eligible).
    assert!(
        matches!(decision.target, ExecutionTarget::Local),
        "expected Local (lowest load) after hint rejection"
    );
    assert!(
        matches!(decision.reason, PlacementReason::HeuristicSelected),
        "expected HeuristicSelected after tier-1 rejection"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Tier 2 tests
// ---------------------------------------------------------------------------

#[test]
fn tier2_filters_browser_for_stateful_service() -> Result<(), PlacementError> {
    let mut contract = base_contract();
    contract.service_type = ServiceType::Stateful;
    contract.permitted_targets = vec![
        ExecutionTarget::Local,
        ExecutionTarget::Browser,
        ExecutionTarget::Cloud,
    ];

    let request = PlacementRequest {
        capability_id: "placement.tests.evaluator-subject".to_string(),
        target_hint: None,
        runtime_snapshot: snapshot_with(&[
            (ExecutionTarget::Browser, 0.1), // lowest load but must be filtered
            (ExecutionTarget::Local, 0.4),
            (ExecutionTarget::Cloud, 0.6),
        ]),
    };

    let decision = evaluator().evaluate(&request, &contract)?;

    assert!(
        !matches!(decision.target, ExecutionTarget::Browser),
        "Browser must be excluded for Stateful services"
    );
    assert!(
        matches!(decision.target, ExecutionTarget::Local),
        "Local should win after Browser exclusion (lower load than Cloud)"
    );
    Ok(())
}

#[test]
fn tier2_permitted_targets_restricts_universe() -> Result<(), PlacementError> {
    let mut contract = base_contract();
    // Only Local is permitted — all others must be ignored regardless of load.
    contract.permitted_targets = vec![ExecutionTarget::Local];

    let request = PlacementRequest {
        capability_id: "placement.tests.evaluator-subject".to_string(),
        target_hint: None,
        runtime_snapshot: snapshot_with(&[
            (ExecutionTarget::Cloud, 0.0), // lower load but not permitted
            (ExecutionTarget::Local, 0.4),
        ]),
    };

    let decision = evaluator().evaluate(&request, &contract)?;

    assert!(
        matches!(decision.target, ExecutionTarget::Local),
        "only permitted target should be chosen"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Tier 3 tests
// ---------------------------------------------------------------------------

#[test]
fn tier3_selects_lowest_load_target() -> Result<(), PlacementError> {
    let contract = base_contract(); // permitted: Local, Cloud, Edge

    let request = PlacementRequest {
        capability_id: "placement.tests.evaluator-subject".to_string(),
        target_hint: None,
        runtime_snapshot: snapshot_with(&[
            (ExecutionTarget::Local, 0.6),
            (ExecutionTarget::Cloud, 0.2),
            (ExecutionTarget::Edge, 0.4),
        ]),
    };

    let decision = evaluator().evaluate(&request, &contract)?;

    assert!(
        matches!(decision.target, ExecutionTarget::Cloud),
        "Cloud has the lowest load and should be selected"
    );
    assert!(
        matches!(decision.reason, PlacementReason::HeuristicSelected),
        "reason must be HeuristicSelected"
    );
    assert!(
        matches!(decision.confidence, PlacementConfidence::High),
        "load 0.2 → High confidence"
    );
    Ok(())
}

#[test]
fn tier3_overloaded_targets_excluded() -> Result<(), PlacementError> {
    let contract = base_contract(); // permitted: Local, Cloud, Edge

    let request = PlacementRequest {
        capability_id: "placement.tests.evaluator-subject".to_string(),
        target_hint: None,
        runtime_snapshot: snapshot_with(&[
            (ExecutionTarget::Local, 0.95), // overloaded → excluded
            (ExecutionTarget::Cloud, 0.91), // overloaded → excluded
            (ExecutionTarget::Edge, 0.85),  // high but still eligible (0.85 <= 0.9)
        ]),
    };

    let decision = evaluator().evaluate(&request, &contract)?;

    assert!(
        matches!(decision.target, ExecutionTarget::Edge),
        "only non-overloaded target should win"
    );
    assert!(
        matches!(decision.confidence, PlacementConfidence::Low),
        "load 0.85 → Low confidence"
    );
    Ok(())
}

#[test]
fn no_eligible_target_returns_error() {
    let contract = base_contract(); // permitted: Local, Cloud, Edge

    // All permitted targets overloaded.
    let request = PlacementRequest {
        capability_id: "placement.tests.evaluator-subject".to_string(),
        target_hint: None,
        runtime_snapshot: snapshot_with(&[
            (ExecutionTarget::Local, 0.95),
            (ExecutionTarget::Cloud, 0.92),
            (ExecutionTarget::Edge, 0.91),
        ]),
    };

    let result = evaluator().evaluate(&request, &contract);
    assert!(
        matches!(result, Err(PlacementError::NoEligibleTarget)),
        "should return NoEligibleTarget when all targets are overloaded"
    );
}

#[test]
fn deterministic_same_inputs_same_output() -> Result<(), PlacementError> {
    let contract = base_contract(); // permitted: Local, Cloud, Edge

    // Two targets with equal load → lexicographic tiebreak must produce the same winner each time.
    let make_request = || PlacementRequest {
        capability_id: "placement.tests.evaluator-subject".to_string(),
        target_hint: None,
        runtime_snapshot: snapshot_with(&[
            (ExecutionTarget::Local, 0.3),
            (ExecutionTarget::Cloud, 0.3),
            (ExecutionTarget::Edge, 0.5),
        ]),
    };

    let ev = evaluator();
    let first = ev.evaluate(&make_request(), &contract)?;
    let second = ev.evaluate(&make_request(), &contract)?;

    assert_eq!(
        format!("{:?}", first.target),
        format!("{:?}", second.target),
        "same inputs must produce the same target (determinism)"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Confidence threshold tests
// ---------------------------------------------------------------------------

#[test]
fn confidence_medium_for_load_between_0_5_and_0_75() -> Result<(), PlacementError> {
    let mut contract = base_contract();
    contract.permitted_targets = vec![ExecutionTarget::Local];

    let request = PlacementRequest {
        capability_id: "placement.tests.evaluator-subject".to_string(),
        target_hint: None,
        runtime_snapshot: snapshot_with(&[(ExecutionTarget::Local, 0.6)]),
    };

    let decision = evaluator().evaluate(&request, &contract)?;

    assert!(
        matches!(decision.confidence, PlacementConfidence::Medium),
        "load 0.6 should produce Medium confidence"
    );
    Ok(())
}
