//! Spec 139 Process Manager for app `state_machine` sessions inside
//! `runtime.wasm` (capability-invoke path). Host-connector bridge waits are
//! staged as explicit pending host requests + deadline registration.

use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub(crate) struct AppStateMachine {
    pub(crate) initial_state: String,
    pub(crate) states: BTreeMap<String, AppState>,
}

#[derive(Debug, Clone)]
pub(crate) struct AppState {
    pub(crate) invoke: Option<AppInvoke>,
    pub(crate) transitions: Vec<AppTransition>,
}

#[derive(Debug, Clone)]
pub(crate) enum AppInvoke {
    Capability {
        capability_id: String,
        input_from: String,
    },
    HostConnector {
        command: String,
    },
}

/// A fail-closed failure resolving a capability's input (Decision 99 /
/// Spec 139 FR-022). Never invoke the capability when this is returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CapabilityInputError {
    pub(crate) code: &'static str,
    pub(crate) message: String,
}

const HOST_CONNECTOR_RESULT_PREFIX: &str = "host_connector_result.";

/// Resolves an `invoke.capability_id` state's input per `input_from`
/// (Decision 99 / Spec 139 FR-019/FR-020/FR-022).
///
/// `"command.payload"` (the default) uses the triggering command's payload
/// verbatim. `"host_connector_result.<field>"` looks up `<field>` in the
/// most recently completed host-connector wait's result and wraps it as the
/// capability's **entire** input under that same key, e.g.
/// `{"artifact_base64": <value>}` — the runtime never re-labels a
/// host-connector-produced field; the adapter names it once and this lookup
/// is a pure key match. Any other `input_from` value fails closed rather
/// than silently falling back to `command.payload`.
pub(crate) fn resolve_capability_input(
    input_from: &str,
    command_payload: &Value,
    last_host_connector_result: Option<&Value>,
) -> Result<Value, CapabilityInputError> {
    if input_from == "command.payload" {
        return Ok(command_payload.clone());
    }
    let Some(field) = input_from
        .strip_prefix(HOST_CONNECTOR_RESULT_PREFIX)
        .filter(|field| !field.trim().is_empty())
    else {
        return Err(CapabilityInputError {
            code: "invalid_input_from",
            message: format!("unsupported invoke.input_from '{input_from}'"),
        });
    };
    let Some(result) = last_host_connector_result else {
        return Err(CapabilityInputError {
            code: "invalid_input",
            message: format!(
                "input_from references host_connector_result.{field} but no host-connector wait has completed in this session"
            ),
        });
    };
    let Some(value) = result.get(field) else {
        return Err(CapabilityInputError {
            code: "invalid_input",
            message: format!(
                "the most recently completed host-connector result has no field '{field}'"
            ),
        });
    };
    Ok(json!({ field: value.clone() }))
}

#[derive(Debug, Clone)]
pub(crate) struct AppTransition {
    pub(crate) on: String,
    pub(crate) to: String,
}

#[derive(Debug, Clone)]
pub(crate) struct AppSession {
    #[allow(dead_code)]
    pub(crate) session_id: String,
    pub(crate) state: String,
    pub(crate) wait: Option<AppWait>,
}

#[derive(Debug, Clone)]
pub(crate) struct AppWait {
    pub(crate) command_id: String,
    pub(crate) kind: AppWaitKind,
}

#[derive(Debug, Clone)]
pub(crate) enum AppWaitKind {
    Capability {
        #[allow(dead_code)]
        capability_id: String,
    },
    HostConnector {
        #[allow(dead_code)]
        command: String,
    },
}

#[derive(Debug, Clone)]
pub(crate) struct AppCommandEnvelope {
    pub(crate) command: String,
    pub(crate) payload: Value,
    pub(crate) session_id: Option<String>,
}

/// Spec 137 / 139 host-connector terminal delivered back into `runtime.wasm`
/// over the existing `traverse_submit` ABI (no new bridge export).
#[derive(Debug, Clone)]
pub(crate) struct HostConnectorTerminal {
    pub(crate) command_id: String,
    pub(crate) session_id: String,
    /// One of Spec 139 FR-010: `host_connector_succeeded|failed|cancelled|timeout`.
    pub(crate) lifecycle_event: String,
    pub(crate) payload: Value,
}

#[derive(Debug, Clone)]
pub(crate) enum SubmitDiscrimination {
    AppCommand(AppCommandEnvelope),
    HostConnectorTerminal(HostConnectorTerminal),
    CapabilityOrWorkflow,
    Ambiguous,
    Invalid(String),
}

/// Fail-closed discrimination between Spec 068 workflow/capability submit,
/// Spec 139 `app_command` envelopes, and Spec 137 host-connector terminals.
pub(crate) fn discriminate_submit(request: &Value) -> SubmitDiscrimination {
    let Some(object) = request.as_object() else {
        return SubmitDiscrimination::Invalid("submit body must be a JSON object".to_string());
    };
    let kind = object.get("kind").and_then(Value::as_str);
    let has_target = object.contains_key("target_id");
    let has_command = object.contains_key("command");

    if kind == Some("app_command") && has_target {
        return SubmitDiscrimination::Ambiguous;
    }
    if kind == Some("app_command") {
        let command = match object.get("command").and_then(Value::as_str) {
            Some(command) if !command.trim().is_empty() => command.to_string(),
            _ => {
                return SubmitDiscrimination::Invalid(
                    "app_command requires a non-empty command".to_string(),
                );
            }
        };
        let session_id = match object.get("session_id") {
            None | Some(Value::Null) => None,
            Some(Value::String(session_id)) if !session_id.trim().is_empty() => {
                Some(session_id.clone())
            }
            _ => {
                return SubmitDiscrimination::Invalid(
                    "session_id must be a non-empty string when present".to_string(),
                );
            }
        };
        let payload = object.get("payload").cloned().unwrap_or_else(|| json!({}));
        return SubmitDiscrimination::AppCommand(AppCommandEnvelope {
            command,
            payload,
            session_id,
        });
    }
    if let Some(kind @ ("host_connector_result" | "deadline_fired")) = kind {
        return parse_host_connector_terminal(object, kind);
    }
    if has_command && !has_target && kind.is_none() {
        // Bare `command` without `kind` is ambiguous with future envelopes.
        return SubmitDiscrimination::Ambiguous;
    }
    SubmitDiscrimination::CapabilityOrWorkflow
}

fn parse_host_connector_terminal(
    object: &serde_json::Map<String, Value>,
    kind: &str,
) -> SubmitDiscrimination {
    let command_id = match object.get("command_id").and_then(Value::as_str) {
        Some(command_id) if !command_id.trim().is_empty() => command_id.to_string(),
        _ => {
            return SubmitDiscrimination::Invalid(
                "host connector terminal requires a non-empty command_id".to_string(),
            );
        }
    };
    let session_id = match object.get("session_id").and_then(Value::as_str) {
        Some(session_id) if !session_id.trim().is_empty() => session_id.to_string(),
        _ => {
            return SubmitDiscrimination::Invalid(
                "host connector terminal requires a non-empty session_id".to_string(),
            );
        }
    };
    let lifecycle_event = if kind == "deadline_fired" {
        "host_connector_timeout".to_string()
    } else {
        match object.get("result_class").and_then(Value::as_str) {
            Some("succeeded") => "host_connector_succeeded".to_string(),
            Some("failed") => "host_connector_failed".to_string(),
            Some("cancelled") => "host_connector_cancelled".to_string(),
            Some("timeout") => "host_connector_timeout".to_string(),
            Some(other) => {
                return SubmitDiscrimination::Invalid(format!(
                    "unsupported host_connector_result result_class '{other}'"
                ));
            }
            None => {
                return SubmitDiscrimination::Invalid(
                    "host_connector_result requires result_class".to_string(),
                );
            }
        }
    };
    let payload = object
        .get("payload")
        .cloned()
        .or_else(|| {
            object
                .get("artifact_ref")
                .map(|artifact_ref| json!({ "artifact_ref": artifact_ref }))
        })
        .unwrap_or_else(|| json!({}));
    SubmitDiscrimination::HostConnectorTerminal(HostConnectorTerminal {
        command_id,
        session_id,
        lifecycle_event,
        payload,
    })
}

pub(crate) fn parse_state_machine(value: &Value) -> Result<AppStateMachine, String> {
    let initial_state = value
        .get("initial_state")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "state_machine.initial_state is required".to_string())?
        .to_string();
    let states_value = value
        .get("states")
        .and_then(Value::as_array)
        .ok_or_else(|| "state_machine.states is required".to_string())?;
    let mut states = BTreeMap::new();
    for state in states_value {
        let id = state
            .get("id")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| "state_machine state id is required".to_string())?
            .to_string();
        let invoke = match state.get("invoke") {
            None | Some(Value::Null) => None,
            Some(invoke) => Some(parse_invoke(invoke)?),
        };
        let transitions = state
            .get("transitions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|transition| {
                Ok(AppTransition {
                    on: transition
                        .get("on")
                        .and_then(Value::as_str)
                        .ok_or_else(|| "transition.on is required".to_string())?
                        .to_string(),
                    to: transition
                        .get("to")
                        .and_then(Value::as_str)
                        .ok_or_else(|| "transition.to is required".to_string())?
                        .to_string(),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        if states
            .insert(
                id.clone(),
                AppState {
                    invoke,
                    transitions,
                },
            )
            .is_some()
        {
            return Err(format!("duplicate state id '{id}'"));
        }
    }
    if !states.contains_key(&initial_state) {
        return Err(format!(
            "initial_state '{initial_state}' is not declared in states"
        ));
    }
    Ok(AppStateMachine {
        initial_state,
        states,
    })
}

fn parse_invoke(value: &Value) -> Result<AppInvoke, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "invoke must be an object".to_string())?;
    let capability_id = object
        .get("capability_id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty());
    let host_connector = object
        .get("host_connector")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty());
    match (capability_id, host_connector) {
        (Some(_), Some(_)) => {
            Err("invoke must not declare both capability_id and host_connector".to_string())
        }
        (Some(capability_id), None) => Ok(AppInvoke::Capability {
            capability_id: capability_id.to_string(),
            input_from: object
                .get("input_from")
                .and_then(Value::as_str)
                .unwrap_or("command.payload")
                .to_string(),
        }),
        (None, Some(command)) => Ok(AppInvoke::HostConnector {
            command: command.to_string(),
        }),
        (None, None) => Err("invoke requires capability_id or host_connector".to_string()),
    }
}

pub(crate) fn resolve_command_transition(
    machine: &AppStateMachine,
    current_state: &str,
    command: &str,
) -> Result<String, String> {
    let state = machine
        .states
        .get(current_state)
        .ok_or_else(|| format!("unknown current state '{current_state}'"))?;
    if state.wait_blocked() {
        // Invoke waits are advanced by terminal events, not UI commands.
        if state.invoke.is_some() {
            return Err(format!(
                "session is waiting in state '{current_state}' and does not accept command '{command}'"
            ));
        }
    }
    state
        .transitions
        .iter()
        .find(|transition| transition.on == command)
        .map(|transition| transition.to.clone())
        .ok_or_else(|| format!("no transition for command '{command}' from '{current_state}'"))
}

impl AppState {
    fn wait_blocked(&self) -> bool {
        self.invoke.is_some()
    }
}

pub(crate) fn resolve_lifecycle_transition(
    machine: &AppStateMachine,
    current_state: &str,
    lifecycle_event: &str,
) -> Result<Option<String>, String> {
    let state = machine
        .states
        .get(current_state)
        .ok_or_else(|| format!("unknown current state '{current_state}'"))?;
    Ok(state
        .transitions
        .iter()
        .find(|transition| transition.on == lifecycle_event)
        .map(|transition| transition.to.clone()))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

    use super::{
        SubmitDiscrimination, discriminate_submit, parse_state_machine, resolve_capability_input,
        resolve_command_transition, resolve_lifecycle_transition,
    };
    use serde_json::json;

    #[test]
    fn discriminates_app_command_from_capability_submit() {
        match discriminate_submit(&json!({
            "kind": "app_command",
            "command": "submit",
            "payload": {"x": 1}
        })) {
            SubmitDiscrimination::AppCommand(envelope) => {
                assert_eq!(envelope.command, "submit");
                assert_eq!(envelope.payload["x"], 1);
            }
            other => panic!("unexpected {other:?}"),
        }

        match discriminate_submit(&json!({"target_id": "demo.cap", "input": {}})) {
            SubmitDiscrimination::CapabilityOrWorkflow => {}
            other => panic!("unexpected {other:?}"),
        }

        assert!(matches!(
            discriminate_submit(&json!({
                "kind": "app_command",
                "target_id": "demo",
                "command": "submit"
            })),
            SubmitDiscrimination::Ambiguous
        ));
    }

    #[test]
    fn parses_and_transitions_capability_machine() {
        let machine = parse_state_machine(&json!({
            "initial_state": "idle",
            "states": [
                {
                    "id": "idle",
                    "transitions": [{ "on": "submit", "to": "processing" }]
                },
                {
                    "id": "processing",
                    "invoke": {
                        "capability_id": "demo.cap",
                        "input_from": "command.payload"
                    },
                    "transitions": [
                        { "on": "capability_succeeded", "to": "done" },
                        { "on": "capability_failed", "to": "error" }
                    ]
                },
                { "id": "done", "transitions": [] },
                { "id": "error", "transitions": [] }
            ]
        }))
        .expect("machine parses");
        assert_eq!(
            resolve_command_transition(&machine, "idle", "submit").expect("transition"),
            "processing"
        );
        assert_eq!(
            resolve_lifecycle_transition(&machine, "processing", "capability_succeeded")
                .expect("ok")
                .expect("matched"),
            "done"
        );
    }

    #[test]
    fn resolves_command_payload_input_from() {
        let payload = json!({"n": 1});
        let resolved = resolve_capability_input("command.payload", &payload, None)
            .expect("command.payload always resolves");
        assert_eq!(resolved, payload);
    }

    #[test]
    fn resolves_host_connector_result_field_wrapped_under_its_own_key() {
        let result = json!({"artifact_ref": "audio-ref-1", "artifact_base64": "QUFB"});
        let resolved = resolve_capability_input(
            "host_connector_result.artifact_base64",
            &json!({}),
            Some(&result),
        )
        .expect("field is present");
        assert_eq!(resolved, json!({"artifact_base64": "QUFB"}));

        // A different field name is wrapped under that same different name —
        // the runtime never hardcodes a semantic field name (Decision 99).
        let resolved = resolve_capability_input(
            "host_connector_result.artifact_ref",
            &json!({}),
            Some(&result),
        )
        .expect("field is present");
        assert_eq!(resolved, json!({"artifact_ref": "audio-ref-1"}));
    }

    #[test]
    fn fails_closed_when_no_host_connector_wait_has_completed() {
        let error = resolve_capability_input("host_connector_result.artifact_base64", &json!({}), None)
            .expect_err("no prior wait");
        assert_eq!(error.code, "invalid_input");
        assert!(error.message.contains("no host-connector wait"));
    }

    #[test]
    fn fails_closed_when_the_referenced_field_is_absent() {
        let result = json!({"permission_state": "granted"});
        let error = resolve_capability_input(
            "host_connector_result.artifact_base64",
            &json!({}),
            Some(&result),
        )
        .expect_err("field absent");
        assert_eq!(error.code, "invalid_input");
        assert!(error.message.contains("artifact_base64"));
    }

    #[test]
    fn fails_closed_on_an_unrecognized_input_from_literal() {
        let error = resolve_capability_input("garbage", &json!({}), None)
            .expect_err("unrecognized literal never silently uses command.payload");
        assert_eq!(error.code, "invalid_input_from");

        let error = resolve_capability_input("host_connector_result.", &json!({}), None)
            .expect_err("empty field name is rejected");
        assert_eq!(error.code, "invalid_input_from");
    }
}
