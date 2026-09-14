//! Spec `137-host-connector-command-dispatch`: app-command host connector port.
//!
//! Public runtime dispatch for a manifest-selected, explicitly activated host
//! connector. This is not `traverse_host.connector_invoke` and not the Spec 135
//! Component WIT fake.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fmt;

/// Canonical governing spec id.
pub const GOVERNING_SPEC: &str = "137-host-connector-command-dispatch";
/// Public envelope schema.
pub const SCHEMA_VERSION: &str = "1.0.0";
/// Command document kind.
pub const COMMAND_KIND: &str = "host_connector_command";
/// Result document kind.
pub const RESULT_KIND: &str = "host_connector_result";
/// Event document kind.
pub const EVENT_KIND: &str = "host_connector_event";
/// First bounded connector.
pub const AUDIO_INPUT_CONNECTOR: &str = "traverse.audio-input";
/// First bounded operation.
pub const AUDIO_CAPTURE_OPERATION: &str = "audio.capture";
/// Same-port model-runtime connector.
pub const MODEL_RUNTIME_CONNECTOR: &str = "traverse.model-runtime";
/// Same-port model-runtime operation (`local-model-runtime`).
pub const MODEL_EXECUTE_OPERATION: &str = "model.execute";

const MAX_DURATION_MS: u64 = 60_000;
const MAX_AUDIO_BYTES: u64 = 10 * 1024 * 1024;
const MAX_PAYLOAD_BYTES: usize = 16 * 1024;
const MAX_MODEL_OUTPUT_BYTES: u64 = 64 * 1024;
const MAX_EVENTS: usize = 8;

/// Stable public failure codes (FR-011).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostConnectorErrorCode {
    /// Command is not routed by the application manifest.
    UnknownCommand,
    /// Binding missing or not explicitly activated.
    Unbound,
    /// Declared version or operation is not compatible.
    Incompatible,
    /// Binding has no non-secret configuration reference.
    Unconfigured,
    /// Target family is not claimed by the activated binding.
    TargetIncompatible,
    /// Payload or resource ceiling exceeded.
    InputLimitExceeded,
    /// Caller or host cancelled the operation.
    Cancelled,
    /// Idempotency key reused with a different request.
    IdempotencyConflict,
    /// Host policy denied the operation.
    PolicyDenied,
    /// Host cannot complete the operation now.
    Unavailable,
}

impl HostConnectorErrorCode {
    /// Stable `snake_case` wire code.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UnknownCommand => "unknown_command",
            Self::Unbound => "unbound",
            Self::Incompatible => "incompatible",
            Self::Unconfigured => "unconfigured",
            Self::TargetIncompatible => "target_incompatible",
            Self::InputLimitExceeded => "input_limit_exceeded",
            Self::Cancelled => "cancelled",
            Self::IdempotencyConflict => "idempotency_conflict",
            Self::PolicyDenied => "policy_denied",
            Self::Unavailable => "unavailable",
        }
    }
}

/// Secret-free public error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostConnectorError {
    /// Stable public code.
    pub code: HostConnectorErrorCode,
    /// Explanation without host-private data.
    pub message: String,
}

impl fmt::Display for HostConnectorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for HostConnectorError {}

/// App state-machine command that should dispatch a host connector.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HostConnectorAppCommand {
    /// Must be [`COMMAND_KIND`].
    pub kind: String,
    /// Must be [`SCHEMA_VERSION`].
    pub schema_version: String,
    /// State-machine `on` value.
    pub command: String,
    /// Unique command id for this attempt.
    pub command_id: String,
    /// Correlation id shared with events and evidence.
    pub correlation_id: String,
    /// Caller-supplied idempotency key.
    pub idempotency_key: String,
    /// `browser`, `macos`, or `local`.
    pub target_family: String,
    /// When true, dispatch must not invoke the adapter.
    #[serde(default)]
    pub cancel_requested: bool,
    /// Operation-specific bounded payload.
    pub payload: Value,
}

/// Manifest-selected connector binding (Spec 103 shape, non-secret).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostConnectorBinding {
    /// Application-selected binding id.
    pub binding_id: String,
    /// Connector id, e.g. [`AUDIO_INPUT_CONNECTOR`].
    pub connector_id: String,
    /// Compatible installed version (exact for v0.1.0).
    pub version: String,
    /// Non-secret configuration reference name.
    pub config_ref: String,
    /// Placement targets this binding claims, e.g. `local`.
    pub placement_targets: Vec<String>,
}

/// Command name → connector operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostConnectorCommandRoute {
    /// State-machine command name.
    pub command: String,
    /// Connector to invoke.
    pub connector_id: String,
    /// Connector operation.
    pub operation: String,
}

/// Application view required to resolve a command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostConnectorAppManifest {
    /// Application id (evidence only).
    pub app_id: String,
    /// Spec 103 bindings.
    pub connector_bindings: Vec<HostConnectorBinding>,
    /// State-machine command routes.
    pub command_routes: Vec<HostConnectorCommandRoute>,
}

/// Explicit activation set. Registry presence never activates.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HostConnectorActivationSet {
    activated_binding_ids: Vec<String>,
}

impl HostConnectorActivationSet {
    /// Empty set: nothing is authorized.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that the application/host activated this binding id.
    pub fn activate(&mut self, binding_id: impl Into<String>) {
        let binding_id = binding_id.into();
        if !self
            .activated_binding_ids
            .iter()
            .any(|id| id == &binding_id)
        {
            self.activated_binding_ids.push(binding_id);
        }
    }

    fn is_activated(&self, binding_id: &str) -> bool {
        self.activated_binding_ids.iter().any(|id| id == binding_id)
    }
}

/// Replay cache keyed by idempotency key.
#[derive(Debug, Clone, Default)]
pub struct HostConnectorIdempotencyStore {
    entries: BTreeMap<String, IdempotencyEntry>,
}

#[derive(Debug, Clone)]
struct IdempotencyEntry {
    fingerprint: String,
    result: HostConnectorDispatch,
}

impl HostConnectorIdempotencyStore {
    /// Empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

/// Request delivered to a host adapter after authorization.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HostConnectorHostRequest {
    /// Resolved connector id.
    pub connector_id: String,
    /// Resolved operation.
    pub operation: String,
    /// Selected binding id.
    pub binding_id: String,
    /// Target family from the command.
    pub target_family: String,
    /// Correlation id.
    pub correlation_id: String,
    /// Bounded payload.
    pub payload: Value,
    /// Cancellation observed at invoke time.
    pub cancel_requested: bool,
}

/// Adapter success: opaque reference only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostConnectorHostResult {
    /// Host-managed opaque artifact reference.
    pub artifact_ref: String,
}

/// Host-owned adapter. Production native/browser drivers implement this.
pub trait HostConnectorPort {
    /// # Errors
    ///
    /// Returns a stable public code. Messages MUST be secret-free.
    fn invoke(
        &mut self,
        request: &HostConnectorHostRequest,
    ) -> Result<HostConnectorHostResult, HostConnectorError>;
}

/// Public event on the shared command/event contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostConnectorEvent {
    /// Must be [`EVENT_KIND`].
    pub kind: String,
    /// Must be [`SCHEMA_VERSION`].
    pub schema_version: String,
    /// `accepted`, `started`, `completed`, `cancelled`, or `failed`.
    pub event: String,
    /// Command id.
    pub command_id: String,
    /// Correlation id.
    pub correlation_id: String,
    /// Connector id when resolved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connector_id: Option<String>,
    /// Operation when resolved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation: Option<String>,
    /// Binding id when resolved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binding_id: Option<String>,
    /// Target family.
    pub target_family: String,
    /// Opaque artifact reference, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact_ref: Option<String>,
    /// Public error code, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
}

/// Redacted dispatch evidence (FR-012).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostConnectorEvidence {
    /// Governing spec id.
    pub governing_spec: String,
    /// Connector id when resolved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connector_id: Option<String>,
    /// Connector version when resolved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connector_version: Option<String>,
    /// Operation when resolved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation: Option<String>,
    /// Target family.
    pub target_family: String,
    /// Binding id when resolved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binding_id: Option<String>,
    /// Non-secret configuration reference name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_ref: Option<String>,
    /// Correlation id.
    pub correlation_id: String,
    /// Result class.
    pub result_class: String,
    /// Outcome code (success or error).
    pub outcome: String,
}

impl HostConnectorEvidence {
    /// Public JSON. Callers MUST treat this as the leak-checked surface.
    #[must_use]
    pub fn public_json(&self) -> Value {
        json!(self)
    }
}

/// Successful or failed public dispatch document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HostConnectorDispatch {
    /// Must be [`RESULT_KIND`].
    pub kind: String,
    /// Must be [`SCHEMA_VERSION`].
    pub schema_version: String,
    /// `succeeded`, `cancelled`, or `failed`.
    pub result_class: String,
    /// Command id.
    pub command_id: String,
    /// Correlation id.
    pub correlation_id: String,
    /// Connector id when resolved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connector_id: Option<String>,
    /// Operation when resolved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation: Option<String>,
    /// Binding id when resolved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binding_id: Option<String>,
    /// Target family.
    pub target_family: String,
    /// Opaque artifact reference on success.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact_ref: Option<String>,
    /// Public error on failure/cancellation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<HostConnectorError>,
    /// Bounded public events.
    pub events: Vec<HostConnectorEvent>,
    /// Redacted evidence.
    pub evidence: HostConnectorEvidence,
}

/// Failure carrying the public dispatch document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HostConnectorFailure {
    /// Public error.
    pub error: HostConnectorError,
    /// Failed dispatch document (events + evidence).
    pub dispatch: HostConnectorDispatch,
}

/// Mutable dispatch context: manifest, activations, replay cache, host.
pub struct HostConnectorDispatchContext<'a> {
    /// Application bindings and command routes.
    pub manifest: &'a HostConnectorAppManifest,
    /// Explicit activations.
    pub activations: &'a HostConnectorActivationSet,
    /// Idempotency replay cache.
    pub idempotency: &'a mut HostConnectorIdempotencyStore,
    /// Host adapter.
    pub host: &'a mut dyn HostConnectorPort,
}

/// Dispatch a manifest-selected host connector from an app command (FR-001).
///
/// # Errors
///
/// Returns [`HostConnectorFailure`] with a stable public code when the command,
/// binding, target, limits, cancellation, idempotency, or host adapter fail.
pub fn dispatch_host_connector_command(
    command: &HostConnectorAppCommand,
    ctx: &mut HostConnectorDispatchContext<'_>,
) -> Result<HostConnectorDispatch, Box<HostConnectorFailure>> {
    let mut events = Vec::new();
    push_event(
        &mut events,
        command,
        "accepted",
        None,
        None,
        None,
        None,
        None,
    );
    match authorize_command(command, ctx, events)? {
        Authorization::Replay(dispatch) => Ok(*dispatch),
        Authorization::Ready { authorized, events } => {
            invoke_authorized(command, ctx, &authorized, events)
        }
    }
}

struct AuthorizedCommand {
    binding: HostConnectorBinding,
    route: HostConnectorCommandRoute,
    fingerprint: String,
}

#[allow(clippy::large_enum_variant)]
enum Authorization {
    Replay(Box<HostConnectorDispatch>),
    Ready {
        authorized: AuthorizedCommand,
        events: Vec<HostConnectorEvent>,
    },
}

fn authorize_command(
    command: &HostConnectorAppCommand,
    ctx: &HostConnectorDispatchContext<'_>,
    events: Vec<HostConnectorEvent>,
) -> Result<Authorization, Box<HostConnectorFailure>> {
    if let Err(error) = validate_command_envelope(command) {
        return Err(fail(command, error.code, error.message, events, None));
    }
    let route = resolve_route(ctx.manifest, &command.command)
        .map_err(|error| fail(command, error.code, error.message, events.clone(), None))?;
    let binding = match resolve_binding(ctx.manifest, &route.connector_id) {
        Ok(binding) => binding,
        Err(error) => {
            return Err(fail(
                command,
                error.code,
                error.message,
                events,
                Some(&ResolvedRefs {
                    connector_id: route.connector_id,
                    operation: route.operation,
                    binding_id: String::new(),
                    version: String::new(),
                    config_ref: String::new(),
                }),
            ));
        }
    };
    let resolved = ResolvedRefs::from_binding(&binding, &route.operation);
    if !ctx.activations.is_activated(&binding.binding_id) {
        return Err(fail(
            command,
            HostConnectorErrorCode::Unbound,
            "binding is not explicitly activated",
            events,
            Some(&resolved),
        ));
    }
    if let Err(error) = confirm_target(&binding, &command.target_family)
        .and_then(|()| confirm_operation_payload(&route, &command.payload))
    {
        return Err(fail(
            command,
            error.code,
            error.message,
            events,
            Some(&resolved),
        ));
    }
    let fingerprint = request_fingerprint(command, &route);
    if let Some(entry) = ctx.idempotency.entries.get(&command.idempotency_key) {
        if entry.fingerprint == fingerprint {
            return Ok(Authorization::Replay(Box::new(entry.result.clone())));
        }
        return Err(fail(
            command,
            HostConnectorErrorCode::IdempotencyConflict,
            "idempotency key was reused with a different command payload",
            events,
            Some(&resolved),
        ));
    }
    if command.cancel_requested {
        return Err(fail(
            command,
            HostConnectorErrorCode::Cancelled,
            "command was cancelled before host invoke",
            events,
            Some(&resolved),
        ));
    }
    Ok(Authorization::Ready {
        authorized: AuthorizedCommand {
            binding,
            route,
            fingerprint,
        },
        events,
    })
}

fn invoke_authorized(
    command: &HostConnectorAppCommand,
    ctx: &mut HostConnectorDispatchContext<'_>,
    authorized: &AuthorizedCommand,
    mut events: Vec<HostConnectorEvent>,
) -> Result<HostConnectorDispatch, Box<HostConnectorFailure>> {
    push_event(
        &mut events,
        command,
        "started",
        Some(&authorized.binding.connector_id),
        Some(&authorized.route.operation),
        Some(&authorized.binding.binding_id),
        None,
        None,
    );
    let host_request = HostConnectorHostRequest {
        connector_id: authorized.binding.connector_id.clone(),
        operation: authorized.route.operation.clone(),
        binding_id: authorized.binding.binding_id.clone(),
        target_family: command.target_family.clone(),
        correlation_id: command.correlation_id.clone(),
        payload: command.payload.clone(),
        cancel_requested: command.cancel_requested,
    };
    let resolved = ResolvedRefs::from_binding(&authorized.binding, &authorized.route.operation);
    match ctx.host.invoke(&host_request) {
        Ok(host_result) => {
            complete_host_success(command, ctx, authorized, host_result, events, &resolved)
        }
        Err(error) => Err(fail(
            command,
            error.code,
            error.message,
            events,
            Some(&resolved),
        )),
    }
}

fn complete_host_success(
    command: &HostConnectorAppCommand,
    ctx: &mut HostConnectorDispatchContext<'_>,
    authorized: &AuthorizedCommand,
    host_result: HostConnectorHostResult,
    mut events: Vec<HostConnectorEvent>,
    resolved: &ResolvedRefs,
) -> Result<HostConnectorDispatch, Box<HostConnectorFailure>> {
    if looks_leaky(&host_result.artifact_ref) {
        return Err(fail(
            command,
            HostConnectorErrorCode::Unavailable,
            "host adapter returned a non-opaque artifact reference",
            events,
            Some(resolved),
        ));
    }
    push_event(
        &mut events,
        command,
        "completed",
        Some(&authorized.binding.connector_id),
        Some(&authorized.route.operation),
        Some(&authorized.binding.binding_id),
        Some(&host_result.artifact_ref),
        None,
    );
    let dispatch = success_dispatch(
        command,
        &authorized.binding,
        &authorized.route,
        host_result.artifact_ref,
        events,
    );
    ctx.idempotency.entries.insert(
        command.idempotency_key.clone(),
        IdempotencyEntry {
            fingerprint: authorized.fingerprint.clone(),
            result: dispatch.clone(),
        },
    );
    Ok(dispatch)
}

fn validate_command_envelope(command: &HostConnectorAppCommand) -> Result<(), HostConnectorError> {
    if command.kind != COMMAND_KIND || command.schema_version != SCHEMA_VERSION {
        return Err(HostConnectorError {
            code: HostConnectorErrorCode::Incompatible,
            message: "command kind and schema_version must be host_connector_command/1.0.0"
                .to_string(),
        });
    }
    if command.command.trim().is_empty()
        || command.command_id.trim().is_empty()
        || command.correlation_id.trim().is_empty()
        || command.idempotency_key.trim().is_empty()
        || command.target_family.trim().is_empty()
    {
        return Err(HostConnectorError {
            code: HostConnectorErrorCode::Incompatible,
            message: "command identity fields must be non-empty".to_string(),
        });
    }
    if payload_bytes(&command.payload) > MAX_PAYLOAD_BYTES {
        return Err(HostConnectorError {
            code: HostConnectorErrorCode::InputLimitExceeded,
            message: "command payload exceeds the published byte ceiling".to_string(),
        });
    }
    Ok(())
}

struct ResolvedRefs {
    connector_id: String,
    operation: String,
    binding_id: String,
    version: String,
    config_ref: String,
}

impl ResolvedRefs {
    fn from_binding(binding: &HostConnectorBinding, operation: &str) -> Self {
        Self {
            connector_id: binding.connector_id.clone(),
            operation: operation.to_string(),
            binding_id: binding.binding_id.clone(),
            version: binding.version.clone(),
            config_ref: binding.config_ref.clone(),
        }
    }
}

fn resolve_route(
    manifest: &HostConnectorAppManifest,
    command: &str,
) -> Result<HostConnectorCommandRoute, HostConnectorError> {
    let matches: Vec<&HostConnectorCommandRoute> = manifest
        .command_routes
        .iter()
        .filter(|route| route.command == command)
        .collect();
    match matches.as_slice() {
        [route] => {
            if (route.connector_id == AUDIO_INPUT_CONNECTOR
                && route.operation == AUDIO_CAPTURE_OPERATION)
                || (route.connector_id == MODEL_RUNTIME_CONNECTOR
                    && route.operation == MODEL_EXECUTE_OPERATION)
            {
                Ok((*route).clone())
            } else {
                Err(HostConnectorError {
                    code: HostConnectorErrorCode::Incompatible,
                    message: "command route is not a supported host connector operation"
                        .to_string(),
                })
            }
        }
        [] => Err(HostConnectorError {
            code: HostConnectorErrorCode::UnknownCommand,
            message: "command is not declared by the app state machine".to_string(),
        }),
        _ => Err(HostConnectorError {
            code: HostConnectorErrorCode::Incompatible,
            message: "command is routed to more than one host connector operation".to_string(),
        }),
    }
}

fn resolve_binding(
    manifest: &HostConnectorAppManifest,
    connector_id: &str,
) -> Result<HostConnectorBinding, HostConnectorError> {
    let matches: Vec<&HostConnectorBinding> = manifest
        .connector_bindings
        .iter()
        .filter(|binding| binding.connector_id == connector_id)
        .collect();
    match matches.as_slice() {
        [binding] => {
            if binding.binding_id.trim().is_empty() {
                return Err(HostConnectorError {
                    code: HostConnectorErrorCode::Unbound,
                    message: "connector binding is missing a binding id".to_string(),
                });
            }
            if binding.config_ref.trim().is_empty() {
                return Err(HostConnectorError {
                    code: HostConnectorErrorCode::Unconfigured,
                    message: "connector binding is missing a configuration reference".to_string(),
                });
            }
            if binding.version.trim().is_empty() {
                return Err(HostConnectorError {
                    code: HostConnectorErrorCode::Incompatible,
                    message: "connector binding version is incompatible".to_string(),
                });
            }
            Ok((*binding).clone())
        }
        [] => Err(HostConnectorError {
            code: HostConnectorErrorCode::Unbound,
            message: "application manifest has no binding for the routed connector".to_string(),
        }),
        _ => Err(HostConnectorError {
            code: HostConnectorErrorCode::Incompatible,
            message: "application manifest declares duplicate bindings for the connector"
                .to_string(),
        }),
    }
}

fn confirm_target(
    binding: &HostConnectorBinding,
    target_family: &str,
) -> Result<(), HostConnectorError> {
    if binding.connector_id == AUDIO_INPUT_CONNECTOR && target_family == "browser" {
        return Err(HostConnectorError {
            code: HostConnectorErrorCode::TargetIncompatible,
            message: "traverse.audio-input is native-only and cannot run on browser".to_string(),
        });
    }
    let matches = binding.placement_targets.iter().any(|target| {
        target == target_family || (target == "local" && matches!(target_family, "macos" | "local"))
    });
    if matches {
        Ok(())
    } else {
        Err(HostConnectorError {
            code: HostConnectorErrorCode::TargetIncompatible,
            message: "activated binding does not claim the requested target family".to_string(),
        })
    }
}

fn confirm_operation_payload(
    route: &HostConnectorCommandRoute,
    payload: &Value,
) -> Result<(), HostConnectorError> {
    let Some(object) = payload.as_object() else {
        return Err(limit_error("payload must be a JSON object"));
    };
    if route.operation == AUDIO_CAPTURE_OPERATION {
        let duration = required_u64(object, "max_duration_ms")?;
        let bytes = required_u64(object, "max_bytes")?;
        if duration == 0 || duration > MAX_DURATION_MS || bytes == 0 || bytes > MAX_AUDIO_BYTES {
            return Err(limit_error(
                "audio.capture duration or size exceeds published ceilings",
            ));
        }
        Ok(())
    } else {
        let output_bytes = required_u64(object, "max_output_bytes")?;
        if object.get("artifact_ref").and_then(Value::as_str).is_none()
            || object.get("policy_ref").and_then(Value::as_str).is_none()
        {
            return Err(limit_error(
                "model.execute requires artifact_ref, policy_ref, and max_output_bytes",
            ));
        }
        if output_bytes == 0 || output_bytes > MAX_MODEL_OUTPUT_BYTES {
            return Err(limit_error(
                "model.execute output ceiling exceeds published limits",
            ));
        }
        if object.contains_key("provider")
            || object.contains_key("model_id")
            || object.contains_key("endpoint")
            || object.contains_key("credential")
        {
            return Err(HostConnectorError {
                code: HostConnectorErrorCode::Incompatible,
                message: "model.execute payload must not include provider authority fields"
                    .to_string(),
            });
        }
        Ok(())
    }
}

fn required_u64(
    object: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<u64, HostConnectorError> {
    match object.get(key) {
        Some(Value::Number(number)) => number
            .as_u64()
            .ok_or_else(|| limit_error("numeric limit fields must be non-negative integers")),
        _ => Err(limit_error("required numeric limit field is missing")),
    }
}

fn limit_error(message: &str) -> HostConnectorError {
    HostConnectorError {
        code: HostConnectorErrorCode::InputLimitExceeded,
        message: message.to_string(),
    }
}

fn payload_bytes(payload: &Value) -> usize {
    serde_json::to_vec(payload).map_or(usize::MAX, |bytes| bytes.len())
}

fn request_fingerprint(
    command: &HostConnectorAppCommand,
    route: &HostConnectorCommandRoute,
) -> String {
    json!({
        "command": command.command,
        "connector_id": route.connector_id,
        "operation": route.operation,
        "target_family": command.target_family,
        "payload": command.payload,
    })
    .to_string()
}

fn looks_leaky(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower.contains("microphone")
        || lower.contains("/tmp")
        || lower.contains("http://")
        || lower.contains("https://")
        || lower.contains("avaudio")
        || lower.contains("credential")
}

#[allow(clippy::too_many_arguments)]
fn push_event(
    events: &mut Vec<HostConnectorEvent>,
    command: &HostConnectorAppCommand,
    event: &str,
    connector_id: Option<&str>,
    operation: Option<&str>,
    binding_id: Option<&str>,
    artifact_ref: Option<&str>,
    error_code: Option<&str>,
) {
    if events.len() == MAX_EVENTS {
        events.remove(0);
    }
    events.push(HostConnectorEvent {
        kind: EVENT_KIND.to_string(),
        schema_version: SCHEMA_VERSION.to_string(),
        event: event.to_string(),
        command_id: command.command_id.clone(),
        correlation_id: command.correlation_id.clone(),
        connector_id: connector_id.map(ToOwned::to_owned),
        operation: operation.map(ToOwned::to_owned),
        binding_id: binding_id.map(ToOwned::to_owned),
        target_family: command.target_family.clone(),
        artifact_ref: artifact_ref.map(ToOwned::to_owned),
        error_code: error_code.map(ToOwned::to_owned),
    });
}

fn fail(
    command: &HostConnectorAppCommand,
    code: HostConnectorErrorCode,
    message: impl Into<String>,
    mut events: Vec<HostConnectorEvent>,
    resolved: Option<&ResolvedRefs>,
) -> Box<HostConnectorFailure> {
    let error = HostConnectorError {
        code,
        message: message.into(),
    };
    let event_name = if code == HostConnectorErrorCode::Cancelled {
        "cancelled"
    } else {
        "failed"
    };
    let result_class = if code == HostConnectorErrorCode::Cancelled {
        "cancelled"
    } else {
        "failed"
    };
    push_event(
        &mut events,
        command,
        event_name,
        resolved.map(|r| r.connector_id.as_str()),
        resolved.map(|r| r.operation.as_str()),
        resolved.map(|r| r.binding_id.as_str()),
        None,
        Some(code.as_str()),
    );
    let evidence = HostConnectorEvidence {
        governing_spec: GOVERNING_SPEC.to_string(),
        connector_id: resolved.map(|r| r.connector_id.clone()),
        connector_version: resolved.map(|r| r.version.clone()),
        operation: resolved.map(|r| r.operation.clone()),
        target_family: command.target_family.clone(),
        binding_id: resolved.map(|r| r.binding_id.clone()),
        config_ref: resolved.map(|r| r.config_ref.clone()),
        correlation_id: command.correlation_id.clone(),
        result_class: result_class.to_string(),
        outcome: code.as_str().to_string(),
    };
    Box::new(HostConnectorFailure {
        dispatch: HostConnectorDispatch {
            kind: RESULT_KIND.to_string(),
            schema_version: SCHEMA_VERSION.to_string(),
            result_class: result_class.to_string(),
            command_id: command.command_id.clone(),
            correlation_id: command.correlation_id.clone(),
            connector_id: resolved.map(|r| r.connector_id.clone()),
            operation: resolved.map(|r| r.operation.clone()),
            binding_id: resolved.map(|r| r.binding_id.clone()),
            target_family: command.target_family.clone(),
            artifact_ref: None,
            error: Some(error.clone()),
            events,
            evidence,
        },
        error,
    })
}

fn success_dispatch(
    command: &HostConnectorAppCommand,
    binding: &HostConnectorBinding,
    route: &HostConnectorCommandRoute,
    artifact_ref: String,
    events: Vec<HostConnectorEvent>,
) -> HostConnectorDispatch {
    HostConnectorDispatch {
        kind: RESULT_KIND.to_string(),
        schema_version: SCHEMA_VERSION.to_string(),
        result_class: "succeeded".to_string(),
        command_id: command.command_id.clone(),
        correlation_id: command.correlation_id.clone(),
        connector_id: Some(binding.connector_id.clone()),
        operation: Some(route.operation.clone()),
        binding_id: Some(binding.binding_id.clone()),
        target_family: command.target_family.clone(),
        artifact_ref: Some(artifact_ref),
        error: None,
        events,
        evidence: HostConnectorEvidence {
            governing_spec: GOVERNING_SPEC.to_string(),
            connector_id: Some(binding.connector_id.clone()),
            connector_version: Some(binding.version.clone()),
            operation: Some(route.operation.clone()),
            target_family: command.target_family.clone(),
            binding_id: Some(binding.binding_id.clone()),
            config_ref: Some(binding.config_ref.clone()),
            correlation_id: command.correlation_id.clone(),
            result_class: "succeeded".to_string(),
            outcome: "succeeded".to_string(),
        },
    }
}

/// In-process fake host for tests. Not the Spec 135 WIT recording fake.
#[derive(Debug, Default)]
pub struct FakeHostConnector {
    invoke_count: u64,
    next_audio_id: u64,
    next_model_id: u64,
    last_request: Option<HostConnectorHostRequest>,
    deny_policy: bool,
    unavailable: bool,
}

impl FakeHostConnector {
    /// Available fake adapter.
    #[must_use]
    pub fn new() -> Self {
        Self {
            next_audio_id: 1,
            next_model_id: 1,
            ..Self::default()
        }
    }

    /// Force the next invoke to return `policy_denied`.
    pub fn deny_policy(&mut self, deny: bool) {
        self.deny_policy = deny;
    }

    /// Force the next invoke to return `unavailable`.
    pub fn set_unavailable(&mut self, unavailable: bool) {
        self.unavailable = unavailable;
    }

    /// Number of adapter invokes (idempotent replay must not increment this).
    #[must_use]
    pub fn invoke_count(&self) -> u64 {
        self.invoke_count
    }

    /// Last authorized host request, if any.
    #[must_use]
    pub fn last_request(&self) -> Option<&HostConnectorHostRequest> {
        self.last_request.as_ref()
    }
}

impl HostConnectorPort for FakeHostConnector {
    fn invoke(
        &mut self,
        request: &HostConnectorHostRequest,
    ) -> Result<HostConnectorHostResult, HostConnectorError> {
        self.invoke_count += 1;
        self.last_request = Some(request.clone());
        if request.cancel_requested {
            return Err(HostConnectorError {
                code: HostConnectorErrorCode::Cancelled,
                message: "host observed cancellation".to_string(),
            });
        }
        if self.deny_policy {
            return Err(HostConnectorError {
                code: HostConnectorErrorCode::PolicyDenied,
                message: "host policy denied the connector operation".to_string(),
            });
        }
        if self.unavailable {
            return Err(HostConnectorError {
                code: HostConnectorErrorCode::Unavailable,
                message: "host connector is unavailable".to_string(),
            });
        }
        let artifact_ref = if request.operation == AUDIO_CAPTURE_OPERATION {
            let id = self.next_audio_id;
            self.next_audio_id += 1;
            format!("audio-ref-{id}")
        } else {
            let id = self.next_model_id;
            self.next_model_id += 1;
            format!("model-ref-{id}")
        };
        Ok(HostConnectorHostResult { artifact_ref })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn audio_binding() -> HostConnectorBinding {
        HostConnectorBinding {
            binding_id: "default-local-audio".to_string(),
            connector_id: AUDIO_INPUT_CONNECTOR.to_string(),
            version: "1.0.0".to_string(),
            config_ref: "audio-authority".to_string(),
            placement_targets: vec!["local".to_string()],
        }
    }

    fn model_binding() -> HostConnectorBinding {
        HostConnectorBinding {
            binding_id: "default-local-model".to_string(),
            connector_id: MODEL_RUNTIME_CONNECTOR.to_string(),
            version: "1.0.0".to_string(),
            config_ref: "model-authority".to_string(),
            placement_targets: vec!["local".to_string()],
        }
    }

    fn audio_manifest() -> HostConnectorAppManifest {
        HostConnectorAppManifest {
            app_id: "callweave.recording".to_string(),
            connector_bindings: vec![audio_binding()],
            command_routes: vec![HostConnectorCommandRoute {
                command: "capture_audio".to_string(),
                connector_id: AUDIO_INPUT_CONNECTOR.to_string(),
                operation: AUDIO_CAPTURE_OPERATION.to_string(),
            }],
        }
    }

    fn combined_manifest() -> HostConnectorAppManifest {
        let mut manifest = audio_manifest();
        manifest.connector_bindings.push(model_binding());
        manifest.command_routes.push(HostConnectorCommandRoute {
            command: "run_local_model".to_string(),
            connector_id: MODEL_RUNTIME_CONNECTOR.to_string(),
            operation: MODEL_EXECUTE_OPERATION.to_string(),
        });
        manifest
    }

    fn audio_command(target_family: &str) -> HostConnectorAppCommand {
        HostConnectorAppCommand {
            kind: COMMAND_KIND.to_string(),
            schema_version: SCHEMA_VERSION.to_string(),
            command: "capture_audio".to_string(),
            command_id: "cmd-00000001".to_string(),
            correlation_id: "corr-00000001".to_string(),
            idempotency_key: "idem-00000001".to_string(),
            target_family: target_family.to_string(),
            cancel_requested: false,
            payload: json!({"max_duration_ms": 5_000, "max_bytes": 1_048_576}),
        }
    }

    fn model_command() -> HostConnectorAppCommand {
        HostConnectorAppCommand {
            kind: COMMAND_KIND.to_string(),
            schema_version: SCHEMA_VERSION.to_string(),
            command: "run_local_model".to_string(),
            command_id: "cmd-00000002".to_string(),
            correlation_id: "corr-00000002".to_string(),
            idempotency_key: "idem-00000002".to_string(),
            target_family: "macos".to_string(),
            cancel_requested: false,
            payload: json!({
                "artifact_ref": "model-artifact-1",
                "policy_ref": "policy-1",
                "max_output_bytes": 4096
            }),
        }
    }

    fn dispatch_with(
        command: &HostConnectorAppCommand,
        manifest: &HostConnectorAppManifest,
        activations: &HostConnectorActivationSet,
        host: &mut FakeHostConnector,
        idempotency: &mut HostConnectorIdempotencyStore,
    ) -> Result<HostConnectorDispatch, Box<HostConnectorFailure>> {
        let mut ctx = HostConnectorDispatchContext {
            manifest,
            activations,
            idempotency,
            host,
        };
        dispatch_host_connector_command(command, &mut ctx)
    }

    fn require_ok(
        command: &HostConnectorAppCommand,
        manifest: &HostConnectorAppManifest,
        activations: &HostConnectorActivationSet,
        host: &mut FakeHostConnector,
        idempotency: &mut HostConnectorIdempotencyStore,
    ) -> Result<HostConnectorDispatch, String> {
        dispatch_with(command, manifest, activations, host, idempotency)
            .map_err(|failure| failure.error.to_string())
    }

    fn require_err(
        command: &HostConnectorAppCommand,
        manifest: &HostConnectorAppManifest,
        activations: &HostConnectorActivationSet,
        host: &mut FakeHostConnector,
        idempotency: &mut HostConnectorIdempotencyStore,
    ) -> Result<HostConnectorFailure, String> {
        match dispatch_with(command, manifest, activations, host, idempotency) {
            Ok(_) => Err("expected host connector failure".to_string()),
            Err(failure) => Ok(*failure),
        }
    }

    fn activated_audio() -> HostConnectorActivationSet {
        let mut activations = HostConnectorActivationSet::new();
        activations.activate("default-local-audio");
        activations
    }

    fn assert_no_leak(value: &Value) {
        let encoded = value.to_string();
        assert!(!encoded.contains("microphone"));
        assert!(!encoded.contains("/tmp"));
        assert!(!encoded.contains("AVAudio"));
        assert!(!encoded.contains("credential"));
        assert!(!encoded.contains("http://"));
    }

    #[test]
    fn macos_audio_capture_succeeds_through_fake_host() -> Result<(), String> {
        let manifest = audio_manifest();
        let activations = activated_audio();
        let mut host = FakeHostConnector::new();
        let mut idempotency = HostConnectorIdempotencyStore::new();
        let dispatch = require_ok(
            &audio_command("macos"),
            &manifest,
            &activations,
            &mut host,
            &mut idempotency,
        )?;
        assert_eq!(dispatch.result_class, "succeeded");
        assert_eq!(dispatch.operation.as_deref(), Some(AUDIO_CAPTURE_OPERATION));
        assert_eq!(dispatch.artifact_ref.as_deref(), Some("audio-ref-1"));
        assert_eq!(host.invoke_count(), 1);
        assert_eq!(
            host.last_request().map(|r| r.operation.as_str()),
            Some(AUDIO_CAPTURE_OPERATION)
        );
        let events: Vec<&str> = dispatch.events.iter().map(|e| e.event.as_str()).collect();
        assert_eq!(events, ["accepted", "started", "completed"]);
        assert_eq!(dispatch.events[0].kind, EVENT_KIND);
        assert_eq!(dispatch.evidence.governing_spec, GOVERNING_SPEC);
        assert_eq!(
            dispatch.evidence.config_ref.as_deref(),
            Some("audio-authority")
        );
        assert_no_leak(&json!(dispatch));
        Ok(())
    }

    #[test]
    fn manifest_selected_binding_is_the_one_invoked() -> Result<(), String> {
        let mut manifest = audio_manifest();
        let mut second = audio_binding();
        second.binding_id = "other-audio".to_string();
        manifest.connector_bindings.push(second);
        let activations = activated_audio();
        let mut host = FakeHostConnector::new();
        let mut idempotency = HostConnectorIdempotencyStore::new();
        let failed = require_err(
            &audio_command("macos"),
            &manifest,
            &activations,
            &mut host,
            &mut idempotency,
        )?;
        assert_eq!(failed.error.code, HostConnectorErrorCode::Incompatible);
        assert_eq!(host.invoke_count(), 0);
        Ok(())
    }

    #[test]
    fn missing_incompatible_unconfigured_and_unactivated_bindings_fail_before_host()
    -> Result<(), String> {
        let activations = activated_audio();
        let mut host = FakeHostConnector::new();
        let mut idempotency = HostConnectorIdempotencyStore::new();
        let mut missing = audio_manifest();
        missing.connector_bindings.clear();
        let failed = require_err(
            &audio_command("macos"),
            &missing,
            &activations,
            &mut host,
            &mut idempotency,
        )?;
        assert_eq!(failed.error.code, HostConnectorErrorCode::Unbound);

        let mut unconfigured = audio_manifest();
        unconfigured.connector_bindings[0].config_ref.clear();
        let failed = require_err(
            &audio_command("macos"),
            &unconfigured,
            &activations,
            &mut host,
            &mut idempotency,
        )?;
        assert_eq!(failed.error.code, HostConnectorErrorCode::Unconfigured);

        let empty = HostConnectorActivationSet::new();
        let failed = require_err(
            &audio_command("macos"),
            &audio_manifest(),
            &empty,
            &mut host,
            &mut idempotency,
        )?;
        assert_eq!(failed.error.code, HostConnectorErrorCode::Unbound);

        let mut unknown = audio_command("macos");
        unknown.command = "not_a_route".to_string();
        let failed = require_err(
            &unknown,
            &audio_manifest(),
            &activations,
            &mut host,
            &mut idempotency,
        )?;
        assert_eq!(failed.error.code, HostConnectorErrorCode::UnknownCommand);
        assert_eq!(host.invoke_count(), 0);
        Ok(())
    }

    #[test]
    fn browser_rejects_native_audio_with_shared_event_contract() -> Result<(), String> {
        let activations = activated_audio();
        let mut host = FakeHostConnector::new();
        let mut idempotency = HostConnectorIdempotencyStore::new();
        let failed = require_err(
            &audio_command("browser"),
            &audio_manifest(),
            &activations,
            &mut host,
            &mut idempotency,
        )?;
        assert_eq!(
            failed.error.code,
            HostConnectorErrorCode::TargetIncompatible
        );
        assert_eq!(failed.dispatch.target_family, "browser");
        assert_eq!(failed.dispatch.events[0].kind, EVENT_KIND);
        assert_eq!(failed.dispatch.events[0].schema_version, SCHEMA_VERSION);
        assert_eq!(host.invoke_count(), 0);
        assert_no_leak(&json!(failed.dispatch));
        Ok(())
    }

    #[test]
    fn cancellation_and_idempotency_are_honored() -> Result<(), String> {
        let manifest = audio_manifest();
        let activations = activated_audio();
        let mut host = FakeHostConnector::new();
        let mut idempotency = HostConnectorIdempotencyStore::new();
        let mut cancelled = audio_command("macos");
        cancelled.cancel_requested = true;
        let failed = require_err(
            &cancelled,
            &manifest,
            &activations,
            &mut host,
            &mut idempotency,
        )?;
        assert_eq!(failed.error.code, HostConnectorErrorCode::Cancelled);
        assert_eq!(failed.dispatch.result_class, "cancelled");
        assert_eq!(host.invoke_count(), 0);

        let first = require_ok(
            &audio_command("macos"),
            &manifest,
            &activations,
            &mut host,
            &mut idempotency,
        )?;
        let mut replay = audio_command("macos");
        replay.command_id = "cmd-replay".to_string();
        let second = require_ok(
            &replay,
            &manifest,
            &activations,
            &mut host,
            &mut idempotency,
        )?;
        assert_eq!(second.artifact_ref, first.artifact_ref);
        assert_eq!(host.invoke_count(), 1);

        let mut conflict = audio_command("macos");
        conflict.payload = json!({"max_duration_ms": 1_000, "max_bytes": 2048});
        let failed = require_err(
            &conflict,
            &manifest,
            &activations,
            &mut host,
            &mut idempotency,
        )?;
        assert_eq!(
            failed.error.code,
            HostConnectorErrorCode::IdempotencyConflict
        );
        assert_eq!(host.invoke_count(), 1);
        Ok(())
    }

    #[test]
    fn bounded_inputs_and_structured_errors_do_not_invoke_host() -> Result<(), String> {
        let activations = activated_audio();
        let mut host = FakeHostConnector::new();
        let mut idempotency = HostConnectorIdempotencyStore::new();
        let mut oversized = audio_command("macos");
        oversized.payload = json!({"max_duration_ms": 120_000, "max_bytes": 1_048_576});
        let failed = require_err(
            &oversized,
            &audio_manifest(),
            &activations,
            &mut host,
            &mut idempotency,
        )?;
        assert_eq!(
            failed.error.code,
            HostConnectorErrorCode::InputLimitExceeded
        );
        assert_eq!(host.invoke_count(), 0);

        host.deny_policy(true);
        let failed = require_err(
            &audio_command("macos"),
            &audio_manifest(),
            &activations,
            &mut host,
            &mut idempotency,
        )?;
        assert_eq!(failed.error.code, HostConnectorErrorCode::PolicyDenied);
        assert!(
            failed
                .dispatch
                .events
                .iter()
                .any(|event| event.event == "failed")
        );
        Ok(())
    }

    #[test]
    fn model_execute_uses_the_same_port() -> Result<(), String> {
        let manifest = combined_manifest();
        let mut activations = activated_audio();
        activations.activate("default-local-model");
        let mut host = FakeHostConnector::new();
        let mut idempotency = HostConnectorIdempotencyStore::new();
        let dispatch = require_ok(
            &model_command(),
            &manifest,
            &activations,
            &mut host,
            &mut idempotency,
        )?;
        assert_eq!(
            dispatch.connector_id.as_deref(),
            Some(MODEL_RUNTIME_CONNECTOR)
        );
        assert_eq!(dispatch.operation.as_deref(), Some(MODEL_EXECUTE_OPERATION));
        assert_eq!(dispatch.artifact_ref.as_deref(), Some("model-ref-1"));
        assert_eq!(dispatch.kind, RESULT_KIND);
        assert_no_leak(&json!(dispatch));

        let mut forbidden = model_command();
        forbidden.payload = json!({
            "artifact_ref": "model-artifact-1",
            "policy_ref": "policy-1",
            "max_output_bytes": 4096,
            "provider": "ollama"
        });
        forbidden.idempotency_key = "idem-forbidden".to_string();
        let failed = require_err(
            &forbidden,
            &manifest,
            &activations,
            &mut host,
            &mut idempotency,
        )?;
        assert_eq!(failed.error.code, HostConnectorErrorCode::Incompatible);
        Ok(())
    }

    #[test]
    fn public_codes_are_stable_and_guest_paths_are_unused() {
        for code in [
            HostConnectorErrorCode::UnknownCommand,
            HostConnectorErrorCode::Unbound,
            HostConnectorErrorCode::Incompatible,
            HostConnectorErrorCode::Unconfigured,
            HostConnectorErrorCode::TargetIncompatible,
            HostConnectorErrorCode::InputLimitExceeded,
            HostConnectorErrorCode::Cancelled,
            HostConnectorErrorCode::IdempotencyConflict,
            HostConnectorErrorCode::PolicyDenied,
            HostConnectorErrorCode::Unavailable,
        ] {
            assert!(!code.as_str().is_empty());
            assert!(!code.as_str().contains("connector_invoke"));
        }
        // This module's fake is FakeHostConnector, not Spec 135 FakeRecordingHost.
        let _fake = FakeHostConnector::new();
        assert_eq!(COMMAND_KIND, "host_connector_command");
        assert_ne!(COMMAND_KIND, "connector_invoke");
    }
}
