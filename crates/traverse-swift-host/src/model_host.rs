//! Spec 138 exact-ref model host behind the sixth audited C-ABI symbol,
//! `traverse_swift_host_model_call` (Decision 104, ADR-0078). This module is
//! safe Rust: the only pointer handling stays in `lib.rs`.
//!
//! **Frame** (request and response): `[u32 LE header_len][JSON header][payload]`.
//! The header's `segments` object maps names to `[offset, length]` in the
//! payload, so large model packages cross without base64 inflation.
//!
//! Operations (`header.op`): `create` (handle 0; returns a model handle),
//! `register`, `stage_input`, `execute`, `read_output`, `rights`, `cancel`,
//! `drop_ref`, `destroy`. Model-level failures are data
//! (`{"ok":false,"error":{code,reason,message}}`); envelope failures return a
//! non-OK status from the ABI.
//!
//! Model hosts live in a registry of `Arc` states: `execute` holds the
//! connector lock for the whole inference while `cancel` only flips the
//! shared atomic, so a cancel from another thread interrupts mid-run at the
//! next `wasmi` fuel slice without aliasing the running host.
#![forbid(unsafe_code)]

use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;
use traverse_runtime::exact_model::{
    ExactModelHostConnector, ExactModelPin, ExecutionPolicy, HostModelLimits, ModelEngine,
    PLACEMENT_WASM_CPU, TrustedModelKeys,
};
use traverse_runtime::host_connector_dispatch::{
    HostConnectorError, HostConnectorHostRequest, HostConnectorPort, MODEL_EXECUTE_OPERATION,
    MODEL_RUNTIME_CONNECTOR, ModelFailureReason,
};

/// Envelope-level failure (maps to a non-OK ABI status).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvelopeError {
    /// Malformed frame, header, segment, or operation.
    InvalidInput(&'static str),
    /// Unknown or destroyed model handle.
    InvalidHandle,
}

struct ModelHost {
    connector: Mutex<ExactModelHostConnector>,
    cancel: Arc<AtomicBool>,
    running: Mutex<Option<String>>,
    staged_lengths: Mutex<HashMap<String, usize>>,
}

fn registry() -> &'static Mutex<HashMap<u64, Arc<ModelHost>>> {
    static REGISTRY: OnceLock<Mutex<HashMap<u64, Arc<ModelHost>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

static NEXT_HANDLE: AtomicU64 = AtomicU64::new(1);

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    // A panicking holder cannot leave these maps logically inconsistent.
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Encode a response frame.
#[must_use]
pub fn encode_frame(header: &Value, segments: &[(&str, &[u8])]) -> Vec<u8> {
    let mut header = header.clone();
    let mut offsets = Map::new();
    let mut payload = Vec::new();
    for (name, bytes) in segments {
        offsets.insert((*name).to_string(), json!([payload.len(), bytes.len()]));
        payload.extend_from_slice(bytes);
    }
    if !segments.is_empty() {
        header["segments"] = Value::Object(offsets);
    }
    let header_bytes = header.to_string().into_bytes();
    let length = u32::try_from(header_bytes.len()).unwrap_or(u32::MAX);
    let mut frame = Vec::with_capacity(4 + header_bytes.len() + payload.len());
    frame.extend_from_slice(&length.to_le_bytes());
    frame.extend_from_slice(&header_bytes);
    frame.extend_from_slice(&payload);
    frame
}

struct Request<'a> {
    header: Map<String, Value>,
    payload: &'a [u8],
}

impl<'a> Request<'a> {
    fn parse(frame: &'a [u8]) -> Result<Self, EnvelopeError> {
        let invalid = EnvelopeError::InvalidInput("model_call_invalid_frame");
        let length_bytes: [u8; 4] = frame
            .get(..4)
            .and_then(|bytes| bytes.try_into().ok())
            .ok_or(invalid.clone())?;
        let header_len = u32::from_le_bytes(length_bytes) as usize;
        let header_bytes = frame.get(4..4 + header_len).ok_or(invalid.clone())?;
        let Ok(Value::Object(header)) = serde_json::from_slice(header_bytes) else {
            return Err(invalid);
        };
        Ok(Self {
            header,
            payload: &frame[4 + header_len..],
        })
    }

    fn str(&self, name: &'static str) -> Result<&str, EnvelopeError> {
        self.header
            .get(name)
            .and_then(Value::as_str)
            .ok_or(EnvelopeError::InvalidInput(name))
    }

    fn u64(&self, name: &'static str) -> Result<u64, EnvelopeError> {
        self.header
            .get(name)
            .and_then(Value::as_u64)
            .ok_or(EnvelopeError::InvalidInput(name))
    }

    fn segment(&self, name: &'static str) -> Result<&'a [u8], EnvelopeError> {
        let range = self
            .header
            .get("segments")
            .and_then(|segments| segments.get(name))
            .and_then(Value::as_array)
            .filter(|pair| pair.len() == 2)
            .and_then(|pair| Some((pair[0].as_u64()?, pair[1].as_u64()?)))
            .ok_or(EnvelopeError::InvalidInput(name))?;
        let start = usize::try_from(range.0).map_err(|_| EnvelopeError::InvalidInput(name))?;
        let length = usize::try_from(range.1).map_err(|_| EnvelopeError::InvalidInput(name))?;
        self.payload
            .get(start..start.saturating_add(length))
            .ok_or(EnvelopeError::InvalidInput(name))
    }
}

fn error_response(error: &HostConnectorError) -> Vec<u8> {
    encode_frame(
        &json!({
            "ok": false,
            "error": {
                "code": error.code.as_str(),
                "reason": error.reason.map(ModelFailureReason::as_str),
                "message": error.message,
            }
        }),
        &[],
    )
}

fn ok(header: Value) -> Vec<u8> {
    let mut header = header;
    header["ok"] = Value::Bool(true);
    encode_frame(&header, &[])
}

/// Dispatch one framed model call. `handle` is `0` only for `create`.
///
/// # Errors
///
/// Returns [`EnvelopeError`] for malformed frames/headers/segments, unknown
/// operations, or an unknown handle. Model failures are encoded in the
/// returned response frame instead.
pub fn model_call(handle: u64, frame: &[u8]) -> Result<Vec<u8>, EnvelopeError> {
    let request = Request::parse(frame)?;
    let op = request.str("op")?;
    if op == "create" {
        if handle != 0 {
            return Err(EnvelopeError::InvalidInput("create_requires_handle_zero"));
        }
        return create(&request);
    }
    let host = lock(registry())
        .get(&handle)
        .cloned()
        .ok_or(EnvelopeError::InvalidHandle)?;
    match op {
        "register" => register(&host, &request),
        "stage_input" => stage_input(&host, &request),
        "execute" => execute(&host, &request),
        "read_output" => read_output(&host, &request),
        "rights" => rights(&host, &request),
        "cancel" => Ok(cancel(&host, &request)),
        "drop_ref" => drop_ref(&host, &request),
        "destroy" => {
            lock(registry()).remove(&handle);
            Ok(ok(json!({})))
        }
        _ => Err(EnvelopeError::InvalidInput("model_call_unknown_op")),
    }
}

fn create(request: &Request<'_>) -> Result<Vec<u8>, EnvelopeError> {
    let pins: Vec<ExactModelPin> = request
        .header
        .get("pins")
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
        .ok_or(EnvelopeError::InvalidInput("pins"))?;
    let keys = request
        .header
        .get("trusted_public_keys_hex")
        .and_then(Value::as_array)
        .ok_or(EnvelopeError::InvalidInput("trusted_public_keys_hex"))?;
    let mut trusted = TrustedModelKeys::new();
    for key in keys {
        let bytes: [u8; 32] = key
            .as_str()
            .and_then(hex_decode)
            .and_then(|bytes| bytes.try_into().ok())
            .ok_or(EnvelopeError::InvalidInput("trusted_public_keys_hex"))?;
        if let Err(error) = trusted.trust(&bytes) {
            return Ok(error_response(&error));
        }
    }
    let limits = request
        .header
        .get("limits")
        .ok_or(EnvelopeError::InvalidInput("limits"))?;
    let limit = |name: &'static str| {
        limits
            .get(name)
            .and_then(Value::as_u64)
            .filter(|value| *value > 0)
            .ok_or(EnvelopeError::InvalidInput(name))
    };
    let host_limits = HostModelLimits {
        max_package_bytes: limit("max_package_bytes")?,
        max_memory_bytes: limit("max_memory_bytes")?,
        max_fuel: limit("max_fuel")?,
    };
    let mut connector = ExactModelHostConnector::new(pins, trusted);
    connector.engine = ModelEngine::Wasmi;
    connector.host_limits = host_limits;
    let cancel = Arc::clone(&connector.cancel);
    let id = NEXT_HANDLE.fetch_add(1, Ordering::SeqCst);
    lock(registry()).insert(
        id,
        Arc::new(ModelHost {
            connector: Mutex::new(connector),
            cancel,
            running: Mutex::new(None),
            staged_lengths: Mutex::new(HashMap::new()),
        }),
    );
    Ok(ok(json!({ "handle": id })))
}

fn register(host: &ModelHost, request: &Request<'_>) -> Result<Vec<u8>, EnvelopeError> {
    let manifest = request.segment("manifest")?;
    let wasm = request.segment("wasm")?;
    let signature = request.segment("signature")?;
    Ok(
        match lock(&host.connector).register_package(manifest, wasm.to_vec(), signature) {
            Ok(digest) => ok(json!({ "digest": digest })),
            Err(error) => error_response(&error),
        },
    )
}

fn stage_input(host: &ModelHost, request: &Request<'_>) -> Result<Vec<u8>, EnvelopeError> {
    let bytes = request.segment("input")?;
    let max = usize::try_from(request.u64("max_bytes")?)
        .map_err(|_| EnvelopeError::InvalidInput("max_bytes"))?;
    Ok(
        match lock(&host.connector).io.stage_model_input(bytes, max) {
            Ok(input_ref) => {
                lock(&host.staged_lengths).insert(input_ref.clone(), bytes.len());
                ok(json!({ "input_ref": input_ref }))
            }
            Err(error) => error_response(&error),
        },
    )
}

fn execute(host: &ModelHost, request: &Request<'_>) -> Result<Vec<u8>, EnvelopeError> {
    let execution_id = request.str("execution_id")?.to_string();
    let payload = request
        .header
        .get("payload")
        .cloned()
        .ok_or(EnvelopeError::InvalidInput("payload"))?;
    let policy_ref = payload
        .get("policy_ref")
        .and_then(Value::as_str)
        .ok_or(EnvelopeError::InvalidInput("policy_ref"))?
        .to_string();
    let allowed: Vec<String> = request
        .header
        .get("allowed_classifications")
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
        .ok_or(EnvelopeError::InvalidInput("allowed_classifications"))?;
    let max_output = payload
        .get("max_output_bytes")
        .and_then(Value::as_u64)
        .ok_or(EnvelopeError::InvalidInput("max_output_bytes"))?;
    let input_ref = payload
        .get("input_ref")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let input_bytes = lock(&host.staged_lengths).remove(&input_ref).unwrap_or(0);

    let mut connector = lock(&host.connector);
    connector.policies.insert(
        policy_ref.clone(),
        ExecutionPolicy {
            policy_ref,
            allowed_classifications: allowed,
            max_output_bytes: max_output,
        },
    );
    host.cancel.store(false, Ordering::SeqCst);
    *lock(&host.running) = Some(execution_id);
    let started = Instant::now();
    let result = connector.invoke(&HostConnectorHostRequest {
        connector_id: MODEL_RUNTIME_CONNECTOR.to_string(),
        operation: MODEL_EXECUTE_OPERATION.to_string(),
        binding_id: "swift-exact-model-host".to_string(),
        target_family: "apple".to_string(),
        correlation_id: request.str("execution_id")?.to_string(),
        payload: payload.clone(),
        cancel_requested: false,
    });
    let duration_ms = started.elapsed().as_secs_f64() * 1000.0;
    *lock(&host.running) = None;
    host.cancel.store(false, Ordering::SeqCst);
    let result = match result {
        Ok(result) => result,
        Err(error) => return Ok(error_response(&error)),
    };
    let output_ref = result.artifact_ref.unwrap_or_default();
    let output_bytes = connector
        .io
        .read_model_output(&output_ref, usize::MAX)
        .map(|bytes| bytes.len())
        .unwrap_or(0);
    let model_ref = payload.get("model_ref").cloned().unwrap_or(Value::Null);
    Ok(ok(json!({
        "output_ref": output_ref,
        "placement": PLACEMENT_WASM_CPU,
        "target": PLACEMENT_WASM_CPU,
        "model_ref": model_ref,
        "trace": {
            "model_ref": model_ref,
            "placement": PLACEMENT_WASM_CPU,
            "data_classification": payload.get("data_classification").cloned().unwrap_or(Value::Null),
            "usage": {
                "input_bytes": input_bytes,
                "output_bytes": output_bytes,
                "duration_ms": duration_ms,
            }
        }
    })))
}

fn read_output(host: &ModelHost, request: &Request<'_>) -> Result<Vec<u8>, EnvelopeError> {
    let output_ref = request.str("output_ref")?;
    let max = usize::try_from(request.u64("max_bytes")?)
        .map_err(|_| EnvelopeError::InvalidInput("max_bytes"))?;
    Ok(
        match lock(&host.connector).io.read_model_output(output_ref, max) {
            Ok(bytes) => encode_frame(&json!({ "ok": true }), &[("output", &bytes)]),
            Err(error) => error_response(&error),
        },
    )
}

fn rights(host: &ModelHost, request: &Request<'_>) -> Result<Vec<u8>, EnvelopeError> {
    let digest = request.str("digest")?;
    let rights = lock(&host.connector)
        .model_rights(digest)
        .map_or(Value::Null, |rights| {
            serde_json::to_value(rights).unwrap_or(Value::Null)
        });
    Ok(ok(json!({ "rights": rights })))
}

/// Flip the shared cancel flag only when the named execution is running, so
/// a late cancel can never interrupt the next execution.
fn cancel(host: &ModelHost, request: &Request<'_>) -> Vec<u8> {
    let target = request.str("execution_id").unwrap_or_default();
    let running = lock(&host.running);
    let matched = running.as_deref() == Some(target);
    if matched {
        host.cancel.store(true, Ordering::SeqCst);
    }
    ok(json!({ "cancelled": matched }))
}

fn drop_ref(host: &ModelHost, request: &Request<'_>) -> Result<Vec<u8>, EnvelopeError> {
    let reference = request.str("ref")?;
    lock(&host.connector).io.drop_ref(reference);
    lock(&host.staged_lengths).remove(reference);
    Ok(ok(json!({})))
}

fn hex_decode(value: &str) -> Option<Vec<u8>> {
    if !value.len().is_multiple_of(2) {
        return None;
    }
    (0..value.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(value.get(index..index + 2)?, 16).ok())
        .collect()
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::needless_pass_by_value,
    clippy::cast_possible_truncation,
    clippy::unreadable_literal,
    clippy::too_many_lines
)]
mod tests {
    use super::*;
    use traverse_runtime::exact_model::{digest_hex, encode_guest_frame, sign_model_manifest};

    const MODELS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/models");

    fn read(path: &str) -> Vec<u8> {
        std::fs::read(format!("{MODELS}/{path}")).expect(path)
    }

    fn key(field: &str) -> String {
        let key: Value = serde_json::from_slice(&read("test-signing-key.json")).expect("key");
        key[field].as_str().expect("hex").to_string()
    }

    fn frame(header: &Value, segments: &[(&str, &[u8])]) -> Vec<u8> {
        encode_frame(header, segments)
    }

    fn decode(response: &[u8]) -> (Value, Vec<u8>) {
        let request = Request::parse(response).expect("response frame");
        let payload = request.payload.to_vec();
        (Value::Object(request.header), payload)
    }

    fn call(handle: u64, header: &Value, segments: &[(&str, &[u8])]) -> Value {
        decode(&model_call(handle, &frame(header, segments)).expect("envelope ok")).0
    }

    fn digits_pin() -> Value {
        let vector: Value =
            serde_json::from_slice(&read("conformance/signed-digits-mlp.json")).expect("vector");
        vector["pin"].clone()
    }

    fn limits() -> Value {
        json!({ "max_package_bytes": 1_000_000, "max_memory_bytes": 1_048_576, "max_fuel": 10_000_000_000_u64 })
    }

    fn create(pins: Value) -> u64 {
        let response = call(
            0,
            &json!({ "op": "create", "pins": pins, "trusted_public_keys_hex": [key("public_key_hex")], "limits": limits() }),
            &[],
        );
        assert_eq!(response["ok"], json!(true), "{response}");
        response["handle"].as_u64().expect("handle")
    }

    fn register_digits(handle: u64) -> Value {
        call(
            handle,
            &json!({ "op": "register" }),
            &[
                ("manifest", &read("digits-mlp-1.0.0/model.manifest.json")),
                ("wasm", &read("digits-mlp-1.0.0/model.wasm")),
                ("signature", &read("digits-mlp-1.0.0/model.sig.json")),
            ],
        )
    }

    fn execute_header(pin: &Value, input_ref: &str, execution_id: &str) -> Value {
        json!({
            "op": "execute",
            "execution_id": execution_id,
            "allowed_classifications": ["sensitive"],
            "payload": {
                "model_ref": { "model_id": pin["model_id"], "version": pin["version"], "digest": pin["digest"] },
                "input_ref": input_ref,
                "policy_ref": "policy-1",
                "data_classification": "sensitive",
                "input_schema_ref": "schema:traverse-digits-mlp-in",
                "input_schema_version": "1.0.0",
                "max_output_bytes": 64
            }
        })
    }

    fn stage(handle: u64, bytes: &[u8]) -> String {
        let response = call(
            handle,
            &json!({ "op": "stage_input", "max_bytes": 4096 }),
            &[("input", bytes)],
        );
        response["input_ref"]
            .as_str()
            .expect("input_ref")
            .to_string()
    }

    #[test]
    fn digits_package_round_trips_through_the_framed_model_call() {
        let pin = digits_pin();
        let handle = create(json!([pin.clone()]));
        let registered = register_digits(handle);
        assert_eq!(registered["digest"], pin["digest"]);

        let vector: Value =
            serde_json::from_slice(&read("conformance/signed-digits-mlp.json")).expect("vector");
        for case in vector["cases"].as_array().expect("cases") {
            let input = hex_decode(case["input_frame_hex"].as_str().expect("in")).expect("hex");
            let input_ref = stage(handle, &input);
            let executed = call(handle, &execute_header(&pin, &input_ref, "exec-1"), &[]);
            assert_eq!(executed["ok"], json!(true), "{executed}");
            assert_eq!(executed["placement"], json!("wasm-cpu"));
            assert_eq!(
                executed["trace"]["usage"]["input_bytes"],
                json!(input.len())
            );
            assert_eq!(executed["trace"]["usage"]["output_bytes"], json!(56));
            let (header, payload) = decode(
                &model_call(
                    handle,
                    &frame(
                        &json!({ "op": "read_output", "output_ref": executed["output_ref"], "max_bytes": 64 }),
                        &[],
                    ),
                )
                .expect("read"),
            );
            assert_eq!(header["ok"], json!(true));
            let range = header["segments"]["output"]
                .as_array()
                .expect("segment")
                .clone();
            let (start, len) = (
                range[0].as_u64().unwrap() as usize,
                range[1].as_u64().unwrap() as usize,
            );
            assert_eq!(
                digest_hex(&payload[start..start + len]),
                digest_hex(
                    &hex_decode(case["output_frame_hex"].as_str().expect("out")).expect("hex")
                )
            );
        }

        let rights = call(
            handle,
            &json!({ "op": "rights", "digest": pin["digest"] }),
            &[],
        );
        assert_eq!(rights["rights"]["license_id"], json!("CC-BY-4.0"));
        let missing = call(handle, &json!({ "op": "rights", "digest": "00" }), &[]);
        assert_eq!(missing["rights"], Value::Null);

        let input_ref = stage(handle, b"x");
        assert_eq!(
            call(handle, &json!({ "op": "drop_ref", "ref": input_ref }), &[])["ok"],
            json!(true)
        );
        let gone = call(handle, &execute_header(&pin, &input_ref, "exec-2"), &[]);
        assert_eq!(gone["ok"], json!(false));
        assert_eq!(gone["error"]["code"], json!("invalid_input"));

        assert_eq!(
            call(handle, &json!({ "op": "destroy" }), &[])["ok"],
            json!(true)
        );
        assert_eq!(
            model_call(
                handle,
                &frame(&json!({ "op": "rights", "digest": "x" }), &[])
            ),
            Err(EnvelopeError::InvalidHandle)
        );
    }

    #[test]
    fn model_failures_are_returned_as_data_with_stable_reasons() {
        let pin = digits_pin();
        let handle = create(json!([pin.clone()]));
        let tampered = call(
            handle,
            &json!({ "op": "register" }),
            &[
                ("manifest", b"{}"),
                ("wasm", &read("digits-mlp-1.0.0/model.wasm")),
                ("signature", &read("digits-mlp-1.0.0/model.sig.json")),
            ],
        );
        assert_eq!(tampered["ok"], json!(false));
        assert_eq!(tampered["error"]["reason"], json!("signature_invalid"));
        let over = call(
            handle,
            &json!({ "op": "stage_input", "max_bytes": 1 }),
            &[("input", b"ab")],
        );
        assert_eq!(over["error"]["code"], json!("input_limit_exceeded"));
        assert_eq!(over["error"]["reason"], Value::Null);
        let read_missing = call(
            handle,
            &json!({ "op": "read_output", "output_ref": "nope", "max_bytes": 8 }),
            &[],
        );
        assert_eq!(read_missing["error"]["code"], json!("unavailable"));

        // Host ceilings reject at registration.
        let tight = call(
            0,
            &json!({ "op": "create", "pins": [pin], "trusted_public_keys_hex": [key("public_key_hex")],
                     "limits": { "max_package_bytes": 100, "max_memory_bytes": 1_048_576, "max_fuel": 1_000_000 } }),
            &[],
        );
        let tight_handle = tight["handle"].as_u64().expect("handle");
        let rejected = register_digits(tight_handle);
        assert_eq!(rejected["error"]["reason"], json!("host_limit_exceeded"));

        // A key that isn't a curve point is a model-level error at create.
        let bad_point = (0_u8..=255)
            .map(|byte| format!("{byte:02x}").repeat(32))
            .find(|hex| {
                let bytes: [u8; 32] = hex_decode(hex).unwrap().try_into().unwrap();
                TrustedModelKeys::new().trust(&bytes).is_err()
            })
            .expect("some constant byte string is not a curve point");
        let created = call(
            0,
            &json!({ "op": "create", "pins": [], "trusted_public_keys_hex": [bad_point], "limits": limits() }),
            &[],
        );
        assert_eq!(created["error"]["reason"], json!("key_untrusted"));
    }

    #[test]
    fn envelope_errors_are_rejected_before_any_model_work() {
        let bad = |bytes: &[u8]| model_call(0, bytes).expect_err("envelope");
        assert!(matches!(bad(b"\x01"), EnvelopeError::InvalidInput(_)));
        assert!(matches!(
            bad(&[9, 0, 0, 0, b'{']),
            EnvelopeError::InvalidInput(_)
        ));
        assert!(matches!(
            bad(&frame(&json!([1]), &[])[..]),
            EnvelopeError::InvalidInput(_)
        ));
        let mut not_object = 2_u32.to_le_bytes().to_vec();
        not_object.extend_from_slice(b"[]");
        assert!(matches!(bad(&not_object), EnvelopeError::InvalidInput(_)));
        assert_eq!(
            bad(&frame(&json!({}), &[])),
            EnvelopeError::InvalidInput("op")
        );
        assert_eq!(
            model_call(7, &frame(&json!({ "op": "create" }), &[])),
            Err(EnvelopeError::InvalidInput("create_requires_handle_zero"))
        );
        for (header, field) in [
            (json!({ "op": "create" }), "pins"),
            (
                json!({ "op": "create", "pins": [] }),
                "trusted_public_keys_hex",
            ),
            (
                json!({ "op": "create", "pins": [], "trusted_public_keys_hex": ["zz"] }),
                "trusted_public_keys_hex",
            ),
            (
                json!({ "op": "create", "pins": [], "trusted_public_keys_hex": [] }),
                "limits",
            ),
            (
                json!({ "op": "create", "pins": [], "trusted_public_keys_hex": [], "limits": { "max_package_bytes": 0 } }),
                "max_package_bytes",
            ),
        ] {
            assert_eq!(
                bad(&frame(&header, &[])),
                EnvelopeError::InvalidInput(field)
            );
        }
        assert_eq!(
            model_call(
                999_999,
                &frame(&json!({ "op": "rights", "digest": "x" }), &[])
            ),
            Err(EnvelopeError::InvalidHandle)
        );

        let handle = create(json!([]));
        let env = |header: Value, segments: &[(&str, &[u8])]| {
            model_call(handle, &frame(&header, segments)).expect_err("envelope")
        };
        assert_eq!(
            env(json!({ "op": "nope" }), &[]),
            EnvelopeError::InvalidInput("model_call_unknown_op")
        );
        assert_eq!(
            env(json!({ "op": "register" }), &[]),
            EnvelopeError::InvalidInput("manifest")
        );
        let mut out_of_range = json!({ "op": "stage_input", "max_bytes": 4 });
        out_of_range["segments"] = json!({ "input": [0, 99] });
        assert_eq!(env(out_of_range, &[]), EnvelopeError::InvalidInput("input"));
        assert_eq!(
            env(json!({ "op": "stage_input" }), &[("input", b"a")]),
            EnvelopeError::InvalidInput("max_bytes")
        );
        assert_eq!(
            env(json!({ "op": "execute" }), &[]),
            EnvelopeError::InvalidInput("execution_id")
        );
        assert_eq!(
            env(json!({ "op": "execute", "execution_id": "e" }), &[]),
            EnvelopeError::InvalidInput("payload")
        );
        assert_eq!(
            env(
                json!({ "op": "execute", "execution_id": "e", "payload": {} }),
                &[]
            ),
            EnvelopeError::InvalidInput("policy_ref")
        );
        assert_eq!(
            env(
                json!({ "op": "execute", "execution_id": "e", "payload": { "policy_ref": "p" } }),
                &[]
            ),
            EnvelopeError::InvalidInput("allowed_classifications")
        );
        assert_eq!(
            env(
                json!({ "op": "execute", "execution_id": "e", "allowed_classifications": [], "payload": { "policy_ref": "p" } }),
                &[]
            ),
            EnvelopeError::InvalidInput("max_output_bytes")
        );
        assert_eq!(
            env(json!({ "op": "read_output" }), &[]),
            EnvelopeError::InvalidInput("output_ref")
        );
        assert_eq!(
            env(json!({ "op": "rights" }), &[]),
            EnvelopeError::InvalidInput("digest")
        );
        assert_eq!(
            env(json!({ "op": "drop_ref" }), &[]),
            EnvelopeError::InvalidInput("ref")
        );
        assert_eq!(hex_decode("abc"), None);
        assert_eq!(hex_decode("zz"), None);
    }

    #[test]
    fn cancel_interrupts_the_matching_execution_mid_run_only() {
        // Signed looping guest: runs until fuel, cancellation, or deadline.
        let wasm = wat::parse_str(
            r#"(module (memory (export "memory") 1)
               (func (export "model_execute") (param i32 i32 i32 i32) (result i32)
                 (loop $spin (br $spin)) i32.const 0))"#,
        )
        .expect("wat");
        let manifest = serde_json::to_vec(&json!({
            "schema_version": "2.0.0", "model_id": "test.looper", "version": "1.0.0",
            "wasm_digest": digest_hex(&wasm), "registry_ref": "registry:test.looper@1.0.0",
            "executable_format": "traverse-model-wasm", "abi_version": 1,
            "input_schema_ref": "schema:traverse-digits-mlp-in", "input_schema_version": "1.0.0",
            "output_schema_ref": "schema:x", "output_schema_version": "1.0.0",
            "rights": { "license_id": "Apache-2.0", "attribution": "test", "redistribution": "test",
                        "commercial_use": "allowed", "source_url": "https://example.invalid" },
            "supported_profiles": ["wasm-cpu"], "max_memory_bytes": 131072,
            "max_fuel": 9_000_000_000_u64, "max_input_bytes": 4096, "max_output_bytes": 64,
            "max_execution_ms": 60_000, "offline_allowed": true
        }))
        .expect("manifest");
        let secret: [u8; 32] = hex_decode(&key("secret_key_hex"))
            .unwrap()
            .try_into()
            .unwrap();
        let signature = serde_json::to_vec(&sign_model_manifest(&secret, &manifest)).expect("sig");
        let pin = json!({
            "model_id": "test.looper", "version": "1.0.0", "digest": digest_hex(&manifest),
            "offline_allowed": true, "target": "wasm-cpu",
            "rights": { "license_id": "Apache-2.0", "commercial_use": "allowed" }
        });
        let handle = create(json!([pin.clone()]));
        let registered = call(
            handle,
            &json!({ "op": "register" }),
            &[
                ("manifest", &manifest),
                ("wasm", &wasm),
                ("signature", &signature),
            ],
        );
        assert_eq!(registered["ok"], json!(true), "{registered}");

        // A cancel for an execution that isn't running is a no-op.
        let stale = call(
            handle,
            &json!({ "op": "cancel", "execution_id": "old" }),
            &[],
        );
        assert_eq!(stale["cancelled"], json!(false));

        let input_ref = stage(handle, &encode_guest_frame(2, &[1], &[0; 4]));
        let header = execute_header(&pin, &input_ref, "exec-long");
        let worker = std::thread::spawn(move || call(handle, &header, &[]));
        let mut cancelled = false;
        for _ in 0..500 {
            std::thread::sleep(std::time::Duration::from_millis(10));
            let wrong = call(
                handle,
                &json!({ "op": "cancel", "execution_id": "other" }),
                &[],
            );
            assert_eq!(wrong["cancelled"], json!(false));
            if call(
                handle,
                &json!({ "op": "cancel", "execution_id": "exec-long" }),
                &[],
            )["cancelled"]
                == json!(true)
            {
                cancelled = true;
                break;
            }
        }
        assert!(cancelled, "execution never became cancellable");
        let result = worker.join().expect("worker");
        assert_eq!(result["ok"], json!(false));
        assert_eq!(result["error"]["code"], json!("cancelled"));
        // The flag is cleared, so the next execution is not pre-cancelled.
        let input_ref = stage(handle, &encode_guest_frame(2, &[1], &[0; 4]));
        let mut timed = execute_header(&pin, &input_ref, "exec-timeout");
        timed["payload"]["timeout_ms"] = json!(0);
        assert_eq!(call(handle, &timed, &[])["error"]["code"], json!("timeout"));
    }
    #[test]
    fn guest_abi_v2_package_round_trips_through_the_swift_model_call() {
        let manifest = read("fixture-echo-v2-1.0.0/model.manifest.json");
        let pin = json!({
            "model_id": "fixture.echo-v2", "version": "1.0.0", "digest": digest_hex(&manifest),
            "offline_allowed": true, "target": "wasm-cpu",
            "rights": { "license_id": "Apache-2.0", "commercial_use": "allowed" }
        });
        let handle = create(json!([pin.clone()]));
        let registered = call(
            handle,
            &json!({ "op": "register" }),
            &[
                ("manifest", &manifest),
                ("wasm", &read("fixture-echo-v2-1.0.0/model.wasm")),
                ("signature", &read("fixture-echo-v2-1.0.0/model.sig.json")),
            ],
        );
        assert_eq!(registered["ok"], json!(true), "{registered}");
        let input = vec![5_u8; 3000];
        let input_ref = stage(handle, &input);
        let mut header = execute_header(&pin, &input_ref, "exec-v2");
        header["payload"]["input_schema_ref"] = json!("schema:fixture-in");
        header["payload"]["max_output_bytes"] = json!(4096);
        let executed = call(handle, &header, &[]);
        assert_eq!(executed["ok"], json!(true), "{executed}");
        let (response, payload) = decode(
            &model_call(
                handle,
                &frame(&json!({ "op": "read_output", "output_ref": executed["output_ref"], "max_bytes": 4096 }), &[]),
            )
            .expect("read"),
        );
        let range = response["segments"]["output"]
            .as_array()
            .expect("segment")
            .clone();
        let start = range[0].as_u64().unwrap() as usize;
        let len = range[1].as_u64().unwrap() as usize;
        assert_eq!(&payload[start..start + len], input.as_slice());
    }
}
