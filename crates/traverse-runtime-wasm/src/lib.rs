//! `runtime.wasm`: the nested-wasmi orchestrator (Spec
//! `1402-runtime-wasm-orchestrator-convergence` Phase 2, FR-003/FR-011).
//!
//! Exports the unchanged `runtime-wasm-bridge/1.0.0` C-ABI (spec `071`
//! FR-006) so existing native host adapters (Swift/wasmi, Kotlin/Chicory,
//! .NET/Wasmtime) require no changes. Internally, capability execution runs
//! on a nested `wasmi` engine (Decision 87) rather than the native
//! `WasmExecutor`'s Wasmtime, because Wasmtime cannot itself target
//! `wasm32` — see [`executor`] for that engine and the shared
//! `emit_event` validation core it calls.
//!
//! # Audited `unsafe_code` exception (ADR-0073)
//!
//! A Rust C-ABI guest boundary has no safe equivalent for turning a raw
//! `(ptr, len)` pair supplied by the host into a Rust slice, or for handing
//! the host a pointer into this module's own heap. That conversion is
//! confined to this file's `extern "C"` exports and the bump allocator
//! below — [`executor`] itself contains no `unsafe` code.
//! `scripts/ci/scoped_unsafe_boundary_check.sh` enforces both the crate-list
//! scope of this exception and this crate's exact exported symbol set.
#![allow(unsafe_code)]

mod app_sm;
mod executor;

use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;

use app_sm::{
    AppInvoke, AppSession, AppStateMachine, AppWait, AppWaitKind, SubmitDiscrimination,
    discriminate_submit, parse_state_machine, resolve_command_transition,
    resolve_lifecycle_transition,
};
use executor::execute_nested_capability;
use traverse_contracts::{
    EventReference, ExecutionTarget, PlacementConstraintEvaluator, PlacementError,
    PlacementRequest, RuntimeSnapshot, ServiceType,
};

/// Traverse Host ABI v1 is independently versioned from this crate (spec
/// `071` FR-006) — unchanged by moving from the WAT fixture to real content.
const BRIDGE_ABI_VERSION: i32 = 10_100;

struct RuntimeState {
    capability_id: String,
    service_type: ServiceType,
    declared_emits: Vec<EventReference>,
    /// Targets this capability may run on (contract `permitted_targets`).
    permitted_targets: Vec<ExecutionTarget>,
    /// Placement target of this `runtime.wasm` instance (the host it is
    /// embedded in). Nested execution only proceeds when the shared
    /// evaluator selects this same target.
    host_placement_target: ExecutionTarget,
    /// Optional caller hint applied on every submit.
    target_hint: Option<ExecutionTarget>,
    runtime_snapshot: RuntimeSnapshot,
    wasm_artifact: Vec<u8>,
    execution_counter: u64,
    pending_events: VecDeque<Vec<u8>>,
    /// Spec 139 app state machine (optional). When present, `app_command`
    /// envelopes are dispatched here instead of as bare capability input.
    app_state_machine: Option<AppStateMachine>,
    /// Process-local Spec 139 sessions (FR-016).
    app_sessions: BTreeMap<String, AppSession>,
    /// Pending host-connector bridge requests awaiting host completion
    /// (drained by the host adapter; populated on `invoke.host_connector`).
    #[allow(dead_code)]
    pending_host_connector: VecDeque<serde_json::Value>,
    /// Pending monotonic deadline registrations for dual-deadline waits
    /// (Spec 139 FR-012); host fires the ceiling callback.
    #[allow(dead_code)]
    pending_deadlines: VecDeque<serde_json::Value>,
}

static STATE: Mutex<Option<RuntimeState>> = Mutex::new(None);

/// `traverse_init`'s payload layout: a 4-byte little-endian header length,
/// that many bytes of UTF-8 JSON metadata, then the raw capability WASM
/// artifact bytes. Chosen over JSON+base64 to avoid a text-encoding
/// dependency for the artifact bytes; the metadata itself stays JSON since
/// it is small and human-inspectable.
struct InitPayload {
    capability_id: String,
    service_type: ServiceType,
    declared_emits: Vec<EventReference>,
    permitted_targets: Vec<ExecutionTarget>,
    host_placement_target: ExecutionTarget,
    target_hint: Option<ExecutionTarget>,
    runtime_snapshot: RuntimeSnapshot,
    wasm_artifact: Vec<u8>,
    app_state_machine: Option<AppStateMachine>,
}

fn parse_execution_target(raw: &str) -> Option<ExecutionTarget> {
    match raw {
        "local" => Some(ExecutionTarget::Local),
        "browser" => Some(ExecutionTarget::Browser),
        "edge" => Some(ExecutionTarget::Edge),
        "cloud" => Some(ExecutionTarget::Cloud),
        "worker" => Some(ExecutionTarget::Worker),
        "device" => Some(ExecutionTarget::Device),
        _ => None,
    }
}

fn default_permitted_targets(host: &ExecutionTarget) -> Vec<ExecutionTarget> {
    // Absent `permitted_targets` means "run here" — matching the default
    // `host_placement_target`. Expanding to every ExecutionTarget would let
    // equal-load heuristics pick Browser (lexicographically first) and fail
    // closed against a Local host.
    vec![host.clone()]
}

fn parse_permitted_targets(
    header: &serde_json::Value,
    host: &ExecutionTarget,
) -> Result<Vec<ExecutionTarget>, String> {
    let Some(entries) = header
        .get("permitted_targets")
        .and_then(serde_json::Value::as_array)
    else {
        return Ok(default_permitted_targets(host));
    };
    if entries.is_empty() {
        return Err("permitted_targets must not be empty when provided".to_string());
    }
    let mut targets = Vec::with_capacity(entries.len());
    for entry in entries {
        let Some(raw) = entry.as_str() else {
            return Err("permitted_targets entries must be strings".to_string());
        };
        let Some(target) = parse_execution_target(raw) else {
            return Err(format!("unknown permitted_targets entry: {raw}"));
        };
        targets.push(target);
    }
    Ok(targets)
}

fn parse_runtime_snapshot(header: &serde_json::Value) -> Result<RuntimeSnapshot, String> {
    let Some(map) = header
        .get("runtime_snapshot")
        .and_then(serde_json::Value::as_object)
    else {
        return Ok(RuntimeSnapshot {
            target_loads: std::collections::HashMap::new(),
        });
    };
    let mut target_loads = std::collections::HashMap::new();
    for (key, value) in map {
        let Some(target) = parse_execution_target(key) else {
            return Err(format!("unknown runtime_snapshot target: {key}"));
        };
        let Some(load) = value.as_f64().and_then(|n| {
            if n.is_finite() && (0.0..=f64::from(f32::MAX)).contains(&n) {
                #[allow(clippy::cast_possible_truncation)]
                {
                    Some(n as f32)
                }
            } else {
                None
            }
        }) else {
            return Err(format!(
                "runtime_snapshot.{key} must be a finite f32-range number"
            ));
        };
        target_loads.insert(target, load);
    }
    Ok(RuntimeSnapshot { target_loads })
}

fn parse_init_payload(bytes: &[u8]) -> Result<InitPayload, String> {
    if bytes.len() < 4 {
        return Err("init payload shorter than the header-length prefix".to_string());
    }
    let header_len = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
    let header_end = 4usize
        .checked_add(header_len)
        .ok_or_else(|| "init header length overflows".to_string())?;
    let header_bytes = bytes
        .get(4..header_end)
        .ok_or_else(|| "init header length exceeds payload".to_string())?;
    let wasm_artifact = bytes.get(header_end..).unwrap_or(&[]).to_vec();

    let header: serde_json::Value =
        serde_json::from_slice(header_bytes).map_err(|error| format!("init header: {error}"))?;
    let app_state_machine = match header.get("state_machine") {
        None | Some(serde_json::Value::Null) => None,
        Some(value) => Some(parse_state_machine(value)?),
    };
    let capability_id = header
        .get("capability_id")
        .or_else(|| header.get("app_id"))
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "init header missing capability_id (or app_id for app mode)".to_string())?
        .to_string();
    let service_type_str = header
        .get("service_type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("stateless");
    let service_type = match service_type_str {
        "subscribable" => ServiceType::Subscribable,
        "stateful" => ServiceType::Stateful,
        _ => ServiceType::Stateless,
    };
    let declared_emits = header
        .get("emits")
        .and_then(serde_json::Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| {
                    let event_id = entry.get("event_id")?.as_str()?.to_string();
                    let version = entry.get("version")?.as_str()?.to_string();
                    Some(EventReference { event_id, version })
                })
                .collect()
        })
        .unwrap_or_default();
    let host_placement_target = header
        .get("host_placement_target")
        .and_then(serde_json::Value::as_str)
        .map(|raw| {
            parse_execution_target(raw)
                .ok_or_else(|| format!("unknown host_placement_target: {raw}"))
        })
        .transpose()?
        .unwrap_or(ExecutionTarget::Local);
    let permitted_targets = parse_permitted_targets(&header, &host_placement_target)?;
    let target_hint = header
        .get("target_hint")
        .and_then(serde_json::Value::as_str)
        .map(|raw| parse_execution_target(raw).ok_or_else(|| format!("unknown target_hint: {raw}")))
        .transpose()?;
    let runtime_snapshot = parse_runtime_snapshot(&header)?;

    Ok(InitPayload {
        capability_id,
        service_type,
        declared_emits,
        permitted_targets,
        host_placement_target,
        target_hint,
        runtime_snapshot,
        wasm_artifact,
        app_state_machine,
    })
}

fn lifecycle_event(kind: &str, session_id: &str, data: &serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "type": kind,
        "session_id": session_id,
        "data": data,
    }))
    .unwrap_or_else(|_| b"{\"type\":\"encode_error\"}".to_vec())
}

/// Result of a `traverse_submit` dispatch.
struct SubmitResult {
    session_id: String,
    status: &'static str,
    error: Option<String>,
}

/// Runs `request` against the initialized capability / app state machine and
/// queues the resulting lifecycle + domain events.
fn submit(state: &mut RuntimeState, request: &[u8]) -> SubmitResult {
    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(request) {
        match discriminate_submit(&value) {
            SubmitDiscrimination::AppCommand(envelope) => {
                return submit_app_command(state, envelope);
            }
            SubmitDiscrimination::Ambiguous => {
                return SubmitResult {
                    session_id: String::new(),
                    status: "rejected",
                    error: Some("ambiguous_submit".to_string()),
                };
            }
            SubmitDiscrimination::Invalid(message) => {
                return SubmitResult {
                    session_id: String::new(),
                    status: "rejected",
                    error: Some(message),
                };
            }
            SubmitDiscrimination::CapabilityOrWorkflow => {}
        }
    }
    submit_capability(state, request)
}

#[allow(clippy::too_many_lines)]
fn submit_app_command(
    state: &mut RuntimeState,
    envelope: app_sm::AppCommandEnvelope,
) -> SubmitResult {
    let Some(machine) = state.app_state_machine.clone() else {
        return SubmitResult {
            session_id: String::new(),
            status: "rejected",
            error: Some("app_state_machine_not_initialized".to_string()),
        };
    };

    state.execution_counter = state.execution_counter.saturating_add(1);
    let (session_id, previous_state) = match envelope.session_id {
        Some(session_id) => {
            let current = state.app_sessions.get(&session_id).map_or_else(
                || machine.initial_state.clone(),
                |session| session.state.clone(),
            );
            (session_id, current)
        }
        None => (
            format!("{}-sess-{}", state.capability_id, state.execution_counter),
            machine.initial_state.clone(),
        ),
    };

    let accepted_state =
        match resolve_command_transition(&machine, &previous_state, &envelope.command) {
            Ok(state_id) => state_id,
            Err(message) => {
                state.pending_events.push_back(lifecycle_event(
                    "error",
                    &session_id,
                    &serde_json::json!({
                        "code": "invalid_transition",
                        "message": message,
                        "state": previous_state,
                    }),
                ));
                return SubmitResult {
                    session_id,
                    status: "rejected",
                    error: Some("invalid_transition".to_string()),
                };
            }
        };

    let command_id = format!("{}-cmd-{}", state.capability_id, state.execution_counter);
    state.pending_events.push_back(lifecycle_event(
        "state_changed",
        &session_id,
        &serde_json::json!({
            "state": accepted_state,
            "previous_state": previous_state,
            "command": envelope.command,
            "command_id": command_id,
        }),
    ));

    let invoke = machine
        .states
        .get(&accepted_state)
        .and_then(|state_def| state_def.invoke.clone());

    match invoke {
        Some(AppInvoke::Capability {
            capability_id,
            input_from: _,
        }) => {
            state.app_sessions.insert(
                session_id.clone(),
                AppSession {
                    session_id: session_id.clone(),
                    state: accepted_state.clone(),
                    wait: Some(AppWait {
                        command_id: command_id.clone(),
                        kind: AppWaitKind::Capability {
                            capability_id: capability_id.clone(),
                        },
                    }),
                },
            );
            run_capability_invoke_wait(
                state,
                &machine,
                &session_id,
                &accepted_state,
                &capability_id,
                &command_id,
                &envelope.payload,
            );
        }
        Some(AppInvoke::HostConnector { command }) => {
            let deadline_ms = 30_000_u64;
            state.pending_host_connector.push_back(serde_json::json!({
                "kind": "host_connector_command",
                "schema_version": "1.0.0",
                "command": command,
                "command_id": command_id,
                "session_id": session_id,
                "payload": envelope.payload,
            }));
            state.pending_deadlines.push_back(serde_json::json!({
                "kind": "deadline_registration",
                "command_id": command_id,
                "session_id": session_id,
                "deadline_ms": deadline_ms,
            }));
            state.app_sessions.insert(
                session_id.clone(),
                AppSession {
                    session_id: session_id.clone(),
                    state: accepted_state,
                    wait: Some(AppWait {
                        command_id,
                        kind: AppWaitKind::HostConnector { command },
                    }),
                },
            );
        }
        None => {
            state.app_sessions.insert(
                session_id.clone(),
                AppSession {
                    session_id: session_id.clone(),
                    state: accepted_state,
                    wait: None,
                },
            );
        }
    }

    SubmitResult {
        session_id,
        status: "accepted",
        error: None,
    }
}

fn run_capability_invoke_wait(
    state: &mut RuntimeState,
    machine: &AppStateMachine,
    session_id: &str,
    invoking_state: &str,
    capability_id: &str,
    command_id: &str,
    payload: &serde_json::Value,
) {
    state.pending_events.push_back(lifecycle_event(
        "capability_invoked",
        session_id,
        &serde_json::json!({
            "capability_id": capability_id,
            "command_id": command_id,
            "state": invoking_state,
        }),
    ));

    let input = serde_json::to_vec(payload).unwrap_or_else(|_| b"{}".to_vec());
    let outcome = execute_nested_capability(
        &state.wasm_artifact,
        &input,
        &state.service_type,
        &state.declared_emits,
    );

    let (lifecycle_event_name, succeeded, output) = match outcome {
        Ok(outcome) => {
            for event in outcome.emitted_events {
                state.pending_events.push_back(lifecycle_event(
                    &event.event_type,
                    session_id,
                    &serde_json::json!({"version": event.version, "payload": event.data}),
                ));
            }
            let stdout_json = serde_json::from_slice::<serde_json::Value>(&outcome.stdout)
                .unwrap_or_else(|_| {
                    serde_json::Value::String(String::from_utf8_lossy(&outcome.stdout).into_owned())
                });
            ("capability_succeeded", true, stdout_json)
        }
        Err(error) => (
            "capability_failed",
            false,
            serde_json::json!({"error": error}),
        ),
    };

    match resolve_lifecycle_transition(machine, invoking_state, lifecycle_event_name) {
        Ok(Some(next_state)) => {
            state.pending_events.push_back(lifecycle_event(
                "state_changed",
                session_id,
                &serde_json::json!({
                    "state": next_state,
                    "previous_state": invoking_state,
                    "command_id": command_id,
                }),
            ));
            if let Some(session) = state.app_sessions.get_mut(session_id) {
                session.state = next_state;
                session.wait = None;
            }
            state.pending_events.push_back(lifecycle_event(
                if succeeded {
                    "capability_result"
                } else {
                    "error"
                },
                session_id,
                &serde_json::json!({
                    "status": if succeeded { "completed" } else { "failed" },
                    "output": output,
                    "capability_id": capability_id,
                }),
            ));
        }
        Ok(None) => {
            state.pending_events.push_back(lifecycle_event(
                "error",
                session_id,
                &serde_json::json!({
                    "code": "no_matching_transition",
                    "message": format!(
                        "no {lifecycle_event_name} transition from '{invoking_state}'"
                    ),
                    "state": invoking_state,
                }),
            ));
            if let Some(session) = state.app_sessions.get_mut(session_id) {
                session.wait = None;
            }
        }
        Err(message) => {
            state.pending_events.push_back(lifecycle_event(
                "error",
                session_id,
                &serde_json::json!({
                    "code": "invalid_state",
                    "message": message,
                }),
            ));
        }
    }
}

/// Capability / workflow stdin path (pre-Spec-139 submit shape).
#[allow(clippy::too_many_lines)]
fn submit_capability(state: &mut RuntimeState, request: &[u8]) -> SubmitResult {
    state.execution_counter += 1;
    let session_id = format!("{}-exec-{}", state.capability_id, state.execution_counter);

    let placement = PlacementConstraintEvaluator.evaluate_targets(
        &PlacementRequest {
            capability_id: state.capability_id.clone(),
            target_hint: state.target_hint.clone(),
            runtime_snapshot: state.runtime_snapshot.clone(),
        },
        &state.permitted_targets,
    );

    match placement {
        Ok(decision) if decision.target != state.host_placement_target => {
            state.pending_events.push_back(lifecycle_event(
                "capability_invoked",
                &session_id,
                &serde_json::json!({
                    "placement_target": format!("{:?}", decision.target),
                    "host_placement_target": format!("{:?}", state.host_placement_target),
                }),
            ));
            state.pending_events.push_back(lifecycle_event(
                "capability_result",
                &session_id,
                &serde_json::json!({
                    "status": "failed",
                    "error": format!(
                        "placement selected {:?}, but this runtime.wasm host is {:?}",
                        decision.target, state.host_placement_target
                    ),
                }),
            ));
            return SubmitResult {
                session_id,
                status: "accepted",
                error: None,
            };
        }
        Err(PlacementError::NoEligibleTarget) => {
            state.pending_events.push_back(lifecycle_event(
                "capability_invoked",
                &session_id,
                &serde_json::json!({}),
            ));
            state.pending_events.push_back(lifecycle_event(
                "capability_result",
                &session_id,
                &serde_json::json!({
                    "status": "failed",
                    "error": "placement failed: NoEligibleTarget",
                }),
            ));
            return SubmitResult {
                session_id,
                status: "accepted",
                error: None,
            };
        }
        Ok(decision) => {
            state.pending_events.push_back(lifecycle_event(
                "capability_invoked",
                &session_id,
                &serde_json::json!({
                    "placement_target": format!("{:?}", decision.target),
                    "placement_reason": format!("{:?}", decision.reason),
                    "placement_confidence": format!("{:?}", decision.confidence),
                }),
            ));
        }
    }

    let outcome = execute_nested_capability(
        &state.wasm_artifact,
        request,
        &state.service_type,
        &state.declared_emits,
    );

    match outcome {
        Ok(outcome) => {
            for event in outcome.emitted_events {
                state.pending_events.push_back(lifecycle_event(
                    &event.event_type,
                    &session_id,
                    &serde_json::json!({"version": event.version, "payload": event.data}),
                ));
            }
            let stdout_json = serde_json::from_slice::<serde_json::Value>(&outcome.stdout)
                .unwrap_or_else(|_| {
                    serde_json::Value::String(String::from_utf8_lossy(&outcome.stdout).into_owned())
                });
            state.pending_events.push_back(lifecycle_event(
                "capability_result",
                &session_id,
                &serde_json::json!({"status": "completed", "output": stdout_json}),
            ));
        }
        Err(error) => {
            state.pending_events.push_back(lifecycle_event(
                "capability_result",
                &session_id,
                &serde_json::json!({"status": "failed", "error": error}),
            ));
        }
    }

    SubmitResult {
        session_id,
        status: "accepted",
        error: None,
    }
}

/// Writes `ptr`/`len` (as two little-endian `i32`s) to the descriptor
/// address the host supplied, matching the WAT fixture's `$out` convention.
///
/// # Safety
///
/// `descriptor` must be a valid, writable address inside this module's own
/// linear memory with at least 8 bytes available — true for any address the
/// host obtained from this module's own `traverse_alloc`/export space, which
/// is the only contract callers of this crate's exports are given.
unsafe fn write_descriptor(descriptor: i32, ptr: i32, len: i32) {
    if descriptor < 0 {
        return;
    }
    let addr = descriptor as *mut i32;
    // SAFETY: caller contract above; `addr` and `addr.add(1)` are 4-byte
    // aligned `i32` slots within a descriptor the host allocated for this
    // exact purpose (spec 071 FR-006's `$out`-shaped response convention).
    unsafe {
        addr.write(ptr);
        addr.add(1).write(len);
    }
}

/// Copies `bytes` into a freshly leaked heap allocation and returns its
/// address, so `write_descriptor` can point the host at it. Freed by a
/// matching `traverse_dealloc` call, or leaked for the process lifetime if
/// the host never calls it — this module's caller (the outer host) owns
/// that lifecycle contract per spec 071 FR-006.
fn leak_bytes(bytes: &[u8]) -> (i32, i32) {
    let boxed: Box<[u8]> = bytes.to_vec().into_boxed_slice();
    let len = i32::try_from(boxed.len()).unwrap_or(i32::MAX);
    let ptr = Box::into_raw(boxed).cast::<u8>() as i32;
    (ptr, len)
}

// ---------------------------------------------------------------------------
// Exported C-ABI (spec 071 FR-006) — the audited unsafe boundary.
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "C" fn traverse_bridge_abi_version() -> i32 {
    BRIDGE_ABI_VERSION
}

#[unsafe(no_mangle)]
pub extern "C" fn traverse_alloc(size: i32) -> i32 {
    if size < 0 {
        return 0;
    }
    let Ok(size) = usize::try_from(size) else {
        return 0;
    };
    let (ptr, _len) = leak_bytes(&vec![0u8; size]);
    ptr
}

#[unsafe(no_mangle)]
pub extern "C" fn traverse_dealloc(ptr: i32, len: i32) {
    if ptr <= 0 || len < 0 {
        return;
    }
    let Ok(len) = usize::try_from(len) else {
        return;
    };
    // SAFETY: `ptr`/`len` must describe a live allocation this module
    // produced via `leak_bytes` (from `traverse_alloc` or an event/response
    // buffer) and not yet freed — the same one-owner contract spec 071
    // FR-006 already requires of every ABI consumer.
    unsafe {
        drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(
            ptr as *mut u8,
            len,
        )));
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn traverse_init(ptr: i32, len: i32, out_descriptor: i32) -> i32 {
    if ptr < 0 || len < 0 {
        return -1;
    }
    let Ok(ptr_u) = usize::try_from(ptr) else {
        return -1;
    };
    let Ok(len_u) = usize::try_from(len) else {
        return -1;
    };
    // SAFETY: the host guarantees `ptr`/`len` describe a readable region of
    // this module's own linear memory for the duration of this call — the
    // same contract every other Rust wasm32 C-ABI export in this codebase
    // relies on (e.g. `traverse-swift-host`, ADR-0015).
    let bytes = unsafe { std::slice::from_raw_parts(ptr_u as *const u8, len_u) };

    let response = match parse_init_payload(bytes) {
        Ok(parsed) => {
            let mut guard = match STATE.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            *guard = Some(RuntimeState {
                capability_id: parsed.capability_id,
                service_type: parsed.service_type,
                declared_emits: parsed.declared_emits,
                permitted_targets: parsed.permitted_targets,
                host_placement_target: parsed.host_placement_target,
                target_hint: parsed.target_hint,
                runtime_snapshot: parsed.runtime_snapshot,
                wasm_artifact: parsed.wasm_artifact,
                execution_counter: 0,
                pending_events: VecDeque::new(),
                app_state_machine: parsed.app_state_machine,
                app_sessions: BTreeMap::new(),
                pending_host_connector: VecDeque::new(),
                pending_deadlines: VecDeque::new(),
            });
            serde_json::json!({"status": "ready", "error": null})
        }
        Err(error) => serde_json::json!({"status": "error", "error": error}),
    };
    let ok = response
        .get("error")
        .is_some_and(serde_json::Value::is_null);
    let bytes = serde_json::to_vec(&response).unwrap_or_default();
    let (out_ptr, out_len) = leak_bytes(&bytes);
    // SAFETY: `out_descriptor` is the caller-supplied response slot per
    // spec 071 FR-006; `write_descriptor`'s own contract covers the rest.
    unsafe {
        write_descriptor(out_descriptor, out_ptr, out_len);
    }
    if ok { 0 } else { -1 }
}

#[unsafe(no_mangle)]
pub extern "C" fn traverse_submit(ptr: i32, len: i32, out_descriptor: i32) -> i32 {
    if ptr < 0 || len < 0 {
        return -1;
    }
    let Ok(ptr_u) = usize::try_from(ptr) else {
        return -1;
    };
    let Ok(len_u) = usize::try_from(len) else {
        return -1;
    };
    // SAFETY: see `traverse_init` — identical host contract.
    let request = unsafe { std::slice::from_raw_parts(ptr_u as *const u8, len_u) }.to_vec();

    let mut guard = match STATE.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    let response = match guard.as_mut() {
        None => serde_json::json!({"status": "error", "error": "not initialized"}),
        Some(state) => {
            let result = submit(state, &request);
            match result.error {
                Some(error) => serde_json::json!({
                    "session_id": result.session_id,
                    "status": result.status,
                    "error": error,
                }),
                None => serde_json::json!({
                    "session_id": result.session_id,
                    "status": result.status,
                    "error": null,
                }),
            }
        }
    };
    let ok = response
        .get("error")
        .is_some_and(serde_json::Value::is_null);
    let bytes = serde_json::to_vec(&response).unwrap_or_default();
    let (out_ptr, out_len) = leak_bytes(&bytes);
    // SAFETY: see `traverse_init`.
    unsafe {
        write_descriptor(out_descriptor, out_ptr, out_len);
    }
    if ok { 0 } else { -1 }
}

#[unsafe(no_mangle)]
pub extern "C" fn traverse_next_event(out_descriptor: i32) -> i32 {
    let mut guard = match STATE.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    let Some(state) = guard.as_mut() else {
        return 0;
    };
    let Some(event_bytes) = state.pending_events.pop_front() else {
        return 0;
    };
    let (out_ptr, out_len) = leak_bytes(&event_bytes);
    // SAFETY: see `traverse_init`.
    unsafe {
        write_descriptor(out_descriptor, out_ptr, out_len);
    }
    1
}

#[unsafe(no_mangle)]
pub extern "C" fn traverse_cancel(_a: i32, _b: i32, _c: i32) -> i32 {
    // Cooperative cancellation of an in-flight nested execution is deferred
    // (issue #1419, durable EventBroker parity) — `traverse_submit` above
    // already runs synchronously to completion, so there is never an
    // in-flight execution for this to interrupt today.
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn traverse_shutdown(out_descriptor: i32) -> i32 {
    let mut guard = match STATE.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    *guard = None;
    let bytes = serde_json::to_vec(&serde_json::json!({"status": "stopped"})).unwrap_or_default();
    let (out_ptr, out_len) = leak_bytes(&bytes);
    // SAFETY: see `traverse_init`.
    unsafe {
        write_descriptor(out_descriptor, out_ptr, out_len);
    }
    0
}

// Deprecated since the real `runtime-wasm-bridge/1.0.0` lifecycle
// (init/submit/next_event/cancel/shutdown) superseded them; kept only so
// the exported symbol set matches spec 071 FR-006 exactly.
#[unsafe(no_mangle)]
pub extern "C" fn traverse_compatible_start(_a: i32, _b: i32, _c: i32) -> i32 {
    -1
}

#[unsafe(no_mangle)]
pub extern "C" fn traverse_compatible_stop(_a: i32, _b: i32, _c: i32) -> i32 {
    -1
}

#[unsafe(no_mangle)]
pub extern "C" fn traverse_compatible_kill(_a: i32, _b: i32, _c: i32) -> i32 {
    -1
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]

    use super::*;

    fn init_payload_bytes(capability_id: &str, service_type: &str, artifact: &[u8]) -> Vec<u8> {
        let header = serde_json::json!({
            "capability_id": capability_id,
            "capability_version": "1.0.0",
            "service_type": service_type,
            "emits": [{"event_id": "order.placed", "version": "1.0.0"}],
        });
        let header_bytes = serde_json::to_vec(&header).unwrap_or_default();
        let header_len = u32::try_from(header_bytes.len()).unwrap_or(u32::MAX);
        let mut payload = header_len.to_le_bytes().to_vec();
        payload.extend_from_slice(&header_bytes);
        payload.extend_from_slice(artifact);
        payload
    }

    #[test]
    fn parse_init_payload_round_trips_metadata_and_artifact() {
        let artifact = b"\0asm-fake-bytes";
        let bytes = init_payload_bytes("example.echo", "subscribable", artifact);
        let parsed = parse_init_payload(&bytes).expect("must parse");
        assert_eq!(parsed.capability_id, "example.echo");
        assert_eq!(parsed.service_type, ServiceType::Subscribable);
        assert_eq!(parsed.declared_emits.len(), 1);
        assert_eq!(parsed.wasm_artifact, artifact);
    }

    #[test]
    fn parse_init_payload_rejects_truncated_header() {
        let bytes = vec![255, 0, 0, 0, 1, 2];
        assert!(parse_init_payload(&bytes).is_err());
    }

    #[test]
    fn parse_init_payload_rejects_payload_shorter_than_length_prefix() {
        assert!(parse_init_payload(&[0, 1, 2]).is_err());
    }

    #[test]
    fn parse_init_payload_rejects_header_length_overflow() {
        let bytes = u32::MAX.to_le_bytes().to_vec();
        assert!(parse_init_payload(&bytes).is_err());
    }

    #[test]
    fn parse_init_payload_rejects_malformed_json_header() {
        let mut bytes = 3u32.to_le_bytes().to_vec();
        bytes.extend_from_slice(b"{no");
        assert!(parse_init_payload(&bytes).is_err());
    }

    #[test]
    fn parse_init_payload_rejects_header_missing_capability_id() {
        let header = serde_json::to_vec(&serde_json::json!({"service_type": "stateless"}))
            .unwrap_or_default();
        let header_len = u32::try_from(header.len()).unwrap_or(u32::MAX);
        let mut bytes = header_len.to_le_bytes().to_vec();
        bytes.extend_from_slice(&header);
        assert!(parse_init_payload(&bytes).is_err());
    }

    #[test]
    fn parse_init_payload_defaults_unknown_service_type_to_stateless() {
        let bytes = init_payload_bytes("example.echo", "not-a-real-type", b"");
        let parsed = parse_init_payload(&bytes).expect("must parse");
        assert_eq!(parsed.service_type, ServiceType::Stateless);
    }

    #[test]
    fn parse_init_payload_accepts_stateful_service_type() {
        let bytes = init_payload_bytes("example.echo", "stateful", b"");
        let parsed = parse_init_payload(&bytes).expect("must parse");
        assert_eq!(parsed.service_type, ServiceType::Stateful);
    }

    #[test]
    fn parse_init_payload_defaults_declared_emits_when_absent() {
        let header = serde_json::to_vec(&serde_json::json!({"capability_id": "example.echo"}))
            .unwrap_or_default();
        let header_len = u32::try_from(header.len()).unwrap_or(u32::MAX);
        let mut bytes = header_len.to_le_bytes().to_vec();
        bytes.extend_from_slice(&header);
        let parsed = parse_init_payload(&bytes).expect("must parse");
        assert!(parsed.declared_emits.is_empty());
    }

    #[test]
    fn leak_bytes_reports_the_correct_length() {
        // Only the length is checked here, not a pointer round trip:
        // `leak_bytes` truncates a real (64-bit, on this host) pointer to
        // the `i32` spec 071's ABI requires, which is only a valid address
        // on an actual `wasm32` target's 32-bit address space. Reconstructing
        // a `Box` from that truncated value on a native test host is
        // undefined behavior (verified — it segfaults); the real round trip
        // is exercised by the `wasm32-unknown-unknown` build in CI and by
        // the cross-host conformance run (#1420), not a native unit test.
        let payload = b"hello runtime.wasm".to_vec();
        let (_ptr, len) = leak_bytes(&payload);
        assert_eq!(usize::try_from(len).unwrap_or(0), payload.len());
    }

    #[test]
    fn write_descriptor_ignores_a_negative_descriptor() {
        // SAFETY: a negative descriptor is rejected before any pointer
        // arithmetic — this call must not read or write memory.
        unsafe {
            write_descriptor(-1, 42, 7);
        }
    }

    /// Full lifecycle through the safe `submit` function (not the raw ABI
    /// exports — those are thin pointer marshalling covered by code review
    /// and the real cross-host conformance run, issue #1420): init state,
    /// submit against a capability that emits a declared event, drain the
    /// resulting lifecycle + domain events.
    #[test]
    fn submit_queues_invoked_domain_and_result_events_in_order() {
        const ECHO_AND_EMIT_WAT: &str = r#"
          (module
            (import "wasi_snapshot_preview1" "fd_read"
              (func $fd_read (param i32 i32 i32 i32) (result i32)))
            (import "wasi_snapshot_preview1" "fd_write"
              (func $fd_write (param i32 i32 i32 i32) (result i32)))
            (import "traverse_host" "emit_event"
              (func $emit_event (param i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 5000) "{\"event_id\":\"order.placed\",\"version\":\"1.0.0\",\"payload\":{}}")
            (func (export "_start")
              (i32.store (i32.const 0) (i32.const 8))
              (i32.store (i32.const 4) (i32.const 1024))
              (drop (call $fd_read (i32.const 0) (i32.const 0) (i32.const 1) (i32.const 4100)))
              (i32.store (i32.const 0) (i32.const 8))
              (i32.store (i32.const 4) (i32.load (i32.const 4100)))
              (drop (call $fd_write (i32.const 1) (i32.const 0) (i32.const 1) (i32.const 4104)))
              (drop (call $emit_event (i32.const 5000) (i32.const 58)))
            )
          )
        "#;
        let artifact = wat::parse_str(ECHO_AND_EMIT_WAT).expect("wat parses");

        let mut state = RuntimeState {
            capability_id: "example.echo".to_string(),
            service_type: ServiceType::Subscribable,
            declared_emits: vec![EventReference {
                event_id: "order.placed".to_string(),
                version: "1.0.0".to_string(),
            }],
            permitted_targets: vec![ExecutionTarget::Local],
            host_placement_target: ExecutionTarget::Local,
            target_hint: None,
            runtime_snapshot: RuntimeSnapshot::default(),
            wasm_artifact: artifact,
            execution_counter: 0,
            pending_events: VecDeque::new(),
            app_state_machine: None,
            app_sessions: BTreeMap::new(),
            pending_host_connector: VecDeque::new(),
            pending_deadlines: VecDeque::new(),
        };

        submit(&mut state, br#"{"ok":true}"#);

        assert_eq!(state.pending_events.len(), 3);
        let invoked: serde_json::Value =
            serde_json::from_slice(&state.pending_events[0]).expect("valid json");
        assert_eq!(invoked["type"], "capability_invoked");
        assert_eq!(invoked["data"]["placement_target"], "Local");
        let domain: serde_json::Value =
            serde_json::from_slice(&state.pending_events[1]).expect("valid json");
        assert_eq!(domain["type"], "order.placed");
        let result: serde_json::Value =
            serde_json::from_slice(&state.pending_events[2]).expect("valid json");
        assert_eq!(result["type"], "capability_result");
        assert_eq!(result["data"]["status"], "completed");
    }

    #[test]
    fn submit_fails_when_placement_has_no_eligible_target() {
        let mut state = RuntimeState {
            capability_id: "example.echo".to_string(),
            service_type: ServiceType::Stateless,
            declared_emits: Vec::new(),
            permitted_targets: vec![ExecutionTarget::Local, ExecutionTarget::Cloud],
            host_placement_target: ExecutionTarget::Local,
            target_hint: None,
            runtime_snapshot: RuntimeSnapshot {
                target_loads: [
                    (ExecutionTarget::Local, 0.95),
                    (ExecutionTarget::Cloud, 0.95),
                ]
                .into_iter()
                .collect(),
            },
            wasm_artifact: Vec::new(),
            execution_counter: 0,
            pending_events: VecDeque::new(),
            app_state_machine: None,
            app_sessions: BTreeMap::new(),
            pending_host_connector: VecDeque::new(),
            pending_deadlines: VecDeque::new(),
        };

        submit(&mut state, br"{}");
        assert_eq!(state.pending_events.len(), 2);
        let result: serde_json::Value =
            serde_json::from_slice(&state.pending_events[1]).expect("valid json");
        assert_eq!(result["data"]["status"], "failed");
        assert!(
            result["data"]["error"]
                .as_str()
                .unwrap_or("")
                .contains("NoEligibleTarget")
        );
    }

    #[test]
    fn submit_fails_when_placement_selects_non_host_target() {
        let mut state = RuntimeState {
            capability_id: "example.echo".to_string(),
            service_type: ServiceType::Stateless,
            declared_emits: Vec::new(),
            permitted_targets: vec![ExecutionTarget::Local, ExecutionTarget::Cloud],
            host_placement_target: ExecutionTarget::Local,
            target_hint: Some(ExecutionTarget::Cloud),
            runtime_snapshot: RuntimeSnapshot::default(),
            wasm_artifact: Vec::new(),
            execution_counter: 0,
            pending_events: VecDeque::new(),
            app_state_machine: None,
            app_sessions: BTreeMap::new(),
            pending_host_connector: VecDeque::new(),
            pending_deadlines: VecDeque::new(),
        };

        submit(&mut state, br"{}");
        assert_eq!(state.pending_events.len(), 2);
        let result: serde_json::Value =
            serde_json::from_slice(&state.pending_events[1]).expect("valid json");
        assert_eq!(result["data"]["status"], "failed");
        assert!(
            result["data"]["error"]
                .as_str()
                .unwrap_or("")
                .contains("placement selected Cloud")
        );
    }

    #[test]
    fn parse_init_payload_reads_placement_fields() {
        let header = serde_json::json!({
            "capability_id": "example.echo",
            "permitted_targets": ["local", "cloud"],
            "host_placement_target": "browser",
            "target_hint": "local",
            "runtime_snapshot": {"local": 0.2, "cloud": 0.8},
        });
        let header_bytes = serde_json::to_vec(&header).unwrap();
        let header_len = u32::try_from(header_bytes.len()).unwrap();
        let mut bytes = header_len.to_le_bytes().to_vec();
        bytes.extend_from_slice(&header_bytes);
        let parsed = parse_init_payload(&bytes).expect("must parse");
        assert_eq!(
            parsed.permitted_targets,
            vec![ExecutionTarget::Local, ExecutionTarget::Cloud]
        );
        assert_eq!(parsed.host_placement_target, ExecutionTarget::Browser);
        assert_eq!(parsed.target_hint, Some(ExecutionTarget::Local));
        assert_eq!(
            parsed
                .runtime_snapshot
                .target_loads
                .get(&ExecutionTarget::Local),
            Some(&0.2)
        );
    }

    #[test]
    fn app_command_submit_transitions_and_invokes_nested_capability() {
        const ECHO_WAT: &str = r#"
          (module
            (import "wasi_snapshot_preview1" "fd_read"
              (func $fd_read (param i32 i32 i32 i32) (result i32)))
            (import "wasi_snapshot_preview1" "fd_write"
              (func $fd_write (param i32 i32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (func (export "_start")
              (i32.store (i32.const 0) (i32.const 8))
              (i32.store (i32.const 4) (i32.const 1024))
              (drop (call $fd_read (i32.const 0) (i32.const 0) (i32.const 1) (i32.const 4100)))
              (i32.store (i32.const 0) (i32.const 8))
              (i32.store (i32.const 4) (i32.load (i32.const 4100)))
              (drop (call $fd_write (i32.const 1) (i32.const 0) (i32.const 1) (i32.const 4104)))
            )
          )
        "#;
        let artifact = wat::parse_str(ECHO_WAT).expect("wat parses");
        let machine = parse_state_machine(&serde_json::json!({
            "initial_state": "idle",
            "states": [
                {
                    "id": "idle",
                    "transitions": [{ "on": "submit", "to": "processing" }]
                },
                {
                    "id": "processing",
                    "invoke": {
                        "capability_id": "example.echo",
                        "input_from": "command.payload"
                    },
                    "transitions": [
                        { "on": "capability_succeeded", "to": "done" },
                        { "on": "capability_failed", "to": "error" }
                    ]
                },
                { "id": "done", "transitions": [] },
                { "id": "error", "transitions": [] }
            ]
        }))
        .expect("machine");

        let mut state = RuntimeState {
            capability_id: "demo.app".to_string(),
            service_type: ServiceType::Stateless,
            declared_emits: Vec::new(),
            permitted_targets: vec![ExecutionTarget::Local],
            host_placement_target: ExecutionTarget::Local,
            target_hint: None,
            runtime_snapshot: RuntimeSnapshot::default(),
            wasm_artifact: artifact,
            execution_counter: 0,
            pending_events: VecDeque::new(),
            app_state_machine: Some(machine),
            app_sessions: BTreeMap::new(),
            pending_host_connector: VecDeque::new(),
            pending_deadlines: VecDeque::new(),
        };

        let result = submit(
            &mut state,
            br#"{"kind":"app_command","command":"submit","payload":{"hello":"sm"}}"#,
        );
        assert_eq!(result.status, "accepted");
        assert!(!result.session_id.is_empty());

        let types: Vec<String> = state
            .pending_events
            .iter()
            .map(|bytes| {
                serde_json::from_slice::<serde_json::Value>(bytes).expect("json")["type"]
                    .as_str()
                    .unwrap_or("")
                    .to_string()
            })
            .collect();
        assert!(types.contains(&"state_changed".to_string()));
        assert!(types.contains(&"capability_invoked".to_string()));
        assert!(types.contains(&"capability_result".to_string()));
        assert_eq!(
            state
                .app_sessions
                .get(&result.session_id)
                .map(|s| s.state.as_str()),
            Some("done")
        );
    }
}
