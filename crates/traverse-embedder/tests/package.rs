//! Linux/CLI package tests: init rejection paths, submit boundary errors,
//! compatible lifecycle edge cases, event replay, release evidence, and the
//! deterministic test double (spec 068 FR-006..FR-008, NFR-001, NFR-002).
#![allow(clippy::expect_used)]

mod common;

use common::{
    BundleFixture, FixtureOptions, PROCESS_CAPABILITY_ID, RENDER_CAPABILITY_ID, collect_events,
    snapshot,
};
use serde_json::json;
use traverse_embedder::{
    BundleEmbedder, CompatibleLifecycleStatus, EMBEDDER_API_VERSION, EMBEDDER_CONFORMANCE_VERSION,
    EmbedderConfig, EmbedderErrorCode, EmbedderTestDouble, SecurityPosture, SubmitStatus,
    TraverseEmbedderApi,
};

fn development_embedder(fixture: &BundleFixture, platform: &str) -> BundleEmbedder {
    let mut config = EmbedderConfig::new(fixture.manifest_path());
    config.platform = platform.to_string();
    config.security = SecurityPosture::Development;
    BundleEmbedder::init(config).expect("fixture bundle should initialize")
}

// --- init rejection paths (NFR-001) ---

#[test]
fn init_rejects_missing_bundle_deterministically() {
    let error = BundleEmbedder::init(EmbedderConfig::new("/nonexistent/app.manifest.json"))
        .err()
        .expect("missing bundle should be rejected");
    assert_eq!(error.code, EmbedderErrorCode::BundleLoadFailed);
    assert!(error.message.contains("application bundle failed to load"));
}

#[test]
fn init_rejects_bundle_that_fails_registration() {
    let options = FixtureOptions {
        emits_missing_event: true,
        ..FixtureOptions::default()
    };
    let fixture = BundleFixture::with_options("register-reject", &options);
    let mut config = EmbedderConfig::new(fixture.manifest_path());
    config.security = SecurityPosture::Development;
    let error = BundleEmbedder::init(config)
        .err()
        .expect("missing event reference should be rejected");
    assert_eq!(error.code, EmbedderErrorCode::BundleLoadFailed);
    assert!(error.message.contains("failed to register"));
    assert!(error.message.contains("fixture.missing-event"));
}

#[test]
fn init_rejects_conflicting_duplicate_capability_contracts() {
    let options = FixtureOptions {
        duplicate_capability_conflict: true,
        ..FixtureOptions::default()
    };
    let fixture = BundleFixture::with_options("conflict-reject", &options);
    let mut config = EmbedderConfig::new(fixture.manifest_path());
    config.security = SecurityPosture::Development;
    let error = BundleEmbedder::init(config)
        .err()
        .expect("conflicting duplicate capability should be rejected");
    assert_eq!(error.code, EmbedderErrorCode::BundleLoadFailed);
}

// --- submit boundary errors ---

#[test]
fn submit_rejects_unknown_targets_with_error_event() {
    let fixture = BundleFixture::new("unknown-target");
    let mut embedder = development_embedder(&fixture, "linux");
    let events = collect_events(&mut embedder);

    let outcome = embedder.submit("fixture.unknown", &json!({}));
    assert_eq!(outcome.status, SubmitStatus::Rejected);
    assert_eq!(outcome.session_id, None);
    assert_eq!(
        outcome.error.expect("unknown target should error").code,
        EmbedderErrorCode::TargetNotFound,
    );

    let events = snapshot(&events);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["event_type"], "error");
    assert_eq!(events[0]["data"]["target_id"], "fixture.unknown");
    assert_eq!(events[0]["data"]["error"]["code"], "target_not_found");
}

#[test]
fn submit_rejects_compatible_targets_toward_lifecycle_operations() {
    let fixture = BundleFixture::new("compatible-submit");
    let mut embedder = development_embedder(&fixture, "linux");

    let outcome = embedder.submit(RENDER_CAPABILITY_ID, &json!({}));
    assert_eq!(outcome.status, SubmitStatus::Rejected);
    assert_eq!(
        outcome.error.expect("compatible submit should error").code,
        EmbedderErrorCode::CompatibleLifecycleRequired,
    );
}

#[test]
fn production_posture_surfaces_unsigned_artifact_errors_as_events() {
    let fixture = BundleFixture::new("production-posture");
    let config = EmbedderConfig::new(fixture.manifest_path());
    assert_eq!(config.security, SecurityPosture::Production);
    assert_eq!(config.workspace_id, "local-default");
    let mut config = config;
    config.platform = "linux".to_string();
    let mut embedder =
        BundleEmbedder::init(config).expect("bundle should initialize in production posture");
    let events = collect_events(&mut embedder);

    let outcome = embedder.submit(PROCESS_CAPABILITY_ID, &json!({ "note": "unsigned" }));
    assert_eq!(outcome.status, SubmitStatus::Accepted);

    let events = snapshot(&events);
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["event_type"], "capability_invoked");
    assert_eq!(events[1]["event_type"], "error");
    assert_eq!(events[1]["data"]["status"], "error");
    assert!(
        events[1]["data"]["error"]["code"].is_string(),
        "runtime error code must be structured"
    );
}

#[test]
fn workflow_error_results_surface_as_error_events() {
    let fixture = BundleFixture::new("workflow-error");
    let mut embedder = development_embedder(&fixture, "linux");
    let events = collect_events(&mut embedder);

    // The fixture capability contract requires a `note` string, so this
    // input fails the runtime's step-input contract validation.
    let outcome = embedder.submit(common::PIPELINE_WORKFLOW_ID, &json!({ "wrong": true }));
    assert_eq!(outcome.status, SubmitStatus::Accepted);

    let events = snapshot(&events);
    let terminal = events.last().expect("workflow should emit terminal event");
    assert_eq!(terminal["event_type"], "error");
    assert_eq!(
        terminal["data"]["workflow_id"],
        common::PIPELINE_WORKFLOW_ID
    );
    assert_eq!(terminal["data"]["status"], "error");
    assert!(terminal["data"]["error"]["code"].is_string());
}

// --- compatible lifecycle edge cases ---

#[test]
fn compatible_lifecycle_rejects_unknown_capabilities_and_instances() {
    let fixture = BundleFixture::new("compatible-edges");
    let mut embedder = development_embedder(&fixture, "linux");

    let unknown = embedder.start_compatible(PROCESS_CAPABILITY_ID, &json!({}));
    assert_eq!(unknown.status, CompatibleLifecycleStatus::Error);
    assert_eq!(
        unknown.error.expect("wasm capability should error").code,
        EmbedderErrorCode::CapabilityNotCompatible,
    );

    let missing = embedder.stop_compatible(RENDER_CAPABILITY_ID, Some("inst-99999999"));
    assert_eq!(missing.status, CompatibleLifecycleStatus::Error);
    assert_eq!(
        missing.error.expect("missing instance should error").code,
        EmbedderErrorCode::InstanceNotFound,
    );

    let none_running = embedder.kill_compatible(RENDER_CAPABILITY_ID, None);
    assert_eq!(none_running.status, CompatibleLifecycleStatus::Error);
    assert_eq!(
        none_running
            .error
            .expect("no running instances should error")
            .code,
        EmbedderErrorCode::InstanceNotRunning,
    );

    let started = embedder.start_compatible(RENDER_CAPABILITY_ID, &json!({}));
    let instance_id = started.instance_id.expect("instance should start");
    let stopped = embedder.stop_compatible(RENDER_CAPABILITY_ID, Some(&instance_id));
    assert_eq!(stopped.status, CompatibleLifecycleStatus::Stopped);
    let stopped_again = embedder.stop_compatible(RENDER_CAPABILITY_ID, Some(&instance_id));
    assert_eq!(stopped_again.status, CompatibleLifecycleStatus::Error);
    assert_eq!(
        stopped_again
            .error
            .expect("stopped instance should not stop again")
            .code,
        EmbedderErrorCode::InstanceNotRunning,
    );

    let wrong_capability = embedder.kill_compatible(PROCESS_CAPABILITY_ID, Some(&instance_id));
    assert_eq!(wrong_capability.status, CompatibleLifecycleStatus::Error);
    assert_eq!(
        wrong_capability
            .error
            .expect("instance of another capability should not match")
            .code,
        EmbedderErrorCode::InstanceNotFound,
    );
}

#[test]
fn compatible_kill_terminates_every_running_instance_of_a_capability() {
    let fixture = BundleFixture::new("compatible-kill-all");
    let mut embedder = development_embedder(&fixture, "linux");

    let first = embedder.start_compatible(RENDER_CAPABILITY_ID, &json!({}));
    let second = embedder.start_compatible(RENDER_CAPABILITY_ID, &json!({}));
    assert_ne!(first.instance_id, second.instance_id);

    let killed = embedder.kill_compatible(RENDER_CAPABILITY_ID, None);
    assert_eq!(killed.status, CompatibleLifecycleStatus::Killed);
    let none_left = embedder.stop_compatible(RENDER_CAPABILITY_ID, None);
    assert_eq!(none_left.status, CompatibleLifecycleStatus::Error);
}

#[test]
fn lifecycle_operations_after_shutdown_are_rejected() {
    let fixture = BundleFixture::new("stopped-lifecycle");
    let mut embedder = development_embedder(&fixture, "linux");
    embedder.shutdown();

    let start = embedder.start_compatible(RENDER_CAPABILITY_ID, &json!({}));
    assert_eq!(
        start.error.expect("stopped start should error").code,
        EmbedderErrorCode::RuntimeStopped,
    );
    let stop = embedder.stop_compatible(RENDER_CAPABILITY_ID, None);
    assert_eq!(
        stop.error.expect("stopped stop should error").code,
        EmbedderErrorCode::RuntimeStopped,
    );
    let kill = embedder.kill_compatible(RENDER_CAPABILITY_ID, None);
    assert_eq!(
        kill.error.expect("stopped kill should error").code,
        EmbedderErrorCode::RuntimeStopped,
    );
}

// --- subscription replay and release evidence ---

#[test]
fn late_subscribers_replay_the_identical_ordered_stream() {
    let fixture = BundleFixture::new("late-subscribe");
    let mut embedder = development_embedder(&fixture, "linux");
    let early = collect_events(&mut embedder);

    embedder.submit(PROCESS_CAPABILITY_ID, &json!({ "note": "replay" }));
    let late = collect_events(&mut embedder);

    assert_eq!(snapshot(&early), snapshot(&late));
}

#[test]
fn release_evidence_records_package_runtime_and_bundle_digests() {
    let fixture = BundleFixture::new("release-evidence");
    let embedder = development_embedder(&fixture, "linux");

    let evidence = embedder.release_evidence();
    assert_eq!(evidence["kind"], "embedder_release_evidence");
    assert_eq!(evidence["package"]["name"], "traverse-embedder");
    assert_eq!(evidence["package"]["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(evidence["embedder_api_version"], EMBEDDER_API_VERSION);
    assert_eq!(
        evidence["conformance_version"],
        EMBEDDER_CONFORMANCE_VERSION
    );
    assert_eq!(evidence["runtime"]["implementation"], "traverse-runtime");
    assert_eq!(evidence["runtime"]["linkage"], "native-static");
    assert_eq!(evidence["bundle"]["app_id"], "fixture-app");
    let components = evidence["bundle"]["wasm_components"]
        .as_array()
        .expect("evidence should list wasm components");
    assert_eq!(components.len(), 1);
    assert_eq!(components[0]["capability_id"], PROCESS_CAPABILITY_ID);
    assert!(
        components[0]["wasm_digest"]
            .as_str()
            .expect("wasm digest should be recorded")
            .starts_with("sha256:")
    );
    assert_eq!(evidence["platform"], "linux");
}

// --- deterministic test double (FR-006) ---

#[test]
fn test_double_mirrors_submit_event_shapes() {
    let mut double = EmbedderTestDouble::new("local-default", "fixture-app", "1.0.0", "linux")
        .with_target_output(PROCESS_CAPABILITY_ID, json!({ "status": "processed" }));
    let events = collect_events(&mut double);

    let outcome = double.submit(PROCESS_CAPABILITY_ID, &json!({ "note": "double" }));
    assert_eq!(outcome.status, SubmitStatus::Accepted);
    assert_eq!(outcome.session_id.as_deref(), Some("sess-00000001"));

    let events = snapshot(&events);
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["event_type"], "capability_invoked");
    assert_eq!(events[0]["data"]["execution_id"], "exec_req-00000001");
    assert_eq!(events[1]["event_type"], "capability_result");
    assert_eq!(
        events[1]["data"]["output"],
        json!({ "status": "processed" })
    );
}

#[test]
fn test_double_scripts_runtime_shaped_errors() {
    let mut double = EmbedderTestDouble::new("local-default", "fixture-app", "1.0.0", "linux")
        .with_target_error(
            PROCESS_CAPABILITY_ID,
            "execution_failed",
            "scripted failure",
        );
    let events = collect_events(&mut double);

    let outcome = double.submit(PROCESS_CAPABILITY_ID, &json!({}));
    assert_eq!(outcome.status, SubmitStatus::Accepted);

    let events = snapshot(&events);
    assert_eq!(events[1]["event_type"], "error");
    assert_eq!(events[1]["data"]["error"]["code"], "execution_failed");
    assert_eq!(events[1]["data"]["error"]["message"], "scripted failure");
}

#[test]
fn test_double_rejects_unscripted_targets_and_stopped_submits() {
    let mut double = EmbedderTestDouble::new("local-default", "fixture-app", "1.0.0", "linux");

    let unknown = double.submit("fixture.unknown", &json!({}));
    assert_eq!(unknown.status, SubmitStatus::Rejected);
    assert_eq!(
        unknown.error.expect("unscripted target should error").code,
        EmbedderErrorCode::TargetNotFound,
    );

    double.shutdown();
    let stopped = double.submit(PROCESS_CAPABILITY_ID, &json!({}));
    assert_eq!(stopped.status, SubmitStatus::Rejected);
    assert_eq!(
        stopped.error.expect("stopped submit should error").code,
        EmbedderErrorCode::RuntimeStopped,
    );
}

#[test]
fn test_double_shares_compatible_lifecycle_and_evidence_boundary() {
    let mut double = EmbedderTestDouble::new("local-default", "fixture-app", "1.0.0", "linux")
        .with_compatible_target(RENDER_CAPABILITY_ID, vec!["linux".to_string()]);
    let events = collect_events(&mut double);

    let started = double.start_compatible(RENDER_CAPABILITY_ID, &json!({}));
    assert_eq!(started.status, CompatibleLifecycleStatus::Started);
    let first_instance = started.instance_id.expect("instance should start");
    let stopped = double.stop_compatible(RENDER_CAPABILITY_ID, Some(&first_instance));
    assert_eq!(stopped.status, CompatibleLifecycleStatus::Stopped);

    let restarted = double.start_compatible(RENDER_CAPABILITY_ID, &json!({}));
    assert_eq!(restarted.status, CompatibleLifecycleStatus::Started);
    let killed = double.kill_compatible(RENDER_CAPABILITY_ID, None);
    assert_eq!(killed.status, CompatibleLifecycleStatus::Killed);

    let final_instance = double.start_compatible(RENDER_CAPABILITY_ID, &json!({}));
    assert_eq!(final_instance.status, CompatibleLifecycleStatus::Started);
    let shutdown = double.shutdown();
    assert_eq!(shutdown.killed_instances, 1);

    let states: Vec<String> = snapshot(&events)
        .iter()
        .filter(|event| event["event_type"] == "state_changed")
        .map(|event| {
            event["data"]["state"]
                .as_str()
                .expect("state should be a string")
                .to_string()
        })
        .collect();
    assert_eq!(
        states,
        [
            "started", "stopped", "started", "killed", "started", "killed"
        ]
    );

    let evidence = double.release_evidence();
    assert_eq!(evidence["runtime"]["implementation"], "test-double");
    assert_eq!(evidence["embedder_api_version"], EMBEDDER_API_VERSION);
}

#[test]
fn test_double_platform_guard_matches_production_semantics() {
    let mut double = EmbedderTestDouble::new("local-default", "fixture-app", "1.0.0", "ios")
        .with_compatible_target(RENDER_CAPABILITY_ID, vec!["linux".to_string()]);

    let outcome = double.start_compatible(RENDER_CAPABILITY_ID, &json!({}));
    assert_eq!(outcome.status, CompatibleLifecycleStatus::Error);
    assert_eq!(
        outcome.error.expect("platform guard should error").code,
        EmbedderErrorCode::PlatformNotSupported,
    );
}
