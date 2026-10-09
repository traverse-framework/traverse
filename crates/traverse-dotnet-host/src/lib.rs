//! .NET P/Invoke shim for the framed Spec 138 exact-ref model host
//! (Decision 111, ADR-0081).
//!
//! Two exported C functions:
//! - [`traverse_dotnet_host_model_call`] forwards one request frame to
//!   [`traverse_model_host_frame::model_call`] with the
//!   [`DOTNET`](traverse_model_host_frame::DOTNET) profile and returns a
//!   library-owned response frame;
//! - [`traverse_dotnet_host_free`] releases that frame.
//!
//! Envelope failures (unknown handle, malformed frame) and panics come back
//! as `{"ok":false,"error":{...}}` frames, so .NET sees one response shape and
//! nothing unwinds across the boundary. All logic is in the safe [`respond`];
//! the exported functions only move bytes.
#![allow(unsafe_code)] // Audited P/Invoke exception; see ADR-0081 and Decision 111.

use serde_json::json;
use traverse_model_host_frame::{DOTNET, EnvelopeError, encode_frame, model_call};

/// Response frame for one request frame (safe; unit tested without .NET).
#[must_use]
pub fn respond(handle: u64, request: &[u8]) -> Vec<u8> {
    match std::panic::catch_unwind(|| model_call(&DOTNET, handle, request)) {
        Ok(Ok(response)) => response,
        Ok(Err(EnvelopeError::InvalidHandle)) => envelope_error("invalid_handle", "handle"),
        Ok(Err(EnvelopeError::InvalidInput(what))) => envelope_error("invalid_input", what),
        Err(_) => envelope_error("unavailable", "model host panicked"),
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

/// Runs one framed model call. Returns a library-owned response frame and
/// writes its length to `response_length_out`; the caller must release it
/// exactly once with [`traverse_dotnet_host_free`]. Returns null only when
/// `response_length_out` is null. A null `request` yields an envelope error
/// frame.
///
/// # Safety
///
/// `request` must be null or valid for reads of `request_length` bytes, and
/// `response_length_out` must be null or point to one writable `usize`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn traverse_dotnet_host_model_call(
    handle: u64,
    request: *const u8,
    request_length: usize,
    response_length_out: *mut usize,
) -> *mut u8 {
    if response_length_out.is_null() {
        return std::ptr::null_mut();
    }
    let response = if request.is_null() {
        envelope_error("invalid_input", "request")
    } else {
        // SAFETY: `request` is non-null and the caller supplies a readable range of the stated length.
        respond(handle, unsafe {
            std::slice::from_raw_parts(request, request_length)
        })
    };
    let response = response.into_boxed_slice();
    // SAFETY: `response_length_out` is non-null and points to one writable `usize`.
    unsafe { response_length_out.write(response.len()) };
    Box::into_raw(response).cast::<u8>()
}

/// Releases a response frame returned by [`traverse_dotnet_host_model_call`].
/// Null is a no-op.
///
/// # Safety
///
/// `response` must be null, or a pointer returned by
/// [`traverse_dotnet_host_model_call`] together with the length it reported,
/// not yet freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn traverse_dotnet_host_free(response: *mut u8, length: usize) {
    if response.is_null() {
        return;
    }
    // SAFETY: the pointer and length come from one `Box<[u8]>` leaked by
    // `traverse_dotnet_host_model_call`, freed exactly once by contract.
    drop(unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(response, length)) });
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

    /// Calls the exported functions exactly as .NET does.
    fn call(handle: u64, frame: &[u8]) -> Value {
        let mut length = 0_usize;
        // SAFETY: `frame` is a live slice and `length` a live `usize`.
        let response = unsafe {
            traverse_dotnet_host_model_call(handle, frame.as_ptr(), frame.len(), &raw mut length)
        };
        assert!(!response.is_null());
        // SAFETY: the shim returned `length` readable bytes at `response`.
        let bytes = unsafe { std::slice::from_raw_parts(response, length) }.to_vec();
        // SAFETY: freed exactly once with the reported length.
        unsafe { traverse_dotnet_host_free(response, length) };
        header(&bytes)
    }

    #[test]
    fn envelope_failures_come_back_as_error_frames() {
        let unknown = call(424_242, &request(&json!({ "op": "rights", "digest": "x" })));
        assert_eq!(unknown["ok"], json!(false));
        assert_eq!(unknown["error"]["code"], json!("invalid_handle"));
        assert_eq!(call(0, b"\x01")["error"]["code"], json!("invalid_input"));

        let mut length = 7_usize;
        // SAFETY: a null request is checked by the shim; `length` is live.
        let response =
            unsafe { traverse_dotnet_host_model_call(0, std::ptr::null(), 0, &raw mut length) };
        // SAFETY: the shim returned `length` readable bytes at `response`.
        let bytes = unsafe { std::slice::from_raw_parts(response, length) }.to_vec();
        // SAFETY: freed exactly once with the reported length.
        unsafe { traverse_dotnet_host_free(response, length) };
        assert_eq!(header(&bytes)["error"]["code"], json!("invalid_input"));

        // SAFETY: a null length pointer is checked and returns null; null frees are no-ops.
        unsafe {
            assert!(
                traverse_dotnet_host_model_call(0, b"x".as_ptr(), 1, std::ptr::null_mut())
                    .is_null()
            );
            traverse_dotnet_host_free(std::ptr::null_mut(), 0);
        }
    }

    #[test]
    fn creates_a_dotnet_host_and_returns_model_failures_as_data() {
        let created = call(
            0,
            &request(&json!({
                "op": "create", "pins": [], "trusted_public_keys_hex": [], "model_usage": "commercial",
                "limits": { "max_package_bytes": 1, "max_memory_bytes": 1, "max_fuel": 1 }
            })),
        );
        assert_eq!(created["ok"], json!(true));
        let handle = created["handle"].as_u64().expect("handle");
        assert_eq!(
            call(handle, &request(&json!({ "op": "rights", "digest": "00" }))),
            json!({ "ok": true, "rights": null })
        );
        assert_eq!(
            call(handle, &request(&json!({ "op": "destroy" })))["ok"],
            json!(true)
        );
    }
}
