//! Production host driver for `runtime.wasm` (spec `1402` FR-003, Decision
//! 89): loads a real `runtime-wasm-bridge/1.0.0` artifact (spec `071`
//! FR-006) via Wasmtime, drives its `init`/`submit`/`next_event`/`shutdown`
//! lifecycle, and publishes the domain events it hands back to a real
//! [`EventBroker`] — the same architecture already used for native
//! Wasmtime-hosted capability execution (`crate::router::PlacementRouter`
//! publishes only after guest execution returns, never from inside the
//! guest). `runtime.wasm` itself only collects and hands off events; this
//! module is what actually publishes them.

use std::sync::Arc;

use chrono::Utc;
use serde_json::Value;
use uuid::Uuid;
use wasmtime::{Engine, Instance, Memory, Module, Store, TypedFunc};

use crate::events::types::{EventBroker, EventError, LifecycleStatus, TraverseEvent};
use traverse_contracts::{EventReference, ExecutionTarget, ServiceType};

/// Lifecycle bookkeeping event types `runtime.wasm` emits alongside domain
/// events (spec 071's `$out`-shaped response convention) — not themselves
/// published to `EventBroker`.
const LIFECYCLE_EVENT_TYPES: [&str; 2] = ["capability_invoked", "capability_result"];

/// A failure driving `runtime.wasm`'s ABI. Carries a stable, secret-free
/// message — never a guest panic or trap detail beyond Wasmtime's own error
/// text.
#[derive(Debug)]
pub struct RuntimeWasmHostError(pub String);

impl std::fmt::Display for RuntimeWasmHostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "runtime.wasm host error: {}", self.0)
    }
}

impl std::error::Error for RuntimeWasmHostError {}

fn err(message: impl Into<String>) -> RuntimeWasmHostError {
    RuntimeWasmHostError(message.into())
}

fn service_type_str(service_type: &ServiceType) -> &'static str {
    match service_type {
        ServiceType::Subscribable => "subscribable",
        ServiceType::Stateful => "stateful",
        ServiceType::Stateless => "stateless",
    }
}

/// Matches `traverse-runtime-wasm`'s `parse_execution_target` wire strings
/// exactly (spec `1402` FR-003/FR-004, Decision 88's shared placement core).
fn execution_target_str(target: &ExecutionTarget) -> &'static str {
    match target {
        ExecutionTarget::Local => "local",
        ExecutionTarget::Browser => "browser",
        ExecutionTarget::Edge => "edge",
        ExecutionTarget::Cloud => "cloud",
        ExecutionTarget::Worker => "worker",
        ExecutionTarget::Device => "device",
    }
}

/// Metadata `RuntimeWasmHost::init` sends to the guest for one capability.
///
/// `host_placement_target` declares the [`ExecutionTarget`] this
/// `runtime.wasm` instance itself runs at — nested execution only proceeds
/// when the guest's shared `PlacementConstraintEvaluator` (spec `1402`
/// FR-003/FR-004, Decision 88) selects this same target. `permitted_targets`
/// restricts which targets that evaluator may choose from; passing just
/// `host_placement_target` removes any ambiguity the evaluator would
/// otherwise have to resolve.
#[derive(Clone, Copy)]
pub struct CapabilityInit<'a> {
    pub capability_id: &'a str,
    pub capability_version: &'a str,
    pub service_type: &'a ServiceType,
    pub declared_emits: &'a [EventReference],
    pub host_placement_target: &'a ExecutionTarget,
    pub permitted_targets: &'a [ExecutionTarget],
}

/// Drives one `runtime.wasm` instance's `runtime-wasm-bridge/1.0.0` ABI
/// (spec 071 FR-006). One instance corresponds to one guest module
/// instantiation — `init` MUST be called before `submit`.
pub struct RuntimeWasmHost {
    store: Store<()>,
    memory: Memory,
    alloc: TypedFunc<i32, i32>,
    dealloc: TypedFunc<(i32, i32), ()>,
    init_fn: TypedFunc<(i32, i32, i32), i32>,
    submit_fn: TypedFunc<(i32, i32, i32), i32>,
    next_event_fn: TypedFunc<i32, i32>,
    shutdown_fn: TypedFunc<i32, i32>,
}

impl RuntimeWasmHost {
    /// Instantiates `runtime_wasm_bytes` as a `runtime-wasm-bridge/1.0.0`
    /// module and resolves its required exports.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeWasmHostError`] if the bytes are not a valid module,
    /// instantiation fails, or a required export is missing or has the
    /// wrong signature.
    pub fn instantiate(runtime_wasm_bytes: &[u8]) -> Result<Self, RuntimeWasmHostError> {
        let engine = Engine::default();
        let module = Module::new(&engine, runtime_wasm_bytes)
            .map_err(|error| err(format!("module: {error}")))?;
        let mut store = Store::new(&engine, ());
        let instance = Instance::new(&mut store, &module, &[])
            .map_err(|error| err(format!("instantiate: {error}")))?;

        let memory = instance
            .get_memory(&mut store, "memory")
            .ok_or_else(|| err("missing export: memory"))?;
        let alloc = instance
            .get_typed_func::<i32, i32>(&mut store, "traverse_alloc")
            .map_err(|error| err(format!("traverse_alloc: {error}")))?;
        let dealloc = instance
            .get_typed_func::<(i32, i32), ()>(&mut store, "traverse_dealloc")
            .map_err(|error| err(format!("traverse_dealloc: {error}")))?;
        let init_fn = instance
            .get_typed_func::<(i32, i32, i32), i32>(&mut store, "traverse_init")
            .map_err(|error| err(format!("traverse_init: {error}")))?;
        let submit_fn = instance
            .get_typed_func::<(i32, i32, i32), i32>(&mut store, "traverse_submit")
            .map_err(|error| err(format!("traverse_submit: {error}")))?;
        let next_event_fn = instance
            .get_typed_func::<i32, i32>(&mut store, "traverse_next_event")
            .map_err(|error| err(format!("traverse_next_event: {error}")))?;
        let shutdown_fn = instance
            .get_typed_func::<i32, i32>(&mut store, "traverse_shutdown")
            .map_err(|error| err(format!("traverse_shutdown: {error}")))?;

        Ok(Self {
            store,
            memory,
            alloc,
            dealloc,
            init_fn,
            submit_fn,
            next_event_fn,
            shutdown_fn,
        })
    }

    /// Writes `bytes` into a freshly guest-allocated region and returns its
    /// pointer. The caller is responsible for eventually freeing it via
    /// [`Self::dealloc`].
    fn write_bytes(&mut self, bytes: &[u8]) -> Result<i32, RuntimeWasmHostError> {
        let len = i32::try_from(bytes.len())
            .map_err(|_| err("payload too large for the wasm32 ABI (max i32::MAX bytes)"))?;
        let ptr = self
            .alloc
            .call(&mut self.store, len)
            .map_err(|error| err(format!("traverse_alloc: {error}")))?;
        self.memory
            .write(&mut self.store, usize::try_from(ptr).unwrap_or(0), bytes)
            .map_err(|error| err(format!("memory write: {error}")))?;
        Ok(ptr)
    }

    fn dealloc(&mut self, ptr: i32, len: i32) {
        // Best-effort: a failed free leaks guest memory for this instance's
        // lifetime but never corrupts state or fails the caller's request.
        let _ = self.dealloc.call(&mut self.store, (ptr, len));
    }

    /// Reads the 8-byte `(ptr, len)` descriptor at `descriptor_ptr` and
    /// returns the bytes it points to, then frees both the descriptor slot
    /// and the response buffer it described.
    fn read_descriptor(&mut self, descriptor_ptr: i32) -> Result<Vec<u8>, RuntimeWasmHostError> {
        let descriptor_addr = usize::try_from(descriptor_ptr).unwrap_or(0);
        let mut header = [0u8; 8];
        self.memory
            .read(&self.store, descriptor_addr, &mut header)
            .map_err(|error| err(format!("descriptor read: {error}")))?;
        let response_ptr = i32::from_le_bytes([header[0], header[1], header[2], header[3]]);
        let response_len = i32::from_le_bytes([header[4], header[5], header[6], header[7]]);

        let response = if response_len > 0 && response_ptr >= 0 {
            let mut buffer = vec![0u8; usize::try_from(response_len).unwrap_or(0)];
            self.memory
                .read(
                    &self.store,
                    usize::try_from(response_ptr).unwrap_or(0),
                    &mut buffer,
                )
                .map_err(|error| err(format!("response read: {error}")))?;
            buffer
        } else {
            Vec::new()
        };

        self.dealloc(descriptor_ptr, 8);
        if response_ptr > 0 {
            self.dealloc(response_ptr, response_len);
        }
        Ok(response)
    }

    /// Writes `payload`, calls `target` with `(payload_ptr, payload_len,
    /// out_descriptor_ptr)`, and returns `(status, response_bytes)`.
    fn call_json(
        &mut self,
        target: &TypedFunc<(i32, i32, i32), i32>,
        payload: &[u8],
    ) -> Result<(i32, Vec<u8>), RuntimeWasmHostError> {
        let out_descriptor = self
            .alloc
            .call(&mut self.store, 8)
            .map_err(|error| err(format!("traverse_alloc (descriptor): {error}")))?;
        let payload_ptr = self.write_bytes(payload)?;
        let payload_len = i32::try_from(payload.len()).unwrap_or(i32::MAX);

        let status = target
            .call(&mut self.store, (payload_ptr, payload_len, out_descriptor))
            .map_err(|error| err(format!("call: {error}")))?;
        self.dealloc(payload_ptr, payload_len);

        let response = self.read_descriptor(out_descriptor)?;
        Ok((status, response))
    }

    /// Calls `traverse_init` with `capability`'s metadata and
    /// `capability_wasm`, using `crates/traverse-runtime-wasm`'s documented
    /// init payload layout: a 4-byte little-endian header length, that many
    /// bytes of JSON metadata, then the raw capability artifact.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeWasmHostError`] if the guest rejects the payload or
    /// any ABI call fails.
    pub fn init(
        &mut self,
        capability: CapabilityInit<'_>,
        capability_wasm: &[u8],
    ) -> Result<Value, RuntimeWasmHostError> {
        let CapabilityInit {
            capability_id,
            capability_version,
            service_type,
            declared_emits,
            host_placement_target,
            permitted_targets,
        } = capability;
        let header = serde_json::json!({
            "capability_id": capability_id,
            "capability_version": capability_version,
            "service_type": service_type_str(service_type),
            "emits": declared_emits
                .iter()
                .map(|reference| serde_json::json!({
                    "event_id": reference.event_id,
                    "version": reference.version,
                }))
                .collect::<Vec<_>>(),
            "host_placement_target": execution_target_str(host_placement_target),
            "permitted_targets": permitted_targets
                .iter()
                .map(execution_target_str)
                .collect::<Vec<_>>(),
        });
        let header_bytes =
            serde_json::to_vec(&header).map_err(|error| err(format!("encode header: {error}")))?;
        let header_len = u32::try_from(header_bytes.len())
            .map_err(|_| err("init header too large for the wasm32 ABI"))?;

        let mut payload = header_len.to_le_bytes().to_vec();
        payload.extend_from_slice(&header_bytes);
        payload.extend_from_slice(capability_wasm);

        let (status, response) = self.call_json(&self.init_fn.clone(), &payload)?;
        let parsed = parse_response(&response)?;
        if status != 0 {
            return Err(err(format!("traverse_init rejected: {parsed}")));
        }
        Ok(parsed)
    }

    /// Calls `traverse_submit` with the raw runtime-request bytes fed to
    /// the nested capability's stdin.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeWasmHostError`] if the guest rejects the request or
    /// any ABI call fails.
    pub fn submit(&mut self, request: &[u8]) -> Result<Value, RuntimeWasmHostError> {
        let (status, response) = self.call_json(&self.submit_fn.clone(), request)?;
        let parsed = parse_response(&response)?;
        if status != 0 {
            return Err(err(format!("traverse_submit rejected: {parsed}")));
        }
        Ok(parsed)
    }

    /// Drains every event `traverse_next_event` currently has queued,
    /// stopping at the first `0` (no more events).
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeWasmHostError`] if the ABI call or a response read
    /// fails.
    pub fn drain_events(&mut self) -> Result<Vec<Value>, RuntimeWasmHostError> {
        let mut events = Vec::new();
        loop {
            let out_descriptor = self
                .alloc
                .call(&mut self.store, 8)
                .map_err(|error| err(format!("traverse_alloc (descriptor): {error}")))?;
            let has_event = self
                .next_event_fn
                .call(&mut self.store, out_descriptor)
                .map_err(|error| err(format!("traverse_next_event: {error}")))?;
            if has_event == 0 {
                self.dealloc(out_descriptor, 8);
                break;
            }
            let response = self.read_descriptor(out_descriptor)?;
            events.push(parse_response(&response)?);
        }
        Ok(events)
    }

    /// Calls `traverse_shutdown`, clearing the guest's session state.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeWasmHostError`] if the ABI call or a response read
    /// fails.
    pub fn shutdown(&mut self) -> Result<Value, RuntimeWasmHostError> {
        let out_descriptor = self
            .alloc
            .call(&mut self.store, 8)
            .map_err(|error| err(format!("traverse_alloc (descriptor): {error}")))?;
        self.shutdown_fn
            .call(&mut self.store, out_descriptor)
            .map_err(|error| err(format!("traverse_shutdown: {error}")))?;
        let response = self.read_descriptor(out_descriptor)?;
        parse_response(&response)
    }
}

fn parse_response(bytes: &[u8]) -> Result<Value, RuntimeWasmHostError> {
    if bytes.is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_slice(bytes).map_err(|error| err(format!("decode response: {error}")))
}

/// Publishes every non-lifecycle event in `events` to `broker`, constructing
/// a [`TraverseEvent`] from each `runtime.wasm` domain-event envelope
/// (`{"type", "session_id", "data": {"version", "payload"}}`, per
/// `crates/traverse-runtime-wasm/src/lib.rs`'s `lifecycle_event` shape).
/// Returns the number of events actually published.
///
/// # Errors
///
/// Returns [`RuntimeWasmHostError`] if a domain event is malformed, or the
/// first [`EventError`] a publish call returns.
pub fn publish_domain_events(
    events: &[Value],
    capability_id: &str,
    broker: &Arc<dyn EventBroker>,
) -> Result<usize, RuntimeWasmHostError> {
    let mut published = 0usize;
    for envelope in events {
        let Some(event_type) = envelope.get("type").and_then(Value::as_str) else {
            return Err(err("event envelope missing \"type\""));
        };
        if LIFECYCLE_EVENT_TYPES.contains(&event_type) {
            continue;
        }
        let data = envelope.get("data").cloned().unwrap_or(Value::Null);
        let version = data
            .get("version")
            .and_then(Value::as_str)
            .unwrap_or("0.0.0")
            .to_string();
        let payload = data.get("payload").cloned().unwrap_or(Value::Null);

        let event = TraverseEvent {
            id: Uuid::new_v4().to_string(),
            source: format!("traverse-runtime-wasm/{capability_id}"),
            event_type: event_type.to_string(),
            datacontenttype: "application/json".to_string(),
            time: Utc::now().to_rfc3339(),
            data: payload,
            owner: capability_id.to_string(),
            version: version.clone(),
            lifecycle_status: LifecycleStatus::Active,
            deduplication_id: Some(format!("{capability_id}:{event_type}:{version}")),
            ordering_scope: Some(capability_id.to_string()),
            correlation_id: None,
            causation_id: None,
            subject_id: None,
            actor_id: None,
        };
        broker
            .publish(event)
            .map_err(|error: EventError| err(format!("publish: {error}")))?;
        published += 1;
    }
    Ok(published)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]

    use super::*;
    use crate::events::broker::InProcessBroker;
    use crate::events::catalog::{EventCatalog, EventCatalogEntry};

    /// Minimal hand-authored fixture whose `traverse_init`/`traverse_submit`
    /// always reject (status `-1`), used to exercise the rejection paths
    /// `RuntimeWasmHost::init`/`submit` take when the guest refuses a call —
    /// paths the real `traverse-runtime-wasm` guest (exercised end-to-end in
    /// `tests/runtime_wasm_host_tests.rs`) never takes for a well-formed
    /// request, since its `init()`/`submit()` callers always send one.
    const REJECTING_FIXTURE_WAT: &str = r#"
      (module
        (memory (export "memory") 1)
        (global $heap (mut i32) (i32.const 4096))
        (data (i32.const 8192) "{\"status\":\"error\",\"error\":\"rejected\"}")
        (func $out (param $d i32) (param $p i32) (param $n i32)
          local.get $d local.get $p i32.store
          local.get $d i32.const 4 i32.add local.get $n i32.store)
        (func (export "traverse_alloc") (param $n i32) (result i32)
          (local $p i32) global.get $heap local.set $p
          global.get $heap local.get $n i32.add global.set $heap local.get $p)
        (func (export "traverse_dealloc") (param i32 i32))
        (func (export "traverse_init") (param i32 i32 i32) (result i32)
          local.get 2 i32.const 8192 i32.const 37 call $out i32.const -1)
        (func (export "traverse_submit") (param i32 i32 i32) (result i32)
          local.get 2 i32.const 8192 i32.const 37 call $out i32.const -1)
        (func (export "traverse_next_event") (param i32) (result i32) i32.const 0)
        (func (export "traverse_shutdown") (param i32) (result i32) i32.const 0))
    "#;

    /// Minimal fixture whose `traverse_shutdown` writes a zero-length
    /// response descriptor, exercising `read_descriptor`'s empty-response
    /// branch and `parse_response`'s empty-bytes branch — a shape the real
    /// guest never produces (every one of its responses is a non-empty JSON
    /// object) but that a differently-implemented `runtime-wasm-bridge/1.0.0`
    /// guest legitimately could.
    const EMPTY_RESPONSE_FIXTURE_WAT: &str = r#"
      (module
        (memory (export "memory") 1)
        (func (export "traverse_alloc") (param i32) (result i32) i32.const 4096)
        (func (export "traverse_dealloc") (param i32 i32))
        (func (export "traverse_init") (param i32 i32 i32) (result i32) i32.const 0)
        (func (export "traverse_submit") (param i32 i32 i32) (result i32) i32.const 0)
        (func (export "traverse_next_event") (param i32) (result i32) i32.const 0)
        (func (export "traverse_shutdown") (param $d i32) (result i32)
          local.get $d i32.const 0 i32.store
          local.get $d i32.const 4 i32.add i32.const 0 i32.store
          i32.const 0))
    "#;

    fn broker_with_event(event_type: &str) -> Arc<dyn EventBroker> {
        let catalog = Arc::new(EventCatalog::new());
        catalog
            .register(EventCatalogEntry {
                event_type: event_type.to_string(),
                owner: "runtime_wasm_host.tests".to_string(),
                version: "1.0.0".to_string(),
                lifecycle_status: LifecycleStatus::Active,
                consumer_count: 0,
            })
            .expect("register event type");
        Arc::new(InProcessBroker::new(catalog).expect("construct broker"))
    }

    #[test]
    fn service_type_str_covers_every_variant() {
        assert_eq!(service_type_str(&ServiceType::Subscribable), "subscribable");
        assert_eq!(service_type_str(&ServiceType::Stateful), "stateful");
        assert_eq!(service_type_str(&ServiceType::Stateless), "stateless");
    }

    #[test]
    fn execution_target_str_covers_every_variant() {
        assert_eq!(execution_target_str(&ExecutionTarget::Local), "local");
        assert_eq!(execution_target_str(&ExecutionTarget::Browser), "browser");
        assert_eq!(execution_target_str(&ExecutionTarget::Edge), "edge");
        assert_eq!(execution_target_str(&ExecutionTarget::Cloud), "cloud");
        assert_eq!(execution_target_str(&ExecutionTarget::Worker), "worker");
        assert_eq!(execution_target_str(&ExecutionTarget::Device), "device");
    }

    #[test]
    fn runtime_wasm_host_error_displays_a_stable_prefixed_message() {
        let error = err("something went wrong");
        assert_eq!(
            error.to_string(),
            "runtime.wasm host error: something went wrong"
        );
    }

    #[test]
    fn instantiate_rejects_bytes_that_are_not_a_wasm_module() {
        let result = RuntimeWasmHost::instantiate(b"not a wasm module");
        assert!(result.is_err());
    }

    #[test]
    fn parse_response_treats_empty_bytes_as_null() {
        assert_eq!(parse_response(&[]).expect("must parse"), Value::Null);
    }

    #[test]
    fn parse_response_rejects_malformed_json() {
        assert!(parse_response(b"{not json").is_err());
    }

    #[test]
    fn init_surfaces_a_guest_rejection() {
        let artifact = wat::parse_str(REJECTING_FIXTURE_WAT).expect("wat parses");
        let mut host = RuntimeWasmHost::instantiate(&artifact).expect("instantiate");
        let result = host.init(
            CapabilityInit {
                capability_id: "example.rejecting",
                capability_version: "1.0.0",
                service_type: &ServiceType::Subscribable,
                declared_emits: &[],
                host_placement_target: &ExecutionTarget::Local,
                permitted_targets: &[ExecutionTarget::Local],
            },
            b"",
        );
        let error = result.expect_err("init must be rejected");
        assert!(error.0.contains("traverse_init rejected"));
    }

    #[test]
    fn submit_surfaces_a_guest_rejection() {
        let artifact = wat::parse_str(REJECTING_FIXTURE_WAT).expect("wat parses");
        let mut host = RuntimeWasmHost::instantiate(&artifact).expect("instantiate");
        let result = host.submit(b"{}");
        let error = result.expect_err("submit must be rejected");
        assert!(error.0.contains("traverse_submit rejected"));
    }

    #[test]
    fn shutdown_handles_a_zero_length_response() {
        let artifact = wat::parse_str(EMPTY_RESPONSE_FIXTURE_WAT).expect("wat parses");
        let mut host = RuntimeWasmHost::instantiate(&artifact).expect("instantiate");
        let response = host.shutdown().expect("shutdown succeeds");
        assert_eq!(response, Value::Null);
    }

    #[test]
    fn publish_domain_events_rejects_an_envelope_missing_type() {
        let broker = broker_with_event("host.test.event");
        let result = publish_domain_events(
            &[serde_json::json!({"no_type_field": true})],
            "cap",
            &broker,
        );
        let error = result.expect_err("missing type must error");
        assert!(error.0.contains("missing \"type\""));
    }

    #[test]
    fn publish_domain_events_skips_lifecycle_events_and_defaults_missing_fields() {
        let broker = broker_with_event("host.test.event");
        let events = vec![
            serde_json::json!({"type": "capability_invoked", "data": {}}),
            serde_json::json!({"type": "host.test.event"}),
        ];
        let published = publish_domain_events(&events, "cap", &broker).expect("publish");
        assert_eq!(published, 1, "only the domain event should publish");
    }
}
