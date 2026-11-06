#![cfg(feature = "wasmtime-executor")]
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! The compatible-capability lifecycle on the real built `runtime.wasm`
//! (Spec 072, Decision 127), driven through the same core-Wasm exports the
//! Swift, Kotlin and .NET hosts call. Also runs
//! `fixtures/cross-host/canonical-input-v1` through `compatible_start`: the
//! canonical input it returns is what a host hands the platform wrapper.

use std::path::PathBuf;
use std::process::Command;

use serde_json::{Value, json};
use wasmtime::{Engine, Instance, Memory, Module, Store, TypedFunc};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/ parent")
        .parent()
        .expect("workspace root")
        .to_path_buf()
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

/// A host for one `runtime.wasm` instance initialized as `demo.camera` in
/// compatible mode on `host_platform`.
struct Bridge {
    store: Store<()>,
    instance: Instance,
    memory: Memory,
}

impl Bridge {
    fn new(runtime_wasm: &[u8], host_platform: &str) -> Self {
        let engine = Engine::default();
        let module = Module::new(&engine, runtime_wasm).expect("runtime.wasm module");
        let mut store = Store::new(&engine, ());
        let instance = Instance::new(&mut store, &module, &[]).expect("instantiate");
        let memory = instance
            .get_memory(&mut store, "memory")
            .expect("memory export");
        let mut bridge = Self {
            store,
            instance,
            memory,
        };
        let header = serde_json::to_vec(&json!({
            "capability_id": "demo.camera",
            "capability_version": "1.0.0",
            "execution_mode": "compatible",
            "platforms": ["ios", "android"],
            "host_platform": host_platform,
        }))
        .expect("header");
        let mut payload = u32::try_from(header.len())
            .expect("len")
            .to_le_bytes()
            .to_vec();
        payload.extend_from_slice(&header);
        let (status, ready) = bridge.call("traverse_init", &payload);
        assert_eq!((status, &ready["status"]), (0, &json!("ready")), "{ready}");
        bridge
    }

    fn func<P: wasmtime::WasmParams, R: wasmtime::WasmResults>(
        &mut self,
        name: &str,
    ) -> TypedFunc<P, R> {
        self.instance
            .get_typed_func(&mut self.store, name)
            .expect(name)
    }

    fn descriptor(&mut self) -> i32 {
        self.func::<i32, i32>("traverse_alloc")
            .call(&mut self.store, 8)
            .expect("alloc descriptor")
    }

    fn read(&mut self, descriptor: i32) -> Value {
        let mut slot = [0u8; 8];
        self.memory
            .read(
                &self.store,
                usize::try_from(descriptor).expect("address"),
                &mut slot,
            )
            .expect("descriptor");
        let ptr = u32::from_le_bytes([slot[0], slot[1], slot[2], slot[3]]) as usize;
        let len = u32::from_le_bytes([slot[4], slot[5], slot[6], slot[7]]) as usize;
        let mut bytes = vec![0u8; len];
        self.memory
            .read(&self.store, ptr, &mut bytes)
            .expect("response");
        serde_json::from_slice(&bytes).expect("response json")
    }

    fn call(&mut self, export: &str, request: &[u8]) -> (i32, Value) {
        let len = i32::try_from(request.len()).expect("len");
        let ptr = self
            .func::<i32, i32>("traverse_alloc")
            .call(&mut self.store, len)
            .expect("alloc");
        self.memory
            .write(
                &mut self.store,
                usize::try_from(ptr).expect("address"),
                request,
            )
            .expect("write request");
        let descriptor = self.descriptor();
        let status = self
            .func::<(i32, i32, i32), i32>(export)
            .call(&mut self.store, (ptr, len, descriptor))
            .expect(export);
        (status, self.read(descriptor))
    }

    fn start(&mut self, input: &str) -> (i32, Value) {
        let request = format!(r#"{{"capability_id":"demo.camera","input":{input}}}"#);
        self.call("traverse_compatible_start", request.as_bytes())
    }

    fn finish(&mut self, export: &str, instance_id: &Value) -> (i32, Value) {
        let request = json!({"capability_id": "demo.camera", "instance_id": instance_id});
        self.call(export, request.to_string().as_bytes())
    }

    fn events(&mut self) -> Vec<Value> {
        let mut events = Vec::new();
        loop {
            let descriptor = self.descriptor();
            let more = self
                .func::<i32, i32>("traverse_next_event")
                .call(&mut self.store, descriptor)
                .expect("next_event");
            if more == 0 {
                return events;
            }
            events.push(self.read(descriptor));
        }
    }

    fn shutdown(&mut self) -> Value {
        let descriptor = self.descriptor();
        let status = self
            .func::<i32, i32>("traverse_shutdown")
            .call(&mut self.store, descriptor)
            .expect("shutdown");
        assert_eq!(status, 0);
        self.read(descriptor)
    }
}

fn event_kinds(events: &[Value]) -> Vec<(&str, &str)> {
    events
        .iter()
        .map(|event| {
            (
                event["type"].as_str().expect("type"),
                event["instance_id"].as_str().expect("instance_id"),
            )
        })
        .collect()
}

#[test]
fn runtime_wasm_runs_the_compatible_lifecycle() {
    let runtime_wasm = std::fs::read(build_runtime_wasm_artifact()).expect("read runtime.wasm");
    let mut bridge = Bridge::new(&runtime_wasm, "ios");

    // Spec 072 acceptance 1: start then stop.
    let (status, started) = bridge.start(r#"{"zoom":2,"lens":"wide"}"#);
    assert_eq!(status, 0, "{started}");
    let first = started["instance_id"].clone();
    assert_eq!(started["status"], "started");
    assert_eq!(started["canonical_input"], r#"{"lens":"wide","zoom":2}"#);
    let (status, stopped) = bridge.finish("traverse_compatible_stop", &Value::Null);
    assert_eq!((status, &stopped["instance_id"]), (0, &first));
    assert_eq!(
        event_kinds(&bridge.events()),
        [
            ("compatible_started", first.as_str().unwrap()),
            ("compatible_stopped", first.as_str().unwrap()),
        ]
    );

    // Acceptance 2: start then kill; a repeated kill is not active and emits nothing.
    let (_, started) = bridge.start("{}");
    let second = started["instance_id"].clone();
    assert_ne!(second, first, "instance identifiers are not reused");
    let (status, killed) = bridge.finish("traverse_compatible_kill", &second);
    assert_eq!((status, &killed["status"]), (0, &json!("killed")));
    bridge.events();
    let (status, again) = bridge.finish("traverse_compatible_kill", &second);
    assert_eq!(
        (status, &again["error"]),
        (-1, &json!("compatible_instance_not_active"))
    );
    assert!(bridge.events().is_empty());

    // Acceptance 3: shutdown kills the running instance and reports it.
    let (_, started) = bridge.start("{}");
    assert_eq!(
        bridge.shutdown(),
        json!({"status": "stopped", "killed_compatible_instances": [started["instance_id"]]})
    );
}

#[test]
fn runtime_wasm_enforces_the_platform_allowlist() {
    let runtime_wasm = std::fs::read(build_runtime_wasm_artifact()).expect("read runtime.wasm");
    let mut bridge = Bridge::new(&runtime_wasm, "web");
    let (status, response) = bridge.start("{}");
    assert_eq!(
        (status, &response["error"]),
        (-1, &json!("platform_not_supported"))
    );
    assert!(bridge.events().is_empty());
}

#[test]
fn compatible_start_returns_canonical_input_for_every_vector() {
    let runtime_wasm = std::fs::read(build_runtime_wasm_artifact()).expect("read runtime.wasm");
    let fixture = workspace_root().join("fixtures/cross-host/canonical-input-v1/vectors.json");
    let fixture: Value =
        serde_json::from_str(&std::fs::read_to_string(fixture).expect("vectors")).expect("json");
    for vector in fixture["vectors"].as_array().expect("vectors") {
        let name = vector["name"].as_str().expect("name");
        let mut bridge = Bridge::new(&runtime_wasm, "ios");
        let (status, response) = bridge.start(vector["input"].as_str().expect("input"));
        if let Some(code) = vector["expected_error"].as_str() {
            assert_eq!(status, -2, "{name}: {response}");
            let error = response["error"].as_str().expect("error text");
            assert!(error.contains(code), "{name}: expected {code}, got {error}");
        } else {
            assert_eq!(status, 0, "{name}: {response}");
            assert_eq!(
                response["canonical_input"], vector["expected_stdin"],
                "{name}: canonical input"
            );
        }
    }
}
