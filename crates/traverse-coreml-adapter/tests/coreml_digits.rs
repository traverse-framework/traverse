//! Real Core ML on macOS (Spec 138 FR-062 to FR-065, #1629 slice 3): the
//! signed `digits-onnx-coreml-1.0.0` fixture runs through the connector on
//! the `coreml` placement, within its variant tolerance of the `wasm-cpu`
//! reference, with `coreml` placement evidence.
#![cfg(target_vendor = "apple")]
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::doc_markdown
)]

use std::fs;
use std::path::PathBuf;

use serde_json::{Value, json};
use traverse_coreml_adapter::{ComputeUnits, CoreMlAdapter};
use traverse_runtime::exact_model::{
    AcceleratorAdapter, ExactModelHostConnector, ExactModelPin, ExecutionPolicy, FuelMetering,
    ModelUsage, TrustedModelKeys, decode_guest_frame, digest_hex,
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

fn read(relative: &str) -> Vec<u8> {
    fs::read(format!("{PACKAGE}/{relative}")).expect(relative)
}

fn hex(value: &str) -> Vec<u8> {
    (0..value.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&value[at..at + 2], 16).expect("hex"))
        .collect()
}

fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

fn cache_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "traverse-coreml-test-{name}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    dir
}

/// `(input frame, wasm-cpu reference output frame)` conformance cases.
type Cases = Vec<(Vec<u8>, Vec<u8>)>;

/// A connector holding the signed fixture with its Core ML variant, and the
/// variant's conformance cases.
fn host(adapter: CoreMlAdapter) -> (ExactModelHostConnector, String, Cases) {
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
    let mut connector = ExactModelHostConnector::new(vec![pin], keys);
    connector.model_usage = Some(ModelUsage::NonCommercial);
    connector.register_adapter(Box::new(adapter));
    let zip_path = variant["path"].as_str().expect("path");
    let vector_path = variant["conformance_vector_path"].as_str().expect("path");
    let (zip, vector) = (read(zip_path), read(vector_path));
    connector
        .register_package_with_variants(
            &manifest_bytes,
            read("model.wasm"),
            &read("model.sig.json"),
            &[(zip_path, &zip), (vector_path, &vector)],
        )
        .expect("register");
    let cases = serde_json::from_slice::<Value>(&vector).expect("vector")["cases"]
        .as_array()
        .expect("cases")
        .iter()
        .map(|case| {
            (
                hex(case["input_frame_hex"].as_str().expect("input")),
                hex(case["output_frame_hex"].as_str().expect("output")),
            )
        })
        .collect();
    (connector, digest, cases)
}

fn execute(
    connector: &mut ExactModelHostConnector,
    digest: &str,
    input: &[u8],
    placements: &[&str],
) -> Result<(Vec<u8>, Value), String> {
    connector.policies.insert(
        "policy-1".to_string(),
        ExecutionPolicy {
            policy_ref: "policy-1".to_string(),
            allowed_classifications: vec!["sensitive".to_string()],
            max_output_bytes: 4096,
            placements: placements.iter().map(ToString::to_string).collect(),
            require_fuel_metering: false,
        },
    );
    let input_ref = connector
        .io
        .stage_model_input(input, 4096)
        .map_err(|error| error.message)?;
    let result = connector
        .invoke(&HostConnectorHostRequest {
            connector_id: MODEL_RUNTIME_CONNECTOR.to_string(),
            operation: MODEL_EXECUTE_OPERATION.to_string(),
            binding_id: "swift-exact-model-host".to_string(),
            target_family: "apple".to_string(),
            correlation_id: "coreml-test".to_string(),
            payload: json!({
                "model_ref": {
                    "model_id": "traverse.digits-onnx-coreml",
                    "version": "1.0.0",
                    "digest": digest,
                },
                "input_ref": input_ref,
                "policy_ref": "policy-1",
                "data_classification": "sensitive",
                "input_schema_ref": "schema:traverse-digits-onnx-in",
                "input_schema_version": "1.0.0",
                "max_output_bytes": 4096,
            }),
            cancel_requested: false,
        })
        .map_err(|error| {
            format!(
                "{} {:?}: {}",
                error.code.as_str(),
                error.reason,
                error.message
            )
        })?;
    let output = connector
        .io
        .read_model_output(&result.artifact_ref.unwrap_or_default(), 4096)
        .map_err(|error| error.message)?;
    let evidence = serde_json::to_value(result.placement_evidence).expect("evidence");
    Ok((output, evidence))
}

fn logits(frame: &[u8]) -> (u8, Vec<u32>, Vec<f32>) {
    let (dtype, dims, payload) = decode_guest_frame(frame).expect("frame");
    let values = payload
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes(chunk.try_into().expect("4 bytes")))
        .collect();
    (dtype, dims, values)
}

#[test]
fn signed_digits_variant_runs_on_core_ml_within_tolerance() {
    let adapter = CoreMlAdapter::new(ComputeUnits::All, cache_dir("all"));
    assert!(adapter.available());
    assert!(!adapter.platform_version().is_empty());
    assert_eq!(adapter.fuel_metering(), FuelMetering::NotApplicable);
    let (mut connector, digest, cases) = host(adapter);
    assert_eq!(cases.len(), 10);
    let mut worst = 0.0_f32;
    for (input, expected) in &cases {
        let (output, evidence) =
            execute(&mut connector, &digest, input, &["coreml"]).expect("coreml execute");
        assert_eq!(
            evidence,
            json!({
                "placement": "coreml",
                "compute_units": "all",
                "fuel_metering": "not_applicable",
            })
        );
        let (dtype, dims, actual) = logits(&output);
        let (expected_dtype, expected_dims, reference) = logits(expected);
        assert_eq!((dtype, dims), (expected_dtype, expected_dims));
        for (a, b) in actual.iter().zip(&reference) {
            worst = worst.max((a - b).abs());
        }
        let argmax = |values: &[f32]| {
            values
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.total_cmp(b.1))
                .map(|(index, _)| index)
        };
        assert_eq!(argmax(&actual), argmax(&reference));
    }
    assert!(worst <= 5e-3, "worst |Δlogit| {worst}");
}

#[test]
fn every_compute_unit_setting_passes_and_is_recorded() {
    for (units, name) in [
        (ComputeUnits::CpuOnly, "cpu_only"),
        (ComputeUnits::CpuAndGpu, "cpu_and_gpu"),
        (ComputeUnits::CpuAndNeuralEngine, "cpu_and_neural_engine"),
    ] {
        let (mut connector, digest, cases) = host(CoreMlAdapter::new(units, cache_dir(name)));
        let (input, _) = &cases[0];
        let (_, evidence) = execute(&mut connector, &digest, input, &["coreml", "wasm-cpu"])
            .expect("coreml execute");
        assert_eq!(evidence["placement"], json!("coreml"));
        assert_eq!(evidence["compute_units"], json!(name));
    }
}

#[test]
fn malformed_inputs_fail_closed_without_touching_the_model() {
    let (mut connector, digest, cases) = host(CoreMlAdapter::new(
        ComputeUnits::CpuOnly,
        cache_dir("malformed"),
    ));
    let (input, _) = &cases[0];
    // Wrong dtype code: the frame contract says the input is dtype 2.
    let mut wrong_dtype = input.clone();
    wrong_dtype[2] = 9;
    let error = execute(&mut connector, &digest, &wrong_dtype, &["coreml"]).expect_err("dtype");
    assert!(error.contains("dtype"), "{error}");
    // Payload shorter than its dims.
    let truncated = &input[..input.len() - 4];
    assert!(execute(&mut connector, &digest, truncated, &["coreml"]).is_err());
    // Spec 138 0.14.0 (Decision 115): the shared reject corpus fails on the
    // `coreml` placement with the same code and reason as on `wasm-cpu`.
    let vector: Value = serde_json::from_slice(
        &fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/models/conformance/signed-digits-onnx.json"
        ))
        .expect("vector"),
    )
    .expect("vector json");
    for case in vector["rejected_cases"].as_array().expect("rejected") {
        let frame = hex(case["input_frame_hex"].as_str().expect("frame"));
        let error = execute(&mut connector, &digest, &frame, &["coreml"]).expect_err("rejected");
        assert!(
            error.starts_with("invalid_input Some(FrameMismatch)"),
            "{}: {error}",
            case["name"]
        );
    }
}

/// BirdNET v2.4 on the Decision 113 clip set, Decision 106's bar: top-5
/// identical and |Δ sigmoid| ≤ 5×10⁻³ against the `wasm-cpu` (tract int8)
/// reference. BirdNET weights and recordings are not redistributable, so
/// this runs only with `--ignored` and `TRAVERSE_BIRDNET_COREML_DIR` naming a
/// directory with `birdnet.mlpackage.zip` (from `model convert-coreml`),
/// `clips/<name>.f32le` (144,000 samples) and `ref/<name>.tract.f32le`.
#[test]
#[ignore = "needs local BirdNET weights and clips (TRAVERSE_BIRDNET_COREML_DIR)"]
fn birdnet_clip_set_within_decision_106_tolerance() {
    use traverse_runtime::exact_model::{CachedAcceleratorVariant, encode_guest_frame};
    let dir = PathBuf::from(
        std::env::var("TRAVERSE_BIRDNET_COREML_DIR").expect("TRAVERSE_BIRDNET_COREML_DIR"),
    );
    let mut clips: Vec<String> = fs::read_dir(dir.join("clips"))
        .expect("clips")
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter_map(|name| name.strip_suffix(".f32le").map(str::to_string))
        .collect();
    clips.sort();
    assert!(!clips.is_empty());
    let frames: Vec<(Vec<u8>, Vec<u8>)> = clips
        .iter()
        .map(|clip| {
            let audio = fs::read(dir.join(format!("clips/{clip}.f32le"))).expect("clip");
            let reference = fs::read(dir.join(format!("ref/{clip}.tract.f32le"))).expect("ref");
            let classes = u32::try_from(reference.len() / 4).expect("classes");
            (
                encode_guest_frame(2, &[1, 144_000], &audio),
                encode_guest_frame(3, &[1, classes], &reference),
            )
        })
        .collect();
    let cases: Vec<Value> = frames
        .iter()
        .map(|(input, output)| {
            json!({ "input_frame_hex": hex_encode(input), "output_frame_hex": hex_encode(output) })
        })
        .collect();
    let variant = CachedAcceleratorVariant {
        adapter: "coreml".to_string(),
        path: "variants/coreml/model.mlpackage.zip".to_string(),
        bytes: fs::read(dir.join("birdnet.mlpackage.zip")).expect("zip"),
        conformance_vector_path: "variants/coreml/conformance.json".to_string(),
        conformance_vector: serde_json::to_vec(&json!({ "cases": cases })).expect("vector"),
    };
    let sigmoid = |value: f32| 1.0 / (1.0 + (-f64::from(value)).exp());
    let top5 = |values: &[f32]| {
        let mut order: Vec<usize> = (0..values.len()).collect();
        order.sort_by(|a, b| values[*b].total_cmp(&values[*a]));
        order.truncate(5);
        order
    };
    for units in [
        ComputeUnits::All,
        ComputeUnits::CpuAndGpu,
        ComputeUnits::CpuAndNeuralEngine,
        ComputeUnits::CpuOnly,
    ] {
        let mut adapter =
            CoreMlAdapter::new(units, cache_dir(&format!("birdnet-{}", units.as_str())));
        let (mut worst, mut times) = (0.0_f64, Vec::new());
        for (input, expected) in &frames {
            let started = std::time::Instant::now();
            let output = adapter.predict(&variant, input).expect("predict");
            times.push(started.elapsed().as_secs_f64() * 1000.0);
            let (dtype, dims, actual) = logits(&output);
            let (expected_dtype, expected_dims, reference) = logits(expected);
            assert_eq!((dtype, dims), (expected_dtype, expected_dims));
            for (a, b) in actual.iter().zip(&reference) {
                worst = worst.max((sigmoid(*a) - sigmoid(*b)).abs());
            }
            assert_eq!(top5(&actual), top5(&reference), "{}", units.as_str());
        }
        // The first call includes on-device compilation.
        let steady = &mut times[1..];
        steady.sort_by(f64::total_cmp);
        let p95 = steady[(steady.len() * 95).div_ceil(100) - 1];
        println!(
            "birdnet {}: worst |Δsig| {worst:.2e}, first {:.0} ms (compile + run), p95 {p95:.1} ms",
            units.as_str(),
            times[0]
        );
        assert!(worst <= 5e-3, "{}: worst |Δsig| {worst}", units.as_str());
    }
}

#[test]
fn host_configuration_enables_and_configures_the_adapter() {
    use traverse_coreml_adapter::coreml_adapters;
    assert!(coreml_adapters(&json!(null)).expect("none").is_empty());
    assert!(coreml_adapters(&json!({})).expect("none").is_empty());
    let defaults = coreml_adapters(&json!({ "coreml": {} })).expect("default");
    assert_eq!(defaults.len(), 1);
    assert_eq!(defaults[0].id(), "coreml");
    assert_eq!(defaults[0].compute_units(), "all");
    let narrowed = coreml_adapters(&json!({
        "coreml": { "compute_units": "cpu_and_neural_engine", "cache_dir": "/tmp/traverse-coreml-config" }
    }))
    .expect("configured");
    assert_eq!(narrowed[0].compute_units(), "cpu_and_neural_engine");
    for (bad, field) in [
        (json!({ "coreml": true }), "accelerators.coreml"),
        (
            json!({ "coreml": { "compute_units": "gpu" } }),
            "accelerators.coreml.compute_units",
        ),
        (
            json!({ "coreml": { "cache_dir": "" } }),
            "accelerators.coreml.cache_dir",
        ),
    ] {
        assert_eq!(coreml_adapters(&bad).err(), Some(field));
    }
}

/// A canonical-format zip (stored entries) of the given files.
fn stored_zip(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for (name, data) in files {
        bytes.extend_from_slice(&0x0403_4b50_u32.to_le_bytes());
        bytes.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        let size = u32::try_from(data.len()).expect("size").to_le_bytes();
        bytes.extend_from_slice(&size);
        bytes.extend_from_slice(&size);
        bytes.extend_from_slice(&u16::try_from(name.len()).expect("name").to_le_bytes());
        bytes.extend_from_slice(&[0, 0]);
        bytes.extend_from_slice(name.as_bytes());
        bytes.extend_from_slice(data);
    }
    bytes.extend_from_slice(&0x0201_4b50_u32.to_le_bytes());
    bytes
}

#[test]
fn unusable_variants_fail_the_prediction_closed() {
    use traverse_runtime::exact_model::{CachedAcceleratorVariant, encode_guest_frame};
    let input = encode_guest_frame(2, &[1, 64], &[0; 256]);
    let output = encode_guest_frame(3, &[1, 10], &[0; 40]);
    let vector = serde_json::to_vec(&json!({
        "cases": [{ "input_frame_hex": hex_encode(&input), "output_frame_hex": hex_encode(&output) }]
    }))
    .expect("vector");
    let variant = |bytes: Vec<u8>, conformance_vector: Vec<u8>| CachedAcceleratorVariant {
        adapter: "coreml".to_string(),
        path: "variants/coreml/model.mlpackage.zip".to_string(),
        bytes,
        conformance_vector_path: "variants/coreml/conformance.json".to_string(),
        conformance_vector,
    };
    let mut adapter = CoreMlAdapter::new(ComputeUnits::CpuOnly, cache_dir("unusable"));
    let corrupt = stored_zip(&[
        ("Data/com.apple.CoreML/model.mlmodel", b"not a protobuf"),
        ("Manifest.json", b"{}"),
    ]);
    let compile = adapter
        .predict(&variant(corrupt, vector.clone()), &input)
        .expect_err("corrupt package");
    // Core ML raises a C++ exception on this manifest, which would abort
    // the process; the adapter rejects it first.
    assert!(
        compile
            .message
            .starts_with("mlpackage-zip variant: Manifest.json"),
        "{}",
        compile.message
    );
    // A package with a valid shape but a corrupt model fails closed too.
    let manifest = br#"{"fileFormatVersion":"1.0.0","itemInfoEntries":{"A":{"author":"com.apple.CoreML","description":"CoreML Model Specification","name":"model.mlmodel","path":"com.apple.CoreML/model.mlmodel"}},"rootModelIdentifier":"A"}"#;
    let garbage = stored_zip(&[
        ("Data/com.apple.CoreML/model.mlmodel", b"not a protobuf"),
        ("Manifest.json", manifest),
    ]);
    let model = adapter
        .predict(&variant(garbage, vector.clone()), &input)
        .expect_err("corrupt model");
    assert!(model.message.starts_with("Core ML"), "{}", model.message);
    assert!(
        !model.message.contains('/'),
        "no host paths: {}",
        model.message
    );
    let not_a_zip = adapter
        .predict(&variant(b"PK??".to_vec(), vector.clone()), &input)
        .expect_err("not a zip");
    assert!(
        not_a_zip.message.contains("mlpackage-zip"),
        "{}",
        not_a_zip.message
    );
    let no_contract = adapter
        .predict(&variant(Vec::new(), b"{\"cases\":[]}".to_vec()), &input)
        .expect_err("no contract");
    assert!(
        no_contract.message.contains("frame contract"),
        "{}",
        no_contract.message
    );
}
