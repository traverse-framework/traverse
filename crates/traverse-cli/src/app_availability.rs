//! Server-owned application availability (spec `133-app-availability-lifecycle`).
//!
//! Distinct from runtime execution lifecycle and the app business workflow.
//! v1 states are `loading`, `ready`, and `failed`. The only permitted
//! transitions are `loading → ready` and `loading → failed`.

use serde::Serialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

pub(crate) const AVAILABILITY_SCHEMA_VERSION: &str = "1.0.0";
pub(crate) const REASON_PERSISTED_APP_STATE_LOADED: &str = "persisted_app_state_loaded";
pub(crate) const REASON_REGISTRATION_REQUIRES_REFRESH: &str = "app_registration_requires_refresh";
pub(crate) const REASON_DECLARATION_UNMATERIALIZABLE: &str =
    "persisted_declaration_unmaterializable";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AvailabilityState {
    Loading,
    Ready,
    Failed,
}

impl AvailabilityState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Loading => "loading",
            Self::Ready => "ready",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct AvailabilityTransition {
    pub(crate) schema_version: String,
    pub(crate) workspace_id: String,
    pub(crate) app_id: String,
    pub(crate) load_attempt_id: String,
    pub(crate) from: AvailabilityState,
    pub(crate) to: AvailabilityState,
    pub(crate) timestamp: String,
    pub(crate) reason_code: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AppAvailability {
    pub(crate) app_id: String,
    pub(crate) load_attempt_id: String,
    pub(crate) current: AvailabilityState,
    pub(crate) reason_code: String,
    pub(crate) transitions: Vec<AvailabilityTransition>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct WorkspaceAppAvailability {
    apps: BTreeMap<String, AppAvailability>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AvailabilityTransitionError {
    pub(crate) code: &'static str,
    pub(crate) message: String,
}

impl WorkspaceAppAvailability {
    pub(crate) fn begin_load_attempt(&mut self, app_id: &str) -> String {
        let load_attempt_id = Uuid::new_v4().to_string();
        self.apps.insert(
            app_id.to_string(),
            AppAvailability {
                app_id: app_id.to_string(),
                load_attempt_id: load_attempt_id.clone(),
                current: AvailabilityState::Loading,
                reason_code: "load_attempt_started".to_string(),
                transitions: Vec::new(),
            },
        );
        load_attempt_id
    }

    pub(crate) fn transition(
        &mut self,
        workspace_id: &str,
        app_id: &str,
        next: AvailabilityState,
        reason_code: &str,
    ) -> Result<AvailabilityTransition, AvailabilityTransitionError> {
        let Some(app) = self.apps.get_mut(app_id) else {
            return Err(AvailabilityTransitionError {
                code: "unknown_app",
                message: "availability transition requires an active load attempt".to_string(),
            });
        };
        if !permitted_transition(app.current, next) {
            return Err(AvailabilityTransitionError {
                code: "invalid_availability_transition",
                message: format!(
                    "availability cannot move from {} to {}",
                    app.current.as_str(),
                    next.as_str()
                ),
            });
        }
        let record = AvailabilityTransition {
            schema_version: AVAILABILITY_SCHEMA_VERSION.to_string(),
            workspace_id: workspace_id.to_string(),
            app_id: app_id.to_string(),
            load_attempt_id: app.load_attempt_id.clone(),
            from: app.current,
            to: next,
            timestamp: unix_timestamp()?,
            reason_code: reason_code.to_string(),
        };
        app.current = next;
        app.reason_code = reason_code.to_string();
        app.transitions.push(record.clone());
        Ok(record)
    }

    pub(crate) fn snapshot(&self) -> Vec<AppAvailability> {
        self.apps.values().cloned().collect()
    }
}

fn permitted_transition(from: AvailabilityState, to: AvailabilityState) -> bool {
    matches!(
        (from, to),
        (
            AvailabilityState::Loading,
            AvailabilityState::Ready | AvailabilityState::Failed
        )
    )
}

fn unix_timestamp() -> Result<String, AvailabilityTransitionError> {
    let duration =
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| AvailabilityTransitionError {
                code: "clock_error",
                message: "system clock is before Unix epoch".to_string(),
            })?;
    Ok(format!("unix:{}", duration.as_secs()))
}

pub(crate) fn emit_availability_diagnostic(record: &AvailabilityTransition) {
    let envelope = json!({
        "kind": "app_availability_transition",
        "schema_version": record.schema_version,
        "workspace_id": record.workspace_id,
        "app_id": record.app_id,
        "load_attempt_id": record.load_attempt_id,
        "from": record.from,
        "to": record.to,
        "timestamp": record.timestamp,
        "reason_code": record.reason_code,
    });
    eprintln!("{envelope}");
}

pub(crate) fn availability_status_envelope(workspace_id: &str, apps: &[AppAvailability]) -> Value {
    json!({
        "api_version": "v1",
        "workspace_id": workspace_id,
        "apps": apps.iter().map(|app| json!({
            "app_id": app.app_id,
            "availability": app.current,
            "load_attempt_id": app.load_attempt_id,
            "reason_code": app.reason_code,
            "transitions": app.transitions,
        })).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn permits_only_loading_to_ready_and_loading_to_failed() {
        assert!(permitted_transition(
            AvailabilityState::Loading,
            AvailabilityState::Ready
        ));
        assert!(permitted_transition(
            AvailabilityState::Loading,
            AvailabilityState::Failed
        ));
        for from in [
            AvailabilityState::Loading,
            AvailabilityState::Ready,
            AvailabilityState::Failed,
        ] {
            for to in [
                AvailabilityState::Loading,
                AvailabilityState::Ready,
                AvailabilityState::Failed,
            ] {
                if matches!(
                    (from, to),
                    (
                        AvailabilityState::Loading,
                        AvailabilityState::Ready | AvailabilityState::Failed
                    )
                ) {
                    continue;
                }
                assert!(
                    !permitted_transition(from, to),
                    "{} → {} must be rejected",
                    from.as_str(),
                    to.as_str()
                );
            }
        }
    }

    #[test]
    fn records_ordered_secret_free_ready_transition() {
        let mut store = WorkspaceAppAvailability::default();
        store.begin_load_attempt("expedition.readiness");
        let ready = store
            .transition(
                "ws-test",
                "expedition.readiness",
                AvailabilityState::Ready,
                REASON_PERSISTED_APP_STATE_LOADED,
            )
            .expect("ready");
        assert_eq!(ready.from, AvailabilityState::Loading);
        assert_eq!(ready.to, AvailabilityState::Ready);
        assert_eq!(ready.reason_code, REASON_PERSISTED_APP_STATE_LOADED);
        let serialized = serde_json::to_string(&ready).expect("serialize");
        assert!(!serialized.contains("file://"));
        assert!(!serialized.contains("/.traverse/"));
        let failed = store.transition(
            "ws-test",
            "expedition.readiness",
            AvailabilityState::Failed,
            REASON_DECLARATION_UNMATERIALIZABLE,
        );
        assert!(failed.is_err());
        let snapshot = store.snapshot();
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].transitions.len(), 1);
        assert_eq!(snapshot[0].current, AvailabilityState::Ready);
    }

    #[test]
    fn status_envelope_lists_current_state_and_ordered_history() {
        let mut store = WorkspaceAppAvailability::default();
        store.begin_load_attempt("expedition.readiness");
        store
            .transition(
                "ws-test",
                "expedition.readiness",
                AvailabilityState::Ready,
                REASON_PERSISTED_APP_STATE_LOADED,
            )
            .expect("ready");
        let envelope = availability_status_envelope("ws-test", &store.snapshot());
        assert_eq!(envelope["api_version"], "v1");
        assert_eq!(envelope["workspace_id"], "ws-test");
        assert_eq!(envelope["apps"][0]["availability"], "ready");
        assert_eq!(envelope["apps"][0]["transitions"][0]["from"], "loading");
        assert_eq!(envelope["apps"][0]["transitions"][0]["to"], "ready");
        let rendered = envelope.to_string();
        assert!(!rendered.contains("file://"));
        assert!(!rendered.contains("/.traverse/"));
    }

    #[test]
    fn restart_creates_a_new_load_attempt_without_prior_history() {
        let mut first = WorkspaceAppAvailability::default();
        first.begin_load_attempt("app");
        first
            .transition(
                "ws-test",
                "app",
                AvailabilityState::Failed,
                REASON_DECLARATION_UNMATERIALIZABLE,
            )
            .expect("failed");
        let first_id = first.snapshot()[0].load_attempt_id.clone();

        let mut restarted = WorkspaceAppAvailability::default();
        restarted.begin_load_attempt("app");
        restarted
            .transition(
                "ws-test",
                "app",
                AvailabilityState::Ready,
                REASON_PERSISTED_APP_STATE_LOADED,
            )
            .expect("ready");
        let second = &restarted.snapshot()[0];
        assert_ne!(second.load_attempt_id, first_id);
        assert_eq!(second.transitions.len(), 1);
        assert_eq!(second.current, AvailabilityState::Ready);
    }
}
