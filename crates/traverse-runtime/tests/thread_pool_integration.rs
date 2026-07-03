//! Integration tests: ThreadPoolExecutor through the TraverseRuntime stack.
//!
//! Governed by spec `047-thread-pool-executor`.

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use traverse_contracts::{
    BinaryFormat, CapabilityContract, Condition, Entrypoint, EntrypointKind, Execution,
    ExecutionConstraints, ExecutionTarget, FilesystemAccess, HostApiAccess, Lifecycle,
    NetworkAccess, Owner, Provenance, ProvenanceSource, SchemaContainer, ServiceType, SideEffect,
    SideEffectKind,
};
use traverse_runtime::{
    events::{EventCatalog, InProcessBroker},
    executor::{
        ArtifactType, CapabilityExecutor, ExecutorCapability, ExecutorError, NativeExecutor,
        ThreadPoolExecutor, ThreadPoolExecutorConfig,
    },
    placement::{PlacementConstraintEvaluator, RuntimeSnapshot},
    router::{CapabilityExecutorRegistry, PlacementRouter, RouterRequest},
    trace::TraceStore,
};

const TEST_SPEC: &str = "047-thread-pool-executor@1.0.0";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn echo_handler(input: &Value) -> Result<Value, String> {
    Ok(input.clone())
}

fn error_handler(_input: &Value) -> Result<Value, String> {
    Err("deliberate error".to_string())
}

fn pool_executor(
    capacity: usize,
    handler: impl Fn(&Value) -> Result<Value, String> + Send + Sync + 'static,
) -> ThreadPoolExecutor {
    ThreadPoolExecutor::new(
        ThreadPoolExecutorConfig { capacity },
        Box::new(NativeExecutor::new(handler)),
    )
    .unwrap_or_else(|e| panic!("ThreadPoolExecutor construction failed: {e}"))
}

fn test_contract() -> CapabilityContract {
    CapabilityContract {
        kind: "capability_contract".to_string(),
        schema_version: "1.0.0".to_string(),
        id: "pool.integration.subject".to_string(),
        namespace: "pool.integration".to_string(),
        name: "subject".to_string(),
        version: "0.1.0".to_string(),
        lifecycle: Lifecycle::Draft,
        owner: Owner {
            team: "traverse-core".to_string(),
            contact: "test@example.com".to_string(),
        },
        summary: "Thread pool integration test subject.".to_string(),
        description: "Used only in thread pool integration tests.".to_string(),
        inputs: SchemaContainer { schema: json!({ "type": "object" }) },
        outputs: SchemaContainer { schema: json!({ "type": "object" }) },
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
                command: "pool.integration.subject".to_string(),
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
            author: "traverse-core".to_string(),
            created_at: "2026-07-03T00:00:00Z".to_string(),
            spec_ref: Some(TEST_SPEC.to_string()),
            adr_refs: Vec::new(),
            exception_refs: Vec::new(),
        },
        evidence: Vec::new(),
        service_type: ServiceType::Stateless,
        permitted_targets: vec![ExecutionTarget::Local],
        event_trigger: None,
        connector_requirements: Vec::new(),
        state_schema: None,
    }
}

fn executor_cap(artifact_type: ArtifactType) -> ExecutorCapability {
    ExecutorCapability {
        capability_id: "pool.integration.subject".to_string(),
        artifact_type,
        wasm_binary_path: None,
        wasm_checksum: None,
        host_abi_version: None,
    }
}

fn idle_snapshot() -> RuntimeSnapshot {
    RuntimeSnapshot {
        target_loads: [(ExecutionTarget::Local, 0.0)].into_iter().collect(),
    }
}

fn build_router(
    executor: Box<dyn CapabilityExecutor>,
) -> (PlacementRouter, Arc<Mutex<TraceStore>>) {
    let catalog = Arc::new(EventCatalog::new());
    let broker = Arc::new(
        InProcessBroker::new(Arc::clone(&catalog))
            .unwrap_or_else(|e| panic!("broker init failed: {e}")),
    );
    let trace_store = Arc::new(Mutex::new(TraceStore::new()));
    let mut registry = CapabilityExecutorRegistry::new();
    registry.insert(ArtifactType::Native, executor);
    let router = PlacementRouter::new(
        PlacementConstraintEvaluator,
        registry,
        Arc::clone(&trace_store),
        broker,
    );
    (router, trace_store)
}

fn make_request(input: Value) -> RouterRequest {
    RouterRequest {
        capability_id: "pool.integration.subject".to_string(),
        artifact_type: ArtifactType::Native,
        contract: test_contract(),
        target_hint: Some(ExecutionTarget::Local),
        runtime_snapshot: idle_snapshot(),
        input,
        executor_capability: executor_cap(ArtifactType::Native),
        emitted_events: Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// Drop-in replacement
// ---------------------------------------------------------------------------

#[test]
fn native_executor_and_thread_pool_produce_identical_output() {
    let input = json!({ "key": "value" });
    let cap = executor_cap(ArtifactType::Native);

    let native = NativeExecutor::new(echo_handler);
    let native_result = native.execute(&cap, &input);

    let pool = pool_executor(2, echo_handler);
    let pool_result = pool.execute(&cap, &input);

    assert_eq!(native_result.ok(), pool_result.ok());
}

#[test]
fn thread_pool_executor_satisfies_capability_executor_trait_object() {
    let executor: Box<dyn CapabilityExecutor> = Box::new(pool_executor(2, echo_handler));
    let cap = executor_cap(ArtifactType::Native);
    let result = executor.execute(&cap, &json!({}));
    assert!(result.is_ok(), "trait object execute failed: {result:?}");
}

// ---------------------------------------------------------------------------
// Router integration
// ---------------------------------------------------------------------------

#[test]
fn router_routes_to_thread_pool_executor() {
    let (router, _) = build_router(Box::new(pool_executor(2, echo_handler)));
    let input = json!({ "x": 1 });
    let result = router.execute(make_request(input.clone()));
    assert!(result.is_ok(), "router execute failed: {result:?}");
    assert_eq!(result.ok().map(|r| r.output), Some(input));
}

#[test]
fn router_concurrent_requests_to_same_capability() {
    let (router, _) = build_router(Box::new(pool_executor(8, echo_handler)));
    let router = Arc::new(router);
    let errors: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(vec![]));

    std::thread::scope(|s| {
        for i in 0_u32..8 {
            let router = Arc::clone(&router);
            let errors = Arc::clone(&errors);
            s.spawn(move || {
                match router.execute(make_request(json!({ "i": i }))) {
                    Ok(resp) => {
                        if resp.output != json!({ "i": i }) {
                            errors
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .push(format!("thread {i}: wrong output {:?}", resp.output));
                        }
                    }
                    Err(e) => {
                        errors
                            .lock()
                            .unwrap_or_else(|e2| e2.into_inner())
                            .push(format!("thread {i}: router error {e:?}"));
                    }
                }
            });
        }
    });

    let errors = errors.lock().unwrap_or_else(|e| e.into_inner());
    assert!(errors.is_empty(), "concurrent router errors: {errors:?}");
}

// ---------------------------------------------------------------------------
// Trace correctness
// ---------------------------------------------------------------------------

#[test]
fn concurrent_executions_produce_isolated_traces() {
    let (router, trace_store) = build_router(Box::new(pool_executor(4, echo_handler)));
    let router = Arc::new(router);

    std::thread::scope(|s| {
        for i in 0_u32..4 {
            let router = Arc::clone(&router);
            s.spawn(move || {
                let _ = router.execute(make_request(json!({ "i": i })));
            });
        }
    });

    let store = trace_store.lock().unwrap_or_else(|e| e.into_inner());
    let entries = store.list_public(None);
    assert_eq!(entries.len(), 4, "expected 4 trace entries, got {}", entries.len());
    assert!(
        entries.iter().all(|e| e.capability_id == "pool.integration.subject"),
        "unexpected capability_id in traces"
    );
}

#[test]
fn trace_capability_id_matches_executed_capability() {
    let (router, trace_store) = build_router(Box::new(pool_executor(2, echo_handler)));
    let _ = router.execute(make_request(json!({})));
    let store = trace_store.lock().unwrap_or_else(|e| e.into_inner());
    let entries = store.list_public(Some("pool.integration.subject"));
    assert!(!entries.is_empty(), "no trace entries found");
    assert!(
        entries.iter().all(|e| e.capability_id == "pool.integration.subject"),
        "capability_id mismatch in trace"
    );
}

#[test]
fn failed_execution_returns_router_error() {
    // The router returns early with RouterError::ExecutionFailed before writing a trace.
    // This test verifies the error propagates correctly through the pool dispatch path.
    let (router, _) = build_router(Box::new(pool_executor(1, error_handler)));
    let result = router.execute(make_request(json!({})));
    assert!(result.is_err(), "expected error from failing capability, got ok");
    let err_msg = format!("{:?}", result.err());
    assert!(
        err_msg.contains("ExecutionFailed") || err_msg.contains("deliberate"),
        "unexpected error shape: {err_msg}"
    );
}

// ---------------------------------------------------------------------------
// WASM path unchanged
// ---------------------------------------------------------------------------

#[test]
fn wasm_capability_type_rejected_by_thread_pool_executor() {
    let pool = pool_executor(2, echo_handler);
    let wasm_cap = executor_cap(ArtifactType::Wasm);
    let result = pool.execute(&wasm_cap, &json!({}));
    assert_eq!(
        result,
        Err(ExecutorError::UnsupportedArtifactType),
        "expected UnsupportedArtifactType for Wasm artifact"
    );
}
