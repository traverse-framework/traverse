#![cfg(feature = "wasmtime-executor")]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::expect_used,
    clippy::unwrap_used
)]

//! Reference run of `fixtures/cross-host/app-state-machine-events-v1` (Spec 139,
//! Spec 140 FR-013/FR-014) against the real built `runtime.wasm`.
//!
//! Drives the bridge ABI with its own thin Wasmtime client (the same posture as
//! `native_bridge_conformance.rs`) and compares the ordered, id-normalized
//! runtime events per step with `golden.json`. Every published embedder must
//! reproduce the same golden log; this test is the reference that produced it.
//!
//! Regenerate after an intentional runtime change with
//! `TRAVERSE_UPDATE_GOLDEN=1 cargo test -p traverse-runtime --test app_state_machine_conformance`
//! and review the diff: the golden is the cross-host contract.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;

use serde_json::{Map, Value, json};
use wasmtime::{Engine, Instance, Memory, Module, Store, TypedFunc};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/ parent")
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

fn fixture_dir() -> PathBuf {
    workspace_root().join("fixtures/cross-host/app-state-machine-events-v1")
}

/// Same shared, uninstrumented wasm32 build as `native_bridge_conformance.rs`.
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
    let artifact = target_dir
        .join("wasm32-unknown-unknown")
        .join("debug")
        .join("traverse_runtime_wasm.wasm");
    assert!(artifact.is_file(), "expected {}", artifact.display());
    artifact
}

struct Bridge {
    store: Store<()>,
    memory: Memory,
    alloc: TypedFunc<i32, i32>,
    init: TypedFunc<(i32, i32, i32), i32>,
    submit: TypedFunc<(i32, i32, i32), i32>,
    next_event: TypedFunc<i32, i32>,
}

impl Bridge {
    fn new(module: &Module, engine: &Engine) -> Self {
        let mut store = Store::new(engine, ());
        let instance = Instance::new(&mut store, module, &[]).expect("instantiate without WASI");
        Self {
            memory: instance.get_memory(&mut store, "memory").expect("memory"),
            alloc: instance
                .get_typed_func(&mut store, "traverse_alloc")
                .expect("alloc"),
            init: instance
                .get_typed_func(&mut store, "traverse_init")
                .expect("init"),
            submit: instance
                .get_typed_func(&mut store, "traverse_submit")
                .expect("submit"),
            next_event: instance
                .get_typed_func(&mut store, "traverse_next_event")
                .expect("next_event"),
            store,
        }
    }

    /// Returns the guest status code and the decoded response JSON. A rejected
    /// submit returns a non-zero status with the response still written.
    fn call(&mut self, function: &str, payload: &[u8]) -> (i32, Value) {
        let pointer = self
            .alloc
            .call(&mut self.store, payload.len() as i32)
            .expect("allocate request");
        self.memory
            .write(&mut self.store, pointer as usize, payload)
            .expect("write request");
        let args = (pointer, payload.len() as i32, 1024);
        let status = match function {
            "init" => self.init.call(&mut self.store, args),
            "submit" => self.submit.call(&mut self.store, args),
            _ => unreachable!("known call"),
        }
        .expect("bridge call");
        (status, self.read_json(1024))
    }

    fn read_json(&self, descriptor: usize) -> Value {
        let data = self.memory.data(&self.store);
        let pointer = u32::from_le_bytes(data[descriptor..descriptor + 4].try_into().unwrap());
        let length = u32::from_le_bytes(data[descriptor + 4..descriptor + 8].try_into().unwrap());
        serde_json::from_slice(&data[pointer as usize..(pointer + length) as usize])
            .expect("valid response JSON")
    }

    fn drain_events(&mut self) -> Vec<Value> {
        let mut events = Vec::new();
        while self
            .next_event
            .call(&mut self.store, 1024)
            .expect("drain event")
            == 1
        {
            events.push(self.read_json(1024));
        }
        events
    }
}

/// Normalizes runtime-assigned ids to `$S<n>` / `$C<n>` by first appearance.
#[derive(Default)]
struct Placeholders {
    sessions: Vec<String>,
    commands: Vec<String>,
}

impl Placeholders {
    fn name(list: &mut Vec<String>, prefix: &str, id: &str) -> String {
        let index = list
            .iter()
            .position(|known| known == id)
            .unwrap_or_else(|| {
                list.push(id.to_string());
                list.len() - 1
            });
        format!("{prefix}{}", index + 1)
    }

    fn normalize(&mut self, value: &Value) -> Value {
        match value {
            Value::Object(map) => {
                let mut out = Map::new();
                for (key, inner) in map {
                    let replaced = match (key.as_str(), inner.as_str()) {
                        ("session_id", Some(id)) => json!(Self::name(&mut self.sessions, "$S", id)),
                        ("command_id", Some(id)) => json!(Self::name(&mut self.commands, "$C", id)),
                        _ => self.normalize(inner),
                    };
                    out.insert(key.clone(), replaced);
                }
                Value::Object(out)
            }
            Value::Array(items) => {
                Value::Array(items.iter().map(|item| self.normalize(item)).collect())
            }
            other => other.clone(),
        }
    }

    fn resolve(list: &[String], placeholder: &str, prefix: &str) -> String {
        let index: usize = placeholder
            .strip_prefix(prefix)
            .and_then(|n| n.parse().ok())
            .expect("valid placeholder");
        list[index - 1].clone()
    }
}

fn init_payload(fixture: &Value) -> Vec<u8> {
    let header = serde_json::to_vec(&fixture["init_header"]).expect("serialize header");
    let mut payload = u32::try_from(header.len())
        .expect("fits")
        .to_le_bytes()
        .to_vec();
    payload.extend_from_slice(&header);
    payload
}

/// Runs one scenario and returns its per-step normalized transcript.
fn run_scenario(bridge: &mut Bridge, init: &[u8], scenario: &Value) -> Value {
    let (status, response) = bridge.call("init", init);
    assert_eq!(status, 0, "init rejected: {response}");
    let mut names = Placeholders::default();
    let mut pending: Vec<(String, String)> = Vec::new(); // (session, command_id) in arrival order
    let mut transcript = Vec::new();

    for (index, step) in scenario["steps"]
        .as_array()
        .expect("steps")
        .iter()
        .enumerate()
    {
        let (kind, spec) = step
            .as_object()
            .and_then(|m| m.iter().next())
            .expect("step");
        let target_wait =
            |names: &Placeholders, pending: &[(String, String)]| -> (String, String) {
                match spec.get("command").and_then(Value::as_str) {
                    Some(placeholder) => {
                        let command_id = Placeholders::resolve(&names.commands, placeholder, "$C");
                        pending
                            .iter()
                            .find(|(_, id)| *id == command_id)
                            .cloned()
                            .expect("known wait")
                    }
                    None => pending.last().cloned().expect("a pending wait"),
                }
            };
        let request = match kind.as_str() {
            "submit" => {
                let mut envelope = json!({
                    "kind": "app_command",
                    "command": spec["command"],
                    "payload": spec["payload"],
                });
                if let Some(placeholder) = spec["session"].as_str() {
                    envelope["session_id"] =
                        json!(Placeholders::resolve(&names.sessions, placeholder, "$S"));
                }
                envelope
            }
            "complete" => {
                let (session, command_id) = target_wait(&names, &pending);
                json!({
                    "kind": "host_connector_result",
                    "command_id": command_id,
                    "session_id": session,
                    "result_class": spec["result_class"],
                    "payload": spec["payload"],
                })
            }
            "fire_deadline" => {
                let (session, command_id) = target_wait(&names, &pending);
                json!({"kind": "deadline_fired", "command_id": command_id, "session_id": session})
            }
            other => unreachable!("unknown step kind {other}"),
        };

        let (status_code, response) = bridge.call("submit", request.to_string().as_bytes());
        for wait in response["pending_host_connector"]
            .as_array()
            .into_iter()
            .flatten()
        {
            pending.push((
                wait["session_id"].as_str().expect("session").to_string(),
                wait["command_id"].as_str().expect("command").to_string(),
            ));
        }
        let events = bridge.drain_events();
        transcript.push(json!({
            "step": index,
            "kind": kind,
            "guest_status": status_code,
            "response": names.normalize(&response),
            "events": names.normalize(&Value::Array(events)),
        }));
    }
    Value::Array(transcript)
}

#[test]
fn real_runtime_wasm_reproduces_the_golden_ordered_event_log() {
    let fixture: Value = serde_json::from_slice(
        &std::fs::read(fixture_dir().join("fixture.json")).expect("read fixture"),
    )
    .expect("fixture is JSON");
    let runtime = std::fs::read(build_runtime_wasm_artifact()).expect("read runtime.wasm");
    let engine = Engine::default();
    let module = Module::new(&engine, &runtime).expect("compile real runtime.wasm");
    let init = init_payload(&fixture);

    let mut actual = BTreeMap::new();
    for scenario in fixture["scenarios"].as_array().expect("scenarios") {
        let mut bridge = Bridge::new(&module, &engine);
        actual.insert(
            scenario["id"].as_str().expect("id").to_string(),
            run_scenario(&mut bridge, &init, scenario),
        );
    }
    let actual = json!({ "golden_version": "1.0.0", "scenarios": actual });

    let golden_path = fixture_dir().join("golden.json");
    if std::env::var_os("TRAVERSE_UPDATE_GOLDEN").is_some() {
        let mut text = serde_json::to_string_pretty(&actual).expect("serialize golden");
        text.push('\n');
        std::fs::write(&golden_path, text).expect("write golden");
        return;
    }
    let golden: Value =
        serde_json::from_slice(&std::fs::read(&golden_path).expect("read golden.json"))
            .expect("golden is JSON");
    assert_eq!(
        first_divergence(&actual, &golden),
        None,
        "runtime events diverged from golden.json"
    );
}

/// Names the first difference as `scenario / step / event index` so a failing
/// host points at the exact diverging event instead of dumping both logs.
fn first_divergence(actual: &Value, golden: &Value) -> Option<String> {
    let (actual_all, golden_all) = (&actual["scenarios"], &golden["scenarios"]);
    for (id, golden_steps) in golden_all.as_object().expect("golden scenarios") {
        let Some(actual_steps) = actual_all.get(id).and_then(Value::as_array) else {
            return Some(format!("scenario {id}: missing from the actual run"));
        };
        let golden_steps = golden_steps.as_array().expect("golden steps");
        if actual_steps.len() != golden_steps.len() {
            return Some(format!(
                "scenario {id}: {} steps, expected {}",
                actual_steps.len(),
                golden_steps.len()
            ));
        }
        for (index, (got, want)) in actual_steps.iter().zip(golden_steps).enumerate() {
            for field in ["kind", "guest_status", "response"] {
                if got[field] != want[field] {
                    return Some(format!(
                        "scenario {id} step {index} {field}: expected {}, got {}",
                        want[field], got[field]
                    ));
                }
            }
            let (got_events, want_events) = (
                got["events"].as_array().expect("events"),
                want["events"].as_array().expect("events"),
            );
            for event_index in 0..got_events.len().max(want_events.len()) {
                if got_events.get(event_index) != want_events.get(event_index) {
                    return Some(format!(
                        "scenario {id} step {index} event {event_index}: expected {}, got {}",
                        want_events
                            .get(event_index)
                            .map_or("<none>".into(), Value::to_string),
                        got_events
                            .get(event_index)
                            .map_or("<none>".into(), Value::to_string),
                    ));
                }
            }
        }
    }
    let extra: Vec<&String> = actual_all
        .as_object()
        .expect("actual scenarios")
        .keys()
        .filter(|id| golden_all.get(id.as_str()).is_none())
        .collect();
    (!extra.is_empty()).then(|| format!("scenarios not in golden.json: {extra:?}"))
}
