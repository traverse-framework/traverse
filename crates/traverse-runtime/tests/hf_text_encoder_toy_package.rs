//! Multi-IO ONNX runner package (#1661): HF-style text-encoder toy with
//! `input_ids` + `attention_mask` → `last_hidden_state` + `pooler_output`.
//! Byte-identical on wasmtime and wasmi through the governed path; mismatched
//! frame bundles fail closed as `frame_mismatch`.
#![cfg(all(feature = "wasmtime-executor", feature = "wasmi-executor"))]
#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_used,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]

use serde_json::{Value, json};
use std::fs;
use traverse_runtime::exact_model::{
    ExactModelHostConnector, ExactModelPin, ExecutionPolicy, ModelEngine, ModelUsage,
    PlacementPolicy, TrustedModelKeys,
};
use traverse_runtime::host_connector_dispatch::{
    HostConnectorError, HostConnectorErrorCode, HostConnectorHostRequest, HostConnectorPort,
    MODEL_EXECUTE_OPERATION, MODEL_RUNTIME_CONNECTOR,
};

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
const DIR: &str = "fixtures/models/hf-text-encoder-toy-1.0.0";
const MAX_INPUT: u64 = 96;
const MAX_OUTPUT: u64 = 196;

fn read(relative: &str) -> Vec<u8> {
    fs::read(format!("{ROOT}/{relative}")).expect(relative)
}

fn hex(value: &str) -> Vec<u8> {
    (0..value.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&value[index..index + 2], 16).expect("hex"))
        .collect()
}

fn vector() -> Value {
    serde_json::from_slice(&read(
        "fixtures/models/conformance/signed-hf-text-encoder-toy.json",
    ))
    .expect("vector")
}

fn registered_host(engine: ModelEngine) -> (ExactModelHostConnector, ExactModelPin) {
    let vector = vector();
    let pin: ExactModelPin = serde_json::from_value(vector["pin"].clone()).expect("pin");
    let public: [u8; 32] = hex(vector["trusted_public_key_hex"].as_str().expect("key"))
        .try_into()
        .expect("32 bytes");
    let mut keys = TrustedModelKeys::new();
    keys.trust(&public).expect("trust");
    let mut host = ExactModelHostConnector::new(vec![pin.clone()], keys);
    host.model_usage = Some(ModelUsage::Commercial);
    host.engine = engine;
    host.register_package(
        &read(&format!("{DIR}/model.manifest.json")),
        read(&format!("{DIR}/model.wasm")),
        &read(&format!("{DIR}/model.sig.json")),
        &PlacementPolicy {
            policy_ref: "policy-1".to_string(),
            ..PlacementPolicy::default()
        },
    )
    .expect("signed multi-IO runner package must verify");
    host.policies.insert(
        "policy-1".to_string(),
        ExecutionPolicy {
            policy_ref: "policy-1".to_string(),
            allowed_classifications: vec!["sensitive".to_string()],
            max_output_bytes: MAX_OUTPUT,
            ..ExecutionPolicy::default()
        },
    );
    (host, pin)
}

fn request(pin: &ExactModelPin, input_ref: &str) -> HostConnectorHostRequest {
    HostConnectorHostRequest {
        connector_id: MODEL_RUNTIME_CONNECTOR.to_string(),
        operation: MODEL_EXECUTE_OPERATION.to_string(),
        binding_id: "b".to_string(),
        target_family: "macos".to_string(),
        correlation_id: "c".to_string(),
        payload: json!({
            "model_ref": { "model_id": pin.model_id, "version": pin.version, "digest": pin.digest },
            "input_ref": input_ref,
            "policy_ref": "policy-1",
            "data_classification": "sensitive",
            "input_schema_ref": "schema:traverse-hf-text-encoder-toy-in",
            "input_schema_version": "1.0.0",
            "max_output_bytes": MAX_OUTPUT
        }),
        cancel_requested: false,
    }
}

fn execute(
    host: &mut ExactModelHostConnector,
    pin: &ExactModelPin,
    frame: &[u8],
) -> Result<Vec<u8>, HostConnectorError> {
    let input_ref = host.io.stage_model_input(frame, 4096).expect("stage");
    let result = host.invoke(&request(pin, &input_ref))?;
    Ok(host
        .io
        .read_model_output(
            result.artifact_ref.as_deref().expect("output_ref"),
            MAX_OUTPUT as usize,
        )
        .expect("read"))
}

fn check_engine(engine: ModelEngine) {
    let (mut host, pin) = registered_host(engine);
    for case in vector()["cases"].as_array().expect("cases") {
        let output = execute(
            &mut host,
            &pin,
            &hex(case["input_frame_hex"].as_str().expect("in")),
        )
        .unwrap_or_else(|code| panic!("{engine:?}: {code:?}"));
        assert_eq!(
            output,
            hex(case["output_frame_hex"].as_str().expect("out")),
            "{engine:?} {}",
            case["name"]
        );
    }
    let rejected = vector()["rejected_cases"]
        .as_array()
        .expect("rejected_cases")
        .clone();
    assert_eq!(rejected.len(), 3);
    for case in &rejected {
        let frame = hex(case["input_frame_hex"].as_str().expect("frame"));
        let error = execute(&mut host, &pin, &frame).expect_err("rejected");
        assert_eq!(
            (
                json!(error.code.as_str()),
                error.reason.map(|reason| json!(reason.as_str()))
            ),
            (case["code"].clone(), Some(case["reason"].clone())),
            "{engine:?} {}",
            case["name"]
        );
    }
    assert_eq!(
        execute(&mut host, &pin, &[0; (MAX_INPUT + 1) as usize])
            .expect_err("too big")
            .code,
        HostConnectorErrorCode::ResourceExhausted,
        "{engine:?}"
    );
}

#[test]
fn multi_io_package_matches_the_vector_on_wasmtime() {
    check_engine(ModelEngine::Wasmtime);
}

#[test]
fn multi_io_package_matches_the_vector_on_wasmi() {
    check_engine(ModelEngine::Wasmi);
}
