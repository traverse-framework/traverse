//! Native host: published crates only, using the shipped `traverse.model-runtime` binding.
use serde_json::{Value, json};
use std::sync::Arc;
use traverse_contracts::ConnectorRequirement;
use traverse_runtime::exact_model::{
    ExactModelHostConnector, ExactModelPin, ExecutionPolicy, ModelUsage, PackageStatus,
    PackageStatusEntry, PlacementPolicy, TrustedModelKeys,
};
use traverse_runtime::executor::{ActivatedConnector, MediatedConnectorContext, WasmExecutor};
use traverse_runtime::model_runtime_mediated::{
    ExactModelMediatedConnector, MODEL_RUNTIME_CONNECTOR_VERSION, activate_exact_model_runtime,
};

const POLICY: &str = "policy:default-wasm-cpu";

fn read(path: &str) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|error| panic!("{path}: {error}"))
}
fn json_file(path: &str) -> Value {
    serde_json::from_slice(&read(path)).expect("json")
}
fn unhex(text: &str) -> Vec<u8> {
    (0..text.len()).step_by(2).map(|i| u8::from_str_radix(&text[i..i + 2], 16).expect("hex")).collect()
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn main() {
    let vector = json_file("published/package/vector.json");
    let input = json_file("published/input.json");
    let guest = read("bundle/components/model-execute/model-execute-agent.wasm");
    let contract = json_file("bundle/components/model-execute/contract.json");
    let declared: Vec<ConnectorRequirement> =
        serde_json::from_value(contract["connector_requirements"].clone()).expect("requirements");
    let frame = unhex(vector["cases"][0]["input_frame_hex"].as_str().expect("frame"));
    let executor = WasmExecutor::new().expect("executor");
    let mut results = Vec::new();
    for scenario in json_file("src/scenarios.json").as_array().expect("scenarios") {
        let mut pin: ExactModelPin = serde_json::from_value(vector["pin"].clone()).expect("pin");
        let digest = pin.digest.clone();
        if let Some(other) = scenario["pin_digest"].as_str() {
            pin.digest = other.to_string();
        }
        let mut keys = TrustedModelKeys::new();
        if scenario["untrusted"] != json!(true) {
            let public: [u8; 32] = unhex(vector["trusted_public_key_hex"].as_str().expect("key"))
                .try_into()
                .expect("key length");
            keys.trust(&public).expect("trust");
        }
        let mut host = ExactModelHostConnector::new(vec![pin], keys);
        if scenario["no_usage"] != json!(true) {
            host.model_usage = Some(ModelUsage::NonCommercial);
        }
        let mut wasm = read("published/package/model.wasm");
        if scenario["tamper_wasm"] == json!(true) {
            let last = wasm.len() - 1;
            wasm[last] ^= 1;
        }
        let registered = host.register_package(
            &read("published/package/model.manifest.json"),
            wasm,
            &read("published/package/model.sig.json"),
            &PlacementPolicy { policy_ref: POLICY.to_string(), ..PlacementPolicy::default() },
        );
        let registration_reason =
            serde_json::to_value(registered.err().and_then(|error| error.reason)).expect("reason");
        if scenario["revoke"] == json!(true) {
            host.set_package_status([(
                digest,
                PackageStatusEntry { status: PackageStatus::Revoked, reason: "demo".to_string() },
            )]);
        }
        host.policies.insert(
            POLICY.to_string(),
            ExecutionPolicy {
                policy_ref: POLICY.to_string(),
                allowed_classifications: vec!["public".to_string()],
                max_output_bytes: 56,
                ..ExecutionPolicy::default()
            },
        );
        host.io.stage_model_input(&frame, 4096).expect("stage");
        let unbound = scenario["unbound"] == json!(true);
        // Keeps a handle to read the frame back, which the one-call binding doesn't offer.
        let connector = Arc::new(ExactModelMediatedConnector::new(host));
        let activated: Vec<ActivatedConnector> = if unbound {
            Vec::new()
        } else {
            vec![ActivatedConnector {
                connector_id: "traverse.model-runtime".to_string(),
                version: MODEL_RUNTIME_CONNECTOR_VERSION.to_string(),
                implementation: connector.clone(),
            }]
        };
        let output = executor
            .run_bytes_with_mediated_connectors(
                &guest,
                &input,
                "model.execute",
                MediatedConnectorContext { declared_requirements: declared.clone(), activated_connectors: activated },
            )
            .expect("capability ran")
            .value;
        let output_frame_hex = output["output_ref"]
            .as_str()
            .filter(|reference| !reference.is_empty())
            .map(|reference| {
                connector
                    .with_host_mut(|host| host.io.read_model_output(reference, 56).expect("read"))
                    .expect("host")
            })
            .map(|bytes| hex(&bytes));
        results.push(json!({
            "name": scenario["name"], "registration_reason": registration_reason,
            "output": output, "output_frame_hex": output_frame_hex,
        }));
    }
    // The documented one-call binding over the same signed package.
    let pin: ExactModelPin = serde_json::from_value(vector["pin"].clone()).expect("pin");
    let mut keys = TrustedModelKeys::new();
    let public: [u8; 32] = unhex(vector["trusted_public_key_hex"].as_str().expect("key"))
        .try_into()
        .expect("key length");
    keys.trust(&public).expect("trust");
    let mut host = ExactModelHostConnector::new(vec![pin], keys);
    host.model_usage = Some(ModelUsage::NonCommercial);
    host.register_package(
        &read("published/package/model.manifest.json"),
        read("published/package/model.wasm"),
        &read("published/package/model.sig.json"),
        &PlacementPolicy { policy_ref: POLICY.to_string(), ..PlacementPolicy::default() },
    )
    .expect("register");
    host.policies.insert(
        POLICY.to_string(),
        ExecutionPolicy {
            policy_ref: POLICY.to_string(),
            allowed_classifications: vec!["public".to_string()],
            max_output_bytes: 56,
            ..ExecutionPolicy::default()
        },
    );
    host.io.stage_model_input(&frame, 4096).expect("stage");
    let one_call = executor
        .run_bytes_with_mediated_connectors(
            &guest,
            &input,
            "model.execute",
            MediatedConnectorContext {
                declared_requirements: declared,
                activated_connectors: vec![activate_exact_model_runtime(host)],
            },
        )
        .expect("capability ran")
        .value;
    println!("{}", serde_json::to_string_pretty(&json!({
        "host": "native: crates.io traverse-runtime 0.16.1, wasmtime",
        "results": results, "one_call_binding_output": one_call,
    })).expect("json"));
}
