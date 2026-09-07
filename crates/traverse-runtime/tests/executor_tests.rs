use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use traverse_contracts::{ConnectorRequirement, EventReference, ServiceType};
use traverse_runtime::executor::{
    ActivatedConnector, ArtifactType, CapabilityExecutor, ConnectorInvokeRequest,
    ConnectorInvokeResponse, ExecutorCapability, ExecutorError, ExecutorOutput, MediatedConnector,
    MediatedConnectorContext, NativeExecutor, SUPPORTED_HOST_ABI_VERSION, WasmExecutionLimits,
    WasmExecutor, WasmModuleCacheConfig, supported_host_abi_versions, verify_wasm_host_abi_bytes,
};

// --- NativeExecutor tests ---

static TEMPFILE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[test]
fn native_executor_runs_handler() {
    let executor = NativeExecutor::new(|input| {
        let name = input["name"].as_str().unwrap_or("world");
        Ok(json!({ "greeting": format!("hello, {name}!") }))
    });

    let cap = native_capability("greet");
    let result = executor.execute(&cap, &json!({ "name": "traverse" }));

    assert_eq!(
        result,
        Ok(ExecutorOutput {
            value: json!({ "greeting": "hello, traverse!" }),
            emitted_events: Vec::new(),
            connector_invocation_evidence: Vec::new(),
        })
    );
}

#[test]
fn native_executor_propagates_handler_error() -> Result<(), String> {
    let executor = NativeExecutor::new(|_| Err("something went wrong".to_string()));

    let cap = native_capability("fail");
    let err = expect_err(
        executor.execute(&cap, &json!({})),
        "expected execution error",
    )?;

    assert_eq!(
        err,
        ExecutorError::ExecutionFailed("something went wrong".to_string())
    );
    Ok(())
}

#[test]
fn native_executor_rejects_wasm_artifact_type() -> Result<(), String> {
    let executor = NativeExecutor::new(|_| Ok(json!({})));

    let cap = ExecutorCapability {
        capability_id: "wrong-type".to_string(),
        artifact_type: ArtifactType::Wasm,
        wasm_binary_path: None,
        wasm_checksum: None,
        host_abi_version: None,
        emits: Vec::new(),
        service_type: ServiceType::Stateless,
    };
    let err = expect_err(executor.execute(&cap, &json!({})), "expected type error")?;

    assert_eq!(err, ExecutorError::UnsupportedArtifactType);
    Ok(())
}

#[test]
fn native_executor_passes_input_through() -> Result<(), String> {
    let executor = NativeExecutor::new(|input| Ok(input.clone()));

    let cap = native_capability("echo");
    let input = json!({ "a": 1, "b": [true, false] });
    let result = executor
        .execute(&cap, &input)
        .map_err(|e| format!("{e:?}"))?;

    assert_eq!(result.value, input);
    Ok(())
}

// --- WasmExecutor tests ---

#[test]
fn wasm_executor_rejects_native_artifact_type() -> Result<(), String> {
    let executor = WasmExecutor::new().map_err(|e| format!("{e:?}"))?;

    let cap = native_capability("wrong");
    let err = expect_err(executor.execute(&cap, &json!({})), "expected type error")?;

    assert_eq!(err, ExecutorError::UnsupportedArtifactType);
    Ok(())
}

#[test]
fn wasm_executor_errors_when_no_path_set() -> Result<(), String> {
    let executor = WasmExecutor::new().map_err(|e| format!("{e:?}"))?;

    let cap = ExecutorCapability {
        capability_id: "no-path".to_string(),
        artifact_type: ArtifactType::Wasm,
        wasm_binary_path: None,
        wasm_checksum: None,
        host_abi_version: None,
        emits: Vec::new(),
        service_type: ServiceType::Stateless,
    };
    let err = expect_err(
        executor.execute(&cap, &json!({})),
        "expected BinaryLoadFailed",
    )?;

    assert!(
        matches!(err, ExecutorError::BinaryLoadFailed(_)),
        "expected BinaryLoadFailed, got {err:?}"
    );
    Ok(())
}

#[test]
fn wasm_executor_errors_on_missing_file() -> Result<(), String> {
    let executor = WasmExecutor::new().map_err(|e| format!("{e:?}"))?;

    let cap = ExecutorCapability {
        capability_id: "missing".to_string(),
        artifact_type: ArtifactType::Wasm,
        wasm_binary_path: Some("/nonexistent/path/module.wasm".to_string()),
        wasm_checksum: None,
        host_abi_version: None,
        emits: Vec::new(),
        service_type: ServiceType::Stateless,
    };
    let err = expect_err(
        executor.execute(&cap, &json!({})),
        "expected BinaryLoadFailed",
    )?;

    assert!(
        matches!(err, ExecutorError::BinaryLoadFailed(_)),
        "expected BinaryLoadFailed, got {err:?}"
    );
    Ok(())
}

#[test]
fn wasm_executor_detects_checksum_mismatch() -> Result<(), String> {
    let executor = WasmExecutor::new().map_err(|e| format!("{e:?}"))?;

    // Build a minimal WAT module that just returns immediately
    let wat_src = r#"
        (module
            (memory 1)
            (func $main (export "_start"))
        )
    "#;
    let wasm_bytes = wat::parse_str(wat_src).map_err(|e| format!("WAT parse: {e}"))?;

    let tmp = tempfile_path();
    std::fs::write(&tmp, &wasm_bytes).map_err(|e| format!("write temp: {e}"))?;

    let cap = ExecutorCapability {
        capability_id: "checksum-test".to_string(),
        artifact_type: ArtifactType::Wasm,
        wasm_binary_path: Some(tmp.clone()),
        wasm_checksum: Some(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
        ),
        host_abi_version: None,
        emits: Vec::new(),
        service_type: ServiceType::Stateless,
    };

    let err = expect_err(
        executor.execute(&cap, &json!({})),
        "expected ChecksumMismatch",
    )?;
    std::fs::remove_file(&tmp).ok();

    assert!(
        matches!(err, ExecutorError::ChecksumMismatch { .. }),
        "expected ChecksumMismatch, got {err:?}"
    );
    Ok(())
}

#[test]
fn wasm_executor_runs_echo_module() -> Result<(), String> {
    let executor = WasmExecutor::new().map_err(|e| format!("{e:?}"))?;

    // WAT module that reads stdin and writes it back to stdout (echo)
    // Uses WASI fd_read (fd=0) and fd_write (fd=1)
    let wat_src = r#"
        (module
            (import "wasi_snapshot_preview1" "fd_read"
                (func $fd_read (param i32 i32 i32 i32) (result i32)))
            (import "wasi_snapshot_preview1" "fd_write"
                (func $fd_write (param i32 i32 i32 i32) (result i32)))
            (import "wasi_snapshot_preview1" "proc_exit"
                (func $proc_exit (param i32)))
            (memory (export "memory") 1)
            (func $_start (export "_start")
                ;; iovec for read: ptr=8, len=4096
                (i32.store (i32.const 0) (i32.const 8))
                (i32.store (i32.const 4) (i32.const 4096))
                ;; read stdin into offset 8
                (drop (call $fd_read (i32.const 0) (i32.const 0) (i32.const 1) (i32.const 4100)))
                ;; nread is at memory[4100]; use it as iovec len for write
                (i32.store (i32.const 0) (i32.const 8))
                (i32.store (i32.const 4) (i32.load (i32.const 4100)))
                ;; write stdout
                (drop (call $fd_write (i32.const 1) (i32.const 0) (i32.const 1) (i32.const 4104)))
            )
        )
    "#;

    let wasm_bytes = wat::parse_str(wat_src).map_err(|e| format!("WAT parse: {e}"))?;
    let input = json!({ "key": "value" });

    let result = executor
        .run_bytes(&wasm_bytes, &input)
        .map_err(|e| format!("{e:?}"))?;
    assert_eq!(result, input, "echo module should return input unchanged");
    Ok(())
}

#[test]
fn wasm_executor_accepts_proc_exit_zero_after_json_stdout() -> Result<(), String> {
    let executor = WasmExecutor::new().map_err(|e| format!("{e:?}"))?;
    let wat_src = r#"
        (module
            (import "wasi_snapshot_preview1" "fd_write"
                (func $fd_write (param i32 i32 i32 i32) (result i32)))
            (import "wasi_snapshot_preview1" "proc_exit"
                (func $proc_exit (param i32)))
            (memory (export "memory") 1)
            (data (i32.const 8) "{\"status\":\"ok\"}")
            (func $_start (export "_start")
                (i32.store (i32.const 0) (i32.const 8))
                (i32.store (i32.const 4) (i32.const 15))
                (drop (call $fd_write (i32.const 1) (i32.const 0) (i32.const 1) (i32.const 32)))
                (call $proc_exit (i32.const 0))
            )
        )
    "#;
    let wasm_bytes = wat::parse_str(wat_src).map_err(|e| format!("WAT parse: {e}"))?;

    let result = executor
        .run_bytes(&wasm_bytes, &json!({}))
        .map_err(|e| format!("{e:?}"))?;

    assert_eq!(result, json!({ "status": "ok" }));
    Ok(())
}

#[test]
fn wasm_executor_rejects_nonzero_proc_exit() -> Result<(), String> {
    let executor = WasmExecutor::new().map_err(|e| format!("{e:?}"))?;
    let wat_src = r#"
        (module
            (import "wasi_snapshot_preview1" "proc_exit"
                (func $proc_exit (param i32)))
            (func $_start (export "_start")
                (call $proc_exit (i32.const 1))
            )
        )
    "#;
    let wasm_bytes = wat::parse_str(wat_src).map_err(|e| format!("WAT parse: {e}"))?;

    let error = expect_err(
        executor.run_bytes(&wasm_bytes, &json!({})),
        "nonzero proc_exit must fail closed",
    )?;

    assert!(matches!(error, ExecutorError::ExecutionFailed(_)));
    Ok(())
}

#[test]
fn wasm_executor_rejects_invalid_json_output() -> Result<(), String> {
    let executor = WasmExecutor::new().map_err(|e| format!("{e:?}"))?;

    // WAT module that writes "not-json" to stdout
    let wat_src = r#"
        (module
            (import "wasi_snapshot_preview1" "fd_write"
                (func $fd_write (param i32 i32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 16) "not-json")
            (func $_start (export "_start")
                ;; iovec: ptr=16, len=8
                (i32.store (i32.const 0) (i32.const 16))
                (i32.store (i32.const 4) (i32.const 8))
                (drop (call $fd_write (i32.const 1) (i32.const 0) (i32.const 1) (i32.const 8)))
            )
        )
    "#;

    let wasm_bytes = wat::parse_str(wat_src).map_err(|e| format!("WAT parse: {e}"))?;
    let err = expect_err(
        executor.run_bytes(&wasm_bytes, &json!({})),
        "expected OutputDeserializationFailed",
    )?;

    assert!(
        matches!(err, ExecutorError::OutputDeserializationFailed(_)),
        "expected OutputDeserializationFailed, got {err:?}"
    );
    Ok(())
}

#[test]
fn wasm_executor_traps_infinite_loop_as_timeout() -> Result<(), String> {
    let executor = WasmExecutor::with_limits(WasmExecutionLimits {
        fuel_budget: 1_000,
        ..WasmExecutionLimits::default()
    })
    .map_err(|e| format!("{e:?}"))?;
    let wat_src = r#"
        (module
            (func $_start (export "_start")
                (loop $again
                    br $again
                )
            )
        )
    "#;
    let wasm_bytes = wat::parse_str(wat_src).map_err(|e| format!("WAT parse: {e}"))?;

    let err = expect_err(
        executor.run_bytes(&wasm_bytes, &json!({})),
        "expected Timeout",
    )?;

    assert!(
        matches!(err, ExecutorError::Timeout(_)),
        "expected Timeout, got {err:?}"
    );
    Ok(())
}

#[test]
fn wasm_executor_traps_memory_growth_as_resource_exhausted() -> Result<(), String> {
    let executor = WasmExecutor::with_limits(WasmExecutionLimits {
        fuel_budget: 100_000,
        memory_bytes: 64 * 1024,
        ..WasmExecutionLimits::default()
    })
    .map_err(|e| format!("{e:?}"))?;
    let wat_src = r#"
        (module
            (memory (export "memory") 1)
            (func $_start (export "_start")
                (drop (memory.grow (i32.const 1)))
            )
        )
    "#;
    let wasm_bytes = wat::parse_str(wat_src).map_err(|e| format!("WAT parse: {e}"))?;

    let err = expect_err(
        executor.run_bytes(&wasm_bytes, &json!({})),
        "expected ResourceExhausted",
    )?;

    assert!(
        matches!(err, ExecutorError::ResourceExhausted(_)),
        "expected ResourceExhausted, got {err:?}"
    );
    Ok(())
}

#[test]
fn wasm_executor_preserves_generic_traps_as_execution_failed() -> Result<(), String> {
    let executor = WasmExecutor::new().map_err(|e| format!("{e:?}"))?;
    let wat_src = r#"
        (module
            (func $_start (export "_start")
                unreachable
            )
        )
    "#;
    let wasm_bytes = wat::parse_str(wat_src).map_err(|e| format!("WAT parse: {e}"))?;

    let err = expect_err(
        executor.run_bytes(&wasm_bytes, &json!({})),
        "expected ExecutionFailed",
    )?;

    assert!(
        matches!(err, ExecutorError::ExecutionFailed(_)),
        "expected ExecutionFailed, got {err:?}"
    );
    Ok(())
}

#[test]
fn wasm_executor_reuses_compiled_module_by_checksum() -> Result<(), String> {
    let executor = WasmExecutor::with_limits_and_cache_config(
        WasmExecutionLimits::default(),
        WasmModuleCacheConfig { max_entries: 2 },
    )
    .map_err(|e| format!("{e:?}"))?;
    let wasm_bytes = wat::parse_str(echo_wat()).map_err(|e| format!("WAT parse: {e}"))?;

    let first_input = json!({ "call": 1 });
    let first = executor
        .run_bytes(&wasm_bytes, &first_input)
        .map_err(|e| format!("{e:?}"))?;
    assert_eq!(first, first_input);

    let after_first = executor.module_cache_stats();
    assert_eq!(after_first.entries, 1);
    assert_eq!(after_first.hits, 0);
    assert_eq!(after_first.misses, 1);

    let second_input = json!({ "call": 2 });
    let second = executor
        .run_bytes(&wasm_bytes, &second_input)
        .map_err(|e| format!("{e:?}"))?;
    assert_eq!(second, second_input);

    let after_second = executor.module_cache_stats();
    assert_eq!(after_second.entries, 1);
    assert_eq!(after_second.hits, 1);
    assert_eq!(after_second.misses, 1);
    assert_eq!(after_second.evictions, 0);
    Ok(())
}

#[test]
fn wasm_executor_cached_module_still_uses_fresh_store() -> Result<(), String> {
    let executor = WasmExecutor::with_limits_and_cache_config(
        WasmExecutionLimits::default(),
        WasmModuleCacheConfig { max_entries: 2 },
    )
    .map_err(|e| format!("{e:?}"))?;
    let wat_src = r#"
        (module
            (import "wasi_snapshot_preview1" "fd_write"
                (func $fd_write (param i32 i32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (global $counter (mut i32) (i32.const 0))
            (data (i32.const 8) "{\"count\":0}")
            (func $_start (export "_start")
                (global.set $counter (i32.add (global.get $counter) (i32.const 1)))
                (i32.store8 (i32.const 17) (i32.add (global.get $counter) (i32.const 48)))
                (i32.store (i32.const 0) (i32.const 8))
                (i32.store (i32.const 4) (i32.const 11))
                (drop (call $fd_write (i32.const 1) (i32.const 0) (i32.const 1) (i32.const 4)))
            )
        )
    "#;
    let wasm_bytes = wat::parse_str(wat_src).map_err(|e| format!("WAT parse: {e}"))?;

    let first = executor
        .run_bytes(&wasm_bytes, &json!({}))
        .map_err(|e| format!("{e:?}"))?;
    let second = executor
        .run_bytes(&wasm_bytes, &json!({}))
        .map_err(|e| format!("{e:?}"))?;

    assert_eq!(first, json!({ "count": 1 }));
    assert_eq!(second, json!({ "count": 1 }));
    assert_eq!(executor.module_cache_stats().hits, 1);
    Ok(())
}

#[test]
fn wasm_executor_cache_evicts_oldest_entry_deterministically() -> Result<(), String> {
    let executor = WasmExecutor::with_limits_and_cache_config(
        WasmExecutionLimits::default(),
        WasmModuleCacheConfig { max_entries: 1 },
    )
    .map_err(|e| format!("{e:?}"))?;
    let first_wasm = wat::parse_str(echo_wat()).map_err(|e| format!("WAT parse: {e}"))?;
    let second_wat = r#"
        (module
            (import "wasi_snapshot_preview1" "fd_write"
                (func $fd_write (param i32 i32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 8) "{}")
            (func $_start (export "_start")
                (i32.store (i32.const 0) (i32.const 8))
                (i32.store (i32.const 4) (i32.const 2))
                (drop (call $fd_write (i32.const 1) (i32.const 0) (i32.const 1) (i32.const 4)))
            )
        )
    "#;
    let second_wasm = wat::parse_str(second_wat).map_err(|e| format!("WAT parse: {e}"))?;

    executor
        .run_bytes(&first_wasm, &json!({ "cached": true }))
        .map_err(|e| format!("{e:?}"))?;
    executor
        .run_bytes(&second_wasm, &json!({}))
        .map_err(|e| format!("{e:?}"))?;
    executor
        .run_bytes(&first_wasm, &json!({ "cached": true }))
        .map_err(|e| format!("{e:?}"))?;

    let stats = executor.module_cache_stats();
    assert_eq!(stats.entries, 1);
    assert_eq!(stats.hits, 0);
    assert_eq!(stats.misses, 3);
    assert_eq!(stats.evictions, 2);
    Ok(())
}

#[test]
fn wasm_executor_cache_miss_when_abi_version_differs() -> Result<(), String> {
    let executor = WasmExecutor::new().map_err(|e| format!("{e:?}"))?;
    let wasm_bytes = wat::parse_str(echo_wat()).map_err(|e| format!("WAT parse: {e}"))?;

    executor
        .run_bytes(&wasm_bytes, &json!({ "abi": "1.0.0" }))
        .map_err(|e| format!("{e:?}"))?;
    let err = expect_err(
        executor.run_bytes_with_host_abi(&wasm_bytes, &json!({}), "2.0.0"),
        "expected unsupported ABI version",
    )?;

    assert!(matches!(err, ExecutorError::UnsupportedAbiVersion { .. }));
    let stats = executor.module_cache_stats();
    assert_eq!(stats.entries, 1);
    assert_eq!(stats.hits, 0);
    assert_eq!(stats.misses, 2);
    Ok(())
}

#[test]
fn wasm_host_abi_verifier_accepts_sanctioned_stdio_imports() -> Result<(), String> {
    let wasm_bytes = wat::parse_str(echo_wat()).map_err(|e| format!("WAT parse: {e}"))?;

    let validation = verify_wasm_host_abi_bytes(&wasm_bytes, SUPPORTED_HOST_ABI_VERSION)
        .map_err(|e| format!("{e:?}"))?;

    assert_eq!(validation.abi_version, SUPPORTED_HOST_ABI_VERSION);
    assert_eq!(supported_host_abi_versions(), &[SUPPORTED_HOST_ABI_VERSION]);
    assert!(
        validation.imports.iter().any(|import| {
            import.module == "wasi_snapshot_preview1" && import.name == "fd_read"
        })
    );
    assert!(
        validation.imports.iter().any(|import| {
            import.module == "wasi_snapshot_preview1" && import.name == "fd_write"
        })
    );
    Ok(())
}

#[test]
fn wasm_host_abi_verifier_accepts_versioned_connector_invoke_import() -> Result<(), String> {
    let wasm_bytes = wat::parse_str(
        r#"
        (module
            (import "traverse_host" "connector_invoke"
                (func $connector_invoke (param i32 i32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (func $_start (export "_start"))
        )
        "#,
    )
    .map_err(|error| format!("WAT parse: {error}"))?;

    let validation = verify_wasm_host_abi_bytes(&wasm_bytes, SUPPORTED_HOST_ABI_VERSION)
        .map_err(|error| format!("{error:?}"))?;

    assert!(
        validation.imports.iter().any(|import| {
            import.module == "traverse_host" && import.name == "connector_invoke"
        })
    );
    Ok(())
}

struct TestConnector;

impl MediatedConnector for TestConnector {
    fn invoke(&self, request: &ConnectorInvokeRequest) -> Result<ConnectorInvokeResponse, String> {
        Ok(ConnectorInvokeResponse {
            abi_version: SUPPORTED_HOST_ABI_VERSION.to_string(),
            result_class: "success".to_string(),
            payload: json!({"request_id": request.payload["id"]}),
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
    fn invoke(&self, _request: &ConnectorInvokeRequest) -> Result<ConnectorInvokeResponse, String> {
        match self.0 {
            ConnectorOutcome::Fails => Err("connector failed".to_string()),
            ConnectorOutcome::UnsafeOutput => Ok(ConnectorInvokeResponse {
                abi_version: SUPPORTED_HOST_ABI_VERSION.to_string(),
                result_class: "success".to_string(),
                payload: json!({"secret": "must not cross the ABI"}),
            }),
            ConnectorOutcome::LargeOutput => Ok(ConnectorInvokeResponse {
                abi_version: SUPPORTED_HOST_ABI_VERSION.to_string(),
                result_class: "success".to_string(),
                payload: json!({"value": "x".repeat(1024)}),
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
    .map_err(|error| format!("WAT parse: {error}"))
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
    .map_err(|error| format!("WAT parse: {error}"))
}

fn connector_context(
    connector: Arc<dyn MediatedConnector>,
    version: &str,
) -> MediatedConnectorContext {
    MediatedConnectorContext {
        declared_requirements: vec![ConnectorRequirement {
            connector_id: "traverse.test".to_string(),
            version: "^1.0.0".to_string(),
        }],
        activated_connectors: vec![ActivatedConnector {
            connector_id: "traverse.test".to_string(),
            version: version.to_string(),
            implementation: connector,
        }],
    }
}

fn declared_connector_context() -> MediatedConnectorContext {
    MediatedConnectorContext {
        declared_requirements: vec![ConnectorRequirement {
            connector_id: "traverse.test".to_string(),
            version: "^1.0.0".to_string(),
        }],
        activated_connectors: Vec::new(),
    }
}

/// Deterministic host-owned fixtures for the public universal connector
/// contracts. The guest receives only portable opaque references; neither
/// connector configuration nor provider-specific handles cross the ABI.
struct UniversalConnectorFixture {
    connector_id: String,
    seen_idempotency_keys: Mutex<BTreeSet<String>>,
}

impl UniversalConnectorFixture {
    fn new(connector_id: &str) -> Self {
        Self {
            connector_id: connector_id.to_string(),
            seen_idempotency_keys: Mutex::new(BTreeSet::new()),
        }
    }
}

impl MediatedConnector for UniversalConnectorFixture {
    fn invoke(&self, request: &ConnectorInvokeRequest) -> Result<ConnectorInvokeResponse, String> {
        if request.connector_id != self.connector_id {
            return Err("connector identity mismatch".to_string());
        }
        let idempotency_key = request
            .payload
            .get("idempotency_key")
            .and_then(Value::as_str)
            .ok_or_else(|| "idempotency_key is required".to_string())?;
        let replay = !self
            .seen_idempotency_keys
            .lock()
            .map_err(|_| "fixture idempotency state unavailable".to_string())?
            .insert(idempotency_key.to_string());
        let payload = match (self.connector_id.as_str(), request.operation.as_str()) {
            ("traverse.object-store", "put_immutable") => {
                if request
                    .payload
                    .get("content_ref")
                    .and_then(Value::as_str)
                    .is_none()
                    || request
                        .payload
                        .get("media_type")
                        .and_then(Value::as_str)
                        .is_none()
                {
                    return Err("invalid immutable object request".to_string());
                }
                json!({
                    "asset_ref": "asset:sha256:fixture-object",
                    "content_digest": "sha256:fixture-object",
                    "size": 128,
                    "result_class": if replay { "replayed" } else { "created" }
                })
            }
            ("traverse.state-store", "append_transition") => {
                if request.payload.get("expected_version") == Some(&json!(0)) {
                    return Err("stale expected version conflict".to_string());
                }
                json!({
                    "result_ref": "state:fixture-transition",
                    "version": 2,
                    "replay": replay,
                    "result_class": if replay { "replayed" } else { "appended" }
                })
            }
            ("traverse.scheduler", "schedule_invocation") => {
                if request.payload.get("logical_deadline") == Some(&json!("late")) {
                    return Err("late logical deadline".to_string());
                }
                json!({
                    "invocation_ref": "invocation:fixture-scheduled",
                    "idempotency_key": idempotency_key,
                    "result_class": if replay { "replayed" } else { "scheduled" }
                })
            }
            _ => return Err("unsupported connector operation".to_string()),
        };
        Ok(ConnectorInvokeResponse {
            abi_version: SUPPORTED_HOST_ABI_VERSION.to_string(),
            result_class: "success".to_string(),
            payload,
        })
    }
}

fn universal_connector_context(
    connector_id: &str,
    connector: Arc<dyn MediatedConnector>,
    version: &str,
) -> MediatedConnectorContext {
    MediatedConnectorContext {
        declared_requirements: vec![ConnectorRequirement {
            connector_id: connector_id.to_string(),
            version: "^1.0.0".to_string(),
        }],
        activated_connectors: vec![ActivatedConnector {
            connector_id: connector_id.to_string(),
            version: version.to_string(),
            implementation: connector,
        }],
    }
}

fn invoke_fixture_connector(
    connector_id: &str,
    request: &str,
    connector: Arc<dyn MediatedConnector>,
) -> Result<ExecutorOutput, String> {
    let request_len = i32::try_from(request.len())
        .map_err(|error| format!("request length conversion: {error}"))?;
    let wasm = connector_response_wasm(request, 32, request_len, 2_048, 2_048)?;
    WasmExecutor::new()
        .map_err(|error| format!("{error:?}"))?
        .run_bytes_with_mediated_connectors(
            &wasm,
            &json!({}),
            "universal-connector-fixture",
            universal_connector_context(connector_id, connector, "1.0.0"),
        )
        .map_err(|error| format!("{error:?}"))
}

fn invoke_fixture_connector_failure(
    connector_id: &str,
    request: &str,
    connector: Arc<dyn MediatedConnector>,
) -> Result<ExecutorOutput, String> {
    let request_len = i32::try_from(request.len())
        .map_err(|error| format!("request length conversion: {error}"))?;
    let wasm = connector_test_wasm(request, 32, request_len, 2_048, 2_048)?;
    WasmExecutor::new()
        .map_err(|error| format!("{error:?}"))?
        .run_bytes_with_mediated_connectors(
            &wasm,
            &json!({}),
            "universal-connector-fixture",
            universal_connector_context(connector_id, connector, "1.0.0"),
        )
        .map_err(|error| format!("{error:?}"))
}

const VALID_CONNECTOR_REQUEST: &str = r#"{"abi_version":"1.0.0","connector_id":"traverse.test","operation":"read","payload":{"id":"x"}}"#;

fn valid_connector_request_len() -> Result<i32, String> {
    i32::try_from(VALID_CONNECTOR_REQUEST.len())
        .map_err(|error| format!("request length conversion: {error}"))
}

fn connector_request(connector_id: &str, operation: &str, payload: &serde_json::Value) -> String {
    json!({
        "abi_version": SUPPORTED_HOST_ABI_VERSION,
        "connector_id": connector_id,
        "operation": operation,
        "payload": payload
    })
    .to_string()
}

fn invoke_connector_fixture(
    connector_id: &str,
    operation: &str,
    payload: &serde_json::Value,
    connector: Arc<dyn MediatedConnector>,
) -> Result<ExecutorOutput, String> {
    let request = connector_request(connector_id, operation, payload);
    let request_len = i32::try_from(request.len())
        .map_err(|error| format!("request length conversion: {error}"))?;
    let wasm = connector_response_wasm(&request, 32, request_len, 256, 2048)?;
    let executor = WasmExecutor::new().map_err(|error| format!("{error:?}"))?;
    executor
        .run_bytes_with_mediated_connectors(
            &wasm,
            &json!({}),
            "universal-connector-fixture",
            universal_connector_context(connector_id, connector, "1.0.0"),
        )
        .map_err(|error| format!("{error:?}"))
}

fn assert_no_private_connector_details(output: &ExecutorOutput) {
    let rendered = format!(
        "{:?} {}",
        output.connector_invocation_evidence, output.value
    );
    for forbidden in [
        "/var/private/traverse",
        "bucket-prod",
        "postgres://",
        "scheduler-device-42",
        "credential",
        "secret-token",
        "https://provider.internal",
    ] {
        assert!(
            !rendered.contains(forbidden),
            "connector output/evidence leaked {forbidden}: {rendered}"
        );
    }
}

struct ObjectStoreFixtureConnector {
    max_bytes: u64,
}

impl MediatedConnector for ObjectStoreFixtureConnector {
    fn invoke(&self, request: &ConnectorInvokeRequest) -> Result<ConnectorInvokeResponse, String> {
        let size = request.payload["size"].as_u64().unwrap_or(u64::MAX);
        let digest = request.payload["content_digest"]
            .as_str()
            .unwrap_or_default();
        let result_class = if size > self.max_bytes {
            "too_large"
        } else if digest != "sha256:fixture-content" {
            "integrity"
        } else {
            "stored"
        };
        Ok(ConnectorInvokeResponse {
            abi_version: SUPPORTED_HOST_ABI_VERSION.to_string(),
            result_class: result_class.to_string(),
            payload: json!({
                "asset_ref": "asset:fixture-content",
                "content_digest": digest,
                "size": size,
                "result_class": result_class
            }),
        })
    }
}

#[derive(Default)]
struct StateStoreFixtureConnector {
    state: Mutex<StateStoreFixtureState>,
}

#[derive(Default)]
struct StateStoreFixtureState {
    version: u64,
    seen_idempotency: BTreeMap<String, u64>,
}

impl MediatedConnector for StateStoreFixtureConnector {
    fn invoke(&self, request: &ConnectorInvokeRequest) -> Result<ConnectorInvokeResponse, String> {
        let idempotency_key = request.payload["idempotency_key"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let expected_version = request.payload["expected_version"].as_u64();
        let mut state = self
            .state
            .lock()
            .map_err(|_| "state fixture lock poisoned".to_string())?;
        if let Some(version) = state.seen_idempotency.get(&idempotency_key) {
            return Ok(ConnectorInvokeResponse {
                abi_version: SUPPORTED_HOST_ABI_VERSION.to_string(),
                result_class: "replay".to_string(),
                payload: json!({
                    "result_ref": "state:fixture-transition",
                    "version": version,
                    "replay": true,
                    "result_class": "replay"
                }),
            });
        }
        if expected_version.is_some_and(|version| version != state.version) {
            return Ok(ConnectorInvokeResponse {
                abi_version: SUPPORTED_HOST_ABI_VERSION.to_string(),
                result_class: "conflict".to_string(),
                payload: json!({
                    "result_ref": "state:fixture-transition",
                    "version": state.version,
                    "replay": false,
                    "result_class": "conflict"
                }),
            });
        }
        state.version += 1;
        let version = state.version;
        state.seen_idempotency.insert(idempotency_key, version);
        Ok(ConnectorInvokeResponse {
            abi_version: SUPPORTED_HOST_ABI_VERSION.to_string(),
            result_class: "appended".to_string(),
            payload: json!({
                "result_ref": "state:fixture-transition",
                "version": version,
                "replay": false,
                "result_class": "appended"
            }),
        })
    }
}

#[derive(Default)]
struct SchedulerFixtureConnector {
    scheduled: Mutex<BTreeSet<String>>,
}

impl MediatedConnector for SchedulerFixtureConnector {
    fn invoke(&self, request: &ConnectorInvokeRequest) -> Result<ConnectorInvokeResponse, String> {
        let deadline = request.payload["logical_deadline"]
            .as_str()
            .unwrap_or_default();
        let idempotency_key = request.payload["idempotency_key"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let result_class = if deadline < "2026-08-18T00:00:00Z" {
            "late"
        } else {
            let mut scheduled = self
                .scheduled
                .lock()
                .map_err(|_| "scheduler fixture lock poisoned".to_string())?;
            if scheduled.insert(idempotency_key.clone()) {
                "scheduled"
            } else {
                "duplicate"
            }
        };
        Ok(ConnectorInvokeResponse {
            abi_version: SUPPORTED_HOST_ABI_VERSION.to_string(),
            result_class: result_class.to_string(),
            payload: json!({
                "invocation_ref": "schedule:fixture-invocation",
                "idempotency_key": idempotency_key,
                "result_class": result_class
            }),
        })
    }
}

#[test]
fn universal_object_store_fixture_returns_only_portable_asset_evidence() -> Result<(), String> {
    let connector = Arc::new(UniversalConnectorFixture::new("traverse.object-store"));
    let request = r#"{"abi_version":"1.0.0","connector_id":"traverse.object-store","operation":"put_immutable","payload":{"content_ref":"content:sha256:fixture","media_type":"application/json","idempotency_key":"object-1"}}"#;
    let output = invoke_fixture_connector("traverse.object-store", request, connector)?;

    assert_eq!(
        output.value["result_class"], "success",
        "evidence: {:?}",
        output.connector_invocation_evidence
    );
    assert_eq!(
        output.value["payload"]["asset_ref"],
        "asset:sha256:fixture-object"
    );
    assert_eq!(
        output.value["payload"]["content_digest"],
        "sha256:fixture-object"
    );
    assert_eq!(
        output.connector_invocation_evidence[0].result_class,
        "success"
    );
    let public_output = output.value.to_string();
    for private_marker in ["secret", "bucket", "endpoint", "path", "://"] {
        assert!(
            !public_output.contains(private_marker),
            "guest-visible object-store output leaked {private_marker}"
        );
    }
    Ok(())
}

#[test]
fn universal_state_store_fixture_replays_duplicates_and_rejects_stale_writes() -> Result<(), String>
{
    let connector = Arc::new(UniversalConnectorFixture::new("traverse.state-store"));
    let request = r#"{"abi_version":"1.0.0","connector_id":"traverse.state-store","operation":"append_transition","payload":{"record_refs":["record:fixture"],"transition":{"to":"accepted"},"expected_version":1,"idempotency_key":"state-1"}}"#;

    let first = invoke_fixture_connector("traverse.state-store", request, connector.clone())?;
    let replay = invoke_fixture_connector("traverse.state-store", request, connector.clone())?;
    assert_eq!(
        first.value["payload"]["replay"], false,
        "evidence: {:?}",
        first.connector_invocation_evidence
    );
    assert_eq!(replay.value["payload"]["replay"], true);
    assert_eq!(
        replay.value["payload"]["result_ref"],
        "state:fixture-transition"
    );

    let stale_request = request.replace("\"expected_version\":1", "\"expected_version\":0");
    let stale =
        invoke_fixture_connector_failure("traverse.state-store", &stale_request, connector)?;
    assert_eq!(
        stale.connector_invocation_evidence[0].result_class,
        "failure"
    );
    assert_eq!(
        stale.connector_invocation_evidence[0]
            .failure_class
            .as_deref(),
        Some("execution_failed")
    );
    Ok(())
}

#[test]
fn universal_scheduler_fixture_replays_duplicates_and_rejects_late_requests() -> Result<(), String>
{
    let connector = Arc::new(UniversalConnectorFixture::new("traverse.scheduler"));
    let request = r#"{"abi_version":"1.0.0","connector_id":"traverse.scheduler","operation":"schedule_invocation","payload":{"job_kind":"reconcile","calendar_policy_ref":"policy:fixture","logical_deadline":"2026-08-20T00:00:00Z","idempotency_key":"schedule-1"}}"#;

    let first = invoke_fixture_connector("traverse.scheduler", request, connector.clone())?;
    let replay = invoke_fixture_connector("traverse.scheduler", request, connector.clone())?;
    assert_eq!(
        first.value["payload"]["result_class"], "scheduled",
        "evidence: {:?}",
        first.connector_invocation_evidence
    );
    assert_eq!(replay.value["payload"]["result_class"], "replayed");
    assert_eq!(
        replay.value["payload"]["invocation_ref"],
        "invocation:fixture-scheduled"
    );

    let late_request = request.replace("2026-08-20T00:00:00Z", "late");
    let late = invoke_fixture_connector_failure("traverse.scheduler", &late_request, connector)?;
    assert_eq!(
        late.connector_invocation_evidence[0].result_class,
        "failure"
    );
    assert_eq!(
        late.connector_invocation_evidence[0]
            .failure_class
            .as_deref(),
        Some("execution_failed")
    );
    Ok(())
}

#[test]
fn wasm_connector_invoke_denies_invalid_requests() -> Result<(), String> {
    let valid_request_len = valid_connector_request_len()?;
    let executor = WasmExecutor::new().map_err(|error| format!("{error:?}"))?;

    let invalid_requests = vec![
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
        (VALID_CONNECTOR_REQUEST, 32, valid_request_len, 256, 65_537),
    ];

    for (request, request_ptr, request_len, response_ptr, response_capacity) in invalid_requests {
        let request_len = if request_len == 0 {
            i32::try_from(request.len())
                .map_err(|error| format!("request length conversion: {error}"))?
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
        let output = executor
            .run_bytes_with_mediated_connectors(
                &wasm,
                &json!({}),
                "test",
                connector_context(Arc::new(TestConnector), "1.0.0"),
            )
            .map_err(|error| format!("{error:?}"))?;
        assert_eq!(output.value, json!(0));
    }

    Ok(())
}

#[test]
fn wasm_connector_invoke_accepts_numeric_payload_values() -> Result<(), String> {
    let numeric_payload_request = r#"{"abi_version":"1.0.0","connector_id":"traverse.test","operation":"read","payload":{"id":1}}"#;
    let numeric_payload_request_len = i32::try_from(numeric_payload_request.len())
        .map_err(|error| format!("request length conversion: {error}"))?;
    let wasm = connector_test_wasm(
        numeric_payload_request,
        32,
        numeric_payload_request_len,
        256,
        64,
    )?;
    let executor = WasmExecutor::new().map_err(|error| format!("{error:?}"))?;
    let output = executor
        .run_bytes_with_mediated_connectors(
            &wasm,
            &json!({}),
            "test",
            connector_context(Arc::new(TestConnector), "1.0.0"),
        )
        .map_err(|error| format!("{error:?}"))?;
    assert_eq!(output.value, json!(0));
    Ok(())
}

#[test]
fn wasm_connector_invoke_denies_modules_without_memory() -> Result<(), String> {
    let wasm_without_memory = wat::parse_str(
        r#"
        (module
          (import "traverse_host" "connector_invoke" (func $invoke (param i32 i32 i32 i32) (result i32)))
          (func $_start (export "_start")
            i32.const 0 i32.const 0 i32.const 0 i32.const 0 call $invoke drop))
        "#,
    )
    .map_err(|error| format!("WAT parse: {error}"))?;
    let executor = WasmExecutor::new().map_err(|error| format!("{error:?}"))?;
    let err = expect_err(
        executor.run_bytes_with_mediated_connectors(
            &wasm_without_memory,
            &json!({}),
            "test",
            connector_context(Arc::new(TestConnector), "1.0.0"),
        ),
        "expected invalid stdout after connector_invoke without memory",
    )?;
    assert!(
        matches!(err, ExecutorError::OutputDeserializationFailed(_)),
        "expected OutputDeserializationFailed, got {err:?}"
    );

    Ok(())
}

#[test]
fn wasm_connector_invoke_records_unbound_and_undeclared_failures() -> Result<(), String> {
    let valid_request_len = valid_connector_request_len()?;
    let executor = WasmExecutor::new().map_err(|error| format!("{error:?}"))?;
    let wasm = connector_test_wasm(VALID_CONNECTOR_REQUEST, 32, valid_request_len, 256, 64)?;
    let unbound = executor
        .run_bytes_with_capability(&wasm, &json!({}), "test", &[], ServiceType::Stateless)
        .map_err(|error| format!("{error:?}"))?;
    assert_eq!(
        unbound.connector_invocation_evidence[0]
            .failure_class
            .as_deref(),
        Some("unbound")
    );

    let undeclared_request = VALID_CONNECTOR_REQUEST.replace("traverse.test", "other.test");
    let undeclared_request_len = i32::try_from(undeclared_request.len())
        .map_err(|error| format!("request length conversion: {error}"))?;
    let wasm = connector_test_wasm(&undeclared_request, 32, undeclared_request_len, 256, 64)?;
    let undeclared = executor
        .run_bytes_with_mediated_connectors(
            &wasm,
            &json!({}),
            "test",
            connector_context(Arc::new(TestConnector), "1.0.0"),
        )
        .map_err(|error| format!("{error:?}"))?;
    assert_eq!(
        undeclared.connector_invocation_evidence[0]
            .failure_class
            .as_deref(),
        Some("undeclared")
    );

    let wasm = connector_test_wasm(VALID_CONNECTOR_REQUEST, 32, valid_request_len, 256, 64)?;
    let declared_unbound = executor
        .run_bytes_with_mediated_connectors(&wasm, &json!({}), "test", declared_connector_context())
        .map_err(|error| format!("{error:?}"))?;
    assert_eq!(
        declared_unbound.connector_invocation_evidence[0]
            .failure_class
            .as_deref(),
        Some("unbound")
    );

    Ok(())
}

#[test]
fn wasm_connector_invoke_records_activated_connector_failures() -> Result<(), String> {
    let valid_request_len = valid_connector_request_len()?;
    let executor = WasmExecutor::new().map_err(|error| format!("{error:?}"))?;

    for (connector, version, response_ptr, response_capacity, failure_class) in [
        (
            Arc::new(TestConnector) as Arc<dyn MediatedConnector>,
            "2.0.0",
            256,
            64,
            "incompatible",
        ),
        (
            Arc::new(OutcomeConnector(ConnectorOutcome::Fails)) as Arc<dyn MediatedConnector>,
            "1.0.0",
            256,
            64,
            "execution_failed",
        ),
        (
            Arc::new(OutcomeConnector(ConnectorOutcome::UnsafeOutput))
                as Arc<dyn MediatedConnector>,
            "1.0.0",
            256,
            64,
            "unauthorized_output",
        ),
        (
            Arc::new(OutcomeConnector(ConnectorOutcome::LargeOutput)) as Arc<dyn MediatedConnector>,
            "1.0.0",
            256,
            1,
            "bounded_io",
        ),
        (
            Arc::new(TestConnector) as Arc<dyn MediatedConnector>,
            "1.0.0",
            65_535,
            1_024,
            "invalid_response_memory",
        ),
    ] {
        let wasm = connector_test_wasm(
            VALID_CONNECTOR_REQUEST,
            32,
            valid_request_len,
            response_ptr,
            response_capacity,
        )?;
        let output = executor
            .run_bytes_with_mediated_connectors(
                &wasm,
                &json!({}),
                "test",
                connector_context(connector, version),
            )
            .map_err(|error| format!("{error:?}"))?;
        assert_eq!(
            output.connector_invocation_evidence[0]
                .failure_class
                .as_deref(),
            Some(failure_class)
        );
    }
    Ok(())
}

#[test]
fn wasm_connector_invoke_requires_declaration_and_activation() -> Result<(), String> {
    let wasm_bytes = wat::parse_str(
        r#"
        (module
          (import "wasi_snapshot_preview1" "fd_write" (func $write (param i32 i32 i32 i32) (result i32)))
          (import "traverse_host" "connector_invoke" (func $invoke (param i32 i32 i32 i32) (result i32)))
          (memory (export "memory") 1)
          (data (i32.const 32) "{\"abi_version\":\"1.0.0\",\"connector_id\":\"traverse.test\",\"operation\":\"read\",\"payload\":{\"id\":\"x\"}}")
          (func $_start (export "_start") (local $len i32)
            i32.const 32 i32.const 94 i32.const 256 i32.const 1024 call $invoke local.set $len
            i32.const 4 i32.const 256 i32.store
            i32.const 8 local.get $len i32.store
            i32.const 1 i32.const 4 i32.const 1 i32.const 12 call $write drop))
        "#,
    )
    .map_err(|error| format!("WAT parse: {error}"))?;
    let executor = WasmExecutor::new().map_err(|error| format!("{error:?}"))?;
    let context = MediatedConnectorContext {
        declared_requirements: vec![ConnectorRequirement {
            connector_id: "traverse.test".to_string(),
            version: "^1.0.0".to_string(),
        }],
        activated_connectors: vec![ActivatedConnector {
            connector_id: "traverse.test".to_string(),
            version: "1.0.0".to_string(),
            implementation: Arc::new(TestConnector),
        }],
    };
    let output = executor
        .run_bytes_with_mediated_connectors(&wasm_bytes, &json!({}), "test", context)
        .map_err(|error| format!("{error:?}"))?;
    assert_eq!(output.value["result_class"], "success");
    assert_eq!(output.value["payload"]["request_id"], "x");
    assert_eq!(output.connector_invocation_evidence.len(), 1);
    assert_eq!(
        output.connector_invocation_evidence[0].connector_id,
        "traverse.test"
    );
    assert_eq!(
        output.connector_invocation_evidence[0]
            .resolved_version
            .as_deref(),
        Some("1.0.0")
    );
    assert_eq!(
        output.connector_invocation_evidence[0].result_class,
        "success"
    );
    assert_eq!(output.connector_invocation_evidence[0].failure_class, None);
    Ok(())
}

#[test]
fn universal_connector_fixtures_invoke_through_mediated_abi() -> Result<(), String> {
    struct UniversalConnectorCase {
        connector_id: &'static str,
        operation: &'static str,
        payload: serde_json::Value,
        connector: Arc<dyn MediatedConnector>,
        result_class: &'static str,
    }

    let cases = vec![
        UniversalConnectorCase {
            connector_id: "traverse.object-store",
            operation: "put_immutable",
            payload: json!({
                "content_ref": "content:fixture",
                "content_digest": "sha256:fixture-content",
                "size": 12,
                "idempotency_key": "object-ok"
            }),
            connector: Arc::new(ObjectStoreFixtureConnector { max_bytes: 64 }),
            result_class: "stored",
        },
        UniversalConnectorCase {
            connector_id: "traverse.state-store",
            operation: "append_transition",
            payload: json!({
                "record_refs": ["record:fixture"],
                "transition": {"kind": "fixture"},
                "expected_version": 0,
                "idempotency_key": "state-ok"
            }),
            connector: Arc::new(StateStoreFixtureConnector::default()),
            result_class: "appended",
        },
        UniversalConnectorCase {
            connector_id: "traverse.scheduler",
            operation: "schedule_invocation",
            payload: json!({
                "job_kind": "fixture-job",
                "calendar_policy_ref": "calendar:fixture",
                "logical_deadline": "2026-08-19T00:00:00Z",
                "idempotency_key": "schedule-ok"
            }),
            connector: Arc::new(SchedulerFixtureConnector::default()),
            result_class: "scheduled",
        },
    ];

    for case in cases {
        let output = invoke_connector_fixture(
            case.connector_id,
            case.operation,
            &case.payload,
            case.connector,
        )?;
        assert_eq!(output.value["result_class"], case.result_class);
        assert_eq!(
            output.connector_invocation_evidence[0].connector_id,
            case.connector_id
        );
        assert_eq!(
            output.connector_invocation_evidence[0]
                .resolved_version
                .as_deref(),
            Some("1.0.0")
        );
        assert_eq!(
            output.connector_invocation_evidence[0].result_class,
            case.result_class
        );
        assert_eq!(output.connector_invocation_evidence[0].failure_class, None);
        assert_no_private_connector_details(&output);
    }

    Ok(())
}

#[test]
fn object_store_fixture_enforces_digest_and_byte_bounds() -> Result<(), String> {
    let connector = Arc::new(ObjectStoreFixtureConnector { max_bytes: 16 });
    let oversized = invoke_connector_fixture(
        "traverse.object-store",
        "put_immutable",
        &json!({
            "content_ref": "content:too-large",
            "content_digest": "sha256:fixture-content",
            "size": 17,
            "idempotency_key": "object-too-large"
        }),
        connector.clone(),
    )?;
    assert_eq!(oversized.value["result_class"], "too_large");
    assert_eq!(
        oversized.connector_invocation_evidence[0].result_class,
        "too_large"
    );
    assert_no_private_connector_details(&oversized);

    let bad_digest = invoke_connector_fixture(
        "traverse.object-store",
        "put_immutable",
        &json!({
            "content_ref": "content:bad-digest",
            "content_digest": "sha256:wrong",
            "size": 12,
            "idempotency_key": "object-bad-digest"
        }),
        connector,
    )?;
    assert_eq!(bad_digest.value["result_class"], "integrity");
    assert_eq!(
        bad_digest.connector_invocation_evidence[0].result_class,
        "integrity"
    );
    assert_no_private_connector_details(&bad_digest);
    Ok(())
}

#[test]
fn state_store_fixture_replays_duplicate_and_reports_stale_conflict() -> Result<(), String> {
    let connector = Arc::new(StateStoreFixtureConnector::default());
    let initial = invoke_connector_fixture(
        "traverse.state-store",
        "append_transition",
        &json!({
            "record_refs": ["record:fixture"],
            "transition": {"kind": "started"},
            "expected_version": 0,
            "idempotency_key": "state-idempotent"
        }),
        connector.clone(),
    )?;
    assert_eq!(initial.value["result_class"], "appended");
    assert_eq!(initial.value["payload"]["version"], 1);

    let replay = invoke_connector_fixture(
        "traverse.state-store",
        "append_transition",
        &json!({
            "record_refs": ["record:fixture"],
            "transition": {"kind": "started"},
            "expected_version": 0,
            "idempotency_key": "state-idempotent"
        }),
        connector.clone(),
    )?;
    assert_eq!(replay.value["result_class"], "replay");
    assert_eq!(replay.value["payload"]["replay"], true);

    let stale = invoke_connector_fixture(
        "traverse.state-store",
        "append_transition",
        &json!({
            "record_refs": ["record:fixture"],
            "transition": {"kind": "stale"},
            "expected_version": 0,
            "idempotency_key": "state-stale"
        }),
        connector,
    )?;
    assert_eq!(stale.value["result_class"], "conflict");
    assert_eq!(stale.value["payload"]["version"], 1);

    assert_no_private_connector_details(&initial);
    assert_no_private_connector_details(&replay);
    assert_no_private_connector_details(&stale);
    Ok(())
}

#[test]
fn scheduler_fixture_reports_duplicate_and_late_requests() -> Result<(), String> {
    let connector = Arc::new(SchedulerFixtureConnector::default());
    let scheduled = invoke_connector_fixture(
        "traverse.scheduler",
        "schedule_invocation",
        &json!({
            "job_kind": "fixture-job",
            "calendar_policy_ref": "calendar:fixture",
            "logical_deadline": "2026-08-19T00:00:00Z",
            "idempotency_key": "schedule-idempotent"
        }),
        connector.clone(),
    )?;
    assert_eq!(scheduled.value["result_class"], "scheduled");

    let duplicate = invoke_connector_fixture(
        "traverse.scheduler",
        "schedule_invocation",
        &json!({
            "job_kind": "fixture-job",
            "calendar_policy_ref": "calendar:fixture",
            "logical_deadline": "2026-08-19T00:00:00Z",
            "idempotency_key": "schedule-idempotent"
        }),
        connector.clone(),
    )?;
    assert_eq!(duplicate.value["result_class"], "duplicate");

    let late = invoke_connector_fixture(
        "traverse.scheduler",
        "schedule_invocation",
        &json!({
            "job_kind": "fixture-job",
            "calendar_policy_ref": "calendar:fixture",
            "logical_deadline": "2026-08-17T23:59:59Z",
            "idempotency_key": "schedule-late"
        }),
        connector,
    )?;
    assert_eq!(late.value["result_class"], "late");

    assert_no_private_connector_details(&scheduled);
    assert_no_private_connector_details(&duplicate);
    assert_no_private_connector_details(&late);
    Ok(())
}

#[test]
fn universal_connector_authorization_failures_are_non_secret() -> Result<(), String> {
    let request = connector_request(
        "traverse.object-store",
        "put_immutable",
        &json!({
            "content_ref": "content:fixture",
            "content_digest": "sha256:fixture-content",
            "size": 12,
            "idempotency_key": "object-authz"
        }),
    );
    let request_len = i32::try_from(request.len())
        .map_err(|error| format!("request length conversion: {error}"))?;
    let wasm = connector_test_wasm(&request, 32, request_len, 256, 2048)?;
    let executor = WasmExecutor::new().map_err(|error| format!("{error:?}"))?;

    let undeclared = executor
        .run_bytes_with_mediated_connectors(
            &wasm,
            &json!({}),
            "universal-connector-authz",
            MediatedConnectorContext {
                declared_requirements: vec![ConnectorRequirement {
                    connector_id: "traverse.scheduler".to_string(),
                    version: "^1.0.0".to_string(),
                }],
                activated_connectors: vec![ActivatedConnector {
                    connector_id: "traverse.object-store".to_string(),
                    version: "1.0.0".to_string(),
                    implementation: Arc::new(ObjectStoreFixtureConnector { max_bytes: 64 }),
                }],
            },
        )
        .map_err(|error| format!("{error:?}"))?;
    assert_eq!(
        undeclared.connector_invocation_evidence[0]
            .failure_class
            .as_deref(),
        Some("undeclared")
    );
    assert_no_private_connector_details(&undeclared);

    let incompatible = executor
        .run_bytes_with_mediated_connectors(
            &wasm,
            &json!({}),
            "universal-connector-authz",
            universal_connector_context(
                "traverse.object-store",
                Arc::new(ObjectStoreFixtureConnector { max_bytes: 64 }),
                "2.0.0",
            ),
        )
        .map_err(|error| format!("{error:?}"))?;
    assert_eq!(
        incompatible.connector_invocation_evidence[0]
            .failure_class
            .as_deref(),
        Some("incompatible")
    );
    assert_no_private_connector_details(&incompatible);

    Ok(())
}

#[test]
fn wasm_host_abi_verifier_rejects_unauthorized_import_before_execution() -> Result<(), String> {
    let wat_src = r#"
        (module
            (import "wasi_snapshot_preview1" "random_get"
                (func $random_get (param i32 i32) (result i32)))
            (memory (export "memory") 1)
            (func $_start (export "_start")
                unreachable
            )
        )
    "#;
    let wasm_bytes = wat::parse_str(wat_src).map_err(|e| format!("WAT parse: {e}"))?;
    let executor = WasmExecutor::new().map_err(|e| format!("{e:?}"))?;

    let err = expect_err(
        executor.run_bytes(&wasm_bytes, &json!({})),
        "expected unauthorized host import",
    )?;

    assert_eq!(
        err,
        ExecutorError::UnauthorizedHostImport {
            error_code: "unauthorized_host_import".to_string(),
            abi_version: SUPPORTED_HOST_ABI_VERSION.to_string(),
            module: "wasi_snapshot_preview1".to_string(),
            name: "random_get".to_string(),
        }
    );
    Ok(())
}

#[test]
fn wasm_host_abi_verifier_rejects_unsupported_abi_version() -> Result<(), String> {
    let wasm_bytes = wat::parse_str(echo_wat()).map_err(|e| format!("WAT parse: {e}"))?;
    let executor = WasmExecutor::new().map_err(|e| format!("{e:?}"))?;

    let err = expect_err(
        executor.run_bytes_with_host_abi(&wasm_bytes, &json!({}), "2.0.0"),
        "expected unsupported ABI version",
    )?;

    assert_eq!(
        err,
        ExecutorError::UnsupportedAbiVersion {
            error_code: "unsupported_abi_version".to_string(),
            requested: "2.0.0".to_string(),
            supported: SUPPORTED_HOST_ABI_VERSION.to_string(),
        }
    );
    Ok(())
}

#[test]
fn wasm_host_abi_verifier_reports_malformed_binary() -> Result<(), String> {
    let err = expect_err(
        verify_wasm_host_abi_bytes(b"not-a-wasm-binary", SUPPORTED_HOST_ABI_VERSION),
        "expected malformed WASM artifact",
    )?;

    assert!(
        matches!(err, ExecutorError::MalformedWasmArtifact { .. }),
        "expected MalformedWasmArtifact, got {err:?}"
    );
    Ok(())
}

// --- Debug impl coverage ---

#[test]
fn native_executor_debug_impl_is_accessible() {
    let executor = NativeExecutor::new(|_| Ok(json!({})));
    let dbg = format!("{executor:?}");
    assert!(dbg.contains("NativeExecutor"), "Debug output: {dbg}");
}

// --- ExecutorError Display coverage ---

#[test]
fn executor_error_display_covers_all_variants() {
    let cases: &[(ExecutorError, &str)] = &[
        (
            ExecutorError::BinaryLoadFailed("oops".to_string()),
            "binary load failed: oops",
        ),
        (
            ExecutorError::ChecksumMismatch {
                expected: "abc".to_string(),
                actual: "def".to_string(),
            },
            "checksum mismatch: expected abc, got def",
        ),
        (
            ExecutorError::RuntimeSetupFailed("bad linker".to_string()),
            "runtime setup failed: bad linker",
        ),
        (
            ExecutorError::MalformedWasmArtifact {
                error_code: "malformed_wasm_artifact".to_string(),
                detail: "bad magic".to_string(),
            },
            "malformed_wasm_artifact: bad magic",
        ),
        (
            ExecutorError::UnsupportedAbiVersion {
                error_code: "unsupported_abi_version".to_string(),
                requested: "2.0.0".to_string(),
                supported: "1.0.0".to_string(),
            },
            "unsupported_abi_version: requested Traverse Host ABI 2.0.0, supported 1.0.0",
        ),
        (
            ExecutorError::UnauthorizedHostImport {
                error_code: "unauthorized_host_import".to_string(),
                abi_version: "1.0.0".to_string(),
                module: "wasi_snapshot_preview1".to_string(),
                name: "random_get".to_string(),
            },
            "unauthorized_host_import: ABI 1.0.0 does not allow import wasi_snapshot_preview1::random_get",
        ),
        (
            ExecutorError::ExecutionFailed("trapped".to_string()),
            "execution failed: trapped",
        ),
        (
            ExecutorError::Timeout("fuel exhausted".to_string()),
            "execution timed out: fuel exhausted",
        ),
        (
            ExecutorError::ResourceExhausted("memory cap".to_string()),
            "resource exhausted: memory cap",
        ),
        (
            ExecutorError::OutputDeserializationFailed("not json".to_string()),
            "output deserialization failed: not json",
        ),
        (
            ExecutorError::UnsupportedArtifactType,
            "unsupported artifact type for this executor",
        ),
    ];
    for (err, expected_msg) in cases {
        assert_eq!(
            format!("{err}"),
            *expected_msg,
            "Display mismatch for {err:?}"
        );
    }
}

#[test]
fn wasm_executor_full_execute_path_via_disk() -> Result<(), String> {
    // Tests the execute() code path (file I/O + optional checksum) end-to-end.
    let executor = WasmExecutor::new().map_err(|e| format!("{e:?}"))?;

    let wat_src = r#"
        (module
            (import "wasi_snapshot_preview1" "fd_read"
                (func $fd_read (param i32 i32 i32 i32) (result i32)))
            (import "wasi_snapshot_preview1" "fd_write"
                (func $fd_write (param i32 i32 i32 i32) (result i32)))
            (import "wasi_snapshot_preview1" "proc_exit"
                (func $proc_exit (param i32)))
            (memory (export "memory") 1)
            (func $_start (export "_start")
                (i32.store (i32.const 0) (i32.const 8))
                (i32.store (i32.const 4) (i32.const 4096))
                (drop (call $fd_read (i32.const 0) (i32.const 0) (i32.const 1) (i32.const 4100)))
                (i32.store (i32.const 0) (i32.const 8))
                (i32.store (i32.const 4) (i32.load (i32.const 4100)))
                (drop (call $fd_write (i32.const 1) (i32.const 0) (i32.const 1) (i32.const 4104)))
            )
        )
    "#;
    let wasm_bytes = wat::parse_str(wat_src).map_err(|e| format!("WAT parse: {e}"))?;
    let tmp = tempfile_path();
    std::fs::write(&tmp, &wasm_bytes).map_err(|e| format!("write: {e}"))?;

    let cap = ExecutorCapability {
        capability_id: "disk-echo".to_string(),
        artifact_type: ArtifactType::Wasm,
        wasm_binary_path: Some(tmp.clone()),
        wasm_checksum: None, // no checksum — exercises the skip-checksum branch
        host_abi_version: None,
        emits: Vec::new(),
        service_type: ServiceType::Stateless,
    };

    let input = json!({ "disk": true });
    let result = executor.execute(&cap, &input).map_err(|e| format!("{e:?}"));
    std::fs::remove_file(&tmp).ok();

    assert_eq!(
        result,
        Ok(ExecutorOutput {
            value: input,
            emitted_events: Vec::new(),
            connector_invocation_evidence: Vec::new(),
        })
    );
    Ok(())
}

#[test]
fn wasm_executor_execute_with_matching_checksum_succeeds() -> Result<(), String> {
    // Exercises the checksum-match success branch in execute() — skipped by run_bytes() tests.
    let executor = WasmExecutor::new().map_err(|e| format!("{e:?}"))?;

    let wat_src = r#"
        (module
            (import "wasi_snapshot_preview1" "fd_write"
                (func $fd_write (param i32 i32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 8) "{}")
            (func $_start (export "_start")
                (i32.store (i32.const 0) (i32.const 8))
                (i32.store (i32.const 4) (i32.const 2))
                (drop (call $fd_write (i32.const 1) (i32.const 0) (i32.const 1) (i32.const 4)))
            )
        )
    "#;
    let wasm_bytes = wat::parse_str(wat_src).map_err(|e| format!("WAT parse: {e}"))?;

    // Compute the correct SHA-256 checksum so the checksum-match branch is taken.
    let mut hasher = Sha256::new();
    hasher.update(&wasm_bytes);
    let checksum: String = hasher
        .finalize()
        .iter()
        .fold(String::new(), |mut acc, byte| {
            let _ = write!(acc, "{byte:02x}");
            acc
        });

    let tmp = tempfile_path();
    std::fs::write(&tmp, &wasm_bytes).map_err(|e| format!("write: {e}"))?;

    let cap = ExecutorCapability {
        capability_id: "checksum-ok".to_string(),
        artifact_type: ArtifactType::Wasm,
        wasm_binary_path: Some(tmp.clone()),
        wasm_checksum: Some(checksum),
        host_abi_version: Some("1.0.0".to_string()),
        emits: Vec::new(),
        service_type: ServiceType::Stateless,
    };

    let result = executor
        .execute(&cap, &json!({}))
        .map_err(|e| format!("{e:?}"));
    std::fs::remove_file(&tmp).ok();

    assert_eq!(
        result,
        Ok(ExecutorOutput {
            value: json!({}),
            emitted_events: Vec::new(),
            connector_invocation_evidence: Vec::new(),
        })
    );
    Ok(())
}

#[test]
fn wasm_executor_reuses_unchanged_binary_without_reading_or_hashing_again() -> Result<(), String> {
    let executor = WasmExecutor::new().map_err(|e| format!("{e:?}"))?;
    let wasm_bytes = wat::parse_str(echo_wat()).map_err(|e| format!("WAT parse: {e}"))?;
    let tmp = tempfile_path();
    std::fs::write(&tmp, &wasm_bytes).map_err(|e| format!("write: {e}"))?;

    let cap = ExecutorCapability {
        capability_id: "cached-disk-echo".to_string(),
        artifact_type: ArtifactType::Wasm,
        wasm_binary_path: Some(tmp.clone()),
        wasm_checksum: None,
        host_abi_version: None,
        emits: Vec::new(),
        service_type: ServiceType::Stateless,
    };

    let first_input = json!({ "call": 1 });
    let first = executor
        .execute(&cap, &first_input)
        .map_err(|e| format!("{e:?}"))?;
    assert_eq!(first.value, first_input);
    assert_eq!(
        executor.binary_cache_stats(),
        traverse_runtime::executor::WasmBinaryCacheStats {
            entries: 1,
            hits: 0,
            loads: 1,
            hashes: 1,
            evictions: 0,
        }
    );

    let second_input = json!({ "call": 2 });
    let second = executor
        .execute(&cap, &second_input)
        .map_err(|e| format!("{e:?}"))?;
    std::fs::remove_file(&tmp).ok();

    assert_eq!(second.value, second_input);
    let stats = executor.binary_cache_stats();
    assert_eq!(stats.hits, 1);
    assert_eq!(stats.loads, 1, "cache hit must skip a disk read");
    assert_eq!(stats.hashes, 1, "cache hit must skip SHA-256");
    Ok(())
}

#[test]
fn wasm_executor_binary_cache_evicts_oldest_path_deterministically() -> Result<(), String> {
    let executor = WasmExecutor::with_limits_and_cache_config(
        WasmExecutionLimits::default(),
        WasmModuleCacheConfig { max_entries: 1 },
    )
    .map_err(|e| format!("{e:?}"))?;
    let wasm_bytes = wat::parse_str(echo_wat()).map_err(|e| format!("WAT parse: {e}"))?;
    let first_path = tempfile_path();
    let second_path = tempfile_path();
    std::fs::write(&first_path, &wasm_bytes).map_err(|e| format!("write first: {e}"))?;
    std::fs::write(&second_path, &wasm_bytes).map_err(|e| format!("write second: {e}"))?;

    let capability = |path: String| ExecutorCapability {
        capability_id: "binary-cache-eviction".to_string(),
        artifact_type: ArtifactType::Wasm,
        wasm_binary_path: Some(path),
        wasm_checksum: None,
        host_abi_version: None,
        emits: Vec::new(),
        service_type: ServiceType::Stateless,
    };

    executor
        .execute(&capability(first_path.clone()), &json!({ "call": 1 }))
        .map_err(|e| format!("{e:?}"))?;
    executor
        .execute(&capability(second_path.clone()), &json!({ "call": 2 }))
        .map_err(|e| format!("{e:?}"))?;
    executor
        .execute(&capability(first_path.clone()), &json!({ "call": 3 }))
        .map_err(|e| format!("{e:?}"))?;
    std::fs::remove_file(&first_path).ok();
    std::fs::remove_file(&second_path).ok();

    assert_eq!(
        executor.binary_cache_stats(),
        traverse_runtime::executor::WasmBinaryCacheStats {
            entries: 1,
            hits: 0,
            loads: 3,
            hashes: 3,
            evictions: 2,
        }
    );
    Ok(())
}

#[test]
fn wasm_executor_cached_module_does_not_bypass_checksum_mismatch() -> Result<(), String> {
    let executor = WasmExecutor::new().map_err(|e| format!("{e:?}"))?;
    let wasm_bytes = wat::parse_str(echo_wat()).map_err(|e| format!("WAT parse: {e}"))?;

    let mut hasher = Sha256::new();
    hasher.update(&wasm_bytes);
    let checksum: String = hasher
        .finalize()
        .iter()
        .fold(String::new(), |mut acc, byte| {
            let _ = write!(acc, "{byte:02x}");
            acc
        });

    let tmp = tempfile_path();
    std::fs::write(&tmp, &wasm_bytes).map_err(|e| format!("write: {e}"))?;

    let cap = ExecutorCapability {
        capability_id: "checksum-cache".to_string(),
        artifact_type: ArtifactType::Wasm,
        wasm_binary_path: Some(tmp.clone()),
        wasm_checksum: Some(checksum),
        host_abi_version: Some("1.0.0".to_string()),
        emits: Vec::new(),
        service_type: ServiceType::Stateless,
    };

    let input = json!({ "cache": true });
    let result = executor
        .execute(&cap, &input)
        .map_err(|e| format!("{e:?}"))?;
    assert_eq!(result.value, input);
    assert_eq!(executor.module_cache_stats().entries, 1);

    std::fs::write(&tmp, b"not-the-same-wasm").map_err(|e| format!("overwrite: {e}"))?;
    let err = expect_err(
        executor.execute(&cap, &json!({})),
        "expected checksum mismatch",
    )?;
    std::fs::remove_file(&tmp).ok();

    assert!(
        matches!(err, ExecutorError::ChecksumMismatch { .. }),
        "expected ChecksumMismatch, got {err:?}"
    );
    Ok(())
}

#[test]
fn wasm_executor_invalid_binary_triggers_runtime_setup_failed() -> Result<(), String> {
    let executor = WasmExecutor::new().map_err(|e| format!("{e:?}"))?;

    // Write garbage bytes — not a valid WASM module
    let tmp = tempfile_path();
    std::fs::write(&tmp, b"not-a-wasm-binary").map_err(|e| format!("write: {e}"))?;

    let cap = ExecutorCapability {
        capability_id: "bad-binary".to_string(),
        artifact_type: ArtifactType::Wasm,
        wasm_binary_path: Some(tmp.clone()),
        wasm_checksum: None,
        host_abi_version: None,
        emits: Vec::new(),
        service_type: ServiceType::Stateless,
    };

    let err = expect_err(executor.execute(&cap, &json!({})), "expected error")?;
    std::fs::remove_file(&tmp).ok();

    assert!(
        matches!(err, ExecutorError::MalformedWasmArtifact { .. }),
        "expected MalformedWasmArtifact, got {err:?}"
    );
    Ok(())
}

// --- `traverse_host::emit_event` host ABI tests (spec 098-capability-event-host-abi) ---

#[test]
fn wasm_executor_emit_event_accepted_for_declared_subscribable_capability() -> Result<(), String> {
    let executor = WasmExecutor::new().map_err(|e| format!("{e:?}"))?;
    let wasm_bytes =
        wat::parse_str(emit_event_wat(EMIT_TEST_EVENT_PAYLOAD)).map_err(|e| format!("{e}"))?;

    let output = executor
        .run_bytes_with_capability(
            &wasm_bytes,
            &json!({}),
            "test.emit.subject",
            &[EventReference {
                event_id: "dev.traverse.test.emitted".to_string(),
                version: "1.0.0".to_string(),
            }],
            ServiceType::Subscribable,
        )
        .map_err(|e| format!("{e:?}"))?;

    assert_eq!(output.emitted_events.len(), 1);
    let event = &output.emitted_events[0];
    assert_eq!(event.event_type, "dev.traverse.test.emitted");
    assert_eq!(event.version, "1.0.0");
    assert_eq!(event.data, json!({ "n": 1 }));
    assert_eq!(event.owner, "test.emit.subject");
    Ok(())
}

#[test]
fn wasm_executor_emit_event_rejected_for_undeclared_event_type() -> Result<(), String> {
    let executor = WasmExecutor::new().map_err(|e| format!("{e:?}"))?;
    let wasm_bytes =
        wat::parse_str(emit_event_wat(EMIT_TEST_EVENT_PAYLOAD)).map_err(|e| format!("{e}"))?;

    // `emits` declares a different event than the one the guest tries to emit.
    let output = executor
        .run_bytes_with_capability(
            &wasm_bytes,
            &json!({}),
            "test.emit.subject",
            &[EventReference {
                event_id: "dev.traverse.test.other".to_string(),
                version: "1.0.0".to_string(),
            }],
            ServiceType::Subscribable,
        )
        .map_err(|e| format!("{e:?}"))?;

    assert!(
        output.emitted_events.is_empty(),
        "undeclared event must not be accepted"
    );
    Ok(())
}

#[test]
fn wasm_executor_emit_event_rejected_for_non_subscribable_capability() -> Result<(), String> {
    let executor = WasmExecutor::new().map_err(|e| format!("{e:?}"))?;
    let wasm_bytes =
        wat::parse_str(emit_event_wat(EMIT_TEST_EVENT_PAYLOAD)).map_err(|e| format!("{e}"))?;

    let output = executor
        .run_bytes_with_capability(
            &wasm_bytes,
            &json!({}),
            "test.emit.subject",
            &[EventReference {
                event_id: "dev.traverse.test.emitted".to_string(),
                version: "1.0.0".to_string(),
            }],
            ServiceType::Stateless, // NOT Subscribable
        )
        .map_err(|e| format!("{e:?}"))?;

    assert!(
        output.emitted_events.is_empty(),
        "non-Subscribable capability must never emit"
    );
    Ok(())
}

#[test]
fn wasm_executor_emit_event_rejected_for_malformed_json_payload() -> Result<(), String> {
    let executor = WasmExecutor::new().map_err(|e| format!("{e:?}"))?;
    // Not valid JSON at all.
    let wasm_bytes = wat::parse_str(emit_event_wat("not-json")).map_err(|e| format!("{e}"))?;

    let output = executor
        .run_bytes_with_capability(
            &wasm_bytes,
            &json!({}),
            "test.emit.subject",
            &[EventReference {
                event_id: "dev.traverse.test.emitted".to_string(),
                version: "1.0.0".to_string(),
            }],
            ServiceType::Subscribable,
        )
        .map_err(|e| format!("{e:?}"))?;

    assert!(
        output.emitted_events.is_empty(),
        "malformed payload must not be accepted, and execution must not trap"
    );
    Ok(())
}

#[test]
fn wasm_executor_emit_event_rejected_for_oversized_payload() -> Result<(), String> {
    let executor = WasmExecutor::new().map_err(|e| format!("{e:?}"))?;
    // The module declares a 1-page (64 KiB) memory but claims a payload
    // length one byte larger than the ABI's maximum accepted size — the
    // host must reject this before ever attempting to read guest memory.
    let wasm_bytes = wat::parse_str(emit_event_wat_with_len(
        EMIT_TEST_EVENT_PAYLOAD,
        64 * 1024 + 1,
    ))
    .map_err(|e| format!("{e}"))?;

    let output = executor
        .run_bytes_with_capability(
            &wasm_bytes,
            &json!({}),
            "test.emit.subject",
            &[EventReference {
                event_id: "dev.traverse.test.emitted".to_string(),
                version: "1.0.0".to_string(),
            }],
            ServiceType::Subscribable,
        )
        .map_err(|e| format!("{e:?}"))?;

    assert!(
        output.emitted_events.is_empty(),
        "oversized payload must not be accepted, and execution must not trap"
    );
    Ok(())
}

#[test]
fn wasm_executor_emit_event_rejected_for_out_of_bounds_pointer() -> Result<(), String> {
    let executor = WasmExecutor::new().map_err(|e| format!("{e:?}"))?;
    // One page of memory is 64 KiB; a pointer past that, with a small
    // length, is out of bounds without exceeding the payload size cap —
    // this exercises the `Memory::read` bounds check itself (FR-008),
    // distinct from the size-cap check above.
    let wasm_bytes =
        wat::parse_str(emit_event_wat_with_ptr_len(200_000, 10)).map_err(|e| format!("{e}"))?;

    let output = executor
        .run_bytes_with_capability(
            &wasm_bytes,
            &json!({}),
            "test.emit.subject",
            &[EventReference {
                event_id: "dev.traverse.test.emitted".to_string(),
                version: "1.0.0".to_string(),
            }],
            ServiceType::Subscribable,
        )
        .map_err(|e| format!("{e:?}"))?;

    assert!(
        output.emitted_events.is_empty(),
        "out-of-bounds pointer must not be accepted, and execution must not trap or read outside guest memory"
    );
    Ok(())
}

#[test]
fn wasm_executor_emit_event_rejected_for_negative_pointer_or_length() -> Result<(), String> {
    let executor = WasmExecutor::new().map_err(|e| format!("{e:?}"))?;
    let wasm_bytes =
        wat::parse_str(emit_event_wat_with_raw_ptr_len(-1, 4)).map_err(|e| format!("{e}"))?;

    let output = executor
        .run_bytes_with_capability(
            &wasm_bytes,
            &json!({}),
            "test.emit.subject",
            &[EventReference {
                event_id: "dev.traverse.test.emitted".to_string(),
                version: "1.0.0".to_string(),
            }],
            ServiceType::Subscribable,
        )
        .map_err(|e| format!("{e:?}"))?;

    assert!(
        output.emitted_events.is_empty(),
        "a negative pointer must not be accepted, and execution must not trap"
    );
    Ok(())
}

#[test]
fn wasm_executor_emit_event_rejected_for_missing_event_id() -> Result<(), String> {
    let executor = WasmExecutor::new().map_err(|e| format!("{e:?}"))?;
    let wasm_bytes =
        wat::parse_str(emit_event_wat(r#"{"version":"1.0.0"}"#)).map_err(|e| format!("{e}"))?;

    let output = executor
        .run_bytes_with_capability(
            &wasm_bytes,
            &json!({}),
            "test.emit.subject",
            &[EventReference {
                event_id: "dev.traverse.test.emitted".to_string(),
                version: "1.0.0".to_string(),
            }],
            ServiceType::Subscribable,
        )
        .map_err(|e| format!("{e:?}"))?;

    assert!(
        output.emitted_events.is_empty(),
        "a payload missing event_id must not be accepted"
    );
    Ok(())
}

#[test]
fn wasm_executor_emit_event_rejected_for_missing_version() -> Result<(), String> {
    let executor = WasmExecutor::new().map_err(|e| format!("{e:?}"))?;
    let wasm_bytes = wat::parse_str(emit_event_wat(
        r#"{"event_id":"dev.traverse.test.emitted"}"#,
    ))
    .map_err(|e| format!("{e}"))?;

    let output = executor
        .run_bytes_with_capability(
            &wasm_bytes,
            &json!({}),
            "test.emit.subject",
            &[EventReference {
                event_id: "dev.traverse.test.emitted".to_string(),
                version: "1.0.0".to_string(),
            }],
            ServiceType::Subscribable,
        )
        .map_err(|e| format!("{e:?}"))?;

    assert!(
        output.emitted_events.is_empty(),
        "a payload missing version must not be accepted"
    );
    Ok(())
}

#[test]
fn wasm_executor_emit_event_handles_module_without_memory_export() -> Result<(), String> {
    let executor = WasmExecutor::new().map_err(|e| format!("{e:?}"))?;
    // No `(memory ...)` declaration at all, so `caller.get_export("memory")`
    // must return `None` — the host function has to reject this gracefully
    // (FR-008) rather than panicking or trapping.
    let wat_src = r#"
        (module
            (import "traverse_host" "emit_event"
                (func $emit_event (param i32 i32) (result i32)))
            (func $_start (export "_start")
                (drop (call $emit_event (i32.const 0) (i32.const 4)))
            )
        )
    "#;
    let wasm_bytes = wat::parse_str(wat_src).map_err(|e| format!("{e}"))?;

    let result = executor.run_bytes_with_capability(
        &wasm_bytes,
        &json!({}),
        "test.emit.subject",
        &[EventReference {
            event_id: "dev.traverse.test.emitted".to_string(),
            version: "1.0.0".to_string(),
        }],
        ServiceType::Subscribable,
    );

    // The module never writes stdout (it has no memory to do so), so this
    // surfaces as a normal, controlled executor error — not a panic/trap —
    // which is exactly what proves the missing-memory-export path inside
    // `handle_emit_event` was handled gracefully.
    assert!(
        result.is_err(),
        "expected a controlled error, not a panic or trap: {result:?}"
    );
    Ok(())
}

#[test]
fn core_transition_action_status_emits_status_transitioned_on_accepted_transition()
-> Result<(), String> {
    let wasm_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
        "../../examples/core-transition-action-status/artifacts/core-transition-action-status.wasm",
    );
    let wasm_bytes = std::fs::read(&wasm_path).map_err(|e| {
        format!(
            "failed to read {}: {e} (run examples/core-transition-action-status/build-fixture.sh first)",
            wasm_path.display()
        )
    })?;

    let executor = WasmExecutor::new().map_err(|e| format!("{e:?}"))?;
    let input = json!({
        "action_item_id": "item-emit-001",
        "current_status": "open",
        "requested_status": "in_progress",
        "actor_id": "user-ada",
        "owner_id": "user-ada",
        "transition_config": {
            "version": "1.0",
            "allowed_transitions": {
                "open": ["in_progress", "cancelled", "snoozed"],
                "in_progress": ["blocked", "done", "cancelled", "snoozed"],
                "blocked": ["in_progress", "cancelled"],
                "snoozed": ["open", "in_progress", "cancelled"],
                "done": [],
                "cancelled": []
            },
            "owner_only": true
        }
    });

    let output = executor
        .run_bytes_with_capability(
            &wasm_bytes,
            &input,
            "core.transition-action-status",
            &[EventReference {
                event_id: "core.action-item.status-transitioned".to_string(),
                version: "1.0.0".to_string(),
            }],
            ServiceType::Subscribable,
        )
        .map_err(|e| format!("{e:?}"))?;

    assert_eq!(output.value["allowed"], json!(true));
    assert_eq!(output.value["reason_code"], json!("ok"));
    assert_eq!(output.value["new_status"], json!("in_progress"));
    assert_eq!(
        output.emitted_events.len(),
        1,
        "accepted transition must emit exactly one governed event"
    );
    let event = &output.emitted_events[0];
    assert_eq!(event.event_type, "core.action-item.status-transitioned");
    assert_eq!(event.version, "1.0.0");
    assert_eq!(event.data["action_item_id"], json!("item-emit-001"));
    assert_eq!(event.data["from_status"], json!("open"));
    assert_eq!(event.data["to_status"], json!("in_progress"));
    assert_eq!(event.data["actor_id"], json!("user-ada"));
    Ok(())
}

#[test]
fn core_transition_action_status_does_not_emit_on_rejected_transition() -> Result<(), String> {
    let wasm_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
        "../../examples/core-transition-action-status/artifacts/core-transition-action-status.wasm",
    );
    let wasm_bytes = std::fs::read(&wasm_path).map_err(|e| {
        format!(
            "failed to read {}: {e} (run examples/core-transition-action-status/build-fixture.sh first)",
            wasm_path.display()
        )
    })?;

    let executor = WasmExecutor::new().map_err(|e| format!("{e:?}"))?;
    let input = json!({
        "action_item_id": "item-emit-002",
        "current_status": "done",
        "requested_status": "open",
        "actor_id": "user-ada",
        "owner_id": "user-ada",
        "transition_config": {
            "version": "1.0",
            "allowed_transitions": {
                "open": ["in_progress", "cancelled", "snoozed"],
                "in_progress": ["blocked", "done", "cancelled", "snoozed"],
                "blocked": ["in_progress", "cancelled"],
                "snoozed": ["open", "in_progress", "cancelled"],
                "done": [],
                "cancelled": []
            },
            "owner_only": true
        }
    });

    let output = executor
        .run_bytes_with_capability(
            &wasm_bytes,
            &input,
            "core.transition-action-status",
            &[EventReference {
                event_id: "core.action-item.status-transitioned".to_string(),
                version: "1.0.0".to_string(),
            }],
            ServiceType::Subscribable,
        )
        .map_err(|e| format!("{e:?}"))?;

    assert_eq!(output.value["allowed"], json!(false));
    assert_eq!(output.value["reason_code"], json!("illegal_transition"));
    assert!(
        output.emitted_events.is_empty(),
        "rejected transition must not emit"
    );
    Ok(())
}

// --- helpers ---

fn native_capability(id: &str) -> ExecutorCapability {
    ExecutorCapability {
        capability_id: id.to_string(),
        artifact_type: ArtifactType::Native,
        wasm_binary_path: None,
        wasm_checksum: None,
        host_abi_version: None,
        emits: Vec::new(),
        service_type: ServiceType::Stateless,
    }
}

fn tempfile_path() -> String {
    let suffix = TEMPFILE_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!(
        "/tmp/traverse-test-{}-{suffix}.wasm",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    )
}

fn echo_wat() -> &'static str {
    r#"
        (module
            (import "wasi_snapshot_preview1" "fd_read"
                (func $fd_read (param i32 i32 i32 i32) (result i32)))
            (import "wasi_snapshot_preview1" "fd_write"
                (func $fd_write (param i32 i32 i32 i32) (result i32)))
            (import "wasi_snapshot_preview1" "proc_exit"
                (func $proc_exit (param i32)))
            (memory (export "memory") 1)
            (func $_start (export "_start")
                (i32.store (i32.const 0) (i32.const 8))
                (i32.store (i32.const 4) (i32.const 4096))
                (drop (call $fd_read (i32.const 0) (i32.const 0) (i32.const 1) (i32.const 4100)))
                (i32.store (i32.const 0) (i32.const 8))
                (i32.store (i32.const 4) (i32.load (i32.const 4100)))
                (drop (call $fd_write (i32.const 1) (i32.const 0) (i32.const 1) (i32.const 4104)))
            )
        )
    "#
}

const EMIT_TEST_EVENT_PAYLOAD: &str =
    r#"{"event_id":"dev.traverse.test.emitted","version":"1.0.0","payload":{"n":1}}"#;

fn wat_escape(payload: &str) -> String {
    payload.replace('\\', "\\\\").replace('"', "\\\"")
}

/// A WASM module that writes `payload` into linear memory at offset 100,
/// calls `traverse_host::emit_event(100, payload.len())`, then writes `{}`
/// to stdout (a minimal valid JSON output, unrelated to the emit call).
fn emit_event_wat(payload: &str) -> String {
    emit_event_wat_with_len(payload, payload.len())
}

/// As [`emit_event_wat`], but the guest claims `claimed_len` bytes instead
/// of `payload.len()` — used to simulate an oversized length claim.
fn emit_event_wat_with_len(payload: &str, claimed_len: usize) -> String {
    format!(
        r#"
        (module
            (import "traverse_host" "emit_event"
                (func $emit_event (param i32 i32) (result i32)))
            (import "wasi_snapshot_preview1" "fd_write"
                (func $fd_write (param i32 i32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 100) "{payload}")
            (data (i32.const 300) "{{}}")
            (func $_start (export "_start")
                (drop (call $emit_event (i32.const 100) (i32.const {claimed_len})))
                (i32.store (i32.const 0) (i32.const 300))
                (i32.store (i32.const 4) (i32.const 2))
                (drop (call $fd_write (i32.const 1) (i32.const 0) (i32.const 1) (i32.const 4100)))
            )
        )
        "#,
        payload = wat_escape(payload),
    )
}

/// A WASM module that calls `traverse_host::emit_event(ptr, len)` directly
/// with caller-supplied coordinates, ignoring what (if anything) is actually
/// at that memory location — used to exercise the guest-memory bounds check.
fn emit_event_wat_with_ptr_len(ptr: usize, len: usize) -> String {
    emit_event_wat_with_raw_ptr_len(
        i32::try_from(ptr).unwrap_or(i32::MAX),
        i32::try_from(len).unwrap_or(i32::MAX),
    )
}

/// As [`emit_event_wat_with_ptr_len`], but accepts raw `i32` coordinates
/// directly — used to exercise a negative pointer or length.
fn emit_event_wat_with_raw_ptr_len(ptr: i32, len: i32) -> String {
    format!(
        r#"
        (module
            (import "traverse_host" "emit_event"
                (func $emit_event (param i32 i32) (result i32)))
            (import "wasi_snapshot_preview1" "fd_write"
                (func $fd_write (param i32 i32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 300) "{{}}")
            (func $_start (export "_start")
                (drop (call $emit_event (i32.const {ptr}) (i32.const {len})))
                (i32.store (i32.const 0) (i32.const 300))
                (i32.store (i32.const 4) (i32.const 2))
                (drop (call $fd_write (i32.const 1) (i32.const 0) (i32.const 1) (i32.const 4100)))
            )
        )
        "#,
    )
}

/// Assert that `result` is `Err`, returning the error value or a descriptive `String` failure.
fn expect_err<T: std::fmt::Debug, E>(result: Result<T, E>, msg: &str) -> Result<E, String> {
    match result {
        Err(e) => Ok(e),
        Ok(v) => Err(format!("{msg}: got Ok({v:?})")),
    }
}
