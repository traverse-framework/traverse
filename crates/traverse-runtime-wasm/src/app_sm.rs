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

#[derive(Debug, Clone)]
pub(crate) struct AppTransition {
    pub(crate) on: String,
    pub(crate) to: String,
}

#[derive(Debug, Clone)]
pub(crate) struct AppSession {
    pub(crate) session_id: String,
    pub(crate) state: String,
    pub(crate) wait: Option<AppWait>,
}

#[derive(Debug, Clone)]
pub(crate) struct AppWait {
    #[allow(dead_code)]
    pub(crate) command_id: String,
    #[allow(dead_code)]
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

#[derive(Debug, Clone)]
pub(crate) enum SubmitDiscrimination {
    AppCommand(AppCommandEnvelope),
    CapabilityOrWorkflow,
    Ambiguous,
    Invalid(String),
}

/// Fail-closed discrimination between Spec 068 workflow/capability submit and
/// Spec 139 `app_command` envelopes.
pub(crate) fn discriminate_submit(request: &Value) -> SubmitDiscrimination {
    let Some(object) = request.as_object() else {
        return SubmitDiscrimination::Invalid("submit body must be a JSON object".to_string());
    };
    let has_kind_app = object
        .get("kind")
        .and_then(Value::as_str)
        .is_some_and(|kind| kind == "app_command");
    let has_target = object.contains_key("target_id");
    let has_command = object.contains_key("command");

    if has_kind_app && has_target {
        return SubmitDiscrimination::Ambiguous;
    }
    if has_kind_app {
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
    if has_command && !has_target {
        // Bare `command` without `kind` is ambiguous with future envelopes.
        return SubmitDiscrimination::Ambiguous;
    }
    SubmitDiscrimination::CapabilityOrWorkflow
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
    use super::{
        SubmitDiscrimination, discriminate_submit, parse_state_machine, resolve_command_transition,
        resolve_lifecycle_transition,
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
}
