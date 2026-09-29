//! First trained Spec 138 exact-ref package (Decision 102, `#1461`): the
//! checked-in, signed `digits-mlp-1.0.0` MLP runs through the governed
//! `register_package` → `model.execute` path on `wasm-cpu`, and must clear
//! the ≥ 95% held-out accuracy floor on the vendored UCI optdigits test split.
#![cfg(feature = "wasmtime-executor")]
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use serde_json::{Value, json};
use std::fs;
use traverse_runtime::exact_model::{
    CommercialUse, ExactModelHostConnector, ExactModelPin, ExecutionPolicy, TrustedModelKeys,
    decode_guest_frame, encode_guest_frame,
};
use traverse_runtime::host_connector_dispatch::{
    HostConnectorErrorCode, HostConnectorHostRequest, HostConnectorPort, MODEL_EXECUTE_OPERATION,
    MODEL_RUNTIME_CONNECTOR,
};

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

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
    serde_json::from_slice(&read("fixtures/models/conformance/signed-digits-mlp.json"))
        .expect("vector")
}

fn registered_host() -> (ExactModelHostConnector, ExactModelPin) {
    let vector = vector();
    let pin: ExactModelPin = serde_json::from_value(vector["pin"].clone()).expect("pin");
    let public: [u8; 32] = hex(vector["trusted_public_key_hex"].as_str().expect("key"))
        .try_into()
        .expect("32 bytes");
    let mut keys = TrustedModelKeys::new();
    keys.trust(&public).expect("trust");
    let mut host = ExactModelHostConnector::new(vec![pin.clone()], keys);
    let dir = "fixtures/models/digits-mlp-1.0.0";
    host.register_package(
        &read(&format!("{dir}/model.manifest.json")),
        read(&format!("{dir}/model.wasm")),
        &read(&format!("{dir}/model.sig.json")),
    )
    .expect("signed digits package must verify");
    host.policies.insert(
        "policy-1".to_string(),
        ExecutionPolicy {
            policy_ref: "policy-1".to_string(),
            allowed_classifications: vec!["sensitive".to_string()],
            max_output_bytes: 64,
        },
    );
    (host, pin)
}

fn request(
    pin: &ExactModelPin,
    input_ref: &str,
    extras: &[(&str, Value)],
) -> HostConnectorHostRequest {
    let mut payload = json!({
        "model_ref": { "model_id": pin.model_id, "version": pin.version, "digest": pin.digest },
        "input_ref": input_ref,
        "policy_ref": "policy-1",
        "data_classification": "sensitive",
        "input_schema_ref": "schema:traverse-digits-mlp-in",
        "input_schema_version": "1.0.0",
        "max_output_bytes": 64
    });
    for (key, value) in extras {
        payload[*key] = value.clone();
    }
    HostConnectorHostRequest {
        connector_id: MODEL_RUNTIME_CONNECTOR.to_string(),
        operation: MODEL_EXECUTE_OPERATION.to_string(),
        binding_id: "b".to_string(),
        target_family: "macos".to_string(),
        correlation_id: "c".to_string(),
        payload,
        cancel_requested: false,
    }
}

fn run(host: &mut ExactModelHostConnector, pin: &ExactModelPin, frame: &[u8]) -> Vec<u8> {
    let input_ref = host.io.stage_model_input(frame, 268).expect("stage");
    let result = host
        .invoke(&request(pin, &input_ref, &[]))
        .expect("execute");
    host.io
        .read_model_output(result.artifact_ref.as_deref().expect("output_ref"), 64)
        .expect("read")
}

fn pixel_frame(pixels: &[f32]) -> Vec<u8> {
    let payload: Vec<u8> = pixels
        .iter()
        .flat_map(|pixel| pixel.to_le_bytes())
        .collect();
    encode_guest_frame(2, &[64], &payload)
}

#[test]
fn trained_package_clears_the_held_out_accuracy_floor() {
    let (mut host, pin) = registered_host();
    let rows =
        String::from_utf8(read("fixtures/datasets/uci-optdigits/optdigits.tes")).expect("utf8");
    let mut total = 0_u32;
    let mut correct = 0_u32;
    for line in rows.lines() {
        let values: Vec<f32> = line
            .split(',')
            .map(|v| v.parse().expect("number"))
            .collect();
        let (label, pixels) = values.split_last().expect("row");
        let (dtype, dims, payload) =
            decode_guest_frame(&run(&mut host, &pin, &pixel_frame(pixels))).expect("output frame");
        assert_eq!((dtype, dims), (3, vec![11]));
        let predicted = f32::from_le_bytes(payload[40..44].try_into().expect("label"));
        total += 1;
        correct += u32::from(predicted.to_bits() == label.to_bits());
    }
    assert_eq!(total, 1797);
    let accuracy = f64::from(correct) / f64::from(total);
    assert!(
        accuracy >= 0.95,
        "held-out accuracy {accuracy:.4} is below the 95% floor"
    );
    // Same count the host trainer and the guest's own host tests report:
    // the wasm guest is bit-identical to the trained model.
    assert_eq!(correct, 1727);
}

#[test]
fn trained_package_matches_the_conformance_vector_and_exposes_rights() {
    let (mut host, pin) = registered_host();
    for case in vector()["cases"].as_array().expect("cases") {
        let output = run(
            &mut host,
            &pin,
            &hex(case["input_frame_hex"].as_str().expect("in")),
        );
        assert_eq!(output, hex(case["output_frame_hex"].as_str().expect("out")));
    }
    let rights = host.model_rights(&pin.digest).expect("rights");
    assert_eq!(rights.license_id, "CC-BY-4.0");
    assert_eq!(rights.commercial_use, CommercialUse::Allowed);
    assert!(rights.attribution.contains("10.24432/C50P49"));
}

#[test]
fn trained_package_fails_closed_on_bad_frames_and_enforces_fuel() {
    let (mut host, pin) = registered_host();
    // Wrong dims: the guest rejects the frame (-1) and the host fails closed.
    let bad = encode_guest_frame(2, &[63], &[0; 252]);
    let input_ref = host.io.stage_model_input(&bad, 268).expect("stage");
    assert_eq!(
        host.invoke(&request(&pin, &input_ref, &[]))
            .expect_err("bad frame")
            .code,
        HostConnectorErrorCode::ResourceExhausted
    );
    // Oversized input exceeds the manifest's 268-byte ceiling.
    let input_ref = host.io.stage_model_input(&[0; 300], 4096).expect("stage");
    assert_eq!(
        host.invoke(&request(&pin, &input_ref, &[]))
            .expect_err("too big")
            .code,
        HostConnectorErrorCode::ResourceExhausted
    );
    // ~56k fuel per inference: a 10k per-call ceiling traps the guest.
    let input_ref = host
        .io
        .stage_model_input(&pixel_frame(&[1.0; 64]), 268)
        .expect("stage");
    assert_eq!(
        host.invoke(&request(&pin, &input_ref, &[("max_fuel", json!(10_000))]))
            .expect_err("fuel")
            .code,
        HostConnectorErrorCode::ExecutionFailed
    );
}
