//! Spec `135-component-model-wit-host-capabilities`: `component-wit-v1` profile.
//!
//! Validates declared Component Model WIT imports and resolves them only through
//! explicitly activated target-local host bindings. Host ABI v1 / `core-wasm-v1`
//! behavior is unchanged.

use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;

/// Unchanged Host ABI v1 / core Wasm profile.
pub const CORE_WASM_V1: &str = "core-wasm-v1";
/// Additive Component Model WIT execution profile.
pub const COMPONENT_WIT_V1: &str = "component-wit-v1";
/// First standard Traverse host-capability package.
pub const RECORDING_HOST_PACKAGE: &str = "traverse:platform";
/// First standard Traverse recording interface.
pub const RECORDING_HOST_INTERFACE: &str = "recording-host";
/// Exact v0.1.0 interface version. Semver ranges are forbidden.
pub const RECORDING_HOST_VERSION: &str = "0.1.0";

const GOVERNING_SPEC: &str = "135-component-model-wit-host-capabilities";
const MAX_EVENT_QUEUE: usize = 8;

/// Exact WIT import identity. No aliases, ranges, or name-only matching.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct WitImportIdentity {
    /// WIT package namespace/name, e.g. `traverse:platform`.
    pub package: String,
    /// Interface name, e.g. `recording-host`.
    pub interface: String,
    /// Exact interface version, e.g. `0.1.0`.
    pub version: String,
}

impl WitImportIdentity {
    /// Standard `traverse:platform/recording-host@0.1.0`.
    #[must_use]
    pub fn recording_host() -> Self {
        Self {
            package: RECORDING_HOST_PACKAGE.to_string(),
            interface: RECORDING_HOST_INTERFACE.to_string(),
            version: RECORDING_HOST_VERSION.to_string(),
        }
    }

    /// Wire form `package/interface@version`.
    #[must_use]
    pub fn key(&self) -> String {
        format!("{}/{}@{}", self.package, self.interface, self.version)
    }

    fn is_callweave_recording(&self) -> bool {
        self.package == "callweave:recording" && self.interface == "recording-host"
    }
}

/// Component type metadata: imported WIT identities and exported operation names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentWitMetadata {
    /// Every imported WIT package/interface/version observed in the component.
    pub imports: BTreeSet<WitImportIdentity>,
    /// Exported WIT operations (informational; not optional privileged imports).
    pub exported_operations: BTreeSet<String>,
}

/// Package manifest declaration for `component-wit-v1`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentWitManifest {
    /// Requested execution profile.
    pub execution_profile: String,
    /// Required WIT imports. Must match component metadata exactly.
    pub required_wit_imports: Vec<WitImportIdentity>,
    /// Explicit binding selection keyed by [`WitImportIdentity::key`].
    pub wit_bindings: BTreeMap<String, String>,
}

/// Host implementation registration. Does not activate native authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostBindingRegistration {
    /// Stable binding id selected by the application.
    pub binding_id: String,
    /// Exact WIT identity this binding implements.
    pub identity: WitImportIdentity,
    /// Precise target families the host claims, e.g. `local`.
    pub target_families: BTreeSet<String>,
    /// Trust / conformance classification.
    pub trust: HostTrustState,
    /// Conformance-evidence reference. Required for official verified hosts.
    pub conformance_evidence_ref: Option<String>,
}

/// Distinguishes official verified hosts from application-local fakes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostTrustState {
    /// Official Traverse implementation with shared + target evidence.
    OfficialVerified,
    /// Application-local host. Never a portable verified default.
    ApplicationLocalUnverified,
}

impl HostTrustState {
    fn as_str(self) -> &'static str {
        match self {
            Self::OfficialVerified => "official_verified",
            Self::ApplicationLocalUnverified => "application_local_unverified",
        }
    }
}

/// Stable Spec 135 FR-008 codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComponentWitErrorCode {
    /// Profile is not `component-wit-v1` or mismatches the requested profile.
    ComponentModelProfileUnsupported,
    /// Component import is absent from the manifest declaration.
    WitImportUndeclared,
    /// Manifest declaration is absent from component metadata.
    WitImportUnfulfilled,
    /// Package/interface/version/operation-shape is not an exact match.
    WitImportVersionIncompatible,
    /// Activated binding does not claim the requested target family.
    WitHostTargetMismatch,
    /// More than one compatible binding and no explicit selection.
    WitHostBindingAmbiguous,
    /// Registration missing, untrusted for the requested use, or activate failed.
    WitHostActivationFailed,
}

impl ComponentWitErrorCode {
    /// Stable `snake_case` wire code.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ComponentModelProfileUnsupported => "component_model_profile_unsupported",
            Self::WitImportUndeclared => "wit_import_undeclared",
            Self::WitImportUnfulfilled => "wit_import_unfulfilled",
            Self::WitImportVersionIncompatible => "wit_import_version_incompatible",
            Self::WitHostTargetMismatch => "wit_host_target_mismatch",
            Self::WitHostBindingAmbiguous => "wit_host_binding_ambiguous",
            Self::WitHostActivationFailed => "wit_host_activation_failed",
        }
    }
}

/// Secret-free Component WIT failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentWitError {
    /// Stable public code.
    pub code: ComponentWitErrorCode,
    /// Human-readable explanation without host-private data.
    pub message: String,
}

impl ComponentWitError {
    fn new(code: ComponentWitErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for ComponentWitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for ComponentWitError {}

/// Activation failure with redacted public evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentWitFailure {
    /// Stable public error.
    pub error: ComponentWitError,
    /// Redacted evidence for the failed attempt.
    pub evidence: ComponentWitEvidence,
}

fn failure(
    code: ComponentWitErrorCode,
    message: impl Into<String>,
    declared: &[WitImportIdentity],
) -> Box<ComponentWitFailure> {
    Box::new(ComponentWitFailure {
        error: ComponentWitError::new(code, message),
        evidence: ComponentWitEvidence::failure(code, declared),
    })
}

/// Redacted activation / invocation evidence (FR-009).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentWitEvidence {
    /// Selected execution profile.
    pub profile: String,
    /// Declared import identities.
    pub declared_imports: Vec<String>,
    /// Selected binding id, if activation succeeded.
    pub selected_binding_id: Option<String>,
    /// Abstract target family.
    pub target_family: Option<String>,
    /// Compatibility result.
    pub compatibility: String,
    /// Stable outcome code.
    pub outcome: String,
    /// Trust classification of the selected binding.
    pub trust: Option<String>,
}

impl ComponentWitEvidence {
    fn failure(code: ComponentWitErrorCode, declared: &[WitImportIdentity]) -> Self {
        Self {
            profile: COMPONENT_WIT_V1.to_string(),
            declared_imports: declared.iter().map(WitImportIdentity::key).collect(),
            selected_binding_id: None,
            target_family: None,
            compatibility: "failed".to_string(),
            outcome: code.as_str().to_string(),
            trust: None,
        }
    }

    /// Public JSON without host-private fields.
    #[must_use]
    pub fn public_json(&self) -> Value {
        json!({
            "kind": "component_wit_evidence",
            "governing_spec": GOVERNING_SPEC,
            "profile": self.profile,
            "declared_imports": self.declared_imports,
            "selected_binding_id": self.selected_binding_id,
            "target_family": self.target_family,
            "compatibility": self.compatibility,
            "outcome": self.outcome,
            "trust": self.trust,
        })
    }
}

/// Portable recording-host domain outcomes (never host-private strings).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordingOutcome {
    /// Advisory portable status.
    Status { state: String },
    /// Recording started; opaque host-managed reference.
    Started { recording_ref: String },
    /// Recording stopped; reference ready for the next host-authorized use.
    Stopped { recording_ref: String },
    /// Bounded lifecycle event.
    Event { kind: String },
    /// Public typed domain failure.
    Domain { code: RecordingDomainCode },
}

/// Portable recording domain codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordingDomainCode {
    /// Start was not user-initiated and foregrounded.
    PermissionRequired,
    /// Background recording is out of scope.
    BackgroundUnsupported,
    /// One active session already exists for this host context.
    RecordingBusy,
    /// Host cannot record right now.
    Unavailable,
}

impl RecordingDomainCode {
    fn as_str(self) -> &'static str {
        match self {
            Self::PermissionRequired => "permission-required",
            Self::BackgroundUnsupported => "background-unsupported",
            Self::RecordingBusy => "recording-busy",
            Self::Unavailable => "unavailable",
        }
    }
}

impl RecordingOutcome {
    /// WIT-defined public JSON. No audio, paths, devices, or native errors.
    #[must_use]
    pub fn public_json(&self) -> Value {
        match self {
            Self::Status { state } => json!({"op": "status", "state": state}),
            Self::Started { recording_ref } => {
                json!({"op": "start", "recording_ref": recording_ref})
            }
            Self::Stopped { recording_ref } => {
                json!({"op": "stop", "recording_ref": recording_ref})
            }
            Self::Event { kind } => json!({"op": "events", "kind": kind}),
            Self::Domain { code } => json!({"op": "domain", "code": code.as_str()}),
        }
    }
}

/// Foreground recording host used by tests and application-local fakes.
pub trait RecordingHost {
    /// Advisory portable status.
    fn status(&self) -> RecordingOutcome;
    /// Authoritative start. `foregrounded && user_initiated` may prompt.
    fn start(&mut self, foregrounded: bool, user_initiated: bool) -> RecordingOutcome;
    /// Stop the named opaque reference.
    fn stop(&mut self, recording_ref: &str) -> RecordingOutcome;
    /// Pop one bounded lifecycle event, if any.
    fn next_event(&mut self) -> Option<RecordingOutcome>;
}

/// In-process fake `traverse:platform/recording-host@0.1.0`.
#[derive(Debug)]
pub struct FakeRecordingHost {
    active: Option<String>,
    next_id: u64,
    events: VecDeque<String>,
    unavailable: bool,
}

impl Default for FakeRecordingHost {
    fn default() -> Self {
        Self {
            active: None,
            next_id: 1,
            events: VecDeque::new(),
            unavailable: false,
        }
    }
}

impl FakeRecordingHost {
    /// Construct an available fake host.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Force the next start to return `unavailable`.
    pub fn set_unavailable(&mut self, unavailable: bool) {
        self.unavailable = unavailable;
    }

    fn push_event(&mut self, kind: &str) {
        if self.events.len() == MAX_EVENT_QUEUE {
            self.events.pop_front();
        }
        self.events.push_back(kind.to_string());
    }
}

impl RecordingHost for FakeRecordingHost {
    fn status(&self) -> RecordingOutcome {
        let state = if self.active.is_some() {
            "recording"
        } else if self.unavailable {
            "unavailable"
        } else {
            "idle"
        };
        RecordingOutcome::Status {
            state: state.to_string(),
        }
    }

    fn start(&mut self, foregrounded: bool, user_initiated: bool) -> RecordingOutcome {
        if self.unavailable {
            return RecordingOutcome::Domain {
                code: RecordingDomainCode::Unavailable,
            };
        }
        if !foregrounded {
            return RecordingOutcome::Domain {
                code: RecordingDomainCode::BackgroundUnsupported,
            };
        }
        if !user_initiated {
            return RecordingOutcome::Domain {
                code: RecordingDomainCode::PermissionRequired,
            };
        }
        if self.active.is_some() {
            return RecordingOutcome::Domain {
                code: RecordingDomainCode::RecordingBusy,
            };
        }
        let recording_ref = format!("rec-{}", self.next_id);
        self.next_id += 1;
        self.active = Some(recording_ref.clone());
        self.push_event("started");
        RecordingOutcome::Started { recording_ref }
    }

    fn stop(&mut self, recording_ref: &str) -> RecordingOutcome {
        match self.active.as_deref() {
            Some(active) if active == recording_ref => {
                self.active = None;
                self.push_event("stopped");
                RecordingOutcome::Stopped {
                    recording_ref: recording_ref.to_string(),
                }
            }
            _ => RecordingOutcome::Domain {
                code: RecordingDomainCode::Unavailable,
            },
        }
    }

    fn next_event(&mut self) -> Option<RecordingOutcome> {
        self.events
            .pop_front()
            .map(|kind| RecordingOutcome::Event { kind })
    }
}

/// Registry of host bindings plus explicit application activations.
#[derive(Debug, Default)]
pub struct ComponentWitActivation {
    registrations: BTreeMap<String, HostBindingRegistration>,
}

impl ComponentWitActivation {
    /// Empty registry. Registry-declared hosts are not auto-activated.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a host implementation. Does not grant authority.
    pub fn register(&mut self, registration: HostBindingRegistration) {
        self.registrations
            .insert(registration.binding_id.clone(), registration);
    }

    /// Validate profile, exact import match, and explicit activation.
    ///
    /// # Errors
    ///
    /// Returns a Spec 135 FR-008 code when the profile, imports, target, or
    /// binding cannot be activated before guest execution.
    pub fn activate(
        &self,
        manifest: &ComponentWitManifest,
        metadata: &ComponentWitMetadata,
        target_family: &str,
    ) -> Result<ComponentWitEvidence, Box<ComponentWitFailure>> {
        if manifest.execution_profile != COMPONENT_WIT_V1 {
            return Err(failure(
                ComponentWitErrorCode::ComponentModelProfileUnsupported,
                "execution profile must be component-wit-v1",
                &manifest.required_wit_imports,
            ));
        }
        if let Some(error) = validate_exact_imports(manifest, metadata) {
            return Err(Box::new(ComponentWitFailure {
                evidence: ComponentWitEvidence::failure(error.code, &manifest.required_wit_imports),
                error,
            }));
        }
        let required = &manifest.required_wit_imports;
        if required.len() != 1 {
            return Err(failure(
                ComponentWitErrorCode::WitHostActivationFailed,
                "v0.1.0 activates exactly one required WIT import",
                required,
            ));
        }
        let identity = &required[0];
        if identity.is_callweave_recording() {
            return Err(failure(
                ComponentWitErrorCode::WitImportVersionIncompatible,
                "callweave recording WIT is not a Traverse-standard default",
                required,
            ));
        }
        let registration = self.select_binding(manifest, identity, required)?;
        confirm_binding(&registration, identity, target_family, required)?;
        Ok(ComponentWitEvidence {
            profile: COMPONENT_WIT_V1.to_string(),
            declared_imports: required.iter().map(WitImportIdentity::key).collect(),
            selected_binding_id: Some(registration.binding_id.clone()),
            target_family: Some(target_family.to_string()),
            compatibility: "exact".to_string(),
            outcome: "activated".to_string(),
            trust: Some(registration.trust.as_str().to_string()),
        })
    }

    fn select_binding(
        &self,
        manifest: &ComponentWitManifest,
        identity: &WitImportIdentity,
        required: &[WitImportIdentity],
    ) -> Result<HostBindingRegistration, Box<ComponentWitFailure>> {
        let selected = if let Some(binding_id) = manifest.wit_bindings.get(&identity.key()) {
            self.registrations.get(binding_id).cloned()
        } else {
            let matches: Vec<_> = self
                .registrations
                .values()
                .filter(|registration| registration.identity == *identity)
                .collect();
            if matches.len() > 1 {
                return Err(failure(
                    ComponentWitErrorCode::WitHostBindingAmbiguous,
                    "application must select an explicit binding",
                    required,
                ));
            }
            matches.first().map(|registration| (*registration).clone())
        };
        selected.ok_or_else(|| {
            failure(
                ComponentWitErrorCode::WitHostActivationFailed,
                "no registered host binding is activated for the declared import",
                required,
            )
        })
    }
}

fn confirm_binding(
    registration: &HostBindingRegistration,
    identity: &WitImportIdentity,
    target_family: &str,
    required: &[WitImportIdentity],
) -> Result<(), Box<ComponentWitFailure>> {
    if registration.identity != *identity {
        return Err(failure(
            ComponentWitErrorCode::WitImportVersionIncompatible,
            "selected binding identity is not an exact WIT match",
            required,
        ));
    }
    if !registration.target_families.contains(target_family) {
        return Err(failure(
            ComponentWitErrorCode::WitHostTargetMismatch,
            "activated binding does not claim the requested target family",
            required,
        ));
    }
    if registration.trust == HostTrustState::OfficialVerified
        && registration.conformance_evidence_ref.is_none()
    {
        return Err(failure(
            ComponentWitErrorCode::WitHostActivationFailed,
            "official host is missing conformance evidence",
            required,
        ));
    }
    Ok(())
}

/// Reject `component-wit-v1` binaries on the core-Wasm Host ABI v1 path.
///
/// # Errors
///
/// Returns [`ComponentWitErrorCode::ComponentModelProfileUnsupported`] when the
/// requested profile is not `core-wasm-v1`.
pub fn require_core_wasm_profile(requested_profile: Option<&str>) -> Result<(), ComponentWitError> {
    match requested_profile {
        None | Some(CORE_WASM_V1) => Ok(()),
        Some(_) => Err(ComponentWitError::new(
            ComponentWitErrorCode::ComponentModelProfileUnsupported,
            "core-wasm-v1 Host ABI path does not execute component-wit-v1",
        )),
    }
}

fn validate_exact_imports(
    manifest: &ComponentWitManifest,
    metadata: &ComponentWitMetadata,
) -> Option<ComponentWitError> {
    let declared: BTreeSet<_> = manifest.required_wit_imports.iter().cloned().collect();
    for import in &metadata.imports {
        if !declared.contains(import) {
            return Some(ComponentWitError::new(
                ComponentWitErrorCode::WitImportUndeclared,
                "component WIT import is not declared by the package manifest",
            ));
        }
        if import.version != RECORDING_HOST_VERSION
            && import.package == RECORDING_HOST_PACKAGE
            && import.interface == RECORDING_HOST_INTERFACE
        {
            return Some(ComponentWitError::new(
                ComponentWitErrorCode::WitImportVersionIncompatible,
                "WIT interface version must match exactly",
            ));
        }
    }
    for required in &manifest.required_wit_imports {
        if !metadata.imports.contains(required) {
            return Some(ComponentWitError::new(
                ComponentWitErrorCode::WitImportUnfulfilled,
                "declared WIT import is missing from component metadata",
            ));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn recording_manifest(binding: Option<&str>) -> ComponentWitManifest {
        let identity = WitImportIdentity::recording_host();
        let mut wit_bindings = BTreeMap::new();
        if let Some(binding_id) = binding {
            wit_bindings.insert(identity.key(), binding_id.to_string());
        }
        ComponentWitManifest {
            execution_profile: COMPONENT_WIT_V1.to_string(),
            required_wit_imports: vec![identity],
            wit_bindings,
        }
    }

    fn recording_metadata() -> ComponentWitMetadata {
        ComponentWitMetadata {
            imports: BTreeSet::from([WitImportIdentity::recording_host()]),
            exported_operations: BTreeSet::from([
                "status".to_string(),
                "start".to_string(),
                "stop".to_string(),
                "events".to_string(),
            ]),
        }
    }

    fn local_fake_registration() -> HostBindingRegistration {
        HostBindingRegistration {
            binding_id: "default-local-recording".to_string(),
            identity: WitImportIdentity::recording_host(),
            target_families: BTreeSet::from(["local".to_string()]),
            trust: HostTrustState::ApplicationLocalUnverified,
            conformance_evidence_ref: None,
        }
    }

    fn activate_ok() -> ComponentWitEvidence {
        let mut registry = ComponentWitActivation::new();
        registry.register(local_fake_registration());
        registry
            .activate(
                &recording_manifest(Some("default-local-recording")),
                &recording_metadata(),
                "local",
            )
            .expect("activate")
    }

    #[test]
    fn core_wasm_profile_remains_the_default_and_rejects_component_wit() {
        require_core_wasm_profile(None).unwrap();
        require_core_wasm_profile(Some(CORE_WASM_V1)).unwrap();
        let error = require_core_wasm_profile(Some(COMPONENT_WIT_V1)).unwrap_err();
        assert_eq!(
            error.code,
            ComponentWitErrorCode::ComponentModelProfileUnsupported
        );
        assert_eq!(
            error.to_string(),
            format!("{}: {}", error.code.as_str(), error.message)
        );
    }

    #[test]
    fn activate_records_redacted_evidence_for_the_fake_local_host() {
        let evidence = activate_ok();
        assert_eq!(evidence.profile, COMPONENT_WIT_V1);
        assert_eq!(
            evidence.selected_binding_id.as_deref(),
            Some("default-local-recording")
        );
        assert_eq!(evidence.target_family.as_deref(), Some("local"));
        assert_eq!(evidence.compatibility, "exact");
        assert_eq!(evidence.outcome, "activated");
        assert_eq!(
            evidence.trust.as_deref(),
            Some(HostTrustState::ApplicationLocalUnverified.as_str())
        );
        let public = evidence.public_json().to_string();
        assert!(public.contains(GOVERNING_SPEC));
        assert!(!public.contains("microphone"));
        assert!(!public.contains("callweave"));
    }

    #[test]
    fn unsupported_profile_fails_before_activation() {
        let mut manifest = recording_manifest(Some("default-local-recording"));
        manifest.execution_profile = CORE_WASM_V1.to_string();
        let mut registry = ComponentWitActivation::new();
        registry.register(local_fake_registration());
        let failed = registry
            .activate(&manifest, &recording_metadata(), "local")
            .expect_err("profile");
        assert_eq!(
            failed.error.code,
            ComponentWitErrorCode::ComponentModelProfileUnsupported
        );
        assert_eq!(
            failed.evidence.outcome,
            "component_model_profile_unsupported"
        );
    }

    #[test]
    fn undeclared_component_import_fails_closed() {
        let metadata = ComponentWitMetadata {
            imports: BTreeSet::from([
                WitImportIdentity::recording_host(),
                WitImportIdentity {
                    package: RECORDING_HOST_PACKAGE.to_string(),
                    interface: "storage-host".to_string(),
                    version: RECORDING_HOST_VERSION.to_string(),
                },
            ]),
            exported_operations: BTreeSet::new(),
        };
        let mut registry = ComponentWitActivation::new();
        registry.register(local_fake_registration());
        let failed = registry
            .activate(
                &recording_manifest(Some("default-local-recording")),
                &metadata,
                "local",
            )
            .expect_err("undeclared");
        assert_eq!(
            failed.error.code,
            ComponentWitErrorCode::WitImportUndeclared
        );
    }

    #[test]
    fn unfulfilled_manifest_import_fails_closed() {
        let metadata = ComponentWitMetadata {
            imports: BTreeSet::new(),
            exported_operations: BTreeSet::new(),
        };
        let mut registry = ComponentWitActivation::new();
        registry.register(local_fake_registration());
        let failed = registry
            .activate(
                &recording_manifest(Some("default-local-recording")),
                &metadata,
                "local",
            )
            .expect_err("unfulfilled");
        assert_eq!(
            failed.error.code,
            ComponentWitErrorCode::WitImportUnfulfilled
        );
    }

    #[test]
    fn version_mismatch_and_callweave_identity_are_not_aliases() {
        let mut metadata = recording_metadata();
        metadata.imports = BTreeSet::from([WitImportIdentity {
            package: RECORDING_HOST_PACKAGE.to_string(),
            interface: RECORDING_HOST_INTERFACE.to_string(),
            version: "0.2.0".to_string(),
        }]);
        let mut manifest = recording_manifest(Some("default-local-recording"));
        manifest.required_wit_imports = metadata.imports.iter().cloned().collect();
        let mut registry = ComponentWitActivation::new();
        registry.register(local_fake_registration());
        let failed = registry
            .activate(&manifest, &metadata, "local")
            .expect_err("version");
        assert_eq!(
            failed.error.code,
            ComponentWitErrorCode::WitImportVersionIncompatible
        );

        let callweave = WitImportIdentity {
            package: "callweave:recording".to_string(),
            interface: "recording-host".to_string(),
            version: RECORDING_HOST_VERSION.to_string(),
        };
        manifest.required_wit_imports = vec![callweave.clone()];
        let metadata = ComponentWitMetadata {
            imports: BTreeSet::from([callweave]),
            exported_operations: BTreeSet::new(),
        };
        let failed = registry
            .activate(&manifest, &metadata, "local")
            .expect_err("callweave");
        assert_eq!(
            failed.error.code,
            ComponentWitErrorCode::WitImportVersionIncompatible
        );
        assert!(
            failed
                .error
                .message
                .contains("not a Traverse-standard default")
        );
    }

    #[test]
    fn missing_host_target_mismatch_and_ambiguous_bindings_fail_before_guest() {
        let registry = ComponentWitActivation::new();
        let failed = registry
            .activate(
                &recording_manifest(Some("default-local-recording")),
                &recording_metadata(),
                "local",
            )
            .expect_err("missing");
        assert_eq!(
            failed.error.code,
            ComponentWitErrorCode::WitHostActivationFailed
        );

        let mut registry = ComponentWitActivation::new();
        registry.register(local_fake_registration());
        let failed = registry
            .activate(
                &recording_manifest(Some("default-local-recording")),
                &recording_metadata(),
                "ios",
            )
            .expect_err("target");
        assert_eq!(
            failed.error.code,
            ComponentWitErrorCode::WitHostTargetMismatch
        );

        let mut second = local_fake_registration();
        second.binding_id = "other-local-recording".to_string();
        registry.register(second);
        let failed = registry
            .activate(&recording_manifest(None), &recording_metadata(), "local")
            .expect_err("ambiguous");
        assert_eq!(
            failed.error.code,
            ComponentWitErrorCode::WitHostBindingAmbiguous
        );
    }

    #[test]
    fn implicit_single_registration_can_activate_and_wrong_binding_identity_fails() {
        let mut registry = ComponentWitActivation::new();
        registry.register(local_fake_registration());
        let evidence = registry
            .activate(&recording_manifest(None), &recording_metadata(), "local")
            .expect("implicit single");
        assert_eq!(
            evidence.selected_binding_id.as_deref(),
            Some("default-local-recording")
        );

        let mut wrong = local_fake_registration();
        wrong.binding_id = "wrong-identity".to_string();
        wrong.identity = WitImportIdentity {
            package: RECORDING_HOST_PACKAGE.to_string(),
            interface: "storage-host".to_string(),
            version: RECORDING_HOST_VERSION.to_string(),
        };
        registry.register(wrong);
        let failed = registry
            .activate(
                &recording_manifest(Some("wrong-identity")),
                &recording_metadata(),
                "local",
            )
            .expect_err("identity");
        assert_eq!(
            failed.error.code,
            ComponentWitErrorCode::WitImportVersionIncompatible
        );
    }

    #[test]
    fn official_verified_host_requires_conformance_evidence() {
        let mut registry = ComponentWitActivation::new();
        let mut official = local_fake_registration();
        official.binding_id = "official-recording".to_string();
        official.trust = HostTrustState::OfficialVerified;
        official.conformance_evidence_ref = None;
        registry.register(official.clone());
        let failed = registry
            .activate(
                &recording_manifest(Some("official-recording")),
                &recording_metadata(),
                "local",
            )
            .expect_err("evidence");
        assert_eq!(
            failed.error.code,
            ComponentWitErrorCode::WitHostActivationFailed
        );

        official.conformance_evidence_ref = Some("wit-contract-tests/recording-host-0.1.0".into());
        registry.register(official);
        let evidence = registry
            .activate(
                &recording_manifest(Some("official-recording")),
                &recording_metadata(),
                "local",
            )
            .expect("verified");
        assert_eq!(
            evidence.trust.as_deref(),
            Some(HostTrustState::OfficialVerified.as_str())
        );
    }

    #[test]
    fn v0_requires_exactly_one_required_import() {
        let mut manifest = recording_manifest(Some("default-local-recording"));
        manifest.required_wit_imports.push(WitImportIdentity {
            package: RECORDING_HOST_PACKAGE.to_string(),
            interface: "storage-host".to_string(),
            version: RECORDING_HOST_VERSION.to_string(),
        });
        let mut metadata = recording_metadata();
        metadata
            .imports
            .insert(manifest.required_wit_imports[1].clone());
        let mut registry = ComponentWitActivation::new();
        registry.register(local_fake_registration());
        let failed = registry
            .activate(&manifest, &metadata, "local")
            .expect_err("one import");
        assert_eq!(
            failed.error.code,
            ComponentWitErrorCode::WitHostActivationFailed
        );
    }

    #[test]
    fn fake_recording_host_covers_status_start_stop_events_and_domain_outcomes() {
        let mut host = FakeRecordingHost::new();
        assert_eq!(
            host.status(),
            RecordingOutcome::Status {
                state: "idle".to_string()
            }
        );
        assert_eq!(
            host.start(false, true),
            RecordingOutcome::Domain {
                code: RecordingDomainCode::BackgroundUnsupported
            }
        );
        assert_eq!(
            host.start(true, false),
            RecordingOutcome::Domain {
                code: RecordingDomainCode::PermissionRequired
            }
        );
        let started = host.start(true, true);
        assert_eq!(
            started,
            RecordingOutcome::Started {
                recording_ref: "rec-1".to_string()
            }
        );
        let recording_ref = "rec-1".to_string();
        assert_eq!(
            host.status(),
            RecordingOutcome::Status {
                state: "recording".to_string()
            }
        );
        assert_eq!(
            host.start(true, true),
            RecordingOutcome::Domain {
                code: RecordingDomainCode::RecordingBusy
            }
        );
        assert_eq!(
            host.next_event(),
            Some(RecordingOutcome::Event {
                kind: "started".to_string()
            })
        );
        assert_eq!(
            host.stop("missing"),
            RecordingOutcome::Domain {
                code: RecordingDomainCode::Unavailable
            }
        );
        assert_eq!(
            host.stop(&recording_ref),
            RecordingOutcome::Stopped {
                recording_ref: recording_ref.clone()
            }
        );
        assert_eq!(
            host.next_event(),
            Some(RecordingOutcome::Event {
                kind: "stopped".to_string()
            })
        );
        assert!(host.next_event().is_none());
        host.set_unavailable(true);
        assert_eq!(
            host.status(),
            RecordingOutcome::Status {
                state: "unavailable".to_string()
            }
        );
        assert_eq!(
            host.start(true, true),
            RecordingOutcome::Domain {
                code: RecordingDomainCode::Unavailable
            }
        );
        for outcome in [
            started,
            RecordingOutcome::Status {
                state: "idle".to_string(),
            },
            RecordingOutcome::Stopped {
                recording_ref: "rec-1".to_string(),
            },
            RecordingOutcome::Event {
                kind: "interrupted".to_string(),
            },
            RecordingOutcome::Domain {
                code: RecordingDomainCode::PermissionRequired,
            },
        ] {
            let encoded = outcome.public_json().to_string();
            assert!(!encoded.contains("microphone"));
            assert!(!encoded.contains("/tmp"));
            assert!(!encoded.contains("AVAudio"));
        }
    }

    #[test]
    fn event_queue_is_bounded_and_error_codes_are_stable() {
        let mut host = FakeRecordingHost::new();
        for _ in 0..MAX_EVENT_QUEUE + 2 {
            let started = host.start(true, true);
            if let RecordingOutcome::Started { recording_ref } = started {
                let _ = host.stop(&recording_ref);
            }
        }
        let mut seen = 0;
        while host.next_event().is_some() {
            seen += 1;
        }
        assert_eq!(seen, MAX_EVENT_QUEUE);

        for code in [
            ComponentWitErrorCode::ComponentModelProfileUnsupported,
            ComponentWitErrorCode::WitImportUndeclared,
            ComponentWitErrorCode::WitImportUnfulfilled,
            ComponentWitErrorCode::WitImportVersionIncompatible,
            ComponentWitErrorCode::WitHostTargetMismatch,
            ComponentWitErrorCode::WitHostBindingAmbiguous,
            ComponentWitErrorCode::WitHostActivationFailed,
        ] {
            assert!(!code.as_str().is_empty());
        }
        for code in [
            RecordingDomainCode::PermissionRequired,
            RecordingDomainCode::BackgroundUnsupported,
            RecordingDomainCode::RecordingBusy,
            RecordingDomainCode::Unavailable,
        ] {
            assert!(!code.as_str().is_empty());
        }
    }
}
