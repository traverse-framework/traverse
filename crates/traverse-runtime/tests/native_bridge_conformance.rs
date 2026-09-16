#![cfg(feature = "wasmtime-executor")]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::expect_used,
    clippy::unwrap_used
)]

//! Proves the real, built `crates/traverse-runtime-wasm` `wasm32-unknown-
//! unknown` artifact — the one `crates/traverse-native-bridge` ships — is a
//! conformant `runtime-wasm-bridge/1.0.0` guest (spec 071 FR-006): its ABI
//! version, alloc/dealloc, and `init/submit/next_event/shutdown` lifecycle
//! transcript match what every native host adapter (Swift/wasmi,
//! Kotlin/Chicory, .NET/Wasmtime) drives independently (issue #1420, spec
//! 1402 FR-004). Unlike `tests/runtime_wasm_host_tests.rs`, this test drives
//! the ABI with its own minimal Wasmtime calls rather than the production
//! `RuntimeWasmHost` driver — the same "roll your own thin ABI client"
//! posture every native package takes, so a bug in `RuntimeWasmHost` itself
//! could never mask a real bridge nonconformance.

use std::path::PathBuf;
use std::process::Command;

use serde_json::{Value, json};
use wasmtime::{Engine, Instance, Memory, Module, Store, TypedFunc};

/// A WASI-command capability that echoes stdin to stdout, then calls
/// `traverse_host::emit_event` with a fixed declared domain event — the same
/// fixture shape `tests/runtime_wasm_host_tests.rs` uses, so both
/// conformance paths exercise identical nested-capability behavior.
const NESTED_CAPABILITY_WAT: &str = r#"
  (module
    (import "wasi_snapshot_preview1" "fd_read"
      (func $fd_read (param i32 i32 i32 i32) (result i32)))
    (import "wasi_snapshot_preview1" "fd_write"
      (func $fd_write (param i32 i32 i32 i32) (result i32)))
    (import "traverse_host" "emit_event"
      (func $emit_event (param i32 i32) (result i32)))
    (memory (export "memory") 1)
    (data (i32.const 5000) "{\"event_id\":\"conformance.echoed\",\"version\":\"1.0.0\",\"payload\":{\"ok\":true}}")
    (func (export "_start")
      (i32.store (i32.const 0) (i32.const 8))
      (i32.store (i32.const 4) (i32.const 1024))
      (drop (call $fd_read (i32.const 0) (i32.const 0) (i32.const 1) (i32.const 4100)))
      (i32.store (i32.const 0) (i32.const 8))
      (i32.store (i32.const 4) (i32.load (i32.const 4100)))
      (drop (call $fd_write (i32.const 1) (i32.const 0) (i32.const 1) (i32.const 4104)))
      (drop (call $emit_event (i32.const 5000) (i32.const 73)))
    )
  )
"#;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/ parent")
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

/// Builds `crates/traverse-runtime-wasm` for `wasm32-unknown-unknown` and
/// returns the path to the resulting `.wasm` cdylib. Shares
/// `tests/runtime_wasm_host_tests.rs`'s dedicated target directory (cargo's
/// own locking serializes concurrent builds against it safely, and a shared
/// directory lets a build already done for that test satisfy this one too)
/// rather than whatever `CARGO_TARGET_DIR` the outer `cargo test` invocation
/// has — sharing that one races the outer build's own lock.
fn build_runtime_wasm_artifact() -> PathBuf {
    let target_dir = workspace_root().join("target/runtime-wasm-host-test");

    // Nested wasm32 builds must not inherit the outer llvm-cov /
    // instrument-coverage RUSTFLAGS — the wasm32 target has no
    // `profiler_builtins`, so coverage instrumentation fails the compile.
    // Likewise clear wrapper env that only applies to the host triple.
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

    let artifact = target_dir
        .join("wasm32-unknown-unknown")
        .join("debug")
        .join("traverse_runtime_wasm.wasm");
    assert!(
        artifact.is_file(),
        "expected built artifact at {}",
        artifact.display()
    );
    artifact
}

/// Builds `traverse_init`'s wire payload: a 4-byte little-endian header
/// length, that many bytes of JSON metadata, then the raw nested-capability
/// artifact bytes (`crates/traverse-runtime-wasm/src/lib.rs`'s documented
/// `parse_init_payload` layout).
fn init_payload_bytes(capability_id: &str, service_type: &str, artifact: &[u8]) -> Vec<u8> {
    let header = json!({
        "capability_id": capability_id,
        "capability_version": "1.0.0",
        "service_type": service_type,
        "emits": [{"event_id": "conformance.echoed", "version": "1.0.0"}],
        "host_placement_target": "local",
        "permitted_targets": ["local"],
    });
    let header_bytes = serde_json::to_vec(&header).expect("serialize init header");
    let header_len = u32::try_from(header_bytes.len()).expect("header fits in u32");
    let mut payload = header_len.to_le_bytes().to_vec();
    payload.extend_from_slice(&header_bytes);
    payload.extend_from_slice(artifact);
    payload
}

struct Bridge {
    store: Store<()>,
    memory: Memory,
    alloc: TypedFunc<i32, i32>,
    init: TypedFunc<(i32, i32, i32), i32>,
    submit: TypedFunc<(i32, i32, i32), i32>,
    next_event: TypedFunc<i32, i32>,
    shutdown: TypedFunc<i32, i32>,
}

impl Bridge {
    /// Writes `payload` into the guest, calls `function` with
    /// `(ptr, len, out_descriptor)`, and returns the decoded JSON response —
    /// the same calling convention every native host adapter uses.
    fn call_raw(&mut self, function: &str, payload: &[u8]) -> Value {
        let pointer = self
            .alloc
            .call(&mut self.store, payload.len() as i32)
            .expect("allocate request");
        self.memory
            .write(&mut self.store, pointer as usize, payload)
            .expect("write request");
        let status = match function {
            "init" => self
                .init
                .call(&mut self.store, (pointer, payload.len() as i32, 1024)),
            "submit" => self
                .submit
                .call(&mut self.store, (pointer, payload.len() as i32, 1024)),
            _ => unreachable!("known fixture call"),
        }
        .expect("bridge call");
        assert_eq!(status, 0, "guest rejected {function}");
        self.read_json(1024)
    }

    fn read_json(&self, descriptor: usize) -> Value {
        let data = self.memory.data(&self.store);
        let pointer = u32::from_le_bytes(data[descriptor..descriptor + 4].try_into().unwrap());
        let length = u32::from_le_bytes(data[descriptor + 4..descriptor + 8].try_into().unwrap());
        serde_json::from_slice(&data[pointer as usize..(pointer + length) as usize])
            .expect("valid response JSON")
    }
}

#[test]
fn core_wasm_bridge_produces_the_cross_platform_lifecycle_transcript() {
    let runtime_wasm_bytes =
        std::fs::read(build_runtime_wasm_artifact()).expect("read built runtime.wasm");
    let nested_capability = wat::parse_str(NESTED_CAPABILITY_WAT).expect("wat parses");

    let engine = Engine::default();
    let module = Module::new(&engine, &runtime_wasm_bytes).expect("compile real runtime.wasm");
    let mut store = Store::new(&engine, ());
    let instance = Instance::new(&mut store, &module, &[]).expect("instantiate without WASI");
    let version = instance
        .get_typed_func::<(), i32>(&mut store, "traverse_bridge_abi_version")
        .expect("version export")
        .call(&mut store, ())
        .expect("read ABI version");
    assert_eq!(version, 10_100);

    let mut bridge = Bridge {
        memory: instance
            .get_memory(&mut store, "memory")
            .expect("memory export"),
        alloc: instance
            .get_typed_func(&mut store, "traverse_alloc")
            .expect("alloc export"),
        init: instance
            .get_typed_func(&mut store, "traverse_init")
            .expect("init export"),
        submit: instance
            .get_typed_func(&mut store, "traverse_submit")
            .expect("submit export"),
        next_event: instance
            .get_typed_func(&mut store, "traverse_next_event")
            .expect("event export"),
        shutdown: instance
            .get_typed_func(&mut store, "traverse_shutdown")
            .expect("shutdown export"),
        store,
    };

    let init_payload = init_payload_bytes("conformance.echo", "subscribable", &nested_capability);
    assert_eq!(bridge.call_raw("init", &init_payload)["status"], "ready");
    assert_eq!(
        bridge.call_raw("submit", br#"{"hello":"conformance"}"#)["status"],
        "accepted"
    );

    let mut event_types = Vec::new();
    loop {
        let status = bridge
            .next_event
            .call(&mut bridge.store, 1024)
            .expect("drain event");
        if status == 0 {
            break;
        }
        assert_eq!(status, 1);
        event_types.push(bridge.read_json(1024)["type"].as_str().unwrap().to_owned());
    }
    assert_eq!(
        event_types,
        [
            "capability_invoked",
            "conformance.echoed",
            "capability_result"
        ]
    );

    assert_eq!(bridge.shutdown.call(&mut bridge.store, 1024).unwrap(), 0);
    assert_eq!(bridge.read_json(1024)["status"], "stopped");
}
