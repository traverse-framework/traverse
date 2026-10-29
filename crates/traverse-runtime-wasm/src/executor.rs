//! Nested-wasmi capability execution (Spec `1402` FR-003/FR-011, Decision 87).
//!
//! This module contains no `unsafe` code — it operates entirely on safe
//! `wasmi` APIs and byte slices. The unavoidable raw-pointer C-ABI boundary
//! (spec `071` FR-006) lives only in `lib.rs`'s exported `extern "C"`
//! functions, which convert guest pointers to slices before calling in here.
//!
//! Mirrors `crates/traverse-runtime/src/executor/wasm.rs`'s Wasmtime-backed
//! `WasmExecutor`: input via WASI `stdin`, output via WASI `stdout`, a
//! `traverse_host::emit_event` import validated by the same shared core
//! (`traverse_contracts::validate_emit_event`) the native executor calls —
//! not a second implementation of that logic (spec `1402` FR-005) — and
//! Spec 104 `traverse_host::connector_invoke` with the same fail-closed
//! mediation order as native Wasmtime (#1719).

use std::sync::Arc;

use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use traverse_contracts::{
    ConnectorRequirement, EventReference, ServiceType, canonicalize_json_bytes, validate_emit_event,
};
use wasmi::{
    Caller, Config, Engine, Error as WasmiError, Extern, Linker, Module, Store, StoreLimitsBuilder,
};

/// Host ABI version accepted on Spec 104 connector request/response envelopes.
/// Must stay aligned with `traverse-runtime`'s `SUPPORTED_HOST_ABI_VERSION`.
pub const SUPPORTED_HOST_ABI_VERSION: &str = "1.0.0";

/// `traverse_host::connector_invoke` is unavailable unless the embedding host
/// supplies an activated, capability-authorized connector binding (Spec 104).
const CONNECTOR_INVOKE_ERR_UNBOUND: i32 = -2;
const CONNECTOR_INVOKE_ERR_INVALID_REQUEST: i32 = -1;
const CONNECTOR_INVOKE_ERR_UNDECLARED: i32 = -3;
const CONNECTOR_INVOKE_ERR_UNAUTHORIZED: i32 = -4;
const CONNECTOR_INVOKE_ERR_PAYLOAD_TOO_LARGE: i32 = -5;
const CONNECTOR_INVOKE_ERR_EXECUTION_FAILED: i32 = -6;
const MAX_CONNECTOR_INVOKE_REQUEST_BYTES: usize = 64 * 1024;
const MAX_CONNECTOR_INVOKE_RESPONSE_BYTES: usize = 64 * 1024;

/// Versioned guest request accepted by `traverse_host::connector_invoke`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConnectorInvokeRequest {
    pub abi_version: String,
    pub connector_id: String,
    pub operation: String,
    pub payload: Value,
}

/// Non-secret response returned to the guest by a mediated connector.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConnectorInvokeResponse {
    pub abi_version: String,
    pub result_class: String,
    pub payload: Value,
}

/// A host-owned activated connector. Its implementation is never visible to a guest.
pub trait MediatedConnector: Send + Sync {
    /// # Errors
    ///
    /// Returns a stable, non-secret failure description when the host-owned
    /// connector cannot complete the requested operation.
    fn invoke(&self, request: &ConnectorInvokeRequest) -> Result<ConnectorInvokeResponse, String>;
}

/// The host-owned authorization context for one nested capability execution.
#[derive(Clone, Default)]
pub struct MediatedConnectorContext {
    pub declared_requirements: Vec<ConnectorRequirement>,
    pub activated_connectors: Vec<ActivatedConnector>,
}

/// One activated connector binding for nested wasmi mediation.
#[derive(Clone)]
pub struct ActivatedConnector {
    pub connector_id: String,
    pub version: String,
    pub implementation: Arc<dyn MediatedConnector>,
}

/// Immutable, non-secret outcome evidence for one mediated connector call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConnectorInvocationEvidence {
    pub connector_id: String,
    pub resolved_version: Option<String>,
    pub result_class: String,
    pub failure_class: Option<String>,
}

const WASI_ERRNO_SUCCESS: i32 = 0;
const WASI_ERRNO_BADF: i32 = 8;
const WASI_ERRNO_INVAL: i32 = 28;
/// Default nested fuel budget when the host does not pass limits (#1656).
pub const DEFAULT_NESTED_FUEL: u64 = 10_000_000;
/// Default nested linear-memory ceiling (Spec 1402 FR-012 / Spec 139 FR-018).
///
/// Matches native `WasmExecutor` / issue `#1336` so certified registry
/// planners that reserve ~273 pages (~17 MiB) initial memory can instantiate
/// inside `runtime.wasm` (for example
/// `core.create-audio-capture-request-plan@1.0.0`). Hosts may raise this via
/// init `limits` (#1656); they may not remove the ceiling.
pub const DEFAULT_NESTED_MEMORY_BYTES: usize = 32 * 1024 * 1024;
/// Absolute upper bound a host may request for nested memory (1 GiB).
pub const MAX_HOST_NESTED_MEMORY_BYTES: usize = 1024 * 1024 * 1024;
/// Absolute upper bound a host may request for nested fuel.
pub const MAX_HOST_NESTED_FUEL: u64 = 2_000_000_000_000;

/// Host-supplied nested execution limits for one `runtime.wasm` session (#1656).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NestedExecutionLimits {
    pub fuel: u64,
    pub max_memory_bytes: usize,
}

impl Default for NestedExecutionLimits {
    fn default() -> Self {
        Self {
            fuel: DEFAULT_NESTED_FUEL,
            max_memory_bytes: DEFAULT_NESTED_MEMORY_BYTES,
        }
    }
}

impl NestedExecutionLimits {
    /// Clamp host-requested limits into the absolute safety ceiling.
    #[must_use]
    pub fn clamped(self) -> Self {
        Self {
            fuel: self.fuel.clamp(1, MAX_HOST_NESTED_FUEL),
            max_memory_bytes: self
                .max_memory_bytes
                .clamp(64 * 1024, MAX_HOST_NESTED_MEMORY_BYTES),
        }
    }
}

/// One capability-declared event, accepted by the shared validation core
/// during a nested execution. The caller (`lib.rs`) turns these into
/// whatever wire representation `traverse_next_event` reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmittedEvent {
    pub event_type: String,
    pub version: String,
    pub data: serde_json::Value,
}

/// The result of running one nested capability to completion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NestedExecutionOutcome {
    pub stdout: Vec<u8>,
    pub emitted_events: Vec<EmittedEvent>,
    pub connector_invocation_evidence: Vec<ConnectorInvocationEvidence>,
}

struct NestedStoreState {
    stdin: Vec<u8>,
    stdin_offset: usize,
    stdout: Vec<u8>,
    limits: wasmi::StoreLimits,
    service_type: ServiceType,
    declared_emits: Vec<EventReference>,
    emitted_events: Vec<EmittedEvent>,
    connector_context: MediatedConnectorContext,
    connector_invocation_evidence: Vec<ConnectorInvocationEvidence>,
}

fn read_memory(caller: &Caller<'_, NestedStoreState>, ptr: i32, len: i32) -> Result<Vec<u8>, i32> {
    if ptr < 0 || len < 0 {
        return Err(WASI_ERRNO_INVAL);
    }
    let Ok(ptr) = usize::try_from(ptr) else {
        return Err(WASI_ERRNO_INVAL);
    };
    let Ok(len) = usize::try_from(len) else {
        return Err(WASI_ERRNO_INVAL);
    };
    let Some(Extern::Memory(memory)) = caller.get_export("memory") else {
        return Err(WASI_ERRNO_INVAL);
    };
    let mut buffer = vec![0u8; len];
    if memory.read(caller, ptr, &mut buffer).is_err() {
        return Err(WASI_ERRNO_INVAL);
    }
    Ok(buffer)
}

fn write_memory(
    caller: &mut Caller<'_, NestedStoreState>,
    ptr: i32,
    bytes: &[u8],
) -> Result<(), i32> {
    if ptr < 0 {
        return Err(WASI_ERRNO_INVAL);
    }
    let Ok(ptr) = usize::try_from(ptr) else {
        return Err(WASI_ERRNO_INVAL);
    };
    let Some(Extern::Memory(memory)) = caller.get_export("memory") else {
        return Err(WASI_ERRNO_INVAL);
    };
    if memory.write(caller, ptr, bytes).is_err() {
        return Err(WASI_ERRNO_INVAL);
    }
    Ok(())
}

fn wasi_fd_read(
    mut caller: Caller<'_, NestedStoreState>,
    fd: i32,
    iovs: i32,
    iovs_len: i32,
    nread_ptr: i32,
) -> i32 {
    if fd != 0 {
        return WASI_ERRNO_BADF;
    }
    if iovs_len < 1 {
        return WASI_ERRNO_INVAL;
    }
    let Ok(header) = read_memory(&caller, iovs, 8) else {
        return WASI_ERRNO_INVAL;
    };
    let buf_ptr = i32::from_le_bytes([header[0], header[1], header[2], header[3]]);
    let buf_len = i32::from_le_bytes([header[4], header[5], header[6], header[7]]);
    if buf_len < 0 {
        return WASI_ERRNO_INVAL;
    }
    let Ok(capacity) = usize::try_from(buf_len) else {
        return WASI_ERRNO_INVAL;
    };
    let remaining = caller
        .data()
        .stdin
        .len()
        .saturating_sub(caller.data().stdin_offset);
    let take = remaining.min(capacity);
    let start = caller.data().stdin_offset;
    let chunk = caller.data().stdin[start..start + take].to_vec();
    if write_memory(&mut caller, buf_ptr, &chunk).is_err() {
        return WASI_ERRNO_INVAL;
    }
    caller.data_mut().stdin_offset += take;
    let nread = i32::try_from(take).unwrap_or(i32::MAX);
    if write_memory(&mut caller, nread_ptr, &nread.to_le_bytes()).is_err() {
        return WASI_ERRNO_INVAL;
    }
    WASI_ERRNO_SUCCESS
}

fn wasi_fd_write(
    mut caller: Caller<'_, NestedStoreState>,
    fd: i32,
    iovs: i32,
    iovs_len: i32,
    nwritten_ptr: i32,
) -> i32 {
    if fd != 1 && fd != 2 {
        return WASI_ERRNO_BADF;
    }
    if iovs_len < 1 {
        return WASI_ERRNO_INVAL;
    }
    let Ok(header) = read_memory(&caller, iovs, 8) else {
        return WASI_ERRNO_INVAL;
    };
    let buf_ptr = i32::from_le_bytes([header[0], header[1], header[2], header[3]]);
    let buf_len = i32::from_le_bytes([header[4], header[5], header[6], header[7]]);
    let Ok(bytes) = read_memory(&caller, buf_ptr, buf_len) else {
        return WASI_ERRNO_INVAL;
    };
    if fd == 1 {
        caller.data_mut().stdout.extend_from_slice(&bytes);
    }
    let nwritten = i32::try_from(bytes.len()).unwrap_or(i32::MAX);
    if write_memory(&mut caller, nwritten_ptr, &nwritten.to_le_bytes()).is_err() {
        return WASI_ERRNO_INVAL;
    }
    WASI_ERRNO_SUCCESS
}

/// WASI's `proc_exit` never returns to its caller. A wasm32 command binary's
/// compiler relies on that contract: LLVM emits `unreachable` after the call.
/// A host stub that returns `()` therefore traps on every guest that calls
/// `proc_exit` (including success/`exit(0)`). Returning `Err(i32_exit(code))`
/// halts the way native Wasmtime's `I32Exit` does — status `0` is success.
fn wasi_proc_exit(_caller: Caller<'_, NestedStoreState>, code: i32) -> Result<(), WasmiError> {
    Err(WasmiError::i32_exit(code))
}

/// `traverse_host::emit_event` as seen by the *nested* capability — same
/// shared validation core the native (Wasmtime) executor calls, so behavior
/// is identical across engines (spec 1402 FR-005).
fn host_emit_event(mut caller: Caller<'_, NestedStoreState>, ptr: i32, len: i32) -> i32 {
    let Ok(payload) = read_memory(&caller, ptr, len) else {
        return traverse_contracts::EMIT_EVENT_ERR_INVALID_PAYLOAD;
    };
    let declared = caller.data().declared_emits.clone();
    match validate_emit_event(&payload, &caller.data().service_type, &declared) {
        Ok(validated) => {
            caller.data_mut().emitted_events.push(EmittedEvent {
                event_type: validated.event_type,
                version: validated.version,
                data: validated.data,
            });
            traverse_contracts::EMIT_EVENT_OK
        }
        Err(error) => error.code(),
    }
}

/// Spec 104 `traverse_host::connector_invoke` — same mediation order as native
/// Wasmtime (`declared → activated → semver → 64 KiB → host-private filter`).
fn host_connector_invoke(
    mut caller: Caller<'_, NestedStoreState>,
    request_ptr: i32,
    request_len: i32,
    response_ptr: i32,
    response_capacity: i32,
) -> i32 {
    let Ok((request, response_ptr, response_capacity)) = parse_connector_request(
        &caller,
        request_ptr,
        request_len,
        response_ptr,
        response_capacity,
    ) else {
        return CONNECTOR_INVOKE_ERR_INVALID_REQUEST;
    };
    let context = caller.data().connector_context.clone();
    let connector = match resolve_activated_connector(&context, &request) {
        Ok(connector) => connector,
        Err(failure) => {
            return connector_failure(
                &mut caller,
                &request.connector_id,
                failure.resolved_version.as_deref(),
                failure.failure_class,
                failure.code,
            );
        }
    };
    invoke_activated_connector(
        &mut caller,
        request,
        response_ptr,
        response_capacity,
        &connector,
    )
}

fn parse_connector_request(
    caller: &Caller<'_, NestedStoreState>,
    request_ptr: i32,
    request_len: i32,
    response_ptr: i32,
    response_capacity: i32,
) -> Result<(ConnectorInvokeRequest, usize, usize), ()> {
    if request_ptr < 0 || request_len < 0 || response_ptr < 0 || response_capacity < 0 {
        return Err(());
    }
    let Ok(request_len_u) = usize::try_from(request_len) else {
        return Err(());
    };
    let Ok(response_ptr_u) = usize::try_from(response_ptr) else {
        return Err(());
    };
    let Ok(response_capacity_u) = usize::try_from(response_capacity) else {
        return Err(());
    };
    if request_len_u > MAX_CONNECTOR_INVOKE_REQUEST_BYTES
        || response_capacity_u > MAX_CONNECTOR_INVOKE_RESPONSE_BYTES
    {
        return Err(());
    }
    let Ok(bytes) = read_memory(caller, request_ptr, request_len) else {
        return Err(());
    };
    let Ok(request) = serde_json::from_slice::<ConnectorInvokeRequest>(&bytes) else {
        return Err(());
    };
    if request.abi_version != SUPPORTED_HOST_ABI_VERSION
        || request.connector_id.is_empty()
        || request.operation.is_empty()
        || contains_host_private_data(&request.payload)
    {
        return Err(());
    }
    Ok((request, response_ptr_u, response_capacity_u))
}

struct ConnectorInvokeFailure {
    failure_class: &'static str,
    resolved_version: Option<String>,
    code: i32,
}

fn resolve_activated_connector(
    context: &MediatedConnectorContext,
    request: &ConnectorInvokeRequest,
) -> Result<ActivatedConnector, ConnectorInvokeFailure> {
    if !context
        .declared_requirements
        .iter()
        .any(|requirement| requirement.connector_id == request.connector_id)
    {
        return Err(ConnectorInvokeFailure {
            failure_class: "undeclared",
            resolved_version: None,
            code: CONNECTOR_INVOKE_ERR_UNDECLARED,
        });
    }
    let Some(connector) = context
        .activated_connectors
        .iter()
        .find(|connector| connector.connector_id == request.connector_id)
    else {
        return Err(ConnectorInvokeFailure {
            failure_class: "unbound",
            resolved_version: None,
            code: CONNECTOR_INVOKE_ERR_UNBOUND,
        });
    };
    let compatible = context
        .declared_requirements
        .iter()
        .filter(|requirement| requirement.connector_id == request.connector_id)
        .any(|requirement| {
            VersionReq::parse(&requirement.version)
                .ok()
                .zip(Version::parse(&connector.version).ok())
                .is_some_and(|(range, version)| range.matches(&version))
        });
    if !compatible {
        return Err(ConnectorInvokeFailure {
            failure_class: "incompatible",
            resolved_version: Some(connector.version.clone()),
            code: CONNECTOR_INVOKE_ERR_UNAUTHORIZED,
        });
    }
    Ok(connector.clone())
}

fn invoke_activated_connector(
    caller: &mut Caller<'_, NestedStoreState>,
    request: ConnectorInvokeRequest,
    response_ptr: usize,
    response_capacity: usize,
    connector: &ActivatedConnector,
) -> i32 {
    let Ok(response) = connector.implementation.invoke(&request) else {
        return connector_failure(
            caller,
            &request.connector_id,
            Some(&connector.version),
            "execution_failed",
            CONNECTOR_INVOKE_ERR_EXECUTION_FAILED,
        );
    };
    if response.abi_version != SUPPORTED_HOST_ABI_VERSION
        || response.result_class.is_empty()
        || contains_host_private_data(&response.payload)
    {
        return connector_failure(
            caller,
            &request.connector_id,
            Some(&connector.version),
            "unauthorized_output",
            CONNECTOR_INVOKE_ERR_UNAUTHORIZED,
        );
    }
    let Ok(response_bytes) = serde_json::to_vec(&response) else {
        return connector_failure(
            caller,
            &request.connector_id,
            Some(&connector.version),
            "execution_failed",
            CONNECTOR_INVOKE_ERR_EXECUTION_FAILED,
        );
    };
    if response_bytes.len() > response_capacity
        || response_bytes.len() > MAX_CONNECTOR_INVOKE_RESPONSE_BYTES
    {
        return connector_failure(
            caller,
            &request.connector_id,
            Some(&connector.version),
            "bounded_io",
            CONNECTOR_INVOKE_ERR_PAYLOAD_TOO_LARGE,
        );
    }
    let Ok(response_len) = i32::try_from(response_bytes.len()) else {
        return connector_failure(
            caller,
            &request.connector_id,
            Some(&connector.version),
            "bounded_io",
            CONNECTOR_INVOKE_ERR_PAYLOAD_TOO_LARGE,
        );
    };
    let Ok(response_ptr_i32) = i32::try_from(response_ptr) else {
        return connector_failure(
            caller,
            &request.connector_id,
            Some(&connector.version),
            "invalid_response_memory",
            CONNECTOR_INVOKE_ERR_INVALID_REQUEST,
        );
    };
    if write_memory(caller, response_ptr_i32, &response_bytes).is_err() {
        return connector_failure(
            caller,
            &request.connector_id,
            Some(&connector.version),
            "invalid_response_memory",
            CONNECTOR_INVOKE_ERR_INVALID_REQUEST,
        );
    }
    caller
        .data_mut()
        .connector_invocation_evidence
        .push(ConnectorInvocationEvidence {
            connector_id: request.connector_id,
            resolved_version: Some(connector.version.clone()),
            result_class: response.result_class,
            failure_class: None,
        });
    response_len
}

fn connector_failure(
    caller: &mut Caller<'_, NestedStoreState>,
    connector_id: &str,
    resolved_version: Option<&str>,
    failure_class: &str,
    code: i32,
) -> i32 {
    caller
        .data_mut()
        .connector_invocation_evidence
        .push(ConnectorInvocationEvidence {
            connector_id: connector_id.to_string(),
            resolved_version: resolved_version.map(str::to_string),
            result_class: "failure".to_string(),
            failure_class: Some(failure_class.to_string()),
        });
    code
}

fn contains_host_private_data(value: &Value) -> bool {
    match value {
        Value::Object(map) => map.iter().any(|(key, value)| {
            let key = key.to_ascii_lowercase();
            [
                "config",
                "credential",
                "secret",
                "password",
                "path",
                "device",
                "endpoint",
                "bucket",
            ]
            .iter()
            .any(|needle| key.contains(needle))
                || contains_host_private_data(value)
        }),
        Value::Array(values) => values.iter().any(contains_host_private_data),
        Value::String(value) => value.starts_with('/') || value.contains("://"),
        _ => false,
    }
}

/// Executes `artifact` as a WASI-command-shaped capability with the RFC 8785
/// canonical form of the JSON `input` as stdin (Spec 1722, Decision 118),
/// inside a nested `wasmi` engine — the same engine and
/// feature matrix Decision 87's spike proved compiles for `wasm32-unknown-
/// unknown` and hosts a Traverse capability's stdin/stdout convention.
///
/// # Errors
///
/// Returns a stable, secret-free error string when module validation,
/// linking, fuel setup, or execution fails.
pub fn execute_nested_capability(
    artifact: &[u8],
    input: &[u8],
    service_type: &ServiceType,
    declared_emits: &[EventReference],
) -> Result<NestedExecutionOutcome, String> {
    execute_nested_capability_with_limits(
        artifact,
        input,
        service_type,
        declared_emits,
        NestedExecutionLimits::default(),
    )
}

/// Like [`execute_nested_capability`], with host-supplied fuel / memory (#1656).
///
/// # Errors
///
/// Returns a stable, secret-free error string when the input is not
/// canonicalizable JSON (`input_not_canonicalizable: …` or `invalid_json: …`)
/// or when module validation, linking, fuel setup, or execution fails.
pub fn execute_nested_capability_with_limits(
    artifact: &[u8],
    input: &[u8],
    service_type: &ServiceType,
    declared_emits: &[EventReference],
    host_limits: NestedExecutionLimits,
) -> Result<NestedExecutionOutcome, String> {
    execute_nested_capability_with_connectors(
        artifact,
        input,
        service_type,
        declared_emits,
        host_limits,
        &MediatedConnectorContext::default(),
    )
}

/// Like [`execute_nested_capability_with_limits`], with Spec 104 mediated
/// connector bindings for nested `traverse_host::connector_invoke` (#1719).
///
/// # Errors
///
/// Returns a stable, secret-free error string when the input is not
/// canonicalizable JSON (`input_not_canonicalizable: …` or `invalid_json: …`)
/// or when module validation, linking, fuel setup, or execution fails.
pub fn execute_nested_capability_with_connectors(
    artifact: &[u8],
    input: &[u8],
    service_type: &ServiceType,
    declared_emits: &[EventReference],
    host_limits: NestedExecutionLimits,
    connector_context: &MediatedConnectorContext,
) -> Result<NestedExecutionOutcome, String> {
    // Spec 1722 FR-001/FR-003: every host's guest reads the same canonical
    // bytes, and input JCS cannot represent fails before any guest runs.
    let input = canonicalize_json_bytes(input).map_err(|error| error.to_string())?;
    let host_limits = host_limits.clamped();
    let mut config = Config::default();
    config.consume_fuel(true);
    let engine = Engine::new(&config);
    let module = Module::new(&engine, artifact).map_err(|error| format!("module: {error}"))?;
    let limits = StoreLimitsBuilder::new()
        .memory_size(host_limits.max_memory_bytes)
        .trap_on_grow_failure(true)
        .build();
    let state = NestedStoreState {
        stdin: input.into_bytes(),
        stdin_offset: 0,
        stdout: Vec::new(),
        limits,
        service_type: service_type.clone(),
        declared_emits: declared_emits.to_vec(),
        emitted_events: Vec::new(),
        connector_context: connector_context.clone(),
        connector_invocation_evidence: Vec::new(),
    };
    let mut store = Store::new(&engine, state);
    store.limiter(|state| &mut state.limits);
    store
        .set_fuel(host_limits.fuel)
        .map_err(|error| format!("fuel: {error}"))?;

    let mut linker = Linker::new(&engine);
    linker
        .func_wrap("wasi_snapshot_preview1", "fd_read", wasi_fd_read)
        .map_err(|error| format!("link fd_read: {error}"))?;
    linker
        .func_wrap("wasi_snapshot_preview1", "fd_write", wasi_fd_write)
        .map_err(|error| format!("link fd_write: {error}"))?;
    linker
        .func_wrap("wasi_snapshot_preview1", "proc_exit", wasi_proc_exit)
        .map_err(|error| format!("link proc_exit: {error}"))?;
    linker
        .func_wrap("traverse_host", "emit_event", host_emit_event)
        .map_err(|error| format!("link emit_event: {error}"))?;
    linker
        .func_wrap("traverse_host", "connector_invoke", host_connector_invoke)
        .map_err(|error| format!("link connector_invoke: {error}"))?;

    let instance = linker
        .instantiate_and_start(&mut store, &module)
        .map_err(|error| format!("instantiate: {error}"))?;
    let start = instance
        .get_typed_func::<(), ()>(&store, "_start")
        .map_err(|error| format!("missing _start: {error}"))?;
    if let Err(error) = start.call(&mut store, ()) {
        // Guest `proc_exit` surfaces as `i32_exit` (see `wasi_proc_exit`);
        // status `0` is successful completion, matching native Wasmtime.
        if error.i32_exit_status() != Some(0) {
            return Err(format!("execution: {error}"));
        }
    }

    let final_state = store.into_data();
    Ok(NestedExecutionOutcome {
        stdout: final_state.stdout,
        emitted_events: final_state.emitted_events,
        connector_invocation_evidence: final_state.connector_invocation_evidence,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        ActivatedConnector, ConnectorInvokeRequest, ConnectorInvokeResponse, EmittedEvent,
        MediatedConnector, MediatedConnectorContext, NestedExecutionLimits,
        NestedExecutionOutcome, SUPPORTED_HOST_ABI_VERSION, execute_nested_capability,
        execute_nested_capability_with_connectors, execute_nested_capability_with_limits,
    };
    use std::sync::Arc;
    use traverse_contracts::{ConnectorRequirement, EventReference, ServiceType};

    /// A WASI-command module that echoes stdin to stdout, then calls
    /// `traverse_host::emit_event` with a fixed declared event.
    const ECHO_AND_EMIT_WAT: &str = r#"
      (module
        (import "wasi_snapshot_preview1" "fd_read"
          (func $fd_read (param i32 i32 i32 i32) (result i32)))
        (import "wasi_snapshot_preview1" "fd_write"
          (func $fd_write (param i32 i32 i32 i32) (result i32)))
        (import "traverse_host" "emit_event"
          (func $emit_event (param i32 i32) (result i32)))
        (memory (export "memory") 1)
        (data (i32.const 5000) "{\"event_id\":\"order.placed\",\"version\":\"1.0.0\",\"payload\":{\"order_id\":\"abc\"}}")
        (func (export "_start")
          (i32.store (i32.const 0) (i32.const 8))
          (i32.store (i32.const 4) (i32.const 1024))
          (drop (call $fd_read (i32.const 0) (i32.const 0) (i32.const 1) (i32.const 4100)))
          (i32.store (i32.const 0) (i32.const 8))
          (i32.store (i32.const 4) (i32.load (i32.const 4100)))
          (drop (call $fd_write (i32.const 1) (i32.const 0) (i32.const 1) (i32.const 4104)))
          (drop (call $emit_event (i32.const 5000) (i32.const 74)))
        )
      )
    "#;

    fn declared() -> Vec<EventReference> {
        vec![EventReference {
            event_id: "order.placed".to_string(),
            version: "1.0.0".to_string(),
        }]
    }

    #[test]
    fn nested_capability_echoes_stdin_and_emits_a_declared_event() -> Result<(), String> {
        let artifact =
            wat::parse_str(ECHO_AND_EMIT_WAT).map_err(|error| format!("wat: {error}"))?;
        let input = br#"{"hello":"nested"}"#;
        let outcome =
            execute_nested_capability(&artifact, input, &ServiceType::Subscribable, &declared())?;

        if outcome.stdout != input {
            return Err("stdout mismatch".to_string());
        }

        let expected = vec![EmittedEvent {
            event_type: "order.placed".to_string(),
            version: "1.0.0".to_string(),
            data: serde_json::json!({"order_id": "abc"}),
        }];
        if outcome.emitted_events != expected {
            return Err(format!(
                "emitted events mismatch: {:?} != {expected:?}",
                outcome.emitted_events
            ));
        }
        Ok(())
    }

    #[test]
    fn nested_capability_reads_the_canonical_form_of_its_input() -> Result<(), String> {
        let artifact =
            wat::parse_str(ECHO_AND_EMIT_WAT).map_err(|error| format!("wat: {error}"))?;
        let input = br#"{ "b" : [1.0, 2e-3], "a" : "x" }"#;
        let outcome = execute_nested_capability(&artifact, input, &ServiceType::Stateless, &[])?;
        if outcome.stdout != br#"{"a":"x","b":[1,0.002]}"# {
            return Err(format!(
                "stdout was not canonical: {}",
                String::from_utf8_lossy(&outcome.stdout)
            ));
        }
        Ok(())
    }

    #[test]
    fn non_canonicalizable_input_fails_before_the_guest_runs() -> Result<(), String> {
        let artifact =
            wat::parse_str(ECHO_AND_EMIT_WAT).map_err(|error| format!("wat: {error}"))?;
        for (input, expected) in [
            (
                &br#"{"id":9007199254740993}"#[..],
                "input_not_canonicalizable: ",
            ),
            (&br#"{"a":1,"a":2}"#[..], "input_not_canonicalizable: "),
            (&br#""\ud800""#[..], "input_not_canonicalizable: "),
            (&b"not json"[..], "invalid_json: "),
        ] {
            match execute_nested_capability(&artifact, input, &ServiceType::Stateless, &[]) {
                Err(error) if error.starts_with(expected) => {}
                other => return Err(format!("unexpected outcome: {other:?}")),
            }
        }
        Ok(())
    }

    #[test]
    fn undeclared_event_is_rejected_by_the_same_shared_core_native_uses() -> Result<(), String> {
        let artifact =
            wat::parse_str(ECHO_AND_EMIT_WAT).map_err(|error| format!("wat: {error}"))?;
        let input = br#"{"hello":"nested"}"#;
        // No declared emits at all: the nested capability's emit_event call
        // must be rejected by the shared core exactly like the native
        // executor rejects an undeclared event — and execution must not
        // fail just because the emit was rejected (the guest ignores the
        // drop'd return code, matching real capability behavior).
        let outcome = execute_nested_capability(&artifact, input, &ServiceType::Subscribable, &[])?;
        if !outcome.emitted_events.is_empty() {
            return Err("undeclared event must not be recorded".to_string());
        }
        Ok(())
    }

    #[test]
    fn non_subscribable_service_type_rejects_the_emit_before_touching_payload() -> Result<(), String>
    {
        let artifact =
            wat::parse_str(ECHO_AND_EMIT_WAT).map_err(|error| format!("wat: {error}"))?;
        let input = br#"{"hello":"nested"}"#;
        let outcome =
            execute_nested_capability(&artifact, input, &ServiceType::Stateless, &declared())?;
        if !outcome.emitted_events.is_empty() {
            return Err("non-subscribable capability must not record events".to_string());
        }
        Ok(())
    }

    #[test]
    fn malformed_module_bytes_fail_closed_with_a_stable_error() {
        let result = execute_nested_capability(
            b"not a real wasm module",
            b"{}",
            &ServiceType::Subscribable,
            &declared(),
        );
        assert!(result.is_err());
    }

    #[test]
    fn module_without_start_export_fails_closed() -> Result<(), String> {
        const NO_START_WAT: &str = r#"(module (memory (export "memory") 1))"#;
        let artifact = wat::parse_str(NO_START_WAT).map_err(|error| format!("wat: {error}"))?;
        let result =
            execute_nested_capability(&artifact, b"{}", &ServiceType::Subscribable, &declared());
        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn fd_read_on_a_non_stdin_descriptor_is_rejected() -> Result<(), String> {
        const BAD_FD_READ_WAT: &str = r#"
          (module
            (import "wasi_snapshot_preview1" "fd_read"
              (func $fd_read (param i32 i32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (global (export "result") (mut i32) (i32.const -1))
            (func (export "_start")
              (i32.store (i32.const 0) (i32.const 8))
              (i32.store (i32.const 4) (i32.const 1))
              (drop (call $fd_read (i32.const 5) (i32.const 0) (i32.const 1) (i32.const 100))))
          )
        "#;
        let artifact = wat::parse_str(BAD_FD_READ_WAT).map_err(|error| format!("wat: {error}"))?;
        // A non-zero fd must not trap the module: `wasi_fd_read` returns
        // `WASI_ERRNO_BADF` and execution continues to completion.
        let outcome =
            execute_nested_capability(&artifact, b"{}", &ServiceType::Subscribable, &declared())?;
        assert!(outcome.stdout.is_empty());
        Ok(())
    }

    #[test]
    fn fd_write_on_a_non_stdout_descriptor_is_rejected() -> Result<(), String> {
        const BAD_FD_WRITE_WAT: &str = r#"
          (module
            (import "wasi_snapshot_preview1" "fd_write"
              (func $fd_write (param i32 i32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (func (export "_start")
              (i32.store (i32.const 0) (i32.const 8))
              (i32.store (i32.const 4) (i32.const 4))
              (drop (call $fd_write (i32.const 5) (i32.const 0) (i32.const 1) (i32.const 100))))
          )
        "#;
        let artifact = wat::parse_str(BAD_FD_WRITE_WAT).map_err(|error| format!("wat: {error}"))?;
        let outcome =
            execute_nested_capability(&artifact, b"{}", &ServiceType::Subscribable, &declared())?;
        assert!(outcome.stdout.is_empty());
        Ok(())
    }

    /// Regression for Spec 1402 FR-012 / issue #1467: modules that reserve
    /// ~273 pages (~17.8 MiB) of initial memory — the released
    /// `core.create-audio-capture-request-plan@1.0.0` shape — must instantiate
    /// under the nested wasmi ceiling (32 MiB), not the old 16 MiB denial.
    #[test]
    fn nested_executor_instantiates_modules_with_273_page_initial_memory() -> Result<(), String> {
        const LARGE_INITIAL_MEMORY_WAT: &str = r#"
          (module
            (memory (export "memory") 273)
            (func (export "_start"))
          )
        "#;
        let artifact =
            wat::parse_str(LARGE_INITIAL_MEMORY_WAT).map_err(|error| format!("wat: {error}"))?;
        let outcome = execute_nested_capability(&artifact, b"{}", &ServiceType::Stateless, &[])?;
        assert!(outcome.stdout.is_empty());
        assert!(outcome.emitted_events.is_empty());
        Ok(())
    }

    /// Real registry capabilities call `proc_exit(0)` after writing stdout;
    /// the host must halt (not fall through to `unreachable`) and succeed.
    #[test]
    fn capability_calling_proc_exit_zero_after_output_succeeds() -> Result<(), String> {
        const WRITE_THEN_PROC_EXIT_ZERO_WAT: &str = r#"
          (module
            (import "wasi_snapshot_preview1" "fd_write"
              (func $fd_write (param i32 i32 i32 i32) (result i32)))
            (import "wasi_snapshot_preview1" "proc_exit" (func $proc_exit (param i32)))
            (memory (export "memory") 1)
            (data (i32.const 5000) "{\"valid\":true}")
            (func (export "_start")
              (i32.store (i32.const 0) (i32.const 5000))
              (i32.store (i32.const 4) (i32.const 14))
              (drop (call $fd_write (i32.const 1) (i32.const 0) (i32.const 1) (i32.const 100)))
              (call $proc_exit (i32.const 0))
            )
          )
        "#;
        let artifact = wat::parse_str(WRITE_THEN_PROC_EXIT_ZERO_WAT)
            .map_err(|error| format!("wat: {error}"))?;
        let outcome = execute_nested_capability(&artifact, b"{}", &ServiceType::Stateless, &[])?;
        if outcome.stdout != br#"{"valid":true}"# {
            return Err("stdout mismatch after proc_exit(0)".to_string());
        }
        Ok(())
    }

    #[test]
    fn capability_calling_proc_exit_nonzero_is_a_distinct_execution_failure() -> Result<(), String>
    {
        const PROC_EXIT_NONZERO_WAT: &str = r#"
          (module
            (import "wasi_snapshot_preview1" "proc_exit" (func $proc_exit (param i32)))
            (memory (export "memory") 1)
            (func (export "_start")
              (call $proc_exit (i32.const 2))
            )
          )
        "#;
        let artifact =
            wat::parse_str(PROC_EXIT_NONZERO_WAT).map_err(|error| format!("wat: {error}"))?;
        let result = execute_nested_capability(&artifact, b"{}", &ServiceType::Stateless, &[]);
        let Err(error) = result else {
            return Err("non-zero proc_exit must fail the call".to_string());
        };
        if error.contains("unreachable") {
            return Err(format!(
                "must not be misreported as the unrelated unreachable trap: {error}"
            ));
        }
        Ok(())
    }

    #[test]
    fn host_limits_below_initial_memory_fail_at_instantiate() -> Result<(), String> {
        const LARGE_MEMORY_WAT: &str = r#"
          (module
            (memory (export "memory") 64)
            (func (export "_start"))
          )
        "#;
        let artifact = wat::parse_str(LARGE_MEMORY_WAT).map_err(|error| format!("wat: {error}"))?;
        // 64 pages = 4 MiB; host ceiling of 1 MiB must deny instantiate.
        let result = execute_nested_capability_with_limits(
            &artifact,
            b"{}",
            &ServiceType::Stateless,
            &[],
            NestedExecutionLimits {
                fuel: 1_000_000,
                max_memory_bytes: 1024 * 1024,
            },
        );
        let Err(error) = result else {
            return Err("undersized host memory must fail instantiate".to_string());
        };
        if !(error.contains("memory") || error.contains("instantiate") || error.contains("limiter"))
        {
            return Err(format!("expected memory/instantiate failure, got: {error}"));
        }
        Ok(())
    }

    #[test]
    fn nested_limits_clamp_rejects_zero_and_caps_ceiling() {
        let zero = NestedExecutionLimits {
            fuel: 0,
            max_memory_bytes: 0,
        }
        .clamped();
        assert_eq!(zero.fuel, 1);
        assert_eq!(zero.max_memory_bytes, 64 * 1024);

        let huge = NestedExecutionLimits {
            fuel: u64::MAX,
            max_memory_bytes: usize::MAX,
        }
        .clamped();
        assert_eq!(huge.fuel, super::MAX_HOST_NESTED_FUEL);
        assert_eq!(huge.max_memory_bytes, super::MAX_HOST_NESTED_MEMORY_BYTES);
    }

    const VALID_CONNECTOR_REQUEST: &str = r#"{"abi_version":"1.0.0","connector_id":"traverse.test","operation":"read","payload":{"id":"1"}}"#;

    struct TestConnector;

    impl MediatedConnector for TestConnector {
        fn invoke(
            &self,
            request: &ConnectorInvokeRequest,
        ) -> Result<ConnectorInvokeResponse, String> {
            Ok(ConnectorInvokeResponse {
                abi_version: SUPPORTED_HOST_ABI_VERSION.to_string(),
                result_class: "success".to_string(),
                payload: serde_json::json!({"request_id": request.payload["id"]}),
            })
        }
    }

    enum ConnectorOutcome {
        Fails,
        UnsafeOutput,
        LargeOutput,
    }

    struct OutcomeConnector(ConnectorOutcome);

    impl MediatedConnector for OutcomeConnector {
        fn invoke(
            &self,
            _request: &ConnectorInvokeRequest,
        ) -> Result<ConnectorInvokeResponse, String> {
            match self.0 {
                ConnectorOutcome::Fails => Err("connector failed".to_string()),
                ConnectorOutcome::UnsafeOutput => Ok(ConnectorInvokeResponse {
                    abi_version: SUPPORTED_HOST_ABI_VERSION.to_string(),
                    result_class: "success".to_string(),
                    payload: serde_json::json!({"secret": "must not cross the ABI"}),
                }),
                ConnectorOutcome::LargeOutput => Ok(ConnectorInvokeResponse {
                    abi_version: SUPPORTED_HOST_ABI_VERSION.to_string(),
                    result_class: "success".to_string(),
                    payload: serde_json::json!({"value": "x".repeat(1024)}),
                }),
            }
        }
    }

    fn connector_test_wasm(
        request: &str,
        request_ptr: i32,
        request_len: i32,
        response_ptr: i32,
        response_capacity: i32,
    ) -> Result<Vec<u8>, String> {
        let escaped_request = request.replace('\\', "\\\\").replace('"', "\\\"");
        wat::parse_str(format!(
            r#"
            (module
              (import "wasi_snapshot_preview1" "fd_write" (func $write (param i32 i32 i32 i32) (result i32)))
              (import "traverse_host" "connector_invoke" (func $invoke (param i32 i32 i32 i32) (result i32)))
              (memory (export "memory") 1)
              (data (i32.const 32) "{escaped_request}")
              (data (i32.const 2048) "0")
              (func $_start (export "_start")
                i32.const {request_ptr} i32.const {request_len} i32.const {response_ptr} i32.const {response_capacity} call $invoke drop
                i32.const 4 i32.const 2048 i32.store
                i32.const 8 i32.const 1 i32.store
                i32.const 1 i32.const 4 i32.const 1 i32.const 12 call $write drop)
            )
            "#
        ))
        .map_err(|error| format!("wat: {error}"))
    }

    fn connector_response_wasm(
        request: &str,
        request_ptr: i32,
        request_len: i32,
        response_ptr: i32,
        response_capacity: i32,
    ) -> Result<Vec<u8>, String> {
        let escaped_request = request.replace('\\', "\\\\").replace('"', "\\\"");
        wat::parse_str(format!(
            r#"
            (module
              (import "wasi_snapshot_preview1" "fd_write" (func $write (param i32 i32 i32 i32) (result i32)))
              (import "traverse_host" "connector_invoke" (func $invoke (param i32 i32 i32 i32) (result i32)))
              (memory (export "memory") 1)
              (data (i32.const 32) "{escaped_request}")
              (func $_start (export "_start") (local $len i32)
                i32.const {request_ptr} i32.const {request_len} i32.const {response_ptr} i32.const {response_capacity} call $invoke local.set $len
                i32.const 4 i32.const {response_ptr} i32.store
                i32.const 8 local.get $len i32.store
                i32.const 1 i32.const 4 i32.const 1 i32.const 12 call $write drop)
            )
            "#
        ))
        .map_err(|error| format!("wat: {error}"))
    }

    fn activated_context(version: &str) -> MediatedConnectorContext {
        MediatedConnectorContext {
            declared_requirements: vec![ConnectorRequirement {
                connector_id: "traverse.test".to_string(),
                version: "^1.0.0".to_string(),
            }],
            activated_connectors: vec![ActivatedConnector {
                connector_id: "traverse.test".to_string(),
                version: version.to_string(),
                implementation: Arc::new(TestConnector),
            }],
        }
    }

    fn declared_only_context() -> MediatedConnectorContext {
        MediatedConnectorContext {
            declared_requirements: vec![ConnectorRequirement {
                connector_id: "traverse.test".to_string(),
                version: "^1.0.0".to_string(),
            }],
            activated_connectors: Vec::new(),
        }
    }

    fn run_with_context(
        wasm: &[u8],
        context: &MediatedConnectorContext,
    ) -> Result<NestedExecutionOutcome, String> {
        execute_nested_capability_with_connectors(
            wasm,
            b"{}",
            &ServiceType::Stateless,
            &[],
            NestedExecutionLimits::default(),
            context,
        )
    }

    #[test]
    fn nested_connector_invoke_import_instantiates() -> Result<(), String> {
        let wasm = wat::parse_str(
            r#"
            (module
              (import "traverse_host" "connector_invoke"
                (func $connector_invoke (param i32 i32 i32 i32) (result i32)))
              (memory (export "memory") 1)
              (func (export "_start"))
            )
            "#,
        )
        .map_err(|error| format!("wat: {error}"))?;
        let outcome = execute_nested_capability(&wasm, b"{}", &ServiceType::Stateless, &[])?;
        assert!(outcome.stdout.is_empty());
        assert!(outcome.connector_invocation_evidence.is_empty());
        Ok(())
    }

    #[test]
    fn nested_connector_invoke_records_unbound_and_undeclared() -> Result<(), String> {
        let request_len = i32::try_from(VALID_CONNECTOR_REQUEST.len())
            .map_err(|error| format!("len: {error}"))?;
        let wasm = connector_test_wasm(VALID_CONNECTOR_REQUEST, 32, request_len, 256, 64)?;

        let unbound = run_with_context(&wasm, &declared_only_context())?;
        assert_eq!(unbound.connector_invocation_evidence.len(), 1);
        assert_eq!(
            unbound.connector_invocation_evidence[0].failure_class.as_deref(),
            Some("unbound")
        );

        let undeclared = run_with_context(&wasm, &MediatedConnectorContext::default())?;
        assert_eq!(undeclared.connector_invocation_evidence.len(), 1);
        assert_eq!(
            undeclared.connector_invocation_evidence[0]
                .failure_class
                .as_deref(),
            Some("undeclared")
        );
        Ok(())
    }

    #[test]
    fn nested_connector_invoke_succeeds_with_activated_binding() -> Result<(), String> {
        let request_len = i32::try_from(VALID_CONNECTOR_REQUEST.len())
            .map_err(|error| format!("len: {error}"))?;
        let wasm = connector_response_wasm(VALID_CONNECTOR_REQUEST, 32, request_len, 256, 256)?;
        let outcome = run_with_context(&wasm, &activated_context("1.0.0"))?;
        let response: ConnectorInvokeResponse = serde_json::from_slice(&outcome.stdout)
            .map_err(|error| format!("stdout json: {error}"))?;
        assert_eq!(response.result_class, "success");
        assert_eq!(response.payload["request_id"], "1");
        assert_eq!(outcome.connector_invocation_evidence.len(), 1);
        assert_eq!(
            outcome.connector_invocation_evidence[0].result_class,
            "success"
        );
        assert!(outcome.connector_invocation_evidence[0].failure_class.is_none());
        Ok(())
    }

    #[test]
    fn nested_connector_invoke_records_activated_failures() -> Result<(), String> {
        let request_len = i32::try_from(VALID_CONNECTOR_REQUEST.len())
            .map_err(|error| format!("len: {error}"))?;
        let wasm = connector_test_wasm(VALID_CONNECTOR_REQUEST, 32, request_len, 256, 64)?;

        let cases: Vec<(Arc<dyn MediatedConnector>, &str, Option<&str>)> = vec![
            (
                Arc::new(OutcomeConnector(ConnectorOutcome::Fails)),
                "failure",
                Some("execution_failed"),
            ),
            (
                Arc::new(OutcomeConnector(ConnectorOutcome::UnsafeOutput)),
                "failure",
                Some("unauthorized_output"),
            ),
            (
                Arc::new(OutcomeConnector(ConnectorOutcome::LargeOutput)),
                "failure",
                Some("bounded_io"),
            ),
            (
                Arc::new(TestConnector),
                "failure",
                Some("incompatible"),
            ),
        ];
        let versions = ["1.0.0", "1.0.0", "1.0.0", "2.0.0"];
        for ((implementation, result_class, failure_class), version) in
            cases.into_iter().zip(versions)
        {
            let context = MediatedConnectorContext {
                declared_requirements: vec![ConnectorRequirement {
                    connector_id: "traverse.test".to_string(),
                    version: "^1.0.0".to_string(),
                }],
                activated_connectors: vec![ActivatedConnector {
                    connector_id: "traverse.test".to_string(),
                    version: version.to_string(),
                    implementation,
                }],
            };
            let outcome = run_with_context(&wasm, &context)?;
            assert_eq!(outcome.connector_invocation_evidence.len(), 1);
            assert_eq!(
                outcome.connector_invocation_evidence[0].result_class,
                result_class
            );
            assert_eq!(
                outcome.connector_invocation_evidence[0]
                    .failure_class
                    .as_deref(),
                failure_class
            );
        }
        Ok(())
    }

    #[test]
    fn nested_connector_invoke_denies_invalid_requests() -> Result<(), String> {
        let valid_len = i32::try_from(VALID_CONNECTOR_REQUEST.len())
            .map_err(|error| format!("len: {error}"))?;
        let invalid_requests = [
            ("not-json", 32, 8, 256, 64),
            (
                r#"{"abi_version":"2.0.0","connector_id":"traverse.test","operation":"read","payload":{}}"#,
                32,
                0,
                256,
                64,
            ),
            (
                r#"{"abi_version":"1.0.0","connector_id":"","operation":"read","payload":{}}"#,
                32,
                0,
                256,
                64,
            ),
            (
                r#"{"abi_version":"1.0.0","connector_id":"traverse.test","operation":"","payload":{}}"#,
                32,
                0,
                256,
                64,
            ),
            (
                r#"{"abi_version":"1.0.0","connector_id":"traverse.test","operation":"read","payload":{"path":"/private"}}"#,
                32,
                0,
                256,
                64,
            ),
            (
                r#"{"abi_version":"1.0.0","connector_id":"traverse.test","operation":"read","payload":{"items":["https://example.test"]}}"#,
                32,
                0,
                256,
                64,
            ),
            (VALID_CONNECTOR_REQUEST, -1, 1, 256, 64),
            (VALID_CONNECTOR_REQUEST, 65_535, 10, 256, 64),
            (VALID_CONNECTOR_REQUEST, 32, valid_len, 256, 65_537),
        ];
        for (request, request_ptr, request_len, response_ptr, response_capacity) in invalid_requests {
            let request_len = if request_len == 0 {
                i32::try_from(request.len()).map_err(|error| format!("len: {error}"))?
            } else {
                request_len
            };
            let wasm = connector_test_wasm(
                request,
                request_ptr,
                request_len,
                response_ptr,
                response_capacity,
            )?;
            let outcome = run_with_context(&wasm, &activated_context("1.0.0"))?;
            assert!(
                outcome.connector_invocation_evidence.is_empty(),
                "invalid requests must not record evidence"
            );
        }
        Ok(())
    }
}
