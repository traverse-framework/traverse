//! Shared embedder conformance corpus (spec 057 `conformance.md`,
//! spec 068 FR-009) executed against the production Rust package.
#![allow(clippy::expect_used)]

mod common;

use common::{
    BundleFixture, FixtureOptions, PIPELINE_WORKFLOW_ID, PROCESS_CAPABILITY_ID, PROCESS_OUTPUT,
    RENDER_CAPABILITY_ID, collect_events, snapshot,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::Arc;
use traverse_embedder::{
    BundleEmbedder, CompatibleLifecycleStatus, EMBEDDED_TRACE_API_VERSION, EmbeddedTraceApi,
    EmbeddedTraceOutcome, EmbedderConfig, EmbedderErrorCode, HostDataStore, SecurityPosture,
    SubmitStatus, TraverseEmbedderApi,
};
use traverse_runtime::data_store::{
    DataStore, DataStoreError, DataStoreErrorCode, InMemoryKeyProvider, KeyProvider,
    LocalDataClassification, LocalFileDataStore, RemoteBackendFailure, RemoteDataStoreBackend,
    RemoteKeyValueDataStore, RemoteObject, RemoteVersionToken, RemoteWriteOutcome, StateRecord,
};

struct FailingDataStore {
    code: DataStoreErrorCode,
}

impl FailingDataStore {
    fn error(&self) -> DataStoreError {
        DataStoreError {
            code: self.code,
            message: "host adapter failure".to_string(),
            details: Value::Null,
        }
    }
}

impl DataStore for FailingDataStore {
    fn read(&self, _key: &str) -> Result<Option<StateRecord>, DataStoreError> {
        Err(self.error())
    }

    fn write(&mut self, _record: StateRecord) -> Result<(), DataStoreError> {
        Err(self.error())
    }

    fn delete(&mut self, _key: &str) -> Result<(), DataStoreError> {
        Err(self.error())
    }

    fn list_keys(&self) -> Result<Vec<String>, DataStoreError> {
        Err(self.error())
    }
}

/// A minimal S3-compatible-shaped host backend: version tokens increment
/// per key, conditional writes compare the caller's expected token, and one
/// scripted operation can be forced to an ambiguous (`Unknown`) outcome to
/// exercise the selected-provider contract failure case end to end.
#[derive(Default)]
struct FakeRemoteBackend {
    objects: BTreeMap<String, (Vec<u8>, u64)>,
    next_version: u64,
    force_unknown: bool,
}

impl RemoteDataStoreBackend for FakeRemoteBackend {
    fn get(&self, key: &str) -> Result<Option<RemoteObject>, RemoteBackendFailure> {
        Ok(self.objects.get(key).map(|(bytes, version)| RemoteObject {
            bytes: bytes.clone(),
            version: RemoteVersionToken(format!("v{version}")),
        }))
    }

    fn put_conditional(
        &mut self,
        key: &str,
        bytes: Vec<u8>,
        expected_version: Option<RemoteVersionToken>,
    ) -> RemoteWriteOutcome {
        if self.force_unknown {
            self.force_unknown = false;
            return RemoteWriteOutcome::Unknown { retry_count: 1 };
        }
        let current = self
            .objects
            .get(key)
            .map(|(_, version)| RemoteVersionToken(format!("v{version}")));
        if expected_version != current {
            return RemoteWriteOutcome::Conflict;
        }
        self.next_version += 1;
        self.objects
            .insert(key.to_string(), (bytes, self.next_version));
        RemoteWriteOutcome::Acknowledged { retry_count: 0 }
    }

    fn delete_conditional(
        &mut self,
        key: &str,
        expected_version: Option<RemoteVersionToken>,
    ) -> RemoteWriteOutcome {
        let current = self
            .objects
            .get(key)
            .map(|(_, version)| RemoteVersionToken(format!("v{version}")));
        if expected_version != current {
            return RemoteWriteOutcome::Conflict;
        }
        self.objects.remove(key);
        RemoteWriteOutcome::Acknowledged { retry_count: 0 }
    }
}

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

    // Use schema-valid inputs that still embed secret substrings so the
    // public trace surface can prove they stay redacted (#1658).
    let _ = embedder.submit(PROCESS_CAPABILITY_ID, &json!({ "note": "input-secret" }));
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

#[test]
fn host_injected_datastore_reopens_without_leaking_record_metadata_to_events() {
    let fixture = BundleFixture::new("host-datastore");
    let root = std::env::temp_dir().join(format!(
        "traverse-embedder-host-datastore-{}",
        std::process::id()
    ));
    let mut first = development_embedder(&fixture, "linux");
    let events = collect_events(&mut first);
    let provider: Arc<dyn KeyProvider> =
        Arc::new(InMemoryKeyProvider::new("embedder-test", [17; 32]));
    let store = LocalFileDataStore::new(&root)
        .expect("host should create store")
        .with_key_provider(Arc::clone(&provider));
    first.inject_data_store(HostDataStore::new(store, LocalDataClassification::Private));
    let record = StateRecord {
        key: "host-note".to_string(),
        value: json!({ "secret": "do-not-emit" }),
        lamport_clock: 1,
        writer_id: "host-writer".to_string(),
    };
    first
        .data_store_write(record.clone())
        .expect("explicit host write should persist");
    drop(first);

    let mut second = development_embedder(&fixture, "linux");
    let reopened = LocalFileDataStore::new(&root)
        .expect("host should reopen released store")
        .with_key_provider(provider);
    second.inject_data_store(HostDataStore::new(
        reopened,
        LocalDataClassification::Private,
    ));
    assert_eq!(
        second
            .data_store_read("host-note")
            .expect("explicit host read should succeed"),
        Some(record)
    );
    second
        .data_store_delete("host-note")
        .expect("explicit host delete should succeed");

    let public_root = root.with_extension("public");
    let mut public_embedder = development_embedder(&fixture, "linux");
    let public_store =
        LocalFileDataStore::with_classification(&public_root, LocalDataClassification::Public)
            .expect("host should create public store");
    public_embedder.inject_data_store(HostDataStore::new(
        public_store,
        LocalDataClassification::Public,
    ));
    assert_eq!(
        public_embedder
            .data_store_read("missing")
            .expect("public store read should succeed"),
        None
    );

    let telemetry = serde_json::to_string(&snapshot(&events)).expect("events should serialize");
    assert!(telemetry.contains("data_store_operation"));
    assert!(!telemetry.contains("host-note"));
    assert!(!telemetry.contains("do-not-emit"));
    drop(second);
    drop(public_embedder);
    let _ignored = std::fs::remove_dir_all(root);
    let _ignored = std::fs::remove_dir_all(public_root);
}

#[test]
fn host_datastore_operations_fail_closed_without_explicit_injection() {
    let fixture = BundleFixture::new("host-datastore-unconfigured");
    let mut embedder = development_embedder(&fixture, "linux");
    let error = embedder
        .data_store_read("host-note")
        .expect_err("runtime must not create an implicit store");
    assert_eq!(error.code, "data_store_not_configured");
    assert_eq!(error.operation, "read");
    let write_error = embedder
        .data_store_write(StateRecord {
            key: "host-note".to_string(),
            value: json!(null),
            lamport_clock: 1,
            writer_id: "host".to_string(),
        })
        .expect_err("runtime must not create an implicit store");
    assert_eq!(write_error.code, "data_store_not_configured");
    let delete_error = embedder
        .data_store_delete("host-note")
        .expect_err("runtime must not create an implicit store");
    assert_eq!(delete_error.code, "data_store_not_configured");
}

#[test]
fn host_datastore_failure_codes_are_safe_and_classified() {
    let fixture = BundleFixture::new("host-datastore-failures");
    let cases = [
        (
            DataStoreErrorCode::IntegrityCheckFailed,
            "integrity_check_failed",
        ),
        (DataStoreErrorCode::StoreLocked, "store_locked"),
        (
            DataStoreErrorCode::DurabilityCommitFailed,
            "durability_commit_failed",
        ),
        (DataStoreErrorCode::IoFailure, "storage_io_failed"),
        (DataStoreErrorCode::InvalidKey, "invalid_key"),
        (
            DataStoreErrorCode::SerializationFailure,
            "serialization_failed",
        ),
        (
            DataStoreErrorCode::SchemaValidationError,
            "schema_validation_failed",
        ),
        (
            DataStoreErrorCode::NoStateSchemaDeclared,
            "state_schema_unavailable",
        ),
        (
            DataStoreErrorCode::LamportClockOverflow,
            "lamport_clock_overflow",
        ),
        (DataStoreErrorCode::SyncFailure, "sync_failed"),
        (
            DataStoreErrorCode::KeyProviderRequired,
            "key_provider_required",
        ),
        (DataStoreErrorCode::KeyNotFound, "key_not_found"),
        (DataStoreErrorCode::KeyExpired, "key_expired"),
        (
            DataStoreErrorCode::KeyProviderFailure,
            "key_provider_failed",
        ),
        (DataStoreErrorCode::CryptoFailure, "crypto_failed"),
        (
            DataStoreErrorCode::ClassificationChangeNotAllowed,
            "classification_change_not_allowed",
        ),
    ];

    for (code, expected) in cases {
        let mut embedder = development_embedder(&fixture, "linux");
        let events = collect_events(&mut embedder);
        embedder.inject_data_store(HostDataStore::new(
            FailingDataStore { code },
            LocalDataClassification::Public,
        ));

        let error = embedder
            .data_store_read("host-note")
            .expect_err("host adapter error should be safe");
        assert_eq!(error.code, expected);
        assert_eq!(error.operation, "read");
        assert_eq!(snapshot(&events)[0]["data"]["classification"], "public");
    }
}

#[test]
fn host_datastore_write_and_delete_errors_are_safe() {
    let fixture = BundleFixture::new("host-datastore-write-delete-failures");
    let mut embedder = development_embedder(&fixture, "linux");
    embedder.inject_data_store(HostDataStore::new(
        FailingDataStore {
            code: DataStoreErrorCode::IoFailure,
        },
        LocalDataClassification::Public,
    ));
    let record = StateRecord {
        key: "host-note".to_string(),
        value: Value::Null,
        lamport_clock: 1,
        writer_id: "host-writer".to_string(),
    };

    assert_eq!(
        embedder
            .data_store_write(record)
            .expect_err("host write failure should be safe")
            .operation,
        "write"
    );
    assert_eq!(
        embedder
            .data_store_delete("host-note")
            .expect_err("host delete failure should be safe")
            .operation,
        "delete"
    );
}

#[test]
fn host_injected_remote_datastore_round_trips_through_the_embedder_boundary() {
    let fixture = BundleFixture::new("remote-datastore-round-trip");
    let mut embedder = development_embedder(&fixture, "linux");
    embedder.inject_data_store(HostDataStore::new(
        RemoteKeyValueDataStore::new(
            FakeRemoteBackend::default(),
            LocalDataClassification::Public,
        ),
        LocalDataClassification::Public,
    ));
    let record = StateRecord {
        key: "remote-note".to_string(),
        value: json!({ "title": "remote fixture" }),
        lamport_clock: 1,
        writer_id: "host-writer".to_string(),
    };

    embedder
        .data_store_write(record.clone())
        .expect("host-injected remote adapter write should succeed");
    let read = embedder
        .data_store_read("remote-note")
        .expect("host-injected remote adapter read should succeed");
    assert_eq!(read, Some(record));

    embedder
        .data_store_delete("remote-note")
        .expect("host-injected remote adapter delete should succeed");
    assert_eq!(
        embedder
            .data_store_read("remote-note")
            .expect("read after delete should succeed"),
        None
    );
}

#[test]
fn host_injected_remote_datastore_ambiguous_write_is_never_reported_as_success() {
    let fixture = BundleFixture::new("remote-datastore-ambiguous-write");
    let mut embedder = development_embedder(&fixture, "linux");
    let backend = FakeRemoteBackend {
        force_unknown: true,
        ..Default::default()
    };
    embedder.inject_data_store(HostDataStore::new(
        RemoteKeyValueDataStore::new(backend, LocalDataClassification::Public),
        LocalDataClassification::Public,
    ));
    let record = StateRecord {
        key: "remote-secret".to_string(),
        value: json!({ "value": "must-not-leak" }),
        lamport_clock: 1,
        writer_id: "host-writer".to_string(),
    };

    let error = embedder
        .data_store_write(record)
        .expect_err("an ambiguous provider outcome must never be reported as success");
    assert_eq!(error.code, "remote_outcome_unknown");
    let debug = format!("{error:?}");
    assert!(!debug.contains("remote-secret"));
    assert!(!debug.contains("must-not-leak"));
    assert_eq!(
        embedder
            .data_store_read("remote-secret")
            .expect("read after ambiguous write should succeed"),
        None,
        "the ambiguous write must not have silently landed"
    );
}
