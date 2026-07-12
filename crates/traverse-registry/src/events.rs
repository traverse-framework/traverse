use crate::{LookupScope, RegistryScope};
use semver::Version;
use std::cmp::Ordering;
use std::collections::BTreeMap;
use traverse_contracts::{
    ErrorSeverity, EventClassification, EventContract, EventProvenance, EventValidationContext,
    Lifecycle, Owner, PublishedEventRecord, ValidationFailure, governed_event_content_digest,
    validate_event_contract,
};

const EVENT_REGISTRY_GOVERNING_SPEC: &str = "011-event-registry";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventRegistration {
    pub scope: RegistryScope,
    pub contract: EventContract,
    pub contract_path: String,
    pub registered_at: String,
    pub governing_spec: String,
    pub validator_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventRegistryRecord {
    pub scope: RegistryScope,
    pub id: String,
    pub version: String,
    pub lifecycle: Lifecycle,
    pub owner: Owner,
    pub summary: String,
    pub classification: EventClassification,
    pub publishers: Vec<String>,
    pub subscribers: Vec<String>,
    pub payload_schema_digest: String,
    pub contract_digest: String,
    pub contract_path: String,
    pub validation_evidence: EventRegistrationEvidence,
    pub registered_at: String,
    pub tags: Vec<String>,
    pub policy_refs: Vec<String>,
    pub provenance: EventProvenance,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventRegistryIndexRecord {
    pub scope: RegistryScope,
    pub id: String,
    pub version: String,
    pub lifecycle: Lifecycle,
    pub summary: String,
    pub publisher_count: usize,
    pub subscriber_count: usize,
    pub classification: EventClassification,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventRegistrationEvidence {
    pub kind: String,
    pub schema_version: String,
    pub governing_spec: String,
    pub status: EventRegistrationStatus,
    pub scope: RegistryScope,
    pub id: String,
    pub version: String,
    pub contract_digest: String,
    pub validated_at: String,
    pub checks: Vec<String>,
    pub violations: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventRegistrationStatus {
    Passed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventRegistrationOutcome {
    pub record: EventRegistryRecord,
    pub index_record: EventRegistryIndexRecord,
    pub evidence: EventRegistrationEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedEvent {
    pub contract: EventContract,
    pub record: EventRegistryRecord,
    pub index_record: EventRegistryIndexRecord,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventLineageRecord {
    pub scope: RegistryScope,
    pub id: String,
    pub versions: Vec<EventLineageVersion>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventLineageVersion {
    pub version: String,
    pub lifecycle: Lifecycle,
    pub contract_digest: String,
    pub registered_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventCompatibilityChangeClass {
    MetadataOnly,
    Additive,
    Breaking,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventDeclaredVersionBump {
    Patch,
    Minor,
    Major,
}

impl PartialOrd for EventDeclaredVersionBump {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for EventDeclaredVersionBump {
    fn cmp(&self, other: &Self) -> Ordering {
        use EventDeclaredVersionBump::{Major, Minor, Patch};
        match (self, other) {
            (Patch, Patch) | (Minor, Minor) | (Major, Major) => Ordering::Equal,
            (Patch, _) | (Minor, Major) => Ordering::Less,
            (Minor, Patch) | (Major, _) => Ordering::Greater,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventVersionCompatibilityRecord {
    pub event_id: String,
    pub previous_version: String,
    pub candidate_version: String,
    pub detected_change_class: EventCompatibilityChangeClass,
    pub declared_bump: EventDeclaredVersionBump,
    pub result: EventCompatibilityResult,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventCompatibilityResult {
    Passed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventRegistryErrorCode {
    ContractValidationFailed,
    MissingRequiredField,
    DuplicateItem,
    ImmutableVersionConflict,
    InvalidSemverProgression,
    SemverTooSmall,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventRegistryError {
    pub code: EventRegistryErrorCode,
    pub target: String,
    pub message: String,
    pub severity: ErrorSeverity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventRegistryFailure {
    pub errors: Vec<EventRegistryError>,
}

#[derive(Debug, Clone, Default)]
pub struct EventRegistry {
    contracts: BTreeMap<(RegistryScope, String, String), EventContract>,
    records: BTreeMap<(RegistryScope, String, String), EventRegistryRecord>,
    index: BTreeMap<(RegistryScope, String, String), EventRegistryIndexRecord>,
    compatibility: Vec<EventVersionCompatibilityRecord>,
}

impl EventRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers one governed event contract into the event registry.
    ///
    /// # Errors
    ///
    /// Returns [`EventRegistryFailure`] when registration metadata is missing,
    /// event contract validation fails, immutable publication semantics would be
    /// violated, or semver progression is too small for the detected payload
    /// compatibility change.
    pub fn register(
        &mut self,
        request: EventRegistration,
    ) -> Result<EventRegistrationOutcome, EventRegistryFailure> {
        let EventRegistration {
            scope,
            contract,
            contract_path,
            registered_at,
            governing_spec,
            validator_version,
        } = request;

        let mut errors = Vec::new();
        validate_registration_fields(&contract_path, &registered_at, &governing_spec, &mut errors);
        if !errors.is_empty() {
            return Err(EventRegistryFailure { errors });
        }

        let key = (scope, contract.id.clone(), contract.version.clone());
        let existing_published = self.records.get(&key).map(published_event_record);
        let validated = validate_event_contract(
            contract,
            &EventValidationContext {
                governing_spec: &governing_spec,
                validator_version: &validator_version,
                existing_published: existing_published.as_ref(),
            },
        )
        .map_err(map_event_contract_failure)?;

        let contract = validated.normalized;
        let contract_digest = governed_event_content_digest(&contract);
        let payload_schema_digest = payload_schema_digest(&contract);
        let record = build_record(
            scope,
            &contract,
            &contract_path,
            &registered_at,
            &governing_spec,
            &contract_digest,
            &payload_schema_digest,
        );
        let index_record = build_index_record(scope, &contract);

        if let Some(existing) = self.records.get(&key) {
            let Some(existing_contract) = self.contracts.get(&key) else {
                return Err(single_event_error(
                    EventRegistryErrorCode::ImmutableVersionConflict,
                    "$.id",
                    "existing event record is missing its authoritative contract",
                ));
            };
            let Some(existing_index) = self.index.get(&key) else {
                return Err(single_event_error(
                    EventRegistryErrorCode::ImmutableVersionConflict,
                    "$.id",
                    "existing event record is missing its index record",
                ));
            };

            if records_match_ignoring_registration_timestamp(existing, &record)
                && existing_index == &index_record
                && existing_contract == &contract
            {
                return Ok(EventRegistrationOutcome {
                    record: existing.clone(),
                    index_record: existing_index.clone(),
                    evidence: existing.validation_evidence.clone(),
                });
            }

            if governed_event_content_digest(existing_contract) != contract_digest {
                return Err(single_event_error(
                    EventRegistryErrorCode::ImmutableVersionConflict,
                    "$.version",
                    "published event versions are immutable within a scope",
                ));
            }

            return Err(single_event_error(
                EventRegistryErrorCode::ImmutableVersionConflict,
                "$.contract_path",
                "published event versions are immutable and cannot be republished with different metadata",
            ));
        }

        let compatibility = if let Some(previous) =
            self.latest_prior_record(scope, &contract.id, &contract.version)
        {
            Some(validate_semver_progression(previous, &contract)?)
        } else {
            None
        };

        self.contracts.insert(key.clone(), contract);
        self.records.insert(key.clone(), record.clone());
        self.index.insert(key, index_record.clone());
        if let Some(compatibility) = compatibility {
            self.compatibility.push(compatibility);
        }

        Ok(EventRegistrationOutcome {
            evidence: record.validation_evidence.clone(),
            record,
            index_record,
        })
    }

    #[must_use]
    pub fn find_exact(
        &self,
        lookup_scope: LookupScope,
        id: &str,
        version: &str,
    ) -> Option<ResolvedEvent> {
        for &scope in lookup_order(lookup_scope) {
            let key = (scope, id.to_string(), version.to_string());
            if let Some(record) = self.records.get(&key) {
                let contract = self.contracts.get(&key)?.clone();
                let index_record = self.index.get(&key)?.clone();
                return Some(ResolvedEvent {
                    contract,
                    record: record.clone(),
                    index_record,
                });
            }
        }
        None
    }

    #[must_use]
    pub fn discover(&self, lookup_scope: LookupScope) -> Vec<EventRegistryIndexRecord> {
        let mut results = Vec::new();
        let mut shadowed = std::collections::BTreeSet::new();

        for &scope in lookup_order(lookup_scope) {
            let entries = self
                .index
                .iter()
                .filter(|((entry_scope, _, _), _)| *entry_scope == scope);

            for ((_, id, version), entry) in entries {
                if lookup_scope == LookupScope::PreferPrivate
                    && scope == RegistryScope::Public
                    && shadowed.contains(&(id.clone(), version.clone()))
                {
                    continue;
                }

                if scope == RegistryScope::Private {
                    shadowed.insert((id.clone(), version.clone()));
                }

                results.push(entry.clone());
            }
        }

        results.sort_by(|left, right| {
            left.id
                .cmp(&right.id)
                .then_with(|| compare_versions(&right.version, &left.version))
                .then_with(|| left.scope.cmp(&right.scope))
        });
        results
    }

    #[must_use]
    pub fn lineage(&self, scope: RegistryScope, id: &str) -> Option<EventLineageRecord> {
        let mut versions = self
            .records
            .iter()
            .filter(|((entry_scope, entry_id, _), _)| *entry_scope == scope && entry_id == id)
            .map(|((_, _, version), record)| EventLineageVersion {
                version: version.clone(),
                lifecycle: record.lifecycle.clone(),
                contract_digest: record.contract_digest.clone(),
                registered_at: record.registered_at.clone(),
            })
            .collect::<Vec<_>>();

        if versions.is_empty() {
            return None;
        }

        versions.sort_by(|left, right| compare_versions(&left.version, &right.version));
        Some(EventLineageRecord {
            scope,
            id: id.to_string(),
            versions,
        })
    }

    #[must_use]
    pub fn compatibility_records(&self) -> &[EventVersionCompatibilityRecord] {
        &self.compatibility
    }

    #[must_use]
    pub(crate) fn graph_entries(&self) -> Vec<ResolvedEvent> {
        self.records
            .iter()
            .filter_map(|((scope, id, version), record)| {
                let key = (*scope, id.clone(), version.clone());
                let contract = self.contracts.get(&key)?.clone();
                let index_record = self.index.get(&key)?.clone();
                Some(ResolvedEvent {
                    contract,
                    record: record.clone(),
                    index_record,
                })
            })
            .collect()
    }

    fn latest_prior_record(
        &self,
        scope: RegistryScope,
        id: &str,
        candidate_version: &str,
    ) -> Option<ResolvedEvent> {
        let candidate = Version::parse(candidate_version).ok()?;
        let mut best: Option<(String, EventRegistryRecord)> = None;

        for ((entry_scope, entry_id, entry_version), record) in &self.records {
            if *entry_scope != scope || entry_id != id {
                continue;
            }
            let Ok(entry) = Version::parse(entry_version) else {
                continue;
            };
            if entry >= candidate {
                continue;
            }

            match &best {
                Some((best_version, _))
                    if compare_versions(entry_version, best_version) != Ordering::Greater => {}
                _ => {
                    best = Some((entry_version.clone(), record.clone()));
                }
            }
        }

        best.and_then(|(version, record)| {
            let key = (scope, id.to_string(), version);
            let contract = self.contracts.get(&key)?.clone();
            let index_record = self.index.get(&key)?.clone();
            Some(ResolvedEvent {
                contract,
                record,
                index_record,
            })
        })
    }
}

fn build_record(
    scope: RegistryScope,
    contract: &EventContract,
    contract_path: &str,
    registered_at: &str,
    governing_spec: &str,
    contract_digest: &str,
    payload_schema_digest: &str,
) -> EventRegistryRecord {
    let validation_evidence = EventRegistrationEvidence {
        kind: "event_registry_registration_evidence".to_string(),
        schema_version: "1.0.0".to_string(),
        governing_spec: governing_spec.to_string(),
        status: EventRegistrationStatus::Passed,
        scope,
        id: contract.id.clone(),
        version: contract.version.clone(),
        contract_digest: contract_digest.to_string(),
        validated_at: registered_at.to_string(),
        checks: vec![
            "event_contract_valid".to_string(),
            "scope_valid".to_string(),
            "immutable_version_clear".to_string(),
            "semver_progression_valid".to_string(),
        ],
        violations: vec![],
    };

    EventRegistryRecord {
        scope,
        id: contract.id.clone(),
        version: contract.version.clone(),
        lifecycle: contract.lifecycle.clone(),
        owner: contract.owner.clone(),
        summary: contract.summary.clone(),
        classification: contract.classification.clone(),
        publishers: contract
            .publishers
            .iter()
            .map(|publisher| publisher.capability_id.clone())
            .collect(),
        subscribers: contract
            .subscribers
            .iter()
            .map(|subscriber| subscriber.capability_id.clone())
            .collect(),
        payload_schema_digest: payload_schema_digest.to_string(),
        contract_digest: contract_digest.to_string(),
        contract_path: contract_path.to_string(),
        validation_evidence,
        registered_at: registered_at.to_string(),
        tags: normalized_tags(contract),
        policy_refs: contract
            .policies
            .iter()
            .map(|policy| policy.id.clone())
            .collect(),
        provenance: contract.provenance.clone(),
    }
}

fn build_index_record(scope: RegistryScope, contract: &EventContract) -> EventRegistryIndexRecord {
    EventRegistryIndexRecord {
        scope,
        id: contract.id.clone(),
        version: contract.version.clone(),
        lifecycle: contract.lifecycle.clone(),
        summary: contract.summary.clone(),
        publisher_count: contract.publishers.len(),
        subscriber_count: contract.subscribers.len(),
        classification: contract.classification.clone(),
        tags: normalized_tags(contract),
    }
}

/// `registered_at` (and the `validated_at` it seeds on the validation evidence)
/// is a server-generated wall-clock stamp, not part of the client's request.
/// Comparing it verbatim made an identical resubmission flip from "already
/// registered" to a spurious immutable-version conflict whenever the two
/// calls landed in different seconds.
fn records_match_ignoring_registration_timestamp(
    existing: &EventRegistryRecord,
    candidate: &EventRegistryRecord,
) -> bool {
    let mut normalized_existing = existing.clone();
    normalized_existing
        .registered_at
        .clone_from(&candidate.registered_at);
    normalized_existing
        .validation_evidence
        .validated_at
        .clone_from(&candidate.validation_evidence.validated_at);
    &normalized_existing == candidate
}

fn normalized_tags(contract: &EventContract) -> Vec<String> {
    let mut tags = contract.tags.clone();
    tags.sort();
    tags.dedup();
    tags
}

fn payload_schema_digest(contract: &EventContract) -> String {
    let json = format!("{:?}", contract.payload.schema);
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in json.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0001_0000_01b3);
    }
    format!("0.1.0:{hash:016x}")
}

fn validate_registration_fields(
    contract_path: &str,
    registered_at: &str,
    governing_spec: &str,
    errors: &mut Vec<EventRegistryError>,
) {
    if contract_path.trim().is_empty() {
        errors.push(event_error(
            EventRegistryErrorCode::MissingRequiredField,
            "$.contract_path",
            "contract_path must be non-empty",
        ));
    }
    if registered_at.trim().is_empty() {
        errors.push(event_error(
            EventRegistryErrorCode::MissingRequiredField,
            "$.registered_at",
            "registered_at must be non-empty",
        ));
    }
    if governing_spec != EVENT_REGISTRY_GOVERNING_SPEC {
        errors.push(event_error(
            EventRegistryErrorCode::MissingRequiredField,
            "$.governing_spec",
            "governing_spec must equal 011-event-registry",
        ));
    }
}

fn published_event_record(record: &EventRegistryRecord) -> PublishedEventRecord {
    PublishedEventRecord {
        id: record.id.clone(),
        version: record.version.clone(),
        governed_content_digest: record.contract_digest.clone(),
        lifecycle: record.lifecycle.clone(),
    }
}

fn map_event_contract_failure(failure: ValidationFailure) -> EventRegistryFailure {
    EventRegistryFailure {
        errors: failure
            .errors
            .into_iter()
            .map(|error| EventRegistryError {
                code: match error.code {
                    traverse_contracts::ValidationErrorCode::ImmutableVersionConflict => {
                        EventRegistryErrorCode::ImmutableVersionConflict
                    }
                    _ => EventRegistryErrorCode::ContractValidationFailed,
                },
                target: error.path,
                message: error.message,
                severity: error.severity,
            })
            .collect(),
    }
}

fn validate_semver_progression(
    previous: ResolvedEvent,
    candidate: &EventContract,
) -> Result<EventVersionCompatibilityRecord, EventRegistryFailure> {
    let previous_version = Version::parse(&previous.record.version).map_err(|_| {
        single_event_error(
            EventRegistryErrorCode::InvalidSemverProgression,
            "$.version",
            "previous published version is not valid semver",
        )
    })?;
    let candidate_version = Version::parse(&candidate.version).map_err(|_| {
        single_event_error(
            EventRegistryErrorCode::InvalidSemverProgression,
            "$.version",
            "candidate version is not valid semver",
        )
    })?;

    if candidate_version <= previous_version {
        return Err(single_event_error(
            EventRegistryErrorCode::InvalidSemverProgression,
            "$.version",
            "candidate version must be greater than the previous published version",
        ));
    }

    let declared_bump = declared_bump(&previous_version, &candidate_version);
    let detected_change_class = classify_event_change(&previous.contract, candidate);
    let required_bump = match detected_change_class {
        EventCompatibilityChangeClass::MetadataOnly => EventDeclaredVersionBump::Patch,
        EventCompatibilityChangeClass::Additive => EventDeclaredVersionBump::Minor,
        EventCompatibilityChangeClass::Breaking => EventDeclaredVersionBump::Major,
    };

    if declared_bump < required_bump {
        return Err(single_event_error(
            EventRegistryErrorCode::SemverTooSmall,
            "$.version",
            "declared semver bump is too small for the detected event compatibility change",
        ));
    }

    Ok(EventVersionCompatibilityRecord {
        event_id: candidate.id.clone(),
        previous_version: previous.record.version,
        candidate_version: candidate.version.clone(),
        detected_change_class,
        declared_bump,
        result: EventCompatibilityResult::Passed,
    })
}

fn classify_event_change(
    previous: &EventContract,
    candidate: &EventContract,
) -> EventCompatibilityChangeClass {
    let payload_changed =
        format!("{:?}", previous.payload.schema) != format!("{:?}", candidate.payload.schema);
    if payload_changed {
        return match candidate.payload.compatibility {
            traverse_contracts::PayloadCompatibility::BackwardCompatible => {
                EventCompatibilityChangeClass::Additive
            }
            traverse_contracts::PayloadCompatibility::ForwardCompatible
            | traverse_contracts::PayloadCompatibility::Breaking => {
                EventCompatibilityChangeClass::Breaking
            }
        };
    }

    if previous.summary != candidate.summary
        || previous.description != candidate.description
        || previous.tags != candidate.tags
        || previous.lifecycle != candidate.lifecycle
        || previous.publishers != candidate.publishers
        || previous.subscribers != candidate.subscribers
        || previous.policies != candidate.policies
        || previous.classification != candidate.classification
    {
        return EventCompatibilityChangeClass::MetadataOnly;
    }

    EventCompatibilityChangeClass::MetadataOnly
}

fn declared_bump(previous: &Version, candidate: &Version) -> EventDeclaredVersionBump {
    if candidate.major > previous.major {
        EventDeclaredVersionBump::Major
    } else if candidate.minor > previous.minor {
        EventDeclaredVersionBump::Minor
    } else {
        EventDeclaredVersionBump::Patch
    }
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

fn single_event_error(
    code: EventRegistryErrorCode,
    target: &str,
    message: &str,
) -> EventRegistryFailure {
    EventRegistryFailure {
        errors: vec![event_error(code, target, message)],
    }
}

fn event_error(code: EventRegistryErrorCode, target: &str, message: &str) -> EventRegistryError {
    EventRegistryError {
        code,
        target: target.to_string(),
        message: message.to_string(),
        severity: ErrorSeverity::Error,
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::needless_pass_by_value)]
mod tests {
    use super::*;
    use serde_json::json;
    use traverse_contracts::{
        CapabilityReference, EventPayload, EventProvenanceSource, EventType, IdReference,
        PayloadCompatibility,
    };

    #[test]
    fn helper_paths_cover_remaining_ordering_lookup_and_classification_branches() {
        assert!(EventDeclaredVersionBump::Minor > EventDeclaredVersionBump::Patch);
        assert!(EventDeclaredVersionBump::Major > EventDeclaredVersionBump::Minor);
        assert_eq!(
            compare_versions("not-semver", "1.0.0"),
            "not-semver".cmp("1.0.0")
        );

        let mut changed_metadata =
            base_event_contract("content.comments.comment-draft-created", "1.0.0");
        changed_metadata.summary = "Changed summary only".to_string();
        assert_eq!(
            classify_event_change(
                &base_event_contract("content.comments.comment-draft-created", "1.0.0"),
                &changed_metadata
            ),
            EventCompatibilityChangeClass::MetadataOnly
        );

        assert_eq!(
            classify_event_change(
                &base_event_contract("content.comments.comment-draft-created", "1.0.0"),
                &base_event_contract("content.comments.comment-draft-created", "1.0.0")
            ),
            EventCompatibilityChangeClass::MetadataOnly
        );
    }

    #[test]
    fn register_reports_invalid_metadata_and_contract_validation_failures() {
        let mut registry = EventRegistry::new();
        let mut request = event_registration(
            RegistryScope::Public,
            base_event_contract("content.comments.comment-draft-created", "1.0.0"),
        );
        request.contract_path.clear();
        request.registered_at.clear();
        request.governing_spec = "wrong-spec".to_string();

        let failure = registry
            .register(request)
            .expect_err("metadata validation should fail");
        assert_eq!(failure.errors.len(), 3);
        assert_eq!(failure.errors[0].target, "$.contract_path");
        assert_eq!(failure.errors[1].target, "$.registered_at");
        assert_eq!(failure.errors[2].target, "$.governing_spec");

        let mut invalid_contract =
            base_event_contract("content.comments.comment-draft-created", "1.0.0");
        invalid_contract.kind = "wrong-kind".to_string();
        let failure = registry
            .register(event_registration(RegistryScope::Public, invalid_contract))
            .expect_err("contract validation failure should map cleanly");
        assert_eq!(
            failure.errors[0].code,
            EventRegistryErrorCode::ContractValidationFailed
        );
    }

    #[test]
    fn register_covers_internal_consistency_and_metadata_immutability_guards() {
        let request = event_registration(
            RegistryScope::Public,
            base_event_contract("content.comments.comment-draft-created", "1.0.0"),
        );
        let key = (
            RegistryScope::Public,
            "content.comments.comment-draft-created".to_string(),
            "1.0.0".to_string(),
        );

        let mut missing_contract_registry = EventRegistry::new();
        missing_contract_registry
            .register(request.clone())
            .expect("seed registration should pass");
        missing_contract_registry.contracts.remove(&key);
        let failure = missing_contract_registry
            .register(request.clone())
            .expect_err("missing contract guard should fire");
        assert_eq!(
            failure.errors[0].message,
            "existing event record is missing its authoritative contract"
        );

        let mut missing_index_registry = EventRegistry::new();
        missing_index_registry
            .register(request.clone())
            .expect("seed registration should pass");
        missing_index_registry.index.remove(&key);
        let failure = missing_index_registry
            .register(request.clone())
            .expect_err("missing index guard should fire");
        assert_eq!(
            failure.errors[0].message,
            "existing event record is missing its index record"
        );

        let mut metadata_change_registry = EventRegistry::new();
        metadata_change_registry
            .register(request.clone())
            .expect("seed registration should pass");
        let mut metadata_changed = request.clone();
        metadata_changed.contract_path = "registry/public/alternate/path.json".to_string();
        let failure = metadata_change_registry
            .register(metadata_changed)
            .expect_err("metadata-only republish should fail");
        assert_eq!(
            failure.errors[0].message,
            "published event versions are immutable and cannot be republished with different metadata"
        );

        let mut retried_registration_registry = EventRegistry::new();
        retried_registration_registry
            .register(request.clone())
            .expect("seed registration should pass");
        let mut retried_with_later_timestamp = request.clone();
        retried_with_later_timestamp.registered_at = "2026-03-30T00:00:01Z".to_string();
        let outcome = retried_registration_registry
            .register(retried_with_later_timestamp)
            .expect("resubmitting identical content with a newer registration timestamp must be idempotent");
        assert_eq!(outcome.record.registered_at, request.registered_at);

        let mut digest_mismatch_registry = EventRegistry::new();
        digest_mismatch_registry
            .register(request.clone())
            .expect("seed registration should pass");
        let mut drifted_contract =
            base_event_contract("content.comments.comment-draft-created", "1.0.0");
        drifted_contract.summary = "Registry drifted from published content.".to_string();
        digest_mismatch_registry
            .contracts
            .insert(key, drifted_contract);
        let failure = digest_mismatch_registry
            .register(request)
            .expect_err("digest mismatch should trip immutable version guard");
        assert_eq!(
            failure.errors[0].message,
            "published event versions are immutable within a scope"
        );
    }

    #[test]
    fn exact_lookup_lineage_and_prior_record_cover_false_paths() {
        let registry = EventRegistry::new();
        assert!(
            registry
                .find_exact(
                    LookupScope::PublicOnly,
                    "content.comments.comment-draft-created",
                    "1.0.0"
                )
                .is_none()
        );
        assert!(
            registry
                .lineage(
                    RegistryScope::Public,
                    "content.comments.comment-draft-created"
                )
                .is_none()
        );
        let private_request = event_registration(
            RegistryScope::Private,
            base_event_contract("content.comments.private-comment-draft-created", "1.0.0"),
        );
        assert_eq!(
            private_request.contract_path,
            "registry/private/content.comments.private-comment-draft-created/1.0.0/contract.json"
        );

        let mut registry = EventRegistry::new();
        insert_event_fixture(
            &mut registry,
            RegistryScope::Public,
            base_event_contract("content.comments.comment-draft-created", "1.0.0"),
        );
        insert_event_fixture(
            &mut registry,
            RegistryScope::Public,
            base_event_contract("content.comments.comment-draft-created", "2.0.0"),
        );
        insert_event_fixture(
            &mut registry,
            RegistryScope::Public,
            base_event_contract("content.comments.comment-draft-created", "0.9.0"),
        );
        insert_event_fixture(
            &mut registry,
            RegistryScope::Private,
            base_event_contract("content.comments.private-comment-draft-created", "0.8.0"),
        );
        insert_event_fixture(
            &mut registry,
            RegistryScope::Public,
            base_event_contract("content.comments.other-event", "0.8.0"),
        );

        let invalid_key = (
            RegistryScope::Public,
            "content.comments.comment-draft-created".to_string(),
            "invalid".to_string(),
        );
        let invalid_contract =
            base_event_contract("content.comments.comment-draft-created", "invalid");
        let invalid_digest = governed_event_content_digest(&invalid_contract);
        let invalid_payload_digest = payload_schema_digest(&invalid_contract);
        registry
            .contracts
            .insert(invalid_key.clone(), invalid_contract.clone());
        registry.records.insert(
            invalid_key.clone(),
            build_record(
                RegistryScope::Public,
                &invalid_contract,
                "registry/public/content.comments.comment-draft-created/invalid/contract.json",
                "2026-03-30T00:00:00Z",
                EVENT_REGISTRY_GOVERNING_SPEC,
                &invalid_digest,
                &invalid_payload_digest,
            ),
        );
        registry.index.insert(
            invalid_key,
            build_index_record(RegistryScope::Public, &invalid_contract),
        );

        let prior = registry
            .latest_prior_record(
                RegistryScope::Public,
                "content.comments.comment-draft-created",
                "1.5.0",
            )
            .expect("a prior version should be found");
        assert_eq!(prior.record.version, "1.0.0");
    }

    #[test]
    fn discover_covers_public_only_and_private_overlay_paths() {
        let mut registry = EventRegistry::new();
        insert_event_fixture(
            &mut registry,
            RegistryScope::Public,
            base_event_contract("content.comments.comment-draft-created", "1.0.0"),
        );
        insert_event_fixture(
            &mut registry,
            RegistryScope::Private,
            base_event_contract("content.comments.comment-draft-created", "1.0.0"),
        );
        insert_event_fixture(
            &mut registry,
            RegistryScope::Public,
            base_event_contract("content.comments.comment-published", "1.1.0"),
        );

        let public_only = registry.discover(LookupScope::PublicOnly);
        let prefer_private = registry.discover(LookupScope::PreferPrivate);

        assert_eq!(public_only.len(), 2);
        assert_eq!(prefer_private.len(), 2);
        assert_eq!(
            prefer_private[0].id,
            "content.comments.comment-draft-created"
        );
        assert_eq!(prefer_private[0].scope, RegistryScope::Private);
    }

    #[test]
    fn semver_progression_guards_cover_invalid_and_non_increasing_versions() {
        let previous = resolved_event(base_event_contract(
            "content.comments.comment-draft-created",
            "1.0.0",
        ));

        let metadata_only = validate_semver_progression(
            previous.clone(),
            &base_event_contract("content.comments.comment-draft-created", "1.0.1"),
        )
        .expect("patch metadata-only change should pass");
        assert_eq!(
            metadata_only.detected_change_class,
            EventCompatibilityChangeClass::MetadataOnly
        );

        let mut invalid_candidate =
            base_event_contract("content.comments.comment-draft-created", "not-semver");
        invalid_candidate.summary = "Changed summary only".to_string();
        let failure = validate_semver_progression(previous.clone(), &invalid_candidate)
            .expect_err("invalid candidate semver should fail");
        assert_eq!(
            failure.errors[0].code,
            EventRegistryErrorCode::InvalidSemverProgression
        );
        assert_eq!(
            failure.errors[0].message,
            "candidate version is not valid semver"
        );

        let mut same_version =
            base_event_contract("content.comments.comment-draft-created", "1.0.0");
        same_version.summary = "Changed summary only".to_string();
        let failure = validate_semver_progression(previous, &same_version)
            .expect_err("non-increasing semver should fail");
        assert_eq!(
            failure.errors[0].message,
            "candidate version must be greater than the previous published version"
        );

        let invalid_previous = resolved_event(base_event_contract(
            "content.comments.comment-draft-created",
            "not-semver",
        ));
        let mut candidate = base_event_contract("content.comments.comment-draft-created", "1.0.1");
        candidate.summary = "Changed summary only".to_string();
        let failure = validate_semver_progression(invalid_previous, &candidate)
            .expect_err("invalid previous semver should fail");
        assert_eq!(
            failure.errors[0].message,
            "previous published version is not valid semver"
        );
    }

    fn insert_event_fixture(
        registry: &mut EventRegistry,
        scope: RegistryScope,
        contract: EventContract,
    ) {
        let key = (scope, contract.id.clone(), contract.version.clone());
        let contract_digest = governed_event_content_digest(&contract);
        let payload_digest = payload_schema_digest(&contract);
        registry.contracts.insert(key.clone(), contract.clone());
        registry.records.insert(
            key.clone(),
            build_record(
                scope,
                &contract,
                &format!(
                    "registry/{}/{}/{}/contract.json",
                    match scope {
                        RegistryScope::Public => "public",
                        RegistryScope::Private => "private",
                    },
                    contract.id,
                    contract.version
                ),
                "2026-03-30T00:00:00Z",
                EVENT_REGISTRY_GOVERNING_SPEC,
                &contract_digest,
                &payload_digest,
            ),
        );
        registry
            .index
            .insert(key, build_index_record(scope, &contract));
    }

    fn resolved_event(contract: EventContract) -> ResolvedEvent {
        let scope = RegistryScope::Public;
        let contract_digest = governed_event_content_digest(&contract);
        let payload_digest = payload_schema_digest(&contract);
        let record = build_record(
            scope,
            &contract,
            &format!(
                "registry/public/{}/{}/contract.json",
                contract.id, contract.version
            ),
            "2026-03-30T00:00:00Z",
            EVENT_REGISTRY_GOVERNING_SPEC,
            &contract_digest,
            &payload_digest,
        );
        let index_record = build_index_record(scope, &contract);
        ResolvedEvent {
            contract,
            record,
            index_record,
        }
    }

    fn event_registration(scope: RegistryScope, contract: EventContract) -> EventRegistration {
        EventRegistration {
            scope,
            contract_path: format!(
                "registry/{}/{}/{}/contract.json",
                match scope {
                    RegistryScope::Public => "public",
                    RegistryScope::Private => "private",
                },
                contract.id,
                contract.version
            ),
            registered_at: "2026-03-30T00:00:00Z".to_string(),
            governing_spec: EVENT_REGISTRY_GOVERNING_SPEC.to_string(),
            validator_version: "registry-test".to_string(),
            contract,
        }
    }

    fn base_event_contract(id: &str, version: &str) -> EventContract {
        let (namespace, name) = split_event_id(id);
        EventContract {
            kind: "event_contract".to_string(),
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
            summary: "Published when a comment draft has been created.".to_string(),
            description: "Governed event contract for comment draft creation.".to_string(),
            payload: EventPayload {
                schema: json!({
                    "type": "object",
                    "required": ["draft_id"],
                    "properties": {
                        "draft_id": {"type": "string"}
                    }
                }),
                compatibility: PayloadCompatibility::BackwardCompatible,
            },
            classification: EventClassification {
                domain: "content.comments".to_string(),
                bounded_context: "comments".to_string(),
                event_type: EventType::Domain,
                tags: vec!["comments".to_string(), "draft".to_string()],
            },
            publishers: vec![CapabilityReference {
                capability_id: "content.comments.create-comment-draft".to_string(),
                version: "1.0.0".to_string(),
            }],
            subscribers: vec![CapabilityReference {
                capability_id: "content.comments.publish-comment".to_string(),
                version: "1.0.0".to_string(),
            }],
            policies: vec![IdReference {
                id: "default-comment-safety".to_string(),
            }],
            tags: vec!["comments".to_string(), "draft".to_string()],
            provenance: EventProvenance {
                source: EventProvenanceSource::Greenfield,
                author: "enricopiovesan".to_string(),
                created_at: "2026-03-30T00:00:00Z".to_string(),
            },
            evidence: vec![],
        }
    }

    fn split_event_id(id: &str) -> (String, String) {
        let mut parts = id.rsplitn(2, '.');
        let name = parts.next().expect("event id must include a name");
        let namespace = parts.next().expect("event id must include a namespace");
        (namespace.to_string(), name.to_string())
    }
}
