//! Nested-wasmi capability execution (Spec `1402` FR-003/FR-011, Decision 87).
//!
//! This module contains no `unsafe` code — it operates entirely on safe
//! `wasmi` APIs and byte slices. The unavoidable raw-pointer C-ABI boundary
//! (spec `071` FR-006) lives only in `lib.rs`'s exported `extern "C"`
//! functions, which convert guest pointers to slices before calling in here.
//!
//! Mirrors `crates/traverse-runtime/src/executor/wasm.rs`'s Wasmtime-backed
//! `WasmExecutor`: input via WASI `stdin`, output via WASI `stdout`, and a
//! `traverse_host::emit_event` import validated by the same shared core
//! (`traverse_contracts::validate_emit_event`) the native executor calls —
//! not a second implementation of that logic (spec `1402` FR-005).

use traverse_contracts::{EventReference, ServiceType, validate_emit_event};
use wasmi::{Caller, Config, Engine, Extern, Linker, Module, Store, StoreLimitsBuilder};

const WASI_ERRNO_SUCCESS: i32 = 0;
const WASI_ERRNO_BADF: i32 = 8;
const WASI_ERRNO_INVAL: i32 = 28;
const FUEL: u64 = 10_000_000;
/// Default nested linear-memory ceiling (Spec 1402 FR-012 / Spec 139 FR-018).
///
/// Matches native `WasmExecutor` / issue `#1336` so certified registry
/// planners that reserve ~273 pages (~17 MiB) initial memory can instantiate
/// inside `runtime.wasm` (for example
/// `core.create-audio-capture-request-plan@1.0.0`).
const MAX_MEMORY_BYTES: usize = 32 * 1024 * 1024;

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
}

struct NestedStoreState {
    stdin: Vec<u8>,
    stdin_offset: usize,
    stdout: Vec<u8>,
    limits: wasmi::StoreLimits,
    service_type: ServiceType,
    declared_emits: Vec<EventReference>,
    emitted_events: Vec<EmittedEvent>,
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

fn wasi_proc_exit(_caller: Caller<'_, NestedStoreState>, _code: i32) {
    // Matches the native executor: a non-zero guest exit does not itself
    // fail the host call — the caller inspects stdout/exit status separately.
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

/// Executes `artifact` as a WASI-command-shaped capability with `input` as
/// stdin JSON bytes, inside a nested `wasmi` engine — the same engine and
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
    let mut config = Config::default();
    config.consume_fuel(true);
    let engine = Engine::new(&config);
    let module = Module::new(&engine, artifact).map_err(|error| format!("module: {error}"))?;
    let limits = StoreLimitsBuilder::new()
        .memory_size(MAX_MEMORY_BYTES)
        .trap_on_grow_failure(true)
        .build();
    let state = NestedStoreState {
        stdin: input.to_vec(),
        stdin_offset: 0,
        stdout: Vec::new(),
        limits,
        service_type: service_type.clone(),
        declared_emits: declared_emits.to_vec(),
        emitted_events: Vec::new(),
    };
    let mut store = Store::new(&engine, state);
    store.limiter(|state| &mut state.limits);
    store
        .set_fuel(FUEL)
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

    let instance = linker
        .instantiate_and_start(&mut store, &module)
        .map_err(|error| format!("instantiate: {error}"))?;
    let start = instance
        .get_typed_func::<(), ()>(&store, "_start")
        .map_err(|error| format!("missing _start: {error}"))?;
    start
        .call(&mut store, ())
        .map_err(|error| format!("execution: {error}"))?;

    let final_state = store.into_data();
    Ok(NestedExecutionOutcome {
        stdout: final_state.stdout,
        emitted_events: final_state.emitted_events,
    })
}

#[cfg(test)]
mod tests {
    use super::{EmittedEvent, execute_nested_capability};
    use traverse_contracts::{EventReference, ServiceType};

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
}
