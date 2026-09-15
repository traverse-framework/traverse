//! Feasibility spike for Traverse #1403 / Spec `1402` Phase 1.
//!
//! Proves that wasmi 2.0.0 with a wasm32-oriented feature set can host a
//! Traverse-shaped WASI command capability (stdin JSON → stdout JSON) using
//! the same hand-linked preview1 surface as the Apple host tests and the
//! browser WASI shim.
//!
//! The outer module is intentionally free of ambient WASI imports (spec
//! `071` FR-008). Browser `WebAssembly.instantiate` of a cdylib that exports
//! a C ABI for artifact bytes is deferred: workspace policy permits
//! `unsafe_code` only in `traverse-swift-host` (Spec 076). This spike proves
//! the interpreter + feature matrix + I/O convention; a follow-up can add an
//! audited export boundary once nested execution is accepted.
//!
//! Not production quality — measure viability before Phase 2.

#![deny(unsafe_code)]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use wasmi::{Caller, Config, Engine, Extern, Linker, Module, Store, StoreLimitsBuilder};

const WASI_ERRNO_SUCCESS: i32 = 0;
const WASI_ERRNO_BADF: i32 = 8;
const WASI_ERRNO_INVAL: i32 = 28;
const FUEL: u64 = 10_000_000;
const MAX_MEMORY_BYTES: usize = 16 * 1024 * 1024;

struct WasiState {
    stdin: Vec<u8>,
    stdin_offset: usize,
    stdout: Vec<u8>,
    limits: wasmi::StoreLimits,
}

fn read_memory(caller: &Caller<'_, WasiState>, ptr: i32, len: i32) -> Result<Vec<u8>, i32> {
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
    let mut buffer = alloc::vec![0u8; len];
    if memory.read(caller, ptr, &mut buffer).is_err() {
        return Err(WASI_ERRNO_INVAL);
    }
    Ok(buffer)
}

fn write_memory(caller: &mut Caller<'_, WasiState>, ptr: i32, bytes: &[u8]) -> Result<(), i32> {
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
    mut caller: Caller<'_, WasiState>,
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
    mut caller: Caller<'_, WasiState>,
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

fn wasi_proc_exit(_caller: Caller<'_, WasiState>, _code: i32) {
    // Soft no-op for the spike; non-zero exits are out of scope for echo fixtures.
}

/// Execute `artifact` as a WASI command module with `input` as stdin JSON bytes.
///
/// # Errors
///
/// Returns a stable, secret-free error string when module validation, linking,
/// fuel setup, or execution fails.
pub fn execute_capability(artifact: &[u8], input: &[u8]) -> Result<Vec<u8>, String> {
    let mut config = Config::default();
    config.consume_fuel(true);
    let engine = Engine::new(&config);
    let module = Module::new(&engine, artifact).map_err(|error| format!("module: {error}"))?;
    let limits = StoreLimitsBuilder::new()
        .memory_size(MAX_MEMORY_BYTES)
        .trap_on_grow_failure(true)
        .build();
    let state = WasiState {
        stdin: input.to_vec(),
        stdin_offset: 0,
        stdout: Vec::new(),
        limits,
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
    let instance = linker
        .instantiate_and_start(&mut store, &module)
        .map_err(|error| format!("instantiate: {error}"))?;
    let start = instance
        .get_typed_func::<(), ()>(&store, "_start")
        .map_err(|error| format!("missing _start: {error}"))?;
    start
        .call(&mut store, ())
        .map_err(|error| format!("execution: {error}"))?;
    Ok(store.into_data().stdout)
}

#[cfg(test)]
mod tests {
    use super::execute_capability;

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

    #[test]
    fn nested_wasmi_echoes_stdin_to_stdout() -> Result<(), String> {
        let artifact = wat::parse_str(ECHO_WAT).map_err(|error| format!("wat: {error}"))?;
        let input = br#"{"hello":"nested"}"#;
        let output = execute_capability(&artifact, input)?;
        if output != input {
            return Err("stdout mismatch".to_string());
        }
        Ok(())
    }
}
