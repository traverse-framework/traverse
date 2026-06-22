use serde_json::json;
use sha2::{Digest, Sha256};
use traverse_runtime::executor::{
    ArtifactType, CapabilityExecutor, ExecutorCapability, ExecutorError, NativeExecutor,
    SUPPORTED_HOST_ABI_VERSION, WasmExecutor, supported_host_abi_versions,
    verify_wasm_host_abi_bytes,
};

// --- NativeExecutor tests ---

#[test]
fn native_executor_runs_handler() {
    let executor = NativeExecutor::new(|input| {
        let name = input["name"].as_str().unwrap_or("world");
        Ok(json!({ "greeting": format!("hello, {name}!") }))
    });

    let cap = native_capability("greet");
    let result = executor.execute(&cap, &json!({ "name": "traverse" }));

    assert_eq!(result, Ok(json!({ "greeting": "hello, traverse!" })));
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

    assert_eq!(result, input);
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
    };

    let input = json!({ "disk": true });
    let result = executor.execute(&cap, &input).map_err(|e| format!("{e:?}"));
    std::fs::remove_file(&tmp).ok();

    assert_eq!(result, Ok(input));
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
    let checksum = format!("{:x}", hasher.finalize());

    let tmp = tempfile_path();
    std::fs::write(&tmp, &wasm_bytes).map_err(|e| format!("write: {e}"))?;

    let cap = ExecutorCapability {
        capability_id: "checksum-ok".to_string(),
        artifact_type: ArtifactType::Wasm,
        wasm_binary_path: Some(tmp.clone()),
        wasm_checksum: Some(checksum),
        host_abi_version: Some("1.0.0".to_string()),
    };

    let result = executor
        .execute(&cap, &json!({}))
        .map_err(|e| format!("{e:?}"));
    std::fs::remove_file(&tmp).ok();

    assert_eq!(result, Ok(json!({})));
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
    };

    let err = expect_err(executor.execute(&cap, &json!({})), "expected error")?;
    std::fs::remove_file(&tmp).ok();

    assert!(
        matches!(err, ExecutorError::MalformedWasmArtifact { .. }),
        "expected MalformedWasmArtifact, got {err:?}"
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
    }
}

fn tempfile_path() -> String {
    format!(
        "/tmp/traverse-test-{}.wasm",
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

/// Assert that `result` is `Err`, returning the error value or a descriptive `String` failure.
fn expect_err<T: std::fmt::Debug, E>(result: Result<T, E>, msg: &str) -> Result<E, String> {
    match result {
        Err(e) => Ok(e),
        Ok(v) => Err(format!("{msg}: got Ok({v:?})")),
    }
}
