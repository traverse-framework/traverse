//! Live `Runtime::execute` → `PlacementRouter` wiring (issue #963).

#![allow(clippy::expect_used)]

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use traverse_contracts::{
    BinaryFormat as ContractBinaryFormat, Condition, Entrypoint, EntrypointKind, EventReference,
    Execution, ExecutionConstraints, ExecutionTarget, FilesystemAccess, HostApiAccess, Lifecycle,
    NetworkAccess, Owner, Provenance, ProvenanceSource, SchemaContainer, ServiceType, SideEffect,
    SideEffectKind,
};
use traverse_registry::{
    ArtifactDigests, BinaryFormat, BinaryReference, CapabilityArtifactRecord,
    CapabilityRegistration, CapabilityRegistry, ComposabilityMetadata, CompositionKind,
    CompositionPattern, ImplementationKind, RegistryProvenance, RegistryScope, SourceKind,
    SourceReference,
};
use traverse_runtime::events::{
    EventBroker, EventCatalog, EventCatalogEntry, InProcessBroker, LifecycleStatus,
    RuntimeEventSink, TraverseEvent,
};
use traverse_runtime::security::RuntimeSecurityConfig;
use traverse_runtime::trace::{TraceOutcome, TraceStore};
use traverse_runtime::{
    ExecutionFailureReason, LocalExecutionFailure, LocalExecutionFailureCode, LocalExecutionOutput,
    LocalExecutor, Runtime, RuntimeContext, RuntimeErrorCode, RuntimeLookup, RuntimeLookupScope,
    RuntimeRequest, RuntimeResultStatus, RuntimeState,
};

#[derive(Debug, Default)]
struct RecordingEventSink {
    events: Mutex<Vec<TraverseEvent>>,
}

impl RuntimeEventSink for RecordingEventSink {
    fn emit(&self, event: TraverseEvent) -> Result<(), traverse_runtime::events::EventError> {
        self.events
            .lock()
            .map_err(|_| {
                traverse_runtime::events::EventError::LifecycleViolation(
                    "recording sink lock poisoned".to_string(),
                )
            })?
            .push(event);
        Ok(())
    }
}

struct EmittingNativeExecutor;

impl LocalExecutor for EmittingNativeExecutor {
    fn execute(
        &self,
        capability: &traverse_registry::ResolvedCapability,
        _input: &Value,
    ) -> Result<LocalExecutionOutput, LocalExecutionFailure> {
        // Spec 101-local-executor-event-emission FR-001/FR-002: emit every
        // event this capability's contract declares, so tests exercising the
        // live `Runtime::execute()` path can assert it reaches `EventBroker`.
        let emitted_events = capability
            .contract
            .emits
            .iter()
            .map(|declared| TraverseEvent {
                id: format!("{}-event", capability.contract.id),
                source: format!("traverse-runtime/{}", capability.contract.id),
                event_type: declared.event_id.clone(),
                datacontenttype: "application/json".to_string(),
                time: "2026-08-06T00:00:00Z".to_string(),
                data: json!({}),
                owner: capability.contract.id.clone(),
                version: declared.version.clone(),
                lifecycle_status: LifecycleStatus::Active,
                deduplication_id: Some(format!("{}-event", capability.contract.id)),
                ordering_scope: Some(capability.contract.id.clone()),
                correlation_id: None,
                causation_id: None,
                subject_id: None,
                actor_id: None,
            })
            .collect();
        Ok(LocalExecutionOutput {
            value: json!({ "draft_id": "draft-live-001" }),
            emitted_events,
        })
    }
}

struct FailingLiveExecutor;

impl LocalExecutor for FailingLiveExecutor {
    fn execute(
        &self,
        _capability: &traverse_registry::ResolvedCapability,
        _input: &Value,
    ) -> Result<LocalExecutionOutput, LocalExecutionFailure> {
        Err(LocalExecutionFailure {
            code: LocalExecutionFailureCode::ExecutionFailed,
            message: "live executor failed".to_string(),
        })
    }
}

fn broker_with_event(event_type: &str) -> Arc<InProcessBroker> {
    let catalog = Arc::new(EventCatalog::new());
    catalog
        .register(EventCatalogEntry {
            event_type: event_type.to_string(),
            owner: "live.wiring".to_string(),
            version: "1.0.0".to_string(),
            lifecycle_status: LifecycleStatus::Active,
            consumer_count: 0,
        })
        .expect("catalog registration should succeed");
    Arc::new(InProcessBroker::new(catalog).expect("default broker config is valid"))
}

fn registry_with(registration: CapabilityRegistration) -> CapabilityRegistry {
    let mut registry = CapabilityRegistry::new();
    registry
        .register(registration)
        .expect("test registration should succeed");
    registry
}

fn native_registration(event_id: &str) -> CapabilityRegistration {
    // Registry currently requires a wasm binary reference for Executable
    // capabilities; the live native path is exercised by a host LocalExecutor
    // bridged into PlacementRouter (BoundLocalExecutor).
    base_registration(
        "live.wiring",
        "native-subject",
        event_id,
        ServiceType::Subscribable,
    )
}

fn base_registration(
    namespace: &str,
    name: &str,
    event_id: &str,
    service_type: ServiceType,
) -> CapabilityRegistration {
    let id = format!("{namespace}.{name}");
    CapabilityRegistration {
        scope: RegistryScope::Private,
        contract: {
            let mut contract = base_contract(&id, namespace, name, service_type);
            contract.emits = vec![EventReference {
                event_id: event_id.to_string(),
                version: "1.0.0".to_string(),
            }];
            contract.event_trigger = Some("dev.traverse.live.trigger".to_string());
            contract
        },
        contract_path: format!("registry/{id}/1.0.0/contract.json"),
        artifact: CapabilityArtifactRecord {
            artifact_ref: format!("artifact:{id}:1.0.0"),
            implementation_kind: ImplementationKind::Executable,
            source: SourceReference {
                kind: SourceKind::Local,
                location: "tests".to_string(),
            },
            binary: Some(BinaryReference {
                format: BinaryFormat::Wasm,
                location: format!("artifacts/{id}/1.0.0/capability.wasm"),
                signature: None,
            }),
            workflow_ref: None,
            digests: ArtifactDigests {
                source_digest: "source".to_string(),
                binary_digest: Some("sha256:deadbeef".to_string()),
            },
            provenance: RegistryProvenance {
                source: "test".to_string(),
                author: "test".to_string(),
                created_at: "2026-08-06T00:00:00Z".to_string(),
            },
        },
        registered_at: "2026-08-06T00:00:00Z".to_string(),
        tags: Vec::new(),
        composability: ComposabilityMetadata {
            kind: CompositionKind::Atomic,
            patterns: vec![CompositionPattern::Sequential],
            provides: Vec::new(),
            requires: Vec::new(),
        },
        governing_spec: "210-runtime-placement-router".to_string(),
        validator_version: "0.1.0".to_string(),
    }
}

fn base_contract(
    id: &str,
    namespace: &str,
    name: &str,
    service_type: ServiceType,
) -> traverse_contracts::CapabilityContract {
    traverse_contracts::CapabilityContract {
        kind: "capability_contract".to_string(),
        schema_version: "1.0.0".to_string(),
        id: id.to_string(),
        namespace: namespace.to_string(),
        name: name.to_string(),
        version: "1.0.0".to_string(),
        lifecycle: Lifecycle::Active,
        owner: Owner {
            team: "traverse-core".to_string(),
            contact: "test@example.com".to_string(),
        },
        summary: "Live PlacementRouter wiring subject.".to_string(),
        description: "Used only by live wiring tests.".to_string(),
        inputs: SchemaContainer {
            schema: json!({
                "type": "object",
                "required": ["comment_text", "resource_id"],
                "properties": {
                    "comment_text": {"type": "string"},
                    "resource_id": {"type": "string"}
                }
            }),
        },
        outputs: SchemaContainer {
            schema: json!({
                "type": "object",
                "required": ["draft_id"],
                "properties": {
                    "draft_id": {"type": "string"},
                    "emitted_events": {"type": "array"}
                }
            }),
        },
        preconditions: vec![Condition {
            id: "always".to_string(),
            description: "always".to_string(),
        }],
        postconditions: vec![Condition {
            id: "always".to_string(),
            description: "always".to_string(),
        }],
        side_effects: vec![SideEffect {
            kind: SideEffectKind::MemoryOnly,
            description: "memory only".to_string(),
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
        },
        policies: Vec::new(),
        dependencies: Vec::new(),
        provenance: Provenance {
            source: ProvenanceSource::Greenfield,
            author: "test".to_string(),
            created_at: "2026-08-06T00:00:00Z".to_string(),
            spec_ref: Some("210-runtime-placement-router".to_string()),
            adr_refs: Vec::new(),
            exception_refs: Vec::new(),
        },
        evidence: Vec::new(),
        service_type,
        permitted_targets: vec![ExecutionTarget::Local, ExecutionTarget::Cloud],
        event_trigger: None,
        connector_requirements: Vec::new(),
        state_schema: None,
        use_cases: Vec::new(),
        risk: traverse_contracts::default_risk_metadata(),
    }
}

fn exact_request(capability_id: &str) -> RuntimeRequest {
    RuntimeRequest {
        kind: "runtime_request".to_string(),
        schema_version: "1.0.0".to_string(),
        request_id: format!("req-{capability_id}"),
        intent: traverse_runtime::RuntimeIntent {
            capability_id: Some(capability_id.to_string()),
            capability_version: Some("1.0.0".to_string()),
            version_range: None,
            intent_key: None,
        },
        input: json!({
            "comment_text": "hello",
            "resource_id": "resource-1"
        }),
        lookup: RuntimeLookup {
            scope: RuntimeLookupScope::PreferPrivate,
            allow_ambiguity: false,
        },
        context: RuntimeContext {
            requested_target: traverse_runtime::PlacementTarget::Local,
            correlation_id: Some("corr-live".to_string()),
            caller: None,
            traceparent: None,
            tracestate: None,
            metadata: None,
            identity: None,
        },
        governing_spec: "006-runtime-request-execution".to_string(),
    }
}

#[test]
fn live_native_execution_publishes_declared_events_and_writes_trace() {
    // `Runtime::execute`'s live path bridges the host-provided `LocalExecutor`
    // through `BoundLocalExecutor` into `PlacementRouter`. Spec
    // 101-local-executor-event-emission FR-002 threads a native
    // `LocalExecutor`'s real `emitted_events` into `ExecutorOutput`, so
    // `PlacementRouter` Step 5 publishes them for `Subscribable` capabilities
    // exactly as it already does for the WASM `CapabilityExecutor` path.
    // This replaces the old test documenting that gap as expected behavior.
    let event_type = "dev.traverse.live.draft-created";
    let broker = broker_with_event(event_type);
    let subscription = broker
        .subscribe(event_type, "0")
        .expect("subscribe should succeed");
    let sink = Arc::new(RecordingEventSink::default());
    let trace_store = Arc::new(Mutex::new(TraceStore::new()));

    let runtime = Runtime::new(
        registry_with(native_registration(event_type)),
        EmittingNativeExecutor,
    )
    .with_security_config(RuntimeSecurityConfig::development())
    .with_event_broker(broker.clone())
    .with_trace_store(Arc::clone(&trace_store))
    .with_event_sink(sink.clone());

    let outcome = runtime.execute(exact_request("live.wiring.native-subject"));
    assert_eq!(outcome.result.status, RuntimeResultStatus::Completed);
    assert_eq!(outcome.trace.emitted_events.len(), 1);
    assert_eq!(outcome.trace.emitted_events[0].event_id, event_type);

    let poll = broker
        .poll(&subscription.subscription_id, 10)
        .expect("poll should succeed");
    assert_eq!(poll.events.len(), 1);
    assert_eq!(poll.events[0].event.event_type, event_type);

    let store = trace_store.lock().expect("trace store lock");
    let entries = store.list_public(Some("live.wiring.native-subject"));
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].id, outcome.result.trace_ref);
    assert_eq!(entries[0].outcome, TraceOutcome::Success);

    let lifecycle = sink.events.lock().expect("sink lock");
    assert_eq!(lifecycle.len(), 1);
    assert_eq!(
        lifecycle[0].event_type,
        "dev.traverse.runtime.execution.completed"
    );
}

#[test]
fn live_placement_failure_surfaces_through_runtime_without_trace_store_entry() {
    let mut registration = native_registration("dev.traverse.live.unused");
    registration.contract.permitted_targets = Vec::new();
    let trace_store = Arc::new(Mutex::new(TraceStore::new()));

    let runtime = Runtime::new(registry_with(registration), EmittingNativeExecutor)
        .with_security_config(RuntimeSecurityConfig::development())
        .with_trace_store(Arc::clone(&trace_store));

    let outcome = runtime.execute(exact_request("live.wiring.native-subject"));
    assert_eq!(outcome.result.status, RuntimeResultStatus::Error);
    assert_eq!(
        outcome.result.error.as_ref().map(|error| error.code),
        Some(RuntimeErrorCode::PlacementUnsupported)
    );
    assert_eq!(
        outcome.trace.execution.failure_reason,
        Some(ExecutionFailureReason::PlacementUnsupported)
    );
    assert!(
        trace_store
            .lock()
            .expect("trace store lock")
            .list_public(None)
            .is_empty()
    );
}

#[test]
fn live_executor_failure_surfaces_through_runtime() {
    let runtime = Runtime::new(
        registry_with(native_registration("dev.traverse.live.unused")),
        FailingLiveExecutor,
    )
    .with_security_config(RuntimeSecurityConfig::development());

    let outcome = runtime.execute(exact_request("live.wiring.native-subject"));
    assert_eq!(outcome.result.status, RuntimeResultStatus::Error);
    assert_eq!(
        outcome.result.error.as_ref().map(|error| error.code),
        Some(RuntimeErrorCode::ExecutionFailed)
    );
    assert_eq!(
        outcome.trace.execution.failure_reason,
        Some(ExecutionFailureReason::ExecutionFailed)
    );
}

#[test]
fn live_runtime_lifecycle_event_still_fires_on_success_and_failure() {
    let sink = Arc::new(RecordingEventSink::default());
    let runtime = Runtime::new(
        registry_with(native_registration("dev.traverse.live.draft-created")),
        EmittingNativeExecutor,
    )
    .with_security_config(RuntimeSecurityConfig::development())
    .with_event_broker(broker_with_event("dev.traverse.live.draft-created"))
    .with_event_sink(sink.clone());

    let success = runtime.execute(exact_request("live.wiring.native-subject"));
    assert_eq!(success.result.status, RuntimeResultStatus::Completed);

    let failing = Runtime::new(
        registry_with(native_registration("dev.traverse.live.unused")),
        FailingLiveExecutor,
    )
    .with_security_config(RuntimeSecurityConfig::development())
    .with_event_sink(sink.clone());
    let failure = failing.execute(exact_request("live.wiring.native-subject"));
    assert_eq!(failure.result.status, RuntimeResultStatus::Error);

    let lifecycle = sink.events.lock().expect("sink lock");
    assert_eq!(lifecycle.len(), 2);
    assert!(
        lifecycle
            .iter()
            .all(|event| { event.event_type == "dev.traverse.runtime.execution.completed" })
    );
    assert!(
        !success
            .state_events
            .iter()
            .any(|event| event.state == RuntimeState::Error)
    );
}
