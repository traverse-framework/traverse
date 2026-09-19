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
/// Permission request on [`AUDIO_INPUT_CONNECTOR`] (Spec 137 0.3.0 / Spec 140).
pub const AUDIO_PERMISSION_REQUEST_OPERATION: &str = "audio.permission.request";
/// Same-port model-runtime connector.
pub const MODEL_RUNTIME_CONNECTOR: &str = "traverse.model-runtime";
/// Same-port model-runtime operation (`local-model-runtime`).
pub const MODEL_EXECUTE_OPERATION: &str = "model.execute";

/// Non-secret permission outcome from `audio.permission.request`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostConnectorPermissionState {
    /// Permission is granted.
    Granted,
    /// Permission was denied.
    Denied,
    /// Host needs an additional user gesture/prompt.
    PromptRequired,
    /// Permission status cannot be determined.
    Unavailable,
}

impl HostConnectorPermissionState {
    /// Stable `snake_case` wire value.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Granted => "granted",
            Self::Denied => "denied",
            Self::PromptRequired => "prompt_required",
            Self::Unavailable => "unavailable",
        }
    }
}

const MAX_DURATION_MS: u64 = 60_000;
const MAX_AUDIO_BYTES: u64 = 10 * 1024 * 1024;
const MAX_PAYLOAD_BYTES: usize = 16 * 1024;
const MAX_MODEL_OUTPUT_BYTES: u64 = 16 * 1024 * 1024;
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
    /// Request payload is invalid for Spec 138.
    InvalidInput,
    /// Exact model pin or verified package is missing.
    ModelUnavailable,
    /// Model package or ABI/schema is incompatible.
    ModelIncompatible,
    /// Memory, fuel, I/O, or output ceilings were exceeded.
    ResourceExhausted,
    /// Execution exceeded the configured timeout.
    Timeout,
    /// Model guest trapped or the executor failed.
    ExecutionFailed,
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
            Self::InvalidInput => "invalid_input",
            Self::ModelUnavailable => "model_unavailable",
            Self::ModelIncompatible => "model_incompatible",
            Self::ResourceExhausted => "resource_exhausted",
            Self::Timeout => "timeout",
            Self::ExecutionFailed => "execution_failed",
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

/// Adapter success payload (opaque artifact and/or permission state).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostConnectorHostResult {
    /// Host-managed opaque artifact reference (capture / model.execute).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_ref: Option<String>,
    /// Non-secret permission outcome (`audio.permission.request`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_state: Option<HostConnectorPermissionState>,
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
    /// Non-secret permission state, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_state: Option<HostConnectorPermissionState>,
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
    /// Non-secret permission state on permission success.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_state: Option<HostConnectorPermissionState>,
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
    let fingerprint = json!({
        "command": command.command,
        "connector_id": route.connector_id,
        "operation": route.operation,
        "target_family": command.target_family,
        "payload": command.payload,
    })
    .to_string();
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
    if authorized.route.operation == AUDIO_PERMISSION_REQUEST_OPERATION {
        return complete_permission_success(
            command,
            ctx,
            authorized,
            host_result,
            events,
            resolved,
        );
    }
    let Some(artifact_ref) = host_result
        .artifact_ref
        .filter(|value| !value.trim().is_empty())
    else {
        return Err(fail(
            command,
            HostConnectorErrorCode::Unavailable,
            "host adapter returned an empty artifact reference",
            events,
            Some(resolved),
        ));
    };
    if looks_leaky(&artifact_ref) {
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
        Some(&artifact_ref),
        None,
        None,
    );
    let dispatch = success_dispatch(
        command,
        &authorized.binding,
        &authorized.route,
        Some(artifact_ref),
        None,
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

fn complete_permission_success(
    command: &HostConnectorAppCommand,
    ctx: &mut HostConnectorDispatchContext<'_>,
    authorized: &AuthorizedCommand,
    host_result: HostConnectorHostResult,
    events: Vec<HostConnectorEvent>,
    resolved: &ResolvedRefs,
) -> Result<HostConnectorDispatch, Box<HostConnectorFailure>> {
    let Some(permission_state) = host_result.permission_state else {
        return Err(fail(
            command,
            HostConnectorErrorCode::Unavailable,
            "host adapter returned no permission_state",
            events,
            Some(resolved),
        ));
    };
    match permission_state {
        HostConnectorPermissionState::Denied => Err(fail(
            command,
            HostConnectorErrorCode::PolicyDenied,
            "audio permission was denied",
            events,
            Some(resolved),
        )),
        HostConnectorPermissionState::Unavailable => Err(fail(
            command,
            HostConnectorErrorCode::Unavailable,
            "audio permission is unavailable",
            events,
            Some(resolved),
        )),
        HostConnectorPermissionState::Granted | HostConnectorPermissionState::PromptRequired => {
            let mut events = events;
            push_event(
                &mut events,
                command,
                "completed",
                Some(&authorized.binding.connector_id),
                Some(&authorized.route.operation),
                Some(&authorized.binding.binding_id),
                None,
                Some(permission_state),
                None,
            );
            let dispatch = success_dispatch(
                command,
                &authorized.binding,
                &authorized.route,
                None,
                Some(permission_state),
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
    }
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
                && (route.operation == AUDIO_CAPTURE_OPERATION
                    || route.operation == AUDIO_PERMISSION_REQUEST_OPERATION))
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
    // Spec 137 FR-006 / Spec 140: target neutrality is binding-declared.
    // A family with no declared/activated adapter fails before the host runs.
    if binding.placement_targets.is_empty() {
        return Err(HostConnectorError {
            code: HostConnectorErrorCode::TargetIncompatible,
            message: "activated binding does not declare any supported target families".to_string(),
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
    if route.operation == AUDIO_PERMISSION_REQUEST_OPERATION {
        if !object.is_empty() {
            return Err(limit_error(
                "audio.permission.request payload must be an empty object",
            ));
        }
        Ok(())
    } else if route.operation == AUDIO_CAPTURE_OPERATION {
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
        let model_ref_ok = object
            .get("model_ref")
            .and_then(Value::as_object)
            .is_some_and(|model_ref| {
                model_ref.get("model_id").and_then(Value::as_str).is_some()
                    && model_ref.get("version").and_then(Value::as_str).is_some()
                    && model_ref.get("digest").and_then(Value::as_str).is_some()
            });
        let required_refs_ok = object.get("input_ref").and_then(Value::as_str).is_some()
            && object.get("policy_ref").and_then(Value::as_str).is_some()
            && object
                .get("data_classification")
                .and_then(Value::as_str)
                .is_some()
            && object
                .get("input_schema_ref")
                .and_then(Value::as_str)
                .is_some()
            && object
                .get("input_schema_version")
                .and_then(Value::as_str)
                .is_some();
        if !model_ref_ok || !required_refs_ok {
            return Err(limit_error(
                "model.execute requires model_ref, input_ref, policy_ref, data_classification, input_schema_ref, input_schema_version, and max_output_bytes",
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
            || object.contains_key("artifact_ref")
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
    payload.to_string().len()
}

fn looks_leaky(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    let mut leaky = false;
    for needle in [
        "microphone",
        "/tmp/",
        "http://",
        "https://",
        "avaudio",
        "credential",
    ] {
        if lower.contains(needle) {
            leaky = true;
        }
    }
    leaky
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
    permission_state: Option<HostConnectorPermissionState>,
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
        permission_state,
        error_code: error_code.map(ToOwned::to_owned),
    });
}

#[doc(hidden)]
#[must_use]
pub fn bound_host_connector_event_queue(command: &HostConnectorAppCommand) -> usize {
    let mut events = Vec::new();
    for _ in 0..=MAX_EVENTS {
        push_event(
            &mut events,
            command,
            "accepted",
            None,
            None,
            None,
            None,
            None,
            None,
        );
    }
    events.len()
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
            permission_state: None,
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
    artifact_ref: Option<String>,
    permission_state: Option<HostConnectorPermissionState>,
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
        artifact_ref,
        permission_state,
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
