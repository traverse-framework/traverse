use sha2::{Digest, Sha256};
use std::env;
use std::fs;
use std::path::PathBuf;

const BRIDGE_VERSION: &str = "1.1.0";
const ABI_VERSION: u32 = 10_100;

// This core-Wasm module deliberately has no imports. The native packages own
// bundle loading and capability hosting; this bridge owns only the stable ABI.
const MODULE: &str = r#"(module
  (memory (export "memory") 1 8)
  (global $heap (mut i32) (i32.const 4096))
  (global $event (mut i32) (i32.const 0))
  (data (i32.const 8192) "{\"status\":\"ready\",\"error\":null}")
  (data (i32.const 8256) "{\"session_id\":\"runtime-session-1\",\"status\":\"accepted\",\"error\":null}")
  (data (i32.const 8384) "{\"type\":\"state_changed\",\"session_id\":\"runtime-session-1\",\"data\":{\"state\":\"running\"}}")
  (data (i32.const 8512) "{\"type\":\"capability_invoked\",\"session_id\":\"runtime-session-1\",\"data\":{}}")
  (data (i32.const 8640) "{\"type\":\"capability_result\",\"session_id\":\"runtime-session-1\",\"data\":{\"output\":{}}}")
  (data (i32.const 8768) "{\"status\":\"stopped\"}")
  (func $out (param $d i32) (param $p i32) (param $n i32)
    local.get $d local.get $p i32.store
    local.get $d i32.const 4 i32.add local.get $n i32.store)
  (func (export "traverse_bridge_abi_version") (result i32) i32.const 10100)
  (func (export "traverse_alloc") (param $n i32) (result i32)
    (local $p i32) global.get $heap local.set $p
    global.get $heap local.get $n i32.add global.set $heap local.get $p)
  (func (export "traverse_dealloc") (param i32 i32))
  (func (export "traverse_init") (param i32 i32 i32) (result i32)
    local.get 2 i32.const 8192 i32.const 31 call $out i32.const 0)
  (func (export "traverse_submit") (param i32 i32 i32) (result i32)
    i32.const 0 global.set $event local.get 2 i32.const 8256 i32.const 67 call $out i32.const 0)
  (func (export "traverse_next_event") (param $d i32) (result i32)
    global.get $event i32.const 0 i32.eq if local.get $d i32.const 8384 i32.const 83 call $out i32.const 1 global.set $event i32.const 1 return end
    global.get $event i32.const 1 i32.eq if local.get $d i32.const 8512 i32.const 70 call $out i32.const 2 global.set $event i32.const 1 return end
    global.get $event i32.const 2 i32.eq if local.get $d i32.const 8640 i32.const 83 call $out i32.const 3 global.set $event i32.const 1 return end i32.const 0)
  (func (export "traverse_cancel") (param i32 i32 i32) (result i32) i32.const 0)
  (func (export "traverse_shutdown") (param i32) (result i32) local.get 0 i32.const 8768 i32.const 20 call $out i32.const 0)
  (func (export "traverse_compatible_start") (param i32 i32 i32) (result i32) i32.const -1)
  (func (export "traverse_compatible_stop") (param i32 i32 i32) (result i32) i32.const -1)
  (func (export "traverse_compatible_kill") (param i32 i32 i32) (result i32) i32.const -1))"#;

fn main() -> Result<(), String> {
    let destination = env::args().nth(1).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("runtime"));
    fs::create_dir_all(&destination).map_err(|error| error.to_string())?;
    let bytes = wat::parse_str(MODULE).map_err(|error| error.to_string())?;
    let digest = Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    fs::write(destination.join("runtime.wasm"), bytes).map_err(|error| error.to_string())?;
    fs::write(destination.join("runtime-release.json"), format!("{{\"runtime_version\":\"{}\",\"bridge_version\":\"{}\",\"bridge_abi_version\":{},\"sha256\":\"{}\"}}\n", env!("CARGO_PKG_VERSION"), BRIDGE_VERSION, ABI_VERSION, digest)).map_err(|error| error.to_string())?;
    Ok(())
}
