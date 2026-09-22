//! Spec 139 / Spec 052 FR-013–FR-015, FR-021: invoke-wait unhappy-route and
//! `input_from` validation.
//!
//! Walks raw application-manifest JSON so `host_connector` is visible even
//! when the published `traverse-registry` invoke schema only materializes
//! `capability_id` (Decision 96; registry follow-up tracks the schema bump).

use serde_json::Value;
use std::collections::{HashMap, HashSet, VecDeque};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InvokeRouteViolation {
    pub(crate) code: &'static str,
    pub(crate) path: String,
    pub(crate) message: String,
}

const CODE_MISSING_ROUTES: &str = "app_state_machine_missing_unhappy_routes";
const CODE_CONFLICTING_INVOKE: &str = "app_state_machine_conflicting_invoke";
const CODE_EMPTY_INVOKE: &str = "app_state_machine_empty_invoke";
/// Decision 99 / Spec 139 FR-021.
const CODE_UNREACHABLE_HOST_CONNECTOR_RESULT: &str =
    "app_state_machine_unreachable_host_connector_result";
/// Not its own FR, but the same authoring mistake FR-021 exists to catch:
/// an `input_from` the runtime would fail closed on at execution time
/// (Spec 139 FR-022) is better caught here, before a session ever runs.
const CODE_INVALID_INPUT_FROM: &str = "app_state_machine_invalid_input_from";
const HOST_CONNECTOR_RESULT_PREFIX: &str = "host_connector_result.";

/// Validate Spec 052 FR-011/FR-013/FR-014/FR-015 against the raw manifest.
pub(crate) fn validate_state_machine_invoke_routes(manifest: &Value) -> Vec<InvokeRouteViolation> {
    let Some(states) = manifest
        .pointer("/state_machine/states")
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };

    let mut violations = Vec::new();
    for state in states {
        let Some(state_id) = state.get("id").and_then(Value::as_str) else {
            continue;
        };
        let Some(invoke) = state.get("invoke") else {
            continue;
        };
        let Some(invoke_obj) = invoke.as_object() else {
            violations.push(InvokeRouteViolation {
                code: CODE_EMPTY_INVOKE,
                path: format!("state_machine.states.{state_id}.invoke"),
                message: "invoke must be an object".to_string(),
            });
            continue;
        };

        let has_capability = invoke_obj
            .get("capability_id")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty());
        let has_host_connector = invoke_obj
            .get("host_connector")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty());

        if has_capability && has_host_connector {
            violations.push(InvokeRouteViolation {
                code: CODE_CONFLICTING_INVOKE,
                path: format!("state_machine.states.{state_id}.invoke"),
                message: "invoke must not declare both capability_id and host_connector"
                    .to_string(),
            });
            continue;
        }
        if !has_capability && !has_host_connector {
            violations.push(InvokeRouteViolation {
                code: CODE_EMPTY_INVOKE,
                path: format!("state_machine.states.{state_id}.invoke"),
                message: "invoke requires capability_id or host_connector".to_string(),
            });
            continue;
        }

        let transition_ons = state
            .get("transitions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|transition| transition.get("on").and_then(Value::as_str))
            .collect::<Vec<_>>();

        let required: &[&str] = if has_host_connector {
            &[
                "host_connector_succeeded",
                "host_connector_failed",
                "host_connector_timeout",
                "host_connector_cancelled",
            ]
        } else {
            &["capability_succeeded", "capability_failed"]
        };

        let missing = required
            .iter()
            .copied()
            .filter(|event| !transition_ons.iter().any(|on| on == event))
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            violations.push(InvokeRouteViolation {
                code: CODE_MISSING_ROUTES,
                path: format!("state_machine.states.{state_id}.transitions"),
                message: format!(
                    "invoke wait is missing required unhappy-route transitions: {}",
                    missing.join(", ")
                ),
            });
        }
    }
    append_host_connector_result_violations(states, &mut violations);
    violations
}

/// Decision 99 / Spec 139 FR-021: a capability step whose `input_from`
/// references `host_connector_result` must have at least one predecessor
/// state, reachable by walking `transitions[].to` edges backward, that
/// declares `invoke.host_connector`. Runs after the main pass and skips any
/// state that pass already flagged as conflicting or empty, so one broken
/// state does not produce two overlapping violations.
fn append_host_connector_result_violations(
    states: &[Value],
    violations: &mut Vec<InvokeRouteViolation>,
) {
    let mut reverse_edges: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut declares_host_connector: HashMap<&str, bool> = HashMap::new();
    for state in states {
        let Some(id) = state.get("id").and_then(Value::as_str) else {
            continue;
        };
        let has_host_connector = state
            .get("invoke")
            .and_then(Value::as_object)
            .and_then(|invoke| invoke.get("host_connector"))
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty());
        declares_host_connector.insert(id, has_host_connector);
        for transition in state
            .get("transitions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(to) = transition.get("to").and_then(Value::as_str) {
                reverse_edges.entry(to).or_default().push(id);
            }
        }
    }

    for state in states {
        let Some(id) = state.get("id").and_then(Value::as_str) else {
            continue;
        };
        let Some(invoke_obj) = state.get("invoke").and_then(Value::as_object) else {
            continue;
        };
        let has_capability = invoke_obj
            .get("capability_id")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty());
        let has_host_connector = invoke_obj
            .get("host_connector")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty());
        // A conflicting or empty invoke was already flagged by the main pass.
        if !has_capability || has_host_connector {
            continue;
        }
        let input_from = invoke_obj
            .get("input_from")
            .and_then(Value::as_str)
            .unwrap_or("command.payload");
        if input_from == "command.payload" {
            continue;
        }
        let Some(_field) = input_from
            .strip_prefix(HOST_CONNECTOR_RESULT_PREFIX)
            .filter(|field| !field.trim().is_empty())
        else {
            violations.push(InvokeRouteViolation {
                code: CODE_INVALID_INPUT_FROM,
                path: format!("state_machine.states.{id}.invoke.input_from"),
                message: format!("unsupported invoke.input_from '{input_from}'"),
            });
            continue;
        };

        let mut visited: HashSet<&str> = HashSet::new();
        let mut queue: VecDeque<&str> = reverse_edges.get(id).cloned().unwrap_or_default().into();
        let mut reachable = false;
        while let Some(predecessor) = queue.pop_front() {
            if !visited.insert(predecessor) {
                continue;
            }
            if *declares_host_connector.get(predecessor).unwrap_or(&false) {
                reachable = true;
                break;
            }
            for next in reverse_edges.get(predecessor).into_iter().flatten() {
                queue.push_back(next);
            }
        }
        if !reachable {
            violations.push(InvokeRouteViolation {
                code: CODE_UNREACHABLE_HOST_CONNECTOR_RESULT,
                path: format!("state_machine.states.{id}.invoke.input_from"),
                message: format!(
                    "input_from '{input_from}' references host_connector_result but no reachable predecessor state declares invoke.host_connector"
                ),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::validate_state_machine_invoke_routes;
    use serde_json::json;

    #[test]
    fn capability_invoke_requires_succeeded_and_failed_routes() {
        let manifest = json!({
            "state_machine": {
                "initial_state": "idle",
                "states": [{
                    "id": "processing",
                    "invoke": {
                        "capability_id": "demo.cap",
                        "input_from": "command.payload"
                    },
                    "transitions": [{ "on": "capability_succeeded", "to": "done" }]
                }]
            }
        });
        let violations = validate_state_machine_invoke_routes(&manifest);
        assert_eq!(violations.len(), 1);
        assert_eq!(
            violations[0].code,
            "app_state_machine_missing_unhappy_routes"
        );
        assert!(violations[0].message.contains("capability_failed"));
    }

    #[test]
    fn host_connector_invoke_requires_full_terminal_set() {
        let manifest = json!({
            "state_machine": {
                "initial_state": "idle",
                "states": [{
                    "id": "capturing",
                    "invoke": { "host_connector": "capture_audio" },
                    "transitions": [
                        { "on": "host_connector_succeeded", "to": "done" },
                        { "on": "host_connector_failed", "to": "error" }
                    ]
                }]
            }
        });
        let violations = validate_state_machine_invoke_routes(&manifest);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].message.contains("host_connector_timeout"));
        assert!(violations[0].message.contains("host_connector_cancelled"));
    }

    #[test]
    fn host_connector_result_input_from_with_a_reachable_predecessor_is_accepted() {
        let manifest = json!({
            "state_machine": {
                "initial_state": "idle",
                "states": [
                    {
                        "id": "capturing",
                        "invoke": { "host_connector": "capture_audio" },
                        "transitions": [
                            { "on": "host_connector_succeeded", "to": "recorded" },
                            { "on": "host_connector_failed", "to": "error" },
                            { "on": "host_connector_timeout", "to": "error" },
                            { "on": "host_connector_cancelled", "to": "error" }
                        ]
                    },
                    { "id": "recorded", "transitions": [{ "on": "analyze", "to": "analyzing" }] },
                    {
                        "id": "analyzing",
                        "invoke": {
                            "capability_id": "demo.analyze",
                            "input_from": "host_connector_result.artifact_base64"
                        },
                        "transitions": [
                            { "on": "capability_succeeded", "to": "done" },
                            { "on": "capability_failed", "to": "error" }
                        ]
                    },
                    { "id": "done", "transitions": [] },
                    { "id": "error", "transitions": [] }
                ]
            }
        });
        assert_eq!(validate_state_machine_invoke_routes(&manifest), Vec::new());
    }

    #[test]
    fn host_connector_result_input_from_without_any_predecessor_is_rejected() {
        let manifest = json!({
            "state_machine": {
                "initial_state": "idle",
                "states": [
                    {
                        "id": "idle",
                        "transitions": [{ "on": "analyze", "to": "analyzing" }]
                    },
                    {
                        "id": "analyzing",
                        "invoke": {
                            "capability_id": "demo.analyze",
                            "input_from": "host_connector_result.artifact_base64"
                        },
                        "transitions": [
                            { "on": "capability_succeeded", "to": "done" },
                            { "on": "capability_failed", "to": "error" }
                        ]
                    },
                    { "id": "done", "transitions": [] },
                    { "id": "error", "transitions": [] }
                ]
            }
        });
        let violations = validate_state_machine_invoke_routes(&manifest);
        assert_eq!(violations.len(), 1);
        assert_eq!(
            violations[0].code,
            "app_state_machine_unreachable_host_connector_result"
        );
        assert_eq!(
            violations[0].path,
            "state_machine.states.analyzing.invoke.input_from"
        );
    }

    #[test]
    fn host_connector_result_input_from_through_an_unrelated_predecessor_is_still_rejected() {
        // "analyzing" is reachable from "idle" only through a plain command
        // transition, not through any state that declares host_connector.
        let manifest = json!({
            "state_machine": {
                "initial_state": "idle",
                "states": [
                    {
                        "id": "idle",
                        "transitions": [{ "on": "go", "to": "unrelated" }]
                    },
                    {
                        "id": "unrelated",
                        "transitions": [{ "on": "analyze", "to": "analyzing" }]
                    },
                    {
                        "id": "analyzing",
                        "invoke": {
                            "capability_id": "demo.analyze",
                            "input_from": "host_connector_result.artifact_base64"
                        },
                        "transitions": [
                            { "on": "capability_succeeded", "to": "done" },
                            { "on": "capability_failed", "to": "error" }
                        ]
                    },
                    { "id": "done", "transitions": [] },
                    { "id": "error", "transitions": [] }
                ]
            }
        });
        let violations = validate_state_machine_invoke_routes(&manifest);
        assert_eq!(
            violations[0].code,
            "app_state_machine_unreachable_host_connector_result"
        );
    }

    #[test]
    fn unrecognized_input_from_literal_is_rejected() {
        let manifest = json!({
            "state_machine": {
                "initial_state": "idle",
                "states": [{
                    "id": "processing",
                    "invoke": {
                        "capability_id": "demo.cap",
                        "input_from": "something_else"
                    },
                    "transitions": [
                        { "on": "capability_succeeded", "to": "done" },
                        { "on": "capability_failed", "to": "error" }
                    ]
                }]
            }
        });
        let violations = validate_state_machine_invoke_routes(&manifest);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].code, "app_state_machine_invalid_input_from");
    }

    #[test]
    fn a_conflicting_invoke_is_not_also_flagged_for_input_from() {
        // The main pass already rejects this; the second pass must not add
        // a redundant/confusing second violation on the same broken state.
        let manifest = json!({
            "state_machine": {
                "initial_state": "idle",
                "states": [{
                    "id": "processing",
                    "invoke": {
                        "capability_id": "demo.cap",
                        "host_connector": "capture_audio",
                        "input_from": "host_connector_result.artifact_base64"
                    },
                    "transitions": []
                }]
            }
        });
        let violations = validate_state_machine_invoke_routes(&manifest);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].code, "app_state_machine_conflicting_invoke");
    }

    #[test]
    fn conflicting_invoke_forms_are_rejected() {
        let manifest = json!({
            "state_machine": {
                "initial_state": "idle",
                "states": [{
                    "id": "processing",
                    "invoke": {
                        "capability_id": "demo.cap",
                        "host_connector": "capture_audio",
                        "input_from": "command.payload"
                    },
                    "transitions": []
                }]
            }
        });
        let violations = validate_state_machine_invoke_routes(&manifest);
        assert_eq!(violations[0].code, "app_state_machine_conflicting_invoke");
    }
}
