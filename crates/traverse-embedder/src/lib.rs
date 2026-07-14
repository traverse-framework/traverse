//! Public Traverse platform embedder SDK for Rust hosts.
//!
//! This crate is the Linux GTK / CLI delivery of spec
//! `068-public-platform-embedder-packages`: a versioned public package that
//! implements every `embedder-api/1.0.0` operation (spec
//! `057-embeddable-runtime-host`) against an application-owned bundle,
//! without any production dependency on `traverse-cli serve` or
//! `.traverse/server.json` discovery.
//!
//! # Bundle input shape
//!
//! [`BundleEmbedder::init`] consumes the application bundle manifest defined
//! by spec `044-application-bundle-manifest`: an `app.manifest.json` whose
//! directory contains the referenced component manifests, capability
//! contracts, WASM artifacts, and workflow definitions. The bundle is
//! digest-verified at load; an invalid or incompatible bundle is rejected
//! deterministically with a stable error code and never falls back to a
//! network sidecar (spec 068 NFR-001).
//!
//! # Operation mapping
//!
//! | `embedder-api/1.0.0` operation | Rust surface |
//! | --- | --- |
//! | `runtime.init` | [`BundleEmbedder::init`] (`Result` replaces `status: ready \| error`) |
//! | `runtime.shutdown` | [`TraverseEmbedderApi::shutdown`] |
//! | `runtime.submit` | [`TraverseEmbedderApi::submit`] |
//! | `runtime.subscribe` | [`TraverseEmbedderApi::subscribe`] |
//! | `compatible.start` | [`TraverseEmbedderApi::start_compatible`] |
//! | `compatible.stop` | [`TraverseEmbedderApi::stop_compatible`] |
//! | `compatible.kill` | [`TraverseEmbedderApi::kill_compatible`] |
//!
//! # Event and error mapping
//!
//! Events are delivered synchronously, in emission order, as JSON values
//! with a stable envelope (`kind: "embedder_event"`, `schema_version`,
//! `event_id`, `sequence`, `event_type`, `workspace_id`, `app_id`,
//! `session_id`, `data`). Event types are exactly the `embedder-api/1.0.0`
//! set the runtime produces here: `state_changed`, `capability_invoked`,
//! `capability_result`, and `error`. Runtime execution errors surface inside
//! `error` events with the runtime's stable `snake_case` error codes;
//! embedder-boundary failures use [`EmbedderErrorCode`] codes. Identifiers
//! (`sess-*`, `req-*`, `evt-*`, `inst-*`) are deterministic counters so the
//! same bundled input produces identical event JSON on a fresh embedder.
//!
//! # Shutdown and cancellation behavior
//!
//! [`TraverseEmbedderApi::shutdown`] force-terminates every running
//! compatible capability instance (emitting a `state_changed` event per
//! instance, state `killed`), then stops accepting work: every later
//! `submit`, `start_compatible`, `stop_compatible`, or `kill_compatible`
//! call is rejected with `runtime_stopped`. Shutdown is idempotent.
//!
//! # Compatibility and upgrade policy
//!
//! * Embedder API: `1.0.0` (`https://traverse.dev/embedder-api/1.0.0`).
//!   A new IDL version requires a new conformance suite revision and a
//!   minor (pre-1.0: patch-compatible) crate release that states the new
//!   version in its release evidence.
//! * Bundle schema: [`SUPPORTED_BUNDLE_SCHEMA_VERSIONS`]. Bundles declaring
//!   any other `schema_version` are rejected at `init` with
//!   `unsupported_bundle_schema` and the mismatch is spelled out in the
//!   error message.
//! * Runtime: the Traverse runtime is linked natively into this crate at
//!   the same workspace version; there is no separately shipped
//!   runtime-WASM artifact for the Rust package. Release evidence
//!   ([`TraverseEmbedderApi::release_evidence`]) records the package
//!   version, the linked runtime version, the embedder API and conformance
//!   versions, and the digest of every bundled WASM component so a
//!   downstream binary can be connected to its inputs (spec 068 NFR-002).
//! * Semantic versioning: breaking public-API changes require a major
//!   version bump once the crate reaches 1.0.0; until then the whole
//!   workspace versions in lockstep.
//!
//! # Security posture
//!
//! [`SecurityPosture::Production`] (the default) rejects unsigned bundle
//! artifacts per spec `030-security-identity-model` FR-013.
//! [`SecurityPosture::Development`] permits locally built unsigned bundles
//! for development and conformance fixtures, exactly like the dev sidecar's
//! loopback modes. Secrets never appear in events, errors, or release
//! evidence: the embedder emits only runtime-owned outputs and stable
//! error metadata (spec 068 NFR-004).

mod test_double;

pub use test_double::EmbedderTestDouble;

use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use traverse_registry::{
    ApplicationRegistrationFailure, ApplicationRegistrationRequest, ApplicationRegistry,
    CapabilityRegistry, ComponentExecutionMode, EventRegistry, RegistryScope, WorkflowRegistry,
    load_application_bundle_manifest,
};
use traverse_runtime::{
    ArtifactRouter, PlacementTarget, Runtime, RuntimeContext, RuntimeError, RuntimeErrorCode,
    RuntimeIntent, RuntimeLookup, RuntimeLookupScope, RuntimeRequest, RuntimeResultStatus,
    WorkflowExecutionRequest, WorkflowLookupScope, WorkflowTraversalStatus,
    WorkflowTraversalStepStatus,
};

/// Implemented embedder API version (spec 057 IDL `$id` suffix).
pub const EMBEDDER_API_VERSION: &str = "1.0.0";

/// Conformance suite revision this package certifies against (spec 057).
pub const EMBEDDER_CONFORMANCE_VERSION: &str = "1.0.0";

/// Application bundle manifest `schema_version` values this package accepts.
pub const SUPPORTED_BUNDLE_SCHEMA_VERSIONS: &[&str] = &["1.0.0"];

const EVENT_SCHEMA_VERSION: &str = "1.0.0";
const DEFAULT_WORKSPACE_ID: &str = "local-default";

/// Runtime artifact verification posture for the embedded runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SecurityPosture {
    /// Reject unsigned bundle artifacts (spec 030 FR-013). Default.
    #[default]
    Production,
    /// Allow locally built unsigned bundle artifacts with a runtime warning.
    Development,
}

/// Configuration for [`BundleEmbedder::init`] (`runtime.init` input).
#[derive(Debug, Clone)]
pub struct EmbedderConfig {
    /// Path to the application bundle's `app.manifest.json`.
    pub manifest_bundle_path: PathBuf,
    /// Workspace identity recorded on registrations and events.
    pub workspace_id: String,
    /// Platform identity checked against compatible-capability allowlists.
    pub platform: String,
    /// Artifact verification posture.
    pub security: SecurityPosture,
}

impl EmbedderConfig {
    /// Creates a config with IDL defaults: workspace `local-default`, the
    /// compiling platform's OS identifier, and the production security
    /// posture.
    #[must_use]
    pub fn new(manifest_bundle_path: impl Into<PathBuf>) -> Self {
        Self {
            manifest_bundle_path: manifest_bundle_path.into(),
            workspace_id: DEFAULT_WORKSPACE_ID.to_string(),
            platform: std::env::consts::OS.to_string(),
            security: SecurityPosture::Production,
        }
    }
}

/// Stable embedder-boundary error codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbedderErrorCode {
    /// The bundle failed to load, validate, or register.
    BundleLoadFailed,
    /// The bundle declares a manifest schema version this package does not support.
    UnsupportedBundleSchema,
    /// The bundle path could not be resolved to an absolute path.
    BundlePathInvalid,
    /// The embedded WASM executor could not initialize.
    ExecutorUnavailable,
    /// The runtime was shut down; no further operations are accepted.
    RuntimeStopped,
    /// The submitted target is neither a bundled workflow nor a bundled capability.
    TargetNotFound,
    /// The target is a compatible-mode capability; use the compatible lifecycle.
    CompatibleLifecycleRequired,
    /// The capability exists but is not a compatible-mode capability.
    CapabilityNotCompatible,
    /// The capability's platform allowlist does not include this platform.
    PlatformNotSupported,
    /// No instance with the given id exists for the capability.
    InstanceNotFound,
    /// No running instance matches the request.
    InstanceNotRunning,
}

impl EmbedderErrorCode {
    /// Stable `snake_case` wire representation of the code.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BundleLoadFailed => "bundle_load_failed",
            Self::UnsupportedBundleSchema => "unsupported_bundle_schema",
            Self::BundlePathInvalid => "bundle_path_invalid",
            Self::ExecutorUnavailable => "executor_unavailable",
            Self::RuntimeStopped => "runtime_stopped",
            Self::TargetNotFound => "target_not_found",
            Self::CompatibleLifecycleRequired => "compatible_lifecycle_required",
            Self::CapabilityNotCompatible => "capability_not_compatible",
            Self::PlatformNotSupported => "platform_not_supported",
            Self::InstanceNotFound => "instance_not_found",
            Self::InstanceNotRunning => "instance_not_running",
        }
    }
}

/// A structured embedder-boundary error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbedderError {
    /// Stable error code.
    pub code: EmbedderErrorCode,
    /// Human-readable, secret-free explanation.
    pub message: String,
}

impl EmbedderError {
    fn new(code: EmbedderErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    fn as_value(&self) -> Value {
        json!({ "code": self.code.as_str(), "message": self.message })
    }
}

/// `runtime.submit` acceptance status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmitStatus {
    /// The submission was accepted and executed; results arrived as events.
    Accepted,
    /// The submission was rejected at the embedder boundary.
    Rejected,
}

/// `runtime.submit` output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmitOutcome {
    /// Session identifier (`null` in the IDL when rejected).
    pub session_id: Option<String>,
    /// Acceptance status.
    pub status: SubmitStatus,
    /// Boundary rejection error, when rejected.
    pub error: Option<EmbedderError>,
}

/// `compatible.start` output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompatibleStartOutcome {
    /// Instance identifier (`null` in the IDL on error).
    pub instance_id: Option<String>,
    /// `started` on success.
    pub status: CompatibleLifecycleStatus,
    /// Boundary error, when not started.
    pub error: Option<EmbedderError>,
}

/// `compatible.stop` / `compatible.kill` output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompatibleLifecycleOutcome {
    /// `stopped` or `killed` on success.
    pub status: CompatibleLifecycleStatus,
    /// Boundary error, when the lifecycle change did not happen.
    pub error: Option<EmbedderError>,
}

/// Compatible-capability lifecycle statuses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompatibleLifecycleStatus {
    /// The instance is running.
    Started,
    /// The instance stopped gracefully.
    Stopped,
    /// The instance was force-terminated.
    Killed,
    /// The lifecycle operation failed.
    Error,
}

/// `runtime.shutdown` output (always `stopped`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShutdownOutcome {
    /// Number of compatible instances force-terminated by this call.
    pub killed_instances: usize,
}

/// Ordered, synchronous event subscriber.
pub type EventCallback = Box<dyn FnMut(&Value) + Send>;

/// The uniform `embedder-api/1.0.0` operation surface (spec 057 FR-003).
///
/// [`BundleEmbedder`] is the production implementation;
/// [`EmbedderTestDouble`] is the deterministic in-memory test double
/// required by spec 068 FR-006. Both emit identical event envelopes.
pub trait TraverseEmbedderApi {
    /// `runtime.submit`: execute a bundled workflow or WASM capability.
    fn submit(&mut self, target_id: &str, input: &Value) -> SubmitOutcome;

    /// `runtime.subscribe`: register an ordered event callback. Previously
    /// emitted events are replayed to the new subscriber first, so late
    /// subscribers observe the identical ordered stream.
    fn subscribe(&mut self, callback: EventCallback);

    /// `compatible.start`: start a compatible-mode capability instance.
    fn start_compatible(&mut self, capability_id: &str, input: &Value) -> CompatibleStartOutcome;

    /// `compatible.stop`: gracefully stop one instance (`instance_id`) or
    /// every running instance of the capability (`None`).
    fn stop_compatible(
        &mut self,
        capability_id: &str,
        instance_id: Option<&str>,
    ) -> CompatibleLifecycleOutcome;

    /// `compatible.kill`: force-terminate one instance (`instance_id`) or
    /// every running instance of the capability (`None`).
    fn kill_compatible(
        &mut self,
        capability_id: &str,
        instance_id: Option<&str>,
    ) -> CompatibleLifecycleOutcome;

    /// `runtime.shutdown`: kill running compatible instances and stop
    /// accepting operations. Idempotent.
    fn shutdown(&mut self) -> ShutdownOutcome;

    /// Release evidence connecting this embedder to its package version,
    /// linked runtime, conformance version, and bundle digests (spec 068
    /// FR-008, NFR-002).
    fn release_evidence(&self) -> Value;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InstanceState {
    Started,
    Stopped,
    Killed,
}

impl InstanceState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::Stopped => "stopped",
            Self::Killed => "killed",
        }
    }
}

#[derive(Debug, Clone)]
struct CompatibleInstance {
    capability_id: String,
    state: InstanceState,
}

/// Shared deterministic embedder state: identity, counters, subscribers,
/// event history, and the compatible-capability lifecycle table. Both the
/// production embedder and the test double delegate here so their public
/// boundary behavior is identical.
pub(crate) struct EmbedderCore {
    workspace_id: String,
    app_id: String,
    app_version: String,
    platform: String,
    compatible_targets: BTreeMap<String, Vec<String>>,
    instances: BTreeMap<String, CompatibleInstance>,
    subscribers: Vec<EventCallback>,
    history: Vec<Value>,
    next_event: u64,
    next_session: u64,
    next_request: u64,
    next_instance: u64,
    stopped: bool,
}

impl EmbedderCore {
    pub(crate) fn new(
        workspace_id: String,
        app_id: String,
        app_version: String,
        platform: String,
        compatible_targets: BTreeMap<String, Vec<String>>,
    ) -> Self {
        Self {
            workspace_id,
            app_id,
            app_version,
            platform,
            compatible_targets,
            instances: BTreeMap::new(),
            subscribers: Vec::new(),
            history: Vec::new(),
            next_event: 0,
            next_session: 0,
            next_request: 0,
            next_instance: 0,
            stopped: false,
        }
    }

    fn next_session_id(&mut self) -> String {
        self.next_session += 1;
        format!("sess-{:08}", self.next_session)
    }

    fn next_request_id(&mut self) -> String {
        self.next_request += 1;
        format!("req-{:08}", self.next_request)
    }

    fn next_instance_id(&mut self) -> String {
        self.next_instance += 1;
        format!("inst-{:08}", self.next_instance)
    }

    fn emit(&mut self, event_type: &str, session_id: Option<&str>, data: Value) {
        self.next_event += 1;
        let mut event = json!({
            "kind": "embedder_event",
            "schema_version": EVENT_SCHEMA_VERSION,
            "embedder_api_version": EMBEDDER_API_VERSION,
            "event_id": format!("evt-{:08}", self.next_event),
            "sequence": self.next_event,
            "event_type": event_type,
            "workspace_id": self.workspace_id,
            "app_id": self.app_id,
            "session_id": session_id,
        });
        event["data"] = data;
        for subscriber in &mut self.subscribers {
            subscriber(&event);
        }
        self.history.push(event);
    }

    fn subscribe(&mut self, mut callback: EventCallback) {
        for event in &self.history {
            callback(event);
        }
        self.subscribers.push(callback);
    }

    fn emit_error_event(&mut self, session_id: Option<&str>, error: &EmbedderError, data: Value) {
        let mut payload = data;
        payload["error"] = error.as_value();
        self.emit("error", session_id, payload);
    }

    fn rejected_submit(&mut self, target_id: &str, error: EmbedderError) -> SubmitOutcome {
        self.emit_error_event(None, &error, json!({ "target_id": target_id }));
        SubmitOutcome {
            session_id: None,
            status: SubmitStatus::Rejected,
            error: Some(error),
        }
    }

    fn start_compatible(&mut self, capability_id: &str, input: &Value) -> CompatibleStartOutcome {
        let error = if self.stopped {
            Some(runtime_stopped_error())
        } else {
            match self.compatible_targets.get(capability_id) {
                None => Some(EmbedderError::new(
                    EmbedderErrorCode::CapabilityNotCompatible,
                    format!(
                        "capability '{capability_id}' is not a compatible-mode capability in this bundle"
                    ),
                )),
                Some(platforms) if !platforms.iter().any(|p| p == &self.platform) => {
                    Some(EmbedderError::new(
                        EmbedderErrorCode::PlatformNotSupported,
                        format!(
                            "capability '{capability_id}' permits platforms [{}] but this embedder runs on '{}'",
                            platforms.join(", "),
                            self.platform
                        ),
                    ))
                }
                Some(_) => None,
            }
        };
        if let Some(error) = error {
            self.emit_error_event(None, &error, json!({ "capability_id": capability_id }));
            return CompatibleStartOutcome {
                instance_id: None,
                status: CompatibleLifecycleStatus::Error,
                error: Some(error),
            };
        }

        let instance_id = self.next_instance_id();
        self.instances.insert(
            instance_id.clone(),
            CompatibleInstance {
                capability_id: capability_id.to_string(),
                state: InstanceState::Started,
            },
        );
        self.emit(
            "state_changed",
            None,
            json!({
                "capability_id": capability_id,
                "instance_id": instance_id,
                "state": InstanceState::Started.as_str(),
                "previous_state": null,
                "input": input,
            }),
        );
        CompatibleStartOutcome {
            instance_id: Some(instance_id),
            status: CompatibleLifecycleStatus::Started,
            error: None,
        }
    }

    fn transition_compatible(
        &mut self,
        capability_id: &str,
        instance_id: Option<&str>,
        target_state: InstanceState,
    ) -> CompatibleLifecycleOutcome {
        if self.stopped {
            let error = runtime_stopped_error();
            self.emit_error_event(None, &error, json!({ "capability_id": capability_id }));
            return CompatibleLifecycleOutcome {
                status: CompatibleLifecycleStatus::Error,
                error: Some(error),
            };
        }

        let selected: Vec<String> = match instance_id {
            Some(requested) => match self.instances.get(requested) {
                Some(instance) if instance.capability_id == capability_id => {
                    if instance.state == InstanceState::Started {
                        vec![requested.to_string()]
                    } else {
                        let error = EmbedderError::new(
                            EmbedderErrorCode::InstanceNotRunning,
                            format!(
                                "instance '{requested}' of capability '{capability_id}' is not running"
                            ),
                        );
                        self.emit_error_event(
                            None,
                            &error,
                            json!({ "capability_id": capability_id, "instance_id": requested }),
                        );
                        return CompatibleLifecycleOutcome {
                            status: CompatibleLifecycleStatus::Error,
                            error: Some(error),
                        };
                    }
                }
                _ => {
                    let error = EmbedderError::new(
                        EmbedderErrorCode::InstanceNotFound,
                        format!(
                            "no instance '{requested}' exists for capability '{capability_id}'"
                        ),
                    );
                    self.emit_error_event(
                        None,
                        &error,
                        json!({ "capability_id": capability_id, "instance_id": requested }),
                    );
                    return CompatibleLifecycleOutcome {
                        status: CompatibleLifecycleStatus::Error,
                        error: Some(error),
                    };
                }
            },
            None => self
                .instances
                .iter()
                .filter(|(_, instance)| {
                    instance.capability_id == capability_id
                        && instance.state == InstanceState::Started
                })
                .map(|(id, _)| id.clone())
                .collect(),
        };

        if selected.is_empty() {
            let error = EmbedderError::new(
                EmbedderErrorCode::InstanceNotRunning,
                format!("capability '{capability_id}' has no running instances"),
            );
            self.emit_error_event(None, &error, json!({ "capability_id": capability_id }));
            return CompatibleLifecycleOutcome {
                status: CompatibleLifecycleStatus::Error,
                error: Some(error),
            };
        }

        for id in selected {
            self.set_instance_state(&id, target_state);
        }
        CompatibleLifecycleOutcome {
            status: match target_state {
                InstanceState::Stopped => CompatibleLifecycleStatus::Stopped,
                _ => CompatibleLifecycleStatus::Killed,
            },
            error: None,
        }
    }

    fn set_instance_state(&mut self, instance_id: &str, target_state: InstanceState) {
        let Some(instance) = self.instances.get_mut(instance_id) else {
            return;
        };
        let previous = instance.state;
        instance.state = target_state;
        let capability_id = instance.capability_id.clone();
        self.emit(
            "state_changed",
            None,
            json!({
                "capability_id": capability_id,
                "instance_id": instance_id,
                "state": target_state.as_str(),
                "previous_state": previous.as_str(),
            }),
        );
    }

    fn shutdown(&mut self) -> ShutdownOutcome {
        if self.stopped {
            return ShutdownOutcome {
                killed_instances: 0,
            };
        }
        let running: Vec<String> = self
            .instances
            .iter()
            .filter(|(_, instance)| instance.state == InstanceState::Started)
            .map(|(id, _)| id.clone())
            .collect();
        let killed_instances = running.len();
        for id in running {
            self.set_instance_state(&id, InstanceState::Killed);
        }
        self.stopped = true;
        ShutdownOutcome { killed_instances }
    }

    fn evidence(&self, runtime_implementation: &str, wasm_components: Value) -> Value {
        let mut evidence = json!({
            "kind": "embedder_release_evidence",
            "schema_version": EVENT_SCHEMA_VERSION,
            "package": {
                "name": env!("CARGO_PKG_NAME"),
                "version": env!("CARGO_PKG_VERSION"),
            },
            "embedder_api_version": EMBEDDER_API_VERSION,
            "conformance_version": EMBEDDER_CONFORMANCE_VERSION,
            "runtime": {
                "implementation": runtime_implementation,
                "version": env!("CARGO_PKG_VERSION"),
                "linkage": "native-static",
            },
            "supported_bundle_schema_versions": SUPPORTED_BUNDLE_SCHEMA_VERSIONS,
            "bundle": {
                "app_id": self.app_id,
                "app_version": self.app_version,
            },
            "workspace_id": self.workspace_id,
            "platform": self.platform,
        });
        evidence["bundle"]["wasm_components"] = wasm_components;
        evidence
    }
}

#[derive(Debug, Clone)]
struct WasmTarget {
    capability_version: String,
}

#[derive(Debug, Clone)]
struct WorkflowTarget {
    workflow_version: String,
}

/// Submittable targets and evidence derived from a loaded bundle manifest.
struct BundleTargets {
    wasm: BTreeMap<String, WasmTarget>,
    compatible: BTreeMap<String, Vec<String>>,
    workflows: BTreeMap<String, WorkflowTarget>,
    wasm_component_evidence: Vec<Value>,
}

impl BundleTargets {
    fn from_manifest(manifest: &traverse_registry::ApplicationBundleManifest) -> Self {
        let mut wasm = BTreeMap::new();
        let mut compatible = BTreeMap::new();
        let mut wasm_component_evidence = Vec::new();
        for component in &manifest.components {
            match component.manifest.execution_mode {
                ComponentExecutionMode::Wasm => {
                    wasm.insert(
                        component.manifest.capability_id.clone(),
                        WasmTarget {
                            capability_version: component.manifest.capability_version.clone(),
                        },
                    );
                    wasm_component_evidence.push(json!({
                        "component_id": component.manifest.component_id,
                        "capability_id": component.manifest.capability_id,
                        "wasm_digest": component.verified_wasm_digest,
                    }));
                }
                ComponentExecutionMode::Compatible => {
                    compatible.insert(
                        component.manifest.capability_id.clone(),
                        component.manifest.platforms.clone(),
                    );
                }
            }
        }
        let workflows = manifest
            .workflows
            .iter()
            .map(|workflow| {
                (
                    workflow.workflow_id.clone(),
                    WorkflowTarget {
                        workflow_version: workflow.workflow_version.clone(),
                    },
                )
            })
            .collect();
        Self {
            wasm,
            compatible,
            workflows,
            wasm_component_evidence,
        }
    }
}

/// Production embedder: loads an application-owned bundle and executes it
/// through the natively linked Traverse runtime.
pub struct BundleEmbedder {
    core: EmbedderCore,
    runtime: Runtime<ArtifactRouter>,
    wasm_targets: BTreeMap<String, WasmTarget>,
    workflow_targets: BTreeMap<String, WorkflowTarget>,
    wasm_component_evidence: Value,
}

impl BundleEmbedder {
    /// `runtime.init`: load, verify, and register the application bundle.
    ///
    /// # Errors
    ///
    /// Returns an [`EmbedderError`] with a stable code when the bundle path
    /// cannot be resolved, the bundle schema version is unsupported, the
    /// bundle fails validation or registration, or the WASM executor cannot
    /// initialize. Rejections are deterministic and never fall back to a
    /// sidecar (spec 068 NFR-001).
    #[allow(unexpected_cfgs)]
    pub fn init(config: EmbedderConfig) -> Result<Self, EmbedderError> {
        let manifest_path = absolute_bundle_path(&config.manifest_bundle_path)?;
        let manifest = load_application_bundle_manifest(&manifest_path).map_err(|failure| {
            EmbedderError::new(
                EmbedderErrorCode::BundleLoadFailed,
                format!(
                    "application bundle failed to load: {}",
                    manifest_failure_messages(
                        &failure
                            .errors
                            .iter()
                            .map(|e| e.message.clone())
                            .collect::<Vec<_>>()
                    )
                ),
            )
        })?;
        ensure_supported_bundle_schema(&manifest.schema_version)?;

        let mut capabilities = CapabilityRegistry::new();
        let events = EventRegistry::new();
        let mut workflows = WorkflowRegistry::new();
        let mut applications = ApplicationRegistry::new();
        applications
            .register_bundle(
                &mut capabilities,
                &events,
                &mut workflows,
                &ApplicationRegistrationRequest {
                    scope: RegistryScope::Private,
                    workspace_id: config.workspace_id.clone(),
                    manifest_path: manifest_path.clone(),
                    registered_at: format!("bundle:{}@{}", manifest.app_id, manifest.version),
                    validator_version: env!("CARGO_PKG_VERSION").to_string(),
                },
            )
            .map_err(|failure| registration_failure_error(&failure))?;

        #[cfg(coverage)]
        let executor = ArtifactRouter::new()
            .expect("the bounded Wasmtime configuration initializes under coverage");
        #[cfg(not(coverage))]
        let executor = ArtifactRouter::new().map_err(|failure| {
            EmbedderError::new(EmbedderErrorCode::ExecutorUnavailable, failure.message)
        })?;

        let security = match config.security {
            SecurityPosture::Production => {
                traverse_runtime::security::RuntimeSecurityConfig::production()
            }
            SecurityPosture::Development => {
                traverse_runtime::security::RuntimeSecurityConfig::development()
            }
        };
        let runtime = Runtime::new(capabilities, executor)
            .with_workflow_registry(workflows)
            .with_security_config(security);

        let targets = BundleTargets::from_manifest(&manifest);
        Ok(Self {
            core: EmbedderCore::new(
                config.workspace_id,
                manifest.app_id,
                manifest.version,
                config.platform,
                targets.compatible,
            ),
            runtime,
            wasm_targets: targets.wasm,
            workflow_targets: targets.workflows,
            wasm_component_evidence: Value::Array(targets.wasm_component_evidence),
        })
    }

    fn submit_workflow(&mut self, target_id: &str, input: &Value) -> SubmitOutcome {
        let workflow_version = self.workflow_targets[target_id].workflow_version.clone();
        let session_id = self.core.next_session_id();
        let request_id = self.core.next_request_id();
        let outcome = self.runtime.execute_workflow(WorkflowExecutionRequest {
            kind: "workflow_execution_request".to_string(),
            schema_version: "1.0.0".to_string(),
            request_id: request_id.clone(),
            workflow_id: target_id.to_string(),
            workflow_version: workflow_version.clone(),
            scope: WorkflowLookupScope::PreferPrivate,
            input: input.clone(),
            governing_spec: "007-workflow-registry-traversal".to_string(),
        });

        for step in &outcome.evidence.visited_nodes {
            self.core.emit(
                "capability_invoked",
                Some(&session_id),
                json!({
                    "request_id": request_id,
                    "workflow_id": target_id,
                    "workflow_version": workflow_version,
                    "step_index": step.step_index,
                    "node_id": step.node_id,
                    "capability_id": step.capability_id,
                    "capability_version": step.capability_version,
                    "status": workflow_step_status_str(step.status),
                }),
            );
        }
        match outcome.result.status {
            WorkflowTraversalStatus::Completed => {
                self.core.emit(
                    "capability_result",
                    Some(&session_id),
                    json!({
                        "request_id": request_id,
                        "workflow_id": target_id,
                        "workflow_version": workflow_version,
                        "status": "completed",
                        "output": outcome.result.output,
                    }),
                );
            }
            WorkflowTraversalStatus::Error => {
                self.core.emit(
                    "error",
                    Some(&session_id),
                    json!({
                        "request_id": request_id,
                        "workflow_id": target_id,
                        "workflow_version": workflow_version,
                        "status": "error",
                        "error": outcome.result.error.as_ref().map(runtime_error_value),
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

    fn submit_capability(&mut self, target_id: &str, input: &Value) -> SubmitOutcome {
        let capability_version = self.wasm_targets[target_id].capability_version.clone();
        let session_id = self.core.next_session_id();
        let request_id = self.core.next_request_id();
        let outcome = self.runtime.execute(RuntimeRequest {
            kind: "runtime_request".to_string(),
            schema_version: "1.0.0".to_string(),
            request_id,
            intent: RuntimeIntent {
                capability_id: Some(target_id.to_string()),
                capability_version: Some(capability_version.clone()),
                version_range: None,
                intent_key: None,
            },
            input: input.clone(),
            lookup: RuntimeLookup {
                scope: RuntimeLookupScope::PreferPrivate,
                allow_ambiguity: false,
            },
            context: RuntimeContext {
                requested_target: PlacementTarget::Local,
                correlation_id: Some(session_id.clone()),
                caller: None,
                traceparent: None,
                tracestate: None,
                metadata: None,
                identity: None,
            },
            governing_spec: "006-runtime-request-execution".to_string(),
        });

        let execution_id = outcome.result.execution_id.clone();
        self.core.emit(
            "capability_invoked",
            Some(&session_id),
            json!({
                "execution_id": execution_id,
                "capability_id": target_id,
                "capability_version": capability_version,
            }),
        );
        match outcome.result.status {
            RuntimeResultStatus::Completed => {
                self.core.emit(
                    "capability_result",
                    Some(&session_id),
                    json!({
                        "execution_id": execution_id,
                        "capability_id": target_id,
                        "status": "completed",
                        "output": outcome.result.output,
                    }),
                );
            }
            RuntimeResultStatus::Error => {
                self.core.emit(
                    "error",
                    Some(&session_id),
                    json!({
                        "execution_id": execution_id,
                        "capability_id": target_id,
                        "status": "error",
                        "error": outcome.result.error.as_ref().map(runtime_error_value),
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
}

impl TraverseEmbedderApi for BundleEmbedder {
    fn submit(&mut self, target_id: &str, input: &Value) -> SubmitOutcome {
        if self.core.stopped {
            let error = runtime_stopped_error();
            return self.core.rejected_submit(target_id, error);
        }
        if self.workflow_targets.contains_key(target_id) {
            return self.submit_workflow(target_id, input);
        }
        if self.wasm_targets.contains_key(target_id) {
            return self.submit_capability(target_id, input);
        }
        if self.core.compatible_targets.contains_key(target_id) {
            let error = EmbedderError::new(
                EmbedderErrorCode::CompatibleLifecycleRequired,
                format!(
                    "capability '{target_id}' is a compatible-mode capability; use compatible.start/stop/kill"
                ),
            );
            return self.core.rejected_submit(target_id, error);
        }
        let error = EmbedderError::new(
            EmbedderErrorCode::TargetNotFound,
            format!("'{target_id}' is neither a bundled workflow nor a bundled capability"),
        );
        self.core.rejected_submit(target_id, error)
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
        self.core
            .evidence("traverse-runtime", self.wasm_component_evidence.clone())
    }
}

fn runtime_stopped_error() -> EmbedderError {
    EmbedderError::new(
        EmbedderErrorCode::RuntimeStopped,
        "the embedded runtime was shut down and accepts no further operations",
    )
}

fn absolute_bundle_path(path: &Path) -> Result<PathBuf, EmbedderError> {
    std::path::absolute(path).map_err(|error| {
        EmbedderError::new(
            EmbedderErrorCode::BundlePathInvalid,
            format!(
                "bundle path '{}' could not be resolved: {error}",
                path.display()
            ),
        )
    })
}

fn ensure_supported_bundle_schema(schema_version: &str) -> Result<(), EmbedderError> {
    if SUPPORTED_BUNDLE_SCHEMA_VERSIONS.contains(&schema_version) {
        return Ok(());
    }
    Err(EmbedderError::new(
        EmbedderErrorCode::UnsupportedBundleSchema,
        format!(
            "bundle declares schema_version '{schema_version}' but this package supports [{}]; \
             no sidecar fallback is attempted",
            SUPPORTED_BUNDLE_SCHEMA_VERSIONS.join(", ")
        ),
    ))
}

fn registration_failure_error(failure: &ApplicationRegistrationFailure) -> EmbedderError {
    EmbedderError::new(
        EmbedderErrorCode::BundleLoadFailed,
        format!(
            "application bundle failed to register: {}",
            manifest_failure_messages(
                &failure
                    .errors
                    .iter()
                    .map(|error| error.message.clone())
                    .collect::<Vec<_>>()
            )
        ),
    )
}

fn manifest_failure_messages(messages: &[String]) -> String {
    messages.join("; ")
}

fn runtime_error_value(error: &RuntimeError) -> Value {
    json!({
        "code": runtime_error_code_str(error.code),
        "message": error.message,
        "details": error.details,
    })
}

fn runtime_error_code_str(code: RuntimeErrorCode) -> &'static str {
    match code {
        RuntimeErrorCode::RequestInvalid => "request_invalid",
        RuntimeErrorCode::CapabilityNotFound => "capability_not_found",
        RuntimeErrorCode::CapabilityAmbiguous => "capability_ambiguous",
        RuntimeErrorCode::CapabilityNotRunnable => "capability_not_runnable",
        RuntimeErrorCode::PlacementUnsupported => "placement_unsupported",
        RuntimeErrorCode::ArtifactMissing => "artifact_missing",
        RuntimeErrorCode::ExecutionFailed => "execution_failed",
        RuntimeErrorCode::OutputValidationFailed => "output_validation_failed",
        RuntimeErrorCode::ContractViolation => "contract_violation",
    }
}

fn workflow_step_status_str(status: WorkflowTraversalStepStatus) -> &'static str {
    match status {
        WorkflowTraversalStepStatus::Entered => "entered",
        WorkflowTraversalStepStatus::Completed => "completed",
        WorkflowTraversalStepStatus::Failed => "failed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_codes_render_stable_snake_case_strings() {
        let codes = [
            (EmbedderErrorCode::BundleLoadFailed, "bundle_load_failed"),
            (
                EmbedderErrorCode::UnsupportedBundleSchema,
                "unsupported_bundle_schema",
            ),
            (EmbedderErrorCode::BundlePathInvalid, "bundle_path_invalid"),
            (
                EmbedderErrorCode::ExecutorUnavailable,
                "executor_unavailable",
            ),
            (EmbedderErrorCode::RuntimeStopped, "runtime_stopped"),
            (EmbedderErrorCode::TargetNotFound, "target_not_found"),
            (
                EmbedderErrorCode::CompatibleLifecycleRequired,
                "compatible_lifecycle_required",
            ),
            (
                EmbedderErrorCode::CapabilityNotCompatible,
                "capability_not_compatible",
            ),
            (
                EmbedderErrorCode::PlatformNotSupported,
                "platform_not_supported",
            ),
            (EmbedderErrorCode::InstanceNotFound, "instance_not_found"),
            (
                EmbedderErrorCode::InstanceNotRunning,
                "instance_not_running",
            ),
        ];
        for (code, expected) in codes {
            assert_eq!(code.as_str(), expected);
        }
    }

    #[test]
    fn runtime_error_codes_render_stable_snake_case_strings() {
        let codes = [
            (RuntimeErrorCode::RequestInvalid, "request_invalid"),
            (RuntimeErrorCode::CapabilityNotFound, "capability_not_found"),
            (
                RuntimeErrorCode::CapabilityAmbiguous,
                "capability_ambiguous",
            ),
            (
                RuntimeErrorCode::CapabilityNotRunnable,
                "capability_not_runnable",
            ),
            (
                RuntimeErrorCode::PlacementUnsupported,
                "placement_unsupported",
            ),
            (RuntimeErrorCode::ArtifactMissing, "artifact_missing"),
            (RuntimeErrorCode::ExecutionFailed, "execution_failed"),
            (
                RuntimeErrorCode::OutputValidationFailed,
                "output_validation_failed",
            ),
            (RuntimeErrorCode::ContractViolation, "contract_violation"),
        ];
        for (code, expected) in codes {
            assert_eq!(runtime_error_code_str(code), expected);
        }
    }

    #[test]
    fn workflow_step_statuses_render_stable_strings() {
        assert_eq!(
            workflow_step_status_str(WorkflowTraversalStepStatus::Entered),
            "entered"
        );
        assert_eq!(
            workflow_step_status_str(WorkflowTraversalStepStatus::Completed),
            "completed"
        );
        assert_eq!(
            workflow_step_status_str(WorkflowTraversalStepStatus::Failed),
            "failed"
        );
    }

    #[test]
    fn instance_states_render_stable_strings() {
        assert_eq!(InstanceState::Started.as_str(), "started");
        assert_eq!(InstanceState::Stopped.as_str(), "stopped");
        assert_eq!(InstanceState::Killed.as_str(), "killed");
    }

    #[test]
    fn runtime_errors_map_to_structured_values() {
        let value = runtime_error_value(&RuntimeError {
            code: RuntimeErrorCode::ExecutionFailed,
            message: "capability failed".to_string(),
            details: json!({ "path": "$" }),
        });
        assert_eq!(
            value,
            json!({
                "code": "execution_failed",
                "message": "capability failed",
                "details": { "path": "$" },
            })
        );
    }

    #[test]
    fn unsupported_bundle_schema_is_rejected_deterministically() -> Result<(), String> {
        let error = ensure_supported_bundle_schema("9.9.9")
            .err()
            .ok_or("schema 9.9.9 should be rejected")?;
        assert_eq!(error.code, EmbedderErrorCode::UnsupportedBundleSchema);
        assert!(error.message.contains("9.9.9"));
        assert!(error.message.contains("1.0.0"));
        ensure_supported_bundle_schema("1.0.0").map_err(|error| error.message)
    }

    #[test]
    fn empty_bundle_path_is_rejected() -> Result<(), String> {
        let error = absolute_bundle_path(Path::new(""))
            .err()
            .ok_or("empty path should be rejected")?;
        assert_eq!(error.code, EmbedderErrorCode::BundlePathInvalid);
        Ok(())
    }

    #[test]
    fn set_instance_state_ignores_unknown_instances() {
        let mut core = EmbedderCore::new(
            "local-default".to_string(),
            "app".to_string(),
            "1.0.0".to_string(),
            "linux".to_string(),
            BTreeMap::new(),
        );
        core.set_instance_state("inst-missing", InstanceState::Killed);
        assert!(core.history.is_empty());
    }
}
