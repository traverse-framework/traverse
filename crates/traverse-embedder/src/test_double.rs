//! Deterministic in-memory test double for the embedder boundary
//! (spec 068 FR-006).

use crate::{
    CompatibleLifecycleOutcome, CompatibleStartOutcome, EmbedderCore, EmbedderError,
    EmbedderErrorCode, EventCallback, InstanceState, ShutdownOutcome, SubmitOutcome, SubmitStatus,
    TraverseEmbedderApi,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// A scripted submit result for one target id.
#[derive(Debug, Clone)]
enum ScriptedResult {
    Output(Value),
    Error { code: String, message: String },
}

/// Deterministic test double implementing [`TraverseEmbedderApi`].
///
/// The double shares the production embedder's event envelope, identifier
/// scheme, compatible-capability lifecycle, and shutdown semantics through
/// the same internal core; only capability execution is replaced with
/// scripted results. It contains no business logic and never replaces
/// runtime-owned behavior in production code paths.
pub struct EmbedderTestDouble {
    core: EmbedderCore,
    scripted: BTreeMap<String, ScriptedResult>,
}

impl EmbedderTestDouble {
    /// Creates a double with the given identity, no scripted targets, and
    /// no compatible capabilities.
    #[must_use]
    pub fn new(
        workspace_id: impl Into<String>,
        app_id: impl Into<String>,
        app_version: impl Into<String>,
        platform: impl Into<String>,
    ) -> Self {
        Self {
            core: EmbedderCore::new(
                workspace_id.into(),
                app_id.into(),
                app_version.into(),
                platform.into(),
                BTreeMap::new(),
            ),
            scripted: BTreeMap::new(),
        }
    }

    /// Scripts `submit(target_id, _)` to succeed with `output`.
    #[must_use]
    pub fn with_target_output(mut self, target_id: impl Into<String>, output: Value) -> Self {
        self.scripted
            .insert(target_id.into(), ScriptedResult::Output(output));
        self
    }

    /// Scripts `submit(target_id, _)` to fail with a runtime-shaped error.
    #[must_use]
    pub fn with_target_error(
        mut self,
        target_id: impl Into<String>,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        self.scripted.insert(
            target_id.into(),
            ScriptedResult::Error {
                code: code.into(),
                message: message.into(),
            },
        );
        self
    }

    /// Declares a compatible-mode capability with a platform allowlist.
    #[must_use]
    pub fn with_compatible_target(
        mut self,
        capability_id: impl Into<String>,
        platforms: Vec<String>,
    ) -> Self {
        self.core
            .compatible_targets
            .insert(capability_id.into(), platforms);
        self
    }
}

impl TraverseEmbedderApi for EmbedderTestDouble {
    fn submit(&mut self, target_id: &str, input: &Value) -> SubmitOutcome {
        let _ = input;
        if self.core.stopped {
            let error = crate::runtime_stopped_error();
            return self.core.rejected_submit(target_id, error);
        }
        let Some(result) = self.scripted.get(target_id).cloned() else {
            let error = EmbedderError::new(
                EmbedderErrorCode::TargetNotFound,
                format!("'{target_id}' is neither a bundled workflow nor a bundled capability"),
            );
            return self.core.rejected_submit(target_id, error);
        };

        let session_id = self.core.next_session_id();
        let request_id = self.core.next_request_id();
        let execution_id = format!("exec_{request_id}");
        self.core.emit(
            "capability_invoked",
            Some(&session_id),
            json!({
                "execution_id": execution_id,
                "capability_id": target_id,
                "capability_version": "1.0.0",
            }),
        );
        match result {
            ScriptedResult::Output(output) => {
                self.core.emit(
                    "capability_result",
                    Some(&session_id),
                    json!({
                        "execution_id": execution_id,
                        "capability_id": target_id,
                        "status": "completed",
                        "output": output,
                    }),
                );
            }
            ScriptedResult::Error { code, message } => {
                self.core.emit(
                    "error",
                    Some(&session_id),
                    json!({
                        "execution_id": execution_id,
                        "capability_id": target_id,
                        "status": "error",
                        "error": { "code": code, "message": message, "details": {} },
                    }),
                );
            }
        }
        SubmitOutcome {
            session_id: Some(session_id),
            status: SubmitStatus::Accepted,
            error: None,
        }
    }

    fn subscribe(&mut self, callback: EventCallback) {
        self.core.subscribe(callback);
    }

    fn start_compatible(&mut self, capability_id: &str, input: &Value) -> CompatibleStartOutcome {
        self.core.start_compatible(capability_id, input)
    }

    fn stop_compatible(
        &mut self,
        capability_id: &str,
        instance_id: Option<&str>,
    ) -> CompatibleLifecycleOutcome {
        self.core
            .transition_compatible(capability_id, instance_id, InstanceState::Stopped)
    }

    fn kill_compatible(
        &mut self,
        capability_id: &str,
        instance_id: Option<&str>,
    ) -> CompatibleLifecycleOutcome {
        self.core
            .transition_compatible(capability_id, instance_id, InstanceState::Killed)
    }

    fn shutdown(&mut self) -> ShutdownOutcome {
        self.core.shutdown()
    }

    fn release_evidence(&self) -> Value {
        self.core.evidence("test-double", json!([]))
    }
}
