#![cfg(feature = "wasmtime-executor")]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::expect_used,
    clippy::format_collect,
    clippy::format_push_string,
    clippy::unwrap_used
)]

use serde_json::{Value, json};
use wasmtime::{Engine, Instance, Memory, Module, Store, TypedFunc};

const INIT: &str = r#"{"status":"ready","error":null}"#;
const SUBMIT: &str = r#"{"session_id":"fixture-session","status":"accepted","error":null}"#;
const STATE: &str =
    r#"{"type":"state_changed","session_id":"fixture-session","data":{"state":"running"}}"#;
const INVOKED: &str = r#"{"type":"capability_invoked","session_id":"fixture-session","data":{"capability_id":"fixture.echo"}}"#;
const RESULT: &str = r#"{"type":"capability_result","session_id":"fixture-session","data":{"output":{"message":"hello"}}}"#;
const STOPPED: &str = r#"{"status":"stopped"}"#;

fn wat_string(value: &str) -> String {
    value
        .as_bytes()
        .iter()
        .map(|byte| format!("\\{byte:02x}"))
        .collect()
}

fn fixture_module() -> String {
    let values = [INIT, SUBMIT, STATE, INVOKED, RESULT, STOPPED];
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
        (init_p, init_l),
        (submit_p, submit_l),
        (state_p, state_l),
        (invoked_p, invoked_l),
        (result_p, result_l),
        (stopped_p, stopped_l),
    ] = regions.as_slice()
    else {
        unreachable!("fixture regions are fixed")
    };

    format!(
        r#"(module
          (memory (export "memory") 1 8)
          (global $heap (mut i32) (i32.const 4096))
          (global $event (mut i32) (i32.const 0))
          {data}
          (func (export "traverse_bridge_abi_version") (result i32) i32.const 10000)
          (func (export "traverse_alloc") (param $len i32) (result i32)
            (local $ptr i32)
            global.get $heap local.set $ptr
            global.get $heap local.get $len i32.add global.set $heap
            local.get $ptr)
          (func (export "traverse_dealloc") (param i32 i32))
          (func $descriptor (param $out i32) (param $ptr i32) (param $len i32)
            local.get $out local.get $ptr i32.store
            local.get $out i32.const 4 i32.add local.get $len i32.store)
          (func (export "traverse_init") (param i32 i32 i32) (result i32)
            local.get 2 i32.const {init_p} i32.const {init_l} call $descriptor
            i32.const 0)
          (func (export "traverse_submit") (param i32 i32 i32) (result i32)
            i32.const 0 global.set $event
            local.get 2 i32.const {submit_p} i32.const {submit_l} call $descriptor
            i32.const 0)
          (func (export "traverse_next_event") (param $out i32) (result i32)
            global.get $event i32.const 0 i32.eq
            if
              local.get $out i32.const {state_p} i32.const {state_l} call $descriptor
              i32.const 1 global.set $event
              i32.const 1 return
            end
            global.get $event i32.const 1 i32.eq
            if
              local.get $out i32.const {invoked_p} i32.const {invoked_l} call $descriptor
              i32.const 2 global.set $event
              i32.const 1 return
            end
            global.get $event i32.const 2 i32.eq
            if
              local.get $out i32.const {result_p} i32.const {result_l} call $descriptor
              i32.const 3 global.set $event
              i32.const 1 return
            end
            i32.const 0)
          (func (export "traverse_cancel") (param i32 i32 i32) (result i32)
            i32.const 0)
          (func (export "traverse_shutdown") (param i32) (result i32)
            local.get 0 i32.const {stopped_p} i32.const {stopped_l} call $descriptor
            i32.const 0))"#
    )
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
    fn call_json(&mut self, function: &str, input: &Value) -> Value {
        let bytes = serde_json::to_vec(input).expect("serialize fixture request");
        let pointer = self
            .alloc
            .call(&mut self.store, bytes.len() as i32)
            .expect("allocate request");
        self.memory
            .write(&mut self.store, pointer as usize, &bytes)
            .expect("write request");
        let status = match function {
            "init" => self
                .init
                .call(&mut self.store, (pointer, bytes.len() as i32, 1024)),
            "submit" => self
                .submit
                .call(&mut self.store, (pointer, bytes.len() as i32, 1024)),
            _ => unreachable!("known fixture call"),
        }
        .expect("bridge call");
        assert_eq!(status, 0);
        self.read_json(1024)
    }

    fn read_json(&self, descriptor: usize) -> Value {
        let data = self.memory.data(&self.store);
        let pointer = u32::from_le_bytes(data[descriptor..descriptor + 4].try_into().unwrap());
        let length = u32::from_le_bytes(data[descriptor + 4..descriptor + 8].try_into().unwrap());
        serde_json::from_slice(&data[pointer as usize..(pointer + length) as usize])
            .expect("valid fixture JSON")
    }
}

#[test]
fn core_wasm_bridge_produces_the_cross_platform_lifecycle_transcript() {
    let engine = Engine::default();
    let module = Module::new(&engine, fixture_module()).expect("compile bridge fixture");
    let mut store = Store::new(&engine, ());
    let instance = Instance::new(&mut store, &module, &[]).expect("instantiate without WASI");
    let version = instance
        .get_typed_func::<(), i32>(&mut store, "traverse_bridge_abi_version")
        .expect("version export")
        .call(&mut store, ())
        .expect("read ABI version");
    assert_eq!(version, 10000);

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

    assert_eq!(
        bridge.call_json("init", &json!({"workspace_id": "fixture"}))["status"],
        "ready"
    );
    assert_eq!(
        bridge.call_json(
            "submit",
            &json!({"target_id": "fixture.echo", "input": {"message": "hello"}})
        )["status"],
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
        ["state_changed", "capability_invoked", "capability_result"]
    );

    assert_eq!(bridge.shutdown.call(&mut bridge.store, 1024).unwrap(), 0);
    assert_eq!(bridge.read_json(1024)["status"], "stopped");
}
