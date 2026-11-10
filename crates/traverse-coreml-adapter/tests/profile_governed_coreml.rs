//! #1833: macOS profiling harness for governed Core ML overhead (Spec 138).
//!
//! Times one governed `coreml` call around the public ExactModelHostConnector
//! path and attributes wall time to stable stage boundaries. Does **not**
//! change Spec 138 checks or remove copies/rechecks.
//!
//! Run on macOS:
//! ```text
//! cargo test -p traverse-coreml-adapter profile_governed_coreml_overhead \
//!   -- --nocapture
//! ```
//!
//! Optional: set `TRAVERSE_WRITE_PROFILE_EVIDENCE=1` to write JSON under
//! `docs/evidence/`.
#![cfg(target_vendor = "apple")]
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::doc_markdown,
    clippy::too_many_lines
)]

use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use serde_json::{Value, json};
use traverse_coreml_adapter::{ComputeUnits, CoreMlAdapter};
use traverse_runtime::exact_model::{
    AcceleratorAdapter, CachedAcceleratorVariant, ExactModelHostConnector, ExactModelPin,
    ExecutionPolicy, ModelUsage, PlacementPolicy, TrustedModelKeys, digest_hex,
};
use traverse_runtime::host_connector_dispatch::{
    HostConnectorHostRequest, HostConnectorPort, MODEL_EXECUTE_OPERATION, MODEL_RUNTIME_CONNECTOR,
};

const PACKAGE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../fixtures/models/digits-onnx-coreml-1.0.0"
);
const SIGNING_KEY: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../fixtures/models/test-signing-key.json"
);
const EVIDENCE_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/evidence");

fn read(relative: &str) -> Vec<u8> {
    fs::read(format!("{PACKAGE}/{relative}")).expect(relative)
}

fn hex(value: &str) -> Vec<u8> {
    (0..value.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&value[at..at + 2], 16).expect("hex"))
        .collect()
}

fn cache_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "traverse-coreml-profile-{name}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    dir
}

fn ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1000.0
}

/// Build a registered host and return `(connector, digest, first input frame, register_ms)`.
fn prepared_host() -> (ExactModelHostConnector, String, Vec<u8>, f64) {
    let manifest_bytes = read("model.manifest.json");
    let manifest: Value = serde_json::from_slice(&manifest_bytes).expect("manifest");
    let variant = &manifest["accelerator_variants"][0];
    let key: Value =
        serde_json::from_slice(&fs::read(SIGNING_KEY).expect("key")).expect("key json");
    let public_key: [u8; 32] = hex(key["public_key_hex"].as_str().expect("hex"))
        .try_into()
        .expect("32 bytes");
    let mut keys = TrustedModelKeys::new();
    keys.trust(&public_key).expect("trust");
    let digest = digest_hex(&manifest_bytes);
    let pin: ExactModelPin = serde_json::from_value(json!({
        "model_id": manifest["model_id"],
        "version": manifest["version"],
        "digest": digest,
        "offline_allowed": true,
        "target": "wasm-cpu",
        "rights": {
            "license_id": manifest["rights"]["license_id"],
            "commercial_use": manifest["rights"]["commercial_use"],
        },
    }))
    .expect("pin");
    let adapter = CoreMlAdapter::new(ComputeUnits::All, cache_dir("profile"));
    let mut connector = ExactModelHostConnector::new(vec![pin], keys);
    connector.model_usage = Some(ModelUsage::NonCommercial);
    connector.register_adapter(Box::new(adapter));

    let zip_path = variant["path"].as_str().expect("path");
    let vector_path = variant["conformance_vector_path"].as_str().expect("path");
    let (zip, vector) = (read(zip_path), read(vector_path));
    let placements = ["coreml"];
    let register_started = Instant::now();
    connector
        .register_package_with_variants(
            &manifest_bytes,
            read("model.wasm"),
            &read("model.sig.json"),
            &[(zip_path, &zip), (vector_path, &vector)],
            &PlacementPolicy {
                policy_ref: "coreml".to_string(),
                placements: placements.iter().map(ToString::to_string).collect(),
                require_fuel_metering: false,
            },
            None,
        )
        .expect("register");
    let register_ms = ms(register_started);

    let vector_json: Value = serde_json::from_slice(&vector).expect("vector");
    let input = hex(vector_json["cases"][0]["input_frame_hex"]
        .as_str()
        .expect("input"));
    (connector, digest, input, register_ms)
}

#[test]
fn profile_governed_coreml_overhead() {
    let (mut connector, digest, input, register_ms) = prepared_host();

    connector.policies.insert(
        "coreml".to_string(),
        ExecutionPolicy {
            policy_ref: "coreml".to_string(),
            allowed_classifications: vec!["sensitive".to_string()],
            max_output_bytes: 4096,
            ..ExecutionPolicy::default()
        },
    );

    // Warm Core ML compile/cache so steady-state execute is attributable.
    let warm_ref = connector
        .io
        .stage_model_input(&input, 4096)
        .expect("warm stage");
    let _ = connector
        .invoke(&HostConnectorHostRequest {
            connector_id: MODEL_RUNTIME_CONNECTOR.to_string(),
            operation: MODEL_EXECUTE_OPERATION.to_string(),
            binding_id: "profile".to_string(),
            target_family: "apple".to_string(),
            correlation_id: "warm".to_string(),
            payload: json!({
                "model_ref": {
                    "model_id": "traverse.digits-onnx-coreml",
                    "version": "1.0.0",
                    "digest": digest,
                },
                "input_ref": warm_ref,
                "policy_ref": "coreml",
                "data_classification": "sensitive",
                "input_schema_ref": "schema:traverse-digits-onnx-in",
                "input_schema_version": "1.0.0",
                "max_output_bytes": 4096,
            }),
            cancel_requested: false,
        })
        .expect("warm execute");

    // --- measured governed call ---
    let stage_started = Instant::now();
    let input_ref = connector
        .io
        .stage_model_input(&input, 4096)
        .expect("stage_input");
    let stage_input_ms = ms(stage_started);

    let execute_started = Instant::now();
    let result = connector
        .invoke(&HostConnectorHostRequest {
            connector_id: MODEL_RUNTIME_CONNECTOR.to_string(),
            operation: MODEL_EXECUTE_OPERATION.to_string(),
            binding_id: "profile".to_string(),
            target_family: "apple".to_string(),
            correlation_id: "profile-run".to_string(),
            payload: json!({
                "model_ref": {
                    "model_id": "traverse.digits-onnx-coreml",
                    "version": "1.0.0",
                    "digest": digest,
                },
                "input_ref": input_ref,
                "policy_ref": "coreml",
                "data_classification": "sensitive",
                "input_schema_ref": "schema:traverse-digits-onnx-in",
                "input_schema_version": "1.0.0",
                "max_output_bytes": 4096,
            }),
            cancel_requested: false,
        })
        .expect("execute");
    let execute_total_ms = ms(execute_started);

    let read_started = Instant::now();
    let output = connector
        .io
        .read_model_output(result.artifact_ref.as_deref().unwrap_or(""), 4096)
        .expect("read_output");
    let read_output_ms = ms(read_started);
    assert!(
        !output.is_empty(),
        "governed call must return an output frame"
    );

    // Raw adapter prediction of the same package variant (outside Spec 138
    // execute) — isolates Core ML MLMultiArray fill/predict/read from
    // digest/rights rechecks and connector glue.
    let mut raw_adapter = CoreMlAdapter::new(ComputeUnits::All, cache_dir("raw"));
    let manifest: Value = serde_json::from_slice(&read("model.manifest.json")).expect("manifest");
    let variant = &manifest["accelerator_variants"][0];
    let zip_path = variant["path"].as_str().expect("path");
    let vector_path = variant["conformance_vector_path"].as_str().expect("path");
    let cached = CachedAcceleratorVariant {
        adapter: "coreml".to_string(),
        path: zip_path.to_string(),
        bytes: read(zip_path),
        conformance_vector_path: vector_path.to_string(),
        conformance_vector: read(vector_path),
    };
    // Warm raw path (compile) then measure one prediction.
    let _ = raw_adapter.predict(&cached, &input).expect("raw warm");
    let raw_started = Instant::now();
    let raw_out = raw_adapter.predict(&cached, &input).expect("raw predict");
    let raw_adapter_predict_ms = ms(raw_started);
    assert_eq!(
        raw_out.len(),
        output.len(),
        "raw and governed frames should match size"
    );

    let end_to_end_ms = stage_input_ms + execute_total_ms + read_output_ms;
    let execute_minus_adapter_ms = (execute_total_ms - raw_adapter_predict_ms).max(0.0);
    let residual_ms = (end_to_end_ms
        - stage_input_ms
        - raw_adapter_predict_ms
        - read_output_ms
        - execute_minus_adapter_ms)
        .abs();

    let evidence = json!({
        "issue": "#1833",
        "parent": "#1707",
        "governing_spec": "138-governed-exact-model-execution",
        "package": {
            "model_id": "traverse.digits-onnx-coreml",
            "version": "1.0.0",
            "digest": digest,
            "fixture": "fixtures/models/digits-onnx-coreml-1.0.0",
        },
        "platform": {
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "placement": "coreml",
            "compute_units": "all",
        },
        "stages_ms": {
            "register": register_ms,
            "stage_input": stage_input_ms,
            "execute_total": execute_total_ms,
            "raw_adapter_predict": raw_adapter_predict_ms,
            "digest_rights_and_connector_glue_estimate": execute_minus_adapter_ms,
            "read_output": read_output_ms,
            "end_to_end_stage_sum": end_to_end_ms,
            "reconciliation_noise_ms": residual_ms,
        },
        "attribution_notes": [
            "register includes Spec 138 registration (signature, rights, variant self-check/compile) and is excluded from the steady execute sum.",
            "stage_input / read_output are public ModelIoStore frame copies around model.execute.",
            "raw_adapter_predict is CoreMlAdapter.predict alone (MLMultiArray fill + Core ML prediction + output read) after a warm compile.",
            "digest_rights_and_connector_glue_estimate = execute_total - raw_adapter_predict: digest/rights rechecks, policy/schema checks, cancel/deadline guards, and connector framing inside invoke. Spec 138 checks are unchanged.",
            "Noise: OS scheduling and Core ML runtime variance; stage totals should reconcile to end-to-end within a few ms.",
        ],
        "spec_138_checks_weakened": false,
    });

    println!("{}", serde_json::to_string_pretty(&evidence).expect("json"));

    // Sanity: stages are finite and non-negative; execute covers adapter work.
    assert!(register_ms >= 0.0);
    assert!(stage_input_ms >= 0.0);
    assert!(execute_total_ms >= 0.0);
    assert!(raw_adapter_predict_ms >= 0.0);
    assert!(read_output_ms >= 0.0);
    assert!(
        execute_total_ms + 5.0 >= raw_adapter_predict_ms,
        "governed execute ({execute_total_ms} ms) should cover raw adapter ({raw_adapter_predict_ms} ms) within tolerance"
    );

    if std::env::var_os("TRAVERSE_WRITE_PROFILE_EVIDENCE").is_some() {
        let path = PathBuf::from(EVIDENCE_DIR).join("governed-coreml-overhead-macos-profile.json");
        fs::write(
            &path,
            format!(
                "{}\n",
                serde_json::to_string_pretty(&evidence).expect("json")
            ),
        )
        .expect("write evidence");
        println!("wrote {}", path.display());
    }
}
