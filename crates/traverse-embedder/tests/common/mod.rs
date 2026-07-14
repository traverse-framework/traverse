//! Shared bundle fixture for embedder conformance and package tests.
#![allow(clippy::expect_used)]

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use traverse_embedder::{EventCallback, TraverseEmbedderApi};

static FIXTURE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Deterministic JSON the fixture WASM capability writes to stdout.
pub const PROCESS_OUTPUT: &str = r#"{"title":"Fixture note","tags":["fixture"],"noteType":"note","suggestedNextAction":"review","status":"processed"}"#;

/// Bundled WASM capability id (reuses the checked-in starter contract).
pub const PROCESS_CAPABILITY_ID: &str = "traverse-starter.process";

/// Bundled compatible-mode capability id.
pub const RENDER_CAPABILITY_ID: &str = "traverse-starter.render";

/// Bundled workflow id.
pub const PIPELINE_WORKFLOW_ID: &str = "fixture.pipeline";

const PROCESS_CONTRACT: &str = include_str!(
    "../../../../contracts/examples/traverse-starter/capabilities/process/contract.json"
);
const PROCESS_WORKFLOW: &str =
    include_str!("../../../../workflows/examples/traverse-starter/process/workflow.json");

/// Options controlling fixture bundle generation.
pub struct FixtureOptions {
    /// `schema_version` written to the app manifest.
    pub schema_version: String,
    /// Platform allowlist of the compatible-mode render component.
    pub compatible_platforms: Vec<String>,
    /// Make the render contract emit an event no registry declares, so the
    /// bundle loads but fails registration.
    pub emits_missing_event: bool,
    /// Duplicate the process component under a second component id with a
    /// conflicting contract, so capability registration conflicts.
    pub duplicate_capability_conflict: bool,
}

impl Default for FixtureOptions {
    fn default() -> Self {
        Self {
            schema_version: "1.0.0".to_string(),
            compatible_platforms: vec!["linux".to_string()],
            emits_missing_event: false,
            duplicate_capability_conflict: false,
        }
    }
}

/// One generated application bundle in a unique temp directory.
pub struct BundleFixture {
    root: PathBuf,
}

impl BundleFixture {
    pub fn new(name: &str) -> Self {
        Self::with_options(name, &FixtureOptions::default())
    }

    pub fn with_options(name: &str, options: &FixtureOptions) -> Self {
        let counter = FIXTURE_COUNTER.fetch_add(1, Ordering::SeqCst);
        let root = std::env::temp_dir().join(format!(
            "traverse-embedder-{name}-{}-{counter}",
            std::process::id()
        ));
        std::fs::create_dir_all(root.join("components/process"))
            .expect("fixture dirs should be created");
        std::fs::create_dir_all(root.join("components/render"))
            .expect("fixture dirs should be created");
        std::fs::create_dir_all(root.join("workflows")).expect("fixture dirs should be created");

        let wasm_bytes = wasm_bytes_for_output(PROCESS_OUTPUT);
        let wasm_digest = format!("sha256:{}", sha256_hex(&wasm_bytes));
        std::fs::write(root.join("components/process/component.wasm"), &wasm_bytes)
            .expect("fixture wasm should be written");
        std::fs::write(
            root.join("components/process/contract.json"),
            PROCESS_CONTRACT,
        )
        .expect("fixture contract should be written");
        write_json(
            &root.join("components/process/component.manifest.json"),
            &component_manifest_json(
                "fixture.process-component",
                PROCESS_CAPABILITY_ID,
                &json!({
                    "execution_mode": "wasm",
                    "wasm_binary_path": "component.wasm",
                    "wasm_digest": wasm_digest,
                }),
            ),
        );

        write_render_component(&root, options);

        let mut workflow: Value =
            serde_json::from_str(PROCESS_WORKFLOW).expect("starter workflow should parse");
        workflow["id"] = json!(PIPELINE_WORKFLOW_ID);
        workflow["name"] = json!("pipeline");
        write_json(&root.join("workflows/pipeline.workflow.json"), &workflow);

        let mut components = vec![
            json!({
                "component_id": "fixture.process-component",
                "version": "1.0.0",
                "digest": wasm_digest,
                "manifest_path": "components/process/component.manifest.json",
            }),
            json!({
                "component_id": "fixture.render-component",
                "version": "1.0.0",
                "digest": format!("sha256:{}", sha256_hex(b"fixture render component")),
                "manifest_path": "components/render/component.manifest.json",
            }),
        ];
        if options.duplicate_capability_conflict {
            write_conflict_component(&root, &wasm_bytes, &wasm_digest);
            components.push(json!({
                "component_id": "fixture.conflict-component",
                "version": "1.0.0",
                "digest": wasm_digest,
                "manifest_path": "components/conflict/component.manifest.json",
            }));
        }

        write_json(
            &root.join("app.manifest.json"),
            &json!({
                "app_id": "fixture-app",
                "version": "1.0.0",
                "schema_version": options.schema_version,
                "workspace_defaults": {
                    "workspace_id": "local-default",
                    "registry_scope": "private",
                },
                "components": components,
                "workflows": [{
                    "workflow_id": PIPELINE_WORKFLOW_ID,
                    "workflow_version": "1.0.0",
                    "path": "workflows/pipeline.workflow.json",
                }],
                "model_dependencies": [],
                "config_schema": { "type": "object" },
                "default_config": {},
                "placement_policy": { "preferred_targets": ["local"], "allow_fallback": false },
                "public_surfaces": ["cli"],
            }),
        );

        Self { root }
    }

    pub fn manifest_path(&self) -> PathBuf {
        self.root.join("app.manifest.json")
    }
}

fn write_render_component(root: &std::path::Path, options: &FixtureOptions) {
    let mut render_contract: Value =
        serde_json::from_str(PROCESS_CONTRACT).expect("starter contract should parse");
    render_contract["id"] = json!(RENDER_CAPABILITY_ID);
    render_contract["name"] = json!("render");
    render_contract["summary"] = json!("Fixture compatible-mode render capability.");
    if options.emits_missing_event {
        render_contract["emits"] =
            json!([{ "event_id": "fixture.missing-event", "version": "1.0.0" }]);
    }
    write_json(
        &root.join("components/render/contract.json"),
        &render_contract,
    );
    write_json(
        &root.join("components/render/wrapper.json"),
        &json!({ "kind": "compatible_wrapper", "capability_id": RENDER_CAPABILITY_ID }),
    );
    write_json(
        &root.join("components/render/component.manifest.json"),
        &component_manifest_json(
            "fixture.render-component",
            RENDER_CAPABILITY_ID,
            &json!({
                "execution_mode": "compatible",
                "platforms": options.compatible_platforms,
                "wrapper_path": "wrapper.json",
            }),
        ),
    );
}

fn write_conflict_component(root: &std::path::Path, wasm_bytes: &[u8], wasm_digest: &str) {
    std::fs::create_dir_all(root.join("components/conflict"))
        .expect("fixture dirs should be created");
    let mut conflict_contract: Value =
        serde_json::from_str(PROCESS_CONTRACT).expect("starter contract should parse");
    conflict_contract["summary"] = json!("Conflicting duplicate of the process contract.");
    write_json(
        &root.join("components/conflict/contract.json"),
        &conflict_contract,
    );
    std::fs::write(root.join("components/conflict/component.wasm"), wasm_bytes)
        .expect("fixture wasm should be written");
    write_json(
        &root.join("components/conflict/component.manifest.json"),
        &component_manifest_json(
            "fixture.conflict-component",
            PROCESS_CAPABILITY_ID,
            &json!({
                "execution_mode": "wasm",
                "wasm_binary_path": "component.wasm",
                "wasm_digest": wasm_digest,
            }),
        ),
    );
}

fn component_manifest_json(component_id: &str, capability_id: &str, mode_fields: &Value) -> Value {
    let mut manifest = json!({
        "component_id": component_id,
        "version": "1.0.0",
        "schema_version": "1.0.0",
        "capability_id": capability_id,
        "capability_version": "1.0.0",
        "contract_path": "contract.json",
        "runtime_constraints": {
            "host_api_access": "none",
            "network_access": "forbidden",
            "filesystem_access": "none",
        },
        "permitted_targets": ["local"],
        "dependencies": [],
        "connector_requirements": [],
        "validation_evidence": [],
    });
    for (key, value) in mode_fields
        .as_object()
        .expect("mode fields should be an object")
    {
        manifest[key] = value.clone();
    }
    manifest
}

fn write_json(path: &std::path::Path, value: &Value) {
    let rendered = serde_json::to_string_pretty(value).expect("fixture JSON should serialize");
    std::fs::write(path, rendered).expect("fixture file should be written");
}

/// Compiles a WASI command module that ignores stdin and writes
/// `output_json` to stdout.
pub fn wasm_bytes_for_output(output_json: &str) -> Vec<u8> {
    let escaped = output_json.replace('\\', "\\\\").replace('"', "\\\"");
    let length = output_json.len();
    let wat_source = format!(
        r#"(module
            (import "wasi_snapshot_preview1" "fd_write"
                (func $fd_write (param i32 i32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 64) "{escaped}")
            (func (export "_start")
                (i32.store (i32.const 0) (i32.const 64))
                (i32.store (i32.const 4) (i32.const {length}))
                (drop (call $fd_write (i32.const 1) (i32.const 0) (i32.const 1) (i32.const 8)))
            )
        )"#
    );
    wat::parse_str(&wat_source).expect("fixture WAT should compile")
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut rendered = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        write!(rendered, "{byte:02x}").expect("hex rendering should not fail");
    }
    rendered
}

/// Subscribes a collector to `embedder` and returns the shared event list.
pub fn collect_events(embedder: &mut dyn TraverseEmbedderApi) -> Arc<Mutex<Vec<Value>>> {
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&events);
    let callback: EventCallback = Box::new(move |event| {
        sink.lock()
            .expect("event sink lock should not be poisoned")
            .push(event.clone());
    });
    embedder.subscribe(callback);
    events
}

/// Snapshots collected events.
pub fn snapshot(events: &Arc<Mutex<Vec<Value>>>) -> Vec<Value> {
    events
        .lock()
        .expect("event sink lock should not be poisoned")
        .clone()
}
