//! Shared embedder conformance corpus (spec 057 `conformance.md`,
//! spec 068 FR-009) executed against the production Rust package.
#![allow(clippy::expect_used)]

mod common;

use common::{
    BundleFixture, FixtureOptions, PIPELINE_WORKFLOW_ID, PROCESS_CAPABILITY_ID, PROCESS_OUTPUT,
    RENDER_CAPABILITY_ID, collect_events, snapshot,
};
use serde_json::{Value, json};
use traverse_embedder::{
    BundleEmbedder, CompatibleLifecycleStatus, EMBEDDED_TRACE_API_VERSION, EmbeddedTraceApi,
    EmbeddedTraceOutcome, EmbedderConfig, EmbedderErrorCode, SecurityPosture, SubmitStatus,
    TraverseEmbedderApi,
};

fn development_embedder(fixture: &BundleFixture, platform: &str) -> BundleEmbedder {
    let mut config = EmbedderConfig::new(fixture.manifest_path());
    config.platform = platform.to_string();
    config.security = SecurityPosture::Development;
    BundleEmbedder::init(config).expect("fixture bundle should initialize")
}

#[test]
fn init_shutdown_scenario_reaches_ready_then_stopped() {
    let fixture = BundleFixture::new("init-shutdown");
    let mut embedder = development_embedder(&fixture, "linux");

    let shutdown = embedder.shutdown();
    assert_eq!(shutdown.killed_instances, 0);
    let repeated = embedder.shutdown();
    assert_eq!(repeated.killed_instances, 0);

    let rejected = embedder.submit(PROCESS_CAPABILITY_ID, &json!({ "note": "late" }));
    assert_eq!(rejected.status, SubmitStatus::Rejected);
    assert_eq!(
        rejected.error.expect("stopped submit should error").code,
        EmbedderErrorCode::RuntimeStopped,
    );
}

#[test]
fn wasm_capability_submit_scenario_emits_capability_result() {
    let fixture = BundleFixture::new("wasm-submit");
    let mut embedder = development_embedder(&fixture, "linux");
    let events = collect_events(&mut embedder);

    let outcome = embedder.submit(PROCESS_CAPABILITY_ID, &json!({ "note": "hello" }));
    assert_eq!(outcome.status, SubmitStatus::Accepted);
    assert_eq!(outcome.session_id.as_deref(), Some("sess-00000001"));
    assert_eq!(outcome.error, None);

    let events = snapshot(&events);
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["event_type"], "capability_invoked");
    assert_eq!(events[0]["data"]["capability_id"], PROCESS_CAPABILITY_ID);
    assert_eq!(events[1]["event_type"], "capability_result");
    assert_eq!(events[1]["data"]["status"], "completed");
    let expected: Value =
        serde_json::from_str(PROCESS_OUTPUT).expect("expected output should parse");
    assert_eq!(events[1]["data"]["output"], expected);
}

#[test]
fn embedded_trace_companion_projects_production_capability_and_workflow_evidence() {
    let fixture = BundleFixture::new("embedded-trace-production");
    let mut embedder = development_embedder(&fixture, "linux");
    assert_eq!(
        embedder.embedded_trace_api_version(),
        EMBEDDED_TRACE_API_VERSION
    );

    let _ = embedder.submit(PROCESS_CAPABILITY_ID, &json!({ "secret": "input-secret" }));
    let _ = embedder.submit(PIPELINE_WORKFLOW_ID, &json!({ "note": "workflow-secret" }));

    let page = embedder
        .trace_list(EMBEDDED_TRACE_API_VERSION, 10, None)
        .expect("production trace list should succeed");
    assert_eq!(page.summaries.len(), 2);
    assert_eq!(page.summaries[0].target_id, PIPELINE_WORKFLOW_ID);
    assert_eq!(page.summaries[0].outcome, EmbeddedTraceOutcome::Completed);
    let capability = page
        .summaries
        .iter()
        .find(|summary| summary.target_id == PROCESS_CAPABILITY_ID)
        .expect("capability submission should have a retained trace");
    let detail = embedder
        .trace_get(EMBEDDED_TRACE_API_VERSION, &capability.trace_id)
        .expect("production trace detail should succeed");
    assert_eq!(detail.summary.execution_id, capability.execution_id);
    assert_eq!(
        detail
            .selected_target
            .as_ref()
            .map(|target| target.target_id.as_str()),
        Some(PROCESS_CAPABILITY_ID)
    );
    assert_eq!(
        detail
            .placement
            .as_ref()
            .map(|placement| placement.target.as_str()),
        Some("local")
    );
    assert!(detail.state_machine_valid.is_some());
    let public_detail = format!("{detail:?}");
    assert!(!public_detail.contains("input-secret"));
    assert!(!public_detail.contains("workflow-secret"));
}

#[test]
fn workflow_submit_returns_runtime_owned_pipeline_output() {
    let fixture = BundleFixture::new("workflow-submit");
    let mut embedder = development_embedder(&fixture, "linux");
    let events = collect_events(&mut embedder);

    let outcome = embedder.submit(PIPELINE_WORKFLOW_ID, &json!({ "note": "hello" }));
    assert_eq!(outcome.status, SubmitStatus::Accepted);

    let events = snapshot(&events);
    let result = events
        .last()
        .expect("workflow submit should emit a terminal event");
    assert_eq!(result["event_type"], "capability_result");
    assert_eq!(result["data"]["workflow_id"], PIPELINE_WORKFLOW_ID);
    assert_eq!(result["data"]["status"], "completed");
    // The merged pipeline output is runtime-owned: workflow input fields
    // plus every step's to_workflow_state mapping (spec 058 FR-007).
    let mut expected: Value =
        serde_json::from_str(PROCESS_OUTPUT).expect("expected output should parse");
    expected["note"] = json!("hello");
    assert_eq!(result["data"]["output"], expected);
    let invoked: Vec<&Value> = events
        .iter()
        .filter(|event| event["event_type"] == "capability_invoked")
        .collect();
    assert!(
        !invoked.is_empty(),
        "each pipeline step must surface a capability_invoked event"
    );
    assert_eq!(invoked[0]["data"]["capability_id"], PROCESS_CAPABILITY_ID);
}

#[test]
fn compatible_lifecycle_scenario_starts_stops_and_kills_on_shutdown() {
    let fixture = BundleFixture::new("compatible-lifecycle");
    let mut embedder = development_embedder(&fixture, "linux");
    let events = collect_events(&mut embedder);

    let started = embedder.start_compatible(RENDER_CAPABILITY_ID, &json!({ "surface": "gtk" }));
    assert_eq!(started.status, CompatibleLifecycleStatus::Started);
    let first_instance = started
        .instance_id
        .expect("started instance should have an id");

    let stopped = embedder.stop_compatible(RENDER_CAPABILITY_ID, Some(&first_instance));
    assert_eq!(stopped.status, CompatibleLifecycleStatus::Stopped);
    assert_eq!(stopped.error, None);

    let restarted = embedder.start_compatible(RENDER_CAPABILITY_ID, &json!({ "surface": "gtk" }));
    assert_eq!(restarted.status, CompatibleLifecycleStatus::Started);

    let shutdown = embedder.shutdown();
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
    assert_eq!(states, ["started", "stopped", "started", "killed"]);
}

#[test]
fn platform_guard_scenario_rejects_wrong_platform_with_deterministic_error() {
    let fixture = BundleFixture::new("platform-guard");
    let mut embedder = development_embedder(&fixture, "ios");
    let events = collect_events(&mut embedder);

    let outcome = embedder.start_compatible(RENDER_CAPABILITY_ID, &json!({}));
    assert_eq!(outcome.status, CompatibleLifecycleStatus::Error);
    assert_eq!(outcome.instance_id, None);
    let error = outcome.error.expect("platform guard should error");
    assert_eq!(error.code, EmbedderErrorCode::PlatformNotSupported);

    let events = snapshot(&events);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["event_type"], "error");
    assert_eq!(events[0]["data"]["error"]["code"], "platform_not_supported");
    assert_eq!(events[0]["data"]["capability_id"], RENDER_CAPABILITY_ID);
}

#[test]
fn determinism_scenario_produces_identical_event_json_twice() {
    let fixture = BundleFixture::new("determinism");

    let run = || {
        let mut embedder = development_embedder(&fixture, "linux");
        let events = collect_events(&mut embedder);
        let submit = embedder.submit(PROCESS_CAPABILITY_ID, &json!({ "note": "same input" }));
        assert_eq!(submit.status, SubmitStatus::Accepted);
        let workflow = embedder.submit(PIPELINE_WORKFLOW_ID, &json!({ "note": "same input" }));
        assert_eq!(workflow.status, SubmitStatus::Accepted);
        let started = embedder.start_compatible(RENDER_CAPABILITY_ID, &json!({}));
        assert_eq!(started.status, CompatibleLifecycleStatus::Started);
        embedder.shutdown();
        snapshot(&events)
    };

    let first = run();
    let second = run();
    assert_eq!(
        serde_json::to_string(&first).expect("events should serialize"),
        serde_json::to_string(&second).expect("events should serialize"),
        "same bundled input must produce identical event JSON"
    );
}

#[test]
fn conformance_matches_unsupported_schema_rejection() {
    let options = FixtureOptions {
        schema_version: "9.9.9".to_string(),
        ..FixtureOptions::default()
    };
    let fixture = BundleFixture::with_options("schema-reject", &options);
    let mut config = EmbedderConfig::new(fixture.manifest_path());
    config.security = SecurityPosture::Development;
    let error = BundleEmbedder::init(config)
        .err()
        .expect("unsupported schema should be rejected");
    assert_eq!(error.code, EmbedderErrorCode::UnsupportedBundleSchema);
    assert!(error.message.contains("9.9.9"));
}
