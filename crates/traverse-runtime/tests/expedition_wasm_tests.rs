//! End-to-end integration test: Expedition WASM port.
//!
//! Governed by spec 027-expedition-wasm-port
//!
//! Exercises the full `PlacementRouter` → `WasmExecutor` path using a WAT-based
//! expedition stub that honours the expedition JSON I/O contract.  The WAT stub
//! is compiled in-process via the `wat` crate so the test does not depend on a
//! pre-built `.wasm` binary on disk.

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use traverse_contracts::{
    BinaryFormat, CapabilityContract, Condition, Entrypoint, EntrypointKind, Execution,
    ExecutionConstraints, ExecutionTarget, FilesystemAccess, HostApiAccess, Lifecycle,
    NetworkAccess, Owner, Provenance, ProvenanceSource, SchemaContainer, ServiceType, SideEffect,
    SideEffectKind,
};
use traverse_runtime::{
    events::{EventBroker, EventCatalog, EventCatalogEntry, InProcessBroker, LifecycleStatus},
    executor::{ArtifactType, ExecutorCapability, WasmExecutor},
    placement::{PlacementConstraintEvaluator, RuntimeSnapshot},
    router::{CapabilityExecutorRegistry, PlacementRouter, RouterRequest},
    trace::TraceStore,
};

// ---------------------------------------------------------------------------
// WAT stub: mimics expedition I/O contract
// ---------------------------------------------------------------------------
//
// The stub reads JSON from stdin, then writes a hard-coded valid expedition
// plan JSON to stdout.  It does not parse the input — its sole purpose is to
// exercise the `WasmExecutor` → `PlacementRouter` plumbing.
//
// Memory layout (all offsets are byte addresses inside the 64 KiB page):
//   0x0000 – 0x0007  : iovec for fd_read  (ptr=0x200, len=8192)
//   0x0008 – 0x000F  : iovec for fd_write (ptr=<output>, len=<n>)
//   0x0010 – 0x0013  : nread scratch (4 bytes, filled by fd_read)
//   0x0200 – 0x21FF  : 8192-byte read buffer (discarded)
//   0x2200 – …       : static output JSON

const EXPEDITION_WAT: &str = r#"
(module
  (import "wasi_snapshot_preview1" "fd_read"
    (func $fd_read (param i32 i32 i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "fd_write"
    (func $fd_write (param i32 i32 i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "proc_exit"
    (func $proc_exit (param i32)))

  ;; One 64 KiB page is enough for the read buffer + static output.
  (memory (export "memory") 2)

  ;; Static expedition plan JSON written at offset 0x2200 (8704 decimal).
  ;; Must be a valid JSON object that satisfies the expedition output schema.
  (data (i32.const 8704)
    "{\"plan_id\":\"plan-alpine-peak-t4\",\"objective_id\":\"obj-summit-attempt\",\"status\":\"ready\",\"recommended_route_style\":\"alpine-traverse\",\"key_steps\":[\"Define objective\",\"Assess destination\",\"Assemble team\",\"Evaluate conditions\",\"Validate readiness\",\"Assemble plan\"],\"constraints\":[\"No host API access\",\"No network access\",\"No filesystem access\"],\"readiness_notes\":[\"Equipment checklist reviewed\",\"Team size meets minimum threshold\",\"Destination briefing completed\"],\"summary\":\"Expedition to Alpine Peak for summit attempt with 4 team members via alpine-traverse route.\"}"
  )

  (func $_start (export "_start")
    ;; --- Drain stdin so WASI is happy ---
    ;; iovec[0]: ptr = 0x0200 (512), len = 8192
    (i32.store (i32.const 0)   (i32.const 512))
    (i32.store (i32.const 4)   (i32.const 8192))
    ;; fd_read(stdin=0, iovecs=0x0000, n_iovecs=1, nread_out=0x0010)
    (drop (call $fd_read (i32.const 0) (i32.const 0) (i32.const 1) (i32.const 16)))

    ;; --- Write static expedition JSON to stdout ---
    ;; iovec[1] at 0x0008: ptr = 8704, len = 462
    (i32.store (i32.const 8)   (i32.const 8704))
    (i32.store (i32.const 12)  (i32.const 563))
    ;; fd_write(stdout=1, iovecs=0x0008, n_iovecs=1, nwritten_out=0x0014)
    (drop (call $fd_write (i32.const 1) (i32.const 8) (i32.const 1) (i32.const 20)))
  )
)
"#;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn expedition_contract() -> CapabilityContract {
    CapabilityContract {
        kind: "capability_contract".to_string(),
        schema_version: "1.0.0".to_string(),
        id: "expedition.planning.plan-expedition".to_string(),
        namespace: "expedition.planning".to_string(),
        name: "plan-expedition".to_string(),
        version: "1.0.0".to_string(),
        lifecycle: Lifecycle::Active,
        owner: Owner {
            team: "traverse-core".to_string(),
            contact: "test@example.com".to_string(),
        },
        summary: "Expedition planning capability (WASM port).".to_string(),
        description: "Governed by spec 027-expedition-wasm-port.".to_string(),
        inputs: SchemaContainer {
            schema: json!({ "type": "object" }),
        },
        outputs: SchemaContainer {
            schema: json!({ "type": "object" }),
        },
        preconditions: vec![Condition {
            id: "workflow-input-available".to_string(),
            description: "Expedition planning input available.".to_string(),
        }],
        postconditions: vec![Condition {
            id: "workflow-plan-produced".to_string(),
            description: "Expedition plan produced.".to_string(),
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
            preferred_targets: vec![ExecutionTarget::Cloud],
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
            created_at: "2026-04-08T00:00:00Z".to_string(),
            spec_ref: Some("027-expedition-wasm-port@1.0.0".to_string()),
            adr_refs: Vec::new(),
            exception_refs: Vec::new(),
        },
        evidence: Vec::new(),
        // FR-002: service_type = Stateless, permitted_targets = [Cloud, Edge, Device]
        service_type: ServiceType::Stateless,
        permitted_targets: vec![
            ExecutionTarget::Cloud,
            ExecutionTarget::Edge,
            ExecutionTarget::Device,
        ],
        event_trigger: None,
        connector_requirements: Vec::new(),
    }
}

fn idle_cloud_snapshot() -> RuntimeSnapshot {
    RuntimeSnapshot {
        target_loads: [(ExecutionTarget::Cloud, 0.0)].into_iter().collect(),
    }
}

fn make_broker() -> Result<Arc<dyn EventBroker>, String> {
    let catalog = Arc::new(EventCatalog::new());
    catalog
        .register(EventCatalogEntry {
            event_type: "expedition.planning.expedition-plan-assembled".to_string(),
            owner: "expedition.planning".to_string(),
            version: "1.0.0".to_string(),
            lifecycle_status: LifecycleStatus::Active,
            consumer_count: 0,
        })
        .map_err(|e| e.to_string())?;
    Ok(Arc::new(
        InProcessBroker::new(catalog).map_err(|e| e.to_string())?,
    ))
}

/// Write `bytes` to a temp file and return the path.
fn write_temp_wasm(bytes: &[u8]) -> Result<String, String> {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = format!(
        "/tmp/traverse-expedition-stub-{}-{}.wasm",
        std::process::id(),
        n
    );
    std::fs::write(&path, bytes).map_err(|e| format!("write temp wasm: {e}"))?;
    Ok(path)
}

/// Build a WAT stub that writes exactly `json_len` bytes from the static data
/// at offset 8704 to stdout.  The WAT constant must match the actual JSON length.
fn compile_expedition_wat() -> Result<Vec<u8>, String> {
    wat::parse_str(EXPEDITION_WAT).map_err(|e| format!("WAT parse: {e}"))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// FR-005 / SC-002: `PlacementRouter` routes expedition request to `WasmExecutor`
/// and returns a valid plan response.
#[test]
fn placement_router_routes_expedition_to_wasm_executor() -> Result<(), String> {
    let wasm_bytes = compile_expedition_wat()?;
    let tmp_path = write_temp_wasm(&wasm_bytes)?;

    let result = run_expedition_via_router(&wasm_bytes, &tmp_path);
    std::fs::remove_file(&tmp_path).ok();
    let response = result?;

    // Verify key output fields from the WAT stub
    assert_eq!(
        response.output["status"].as_str(),
        Some("ready"),
        "status must be 'ready'"
    );
    assert_eq!(
        response.output["plan_id"].as_str(),
        Some("plan-alpine-peak-t4"),
        "plan_id must match"
    );
    assert_eq!(
        response.output["recommended_route_style"].as_str(),
        Some("alpine-traverse"),
        "route style must match"
    );
    assert!(
        response.output["key_steps"].is_array(),
        "key_steps must be an array"
    );
    assert!(
        response.output["constraints"].is_array(),
        "constraints must be an array"
    );
    assert!(
        response.output["readiness_notes"].is_array(),
        "readiness_notes must be an array"
    );
    assert!(
        !response.output["summary"].as_str().unwrap_or("").is_empty(),
        "summary must be non-empty"
    );

    Ok(())
}

/// Trace is recorded after a successful expedition WASM execution.
#[test]
fn expedition_wasm_execution_writes_trace() -> Result<(), String> {
    let wasm_bytes = compile_expedition_wat()?;
    let tmp_path = write_temp_wasm(&wasm_bytes)?;
    let trace_store = Arc::new(Mutex::new(TraceStore::new()));
    let broker = make_broker()?;

    let mut registry: CapabilityExecutorRegistry = CapabilityExecutorRegistry::new();
    registry.insert(
        ArtifactType::Wasm,
        Box::new(WasmExecutor::new().map_err(|e| format!("{e}"))?),
    );

    let router = PlacementRouter::new(
        PlacementConstraintEvaluator,
        registry,
        Arc::clone(&trace_store),
        broker,
    );

    let request = RouterRequest {
        capability_id: "expedition.planning.plan-expedition".to_string(),
        artifact_type: ArtifactType::Wasm,
        contract: expedition_contract(),
        target_hint: Some(ExecutionTarget::Cloud),
        runtime_snapshot: idle_cloud_snapshot(),
        input: expedition_input(),
        executor_capability: ExecutorCapability {
            capability_id: "expedition.planning.plan-expedition".to_string(),
            artifact_type: ArtifactType::Wasm,
            wasm_binary_path: Some(tmp_path.clone()),
            wasm_checksum: None,
            host_abi_version: None,
        },
        emitted_events: Vec::new(),
    };

    let response = router.execute(request).map_err(|e| format!("{e}"));
    std::fs::remove_file(&tmp_path).ok();
    response?;

    let store = trace_store.lock().map_err(|_| "lock poisoned")?;
    let entries = store.list_public(Some("expedition.planning.plan-expedition"));
    assert_eq!(entries.len(), 1, "exactly one trace entry must be written");
    assert!(
        !entries[0].id.is_empty(),
        "trace entry id must be non-empty"
    );

    Ok(())
}

/// The WAT stub itself (`WasmExecutor.run_bytes`) produces valid JSON without
/// going through the router.
#[test]
fn expedition_wat_stub_produces_valid_json_via_run_bytes() -> Result<(), String> {
    let wasm_bytes = compile_expedition_wat()?;
    let executor = WasmExecutor::new().map_err(|e| format!("{e}"))?;

    let output = executor
        .run_bytes(&wasm_bytes, &expedition_input())
        .map_err(|e| format!("{e:?}"))?;

    // Verify the output is a JSON object with required expedition fields
    assert!(output.is_object(), "output must be a JSON object");
    assert!(output["plan_id"].is_string());
    assert!(output["objective_id"].is_string());
    assert!(output["status"].is_string());
    assert!(output["key_steps"].is_array());

    Ok(())
}

/// The contract used in the integration test declares the correct metadata
/// required by FR-002.
#[test]
fn expedition_contract_has_correct_wasm_metadata() {
    let contract = expedition_contract();
    assert_eq!(contract.service_type, ServiceType::Stateless);
    assert!(
        contract.permitted_targets.contains(&ExecutionTarget::Cloud),
        "Cloud must be permitted"
    );
    assert!(
        contract.permitted_targets.contains(&ExecutionTarget::Edge),
        "Edge must be permitted"
    );
    assert!(
        contract
            .permitted_targets
            .contains(&ExecutionTarget::Device),
        "Device must be permitted"
    );
    assert_eq!(contract.execution.binary_format, BinaryFormat::Wasm);
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

fn expedition_input() -> Value {
    json!({
        "destination": "Alpine Peak",
        "target_window": {
            "start": "2026-07-01T00:00:00Z",
            "end": "2026-07-14T00:00:00Z"
        },
        "preferences": {
            "style": "alpine",
            "risk_tolerance": "moderate",
            "priority": "summit"
        },
        "notes": "Integration test expedition",
        "planning_intent": "summit attempt",
        "team_profile": {
            "team_id": "team-alpha",
            "member_count": 4,
            "experience_level": "advanced",
            "equipment_ready": true
        }
    })
}

struct RouterResponse {
    output: Value,
}

fn run_expedition_via_router(wasm_bytes: &[u8], tmp_path: &str) -> Result<RouterResponse, String> {
    let trace_store = Arc::new(Mutex::new(TraceStore::new()));
    let broker = make_broker()?;

    let mut registry: CapabilityExecutorRegistry = CapabilityExecutorRegistry::new();
    registry.insert(
        ArtifactType::Wasm,
        Box::new(WasmExecutor::new().map_err(|e| format!("{e}"))?),
    );

    let router = PlacementRouter::new(PlacementConstraintEvaluator, registry, trace_store, broker);

    let request = RouterRequest {
        capability_id: "expedition.planning.plan-expedition".to_string(),
        artifact_type: ArtifactType::Wasm,
        contract: expedition_contract(),
        target_hint: Some(ExecutionTarget::Cloud),
        runtime_snapshot: idle_cloud_snapshot(),
        input: expedition_input(),
        executor_capability: ExecutorCapability {
            capability_id: "expedition.planning.plan-expedition".to_string(),
            artifact_type: ArtifactType::Wasm,
            wasm_binary_path: Some(tmp_path.to_string()),
            wasm_checksum: None,
            host_abi_version: None,
        },
        emitted_events: Vec::new(),
    };

    // We need to hold the bytes alive for WAT modules loaded via run_bytes,
    // but since we're using the file path here we just need the bytes to be
    // valid (which they are, coming from compile_expedition_wat).
    let _ = wasm_bytes; // suppress unused warning

    let resp = router.execute(request).map_err(|e| format!("{e}"))?;
    Ok(RouterResponse {
        output: resp.output,
    })
}
