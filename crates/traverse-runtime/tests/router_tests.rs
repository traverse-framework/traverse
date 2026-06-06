//! Governed by spec 016-runtime-placement-router
//!
//! Integration tests for [`PlacementRouter`].

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use traverse_contracts::{
    BinaryFormat, CapabilityContract, Condition, Entrypoint, EntrypointKind, EventReference,
    Execution, ExecutionConstraints, ExecutionTarget, FilesystemAccess, HostApiAccess, Lifecycle,
    NetworkAccess, Owner, Provenance, ProvenanceSource, SchemaContainer, ServiceType, SideEffect,
    SideEffectKind,
};
use traverse_runtime::{
    events::{
        EventBroker, EventCatalog, EventCatalogEntry, InProcessBroker, LifecycleStatus,
        TraverseEvent,
    },
    executor::{ArtifactType, ExecutorCapability, NativeExecutor},
    placement::{PlacementConstraintEvaluator, PlacementError, RuntimeSnapshot},
    router::{CapabilityExecutorRegistry, PlacementRouter, RouterError, RouterRequest},
    trace::TraceStore,
};

// ---------------------------------------------------------------------------
// Fixtures & helpers
// ---------------------------------------------------------------------------

const TEST_SPEC: &str = "016-runtime-placement-router@1.0.0";

/// Helper: assert result is Err and return the error without using `expect_err`.
fn must_err<T: std::fmt::Debug, E>(result: Result<T, E>, msg: &str) -> Result<E, String> {
    match result {
        Err(e) => Ok(e),
        Ok(v) => Err(format!("{msg}: got Ok({v:?})")),
    }
}

fn base_contract(service_type: ServiceType) -> CapabilityContract {
    CapabilityContract {
        kind: "capability_contract".to_string(),
        schema_version: "1.0.0".to_string(),
        id: "router.tests.subject".to_string(),
        namespace: "router.tests".to_string(),
        name: "subject".to_string(),
        version: "0.1.0".to_string(),
        lifecycle: Lifecycle::Draft,
        owner: Owner {
            team: "traverse-core".to_string(),
            contact: "test@example.com".to_string(),
        },
        summary: "Router integration test subject.".to_string(),
        description: "Used only in router tests.".to_string(),
        inputs: SchemaContainer {
            schema: json!({ "type": "object" }),
        },
        outputs: SchemaContainer {
            schema: json!({ "type": "object" }),
        },
        preconditions: vec![Condition {
            id: "always-met".to_string(),
            description: "No preconditions.".to_string(),
        }],
        postconditions: vec![Condition {
            id: "always-met".to_string(),
            description: "No postconditions.".to_string(),
        }],
        side_effects: vec![SideEffect {
            kind: SideEffectKind::MemoryOnly,
            description: "No durable side effects.".to_string(),
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
            spec_ref: Some(TEST_SPEC.to_string()),
            adr_refs: Vec::new(),
            exception_refs: Vec::new(),
        },
        evidence: Vec::new(),
        service_type,
        permitted_targets: vec![ExecutionTarget::Local, ExecutionTarget::Cloud],
        event_trigger: None,
        connector_requirements: Vec::new(),
        state_schema: None,
    }
}

fn idle_snapshot() -> RuntimeSnapshot {
    RuntimeSnapshot {
        target_loads: [(ExecutionTarget::Local, 0.0)].into_iter().collect(),
    }
}

fn native_executor_capability(capability_id: &str) -> ExecutorCapability {
    ExecutorCapability {
        capability_id: capability_id.to_string(),
        artifact_type: ArtifactType::Native,
        wasm_binary_path: None,
        wasm_checksum: None,
        host_abi_version: None,
    }
}

/// Build a minimal broker with one active event type.
fn broker_with_event(event_type: &str) -> Result<Arc<InProcessBroker>, String> {
    let catalog = Arc::new(EventCatalog::new());
    catalog
        .register(EventCatalogEntry {
            event_type: event_type.to_string(),
            owner: "router.tests".to_string(),
            version: "1.0.0".to_string(),
            lifecycle_status: LifecycleStatus::Active,
            consumer_count: 0,
        })
        .map_err(|e| e.to_string())?;
    Ok(Arc::new(
        InProcessBroker::new(catalog).map_err(|e| e.to_string())?,
    ))
}

/// Build a router with a single native executor that returns `output`.
fn make_router_with_native(
    output: Value,
    trace_store: Arc<Mutex<TraceStore>>,
    broker: Arc<dyn EventBroker>,
) -> PlacementRouter {
    let mut registry: CapabilityExecutorRegistry = CapabilityExecutorRegistry::new();
    registry.insert(
        ArtifactType::Native,
        Box::new(NativeExecutor::new(move |_| Ok(output.clone()))),
    );
    PlacementRouter::new(PlacementConstraintEvaluator, registry, trace_store, broker)
}

/// Build a [`TraverseEvent`] for tests.
fn sample_event(event_type: &str) -> TraverseEvent {
    TraverseEvent {
        id: uuid::Uuid::new_v4().to_string(),
        source: "traverse-runtime/router.tests.subject".to_string(),
        event_type: event_type.to_string(),
        datacontenttype: "application/json".to_string(),
        time: "2026-04-08T00:00:00Z".to_string(),
        data: json!({}),
        owner: "router.tests".to_string(),
        version: "1.0.0".to_string(),
        lifecycle_status: LifecycleStatus::Active,
    }
}

// ---------------------------------------------------------------------------
// Test: Native capability executes end-to-end; trace is written
// ---------------------------------------------------------------------------

#[test]
fn native_capability_executes_and_writes_trace() -> Result<(), String> {
    let trace_store = Arc::new(Mutex::new(TraceStore::new()));
    let broker: Arc<dyn EventBroker> = broker_with_event("dev.traverse.test.happened")?;

    let router =
        make_router_with_native(json!({ "result": "ok" }), Arc::clone(&trace_store), broker);

    let request = RouterRequest {
        capability_id: "router.tests.subject".to_string(),
        artifact_type: ArtifactType::Native,
        contract: base_contract(ServiceType::Stateless),
        target_hint: Some(ExecutionTarget::Local),
        runtime_snapshot: idle_snapshot(),
        input: json!({ "key": "value" }),
        executor_capability: native_executor_capability("router.tests.subject"),
        emitted_events: Vec::new(),
    };

    let response = router.execute(request).map_err(|e| e.to_string())?;

    // Output is correct
    assert_eq!(response.output, json!({ "result": "ok" }));
    assert!(!response.trace_id.is_empty(), "trace_id must be non-empty");

    // Trace entry was written
    let store = trace_store.lock().map_err(|_| "lock poisoned")?;
    let entries = store.list_public(Some("router.tests.subject"));
    assert_eq!(entries.len(), 1, "exactly one trace entry expected");

    let entry = entries[0];
    assert_eq!(entry.capability_id, "router.tests.subject");
    assert_eq!(
        entry.outcome,
        traverse_runtime::trace::TraceOutcome::Success
    );
    assert!(!entry.placement_target.is_empty());

    Ok(())
}

// ---------------------------------------------------------------------------
// Test: Placement failure → PlacementFailed, no trace
// ---------------------------------------------------------------------------

#[test]
fn placement_failure_returns_error_and_no_trace() -> Result<(), String> {
    let trace_store = Arc::new(Mutex::new(TraceStore::new()));
    let broker: Arc<dyn EventBroker> = broker_with_event("dev.traverse.test.happened")?;

    let router = make_router_with_native(json!({}), Arc::clone(&trace_store), broker);

    // Contract only permits Cloud; snapshot has Browser overloaded, no Cloud entry → no target survives
    let mut contract = base_contract(ServiceType::Stateless);
    contract.permitted_targets = vec![ExecutionTarget::Cloud];

    // All targets at 100% load → placement fails
    let request = RouterRequest {
        capability_id: "router.tests.subject".to_string(),
        artifact_type: ArtifactType::Native,
        contract,
        target_hint: None,
        runtime_snapshot: RuntimeSnapshot {
            target_loads: [(ExecutionTarget::Cloud, 1.0)].into_iter().collect(),
        },
        input: json!({}),
        executor_capability: native_executor_capability("router.tests.subject"),
        emitted_events: Vec::new(),
    };

    let err = must_err(router.execute(request), "expected placement error")?;

    assert_eq!(
        err,
        RouterError::PlacementFailed(PlacementError::NoEligibleTarget)
    );

    // No trace must have been written
    let store = trace_store.lock().map_err(|_| "lock poisoned")?;
    assert!(
        store.list_public(None).is_empty(),
        "no trace must be written on placement failure"
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Test: Missing executor → ExecutorNotFound
// ---------------------------------------------------------------------------

#[test]
fn missing_executor_returns_not_found_error() -> Result<(), String> {
    let trace_store = Arc::new(Mutex::new(TraceStore::new()));
    let broker: Arc<dyn EventBroker> = broker_with_event("dev.traverse.test.happened")?;

    // Registry has no executors at all
    let empty_registry: CapabilityExecutorRegistry = CapabilityExecutorRegistry::new();
    let router = PlacementRouter::new(
        PlacementConstraintEvaluator,
        empty_registry,
        Arc::clone(&trace_store),
        broker,
    );

    let request = RouterRequest {
        capability_id: "router.tests.subject".to_string(),
        artifact_type: ArtifactType::Native,
        contract: base_contract(ServiceType::Stateless),
        target_hint: Some(ExecutionTarget::Local),
        runtime_snapshot: idle_snapshot(),
        input: json!({}),
        executor_capability: native_executor_capability("router.tests.subject"),
        emitted_events: Vec::new(),
    };

    let err = must_err(router.execute(request), "expected executor-not-found error")?;

    assert!(
        matches!(err, RouterError::ExecutorNotFound(_)),
        "expected ExecutorNotFound, got {err:?}"
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Test: Subscribable capability publishes emitted events
// ---------------------------------------------------------------------------

#[test]
fn subscribable_capability_publishes_events() -> Result<(), String> {
    let trace_store = Arc::new(Mutex::new(TraceStore::new()));
    let event_type = "dev.traverse.router.test.emitted";
    let broker = broker_with_event(event_type)?;

    let sub = broker
        .subscribe(event_type, "0")
        .map_err(|e| e.to_string())?;

    let broker_arc: Arc<dyn EventBroker> = broker;

    let router = make_router_with_native(
        json!({ "done": true }),
        Arc::clone(&trace_store),
        Arc::clone(&broker_arc),
    );

    // Subscribable contract
    let mut contract = base_contract(ServiceType::Subscribable);
    contract.event_trigger = Some("dev.traverse.router.test.triggered".to_string());
    contract.emits = vec![EventReference {
        event_id: event_type.to_string(),
        version: "1.0.0".to_string(),
    }];

    let request = RouterRequest {
        capability_id: "router.tests.subject".to_string(),
        artifact_type: ArtifactType::Native,
        contract,
        target_hint: Some(ExecutionTarget::Local),
        runtime_snapshot: idle_snapshot(),
        input: json!({}),
        executor_capability: native_executor_capability("router.tests.subject"),
        emitted_events: vec![sample_event(event_type)],
    };

    router.execute(request).map_err(|e| e.to_string())?;

    let poll = broker_arc
        .poll(&sub.subscription_id, 10)
        .map_err(|e| e.to_string())?;
    assert_eq!(poll.events.len(), 1, "one event must be delivered");
    assert_eq!(poll.events[0].event.event_type, event_type);

    Ok(())
}

#[test]
fn undeclared_event_emission_fails_execution_and_is_recorded() -> Result<(), String> {
    let trace_store = Arc::new(Mutex::new(TraceStore::new()));
    let event_type = "dev.traverse.router.test.undeclared";
    let broker = broker_with_event(event_type)?;
    let broker_arc: Arc<dyn EventBroker> = broker;

    let router = make_router_with_native(
        json!({ "done": true }),
        Arc::clone(&trace_store),
        Arc::clone(&broker_arc),
    );

    let mut contract = base_contract(ServiceType::Subscribable);
    contract.event_trigger = Some("dev.traverse.router.test.triggered".to_string());
    contract.emits = Vec::new();

    let request = RouterRequest {
        capability_id: "router.tests.subject".to_string(),
        artifact_type: ArtifactType::Native,
        contract,
        target_hint: Some(ExecutionTarget::Local),
        runtime_snapshot: idle_snapshot(),
        input: json!({}),
        executor_capability: native_executor_capability("router.tests.subject"),
        emitted_events: vec![sample_event(event_type)],
    };

    let err = must_err(router.execute(request), "expected contract violation")?;
    assert!(
        matches!(err, RouterError::ContractViolation(_)),
        "expected ContractViolation, got {err:?}"
    );

    let store = trace_store
        .lock()
        .map_err(|_| "trace store lock poisoned".to_string())?;
    let traces = store.list_public(Some("router.tests.subject"));
    assert!(
        traces.iter().any(|trace| {
            trace.outcome == traverse_runtime::trace::TraceOutcome::Failure
                && trace
                    .violations
                    .iter()
                    .any(|v| v.violation_code == "undeclared_event_emission")
        }),
        "expected a failure trace entry with undeclared_event_emission"
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Test: Stateless capability does NOT publish events even if emitted_events is non-empty
// ---------------------------------------------------------------------------

#[test]
fn stateless_capability_does_not_publish_events() -> Result<(), String> {
    let trace_store = Arc::new(Mutex::new(TraceStore::new()));
    let event_type = "dev.traverse.router.test.emitted";
    let broker = broker_with_event(event_type)?;

    let sub = broker
        .subscribe(event_type, "0")
        .map_err(|e| e.to_string())?;

    let broker_arc: Arc<dyn EventBroker> = broker;

    let router =
        make_router_with_native(json!({}), Arc::clone(&trace_store), Arc::clone(&broker_arc));

    let request = RouterRequest {
        capability_id: "router.tests.subject".to_string(),
        artifact_type: ArtifactType::Native,
        contract: base_contract(ServiceType::Stateless), // NOT Subscribable
        target_hint: Some(ExecutionTarget::Local),
        runtime_snapshot: idle_snapshot(),
        input: json!({}),
        executor_capability: native_executor_capability("router.tests.subject"),
        emitted_events: vec![sample_event(event_type)], // provided but must not be published
    };

    router.execute(request).map_err(|e| e.to_string())?;

    let poll = broker_arc
        .poll(&sub.subscription_id, 10)
        .map_err(|e| e.to_string())?;
    assert!(poll.events.is_empty(), "stateless must not publish events");

    Ok(())
}

// ---------------------------------------------------------------------------
// Test: RouterError Display covers all variants
// ---------------------------------------------------------------------------

#[test]
fn router_error_display_covers_all_variants() {
    use traverse_runtime::router::RouterError;

    use traverse_contracts::ViolationRecord;
    let cases: Vec<RouterError> = vec![
        RouterError::PlacementFailed(PlacementError::NoEligibleTarget),
        RouterError::ExecutorNotFound("Wasm".to_string()),
        RouterError::ExecutionFailed("test failure".to_string()),
        RouterError::ContractViolation(vec![ViolationRecord::new(
            "undeclared_event_emission",
            "test.cap",
            "test message",
        )]),
        RouterError::TraceLockPoisoned,
    ];

    for err in &cases {
        let msg = err.to_string();
        assert!(
            !msg.is_empty(),
            "Display must produce non-empty string for {err:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// Test: Executor error → ExecutionFailed, no trace written
// ---------------------------------------------------------------------------

#[test]
fn executor_error_returns_execution_failed() -> Result<(), String> {
    let trace_store = Arc::new(Mutex::new(TraceStore::new()));
    let broker: Arc<dyn EventBroker> = broker_with_event("dev.traverse.test.happened")?;

    let mut registry: CapabilityExecutorRegistry = CapabilityExecutorRegistry::new();
    registry.insert(
        ArtifactType::Native,
        Box::new(NativeExecutor::new(|_| {
            Err("simulated executor failure".to_string())
        })),
    );

    let router = PlacementRouter::new(
        PlacementConstraintEvaluator,
        registry,
        Arc::clone(&trace_store),
        broker,
    );

    let request = RouterRequest {
        capability_id: "router.tests.subject".to_string(),
        artifact_type: ArtifactType::Native,
        contract: base_contract(ServiceType::Stateless),
        target_hint: Some(ExecutionTarget::Local),
        runtime_snapshot: idle_snapshot(),
        input: json!({}),
        executor_capability: native_executor_capability("router.tests.subject"),
        emitted_events: Vec::new(),
    };

    let err = must_err(router.execute(request), "expected execution error")?;

    assert!(
        matches!(err, RouterError::ExecutionFailed(_)),
        "expected ExecutionFailed, got {err:?}"
    );

    Ok(())
}
