#![cfg(feature = "wasmtime-executor")]
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! Reference run of `fixtures/cross-host/canonical-input-v1` (Spec 1722
//! FR-006, Decision 118) on the two places a host writes guest stdin: the
//! native wasmtime executor and the nested executor inside the real built
//! `runtime.wasm` (which the web, Node, Swift, Kotlin and .NET hosts run).
//! Every host must agree on each vector's bytes, digest, or rejection.

use std::fmt::Write as _;
use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;
use sha2::{Digest, Sha256};
use traverse_contracts::{
    ExecutionTarget, INPUT_NOT_CANONICALIZABLE, ServiceType, canonicalize_json_bytes,
    parse_canonicalizable,
};
use traverse_runtime::executor::ExecutorError;
use traverse_runtime::executor::wasm::WasmExecutor;
use traverse_runtime::parse_runtime_request;
use traverse_runtime::runtime_wasm_host::{
    CapabilityInit, RUNTIME_WASM_HOST_FUEL_BUDGET, RUNTIME_WASM_HOST_MEMORY_LIMIT_BYTES,
    RuntimeWasmHost, RuntimeWasmHostLimits,
};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/ parent")
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

fn fixture_dir() -> PathBuf {
    workspace_root().join("fixtures/cross-host/canonical-input-v1")
}

fn vectors() -> Vec<Value> {
    let text = std::fs::read_to_string(fixture_dir().join("vectors.json")).expect("vectors");
    let fixture: Value = serde_json::from_str(&text).expect("vectors parse");
    fixture["vectors"].as_array().expect("vector list").clone()
}

fn guest() -> Vec<u8> {
    std::fs::read(fixture_dir().join("stdin-echo.wasm")).expect("stdin-echo.wasm")
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .fold(String::new(), |mut acc, byte| {
            let _ = write!(acc, "{byte:02x}");
            acc
        })
}

/// Checks one host's result for `vector`: the echoed stdin on success, or
/// an error message carrying the expected stable code.
fn assert_vector(host: &str, vector: &Value, result: Result<Value, String>) {
    let name = vector["name"].as_str().expect("name");
    match (vector["expected_error"].as_str(), result) {
        (Some(code), Err(message)) => assert!(
            message.contains(code),
            "{host}/{name}: expected {code}, got {message}"
        ),
        (None, Ok(output)) => {
            let stdin = output["stdin"].as_str().expect("echoed stdin");
            assert_eq!(
                stdin, vector["expected_stdin"],
                "{host}/{name}: stdin bytes"
            );
            assert_eq!(
                sha256_hex(stdin.as_bytes()),
                vector["expected_sha256"].as_str().expect("digest"),
                "{host}/{name}: digest"
            );
        }
        (expected, actual) => panic!("{host}/{name}: expected {expected:?}, got {actual:?}"),
    }
}

#[test]
fn committed_guest_matches_its_wat_source() {
    let source = fixture_dir().join("stdin-echo.wat");
    assert_eq!(wat::parse_file(source).expect("wat parses"), guest());
}

#[test]
fn vectors_match_the_rust_reference() {
    for vector in vectors() {
        let input = vector["input"].as_str().expect("input");
        match canonicalize_json_bytes(input.as_bytes()) {
            Ok(canonical) => assert_eq!(canonical, vector["expected_stdin"]),
            Err(error) => assert_eq!(error.code(), vector["expected_error"]),
        }
    }
}

#[test]
fn native_executor_feeds_canonical_stdin() {
    let executor = WasmExecutor::new().expect("executor");
    let guest = guest();
    for vector in vectors() {
        let input = vector["input"].as_str().expect("input");
        // A native caller hands the runtime parsed JSON; duplicate names and
        // invalid Unicode are rejected where it parses bytes (see below).
        let result = parse_canonicalizable(input.as_bytes())
            .map_err(|error| error.to_string())
            .and_then(|value| {
                executor
                    .run_bytes(&guest, &value)
                    .map_err(|error| error.to_string())
            });
        assert_vector("native", &vector, result);
    }
}

#[test]
fn native_executor_rejects_unsafe_numbers_with_a_typed_error() {
    let executor = WasmExecutor::new().expect("executor");
    let error = executor
        .run_bytes(&guest(), &serde_json::json!({"id": u64::MAX}))
        .expect_err("unsafe integer must be rejected");
    assert!(matches!(error, ExecutorError::InputNotCanonicalizable(_)));
    assert!(error.to_string().starts_with(INPUT_NOT_CANONICALIZABLE));
}

#[test]
fn runtime_request_parsing_rejects_bytes_jcs_cannot_represent() {
    for vector in vectors() {
        let input = vector["input"].as_str().expect("input");
        let request = format!(
            r#"{{"kind":"runtime_request","schema_version":"1.0.0","request_id":"r",
                "intent":{{"capability_id":"x","capability_version":"1.0.0"}},
                "input":{input},"lookup":{{"scope":"public_only"}},
                "context":{{"requested_target":"local"}}}}"#
        );
        let parsed = parse_runtime_request(&request);
        let name = vector["name"].as_str().expect("name");
        match name {
            "duplicate_member_name" | "lone_surrogate" => {
                let failure = parsed.expect_err(name);
                assert!(
                    failure.message.starts_with(INPUT_NOT_CANONICALIZABLE),
                    "{name}"
                );
            }
            _ => {
                if let Err(failure) = parsed {
                    assert!(
                        !failure.message.starts_with(INPUT_NOT_CANONICALIZABLE),
                        "{name}: byte boundary must only reject duplicates and invalid Unicode"
                    );
                }
            }
        }
    }
}

/// Same shared, uninstrumented wasm32 build as `runtime_wasm_host_tests.rs`.
fn build_runtime_wasm_artifact() -> PathBuf {
    let target_dir = workspace_root().join("target/runtime-wasm-host-test");
    let status = Command::new("cargo")
        .args([
            "build",
            "-p",
            "traverse-runtime-wasm",
            "--target",
            "wasm32-unknown-unknown",
        ])
        .env("CARGO_TARGET_DIR", &target_dir)
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("RUSTC_WRAPPER")
        .env_remove("RUSTC_WORKSPACE_WRAPPER")
        .current_dir(workspace_root())
        .status()
        .expect("cargo build -p traverse-runtime-wasm must run");
    assert!(status.success(), "building traverse-runtime-wasm failed");
    target_dir
        .join("wasm32-unknown-unknown")
        .join("debug")
        .join("traverse_runtime_wasm.wasm")
}

fn submit_to_runtime_wasm(
    runtime_wasm: &[u8],
    guest: &[u8],
    input: &[u8],
) -> Result<Value, String> {
    let mut host = RuntimeWasmHost::instantiate(
        runtime_wasm,
        RuntimeWasmHostLimits {
            fuel_budget: RUNTIME_WASM_HOST_FUEL_BUDGET,
            memory_bytes: RUNTIME_WASM_HOST_MEMORY_LIMIT_BYTES,
        },
    )
    .expect("instantiate runtime.wasm");
    host.init(
        CapabilityInit {
            capability_id: "conformance.stdin-echo",
            capability_version: "1.0.0",
            service_type: &ServiceType::Stateless,
            declared_emits: &[],
            host_placement_target: &ExecutionTarget::Local,
            permitted_targets: &[ExecutionTarget::Local],
        },
        guest,
    )
    .expect("traverse_init succeeds");
    host.submit(input).map_err(|error| error.to_string())?;
    let events = host.drain_events().expect("drain events");
    let result = events
        .iter()
        .find(|event| event["type"] == "capability_result")
        .expect("a capability_result event");
    match result["data"]["status"].as_str() {
        Some("completed") => Ok(result["data"]["output"].clone()),
        _ => Err(result["data"]["error"].to_string()),
    }
}

#[test]
fn runtime_wasm_feeds_canonical_stdin() {
    let runtime_wasm = std::fs::read(build_runtime_wasm_artifact()).expect("read runtime.wasm");
    let guest = guest();
    for vector in vectors() {
        let input = vector["input"].as_str().expect("input");
        let result = submit_to_runtime_wasm(&runtime_wasm, &guest, input.as_bytes());
        assert_vector("runtime.wasm", &vector, result);
    }
}
