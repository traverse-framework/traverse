use std::{collections::BTreeSet, fs, path::PathBuf};

use serde_json::Value;
use traverse_contracts::{
    ConnectorContract, HostAdapterWit, ValidationErrorCode, ValidationFailure,
    parse_connector_contract, validate_connector_contract, validate_host_adapter_wit,
};

const CONNECTOR_DIR: &str = "contracts/connectors/traverse.audio-input";
const FIXTURE_PATH: &str = "fixtures/cross-host/host-authority-audio-input-v1/fixture.json";
const SPEC_PATH: &str = "specs/140-host-authority-wit-adapters/spec.md";

const PUBLIC_ERROR_CODES: &[&str] = &[
    "unbound",
    "incompatible",
    "unconfigured",
    "target_incompatible",
    "input_limit_exceeded",
    "cancelled",
    "idempotency_conflict",
    "policy_denied",
    "unavailable",
];

fn repo_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

fn read(relative: &str) -> Result<String, String> {
    fs::read_to_string(repo_path(relative)).map_err(|error| format!("read {relative}: {error}"))
}

fn audio_contract() -> Result<ConnectorContract, String> {
    let json = read(&format!("{CONNECTOR_DIR}/connector_contract.json"))?;
    parse_connector_contract(&json).map_err(|error| format!("{error:?}"))
}

fn pin() -> Result<HostAdapterWit, String> {
    audio_contract()?
        .host_adapter_wit
        .ok_or_else(|| "audio-input contract must pin its WIT package".to_string())
}

fn sample_pin() -> HostAdapterWit {
    HostAdapterWit {
        package: "traverse:audio-input".to_string(),
        version: "1.0.0".to_string(),
        path: "wit/capture.wit".to_string(),
        interface: "capture".to_string(),
    }
}

fn failure_paths(failure: &ValidationFailure) -> Vec<String> {
    failure
        .errors
        .iter()
        .map(|error| error.path.clone())
        .collect()
}

fn expect_wit_failure(pin: &HostAdapterWit, source: &str) -> Result<ValidationFailure, String> {
    match validate_host_adapter_wit(pin, source) {
        Ok(()) => Err("expected wit validation to fail".to_string()),
        Err(failure) => Ok(failure),
    }
}

fn expect_contract_failure(contract: ConnectorContract) -> Result<Vec<String>, String> {
    match validate_connector_contract(contract) {
        Ok(_) => Err("expected connector contract validation to fail".to_string()),
        Err(failure) => Ok(failure_paths(&failure)),
    }
}

#[test]
fn audio_input_contract_pins_a_valid_wit_package() -> Result<(), String> {
    let contract = audio_contract()?;
    let pin = pin()?;
    assert_eq!(pin.package, "traverse:audio-input");
    assert_eq!(pin.version, "1.0.0");
    validate_connector_contract(contract).map_err(|error| format!("{error:?}"))?;

    let source = read(&format!("{CONNECTOR_DIR}/{}", pin.path))?;
    validate_host_adapter_wit(&pin, &source).map_err(|error| format!("{error:?}"))
}

#[test]
fn committed_wit_matches_spec_140_verbatim() -> Result<(), String> {
    let pin = pin()?;
    let committed = read(&format!("{CONNECTOR_DIR}/{}", pin.path))?;
    let spec = read(SPEC_PATH)?;
    let block: Vec<&str> = spec
        .lines()
        .skip_while(|line| *line != "```wit")
        .skip(1)
        .take_while(|line| *line != "```")
        .collect();
    assert!(!block.is_empty(), "spec 140 must contain a wit block");
    assert_eq!(committed.trim_end(), block.join("\n"));
    Ok(())
}

#[test]
fn wit_validation_rejects_forbidden_identifiers() -> Result<(), String> {
    let base = "package traverse:audio-input@1.0.0;\ninterface capture {\n";
    for (line, segment) in [
        ("  record r { device-id: string }", "device"),
        ("  record r { file-path: string }", "path"),
        ("  record r { api-credential: string }", "credential"),
        ("  record r { endpoint: string }", "endpoint"),
        ("  record r { CoreAudio-handle: string }", "coreaudio"),
        ("  record r { workflow-step: string }", "workflow"),
    ] {
        let source = format!("{base}{line}\n}}\n");
        let failure = expect_wit_failure(&sample_pin(), &source)?;
        assert_eq!(failure_paths(&failure), vec!["$.wit.line[3]".to_string()]);
        assert!(
            failure.errors[0].message.contains(segment),
            "{segment} missing from {}",
            failure.errors[0].message
        );
        assert_eq!(
            failure.errors[0].code,
            ValidationErrorCode::InvalidConnectorContract
        );
    }
    Ok(())
}

#[test]
fn wit_validation_requires_pinned_package_and_interface() -> Result<(), String> {
    let missing_package = expect_wit_failure(&sample_pin(), "interface capture {\n}\n")?;
    assert_eq!(failure_paths(&missing_package), vec!["$.wit.package"]);

    let wrong_version = expect_wit_failure(
        &sample_pin(),
        "package traverse:audio-input@2.0.0;\ninterface capture {\n}\n",
    )?;
    assert_eq!(failure_paths(&wrong_version), vec!["$.wit.package"]);

    let missing_interface =
        expect_wit_failure(&sample_pin(), "package traverse:audio-input@1.0.0;\n")?;
    assert_eq!(failure_paths(&missing_interface), vec!["$.wit.interface"]);

    let other_interface = expect_wit_failure(
        &sample_pin(),
        "package traverse:audio-input@1.0.0;\ninterface other {\n}\n",
    )?;
    assert_eq!(failure_paths(&other_interface), vec!["$.wit.interface"]);
    Ok(())
}

#[test]
fn connector_validation_rejects_malformed_wit_pins() -> Result<(), String> {
    let mut contract = audio_contract()?;
    contract.host_adapter_wit = Some(HostAdapterWit {
        package: "no-namespace".to_string(),
        version: "one".to_string(),
        path: "wit/capture.wit".to_string(),
        interface: "Bad_Name".to_string(),
    });
    assert_eq!(
        expect_contract_failure(contract.clone())?,
        vec![
            "$.host_adapter_wit.package",
            "$.host_adapter_wit.version",
            "$.host_adapter_wit.interface",
        ]
    );

    contract.host_adapter_wit = Some(HostAdapterWit {
        package: "Traverse:audio-input".to_string(),
        ..sample_pin()
    });
    assert_eq!(
        expect_contract_failure(contract.clone())?,
        vec!["$.host_adapter_wit.package"]
    );

    for bad_path in [
        "capture.wit",
        "wit/capture.txt",
        "wit/../capture.wit",
        "wit\\capture.wit",
        "/wit/capture.wit",
    ] {
        contract.host_adapter_wit = Some(HostAdapterWit {
            path: bad_path.to_string(),
            ..sample_pin()
        });
        assert_eq!(
            expect_contract_failure(contract.clone())?,
            vec!["$.host_adapter_wit.path"],
            "{bad_path}"
        );
    }
    Ok(())
}

#[test]
fn contracts_without_a_wit_pin_still_validate() -> Result<(), String> {
    let mut contract = audio_contract()?;
    contract.host_adapter_wit = None;
    validate_connector_contract(contract).map_err(|error| format!("{error:?}"))?;
    Ok(())
}

fn str_field<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing string field {key}"))
}

fn array_field<'a>(value: &'a Value, key: &str) -> Result<&'a Vec<Value>, String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("missing array field {key}"))
}

fn event_names(expected: &Value) -> Result<Vec<String>, String> {
    array_field(expected, "events")?
        .iter()
        .map(|event| str_field(event, "event").map(ToOwned::to_owned))
        .collect()
}

#[test]
fn fixture_covers_every_spec_140_fr_013_category() -> Result<(), String> {
    let fixture: Value =
        serde_json::from_str(&read(FIXTURE_PATH)?).map_err(|error| error.to_string())?;
    assert_eq!(
        str_field(&fixture, "kind")?,
        "host_authority_conformance_fixture"
    );

    let required: BTreeSet<&str> = array_field(&fixture, "fr_013_categories")?
        .iter()
        .filter_map(Value::as_str)
        .collect();
    let mut tagged = BTreeSet::new();
    let mut ids = BTreeSet::new();
    for case in array_field(&fixture, "cases")? {
        assert!(ids.insert(str_field(case, "id")?), "duplicate case id");
        for tag in array_field(case, "fr_013")? {
            tagged.insert(tag.as_str().ok_or("tag must be a string")?);
        }
    }
    assert_eq!(tagged, required, "every FR-013 category needs a case");
    Ok(())
}

#[test]
fn fixture_cases_are_internally_consistent() -> Result<(), String> {
    let fixture: Value =
        serde_json::from_str(&read(FIXTURE_PATH)?).map_err(|error| error.to_string())?;
    let mapping = fixture
        .get("adapter_failure_class_mapping")
        .and_then(Value::as_object)
        .ok_or("missing adapter_failure_class_mapping")?;
    for public_code in mapping.values() {
        let code = public_code
            .as_str()
            .ok_or("mapping value must be a string")?;
        assert!(PUBLIC_ERROR_CODES.contains(&code), "unknown code {code}");
    }
    let forbidden: Vec<&str> = array_field(
        fixture.get("redaction").ok_or("missing redaction")?,
        "forbidden_public_substrings",
    )?
    .iter()
    .filter_map(Value::as_str)
    .collect();

    for case in array_field(&fixture, "cases")? {
        let id = str_field(case, "id")?;
        let steps = array_field(case, "steps")?;
        assert!(!steps.is_empty(), "{id}: needs steps");
        for (index, step) in steps.iter().enumerate() {
            let expected = step.get("expected").ok_or("missing expected")?;
            if let Some(same_as) = expected.get("same_as_step").and_then(Value::as_u64) {
                let current = u64::try_from(index).map_err(|error| error.to_string())?;
                assert!(
                    same_as >= 1 && same_as <= current,
                    "{id}: same_as_step must reference an earlier step"
                );
                assert!(array_field(step, "adapter_calls")?.is_empty());
                continue;
            }
            let names = event_names(expected)?;
            assert_eq!(names.first().map(String::as_str), Some("accepted"), "{id}");
            let terminal = names.last().map(String::as_str);
            let result_class = str_field(expected, "result_class")?;
            let want_terminal = match result_class {
                "succeeded" => "completed",
                "cancelled" => "cancelled",
                _ => "failed",
            };
            assert_eq!(terminal, Some(want_terminal), "{id}: terminal event");
            if let Some(code) = expected.get("error_code").and_then(Value::as_str) {
                assert!(PUBLIC_ERROR_CODES.contains(&code), "{id}: {code}");
            }
            if !array_field(step, "adapter_calls")?.is_empty() {
                assert_eq!(names.get(1).map(String::as_str), Some("started"), "{id}");
            }
            let public = expected.to_string();
            for needle in &forbidden {
                assert!(
                    !public.contains(needle),
                    "{id}: public output leaks {needle}"
                );
            }
        }
    }
    Ok(())
}

#[test]
fn fixture_adapter_failures_use_only_wit_failure_classes() -> Result<(), String> {
    let fixture: Value =
        serde_json::from_str(&read(FIXTURE_PATH)?).map_err(|error| error.to_string())?;
    let mapping = fixture
        .get("adapter_failure_class_mapping")
        .and_then(Value::as_object)
        .ok_or("missing adapter_failure_class_mapping")?;
    let wit = read(&format!("{CONNECTOR_DIR}/wit/capture.wit"))?;
    for class in mapping.keys() {
        assert!(
            wit.contains(class.as_str()),
            "{class} not in WIT failure-class"
        );
    }
    for case in array_field(&fixture, "cases")? {
        for step in array_field(case, "steps")? {
            for call in array_field(step, "adapter_calls")? {
                let Some(failure) = call.get("returns").and_then(|r| r.get("err")) else {
                    continue;
                };
                let class = str_field(failure, "class")?;
                assert!(mapping.contains_key(class), "unknown failure class {class}");
                let expected = step
                    .get("expected")
                    .and_then(|e| e.get("error_code"))
                    .and_then(Value::as_str);
                assert_eq!(
                    expected,
                    mapping.get(class).and_then(Value::as_str),
                    "adapter class {class} must map to the public error code"
                );
            }
        }
    }
    Ok(())
}
