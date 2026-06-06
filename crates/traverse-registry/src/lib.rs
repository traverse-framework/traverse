//! Registry support for Traverse.

mod bundle;
pub mod dependency_resolver;
mod events;
mod federation;
mod graph;
pub mod semver_resolver;
mod workflows;
pub use bundle::*;
pub use dependency_resolver::{
    DigestMismatch, MAX_TRANSITIVE_DEPTH, ResolutionError, ResolvedDependencyLock,
    lookup_lock_record, resolve_dependencies, verify_lock_digests,
};
pub use events::*;
pub use federation::*;
pub use graph::*;
pub use semver_resolver::{
    AmbiguousCandidate, RangeResolutionError, ResolvedRangeCapability, resolve_version_range,
};
pub use workflows::*;

use semver::{Version, VersionReq};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use traverse_contracts::{
    CapabilityContract, ConnectorContract, ConnectorRequirement, ErrorSeverity, EventReference,
    ExecutionTarget, IdReference, Lifecycle, Owner, PublishedContractRecord, ValidationContext,
    ValidationFailure, governed_content_digest, validate_connector_contract, validate_contract,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RegistryScope {
    Public,
    Private,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LookupScope {
    PublicOnly,
    PreferPrivate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImplementationKind {
    Executable,
    Workflow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    Git,
    Local,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceReference {
    pub kind: SourceKind,
    pub location: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryFormat {
    Wasm,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinaryReference {
    pub format: BinaryFormat,
    pub location: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowReference {
    pub workflow_id: String,
    pub workflow_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactDigests {
    pub source_digest: String,
    pub binary_digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityArtifactRecord {
    pub artifact_ref: String,
    pub implementation_kind: ImplementationKind,
    pub source: SourceReference,
    pub binary: Option<BinaryReference>,
    pub workflow_ref: Option<WorkflowReference>,
    pub digests: ArtifactDigests,
    pub provenance: RegistryProvenance,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryProvenance {
    pub source: String,
    pub author: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompositionKind {
    Atomic,
    Composite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CompositionPattern {
    Sequential,
    EventDriven,
    Enrichment,
    Validation,
    FanOut,
    Aggregation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposabilityMetadata {
    pub kind: CompositionKind,
    pub patterns: Vec<CompositionPattern>,
    pub provides: Vec<String>,
    pub requires: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityRegistration {
    pub scope: RegistryScope,
    pub contract: CapabilityContract,
    pub contract_path: String,
    pub artifact: CapabilityArtifactRecord,
    pub registered_at: String,
    pub tags: Vec<String>,
    pub composability: ComposabilityMetadata,
    pub governing_spec: String,
    pub validator_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectorRegistration {
    pub scope: RegistryScope,
    pub contract: ConnectorContract,
    pub contract_path: String,
    pub registered_at: String,
    pub governing_spec: String,
    pub validator_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectorRegistryRecord {
    pub scope: RegistryScope,
    pub connector_id: String,
    pub version: String,
    pub capabilities_provided: Vec<String>,
    pub supported_placement_targets: Vec<ExecutionTarget>,
    pub contract_path: String,
    pub registered_at: String,
    pub governing_spec: String,
    pub validator_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityRegistryRecord {
    pub scope: RegistryScope,
    pub id: String,
    pub version: String,
    pub lifecycle: Lifecycle,
    pub owner: Owner,
    pub contract_path: String,
    pub contract_digest: String,
    pub implementation_kind: ImplementationKind,
    pub artifact_ref: String,
    pub registered_at: String,
    pub provenance: RegistryProvenance,
    pub evidence: RegistrationEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveryIndexEntry {
    pub scope: RegistryScope,
    pub id: String,
    pub version: String,
    pub lifecycle: Lifecycle,
    pub owner: Owner,
    pub summary: String,
    pub tags: Vec<String>,
    pub permissions: Vec<String>,
    pub emits: Vec<String>,
    pub consumes: Vec<String>,
    pub implementation_kind: ImplementationKind,
    pub composability: ComposabilityMetadata,
    pub artifact_ref: String,
    pub registered_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistrationEvidence {
    pub evidence_id: String,
    pub artifact_ref: String,
    pub capability_id: String,
    pub capability_version: String,
    pub scope: RegistryScope,
    pub governing_spec: String,
    pub validator_version: String,
    pub produced_at: String,
    pub result: RegistrationResult,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistrationResult {
    Passed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistrationOutcome {
    pub record: CapabilityRegistryRecord,
    pub artifact: CapabilityArtifactRecord,
    pub index_entry: DiscoveryIndexEntry,
    pub evidence: RegistrationEvidence,
    /// `true` when the exact same version and digest were already in the
    /// registry — the call was a no-op (idempotent).  `false` on first
    /// registration of this version.
    pub already_registered: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedCapability {
    pub contract: CapabilityContract,
    pub record: CapabilityRegistryRecord,
    pub artifact: CapabilityArtifactRecord,
    pub index_entry: DiscoveryIndexEntry,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiscoveryQuery {
    pub owner_team: Option<String>,
    pub lifecycle: Option<Lifecycle>,
    pub implementation_kind: Option<ImplementationKind>,
    pub composition_kind: Option<CompositionKind>,
    pub composition_pattern: Option<CompositionPattern>,
    pub emits_event_id: Option<String>,
    pub consumes_event_id: Option<String>,
    pub tag: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompatibilityChangeClass {
    MetadataOnly,
    Additive,
    Breaking,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DeclaredVersionBump {
    Patch,
    Minor,
    Major,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionCompatibilityRecord {
    pub capability_id: String,
    pub previous_version: String,
    pub candidate_version: String,
    pub detected_change_class: CompatibilityChangeClass,
    pub declared_bump: DeclaredVersionBump,
    pub result: CompatibilityResult,
    pub evidence_ref: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompatibilityResult {
    Passed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistryErrorCode {
    ContractValidationFailed,
    MissingRequiredField,
    DuplicateItem,
    ImmutableVersionConflict,
    ArtifactConflict,
    ImplementationMismatch,
    InvalidSemverProgression,
    SemverTooSmall,
    UnknownCompatibility,
    InvalidConnectorContract,
    MissingRequiredConnector,
    ConnectorVersionIncompatible,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryError {
    pub code: RegistryErrorCode,
    pub target: String,
    pub message: String,
    pub severity: ErrorSeverity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryFailure {
    pub errors: Vec<RegistryError>,
}

#[derive(Debug, Clone, Default)]
pub struct CapabilityRegistry {
    contracts: BTreeMap<RegistryKey, CapabilityContract>,
    connectors: BTreeMap<RegistryKey, ConnectorRegistryRecord>,
    records: BTreeMap<RegistryKey, CapabilityRegistryRecord>,
    artifacts: BTreeMap<String, CapabilityArtifactRecord>,
    index: BTreeMap<RegistryKey, DiscoveryIndexEntry>,
    compatibility: Vec<VersionCompatibilityRecord>,
}

type RegistryKey = (RegistryScope, String, String);

impl CapabilityRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a connector contract as a first-class registry entry.
    ///
    /// # Errors
    ///
    /// Returns [`RegistryFailure`] when the connector contract is invalid or
    /// the same connector version has already been registered with different metadata.
    pub fn register_connector(
        &mut self,
        request: ConnectorRegistration,
    ) -> Result<ConnectorRegistryRecord, RegistryFailure> {
        let connector =
            validate_connector_contract(request.contract).map_err(|failure| RegistryFailure {
                errors: failure
                    .errors
                    .into_iter()
                    .map(|error| RegistryError {
                        code: RegistryErrorCode::InvalidConnectorContract,
                        target: error.path,
                        message: error.message,
                        severity: error.severity,
                    })
                    .collect(),
            })?;
        let key = (
            request.scope,
            connector.connector_id.clone(),
            connector.version.clone(),
        );
        let record = ConnectorRegistryRecord {
            scope: request.scope,
            connector_id: connector.connector_id,
            version: connector.version,
            capabilities_provided: connector.capabilities_provided,
            supported_placement_targets: connector.supported_placement_targets,
            contract_path: request.contract_path,
            registered_at: request.registered_at,
            governing_spec: request.governing_spec,
            validator_version: request.validator_version,
        };

        if let Some(existing) = self.connectors.get(&key) {
            if existing == &record {
                return Ok(existing.clone());
            }
            return Err(single_error(
                RegistryErrorCode::ImmutableVersionConflict,
                "$.connector",
                "connector version is immutable once registered",
            ));
        }

        self.connectors.insert(key, record.clone());
        Ok(record)
    }

    /// Registers a capability publication into the registry.
    ///
    /// # Idempotency
    ///
    /// Re-registering the same version with the same digest and metadata
    /// succeeds silently and returns the existing record with
    /// [`RegistrationOutcome::already_registered`] set to `true`.  No state
    /// is mutated.  This means an agent that crashes after registering but
    /// before recording the outcome can safely retry — the retry will succeed
    /// and produce the same `RegistrationOutcome` as the original call.
    ///
    /// Re-registering the same version with a *different* digest or metadata
    /// returns [`RegistryErrorCode::ImmutableVersionConflict`].  Published
    /// versions are immutable; to ship a correction, publish a new semver
    /// version.
    ///
    /// # Errors
    ///
    /// Returns [`RegistryFailure`] when contract validation fails, when
    /// immutable publication semantics would be violated, when artifact
    /// metadata is inconsistent, or when the semver bump is unsafe for the
    /// detected contract change.
    pub fn register(
        &mut self,
        request: CapabilityRegistration,
    ) -> Result<RegistrationOutcome, RegistryFailure> {
        let CapabilityRegistration {
            scope,
            contract,
            contract_path,
            artifact,
            registered_at,
            tags,
            composability,
            governing_spec,
            validator_version,
        } = request;

        let mut errors = Vec::new();
        validate_registration_fields(
            &contract_path,
            &registered_at,
            &tags,
            &artifact,
            &composability,
            &mut errors,
        );
        if !errors.is_empty() {
            return Err(RegistryFailure { errors });
        }

        let key = (scope, contract.id.clone(), contract.version.clone());
        let existing_published = self.records.get(&key).map(published_contract_record);

        let validated = validate_contract(
            contract,
            &ValidationContext {
                governing_spec: &governing_spec,
                validator_version: &validator_version,
                existing_published: existing_published.as_ref(),
            },
        )
        .map_err(map_contract_failure)?;

        let contract = validated.normalized;
        validate_connector_requirements_for_registration(
            &contract.connector_requirements,
            &self.connectors,
            scope,
        )?;
        let contract_digest = governed_content_digest(&contract);
        let artifact_ref = artifact.artifact_ref.clone();
        let record = build_registry_record(&RegistryRecordInput {
            scope,
            contract: &contract,
            contract_path: &contract_path,
            contract_digest: &contract_digest,
            artifact: &artifact,
            registered_at: &registered_at,
            governing_spec: &governing_spec,
            validator_version: &validator_version,
        });
        let index_entry = build_index_entry(
            scope,
            &contract,
            &artifact,
            &registered_at,
            tags,
            composability,
        );

        match self.artifacts.get(&artifact_ref) {
            Some(existing) if existing != &artifact => {
                return Err(single_error(
                    RegistryErrorCode::ArtifactConflict,
                    "$.artifact.artifact_ref",
                    "artifact_ref must not resolve to different artifact metadata",
                ));
            }
            _ => {}
        }

        if let Some(existing) = self.records.get(&key) {
            return self.reconcile_existing(&key, existing, &record, &artifact, &index_entry);
        }

        let compatibility =
            if let Some(prior) = self.latest_prior_record(scope, &contract.id, &contract.version) {
                Some(Self::validate_semver_progression(
                    prior,
                    &contract,
                    &record.evidence.evidence_id,
                )?)
            } else {
                None
            };

        self.contracts.insert(key.clone(), contract.clone());
        self.records.insert(key.clone(), record.clone());
        self.artifacts.insert(artifact_ref, artifact.clone());
        self.index.insert(key, index_entry.clone());
        if let Some(compatibility) = compatibility {
            self.compatibility.push(compatibility);
        }

        Ok(RegistrationOutcome {
            evidence: record.evidence.clone(),
            record,
            artifact,
            index_entry,
            already_registered: false,
        })
    }

    #[must_use]
    pub fn find_exact(
        &self,
        lookup_scope: LookupScope,
        id: &str,
        version: &str,
    ) -> Option<ResolvedCapability> {
        for &scope in lookup_order(lookup_scope) {
            let key = (scope, id.to_string(), version.to_string());
            if let Some(record) = self.records.get(&key) {
                return self.resolve(&key, record.clone());
            }
        }
        None
    }

    #[must_use]
    pub fn discover(
        &self,
        lookup_scope: LookupScope,
        query: &DiscoveryQuery,
    ) -> Vec<DiscoveryIndexEntry> {
        let mut results = Vec::new();
        let mut shadowed = BTreeSet::new();

        for &scope in lookup_order(lookup_scope) {
            let entries = self
                .index
                .iter()
                .filter(|((entry_scope, _, _), _)| *entry_scope == scope)
                .map(|((_, id, version), entry)| ((id.clone(), version.clone()), entry))
                .filter(|(_, entry)| matches_query(entry, query));

            for ((id, version), entry) in entries {
                if lookup_scope == LookupScope::PreferPrivate
                    && scope == RegistryScope::Public
                    && shadowed.contains(&(id.clone(), version.clone()))
                {
                    continue;
                }

                if scope == RegistryScope::Private {
                    shadowed.insert((id, version));
                }

                results.push(entry.clone());
            }
        }

        results.sort_by(compare_index_entries);
        results
    }

    #[must_use]
    pub fn discover_connectors(
        &self,
        lookup_scope: LookupScope,
        connector_id: &str,
        version_range: &str,
    ) -> Vec<ConnectorRegistryRecord> {
        let Ok(requirement) = VersionReq::parse(version_range) else {
            return Vec::new();
        };
        let mut results = Vec::new();
        let mut shadowed = BTreeSet::new();
        for &scope in lookup_order(lookup_scope) {
            for ((entry_scope, id, version), record) in &self.connectors {
                if *entry_scope != scope || id != connector_id {
                    continue;
                }
                if !shadowed.insert((id.clone(), version.clone())) {
                    continue;
                }
                let Ok(parsed) = Version::parse(version) else {
                    continue;
                };
                if requirement.matches(&parsed) {
                    results.push(record.clone());
                }
            }
        }
        results.sort_by(|a, b| a.version.cmp(&b.version));
        results
    }

    #[must_use]
    pub fn compatibility_records(&self) -> &[VersionCompatibilityRecord] {
        &self.compatibility
    }

    #[must_use]
    pub(crate) fn graph_entries(&self) -> Vec<ResolvedCapability> {
        self.records
            .iter()
            .filter_map(|(key, record)| self.resolve(key, record.clone()))
            .collect()
    }

    fn reconcile_existing(
        &self,
        key: &RegistryKey,
        existing: &CapabilityRegistryRecord,
        candidate_record: &CapabilityRegistryRecord,
        candidate_artifact: &CapabilityArtifactRecord,
        candidate_index: &DiscoveryIndexEntry,
    ) -> Result<RegistrationOutcome, RegistryFailure> {
        let Some(existing_contract) = self.contracts.get(key) else {
            return Err(single_error(
                RegistryErrorCode::ImmutableVersionConflict,
                "$.contract_path",
                "existing registry record is missing its authoritative contract artifact",
            ));
        };
        let Some(existing_artifact) = self.artifacts.get(&existing.artifact_ref) else {
            return Err(single_error(
                RegistryErrorCode::ArtifactConflict,
                "$.artifact_ref",
                "existing registry record is missing its artifact metadata",
            ));
        };
        let Some(existing_index) = self.index.get(key) else {
            return Err(single_error(
                RegistryErrorCode::ImmutableVersionConflict,
                "$.id",
                "existing registry record is missing its discovery index entry",
            ));
        };

        if existing == candidate_record
            && existing_artifact == candidate_artifact
            && existing_index == candidate_index
        {
            return Ok(RegistrationOutcome {
                evidence: existing.evidence.clone(),
                record: existing.clone(),
                artifact: existing_artifact.clone(),
                index_entry: existing_index.clone(),
                already_registered: true,
            });
        }

        if governed_content_digest(existing_contract) != candidate_record.contract_digest {
            return Err(single_error(
                RegistryErrorCode::ImmutableVersionConflict,
                "$.version",
                "published capability versions are immutable within a scope",
            ));
        }

        Err(single_error(
            RegistryErrorCode::ImmutableVersionConflict,
            "$.artifact_ref",
            "published capability versions are immutable and cannot be republished with different registry metadata",
        ))
    }

    fn latest_prior_record(
        &self,
        scope: RegistryScope,
        id: &str,
        candidate_version: &str,
    ) -> Option<ResolvedCapability> {
        let candidate = Version::parse(candidate_version).ok()?;
        let mut best: Option<(&RegistryKey, &CapabilityRegistryRecord)> = None;

        for (key @ (entry_scope, entry_id, entry_version), record) in &self.records {
            if *entry_scope != scope || entry_id != id {
                continue;
            }

            let Ok(entry) = Version::parse(entry_version) else {
                continue;
            };
            if entry >= candidate {
                continue;
            }

            if let Some((best_key, _)) = best
                && compare_versions(entry_version, &best_key.2) != Ordering::Greater
            {
                continue;
            }

            best = Some((key, record));
        }

        best.and_then(|(key, record)| self.resolve(key, record.clone()))
    }

    fn resolve(
        &self,
        key: &RegistryKey,
        record: CapabilityRegistryRecord,
    ) -> Option<ResolvedCapability> {
        let contract = self.contracts.get(key).cloned()?;
        let artifact = self.artifacts.get(&record.artifact_ref).cloned()?;
        let index_entry = self.index.get(key).cloned()?;

        Some(ResolvedCapability {
            contract,
            record,
            artifact,
            index_entry,
        })
    }

    fn validate_semver_progression(
        previous: ResolvedCapability,
        candidate: &CapabilityContract,
        evidence_ref: &str,
    ) -> Result<VersionCompatibilityRecord, RegistryFailure> {
        let previous_version = Version::parse(&previous.record.version).map_err(|_| {
            single_error(
                RegistryErrorCode::InvalidSemverProgression,
                "$.version",
                "previous published version is not valid semver",
            )
        })?;
        let candidate_version = Version::parse(&candidate.version).map_err(|_| {
            single_error(
                RegistryErrorCode::InvalidSemverProgression,
                "$.version",
                "candidate version is not valid semver",
            )
        })?;

        if candidate_version <= previous_version {
            return Err(single_error(
                RegistryErrorCode::InvalidSemverProgression,
                "$.version",
                "candidate version must be greater than the previous published version",
            ));
        }

        let declared_bump = declared_bump(&previous_version, &candidate_version);
        let detected_change_class = classify_contract_change(&previous.contract, candidate);

        let required_bump = match detected_change_class {
            CompatibilityChangeClass::MetadataOnly => DeclaredVersionBump::Patch,
            CompatibilityChangeClass::Additive => DeclaredVersionBump::Minor,
            CompatibilityChangeClass::Breaking => DeclaredVersionBump::Major,
            CompatibilityChangeClass::Unknown => {
                return Err(single_error(
                    RegistryErrorCode::UnknownCompatibility,
                    "$.version",
                    "contract diff could not be classified safely for semver progression",
                ));
            }
        };

        if declared_bump < required_bump {
            return Err(single_error(
                RegistryErrorCode::SemverTooSmall,
                "$.version",
                "declared semver bump is too small for the detected compatibility change",
            ));
        }

        Ok(VersionCompatibilityRecord {
            capability_id: candidate.id.clone(),
            previous_version: previous.record.version,
            candidate_version: candidate.version.clone(),
            detected_change_class,
            declared_bump,
            result: CompatibilityResult::Passed,
            evidence_ref: evidence_ref.to_string(),
        })
    }
}

struct RegistryRecordInput<'a> {
    scope: RegistryScope,
    contract: &'a CapabilityContract,
    contract_path: &'a str,
    contract_digest: &'a str,
    artifact: &'a CapabilityArtifactRecord,
    registered_at: &'a str,
    governing_spec: &'a str,
    validator_version: &'a str,
}

fn published_contract_record(record: &CapabilityRegistryRecord) -> PublishedContractRecord {
    PublishedContractRecord {
        id: record.id.clone(),
        version: record.version.clone(),
        governed_content_digest: record.contract_digest.clone(),
        lifecycle: record.lifecycle.clone(),
    }
}

fn build_registry_record(input: &RegistryRecordInput<'_>) -> CapabilityRegistryRecord {
    let evidence = RegistrationEvidence {
        evidence_id: format!(
            "{}:{}:{}:{}",
            input.governing_spec, input.contract.id, input.contract.version, input.registered_at
        ),
        artifact_ref: input.artifact.artifact_ref.clone(),
        capability_id: input.contract.id.clone(),
        capability_version: input.contract.version.clone(),
        scope: input.scope,
        governing_spec: input.governing_spec.to_string(),
        validator_version: input.validator_version.to_string(),
        produced_at: input.registered_at.to_string(),
        result: RegistrationResult::Passed,
    };

    CapabilityRegistryRecord {
        scope: input.scope,
        id: input.contract.id.clone(),
        version: input.contract.version.clone(),
        lifecycle: input.contract.lifecycle.clone(),
        owner: input.contract.owner.clone(),
        contract_path: input.contract_path.to_string(),
        contract_digest: input.contract_digest.to_string(),
        implementation_kind: input.artifact.implementation_kind,
        artifact_ref: input.artifact.artifact_ref.clone(),
        registered_at: input.registered_at.to_string(),
        provenance: input.artifact.provenance.clone(),
        evidence,
    }
}

fn build_index_entry(
    scope: RegistryScope,
    contract: &CapabilityContract,
    artifact: &CapabilityArtifactRecord,
    registered_at: &str,
    tags: Vec<String>,
    composability: ComposabilityMetadata,
) -> DiscoveryIndexEntry {
    DiscoveryIndexEntry {
        scope,
        id: contract.id.clone(),
        version: contract.version.clone(),
        lifecycle: contract.lifecycle.clone(),
        owner: contract.owner.clone(),
        summary: contract.summary.clone(),
        tags: normalized_tags(contract, tags),
        permissions: normalized_ids(&contract.permissions),
        emits: normalized_events(&contract.emits),
        consumes: normalized_events(&contract.consumes),
        implementation_kind: artifact.implementation_kind,
        composability,
        artifact_ref: artifact.artifact_ref.clone(),
        registered_at: registered_at.to_string(),
    }
}

fn validate_registration_fields(
    contract_path: &str,
    registered_at: &str,
    tags: &[String],
    artifact: &CapabilityArtifactRecord,
    composability: &ComposabilityMetadata,
    errors: &mut Vec<RegistryError>,
) {
    if contract_path.trim().is_empty() {
        errors.push(registry_error(
            RegistryErrorCode::MissingRequiredField,
            "$.contract_path",
            "contract_path must be non-empty",
        ));
    }
    if registered_at.trim().is_empty() {
        errors.push(registry_error(
            RegistryErrorCode::MissingRequiredField,
            "$.registered_at",
            "registered_at must be non-empty",
        ));
    }
    validate_unique_strings(tags, "$.tags", errors);
    validate_unique_strings(&composability.provides, "$.composability.provides", errors);
    validate_unique_strings(&composability.requires, "$.composability.requires", errors);
    validate_patterns(&composability.patterns, errors);
    validate_artifact(artifact, composability, errors);
}

fn validate_connector_requirements_for_registration(
    requirements: &[ConnectorRequirement],
    connectors: &BTreeMap<RegistryKey, ConnectorRegistryRecord>,
    scope: RegistryScope,
) -> Result<(), RegistryFailure> {
    for requirement in requirements {
        let matching_id = connectors
            .iter()
            .filter(|((entry_scope, connector_id, _), _)| {
                (*entry_scope == scope || *entry_scope == RegistryScope::Public)
                    && connector_id == &requirement.connector_id
            })
            .collect::<Vec<_>>();
        if matching_id.is_empty() {
            return Err(single_error(
                RegistryErrorCode::MissingRequiredConnector,
                "$.connector_requirements",
                &format!(
                    "missing_required_connector: {} {}",
                    requirement.connector_id, requirement.version
                ),
            ));
        }
        let parsed_range = VersionReq::parse(&requirement.version).map_err(|_| {
            single_error(
                RegistryErrorCode::ConnectorVersionIncompatible,
                "$.connector_requirements",
                &format!(
                    "connector_version_incompatible: {} {}",
                    requirement.connector_id, requirement.version
                ),
            )
        })?;
        let has_satisfying_version = matching_id.iter().any(|((_, _, version), _)| {
            Version::parse(version)
                .map(|parsed| parsed_range.matches(&parsed))
                .unwrap_or(false)
        });
        if !has_satisfying_version {
            return Err(single_error(
                RegistryErrorCode::ConnectorVersionIncompatible,
                "$.connector_requirements",
                &format!(
                    "connector_version_incompatible: {} {}",
                    requirement.connector_id, requirement.version
                ),
            ));
        }
    }
    Ok(())
}

fn validate_artifact(
    artifact: &CapabilityArtifactRecord,
    composability: &ComposabilityMetadata,
    errors: &mut Vec<RegistryError>,
) {
    if artifact.artifact_ref.trim().is_empty() {
        errors.push(registry_error(
            RegistryErrorCode::MissingRequiredField,
            "$.artifact.artifact_ref",
            "artifact_ref must be non-empty",
        ));
    }
    if artifact.source.location.trim().is_empty() {
        errors.push(registry_error(
            RegistryErrorCode::MissingRequiredField,
            "$.artifact.source.location",
            "source.location must be non-empty",
        ));
    }
    if artifact.digests.source_digest.trim().is_empty() {
        errors.push(registry_error(
            RegistryErrorCode::MissingRequiredField,
            "$.artifact.digests.source_digest",
            "source_digest must be non-empty",
        ));
    }
    if artifact.provenance.author.trim().is_empty() {
        errors.push(registry_error(
            RegistryErrorCode::MissingRequiredField,
            "$.artifact.provenance.author",
            "artifact provenance author must be non-empty",
        ));
    }
    if artifact.provenance.created_at.trim().is_empty() {
        errors.push(registry_error(
            RegistryErrorCode::MissingRequiredField,
            "$.artifact.provenance.created_at",
            "artifact provenance created_at must be non-empty",
        ));
    }

    match artifact.implementation_kind {
        ImplementationKind::Executable => {
            if artifact.binary.is_none() {
                errors.push(registry_error(
                    RegistryErrorCode::MissingRequiredField,
                    "$.artifact.binary",
                    "executable capabilities require a wasm binary reference",
                ));
            }
            if artifact.workflow_ref.is_some() {
                errors.push(registry_error(
                    RegistryErrorCode::ImplementationMismatch,
                    "$.artifact.workflow_ref",
                    "executable capabilities must not include workflow_ref",
                ));
            }
            if composability.kind != CompositionKind::Atomic {
                errors.push(registry_error(
                    RegistryErrorCode::ImplementationMismatch,
                    "$.composability.kind",
                    "atomic composability is required for executable capabilities",
                ));
            }
        }
        ImplementationKind::Workflow => {
            if artifact.workflow_ref.is_none() {
                errors.push(registry_error(
                    RegistryErrorCode::MissingRequiredField,
                    "$.artifact.workflow_ref",
                    "workflow-backed capabilities require workflow_ref",
                ));
            }
            if artifact.binary.is_some() {
                errors.push(registry_error(
                    RegistryErrorCode::ImplementationMismatch,
                    "$.artifact.binary",
                    "workflow-backed capabilities must not include executable binary metadata",
                ));
            }
            if composability.kind != CompositionKind::Composite {
                errors.push(registry_error(
                    RegistryErrorCode::ImplementationMismatch,
                    "$.composability.kind",
                    "composite composability is required for workflow-backed capabilities",
                ));
            }
        }
    }
}

fn validate_patterns(patterns: &[CompositionPattern], errors: &mut Vec<RegistryError>) {
    if patterns.is_empty() {
        errors.push(registry_error(
            RegistryErrorCode::MissingRequiredField,
            "$.composability.patterns",
            "composability.patterns must contain at least one item",
        ));
        return;
    }

    let unique: BTreeSet<_> = patterns.iter().copied().collect();
    if unique.len() != patterns.len() {
        errors.push(registry_error(
            RegistryErrorCode::DuplicateItem,
            "$.composability.patterns",
            "composability.patterns must be unique",
        ));
    }
}

fn normalized_tags(contract: &CapabilityContract, tags: Vec<String>) -> Vec<String> {
    let mut values = BTreeSet::new();
    for tag in tags {
        let trimmed = tag.trim();
        if !trimmed.is_empty() {
            values.insert(trimmed.to_string());
        }
    }
    for namespace_segment in contract.namespace.split('.') {
        values.insert(namespace_segment.to_string());
    }
    values.insert(contract.name.clone());
    values.into_iter().collect()
}

fn normalized_ids(items: &[IdReference]) -> Vec<String> {
    let mut values: Vec<_> = items.iter().map(|item| item.id.clone()).collect();
    values.sort();
    values
}

fn normalized_events(items: &[EventReference]) -> Vec<String> {
    let mut values: Vec<_> = items
        .iter()
        .map(|item| format!("{}@{}", item.event_id, item.version))
        .collect();
    values.sort();
    values
}

fn validate_unique_strings(values: &[String], target: &str, errors: &mut Vec<RegistryError>) {
    let mut seen = BTreeSet::new();
    for value in values {
        if value.trim().is_empty() {
            errors.push(registry_error(
                RegistryErrorCode::MissingRequiredField,
                target,
                "values must be non-empty",
            ));
            return;
        }
        if !seen.insert(value.clone()) {
            errors.push(registry_error(
                RegistryErrorCode::DuplicateItem,
                target,
                "values must be unique",
            ));
            return;
        }
    }
}

fn compare_index_entries(left: &DiscoveryIndexEntry, right: &DiscoveryIndexEntry) -> Ordering {
    let by_id = left.id.cmp(&right.id);
    if by_id != Ordering::Equal {
        return by_id;
    }

    let by_version = compare_versions(&right.version, &left.version);
    if by_version != Ordering::Equal {
        return by_version;
    }

    left.scope.cmp(&right.scope)
}

fn compare_versions(left: &str, right: &str) -> Ordering {
    match (Version::parse(left), Version::parse(right)) {
        (Ok(left), Ok(right)) => left.cmp(&right),
        _ => left.cmp(right),
    }
}

fn lookup_order(lookup_scope: LookupScope) -> &'static [RegistryScope] {
    match lookup_scope {
        LookupScope::PublicOnly => &[RegistryScope::Public],
        LookupScope::PreferPrivate => &[RegistryScope::Private, RegistryScope::Public],
    }
}

fn matches_query(entry: &DiscoveryIndexEntry, query: &DiscoveryQuery) -> bool {
    if let Some(owner) = &query.owner_team
        && entry.owner.team != *owner
    {
        return false;
    }

    if let Some(lifecycle) = &query.lifecycle
        && entry.lifecycle != *lifecycle
    {
        return false;
    }

    if let Some(kind) = &query.implementation_kind
        && entry.implementation_kind != *kind
    {
        return false;
    }

    if let Some(kind) = &query.composition_kind
        && entry.composability.kind != *kind
    {
        return false;
    }

    if let Some(pattern) = &query.composition_pattern
        && !entry.composability.patterns.contains(pattern)
    {
        return false;
    }

    if let Some(event_id) = &query.emits_event_id
        && !entry.emits.iter().any(|value| value.starts_with(event_id))
    {
        return false;
    }

    if let Some(event_id) = &query.consumes_event_id
        && !entry
            .consumes
            .iter()
            .any(|value| value.starts_with(event_id))
    {
        return false;
    }

    if let Some(tag) = &query.tag
        && !entry.tags.iter().any(|entry_tag| entry_tag == tag)
    {
        return false;
    }

    true
}

fn declared_bump(previous: &Version, candidate: &Version) -> DeclaredVersionBump {
    if candidate.major > previous.major {
        DeclaredVersionBump::Major
    } else if candidate.minor > previous.minor {
        DeclaredVersionBump::Minor
    } else {
        DeclaredVersionBump::Patch
    }
}

fn classify_contract_change(
    previous: &CapabilityContract,
    candidate: &CapabilityContract,
) -> CompatibilityChangeClass {
    if previous.id != candidate.id
        || previous.namespace != candidate.namespace
        || previous.name != candidate.name
    {
        return CompatibilityChangeClass::Breaking;
    }

    if previous.inputs != candidate.inputs || previous.outputs != candidate.outputs {
        return CompatibilityChangeClass::Unknown;
    }

    if previous.preconditions != candidate.preconditions
        || previous.postconditions != candidate.postconditions
        || previous.side_effects != candidate.side_effects
        || previous.execution != candidate.execution
    {
        return CompatibilityChangeClass::Breaking;
    }

    let set_relationships = [
        compare_superset(&event_keys(&previous.emits), &event_keys(&candidate.emits)),
        compare_superset(
            &event_keys(&previous.consumes),
            &event_keys(&candidate.consumes),
        ),
        compare_superset(
            &id_keys(&previous.permissions),
            &id_keys(&candidate.permissions),
        ),
        compare_superset(&id_keys(&previous.policies), &id_keys(&candidate.policies)),
        compare_superset(
            &dependency_keys(&previous.dependencies),
            &dependency_keys(&candidate.dependencies),
        ),
    ];

    if set_relationships.contains(&SetRelationship::Breaking) {
        return CompatibilityChangeClass::Breaking;
    }

    if set_relationships.contains(&SetRelationship::Additive) {
        return CompatibilityChangeClass::Additive;
    }

    CompatibilityChangeClass::MetadataOnly
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SetRelationship {
    Same,
    Additive,
    Breaking,
}

fn compare_superset(previous: &BTreeSet<String>, candidate: &BTreeSet<String>) -> SetRelationship {
    if !previous.is_subset(candidate) {
        SetRelationship::Breaking
    } else if previous == candidate {
        SetRelationship::Same
    } else {
        SetRelationship::Additive
    }
}

fn event_keys(events: &[EventReference]) -> BTreeSet<String> {
    events
        .iter()
        .map(|event| format!("{}@{}", event.event_id, event.version))
        .collect()
}

fn id_keys(ids: &[IdReference]) -> BTreeSet<String> {
    ids.iter().map(|item| item.id.clone()).collect()
}

fn dependency_keys(dependencies: &[traverse_contracts::DependencyReference]) -> BTreeSet<String> {
    dependencies
        .iter()
        .map(|dependency| {
            format!(
                "{:?}:{}@{}",
                dependency.artifact_type, dependency.id, dependency.version
            )
        })
        .collect()
}

fn map_contract_failure(failure: ValidationFailure) -> RegistryFailure {
    RegistryFailure {
        errors: failure
            .errors
            .into_iter()
            .map(|error| RegistryError {
                code: RegistryErrorCode::ContractValidationFailed,
                target: error.path,
                message: error.message,
                severity: error.severity,
            })
            .collect(),
    }
}

fn single_error(code: RegistryErrorCode, target: &str, message: &str) -> RegistryFailure {
    RegistryFailure {
        errors: vec![registry_error(code, target, message)],
    }
}

fn registry_error(code: RegistryErrorCode, target: &str, message: &str) -> RegistryError {
    RegistryError {
        code,
        target: target.to_string(),
        message: message.to_string(),
        severity: ErrorSeverity::Error,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;
    use serde_json::json;
    use traverse_contracts::{
        BinaryFormat as ContractBinaryFormat, Condition, DependencyArtifactType,
        DependencyReference, Entrypoint, EntrypointKind, EvidenceStatus, EvidenceType, Execution,
        ExecutionConstraints, ExecutionTarget, FilesystemAccess, HostApiAccess, NetworkAccess,
        Provenance, ProvenanceSource, SchemaContainer, ServiceType, SideEffect, SideEffectKind,
        ValidationEvidence,
    };

    #[test]
    fn find_exact_and_discover_cover_lookup_paths() {
        let mut registry = CapabilityRegistry::new();
        assert!(
            registry
                .find_exact(
                    LookupScope::PublicOnly,
                    "content.comments.create-comment-draft",
                    "1.0.0",
                )
                .is_none()
        );

        registry
            .register(registration(
                RegistryScope::Public,
                base_contract("content.comments.create-comment-draft", "1.0.0"),
            ))
            .expect("public registration should succeed");
        registry
            .register(registration(
                RegistryScope::Private,
                base_contract("content.comments.create-comment-draft", "1.0.0"),
            ))
            .expect("private registration should succeed");

        let discovered = registry.discover(LookupScope::PreferPrivate, &DiscoveryQuery::default());
        assert_eq!(discovered.len(), 1);
        assert_eq!(discovered[0].scope, RegistryScope::Private);
    }

    #[test]
    fn connector_helpers_cover_invalid_range_and_invalid_stored_version_paths() {
        let mut registry = CapabilityRegistry::new();
        let key = (
            RegistryScope::Public,
            "traverse.env".to_string(),
            "not-semver".to_string(),
        );
        registry.connectors.insert(
            key,
            ConnectorRegistryRecord {
                scope: RegistryScope::Public,
                connector_id: "traverse.env".to_string(),
                version: "not-semver".to_string(),
                capabilities_provided: vec!["traverse.env.read".to_string()],
                supported_placement_targets: vec![ExecutionTarget::Local],
                contract_path: "contracts/connectors/traverse.env/connector_contract.json"
                    .to_string(),
                registered_at: "2026-04-19T00:00:00Z".to_string(),
                governing_spec: "039-connector-plugin-architecture".to_string(),
                validator_version: "registry-test".to_string(),
            },
        );

        assert!(
            registry
                .discover_connectors(LookupScope::PublicOnly, "traverse.env", "^1.0.0")
                .is_empty()
        );

        let requirements = vec![traverse_contracts::ConnectorRequirement {
            connector_id: "traverse.env".to_string(),
            version: "not a range".to_string(),
        }];
        let failure = validate_connector_requirements_for_registration(
            &requirements,
            &registry.connectors,
            RegistryScope::Public,
        )
        .expect_err("invalid range should fail");

        assert_eq!(
            failure.errors[0].code,
            RegistryErrorCode::ConnectorVersionIncompatible
        );
    }

    #[test]
    fn reconcile_existing_covers_internal_consistency_guards() {
        let mut registry = CapabilityRegistry::new();
        let contract = base_contract("content.comments.create-comment-draft", "1.0.0");
        let artifact = executable_artifact(&contract);
        let record = build_registry_record(&RegistryRecordInput {
            scope: RegistryScope::Public,
            contract: &contract,
            contract_path: "registry/public/content.comments.create-comment-draft/1.0.0/contract.json",
            contract_digest: &governed_content_digest(&contract),
            artifact: &artifact,
            registered_at: "2026-03-27T00:00:00Z",
            governing_spec: "005-capability-registry",
            validator_version: "test",
        });
        let index = build_index_entry(
            RegistryScope::Public,
            &contract,
            &artifact,
            "2026-03-27T00:00:00Z",
            vec!["comments".to_string()],
            atomic_composability(),
        );
        let key = (
            RegistryScope::Public,
            contract.id.clone(),
            contract.version.clone(),
        );

        assert_eq!(
            registry
                .reconcile_existing(&key, &record, &record, &artifact, &index)
                .expect_err("missing contract should fail")
                .errors[0]
                .code,
            RegistryErrorCode::ImmutableVersionConflict
        );

        registry.contracts.insert(key.clone(), contract.clone());
        assert_eq!(
            registry
                .reconcile_existing(&key, &record, &record, &artifact, &index)
                .expect_err("missing artifact should fail")
                .errors[0]
                .code,
            RegistryErrorCode::ArtifactConflict
        );

        registry
            .artifacts
            .insert(record.artifact_ref.clone(), artifact.clone());
        assert_eq!(
            registry
                .reconcile_existing(&key, &record, &record, &artifact, &index)
                .expect_err("missing index should fail")
                .errors[0]
                .code,
            RegistryErrorCode::ImmutableVersionConflict
        );

        registry.index.insert(key.clone(), index.clone());
        let outcome = registry
            .reconcile_existing(&key, &record, &record, &artifact, &index)
            .expect("same publication should be idempotent");
        assert_eq!(outcome.record.id, contract.id);

        let mut changed_digest = record.clone();
        changed_digest.contract_digest = "sha256:changed".to_string();
        assert_eq!(
            registry
                .reconcile_existing(&key, &record, &changed_digest, &artifact, &index)
                .expect_err("governed content changes should fail")
                .errors[0]
                .code,
            RegistryErrorCode::ImmutableVersionConflict
        );

        let mut changed_metadata = record.clone();
        changed_metadata.registered_at = "2026-03-28T00:00:00Z".to_string();
        assert_eq!(
            registry
                .reconcile_existing(&key, &record, &changed_metadata, &artifact, &index)
                .expect_err("metadata-only republishes should fail")
                .errors[0]
                .code,
            RegistryErrorCode::ImmutableVersionConflict
        );
    }

    #[test]
    fn semver_and_prior_record_guards_are_covered() {
        let mut registry = CapabilityRegistry::new();
        assert!(
            registry
                .latest_prior_record(
                    RegistryScope::Public,
                    "content.comments.create-comment-draft",
                    "not-semver",
                )
                .is_none()
        );

        registry
            .register(registration(
                RegistryScope::Public,
                base_contract("content.comments.create-comment-draft", "1.0.0"),
            ))
            .expect("1.0.0 registration should succeed");
        registry
            .register(registration(
                RegistryScope::Public,
                base_contract("content.comments.create-comment-draft", "1.1.0"),
            ))
            .expect("1.1.0 registration should succeed");

        let invalid_key = (
            RegistryScope::Public,
            "content.comments.create-comment-draft".to_string(),
            "not-semver".to_string(),
        );
        let invalid_contract = base_contract("content.comments.create-comment-draft", "1.0.0");
        let invalid_artifact = executable_artifact(&invalid_contract);
        registry.records.insert(
            invalid_key,
            build_registry_record(&RegistryRecordInput {
                scope: RegistryScope::Public,
                contract: &invalid_contract,
                contract_path:
                    "registry/public/content.comments.create-comment-draft/not-semver/contract.json",
                contract_digest: &governed_content_digest(&invalid_contract),
                artifact: &invalid_artifact,
                registered_at: "2026-03-27T00:00:00Z",
                governing_spec: "005-capability-registry",
                validator_version: "test",
            }),
        );

        let prior = registry
            .latest_prior_record(
                RegistryScope::Public,
                "content.comments.create-comment-draft",
                "2.0.0",
            )
            .expect("prior version should resolve");
        assert_eq!(prior.record.version, "1.1.0");

        let previous = resolved_capability("1.0.0", "not-semver");
        let candidate = base_contract("content.comments.create-comment-draft", "1.0.1");
        assert_eq!(
            CapabilityRegistry::validate_semver_progression(previous, &candidate, "evidence")
                .expect_err("invalid previous version should fail")
                .errors[0]
                .code,
            RegistryErrorCode::InvalidSemverProgression
        );

        let mut invalid_candidate = candidate.clone();
        invalid_candidate.version = "invalid".to_string();
        assert_eq!(
            CapabilityRegistry::validate_semver_progression(
                resolved_capability("1.0.0", "1.0.0"),
                &invalid_candidate,
                "evidence",
            )
            .expect_err("invalid candidate version should fail")
            .errors[0]
                .code,
            RegistryErrorCode::InvalidSemverProgression
        );

        assert_eq!(
            CapabilityRegistry::validate_semver_progression(
                resolved_capability("1.1.0", "1.1.0"),
                &base_contract("content.comments.create-comment-draft", "1.0.1"),
                "evidence",
            )
            .expect_err("regression should fail")
            .errors[0]
                .code,
            RegistryErrorCode::InvalidSemverProgression
        );

        let mut metadata_patch = base_contract("content.comments.create-comment-draft", "1.0.1");
        metadata_patch.summary = "Updated summary only.".to_string();
        assert!(
            CapabilityRegistry::validate_semver_progression(
                resolved_capability("1.0.0", "1.0.0"),
                &metadata_patch,
                "evidence",
            )
            .is_ok()
        );
    }

    #[test]
    fn latest_prior_record_skips_non_prior_and_non_better_versions() {
        let mut registry = CapabilityRegistry::new();
        let id = "content.comments.create-comment-draft";
        for version in ["1.0.0", "1.1.0", "1.10.0", "1.2.0"] {
            registry
                .register(registration(
                    RegistryScope::Public,
                    base_contract(id, version),
                ))
                .expect("registration should succeed");
        }

        let prior_for_equal_candidate = registry
            .latest_prior_record(RegistryScope::Public, id, "1.1.0")
            .expect("equal candidate should skip itself and resolve lower version");
        assert_eq!(prior_for_equal_candidate.record.version, "1.0.0");

        let highest_prior = registry
            .latest_prior_record(RegistryScope::Public, id, "2.0.0")
            .expect("highest prior version should resolve");
        assert_eq!(highest_prior.record.version, "1.10.0");
    }

    #[test]
    fn validation_helpers_cover_registration_and_artifact_branches() {
        let contract = base_contract("content.comments.create-comment-draft", "1.0.0");
        let mut artifact = executable_artifact(&contract);
        artifact.artifact_ref.clear();
        artifact.source.location.clear();
        artifact.digests.source_digest.clear();
        artifact.provenance.author.clear();
        artifact.provenance.created_at.clear();
        artifact.binary = None;
        artifact.workflow_ref = Some(WorkflowReference {
            workflow_id: "wf".to_string(),
            workflow_version: "1.0.0".to_string(),
        });

        let mut errors = Vec::new();
        validate_registration_fields(
            " ",
            "",
            &[String::new(), "dup".to_string(), "dup".to_string()],
            &artifact,
            &ComposabilityMetadata {
                kind: CompositionKind::Composite,
                patterns: vec![],
                provides: vec![String::new()],
                requires: vec!["dup".to_string(), "dup".to_string()],
            },
            &mut errors,
        );
        assert!(errors.len() >= 10);

        let mut workflow_artifact = executable_artifact(&contract);
        workflow_artifact.implementation_kind = ImplementationKind::Workflow;
        workflow_artifact.workflow_ref = None;
        let mut workflow_errors = Vec::new();
        validate_artifact(
            &workflow_artifact,
            &atomic_composability(),
            &mut workflow_errors,
        );
        assert_eq!(workflow_errors.len(), 3);
    }

    #[test]
    fn query_filters_cover_false_paths() {
        let contract = base_contract("content.comments.create-comment-draft", "1.0.0");
        let entry = build_index_entry(
            RegistryScope::Public,
            &contract,
            &executable_artifact(&contract),
            "2026-03-27T00:00:00Z",
            vec!["comments".to_string(), String::new()],
            atomic_composability(),
        );
        assert!(!matches_query(
            &entry,
            &DiscoveryQuery {
                owner_team: Some("missing-team".to_string()),
                ..DiscoveryQuery::default()
            }
        ));
        assert!(!matches_query(
            &entry,
            &DiscoveryQuery {
                lifecycle: Some(Lifecycle::Deprecated),
                ..DiscoveryQuery::default()
            }
        ));
        assert!(!matches_query(
            &entry,
            &DiscoveryQuery {
                implementation_kind: Some(ImplementationKind::Workflow),
                ..DiscoveryQuery::default()
            }
        ));
        assert!(!matches_query(
            &entry,
            &DiscoveryQuery {
                composition_kind: Some(CompositionKind::Composite),
                ..DiscoveryQuery::default()
            }
        ));
        assert!(!matches_query(
            &entry,
            &DiscoveryQuery {
                composition_pattern: Some(CompositionPattern::FanOut),
                ..DiscoveryQuery::default()
            }
        ));
        assert!(!matches_query(
            &entry,
            &DiscoveryQuery {
                emits_event_id: Some("missing.event".to_string()),
                ..DiscoveryQuery::default()
            }
        ));
        assert!(!matches_query(
            &entry,
            &DiscoveryQuery {
                consumes_event_id: Some("missing.event".to_string()),
                ..DiscoveryQuery::default()
            }
        ));
        assert!(!matches_query(
            &entry,
            &DiscoveryQuery {
                tag: Some("missing-tag".to_string()),
                ..DiscoveryQuery::default()
            }
        ));
    }

    #[test]
    fn helper_paths_cover_ordering_and_change_classification() {
        let contract = base_contract("content.comments.create-comment-draft", "1.0.0");
        let left = build_index_entry(
            RegistryScope::Public,
            &contract,
            &executable_artifact(&contract),
            "2026-03-27T00:00:00Z",
            vec!["comments".to_string()],
            atomic_composability(),
        );
        let other_id = DiscoveryIndexEntry {
            id: "content.alpha.other".to_string(),
            ..left.clone()
        };
        assert_eq!(compare_index_entries(&left, &other_id), Ordering::Greater);
        let newer = DiscoveryIndexEntry {
            version: "1.1.0".to_string(),
            ..left.clone()
        };
        assert_eq!(compare_index_entries(&left, &newer), Ordering::Greater);
        let private_same_version = DiscoveryIndexEntry {
            scope: RegistryScope::Private,
            ..left.clone()
        };
        assert_eq!(
            compare_index_entries(&left, &private_same_version),
            Ordering::Less
        );
        assert_eq!(
            compare_versions("invalid", "also-invalid"),
            "invalid".cmp("also-invalid")
        );

        let mut identity_change = base_contract("content.other.create-comment-draft", "2.0.0");
        identity_change.namespace = "content.other".to_string();
        assert_eq!(
            classify_contract_change(&contract, &identity_change),
            CompatibilityChangeClass::Breaking
        );

        let mut schema_change = base_contract("content.comments.create-comment-draft", "1.1.0");
        schema_change.inputs = SchemaContainer {
            schema: json!({"type": "object", "required": ["comment_text", "resource_id"]}),
        };
        assert_eq!(
            classify_contract_change(&contract, &schema_change),
            CompatibilityChangeClass::Unknown
        );

        let mut precondition_break =
            base_contract("content.comments.create-comment-draft", "2.0.0");
        precondition_break.preconditions.push(Condition {
            id: "resource-exists".to_string(),
            description: "Resource must already exist.".to_string(),
        });
        assert_eq!(
            classify_contract_change(&contract, &precondition_break),
            CompatibilityChangeClass::Breaking
        );

        let mut additive = base_contract("content.comments.create-comment-draft", "1.1.0");
        additive.emits.push(EventReference {
            event_id: "content.comments.comment-draft-indexed".to_string(),
            version: "1.0.0".to_string(),
        });
        assert_eq!(
            classify_contract_change(&contract, &additive),
            CompatibilityChangeClass::Additive
        );
        assert_eq!(
            classify_contract_change(
                &contract,
                &base_contract("content.comments.create-comment-draft", "1.0.1")
            ),
            CompatibilityChangeClass::MetadataOnly
        );
    }

    fn resolved_capability(previous_version: &str, record_version: &str) -> ResolvedCapability {
        let contract = base_contract("content.comments.create-comment-draft", previous_version);
        let artifact = executable_artifact(&contract);
        ResolvedCapability {
            contract: CapabilityContract {
                version: previous_version.to_string(),
                ..contract.clone()
            },
            record: CapabilityRegistryRecord {
                scope: RegistryScope::Public,
                id: contract.id.clone(),
                version: record_version.to_string(),
                lifecycle: contract.lifecycle.clone(),
                owner: contract.owner.clone(),
                contract_path:
                    "registry/public/content.comments.create-comment-draft/1.0.0/contract.json"
                        .to_string(),
                contract_digest: governed_content_digest(&contract),
                implementation_kind: ImplementationKind::Executable,
                artifact_ref: artifact.artifact_ref.clone(),
                registered_at: "2026-03-27T00:00:00Z".to_string(),
                provenance: artifact.provenance.clone(),
                evidence: RegistrationEvidence {
                    evidence_id: "evidence".to_string(),
                    artifact_ref: artifact.artifact_ref.clone(),
                    capability_id: contract.id.clone(),
                    capability_version: record_version.to_string(),
                    scope: RegistryScope::Public,
                    governing_spec: "005-capability-registry".to_string(),
                    validator_version: "test".to_string(),
                    produced_at: "2026-03-27T00:00:00Z".to_string(),
                    result: RegistrationResult::Passed,
                },
            },
            artifact: artifact.clone(),
            index_entry: build_index_entry(
                RegistryScope::Public,
                &contract,
                &artifact,
                "2026-03-27T00:00:00Z",
                vec!["comments".to_string()],
                atomic_composability(),
            ),
        }
    }

    fn registration(scope: RegistryScope, contract: CapabilityContract) -> CapabilityRegistration {
        CapabilityRegistration {
            scope,
            contract_path: format!(
                "registry/{}/{}/{}",
                scope_label(scope),
                contract.id,
                contract.version
            ) + "/contract.json",
            artifact: executable_artifact(&contract),
            registered_at: "2026-03-27T00:00:00Z".to_string(),
            tags: vec!["comments".to_string()],
            composability: atomic_composability(),
            governing_spec: "005-capability-registry".to_string(),
            validator_version: "test".to_string(),
            contract,
        }
    }

    fn executable_artifact(contract: &CapabilityContract) -> CapabilityArtifactRecord {
        CapabilityArtifactRecord {
            artifact_ref: format!("artifact:{}:{}", contract.name, contract.version),
            implementation_kind: ImplementationKind::Executable,
            source: SourceReference {
                kind: SourceKind::Git,
                location: format!("https://github.com/enricopiovesan/cogolo/{}", contract.name),
            },
            binary: Some(BinaryReference {
                format: BinaryFormat::Wasm,
                location: format!("artifacts/{}/{}.wasm", contract.name, contract.version),
            }),
            workflow_ref: None,
            digests: ArtifactDigests {
                source_digest: format!("source:{}:{}", contract.name, contract.version),
                binary_digest: Some(format!("binary:{}:{}", contract.name, contract.version)),
            },
            provenance: RegistryProvenance {
                source: "greenfield".to_string(),
                author: "enricopiovesan".to_string(),
                created_at: "2026-03-27T00:00:00Z".to_string(),
            },
        }
    }

    fn atomic_composability() -> ComposabilityMetadata {
        ComposabilityMetadata {
            kind: CompositionKind::Atomic,
            patterns: vec![CompositionPattern::Sequential],
            provides: vec!["comment-draft".to_string()],
            requires: vec!["validated-request".to_string()],
        }
    }

    fn base_contract(id: &str, version: &str) -> CapabilityContract {
        let (namespace, name) = split_id(id);
        CapabilityContract {
            kind: "capability_contract".to_string(),
            schema_version: "1.0.0".to_string(),
            id: id.to_string(),
            namespace,
            name,
            version: version.to_string(),
            lifecycle: Lifecycle::Active,
            owner: Owner {
                team: "traverse-core".to_string(),
                contact: "enrico.piovesan10@gmail.com".to_string(),
            },
            summary: "Create a validated comment draft for downstream composition.".to_string(),
            description: "Portable capability for creating a validated comment draft.".to_string(),
            inputs: SchemaContainer {
                schema: json!({"type": "object", "required": ["comment_text"]}),
            },
            outputs: SchemaContainer {
                schema: json!({"type": "object", "required": ["draft_id"]}),
            },
            preconditions: vec![Condition {
                id: "request-authenticated".to_string(),
                description: "Caller identity has already been established.".to_string(),
            }],
            postconditions: vec![Condition {
                id: "draft-created".to_string(),
                description: "A draft payload is produced.".to_string(),
            }],
            side_effects: vec![SideEffect {
                kind: SideEffectKind::MemoryOnly,
                description: "The capability produces in-memory state only.".to_string(),
            }],
            emits: vec![EventReference {
                event_id: "content.comments.comment-draft-created".to_string(),
                version: "1.0.0".to_string(),
            }],
            consumes: vec![],
            permissions: vec![IdReference {
                id: "comments.create".to_string(),
            }],
            execution: Execution {
                binary_format: ContractBinaryFormat::Wasm,
                entrypoint: Entrypoint {
                    kind: EntrypointKind::WasiCommand,
                    command: "run".to_string(),
                },
                preferred_targets: vec![ExecutionTarget::Local],
                constraints: ExecutionConstraints {
                    host_api_access: HostApiAccess::None,
                    network_access: NetworkAccess::Forbidden,
                    filesystem_access: FilesystemAccess::None,
                },
            },
            policies: vec![IdReference {
                id: "default-comment-safety".to_string(),
            }],
            dependencies: vec![DependencyReference {
                artifact_type: DependencyArtifactType::Event,
                id: "content.comments.comment-draft-created".to_string(),
                version: "1.0.0".to_string(),
            }],
            provenance: Provenance {
                source: ProvenanceSource::Greenfield,
                author: "enricopiovesan".to_string(),
                created_at: "2026-03-27T00:00:00Z".to_string(),
                spec_ref: Some("002-capability-contracts".to_string()),
                adr_refs: vec![],
                exception_refs: vec![],
            },
            evidence: vec![ValidationEvidence {
                evidence_id: "validation:contract".to_string(),
                evidence_type: EvidenceType::ContractValidation,
                status: EvidenceStatus::Passed,
            }],
            service_type: ServiceType::Stateless,
            permitted_targets: vec![
                ExecutionTarget::Local,
                ExecutionTarget::Cloud,
                ExecutionTarget::Edge,
                ExecutionTarget::Device,
            ],
            event_trigger: None,
            connector_requirements: Vec::new(),
        }
    }

    fn split_id(id: &str) -> (String, String) {
        let mut parts = id.rsplitn(2, '.');
        let name = parts.next().expect("id must include a name").to_string();
        let namespace = parts
            .next()
            .expect("id must include a namespace")
            .to_string();
        (namespace, name)
    }

    fn scope_label(scope: RegistryScope) -> &'static str {
        match scope {
            RegistryScope::Public => "public",
            RegistryScope::Private => "private",
        }
    }
}
