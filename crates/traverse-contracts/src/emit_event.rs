//! Shared, engine-agnostic `traverse_host::emit_event` validation core (spec
//! `098-capability-event-host-abi` FR-002/FR-003/FR-008; spec
//! `1402-runtime-wasm-orchestrator-convergence` FR-005/FR-011).
//!
//! This module validates an `emit_event` guest payload once a host has
//! already turned the guest's raw `(ptr, len)` pair into a safe byte slice —
//! it never touches a pointer itself, so every native (Wasmtime) and nested
//! (`wasmi`) `runtime.wasm` executor calls the same code path for the part
//! that must behave identically across engines (spec 1402 FR-005), instead
//! of each engine maintaining its own copy of this logic.

use serde_json::Value;

use crate::{EventReference, ServiceType};

/// Maximum bytes accepted for one `emit_event` payload (spec 098 FR-008).
pub const MAX_EVENT_EMIT_PAYLOAD_BYTES: usize = 64 * 1024;

/// `emit_event` accepted the event.
pub const EMIT_EVENT_OK: i32 = 0;
/// The payload was too large, unreadable, malformed JSON, or missing a
/// required `event_id`/`version` string field (spec 098 FR-008).
pub const EMIT_EVENT_ERR_INVALID_PAYLOAD: i32 = -1;
/// The event type/version is not declared in the capability's `emits` list
/// (spec 098 FR-002).
pub const EMIT_EVENT_ERR_UNDECLARED_EVENT: i32 = -2;
/// The calling capability's `service_type` is not `Subscribable` (spec 098
/// FR-003).
pub const EMIT_EVENT_ERR_NOT_SUBSCRIBABLE: i32 = -3;

/// A validated `emit_event` call, ready for a caller to turn into its own
/// event-record type (e.g. `traverse-runtime`'s `TraverseEvent`). This
/// module deliberately stops short of constructing that record itself —
/// callers differ in what wraps it (ids, timestamps), not in validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedEmitEvent {
    pub event_type: String,
    pub version: String,
    pub data: Value,
}

/// One `emit_event` validation failure, carrying its stable error code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmitEventError {
    InvalidPayload,
    UndeclaredEvent,
    NotSubscribable,
}

impl EmitEventError {
    #[must_use]
    pub fn code(self) -> i32 {
        match self {
            Self::InvalidPayload => EMIT_EVENT_ERR_INVALID_PAYLOAD,
            Self::UndeclaredEvent => EMIT_EVENT_ERR_UNDECLARED_EVENT,
            Self::NotSubscribable => EMIT_EVENT_ERR_NOT_SUBSCRIBABLE,
        }
    }
}

/// Validates a raw `emit_event` payload against the calling capability's
/// `service_type` and declared `emits` list. `payload` is the guest's own
/// linear-memory bytes, already safely read by the caller — this function
/// never touches a pointer.
///
/// Order matches `098` FR-002/FR-003/FR-008 and both existing engine
/// implementations: `service_type` is checked before any payload
/// inspection, then payload size/shape, then the declared-emission check.
///
/// # Errors
///
/// Returns [`EmitEventError`] — never panics — for a non-`Subscribable`
/// capability, an oversized or malformed payload, a payload missing
/// `event_id`/`version` string fields, or an event not present in
/// `declared_emits`.
pub fn validate_emit_event(
    payload: &[u8],
    service_type: &ServiceType,
    declared_emits: &[EventReference],
) -> Result<ValidatedEmitEvent, EmitEventError> {
    if *service_type != ServiceType::Subscribable {
        return Err(EmitEventError::NotSubscribable);
    }

    if payload.len() > MAX_EVENT_EMIT_PAYLOAD_BYTES {
        return Err(EmitEventError::InvalidPayload);
    }

    let parsed: Value =
        serde_json::from_slice(payload).map_err(|_| EmitEventError::InvalidPayload)?;

    let event_type = parsed
        .get("event_id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or(EmitEventError::InvalidPayload)?;
    let version = parsed
        .get("version")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or(EmitEventError::InvalidPayload)?;
    let data = parsed
        .get("payload")
        .cloned()
        .unwrap_or_else(|| Value::Object(serde_json::Map::new()));

    let declared = declared_emits
        .iter()
        .any(|decl| decl.event_id == event_type && decl.version == version);
    if !declared {
        return Err(EmitEventError::UndeclaredEvent);
    }

    Ok(ValidatedEmitEvent {
        event_type,
        version,
        data,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]

    use super::*;

    fn declared() -> Vec<EventReference> {
        vec![EventReference {
            event_id: "order.placed".to_string(),
            version: "1.0.0".to_string(),
        }]
    }

    fn valid_payload() -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "event_id": "order.placed",
            "version": "1.0.0",
            "payload": {"order_id": "abc"}
        }))
        .expect("json encode")
    }

    #[test]
    fn accepts_a_declared_subscribable_event() {
        let result = validate_emit_event(&valid_payload(), &ServiceType::Subscribable, &declared());
        let validated = result.expect("must validate");
        assert_eq!(validated.event_type, "order.placed");
        assert_eq!(validated.version, "1.0.0");
        assert_eq!(validated.data, serde_json::json!({"order_id": "abc"}));
    }

    #[test]
    fn rejects_non_subscribable_before_touching_payload() {
        let result = validate_emit_event(b"not even json", &ServiceType::Stateless, &declared());
        assert_eq!(result, Err(EmitEventError::NotSubscribable));
    }

    #[test]
    fn rejects_oversized_payload() {
        let oversized = vec![b'a'; MAX_EVENT_EMIT_PAYLOAD_BYTES + 1];
        let result = validate_emit_event(&oversized, &ServiceType::Subscribable, &declared());
        assert_eq!(result, Err(EmitEventError::InvalidPayload));
    }

    #[test]
    fn rejects_malformed_json() {
        let result = validate_emit_event(b"{not json", &ServiceType::Subscribable, &declared());
        assert_eq!(result, Err(EmitEventError::InvalidPayload));
    }

    #[test]
    fn rejects_missing_event_id() {
        let payload = serde_json::to_vec(&serde_json::json!({"version": "1.0.0"})).unwrap();
        let result = validate_emit_event(&payload, &ServiceType::Subscribable, &declared());
        assert_eq!(result, Err(EmitEventError::InvalidPayload));
    }

    #[test]
    fn rejects_missing_version() {
        let payload = serde_json::to_vec(&serde_json::json!({"event_id": "order.placed"})).unwrap();
        let result = validate_emit_event(&payload, &ServiceType::Subscribable, &declared());
        assert_eq!(result, Err(EmitEventError::InvalidPayload));
    }

    #[test]
    fn rejects_undeclared_event() {
        let payload = serde_json::to_vec(&serde_json::json!({
            "event_id": "order.cancelled",
            "version": "1.0.0"
        }))
        .unwrap();
        let result = validate_emit_event(&payload, &ServiceType::Subscribable, &declared());
        assert_eq!(result, Err(EmitEventError::UndeclaredEvent));
    }

    #[test]
    fn defaults_missing_payload_field_to_empty_object() {
        let payload = serde_json::to_vec(&serde_json::json!({
            "event_id": "order.placed",
            "version": "1.0.0"
        }))
        .unwrap();
        let result = validate_emit_event(&payload, &ServiceType::Subscribable, &declared());
        let validated = result.expect("must validate");
        assert_eq!(validated.data, serde_json::json!({}));
    }

    #[test]
    fn error_codes_match_spec_098() {
        assert_eq!(EmitEventError::InvalidPayload.code(), -1);
        assert_eq!(EmitEventError::UndeclaredEvent.code(), -2);
        assert_eq!(EmitEventError::NotSubscribable.code(), -3);
        assert_eq!(EMIT_EVENT_OK, 0);
    }
}
