//! Spec 139 / Spec 052 FR-013–FR-015: invoke-wait unhappy-route validation.
//!
//! Walks raw application-manifest JSON so `host_connector` is visible even
//! when the published `traverse-registry` invoke schema only materializes
//! `capability_id` (Decision 96; registry follow-up tracks the schema bump).

use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InvokeRouteViolation {
    pub(crate) code: &'static str,
    pub(crate) path: String,
    pub(crate) message: String,
}

const CODE_MISSING_ROUTES: &str = "app_state_machine_missing_unhappy_routes";
const CODE_CONFLICTING_INVOKE: &str = "app_state_machine_conflicting_invoke";
const CODE_EMPTY_INVOKE: &str = "app_state_machine_empty_invoke";

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
    violations
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
