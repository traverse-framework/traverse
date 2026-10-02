//! Android JNI shim for the framed Spec 138 exact-ref model host
//! (Decision 108, ADR-0079).
//!
//! One exported native method,
//! `dev.traverse.embedder.ExactModelNative.modelCall(long, byte[]): byte[]`,
//! forwards a request frame to [`traverse_model_host_frame::model_call`] with
//! the [`ANDROID`](traverse_model_host_frame::ANDROID) profile and returns
//! the response frame. Envelope failures (unknown handle, malformed frame)
//! are returned as `{"ok":false,"error":{...}}` frames rather than Java
//! exceptions, so Kotlin sees one response shape. All logic is in the safe
//! [`respond`]; the exported function only converts bytes, and `jni`'s
//! `with_env` keeps panics from unwinding across the FFI boundary.
#![allow(unsafe_code)] // Audited JNI exception; see ADR-0079 and Decision 108.

use jni::EnvUnowned;
use jni::errors::ThrowRuntimeExAndDefault;
use jni::objects::{JByteArray, JClass};
use serde_json::json;
use traverse_model_host_frame::{ANDROID, EnvelopeError, encode_frame, model_call};

/// Response frame for one request frame (safe; unit tested without a JVM).
#[must_use]
pub fn respond(handle: i64, request: &[u8]) -> Vec<u8> {
    let Ok(handle) = u64::try_from(handle) else {
        return envelope_error("invalid_input", "handle");
    };
    match model_call(&ANDROID, handle, request) {
        Ok(response) => response,
        Err(EnvelopeError::InvalidHandle) => envelope_error("invalid_handle", "handle"),
        Err(EnvelopeError::InvalidInput(what)) => envelope_error("invalid_input", what),
    }
}

fn envelope_error(code: &str, what: &str) -> Vec<u8> {
    encode_frame(
        &json!({
            "ok": false,
            "error": { "code": code, "reason": null, "message": format!("model call envelope rejected: {what}") }
        }),
        &[],
    )
}

/// `ExactModelNative.modelCall(handle, request)`. A JNI failure (it cannot
/// read the request or allocate the response) raises a Java
/// `RuntimeException` instead of returning a frame.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_traverse_embedder_ExactModelNative_modelCall<'caller>(
    mut unowned_env: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    handle: i64,
    request: JByteArray<'caller>,
) -> JByteArray<'caller> {
    unowned_env
        .with_env(|env| -> jni::errors::Result<_> {
            let bytes = env.convert_byte_array(&request)?;
            env.byte_array_from_slice(&respond(handle, &bytes))
        })
        .resolve::<ThrowRuntimeExAndDefault>()
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn header(frame: &[u8]) -> Value {
        let length = u32::from_le_bytes(frame[..4].try_into().expect("4 bytes")) as usize;
        serde_json::from_slice(&frame[4..4 + length]).expect("json header")
    }

    fn request(value: &Value) -> Vec<u8> {
        encode_frame(value, &[])
    }

    #[test]
    fn envelope_failures_come_back_as_error_frames() {
        let negative = header(&respond(-1, &request(&json!({ "op": "rights" }))));
        assert_eq!(negative["ok"], json!(false));
        assert_eq!(negative["error"]["code"], json!("invalid_input"));

        let unknown = header(&respond(
            424_242,
            &request(&json!({ "op": "rights", "digest": "x" })),
        ));
        assert_eq!(unknown["error"]["code"], json!("invalid_handle"));

        let malformed = header(&respond(0, b"\x01"));
        assert_eq!(malformed["error"]["code"], json!("invalid_input"));
    }

    #[test]
    fn creates_an_android_host_and_returns_model_failures_as_data() {
        let created = header(&respond(
            0,
            &request(&json!({
                "op": "create", "pins": [], "trusted_public_keys_hex": [], "model_usage": "commercial",
                "limits": { "max_package_bytes": 1, "max_memory_bytes": 1, "max_fuel": 1 }
            })),
        ));
        assert_eq!(created["ok"], json!(true));
        let handle = i64::try_from(created["handle"].as_u64().expect("handle")).expect("i64");
        let rights = header(&respond(
            handle,
            &request(&json!({ "op": "rights", "digest": "00" })),
        ));
        assert_eq!(rights, json!({ "ok": true, "rights": null }));
        let destroyed = header(&respond(handle, &request(&json!({ "op": "destroy" }))));
        assert_eq!(destroyed["ok"], json!(true));
    }
}
