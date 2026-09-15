#![cfg(feature = "wasmtime-executor")]
#![allow(clippy::expect_used, clippy::unwrap_used)]

//! Proves `runtime_wasm_host` drives a real, built `crates/traverse-runtime-wasm`
//! `wasm32-unknown-unknown` artifact (not a throwaway fixture) end-to-end: a
//! nested capability's `emit_event` call reaches a live `EventBroker`
//! subscriber (spec 1402 FR-003, Decision 89).

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use serde_json::json;
use traverse_contracts::{EventReference, ServiceType};
use traverse_runtime::events::broker::InProcessBroker;
use traverse_runtime::events::catalog::{EventCatalog, EventCatalogEntry};
use traverse_runtime::events::types::{EventBroker, LifecycleStatus};
use traverse_runtime::runtime_wasm_host::{RuntimeWasmHost, publish_domain_events};

/// A WASI-command capability that echoes stdin to stdout, then calls
/// `traverse_host::emit_event` with a fixed declared domain event.
const NESTED_CAPABILITY_WAT: &str = r#"
  (module
    (import "wasi_snapshot_preview1" "fd_read"
      (func $fd_read (param i32 i32 i32 i32) (result i32)))
    (import "wasi_snapshot_preview1" "fd_write"
      (func $fd_write (param i32 i32 i32 i32) (result i32)))
    (import "traverse_host" "emit_event"
      (func $emit_event (param i32 i32) (result i32)))
    (memory (export "memory") 1)
    (data (i32.const 5000) "{\"event_id\":\"host.driver.smoke\",\"version\":\"1.0.0\",\"payload\":{\"ok\":true}}")
    (func (export "_start")
      (i32.store (i32.const 0) (i32.const 8))
      (i32.store (i32.const 4) (i32.const 1024))
      (drop (call $fd_read (i32.const 0) (i32.const 0) (i32.const 1) (i32.const 4100)))
      (i32.store (i32.const 0) (i32.const 8))
      (i32.store (i32.const 4) (i32.load (i32.const 4100)))
      (drop (call $fd_write (i32.const 1) (i32.const 0) (i32.const 1) (i32.const 4104)))
      (drop (call $emit_event (i32.const 5000) (i32.const 72)))
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
/// returns the path to the resulting `.wasm` cdylib. Uses a dedicated target
/// directory rather than whatever `CARGO_TARGET_DIR` the outer `cargo test`
/// invocation has — sharing that directory races with the outer build's own
/// lock (and, in a shared multi-session sandbox, with unrelated concurrent
/// builds against the same cache), which was observed to fail intermittently.
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

#[test]
fn host_driver_runs_real_artifact_and_publishes_a_real_domain_event() {
    let runtime_wasm_bytes =
        std::fs::read(build_runtime_wasm_artifact()).expect("read built runtime.wasm");
    let nested_capability = wat::parse_str(NESTED_CAPABILITY_WAT).expect("wat parses");

    let mut host =
        RuntimeWasmHost::instantiate(&runtime_wasm_bytes).expect("instantiate runtime.wasm");

    let init_response = host
        .init(
            "example.host-driver-smoke",
            "1.0.0",
            &ServiceType::Subscribable,
            &[EventReference {
                event_id: "host.driver.smoke".to_string(),
                version: "1.0.0".to_string(),
            }],
            &nested_capability,
        )
        .expect("traverse_init succeeds");
    assert_eq!(init_response["status"], "ready");

    host.submit(br#"{"hello":"host-driver"}"#)
        .expect("traverse_submit succeeds");

    let events = host.drain_events().expect("drain events");
    assert!(
        events
            .iter()
            .any(|event| event["type"] == "capability_result"
                && event["data"]["status"] == "completed"),
        "expected a completed capability_result among {events:?}"
    );

    let catalog = Arc::new(EventCatalog::new());
    catalog
        .register(EventCatalogEntry {
            event_type: "host.driver.smoke".to_string(),
            owner: "example.host-driver-smoke".to_string(),
            version: "1.0.0".to_string(),
            lifecycle_status: LifecycleStatus::Active,
            consumer_count: 0,
        })
        .expect("register event type");
    let broker: Arc<dyn EventBroker> =
        Arc::new(InProcessBroker::new(catalog).expect("construct broker"));
    let subscription = broker
        .subscribe("host.driver.smoke", "0")
        .expect("subscribe");

    let published = publish_domain_events(&events, "example.host-driver-smoke", &broker)
        .expect("publish domain events");
    assert_eq!(published, 1, "exactly one domain event should publish");

    let poll = broker
        .poll(&subscription.subscription_id, 10)
        .expect("poll subscription");
    assert_eq!(poll.events.len(), 1);
    let delivered = &poll.events[0].event;
    assert_eq!(delivered.event_type, "host.driver.smoke");
    assert_eq!(delivered.data, json!({"ok": true}));
    assert_eq!(delivered.owner, "example.host-driver-smoke");

    let shutdown_response = host.shutdown().expect("traverse_shutdown succeeds");
    assert_eq!(shutdown_response["status"], "stopped");
}
