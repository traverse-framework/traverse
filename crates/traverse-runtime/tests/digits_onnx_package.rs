//! Runner-built Spec 138 package (Decision 105/106, `#1591`): the digits MLP
//! exported to ONNX and packaged on the generic ONNX runner guest with
//! `traverse-cli model package-onnx`. The signed package must produce the
//! conformance vector byte-for-byte on both native engines through the
//! governed `register_package` → `model.execute` path, fail closed on
//! mismatched frames, and score exactly what the hand-written guest scores.
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
    CommercialUse, DerivationKind, ExactModelHostConnector, ExactModelPin, ExecutionPolicy,
    ModelEngine, ModelUsage, TrustedModelKeys, decode_guest_frame, encode_guest_frame,
};
use traverse_runtime::host_connector_dispatch::{
    HostConnectorErrorCode, HostConnectorHostRequest, HostConnectorPort, MODEL_EXECUTE_OPERATION,
    MODEL_RUNTIME_CONNECTOR,
};

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
const DIR: &str = "fixtures/models/digits-onnx-1.0.0";
const MAX_INPUT: u64 = 272;
const MAX_OUTPUT: u64 = 56;

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
    serde_json::from_slice(&read("fixtures/models/conformance/signed-digits-onnx.json"))
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
    )
    .expect("signed runner-built package must verify");
    host.policies.insert(
        "policy-1".to_string(),
        ExecutionPolicy {
            policy_ref: "policy-1".to_string(),
            allowed_classifications: vec!["sensitive".to_string()],
            max_output_bytes: MAX_OUTPUT,
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
            "input_schema_ref": "schema:traverse-digits-onnx-in",
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
) -> Result<Vec<u8>, HostConnectorErrorCode> {
    let input_ref = host.io.stage_model_input(frame, 4096).expect("stage");
    let result = host.invoke(&request(pin, &input_ref)).map_err(|e| e.code)?;
    Ok(host
        .io
        .read_model_output(
            result.artifact_ref.as_deref().expect("output_ref"),
            MAX_OUTPUT as usize,
        )
        .expect("read"))
}

fn pixel_frame(dtype: u8, dims: &[u32], pixels: &[f32]) -> Vec<u8> {
    let payload: Vec<u8> = pixels
        .iter()
        .flat_map(|pixel| pixel.to_le_bytes())
        .collect();
    encode_guest_frame(dtype, dims, &payload)
}

fn argmax(output: &[u8]) -> usize {
    let (dtype, dims, payload) = decode_guest_frame(output).expect("output frame");
    assert_eq!((dtype, dims), (3, vec![1, 10]));
    let logits: Vec<f32> = payload
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes(c.try_into().expect("f32")))
        .collect();
    (0..logits.len())
        .max_by(|a, b| logits[*a].total_cmp(&logits[*b]))
        .expect("logits")
}

/// One registration (and so one compile) per engine: the vector must match
/// byte-for-byte, then mismatched frames must fail closed.
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
            "{engine:?}"
        );
        assert_eq!(
            argmax(&output),
            case["predicted"].as_u64().expect("predicted") as usize
        );
    }
    let rights = host.model_rights(&pin.digest).expect("rights");
    assert_eq!(rights.license_id, "CC-BY-4.0");
    assert_eq!(rights.commercial_use, CommercialUse::Allowed);
    assert!(rights.attribution.contains("10.24432/C50P49"));
    let derivation = rights.derivation.as_ref().expect("rights.derivation");
    assert_eq!(derivation.kind, DerivationKind::Converted);
    assert_eq!(
        derivation.source_digest,
        "c16f5e4d2b901067512812fa821f540fc8b5770845c133d608e30f8d61fd5efc"
    );
    assert_eq!(derivation.source_commercial_use, CommercialUse::Allowed);

    let good = pixel_frame(2, &[1, 64], &[1.0; 64]);
    let mut oversize = good.clone();
    oversize.extend_from_slice(&[0; 4]);
    let mut nan = [1.0_f32; 64];
    nan[3] = f32::NAN;
    let rejected = [
        // dtype mismatch (output dtype code on the input)
        pixel_frame(3, &[1, 64], &[1.0; 64]),
        // shape mismatch: rank, then dims
        pixel_frame(2, &[64], &[1.0; 64]),
        pixel_frame(2, &[2, 32], &[1.0; 64]),
        // trailing bytes past the declared payload
        oversize,
        // non-finite values
        pixel_frame(2, &[1, 64], &nan),
    ];
    for frame in &rejected {
        assert_eq!(
            execute(&mut host, &pin, frame).expect_err("rejected"),
            HostConnectorErrorCode::ResourceExhausted,
            "{engine:?}"
        );
    }
    // Larger than the manifest's max_input_bytes: refused before the guest.
    assert_eq!(
        execute(&mut host, &pin, &[0; (MAX_INPUT + 1) as usize]).expect_err("too big"),
        HostConnectorErrorCode::ResourceExhausted,
        "{engine:?}"
    );
}

#[test]
fn runner_package_matches_the_vector_and_fails_closed_on_wasmtime() {
    check_engine(ModelEngine::Wasmtime);
}

#[test]
fn runner_package_matches_the_vector_and_fails_closed_on_wasmi() {
    check_engine(ModelEngine::Wasmi);
}

/// The full held-out split on one warm instance (the governed host compiles
/// per call, which is the wrong cost model for 1797 rows).
#[test]
fn runner_package_scores_the_same_held_out_accuracy_as_the_hand_written_guest() {
    use wasmtime::{Engine, Instance, Module, Store};

    let engine = Engine::default();
    let module = Module::new(&engine, read(&format!("{DIR}/model.wasm"))).expect("module");
    let mut store = Store::new(&engine, ());
    let instance = Instance::new(&mut store, &module, &[]).expect("import-free");
    let alloc = instance
        .get_typed_func::<i32, i32>(&mut store, "model_alloc")
        .expect("model_alloc");
    let execute = instance
        .get_typed_func::<(i32, i32, i32, i32), i32>(&mut store, "model_execute")
        .expect("model_execute");
    let memory = instance.get_memory(&mut store, "memory").expect("memory");
    let in_ptr = alloc.call(&mut store, MAX_INPUT as i32).expect("alloc");
    let out_ptr = alloc.call(&mut store, MAX_OUTPUT as i32).expect("alloc");

    let rows =
        String::from_utf8(read("fixtures/datasets/uci-optdigits/optdigits.tes")).expect("utf8");
    let (mut total, mut correct) = (0_u32, 0_u32);
    for line in rows.lines() {
        let values: Vec<f32> = line
            .split(',')
            .map(|v| v.parse().expect("number"))
            .collect();
        let (label, pixels) = values.split_last().expect("row");
        let frame = pixel_frame(2, &[1, 64], pixels);
        memory
            .write(&mut store, in_ptr as usize, &frame)
            .expect("write");
        let len = execute
            .call(
                &mut store,
                (in_ptr, frame.len() as i32, out_ptr, MAX_OUTPUT as i32),
            )
            .expect("execute");
        let mut output = vec![0; usize::try_from(len).expect("runner accepted the frame")];
        memory
            .read(&store, out_ptr as usize, &mut output)
            .expect("read");
        total += 1;
        correct += u32::from(argmax(&output) == *label as usize);
    }
    assert_eq!((correct, total), (1727, 1797));
}
