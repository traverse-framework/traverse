#![cfg(feature = "wasmtime-executor")]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::expect_used,
    clippy::format_collect,
    clippy::format_push_string,
    clippy::too_many_lines,
    clippy::unwrap_used
)]

use serde_json::{Value, json};
use wasmtime::{Engine, Instance, Memory, Module, Store, TypedFunc};

const START: &str = r#"{"instance_id":"fixture-compatible-1","status":"started","error":null}"#;
const START_EVENT: &str = r#"{"type":"compatible_started","instance_id":"fixture-compatible-1"}"#;
const STOP: &str = r#"{"instance_id":"fixture-compatible-1","status":"stopped","error":null}"#;
const STOP_EVENT: &str = r#"{"type":"compatible_stopped","instance_id":"fixture-compatible-1"}"#;
const KILL: &str = r#"{"instance_id":"fixture-compatible-1","status":"killed","error":null}"#;
const KILL_EVENT: &str = r#"{"type":"compatible_killed","instance_id":"fixture-compatible-1"}"#;
const SHUTDOWN_KILLED: &str =
    r#"{"status":"stopped","killed_compatible_instances":["fixture-compatible-1"]}"#;
const SHUTDOWN_EMPTY: &str = r#"{"status":"stopped","killed_compatible_instances":[]}"#;

fn wat_string(value: &str) -> String {
    value
        .as_bytes()
        .iter()
        .map(|byte| format!("\\{byte:02x}"))
        .collect()
}

fn fixture_module() -> String {
    let values = [
        START,
        START_EVENT,
        STOP,
        STOP_EVENT,
        KILL,
        KILL_EVENT,
        SHUTDOWN_KILLED,
        SHUTDOWN_EMPTY,
    ];
    let mut offset = 8192_u32;
    let mut regions = Vec::new();
    let mut data = String::new();
    for value in values {
        regions.push((offset, value.len()));
        data.push_str(&format!(
            "(data (i32.const {offset}) \"{}\")\n",
            wat_string(value)
        ));
        offset += value.len() as u32;
    }
    let [
        (start_p, start_l),
        (start_event_p, start_event_l),
        (stop_p, stop_l),
        (stop_event_p, stop_event_l),
        (kill_p, kill_l),
        (kill_event_p, kill_event_l),
        (shutdown_killed_p, shutdown_killed_l),
        (shutdown_empty_p, shutdown_empty_l),
    ] = regions.as_slice()
    else {
        unreachable!("fixture regions are fixed")
    };

    format!(
        r#"(module
          (memory (export "memory") 1 8)
          (global $heap (mut i32) (i32.const 4096))
          (global $active (mut i32) (i32.const 0))
          (global $event (mut i32) (i32.const 0))
          {data}
          (func (export "traverse_bridge_abi_version") (result i32) i32.const 10100)
          (func (export "traverse_alloc") (param $len i32) (result i32)
            (local $ptr i32)
            global.get $heap local.set $ptr
            global.get $heap local.get $len i32.add global.set $heap
            local.get $ptr)
          (func (export "traverse_dealloc") (param i32 i32))
          (func $descriptor (param $out i32) (param $ptr i32) (param $len i32)
            local.get $out local.get $ptr i32.store
            local.get $out i32.const 4 i32.add local.get $len i32.store)
          (func (export "traverse_compatible_start") (param i32 i32 i32) (result i32)
            global.get $active
            if (result i32)
              i32.const -1
            else
              i32.const 1 global.set $active
              i32.const 1 global.set $event
              local.get 2 i32.const {start_p} i32.const {start_l} call $descriptor
              i32.const 0
            end)
          (func (export "traverse_compatible_stop") (param i32 i32 i32) (result i32)
            global.get $active i32.eqz
            if (result i32)
              i32.const -1
            else
              i32.const 0 global.set $active
              i32.const 2 global.set $event
              local.get 2 i32.const {stop_p} i32.const {stop_l} call $descriptor
              i32.const 0
            end)
          (func (export "traverse_compatible_kill") (param i32 i32 i32) (result i32)
            global.get $active i32.eqz
            if (result i32)
              i32.const -1
            else
              i32.const 0 global.set $active
              i32.const 3 global.set $event
              local.get 2 i32.const {kill_p} i32.const {kill_l} call $descriptor
              i32.const 0
            end)
          (func (export "traverse_next_event") (param $out i32) (result i32)
            global.get $event i32.const 1 i32.eq
            if
              local.get $out i32.const {start_event_p} i32.const {start_event_l} call $descriptor
              i32.const 0 global.set $event
              i32.const 1 return
            end
            global.get $event i32.const 2 i32.eq
            if
              local.get $out i32.const {stop_event_p} i32.const {stop_event_l} call $descriptor
              i32.const 0 global.set $event
              i32.const 1 return
            end
            global.get $event i32.const 3 i32.eq
            if
              local.get $out i32.const {kill_event_p} i32.const {kill_event_l} call $descriptor
              i32.const 0 global.set $event
              i32.const 1 return
            end
            i32.const 0)
          (func (export "traverse_shutdown") (param i32) (result i32)
            global.get $active
            if
              i32.const 0 global.set $active
              i32.const 0 global.set $event
              local.get 0 i32.const {shutdown_killed_p} i32.const {shutdown_killed_l} call $descriptor
            else
              local.get 0 i32.const {shutdown_empty_p} i32.const {shutdown_empty_l} call $descriptor
            end
            i32.const 0))"#
    )
}

struct Bridge {
    store: Store<()>,
    memory: Memory,
    alloc: TypedFunc<i32, i32>,
    start: TypedFunc<(i32, i32, i32), i32>,
    stop: TypedFunc<(i32, i32, i32), i32>,
    kill: TypedFunc<(i32, i32, i32), i32>,
    next_event: TypedFunc<i32, i32>,
    shutdown: TypedFunc<i32, i32>,
}

impl Bridge {
    fn call(&mut self, operation: &str, input: &Value) -> (i32, Option<Value>) {
        let bytes = serde_json::to_vec(input).expect("serialize fixture request");
        let length = i32::try_from(bytes.len()).expect("fixture input length");
        let pointer = self
            .alloc
            .call(&mut self.store, length)
            .expect("allocate input");
        self.memory
            .write(&mut self.store, pointer as usize, &bytes)
            .expect("write input");
        let arguments = (pointer, length, 1024);
        let status = match operation {
            "start" => self.start.call(&mut self.store, arguments),
            "stop" => self.stop.call(&mut self.store, arguments),
            "kill" => self.kill.call(&mut self.store, arguments),
            _ => unreachable!("known fixture operation"),
        }
        .expect("bridge call");
        let output = (status == 0).then(|| self.read_json(1024));
        (status, output)
    }

    fn event(&mut self) -> Option<Value> {
        let status = self
            .next_event
            .call(&mut self.store, 1024)
            .expect("drain event");
        (status == 1).then(|| self.read_json(1024))
    }

    fn read_json(&self, descriptor: usize) -> Value {
        let data = self.memory.data(&self.store);
        let pointer = u32::from_le_bytes(data[descriptor..descriptor + 4].try_into().unwrap());
        let length = u32::from_le_bytes(data[descriptor + 4..descriptor + 8].try_into().unwrap());
        serde_json::from_slice(&data[pointer as usize..(pointer + length) as usize])
            .expect("valid fixture JSON")
    }
}

fn bridge() -> Bridge {
    let engine = Engine::default();
    let module = Module::new(&engine, fixture_module()).expect("compile bridge fixture");
    let mut store = Store::new(&engine, ());
    let instance = Instance::new(&mut store, &module, &[]).expect("instantiate without WASI");
    let version = instance
        .get_typed_func::<(), i32>(&mut store, "traverse_bridge_abi_version")
        .expect("version export")
        .call(&mut store, ())
        .expect("version call");
    assert_eq!(version, 10100);
    Bridge {
        memory: instance
            .get_memory(&mut store, "memory")
            .expect("memory export"),
        alloc: instance
            .get_typed_func(&mut store, "traverse_alloc")
            .unwrap(),
        start: instance
            .get_typed_func(&mut store, "traverse_compatible_start")
            .unwrap(),
        stop: instance
            .get_typed_func(&mut store, "traverse_compatible_stop")
            .unwrap(),
        kill: instance
            .get_typed_func(&mut store, "traverse_compatible_kill")
            .unwrap(),
        next_event: instance
            .get_typed_func(&mut store, "traverse_next_event")
            .unwrap(),
        shutdown: instance
            .get_typed_func(&mut store, "traverse_shutdown")
            .unwrap(),
        store,
    }
}

#[test]
fn bridge_1_1_owns_compatible_lifecycle_and_ordered_events() {
    let mut bridge = bridge();
    let start_request = json!({"capability_id": "fixture.compatible", "input": {}});
    let instance_request = json!({
        "capability_id": "fixture.compatible",
        "instance_id": "fixture-compatible-1"
    });

    let (status, output) = bridge.call("start", &start_request);
    assert_eq!(status, 0);
    assert_eq!(output.unwrap()["status"], "started");
    assert_eq!(bridge.event().unwrap()["type"], "compatible_started");

    let (status, output) = bridge.call("stop", &instance_request);
    assert_eq!(status, 0);
    assert_eq!(output.unwrap()["status"], "stopped");
    assert_eq!(bridge.event().unwrap()["type"], "compatible_stopped");
    assert_eq!(bridge.call("stop", &instance_request).0, -1);
    assert!(bridge.event().is_none());

    assert_eq!(bridge.call("start", &start_request).0, 0);
    assert_eq!(bridge.event().unwrap()["type"], "compatible_started");
    assert_eq!(bridge.call("kill", &instance_request).0, 0);
    assert_eq!(bridge.event().unwrap()["type"], "compatible_killed");
    assert_eq!(bridge.call("kill", &instance_request).0, -1);
}

#[test]
fn shutdown_kills_the_remaining_compatible_instance_once() {
    let mut bridge = bridge();
    let start_request = json!({"capability_id": "fixture.compatible", "input": {}});
    let stop_request = json!({"capability_id": "fixture.compatible", "instance_id": null});
    assert_eq!(bridge.call("start", &start_request).0, 0);
    assert!(bridge.event().is_some());

    assert_eq!(bridge.shutdown.call(&mut bridge.store, 1024).unwrap(), 0);
    assert_eq!(
        bridge.read_json(1024)["killed_compatible_instances"],
        json!(["fixture-compatible-1"])
    );
    assert_eq!(bridge.call("stop", &stop_request).0, -1);

    assert_eq!(bridge.shutdown.call(&mut bridge.store, 1024).unwrap(), 0);
    assert_eq!(
        bridge.read_json(1024)["killed_compatible_instances"],
        json!([])
    );
}
