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
use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use traverse_registry::{
    ApplicationRegistrationFailure, ApplicationRegistrationRequest, ApplicationRegistry,
    CapabilityRegistry, ComponentExecutionMode, EventRegistry, RegistryScope, WorkflowRegistry,
    load_application_bundle_manifest,
};
use traverse_runtime::{
    ArtifactRouter, ExecutionFailureReason, PlacementTarget, Runtime, RuntimeContext, RuntimeError,
    RuntimeErrorCode, RuntimeExecutionOutcome, RuntimeIntent, RuntimeLookup, RuntimeLookupScope,
    RuntimeRequest, RuntimeResultStatus, WorkflowExecutionOutcome, WorkflowExecutionRequest,
    WorkflowLookupScope, WorkflowTraversalStatus, WorkflowTraversalStepStatus,
};

/// Implemented embedder API version (spec 057 IDL `$id` suffix).
pub const EMBEDDER_API_VERSION: &str = "1.0.0";

/// Conformance suite revision this package certifies against (spec 057).
pub const EMBEDDER_CONFORMANCE_VERSION: &str = "1.0.0";

/// Implemented companion Trace API version (spec 517).
pub const EMBEDDED_TRACE_API_VERSION: &str = "1.0.0";

/// Maximum number of public trace records retained by one embedded session.
pub const EMBEDDED_TRACE_RETENTION_LIMIT: usize = 100;

/// Largest page the public embedded Trace API returns in one call.
pub const EMBEDDED_TRACE_MAX_PAGE_SIZE: usize = 100;

/// Application bundle manifest `schema_version` values this package accepts.
pub const SUPPORTED_BUNDLE_SCHEMA_VERSIONS: &[&str] = &["1.0.0"];

const EVENT_SCHEMA_VERSION: &str = "1.0.0";
const DEFAULT_WORKSPACE_ID: &str = "local-default";
static NEXT_EMBEDDED_TRACE_SESSION: AtomicU64 = AtomicU64::new(1);

/// Stable machine-readable public Trace API failure codes (spec 517 FR-010).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddedTraceApiErrorCode {
    /// The cursor is malformed, stale, or belongs to another embedder session.
    InvalidCursor,
    /// The requested trace is no longer retained by this session.
    TraceNotFound,
    /// The embedder has been stopped and cannot serve local diagnostics.
    TraceApiUnavailable,
    /// The caller requested a companion API version this package does not support.
    IncompatibleVersion,
}

impl EmbeddedTraceApiErrorCode {
    /// Returns the stable wire representation of this code.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidCursor => "invalid_cursor",
            Self::TraceNotFound => "trace_not_found",
            Self::TraceApiUnavailable => "trace_api_unavailable",
            Self::IncompatibleVersion => "incompatible_version",
        }
    }
}

/// A public Trace API failure with a stable code and deliberately generic text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddedTraceApiError {
    /// Machine-readable failure classification.
    pub code: EmbeddedTraceApiErrorCode,
    /// Safe explanatory text. It never contains runtime error details.
    pub message: &'static str,
}

impl EmbeddedTraceApiError {
    fn new(code: EmbeddedTraceApiErrorCode) -> Self {
        let message = match code {
            EmbeddedTraceApiErrorCode::InvalidCursor => {
                "the trace cursor is invalid for this embedded session"
            }
            EmbeddedTraceApiErrorCode::TraceNotFound => {
                "the requested trace is not retained by this embedded session"
            }
            EmbeddedTraceApiErrorCode::TraceApiUnavailable => {
                "the embedded Trace API is unavailable because the host is stopped"
            }
            EmbeddedTraceApiErrorCode::IncompatibleVersion => {
                "the requested embedded Trace API version is not supported"
            }
        };
        Self { code, message }
    }
}

/// The safe terminal outcome exposed by the public embedded Trace API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddedTraceOutcome {
    /// Runtime execution completed successfully.
    Completed,
    /// Runtime execution produced a stable failure classification.
    Error,
}

/// One public phase code in a safe trace projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddedTracePhase {
    /// Stable phase classification; no phase payload or telemetry is exposed.
    pub code: String,
}

/// Safe selected-target evidence for a public trace detail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddedTraceSelectedTarget {
    /// Runtime-selected capability or workflow identity.
    pub target_id: String,
    /// Selected target version when runtime evidence has one.
    pub target_version: Option<String>,
}

/// Safe placement evidence for a public trace detail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddedTracePlacement {
    /// Placement selected by the runtime, rendered as a stable code.
    pub target: String,
}

/// Safe list-oriented record for one completed local execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddedTraceSummary {
    /// Opaque public identifier scoped to this embedded session.
    pub trace_id: String,
    /// Runtime execution identifier, or a derived workflow execution identifier.
    pub execution_id: String,
    /// Submitted bundled target identity.
    pub target_id: String,
    /// Deterministic session-local completion time in UTC representation.
    pub completed_at: String,
    /// Monotonic completion evidence used for deterministic ordering.
    pub completion_sequence: u64,
    /// Safe terminal outcome.
    pub outcome: EmbeddedTraceOutcome,
}

/// Safe public diagnostic detail for one retained local trace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddedTraceDetail {
    /// The corresponding list summary.
    pub summary: EmbeddedTraceSummary,
    /// Safe runtime or workflow phase classifications.
    pub phases: Vec<EmbeddedTracePhase>,
    /// Selected target evidence when runtime evidence reached selection.
    pub selected_target: Option<EmbeddedTraceSelectedTarget>,
    /// Selected placement evidence when available.
    pub placement: Option<EmbeddedTracePlacement>,
    /// Stable runtime or traversal failure classification, never error text.
    pub failure_code: Option<String>,
    /// Whether runtime state-machine evidence has no recorded violations.
    pub state_machine_valid: Option<bool>,
}

/// A bounded, cursor-paged public Trace API response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddedTracePage {
    /// Newest-first public trace summaries.
    pub summaries: Vec<EmbeddedTraceSummary>,
    /// Opaque cursor for the following page, if retained results remain.
    pub next_cursor: Option<String>,
    /// Fixed process-local retention capacity advertised to consumers.
    pub retention_limit: usize,
}

/// The additive `embedded-trace-api/1.0.0` companion surface (spec 517).
///
/// This trait intentionally does not alter [`TraverseEmbedderApi`]. A host
/// that implements it provides `trace.list` and `trace.get` over a bounded,
/// process-local safe projection; callers must request the advertised version.
pub trait EmbeddedTraceApi {
    /// Returns the companion API version advertised by this host.
    fn embedded_trace_api_version(&self) -> &'static str;

    /// `trace.list`: returns a deterministic page of safe local summaries.
    fn trace_list(
        &self,
        requested_version: &str,
        page_size: usize,
        cursor: Option<&str>,
    ) -> Result<EmbeddedTracePage, EmbeddedTraceApiError>;

    /// `trace.get`: returns one safe retained local trace detail.
    fn trace_get(
        &self,
        requested_version: &str,
        trace_id: &str,
    ) -> Result<EmbeddedTraceDetail, EmbeddedTraceApiError>;
}

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

struct EmbeddedTraceRecordInput {
    execution_id: String,
    target_id: String,
    outcome: EmbeddedTraceOutcome,
    phases: Vec<EmbeddedTracePhase>,
    selected_target: Option<EmbeddedTraceSelectedTarget>,
    placement: Option<EmbeddedTracePlacement>,
    failure_code: Option<String>,
    state_machine_valid: Option<bool>,
}

fn logical_completion_time(sequence: u64) -> String {
    let minute = (sequence / 60) % 60;
    let second = sequence % 60;
    format!("1970-01-01T00:{minute:02}:{second:02}Z")
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
    trace_session: u64,
    next_trace: u64,
    traces: VecDeque<EmbeddedTraceDetail>,
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
            trace_session: NEXT_EMBEDDED_TRACE_SESSION.fetch_add(1, Ordering::Relaxed),
            next_trace: 0,
            traces: VecDeque::new(),
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

    fn record_trace(&mut self, input: EmbeddedTraceRecordInput) {
        self.next_trace += 1;
        let sequence = self.next_trace;
        let summary = EmbeddedTraceSummary {
            trace_id: format!("embedded-trace-{:08}-{:08}", self.trace_session, sequence),
            execution_id: input.execution_id,
            target_id: input.target_id,
            completed_at: logical_completion_time(sequence),
            completion_sequence: sequence,
            outcome: input.outcome,
        };
        self.traces.push_back(EmbeddedTraceDetail {
            summary,
            phases: input.phases,
            selected_target: input.selected_target,
            placement: input.placement,
            failure_code: input.failure_code,
            state_machine_valid: input.state_machine_valid,
        });
        if self.traces.len() > EMBEDDED_TRACE_RETENTION_LIMIT {
            let _ = self.traces.pop_front();
        }
    }

    fn trace_list(
        &self,
        requested_version: &str,
        page_size: usize,
        cursor: Option<&str>,
    ) -> Result<EmbeddedTracePage, EmbeddedTraceApiError> {
        self.ensure_trace_api_available(requested_version)?;
        let traces = self.newest_traces();
        let start = match cursor {
            None => 0,
            Some(cursor) => self.cursor_start(cursor, &traces)?,
        };
        let page_size = page_size.clamp(1, EMBEDDED_TRACE_MAX_PAGE_SIZE);
        let end = start.saturating_add(page_size).min(traces.len());
        let summaries = traces[start..end]
            .iter()
            .map(|detail| detail.summary.clone())
            .collect::<Vec<_>>();
        let next_cursor =
            (end < traces.len()).then(|| self.cursor_for(&traces[end - 1].summary.trace_id));
        Ok(EmbeddedTracePage {
            summaries,
            next_cursor,
            retention_limit: EMBEDDED_TRACE_RETENTION_LIMIT,
        })
    }

    fn trace_get(
        &self,
        requested_version: &str,
        trace_id: &str,
    ) -> Result<EmbeddedTraceDetail, EmbeddedTraceApiError> {
        self.ensure_trace_api_available(requested_version)?;
        self.traces
            .iter()
            .find(|detail| detail.summary.trace_id == trace_id)
            .cloned()
            .ok_or_else(|| EmbeddedTraceApiError::new(EmbeddedTraceApiErrorCode::TraceNotFound))
    }

    fn ensure_trace_api_available(
        &self,
        requested_version: &str,
    ) -> Result<(), EmbeddedTraceApiError> {
        if self.stopped {
            return Err(EmbeddedTraceApiError::new(
                EmbeddedTraceApiErrorCode::TraceApiUnavailable,
            ));
        }
        if requested_version != EMBEDDED_TRACE_API_VERSION {
            return Err(EmbeddedTraceApiError::new(
                EmbeddedTraceApiErrorCode::IncompatibleVersion,
            ));
        }
        Ok(())
    }

    fn newest_traces(&self) -> Vec<&EmbeddedTraceDetail> {
        let mut traces = self.traces.iter().collect::<Vec<_>>();
        traces.sort_by(|left, right| {
            right
                .summary
                .completion_sequence
                .cmp(&left.summary.completion_sequence)
                .then_with(|| left.summary.trace_id.cmp(&right.summary.trace_id))
        });
        traces
    }

    fn cursor_for(&self, trace_id: &str) -> String {
        format!("embedded-trace-cursor:{}:{trace_id}", self.trace_session)
    }

    fn cursor_start(
        &self,
        cursor: &str,
        traces: &[&EmbeddedTraceDetail],
    ) -> Result<usize, EmbeddedTraceApiError> {
        let Some((prefix, trace_id)) = cursor.rsplit_once(':') else {
            return Err(EmbeddedTraceApiError::new(
                EmbeddedTraceApiErrorCode::InvalidCursor,
            ));
        };
        let expected_prefix = format!("embedded-trace-cursor:{}", self.trace_session);
        if prefix != expected_prefix {
            return Err(EmbeddedTraceApiError::new(
                EmbeddedTraceApiErrorCode::InvalidCursor,
            ));
        }
        traces
            .iter()
            .position(|detail| detail.summary.trace_id == trace_id)
            .map(|position| position + 1)
            .ok_or_else(|| EmbeddedTraceApiError::new(EmbeddedTraceApiErrorCode::InvalidCursor))
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
        self.traces.clear();
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
            "companion_apis": {
                "embedded-trace-api": EMBEDDED_TRACE_API_VERSION,
            },
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

        self.core
            .record_trace(workflow_trace_input(&outcome, target_id, &workflow_version));

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

        self.core
            .record_trace(runtime_trace_input(&outcome, target_id));

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

impl EmbeddedTraceApi for BundleEmbedder {
    fn embedded_trace_api_version(&self) -> &'static str {
        EMBEDDED_TRACE_API_VERSION
    }

    fn trace_list(
        &self,
        requested_version: &str,
        page_size: usize,
        cursor: Option<&str>,
    ) -> Result<EmbeddedTracePage, EmbeddedTraceApiError> {
        self.core.trace_list(requested_version, page_size, cursor)
    }

    fn trace_get(
        &self,
        requested_version: &str,
        trace_id: &str,
    ) -> Result<EmbeddedTraceDetail, EmbeddedTraceApiError> {
        self.core.trace_get(requested_version, trace_id)
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

fn runtime_trace_input(
    outcome: &RuntimeExecutionOutcome,
    target_id: &str,
) -> EmbeddedTraceRecordInput {
    let phases = outcome
        .trace
        .state_progression
        .transitions
        .iter()
        .map(|transition| EmbeddedTracePhase {
            code: runtime_state_code(transition.to_state).to_string(),
        })
        .collect();
    let selected_target =
        outcome
            .trace
            .selection
            .selected_capability_id
            .as_ref()
            .map(|target_id| EmbeddedTraceSelectedTarget {
                target_id: target_id.clone(),
                target_version: outcome.trace.selection.selected_capability_version.clone(),
            });
    let placement = outcome
        .trace
        .execution
        .placement
        .selected_target
        .map(|target| EmbeddedTracePlacement {
            target: placement_target_code(target).to_string(),
        });
    let failure_code = outcome
        .result
        .error
        .as_ref()
        .map(|error| runtime_error_code_str(error.code).to_string())
        .or_else(|| {
            outcome
                .trace
                .execution
                .failure_reason
                .map(|reason| execution_failure_code(reason).to_string())
        });
    EmbeddedTraceRecordInput {
        execution_id: outcome.result.execution_id.clone(),
        target_id: target_id.to_string(),
        outcome: match outcome.result.status {
            RuntimeResultStatus::Completed => EmbeddedTraceOutcome::Completed,
            RuntimeResultStatus::Error => EmbeddedTraceOutcome::Error,
        },
        phases,
        selected_target,
        placement,
        failure_code,
        state_machine_valid: Some(outcome.trace.state_machine_validation.violations.is_empty()),
    }
}

fn workflow_trace_input(
    outcome: &WorkflowExecutionOutcome,
    target_id: &str,
    workflow_version: &str,
) -> EmbeddedTraceRecordInput {
    let phases = outcome
        .evidence
        .visited_nodes
        .iter()
        .map(|step| EmbeddedTracePhase {
            code: format!("workflow_{}", workflow_step_status_str(step.status)),
        })
        .collect();
    EmbeddedTraceRecordInput {
        execution_id: format!("workflow-{}", outcome.result.request_id),
        target_id: target_id.to_string(),
        outcome: match outcome.result.status {
            WorkflowTraversalStatus::Completed => EmbeddedTraceOutcome::Completed,
            WorkflowTraversalStatus::Error => EmbeddedTraceOutcome::Error,
        },
        phases,
        selected_target: Some(EmbeddedTraceSelectedTarget {
            target_id: target_id.to_string(),
            target_version: Some(workflow_version.to_string()),
        }),
        placement: None,
        failure_code: outcome
            .result
            .error
            .as_ref()
            .map(|error| runtime_error_code_str(error.code).to_string()),
        state_machine_valid: None,
    }
}

fn runtime_state_code(state: traverse_runtime::RuntimeState) -> &'static str {
    match state {
        traverse_runtime::RuntimeState::Idle => "idle",
        traverse_runtime::RuntimeState::LoadingRegistry => "loading_registry",
        traverse_runtime::RuntimeState::Ready => "ready",
        traverse_runtime::RuntimeState::Discovering => "discovering",
        traverse_runtime::RuntimeState::EvaluatingConstraints => "evaluating_constraints",
        traverse_runtime::RuntimeState::Selecting => "selecting",
        traverse_runtime::RuntimeState::Executing => "executing",
        traverse_runtime::RuntimeState::EmittingEvents => "emitting_events",
        traverse_runtime::RuntimeState::Completed => "completed",
        traverse_runtime::RuntimeState::Error => "error",
    }
}

fn placement_target_code(target: PlacementTarget) -> &'static str {
    match target {
        PlacementTarget::Local => "local",
        PlacementTarget::Browser => "browser",
        PlacementTarget::Edge => "edge",
        PlacementTarget::Cloud => "cloud",
        PlacementTarget::Worker => "worker",
        PlacementTarget::Device => "device",
    }
}

fn execution_failure_code(reason: ExecutionFailureReason) -> &'static str {
    match reason {
        ExecutionFailureReason::ContractInputInvalid => "contract_input_invalid",
        ExecutionFailureReason::ArtifactMissing => "artifact_missing",
        ExecutionFailureReason::ArtifactNotRunnable => "artifact_not_runnable",
        ExecutionFailureReason::PlacementUnsupported => "placement_unsupported",
        ExecutionFailureReason::ExecutionFailed => "execution_failed",
        ExecutionFailureReason::ContractOutputInvalid => "contract_output_invalid",
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

    #[test]
    fn embedded_trace_api_pages_safe_test_double_records() -> Result<(), String> {
        let secret_input = "input-secret-never-public";
        let secret_output = "output-secret-never-public";
        let secret_error = "error-secret-never-public";
        let mut embedder = EmbedderTestDouble::new("workspace", "app", "1.0.0", "web")
            .with_target_output("demo.success", json!({ "secret": secret_output }))
            .with_target_error("demo.failure", "execution_failed", secret_error);

        let accepted = embedder.submit("demo.success", &json!({ "secret": secret_input }));
        assert_eq!(accepted.status, SubmitStatus::Accepted);
        let accepted = embedder.submit("demo.failure", &json!({ "secret": secret_input }));
        assert_eq!(accepted.status, SubmitStatus::Accepted);

        let first_page = embedder
            .trace_list(EMBEDDED_TRACE_API_VERSION, 1, None)
            .map_err(|error| error.message.to_string())?;
        assert_eq!(first_page.retention_limit, EMBEDDED_TRACE_RETENTION_LIMIT);
        assert_eq!(first_page.summaries.len(), 1);
        assert_eq!(first_page.summaries[0].target_id, "demo.failure");
        assert_eq!(first_page.summaries[0].outcome, EmbeddedTraceOutcome::Error);
        let cursor = first_page
            .next_cursor
            .ok_or("the first page should have a continuation cursor")?;
        let failure = embedder
            .trace_get(
                EMBEDDED_TRACE_API_VERSION,
                &first_page.summaries[0].trace_id,
            )
            .map_err(|error| error.message.to_string())?;
        assert_eq!(failure.failure_code.as_deref(), Some("execution_failed"));
        assert_eq!(failure.phases[0].code, "error");
        let safe_debug = format!("{failure:?}");
        assert!(!safe_debug.contains(secret_input));
        assert!(!safe_debug.contains(secret_output));
        assert!(!safe_debug.contains(secret_error));

        let second_page = embedder
            .trace_list(EMBEDDED_TRACE_API_VERSION, 1, Some(&cursor))
            .map_err(|error| error.message.to_string())?;
        assert_eq!(second_page.summaries.len(), 1);
        assert_eq!(second_page.summaries[0].target_id, "demo.success");
        assert!(second_page.next_cursor.is_none());
        Ok(())
    }

    #[test]
    fn embedded_trace_api_rejects_stale_versions_cursors_and_stopped_hosts() -> Result<(), String> {
        let mut embedder = EmbedderTestDouble::new("workspace", "app", "1.0.0", "web")
            .with_target_output("demo.success", json!({ "value": "safe" }));
        let _ = embedder.submit("demo.success", &json!({}));

        let version_error = embedder
            .trace_list("2.0.0", 10, None)
            .err()
            .ok_or("an incompatible version should fail")?;
        assert_eq!(
            version_error.code,
            EmbeddedTraceApiErrorCode::IncompatibleVersion
        );
        let cursor_error = embedder
            .trace_list(EMBEDDED_TRACE_API_VERSION, 10, Some("not-a-cursor"))
            .err()
            .ok_or("a malformed cursor should fail")?;
        assert_eq!(cursor_error.code, EmbeddedTraceApiErrorCode::InvalidCursor);

        let _ = embedder.shutdown();
        let stopped_error = embedder
            .trace_list(EMBEDDED_TRACE_API_VERSION, 10, None)
            .err()
            .ok_or("a stopped host should be unavailable")?;
        assert_eq!(
            stopped_error.code,
            EmbeddedTraceApiErrorCode::TraceApiUnavailable
        );
        Ok(())
    }

    #[test]
    fn embedded_trace_api_evicts_oldest_records_deterministically() -> Result<(), String> {
        let mut embedder = EmbedderTestDouble::new("workspace", "app", "1.0.0", "web")
            .with_target_output("demo.success", json!({ "value": "safe" }));
        let mut first_trace_id = None;
        for index in 0..=EMBEDDED_TRACE_RETENTION_LIMIT {
            let _ = embedder.submit("demo.success", &json!({ "index": index }));
            if index == 0 {
                first_trace_id = embedder
                    .trace_list(EMBEDDED_TRACE_API_VERSION, 1, None)
                    .map_err(|error| error.message.to_string())?
                    .summaries
                    .first()
                    .map(|summary| summary.trace_id.clone());
            }
        }
        let first_trace_id =
            first_trace_id.ok_or("the first trace should be retained initially")?;
        let retained = embedder
            .trace_list(
                EMBEDDED_TRACE_API_VERSION,
                EMBEDDED_TRACE_RETENTION_LIMIT,
                None,
            )
            .map_err(|error| error.message.to_string())?;
        assert_eq!(retained.summaries.len(), EMBEDDED_TRACE_RETENTION_LIMIT);
        assert_eq!(retained.summaries[0].completion_sequence, 101);
        let evicted = embedder
            .trace_get(EMBEDDED_TRACE_API_VERSION, &first_trace_id)
            .err()
            .ok_or("the oldest record should have been evicted")?;
        assert_eq!(evicted.code, EmbeddedTraceApiErrorCode::TraceNotFound);
        Ok(())
    }
}
