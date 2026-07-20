//! Audited five-symbol C ABI for the bounded Apple `wasmi` bridge.
//!
//! All raw-pointer conversion is intentionally confined to this file. The
//! opaque handle owns a `wasmi` store and cannot grant filesystem, network,
//! environment, clock, process, or arbitrary-import authority.

#![allow(unsafe_code)] // Audited C-ABI exception; see ADR-0015 and Spec 076.

use sha2::{Digest, Sha256};
use wasmi::{
    Config, Engine, Instance, Linker, Memory, Module, Store, StoreLimits, StoreLimitsBuilder,
};

const ABI_VERSION: u32 = 2;
const OK: i32 = 0;
const INVALID_HANDLE: i32 = -1;
const INVALID_INPUT: i32 = -2;
const INVALID_DESCRIPTOR: i32 = -3;
const RESOURCE_LIMIT: i32 = -4;
const INTERNAL_ERROR: i32 = -5;
const BUFFER_TOO_SMALL: i32 = -6;
const BRIDGE_VERSION: i32 = 10_100;
const MAX_ERROR_BYTES: usize = 512;

#[repr(C)]
pub struct TraverseSwiftHostLimits {
    maximum_artifact_bytes: u64,
    maximum_memory_bytes: u64,
    fuel_per_invocation: u64,
    maximum_input_bytes: u64,
    maximum_output_bytes: u64,
    maximum_queued_events: u64,
}

impl TraverseSwiftHostLimits {
    fn checked(&self) -> Result<Limits, HostError> {
        let values = [
            self.maximum_artifact_bytes,
            self.maximum_memory_bytes,
            self.fuel_per_invocation,
            self.maximum_input_bytes,
            self.maximum_output_bytes,
            self.maximum_queued_events,
        ];
        if values.contains(&0) {
            return Err(HostError::new(RESOURCE_LIMIT, "bridge_resource_limit"));
        }
        Ok(Limits {
            maximum_artifact_bytes: usize::try_from(self.maximum_artifact_bytes)
                .map_err(|_| HostError::new(RESOURCE_LIMIT, "bridge_resource_limit"))?,
            maximum_memory_bytes: usize::try_from(self.maximum_memory_bytes)
                .map_err(|_| HostError::new(RESOURCE_LIMIT, "bridge_resource_limit"))?,
            fuel_per_invocation: self.fuel_per_invocation,
            maximum_input_bytes: usize::try_from(self.maximum_input_bytes)
                .map_err(|_| HostError::new(RESOURCE_LIMIT, "bridge_resource_limit"))?,
            maximum_output_bytes: usize::try_from(self.maximum_output_bytes)
                .map_err(|_| HostError::new(RESOURCE_LIMIT, "bridge_resource_limit"))?,
        })
    }
}

struct Limits {
    maximum_artifact_bytes: usize,
    maximum_memory_bytes: usize,
    fuel_per_invocation: u64,
    maximum_input_bytes: usize,
    maximum_output_bytes: usize,
}

struct Host {
    store: Store<StoreLimits>,
    instance: Instance,
    memory: Memory,
    limits: Limits,
}

struct HostError {
    status: i32,
    code: &'static str,
}

impl HostError {
    const fn new(status: i32, code: &'static str) -> Self {
        Self { status, code }
    }
    fn json(&self) -> Vec<u8> {
        format!(
            r#"{{\"code\":\"{}\",\"message\":\"{}\",\"details\":{{}}}}"#,
            self.code, self.code
        )
        .into_bytes()
    }
}

fn required_operation(operation: &str) -> bool {
    matches!(
        operation,
        "init"
            | "submit"
            | "next_event"
            | "cancel"
            | "compatible_start"
            | "compatible_stop"
            | "compatible_kill"
            | "shutdown"
    )
}

fn export_name(operation: &str) -> &'static str {
    match operation {
        "init" => "traverse_init",
        "submit" => "traverse_submit",
        "next_event" => "traverse_next_event",
        "cancel" => "traverse_cancel",
        "compatible_start" => "traverse_compatible_start",
        "compatible_stop" => "traverse_compatible_stop",
        "compatible_kill" => "traverse_compatible_kill",
        "shutdown" => "traverse_shutdown",
        _ => "",
    }
}

fn digest(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let hex = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("sha256:{hex}")
}

fn normalised_digest(bytes: &[u8]) -> Result<String, HostError> {
    let value = std::str::from_utf8(bytes)
        .map_err(|_| HostError::new(INVALID_INPUT, "bundle_digest_mismatch"))?;
    let value = value.trim().to_ascii_lowercase();
    Ok(if value.starts_with("sha256:") {
        value
    } else {
        format!("sha256:{value}")
    })
}

fn require_exports(
    store: &mut Store<StoreLimits>,
    instance: &Instance,
) -> Result<Memory, HostError> {
    let memory = instance
        .get_memory(&*store, "memory")
        .ok_or_else(|| HostError::new(INVALID_DESCRIPTOR, "bridge_invalid_descriptor"))?;
    let version = instance
        .get_typed_func::<(), i32>(&*store, "traverse_bridge_abi_version")
        .map_err(|_| HostError::new(INVALID_DESCRIPTOR, "bridge_invalid_descriptor"))?;
    if version
        .call(&mut *store, ())
        .map_err(|_| HostError::new(INTERNAL_ERROR, "bridge_trap"))?
        != BRIDGE_VERSION
    {
        return Err(HostError::new(INVALID_INPUT, "bridge_version_mismatch"));
    }
    for name in [
        "traverse_alloc",
        "traverse_dealloc",
        "traverse_init",
        "traverse_submit",
        "traverse_next_event",
        "traverse_cancel",
        "traverse_compatible_start",
        "traverse_compatible_stop",
        "traverse_compatible_kill",
        "traverse_shutdown",
    ] {
        if instance.get_func(&*store, name).is_none() {
            return Err(HostError::new(
                INVALID_DESCRIPTOR,
                "bridge_invalid_descriptor",
            ));
        }
    }
    Ok(memory)
}

fn create_host(runtime: &[u8], expected_digest: &[u8], limits: Limits) -> Result<Host, HostError> {
    if runtime.len() > limits.maximum_artifact_bytes {
        return Err(HostError::new(RESOURCE_LIMIT, "bridge_resource_limit"));
    }
    if normalised_digest(expected_digest)? != digest(runtime) {
        return Err(HostError::new(INVALID_INPUT, "bundle_digest_mismatch"));
    }
    let mut config = Config::default();
    config.consume_fuel(true);
    let engine = Engine::new(&config);
    let module = Module::new(&engine, runtime)
        .map_err(|_| HostError::new(INVALID_INPUT, "bridge_invalid_module"))?;
    if module.imports().next().is_some() {
        return Err(HostError::new(INVALID_INPUT, "bridge_ambient_import"));
    }
    let store_limits = StoreLimitsBuilder::new()
        .memory_size(limits.maximum_memory_bytes)
        .trap_on_grow_failure(true)
        .build();
    let mut store = Store::new(&engine, store_limits);
    store.limiter(|value| value);
    let instance = Linker::new(&engine)
        .instantiate_and_start(&mut store, &module)
        .map_err(|_| HostError::new(INTERNAL_ERROR, "bridge_trap"))?;
    let memory = require_exports(&mut store, &instance)?;
    Ok(Host {
        store,
        instance,
        memory,
        limits,
    })
}

fn output(result: &[u8], buffer: *mut u8, capacity: usize, length_out: *mut usize) -> i32 {
    // SAFETY: callers validate `length_out` before this helper; its one write is bounded to that object.
    unsafe {
        *length_out = result.len();
    }
    if result.len() > capacity {
        return BUFFER_TOO_SMALL;
    }
    if result.is_empty() {
        return OK;
    }
    if buffer.is_null() {
        return INVALID_DESCRIPTOR;
    }
    // SAFETY: the caller promises a writable output region of `capacity`; this copy is limited to `result.len() <= capacity`.
    unsafe {
        std::ptr::copy_nonoverlapping(result.as_ptr(), buffer, result.len());
    }
    OK
}

fn invoke(host: &mut Host, operation: &str, input: &[u8]) -> Result<Vec<u8>, HostError> {
    if !required_operation(operation) {
        return Err(HostError::new(
            INVALID_INPUT,
            "bridge_operation_not_allowed",
        ));
    }
    if input.len() > host.limits.maximum_input_bytes {
        return Err(HostError::new(RESOURCE_LIMIT, "bridge_resource_limit"));
    }
    if !input.is_empty() {
        serde_json::from_slice::<serde_json::Value>(input)
            .map_err(|_| HostError::new(INVALID_INPUT, "bridge_invalid_json"))?;
    }
    host.store
        .set_fuel(host.limits.fuel_per_invocation)
        .map_err(|_| HostError::new(RESOURCE_LIMIT, "bridge_resource_limit"))?;
    let alloc = host
        .instance
        .get_typed_func::<i32, i32>(&host.store, "traverse_alloc")
        .map_err(|_| HostError::new(INTERNAL_ERROR, "bridge_trap"))?;
    let dealloc = host
        .instance
        .get_typed_func::<(i32, i32), ()>(&host.store, "traverse_dealloc")
        .map_err(|_| HostError::new(INTERNAL_ERROR, "bridge_trap"))?;
    let descriptor = alloc
        .call(&mut host.store, 8)
        .map_err(|_| HostError::new(INTERNAL_ERROR, "bridge_trap"))?;
    let input_pointer = if input.is_empty() {
        0
    } else {
        alloc
            .call(
                &mut host.store,
                i32::try_from(input.len())
                    .map_err(|_| HostError::new(RESOURCE_LIMIT, "bridge_resource_limit"))?,
            )
            .map_err(|_| HostError::new(INTERNAL_ERROR, "bridge_trap"))?
    };
    if !input.is_empty() {
        host.memory
            .write(
                &mut host.store,
                usize::try_from(input_pointer)
                    .map_err(|_| HostError::new(INVALID_DESCRIPTOR, "bridge_invalid_descriptor"))?,
                input,
            )
            .map_err(|_| HostError::new(INVALID_DESCRIPTOR, "bridge_invalid_descriptor"))?;
    }
    let status = if operation == "next_event" || operation == "shutdown" {
        host.instance
            .get_typed_func::<i32, i32>(&host.store, export_name(operation))
            .map_err(|_| HostError::new(INTERNAL_ERROR, "bridge_trap"))?
            .call(&mut host.store, descriptor)
    } else {
        host.instance
            .get_typed_func::<(i32, i32, i32), i32>(&host.store, export_name(operation))
            .map_err(|_| HostError::new(INTERNAL_ERROR, "bridge_trap"))?
            .call(
                &mut host.store,
                (
                    input_pointer,
                    i32::try_from(input.len())
                        .map_err(|_| HostError::new(RESOURCE_LIMIT, "bridge_resource_limit"))?,
                    descriptor,
                ),
            )
    }
    .map_err(|_| HostError::new(INTERNAL_ERROR, "bridge_trap"))?;
    let mut descriptor_bytes = [0_u8; 8];
    host.memory
        .read(
            &host.store,
            usize::try_from(descriptor)
                .map_err(|_| HostError::new(INVALID_DESCRIPTOR, "bridge_invalid_descriptor"))?,
            &mut descriptor_bytes,
        )
        .map_err(|_| HostError::new(INVALID_DESCRIPTOR, "bridge_invalid_descriptor"))?;
    let pointer = u32::from_le_bytes(
        descriptor_bytes[..4]
            .try_into()
            .map_err(|_| HostError::new(INVALID_DESCRIPTOR, "bridge_invalid_descriptor"))?,
    );
    let length = u32::from_le_bytes(
        descriptor_bytes[4..]
            .try_into()
            .map_err(|_| HostError::new(INVALID_DESCRIPTOR, "bridge_invalid_descriptor"))?,
    ) as usize;
    if length > host.limits.maximum_output_bytes {
        return Err(HostError::new(RESOURCE_LIMIT, "bridge_resource_limit"));
    }
    let mut result = vec![0; length];
    host.memory
        .read(&host.store, pointer as usize, &mut result)
        .map_err(|_| HostError::new(INVALID_DESCRIPTOR, "bridge_invalid_descriptor"))?;
    let _ = dealloc.call(&mut host.store, (descriptor, 8));
    if input_pointer != 0 {
        let _ = dealloc.call(
            &mut host.store,
            (input_pointer, i32::try_from(input.len()).unwrap_or(0)),
        );
    }
    if status < 0 {
        return Err(HostError::new(status, "bridge_runtime_error"));
    }
    Ok(result)
}

/// C-compatible ABI marker.
#[unsafe(no_mangle)]
pub extern "C" fn traverse_swift_host_abi_version() -> u32 {
    ABI_VERSION
}

/// Creates one verified, bounded host and returns its opaque handle.
#[unsafe(no_mangle)]
pub extern "C" fn traverse_swift_host_create(
    runtime: *const u8,
    runtime_length: usize,
    expected: *const u8,
    expected_length: usize,
    limits: *const TraverseSwiftHostLimits,
    handle_out: *mut u64,
) -> i32 {
    if runtime.is_null() || expected.is_null() || limits.is_null() || handle_out.is_null() {
        return INVALID_INPUT;
    }
    // SAFETY: every pointer is checked non-null; callers supply immutable ranges of the stated lengths.
    // SAFETY: every pointer is checked non-null; callers supply immutable ranges of the stated lengths.
    let result = unsafe {
        (*limits).checked().and_then(|value| {
            create_host(
                std::slice::from_raw_parts(runtime, runtime_length),
                std::slice::from_raw_parts(expected, expected_length),
                value,
            )
        })
    };
    match result {
        Ok(host) => {
            let raw = Box::into_raw(Box::new(host)) as u64; // SAFETY: validated non-null output pointer writes one u64.
            unsafe {
                *handle_out = raw;
            }
            OK
        }
        Err(error) => error.status,
    }
}

/// Invokes a governed bridge operation using caller-owned buffers.
#[unsafe(no_mangle)]
pub extern "C" fn traverse_swift_host_invoke(
    handle: u64,
    operation: *const u8,
    operation_length: usize,
    input: *const u8,
    input_length: usize,
    output_buffer: *mut u8,
    output_capacity: usize,
    output_length_out: *mut usize,
) -> i32 {
    if handle == 0 || operation.is_null() || input.is_null() || output_length_out.is_null() {
        return INVALID_INPUT;
    }
    // SAFETY: C caller supplies valid ranges; the opaque handle came from `create` and is used only synchronously by the Swift serialization wrapper.
    let result = unsafe {
        (|| -> Result<Vec<u8>, HostError> {
            let host = &mut *(handle as *mut Host);
            let operation =
                std::str::from_utf8(std::slice::from_raw_parts(operation, operation_length))
                    .map_err(|_| HostError::new(INVALID_INPUT, "bridge_invalid_json"))?;
            invoke(
                host,
                operation,
                std::slice::from_raw_parts(input, input_length),
            )
        })()
    };
    match result {
        Ok(value) => output(&value, output_buffer, output_capacity, output_length_out),
        Err(error) => {
            let details = error.json();
            let write_status = output(
                &details[..details.len().min(MAX_ERROR_BYTES)],
                output_buffer,
                output_capacity,
                output_length_out,
            );
            if write_status == BUFFER_TOO_SMALL {
                BUFFER_TOO_SMALL
            } else {
                error.status
            }
        }
    }
}

/// Destroys a host handle; null is an idempotent success.
#[unsafe(no_mangle)]
pub extern "C" fn traverse_swift_host_destroy(handle: u64) -> i32 {
    if handle == 0 {
        return OK;
    } // SAFETY: `create` allocates this exact opaque Host pointer; callers must destroy at most once.
    unsafe {
        drop(Box::from_raw(handle as *mut Host));
    }
    OK
}

/// Maps a status to a bounded static UTF-8 message.
#[unsafe(no_mangle)]
pub extern "C" fn traverse_swift_host_status_message(status: i32) -> *const std::ffi::c_char {
    let value = match status {
        OK => "ok\0",
        INVALID_HANDLE => "invalid_handle\0",
        INVALID_INPUT => "invalid_input\0",
        INVALID_DESCRIPTOR => "invalid_descriptor\0",
        RESOURCE_LIMIT => "resource_limit\0",
        INTERNAL_ERROR => "internal_error\0",
        BUFFER_TOO_SMALL => "buffer_too_small\0",
        _ => "unknown_status\0",
    };
    value.as_ptr().cast()
}

#[cfg(test)]
mod tests {
    use super::{
        ABI_VERSION, RESOURCE_LIMIT, TraverseSwiftHostLimits, traverse_swift_host_abi_version,
        traverse_swift_host_create,
    };

    #[test]
    fn exposes_a_versioned_production_boundary() {
        assert_eq!(traverse_swift_host_abi_version(), ABI_VERSION);
    }

    #[test]
    fn rejects_unbounded_limits_before_module_instantiation() {
        let limits = TraverseSwiftHostLimits {
            maximum_artifact_bytes: 0,
            maximum_memory_bytes: 1,
            fuel_per_invocation: 1,
            maximum_input_bytes: 1,
            maximum_output_bytes: 1,
            maximum_queued_events: 1,
        };
        let bytes = [0_u8];
        let digest = b"sha256:00";
        let mut handle = 0_u64;
        assert_eq!(
            traverse_swift_host_create(
                bytes.as_ptr(),
                bytes.len(),
                digest.as_ptr(),
                digest.len(),
                &limits,
                &mut handle,
            ),
            RESOURCE_LIMIT
        );
        assert_eq!(handle, 0);
    }
}
