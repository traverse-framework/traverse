//! Shared Spec 138 rights conformance suite (0.8.0, Decision 107, FR-041):
//! runs every data-only case in `fixtures/models/rights-conformance/suite.json`
//! through the native `ExactModelHostConnector` on a fresh host per case. The
//! web and Swift embedders run the same file and must match it exactly.
#![cfg(feature = "wasmtime-executor")]
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use serde_json::{Map, Value, json};
use std::fs;
use traverse_runtime::exact_model::{
    ExactModelHostConnector, ExactModelPin, ExecutionPolicy, ModelUsage, PackageStatusEntry,
    TrustedModelKeys,
};
use traverse_runtime::host_connector_dispatch::{
    HostConnectorError, HostConnectorHostRequest, HostConnectorPort, MODEL_EXECUTE_OPERATION,
    MODEL_RUNTIME_CONNECTOR, ModelFailureReason,
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

fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

fn suite() -> Value {
    serde_json::from_slice(&read("fixtures/models/rights-conformance/suite.json")).expect("suite")
}

/// The public error shape every embedder compares: code, reason, detail.
fn error_json(error: &HostConnectorError) -> Value {
    let mut out = json!({
        "ok": false,
        "code": error.code.as_str(),
        "reason": error.reason.map(ModelFailureReason::as_str),
    });
    if let Some(detail) = &error.detail {
        out["detail"] = serde_json::to_value(detail).expect("detail");
    }
    out
}

fn status_entries(value: &Value) -> Vec<(String, PackageStatusEntry)> {
    value
        .as_object()
        .map(|entries| {
            entries
                .iter()
                .map(|(digest, entry)| {
                    (
                        digest.clone(),
                        serde_json::from_value(entry.clone()).expect("status entry"),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

fn host_for(suite: &Value, case: &Value) -> ExactModelHostConnector {
    let pins: Vec<ExactModelPin> = serde_json::from_value(case["pins"].clone()).expect("pins");
    let public: [u8; 32] = hex(suite["trusted_public_key_hex"].as_str().expect("key"))
        .try_into()
        .expect("32-byte key");
    let mut keys = TrustedModelKeys::new();
    keys.trust(&public).expect("trust");
    let mut host = ExactModelHostConnector::new(pins, keys);
    host.model_usage = serde_json::from_value::<Option<ModelUsage>>(case["model_usage"].clone())
        .expect("model_usage");
    host.host_requires_commercial = case["host_requires_commercial"].as_bool().unwrap_or(false);
    host.set_package_status(status_entries(&case["package_status"]));
    let execute = &suite["execute"];
    host.policies.insert(
        execute["policy_ref"]
            .as_str()
            .expect("policy_ref")
            .to_string(),
        ExecutionPolicy {
            policy_ref: execute["policy_ref"]
                .as_str()
                .expect("policy_ref")
                .to_string(),
            allowed_classifications: serde_json::from_value(
                execute["allowed_classifications"].clone(),
            )
            .expect("classifications"),
            max_output_bytes: execute["max_output_bytes"].as_u64().expect("max_output"),
        },
    );
    host
}

fn register(suite: &Value, host: &mut ExactModelHostConnector, step: &Value) -> Value {
    let dir = format!(
        "{}/{}",
        suite["package_dir"].as_str().expect("package_dir"),
        step["package"].as_str().expect("package")
    );
    let manifest = read(&format!("{dir}/model.manifest.json"));
    let mut wasm = read(suite["wasm_path"].as_str().expect("wasm_path"));
    let mut signature = read(&format!("{dir}/model.sig.json"));
    match step["tamper"].as_str() {
        Some("wasm") => wasm.push(0),
        Some("signature") => {
            let mut document: Value = serde_json::from_slice(&signature).expect("sig json");
            let mut bytes = hex(document["signature"].as_str().expect("signature"));
            bytes[0] ^= 0x01;
            document["signature"] = json!(hex_encode(&bytes));
            signature = serde_json::to_vec(&document).expect("sig bytes");
        }
        Some(other) => panic!("unknown tamper {other}"),
        None => {}
    }
    match host.register_package(&manifest, wasm, &signature) {
        Ok(digest) => json!({ "ok": true, "digest": digest }),
        Err(error) => error_json(&error),
    }
}

fn execute(suite: &Value, host: &mut ExactModelHostConnector, step: &Value) -> Value {
    let package = step["package"].as_str().expect("package");
    let pin = host
        .pins
        .iter()
        .find(|pin| pin.model_id == format!("fixture.rights.{package}"))
        .expect("pin for package")
        .clone();
    let execute = &suite["execute"];
    let input = hex(execute["input_hex"].as_str().expect("input"));
    let input_ref = host.io.stage_model_input(&input, 4096).expect("stage");
    let mut payload = Map::new();
    payload.insert(
        "model_ref".to_string(),
        json!({ "model_id": pin.model_id, "version": pin.version, "digest": pin.digest }),
    );
    payload.insert("input_ref".to_string(), json!(input_ref));
    for field in [
        "policy_ref",
        "data_classification",
        "input_schema_ref",
        "input_schema_version",
        "max_output_bytes",
    ] {
        payload.insert(field.to_string(), execute[field].clone());
    }
    let result = host.invoke(&HostConnectorHostRequest {
        connector_id: MODEL_RUNTIME_CONNECTOR.to_string(),
        operation: MODEL_EXECUTE_OPERATION.to_string(),
        binding_id: "rights-conformance".to_string(),
        target_family: "native".to_string(),
        correlation_id: "rights-conformance".to_string(),
        payload: Value::Object(payload),
        cancel_requested: false,
    });
    match result {
        Ok(result) => {
            let output = host
                .io
                .read_model_output(result.artifact_ref.as_deref().expect("output_ref"), 4096)
                .expect("read output");
            json!({
                "ok": true,
                "output_hex": hex_encode(&output),
                "model_evidence": result.model_evidence,
            })
        }
        Err(error) => error_json(&error),
    }
}

#[test]
fn rights_conformance_suite_passes_on_the_native_host() {
    let suite = suite();
    let cases = suite["cases"].as_array().expect("cases");
    let mut scenarios: Vec<u64> = Vec::new();
    for case in cases {
        let id = case["id"].as_str().expect("id");
        let mut host = host_for(&suite, case);
        for (index, step) in case["steps"].as_array().expect("steps").iter().enumerate() {
            let actual = match step["op"].as_str().expect("op") {
                "register" => register(&suite, &mut host, step),
                "execute" => execute(&suite, &mut host, step),
                "rights_record" => {
                    let package = step["package"].as_str().expect("package");
                    let digest = host
                        .pins
                        .iter()
                        .find(|pin| pin.model_id == format!("fixture.rights.{package}"))
                        .expect("pin")
                        .digest
                        .clone();
                    serde_json::to_value(host.model_rights_record(&digest)).expect("record")
                }
                "set_package_status" => {
                    host.set_package_status(status_entries(&step["entries"]));
                    continue;
                }
                other => panic!("{id}: unknown op {other}"),
            };
            assert_eq!(actual, step["expect"], "{id} step {index}");
        }
        scenarios.extend(
            case["scenarios"]
                .as_array()
                .expect("scenarios")
                .iter()
                .map(|scenario| scenario.as_u64().expect("scenario")),
        );
    }
    // Scenario 10 (native/browser parity) is this shared file itself.
    scenarios.push(10);
    scenarios.sort_unstable();
    scenarios.dedup();
    assert_eq!(scenarios, (1..=10).collect::<Vec<_>>());
}
