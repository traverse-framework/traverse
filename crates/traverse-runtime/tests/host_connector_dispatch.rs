use serde_json::{Value, json};
use traverse_runtime::host_connector_dispatch::*;

/// In-process fake host for tests. Not the Spec 135 WIT recording fake.
#[derive(Debug, Default)]
#[allow(clippy::struct_excessive_bools)]
pub struct FakeHostConnector {
    invoke_count: u64,
    next_audio_id: u64,
    next_model_id: u64,
    last_request: Option<HostConnectorHostRequest>,
    deny_policy: bool,
    unavailable: bool,
    leaky: bool,
    cancel_on_invoke: bool,
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

    /// Force adapter-observed cancellation.
    pub fn cancel_on_invoke(&mut self, cancel: bool) {
        self.cancel_on_invoke = cancel;
    }

    /// Force a host-private artifact reference so dispatch must redact it.
    pub fn leak_artifact(&mut self, leaky: bool) {
        self.leaky = leaky;
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
        if self.cancel_on_invoke {
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
        if self.leaky {
            return Ok(HostConnectorHostResult {
                artifact_ref: "microphone:/tmp/capture.wav".to_string(),
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

fn model_execute_payload() -> serde_json::Value {
    json!({
        "model_ref": {
            "model_id": "fixture.model",
            "version": "1.0.0",
            "digest": "sha256:fixture"
        },
        "input_ref": "input-1",
        "policy_ref": "policy-1",
        "data_classification": "sensitive",
        "input_schema_ref": "schema:fixture-in",
        "input_schema_version": "1.0.0",
        "max_output_bytes": 4096
    })
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
        payload: model_execute_payload(),
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
    let mut forbidden_payload = model_execute_payload();
    forbidden_payload["provider"] = json!("ollama");
    forbidden.payload = forbidden_payload;
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
#[allow(clippy::too_many_lines)]
fn remaining_fail_closed_branches_are_covered() -> Result<(), String> {
    let manifest = audio_manifest();
    let activations = activated_audio();
    let mut host = FakeHostConnector::new();
    let mut idempotency = HostConnectorIdempotencyStore::new();

    let mut bad_kind = audio_command("macos");
    bad_kind.kind = "not-a-command".to_string();
    assert_eq!(
        require_err(
            &bad_kind,
            &manifest,
            &activations,
            &mut host,
            &mut idempotency
        )?
        .error
        .code,
        HostConnectorErrorCode::Incompatible
    );

    let mut empty_id = audio_command("macos");
    empty_id.command_id = "   ".to_string();
    assert_eq!(
        require_err(
            &empty_id,
            &manifest,
            &activations,
            &mut host,
            &mut idempotency
        )?
        .error
        .code,
        HostConnectorErrorCode::Incompatible
    );

    let mut huge = audio_command("macos");
    huge.payload = json!({"max_duration_ms": 5, "max_bytes": 10, "pad": "x".repeat(20_000)});
    assert_eq!(
        require_err(&huge, &manifest, &activations, &mut host, &mut idempotency)?
            .error
            .code,
        HostConnectorErrorCode::InputLimitExceeded
    );

    let mut unsupported = audio_manifest();
    unsupported.command_routes[0].operation = "audio.stream".to_string();
    assert_eq!(
        require_err(
            &audio_command("macos"),
            &unsupported,
            &activations,
            &mut host,
            &mut idempotency
        )?
        .error
        .code,
        HostConnectorErrorCode::Incompatible
    );

    let mut duplicated_route = audio_manifest();
    duplicated_route
        .command_routes
        .push(duplicated_route.command_routes[0].clone());
    assert_eq!(
        require_err(
            &audio_command("macos"),
            &duplicated_route,
            &activations,
            &mut host,
            &mut idempotency
        )?
        .error
        .code,
        HostConnectorErrorCode::Incompatible
    );

    let mut no_binding_id = audio_manifest();
    no_binding_id.connector_bindings[0].binding_id.clear();
    assert_eq!(
        require_err(
            &audio_command("macos"),
            &no_binding_id,
            &activations,
            &mut host,
            &mut idempotency
        )?
        .error
        .code,
        HostConnectorErrorCode::Unbound
    );

    let mut no_version = audio_manifest();
    no_version.connector_bindings[0].version.clear();
    assert_eq!(
        require_err(
            &audio_command("macos"),
            &no_version,
            &activations,
            &mut host,
            &mut idempotency
        )?
        .error
        .code,
        HostConnectorErrorCode::Incompatible
    );

    assert_eq!(
        require_err(
            &audio_command("ios"),
            &manifest,
            &activations,
            &mut host,
            &mut idempotency
        )?
        .error
        .code,
        HostConnectorErrorCode::TargetIncompatible
    );

    let mut not_object = audio_command("macos");
    not_object.payload = json!(["capture"]);
    assert_eq!(
        require_err(
            &not_object,
            &manifest,
            &activations,
            &mut host,
            &mut idempotency
        )?
        .error
        .code,
        HostConnectorErrorCode::InputLimitExceeded
    );

    let mut zero_bytes = audio_command("macos");
    zero_bytes.payload = json!({"max_duration_ms": 5, "max_bytes": 0});
    assert_eq!(
        require_err(
            &zero_bytes,
            &manifest,
            &activations,
            &mut host,
            &mut idempotency
        )?
        .error
        .code,
        HostConnectorErrorCode::InputLimitExceeded
    );

    let mut missing_limit = audio_command("macos");
    missing_limit.payload = json!({"max_duration_ms": 5});
    assert_eq!(
        require_err(
            &missing_limit,
            &manifest,
            &activations,
            &mut host,
            &mut idempotency
        )?
        .error
        .code,
        HostConnectorErrorCode::InputLimitExceeded
    );

    let mut negative = audio_command("macos");
    negative.payload = json!({"max_duration_ms": -1, "max_bytes": 10});
    assert_eq!(
        require_err(
            &negative,
            &manifest,
            &activations,
            &mut host,
            &mut idempotency
        )?
        .error
        .code,
        HostConnectorErrorCode::InputLimitExceeded
    );

    let combined = combined_manifest();
    let mut model_activations = activated_audio();
    model_activations.activate("default-local-model");
    let mut missing_model = model_command();
    missing_model.payload = json!({"max_output_bytes": 8});
    assert_eq!(
        require_err(
            &missing_model,
            &combined,
            &model_activations,
            &mut host,
            &mut idempotency
        )?
        .error
        .code,
        HostConnectorErrorCode::InputLimitExceeded
    );
    let mut huge_model = model_command();
    let mut zero_payload = model_execute_payload();
    zero_payload["max_output_bytes"] = json!(0);
    huge_model.payload = zero_payload;
    huge_model.idempotency_key = "idem-model-zero".to_string();
    assert_eq!(
        require_err(
            &huge_model,
            &combined,
            &model_activations,
            &mut host,
            &mut idempotency
        )?
        .error
        .code,
        HostConnectorErrorCode::InputLimitExceeded
    );

    host.set_unavailable(true);
    let failed = require_err(
        &audio_command("macos"),
        &manifest,
        &activations,
        &mut host,
        &mut idempotency,
    )?;
    assert_eq!(failed.error.code, HostConnectorErrorCode::Unavailable);
    assert!(failed.error.to_string().contains("unavailable"));
    let _: &dyn std::error::Error = &failed.error;
    assert_eq!(
        failed.dispatch.evidence.public_json()["outcome"],
        "unavailable"
    );
    host.set_unavailable(false);

    host.cancel_on_invoke(true);
    let mut cancel_cmd = audio_command("macos");
    cancel_cmd.idempotency_key = "idem-host-cancel".to_string();
    assert_eq!(
        require_err(
            &cancel_cmd,
            &manifest,
            &activations,
            &mut host,
            &mut idempotency
        )?
        .error
        .code,
        HostConnectorErrorCode::Cancelled
    );
    host.cancel_on_invoke(false);

    host.leak_artifact(true);
    let mut leak_cmd = audio_command("macos");
    leak_cmd.idempotency_key = "idem-leak".to_string();
    let leaked = require_err(
        &leak_cmd,
        &manifest,
        &activations,
        &mut host,
        &mut idempotency,
    )?;
    assert_eq!(leaked.error.code, HostConnectorErrorCode::Unavailable);
    assert_no_leak(&json!(leaked.dispatch));
    host.leak_artifact(false);

    let mut activations = HostConnectorActivationSet::new();
    activations.activate("default-local-audio");
    activations.activate("default-local-audio");
    let local = require_ok(
        &audio_command("local"),
        &manifest,
        &activations,
        &mut host,
        &mut idempotency,
    )?;
    assert_eq!(local.result_class, "succeeded");
    assert!(host.last_request().is_some());

    let unexpected = require_err(
        &{
            let mut cmd = audio_command("macos");
            cmd.idempotency_key = "idem-success-as-err".to_string();
            cmd
        },
        &manifest,
        &activations,
        &mut host,
        &mut idempotency,
    );
    assert!(unexpected.is_err());

    let mut bad_schema = audio_command("macos");
    bad_schema.schema_version = "0.0.0".to_string();
    bad_schema.idempotency_key = "idem-schema".to_string();
    assert_eq!(
        require_err(
            &bad_schema,
            &manifest,
            &activations,
            &mut host,
            &mut idempotency
        )?
        .error
        .code,
        HostConnectorErrorCode::Incompatible
    );

    for (key, value) in [
        ("command", "   "),
        ("correlation_id", ""),
        ("idempotency_key", " "),
        ("target_family", ""),
    ] {
        let mut empty = audio_command("macos");
        empty.idempotency_key = format!("idem-empty-{key}");
        match key {
            "command" => empty.command = value.to_string(),
            "correlation_id" => empty.correlation_id = value.to_string(),
            "idempotency_key" => empty.idempotency_key = value.to_string(),
            _ => empty.target_family = value.to_string(),
        }
        assert_eq!(
            require_err(&empty, &manifest, &activations, &mut host, &mut idempotency)?
                .error
                .code,
            HostConnectorErrorCode::Incompatible
        );
    }

    let mut zero_duration = audio_command("macos");
    zero_duration.payload = json!({"max_duration_ms": 0, "max_bytes": 10});
    zero_duration.idempotency_key = "idem-zero-duration".to_string();
    assert_eq!(
        require_err(
            &zero_duration,
            &manifest,
            &activations,
            &mut host,
            &mut idempotency
        )?
        .error
        .code,
        HostConnectorErrorCode::InputLimitExceeded
    );

    let mut huge_audio = audio_command("macos");
    huge_audio.payload = json!({"max_duration_ms": 5, "max_bytes": 10 * 1024 * 1024 + 1});
    huge_audio.idempotency_key = "idem-huge-audio".to_string();
    assert_eq!(
        require_err(
            &huge_audio,
            &manifest,
            &activations,
            &mut host,
            &mut idempotency
        )?
        .error
        .code,
        HostConnectorErrorCode::InputLimitExceeded
    );

    let mut missing_model_ref = model_command();
    missing_model_ref.payload = json!({
        "input_ref": "input-1",
        "policy_ref": "policy-1",
        "data_classification": "sensitive",
        "input_schema_ref": "schema:fixture-in",
        "input_schema_version": "1.0.0",
        "max_output_bytes": 8
    });
    missing_model_ref.idempotency_key = "idem-missing-model-ref".to_string();
    assert_eq!(
        require_err(
            &missing_model_ref,
            &combined,
            &model_activations,
            &mut host,
            &mut idempotency
        )?
        .error
        .code,
        HostConnectorErrorCode::InputLimitExceeded
    );
    let mut missing_policy = model_command();
    let mut missing_policy_payload = model_execute_payload();
    if let Some(object) = missing_policy_payload.as_object_mut() {
        object.remove("policy_ref");
    }
    missing_policy.payload = missing_policy_payload;
    missing_policy.idempotency_key = "idem-missing-policy".to_string();
    assert_eq!(
        require_err(
            &missing_policy,
            &combined,
            &model_activations,
            &mut host,
            &mut idempotency
        )?
        .error
        .code,
        HostConnectorErrorCode::InputLimitExceeded
    );
    let mut huge_output = model_command();
    let mut huge_payload = model_execute_payload();
    huge_payload["max_output_bytes"] = json!(16 * 1024 * 1024 + 1);
    huge_output.payload = huge_payload;
    huge_output.idempotency_key = "idem-huge-output".to_string();
    assert_eq!(
        require_err(
            &huge_output,
            &combined,
            &model_activations,
            &mut host,
            &mut idempotency
        )?
        .error
        .code,
        HostConnectorErrorCode::InputLimitExceeded
    );
    for forbidden_field in ["model_id", "endpoint", "credential", "artifact_ref"] {
        let mut forbidden = model_command();
        let mut payload = model_execute_payload();
        payload[forbidden_field] = json!("blocked");
        forbidden.payload = payload;
        forbidden.idempotency_key = format!("idem-forbidden-{forbidden_field}");
        assert_eq!(
            require_err(
                &forbidden,
                &combined,
                &model_activations,
                &mut host,
                &mut idempotency
            )?
            .error
            .code,
            HostConnectorErrorCode::Incompatible
        );
    }

    let mut macos_only = audio_manifest();
    macos_only.connector_bindings[0].placement_targets = vec!["macos".to_string()];
    let macos_ok = require_ok(
        &{
            let mut cmd = audio_command("macos");
            cmd.idempotency_key = "idem-macos-only".to_string();
            cmd
        },
        &macos_only,
        &activations,
        &mut host,
        &mut idempotency,
    )?;
    assert_eq!(macos_ok.result_class, "succeeded");
    assert_eq!(
        require_err(
            &{
                let mut cmd = audio_command("local");
                cmd.idempotency_key = "idem-macos-only-local".to_string();
                cmd
            },
            &macos_only,
            &activations,
            &mut host,
            &mut idempotency
        )?
        .error
        .code,
        HostConnectorErrorCode::TargetIncompatible
    );

    assert_eq!(bound_host_connector_event_queue(&audio_command("macos")), 8);
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
        HostConnectorErrorCode::InvalidInput,
        HostConnectorErrorCode::ModelUnavailable,
        HostConnectorErrorCode::ModelIncompatible,
        HostConnectorErrorCode::ResourceExhausted,
        HostConnectorErrorCode::Timeout,
        HostConnectorErrorCode::ExecutionFailed,
    ] {
        assert!(!code.as_str().is_empty());
        assert!(!code.as_str().contains("connector_invoke"));
    }
    // This module's fake is FakeHostConnector, not Spec 135 FakeRecordingHost.
    let _fake = FakeHostConnector::new();
    assert_eq!(COMMAND_KIND, "host_connector_command");
    assert_ne!(COMMAND_KIND, "connector_invoke");
}
