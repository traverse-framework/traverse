//! Diagnosis harness for exact-version `registry_ref` resolution failures
//! (issue #1273, governing spec `1258-offline-cache-activation`).
//!
//! This module is **diagnosis only** and is compiled under `#[cfg(test)]`. It
//! does not change resolution or activation semantics. It captures, as an
//! executable specification, the stable per-boundary error taxonomy that the
//! successor implementation slice (#1272) must surface from
//! [`crate::SyncedRegistryComponentResolver`].
//!
//! Today every boundary below collapses onto the single
//! `ApplicationManifestErrorCode::RegistryReferenceRequiresResolution` code, and
//! [`RegistryResolutionStage::Signature`], [`RegistryResolutionStage::Abi`], and
//! [`RegistryResolutionStage::Target`] are not evaluated at all. See
//! `docs/registry-resolution-diagnosis.md` and
//! `tests/fixtures/registry-resolution/boundary-error-map.json`.

/// One boundary in exact-version `registry_ref` resolution, in evaluation
/// order. [`classify`] reports the first boundary that fails, so the ordering is
/// part of the contract: an earlier failure hides later ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RegistryResolutionStage {
    /// Synced state presence/readability and a record for the namespace/id.
    IndexSelection,
    /// The exact `==x.y.z` range selects a synced version (never a fallback).
    VersionRange,
    /// The selected version is activatable (not deprecated/yanked/draft).
    Lifecycle,
    /// Contract bytes are retrievable.
    ContractRetrieval,
    /// Contract bytes match `contract_digest`.
    ContractDigest,
    /// Artifact bytes are retrievable.
    ArtifactRetrieval,
    /// Artifact bytes match `digest`.
    ArtifactDigest,
    /// The signed trust bundle verifies.
    Signature,
    /// The artifact Host ABI major is supported by this build.
    Abi,
    /// The selected version permits the requested placement target.
    Target,
    /// Verified bytes commit to the immutable digest-keyed cache.
    CachePersistence,
}

impl RegistryResolutionStage {
    /// Every boundary, in evaluation order.
    pub const ALL: [RegistryResolutionStage; 11] = [
        RegistryResolutionStage::IndexSelection,
        RegistryResolutionStage::VersionRange,
        RegistryResolutionStage::Lifecycle,
        RegistryResolutionStage::ContractRetrieval,
        RegistryResolutionStage::ContractDigest,
        RegistryResolutionStage::ArtifactRetrieval,
        RegistryResolutionStage::ArtifactDigest,
        RegistryResolutionStage::Signature,
        RegistryResolutionStage::Abi,
        RegistryResolutionStage::Target,
        RegistryResolutionStage::CachePersistence,
    ];

    /// Stable snake_case identifier used in the fixture map and (by the
    /// successor slice) in emitted evidence.
    pub fn slug(self) -> &'static str {
        match self {
            RegistryResolutionStage::IndexSelection => "index_selection",
            RegistryResolutionStage::VersionRange => "version_range",
            RegistryResolutionStage::Lifecycle => "lifecycle",
            RegistryResolutionStage::ContractRetrieval => "contract_retrieval",
            RegistryResolutionStage::ContractDigest => "contract_digest",
            RegistryResolutionStage::ArtifactRetrieval => "artifact_retrieval",
            RegistryResolutionStage::ArtifactDigest => "artifact_digest",
            RegistryResolutionStage::Signature => "signature",
            RegistryResolutionStage::Abi => "abi",
            RegistryResolutionStage::Target => "target",
            RegistryResolutionStage::CachePersistence => "cache_persistence",
        }
    }

    /// Proposed stable error code for this boundary (issue #1273 deliverable).
    pub fn proposed_code(self) -> &'static str {
        match self {
            RegistryResolutionStage::IndexSelection => "registry_index_selection_failed",
            RegistryResolutionStage::VersionRange => "registry_version_range_unsatisfied",
            RegistryResolutionStage::Lifecycle => "registry_lifecycle_rejected",
            RegistryResolutionStage::ContractRetrieval => "registry_contract_unreachable",
            RegistryResolutionStage::ContractDigest => "registry_contract_digest_mismatch",
            RegistryResolutionStage::ArtifactRetrieval => "registry_artifact_unreachable",
            RegistryResolutionStage::ArtifactDigest => "registry_artifact_digest_mismatch",
            RegistryResolutionStage::Signature => "registry_signature_unverified",
            RegistryResolutionStage::Abi => "registry_abi_incompatible",
            RegistryResolutionStage::Target => "registry_target_incompatible",
            RegistryResolutionStage::CachePersistence => "registry_cache_commit_failed",
        }
    }

    /// Fixed, path/URL/credential-free one-line summary for this boundary.
    pub fn summary(self) -> &'static str {
        match self {
            RegistryResolutionStage::IndexSelection => {
                "synced registry state is missing, unreadable, or has no record for this capability"
            }
            RegistryResolutionStage::VersionRange => {
                "no synced version satisfies the requested exact range; resolver does not fall back"
            }
            RegistryResolutionStage::Lifecycle => {
                "the selected version is not in an activatable lifecycle state"
            }
            RegistryResolutionStage::ContractRetrieval => {
                "the capability contract could not be retrieved"
            }
            RegistryResolutionStage::ContractDigest => {
                "retrieved contract bytes do not match the expected contract digest"
            }
            RegistryResolutionStage::ArtifactRetrieval => {
                "the capability artifact could not be retrieved"
            }
            RegistryResolutionStage::ArtifactDigest => {
                "retrieved artifact bytes do not match the expected artifact digest"
            }
            RegistryResolutionStage::Signature => {
                "the signed trust bundle for the artifact did not verify"
            }
            RegistryResolutionStage::Abi => {
                "the artifact Host ABI major version is not supported by this build"
            }
            RegistryResolutionStage::Target => {
                "the selected version does not permit the requested placement target"
            }
            RegistryResolutionStage::CachePersistence => {
                "verified bytes could not be committed to the immutable digest-keyed cache"
            }
        }
    }
}

/// Non-secret identity facts about the `registry_ref` under diagnosis. Only
/// these fields (plus the stage slug/code/summary) are permitted in redacted
/// evidence per spec `1258-offline-cache-activation` FR-003.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryReferenceFacts {
    pub namespace: String,
    pub id: String,
    pub requested_range: String,
    pub selected_version: Option<String>,
    pub contract_digest: Option<String>,
    pub artifact_digest: Option<String>,
    pub target: Option<String>,
}

impl RegistryReferenceFacts {
    /// Returns `Err` with the offending field name if any fact carries a
    /// filesystem path, URL, endpoint, or credential-shaped token.
    pub fn assert_redacted(&self) -> Result<(), String> {
        let fields = [
            ("namespace", Some(self.namespace.as_str())),
            ("id", Some(self.id.as_str())),
            ("requested_range", Some(self.requested_range.as_str())),
            ("selected_version", self.selected_version.as_deref()),
            ("contract_digest", self.contract_digest.as_deref()),
            ("artifact_digest", self.artifact_digest.as_deref()),
            ("target", self.target.as_deref()),
        ];
        for (name, value) in fields {
            let Some(value) = value else { continue };
            let lower = value.to_ascii_lowercase();
            let leaks = value.contains('/')
                || value.contains('\\')
                || lower.contains("://")
                || lower.contains("authorization")
                || lower.contains("bearer ")
                || lower.contains("secret")
                || lower.contains("password")
                || lower.contains("token=");
            if leaks {
                return Err(name.to_string());
            }
        }
        Ok(())
    }
}

/// Outcome observed at one boundary during a resolution attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundaryOutcome {
    /// The boundary was evaluated and passed.
    Passed,
    /// The boundary was not reached (an earlier boundary stopped resolution) or
    /// is not evaluated by the current resolver.
    NotEvaluated,
    /// The boundary was evaluated and failed.
    Failed,
}

/// Per-boundary outcomes for one resolution attempt, plus the reference facts.
#[derive(Debug, Clone)]
pub struct RegistryResolutionObservation {
    pub reference: RegistryReferenceFacts,
    outcomes: [(RegistryResolutionStage, BoundaryOutcome); 11],
}

impl RegistryResolutionObservation {
    /// A fully-passing observation for `reference`; mutate individual boundaries
    /// with [`Self::with`].
    pub fn all_passed(reference: RegistryReferenceFacts) -> Self {
        Self {
            reference,
            outcomes: RegistryResolutionStage::ALL.map(|stage| (stage, BoundaryOutcome::Passed)),
        }
    }

    /// Sets one boundary's outcome.
    #[must_use]
    pub fn with(mut self, stage: RegistryResolutionStage, outcome: BoundaryOutcome) -> Self {
        for entry in &mut self.outcomes {
            if entry.0 == stage {
                entry.1 = outcome;
            }
        }
        self
    }
}

/// A stable, redacted diagnostic for the first failing boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryResolutionDiagnostic {
    pub stage: RegistryResolutionStage,
    pub code: &'static str,
    pub summary: &'static str,
    pub reference: RegistryReferenceFacts,
}

impl RegistryResolutionDiagnostic {
    /// Redacted evidence object: only the fields permitted by spec
    /// `1258-offline-cache-activation` FR-003.
    pub fn redacted_evidence(&self) -> serde_json::Value {
        let mut object = serde_json::Map::new();
        object.insert("stage".to_string(), self.stage.slug().into());
        object.insert("code".to_string(), self.code.into());
        object.insert("summary".to_string(), self.summary.into());
        object.insert(
            "namespace".to_string(),
            self.reference.namespace.clone().into(),
        );
        object.insert("id".to_string(), self.reference.id.clone().into());
        object.insert(
            "requested_range".to_string(),
            self.reference.requested_range.clone().into(),
        );
        if let Some(version) = &self.reference.selected_version {
            object.insert("selected_version".to_string(), version.clone().into());
        }
        if let Some(digest) = &self.reference.contract_digest {
            object.insert("contract_digest".to_string(), digest.clone().into());
        }
        if let Some(digest) = &self.reference.artifact_digest {
            object.insert("artifact_digest".to_string(), digest.clone().into());
        }
        if let Some(target) = &self.reference.target {
            object.insert("target".to_string(), target.clone().into());
        }
        serde_json::Value::Object(object)
    }
}

/// Returns a diagnostic for the first failing boundary in evaluation order, or
/// `None` when every boundary passed. Deterministic: the boundary ordering is
/// fixed, so the same observation always yields the same diagnostic (no silent
/// ambiguity, per the constitution's resolution-behavior rule).
pub fn classify(
    observation: &RegistryResolutionObservation,
) -> Option<RegistryResolutionDiagnostic> {
    observation
        .outcomes
        .iter()
        .find(|(_, outcome)| *outcome == BoundaryOutcome::Failed)
        .map(|(stage, _)| RegistryResolutionDiagnostic {
            stage: *stage,
            code: stage.proposed_code(),
            summary: stage.summary(),
            reference: observation.reference.clone(),
        })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::{
        BoundaryOutcome, RegistryReferenceFacts, RegistryResolutionObservation,
        RegistryResolutionStage, classify,
    };
    use std::collections::BTreeSet;

    fn facts() -> RegistryReferenceFacts {
        RegistryReferenceFacts {
            namespace: "diagnostic".to_string(),
            id: "diagnostic.evidence-normalize".to_string(),
            requested_range: "==1.0.1".to_string(),
            selected_version: Some("1.0.1".to_string()),
            contract_digest: Some("sha256:".to_string() + &"a".repeat(64)),
            artifact_digest: Some("sha256:".to_string() + &"b".repeat(64)),
            target: Some("local".to_string()),
        }
    }

    #[test]
    fn every_boundary_has_a_distinct_stable_code() {
        let mut codes = BTreeSet::new();
        let mut slugs = BTreeSet::new();
        for stage in RegistryResolutionStage::ALL {
            assert!(
                codes.insert(stage.proposed_code()),
                "duplicate proposed_code for {stage:?}"
            );
            assert!(slugs.insert(stage.slug()), "duplicate slug for {stage:?}");
            assert!(
                stage.proposed_code().starts_with("registry_"),
                "{stage:?} code is not registry_-prefixed"
            );
            assert!(
                stage
                    .proposed_code()
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b == b'_'),
                "{stage:?} code is not snake_case"
            );
        }
        assert_eq!(codes.len(), RegistryResolutionStage::ALL.len());
    }

    #[test]
    fn classify_reports_first_failing_boundary_in_evaluation_order() {
        // Lifecycle (order 3) and Target (order 10) both fail; the earlier one wins.
        let observation = RegistryResolutionObservation::all_passed(facts())
            .with(RegistryResolutionStage::Lifecycle, BoundaryOutcome::Failed)
            .with(RegistryResolutionStage::Target, BoundaryOutcome::Failed);
        let diagnostic = classify(&observation).expect("a failing boundary yields a diagnostic");
        assert_eq!(diagnostic.stage, RegistryResolutionStage::Lifecycle);
        assert_eq!(diagnostic.code, "registry_lifecycle_rejected");
    }

    #[test]
    fn classify_is_none_when_every_boundary_passes() {
        let observation = RegistryResolutionObservation::all_passed(facts());
        assert!(classify(&observation).is_none());
    }

    #[test]
    fn classify_skips_not_evaluated_boundaries() {
        // A boundary the resolver never reached (or never checks) must not be
        // reported as the failure; only an actual `Failed` boundary is.
        let observation = RegistryResolutionObservation::all_passed(facts())
            .with(
                RegistryResolutionStage::Signature,
                BoundaryOutcome::NotEvaluated,
            )
            .with(RegistryResolutionStage::Abi, BoundaryOutcome::NotEvaluated)
            .with(RegistryResolutionStage::Target, BoundaryOutcome::Failed);
        let diagnostic = classify(&observation).expect("the failed boundary yields a diagnostic");
        assert_eq!(diagnostic.stage, RegistryResolutionStage::Target);
    }

    #[test]
    fn each_boundary_maps_to_its_own_diagnostic() {
        for stage in RegistryResolutionStage::ALL {
            let observation = RegistryResolutionObservation::all_passed(facts())
                .with(stage, BoundaryOutcome::Failed);
            let diagnostic = classify(&observation).expect("failing boundary yields diagnostic");
            assert_eq!(diagnostic.stage, stage);
            assert_eq!(diagnostic.code, stage.proposed_code());
        }
    }

    #[test]
    fn redacted_evidence_carries_only_permitted_fields() {
        let allowed: BTreeSet<&str> = [
            "stage",
            "code",
            "summary",
            "namespace",
            "id",
            "requested_range",
            "selected_version",
            "contract_digest",
            "artifact_digest",
            "target",
        ]
        .into_iter()
        .collect();
        for stage in RegistryResolutionStage::ALL {
            let observation = RegistryResolutionObservation::all_passed(facts())
                .with(stage, BoundaryOutcome::Failed);
            let diagnostic = classify(&observation).expect("failing boundary yields diagnostic");
            let evidence = diagnostic.redacted_evidence();
            let object = evidence.as_object().expect("evidence is a JSON object");
            for key in object.keys() {
                assert!(
                    allowed.contains(key.as_str()),
                    "unexpected evidence field {key}"
                );
            }
            let serialized = serde_json::to_string(&evidence).expect("evidence serializes");
            assert!(
                !serialized.contains('/'),
                "evidence leaked a path/URL: {serialized}"
            );
            assert!(
                !serialized.to_ascii_lowercase().contains("authorization"),
                "evidence leaked a header: {serialized}"
            );
        }
    }

    #[test]
    fn assert_redacted_rejects_paths_and_credentials() {
        let mut leaky = facts();
        leaky.id = "/home/ci/.traverse/cache/sha256/abcd".to_string();
        assert_eq!(leaky.assert_redacted(), Err("id".to_string()));

        let mut url = facts();
        url.target = Some("https://registry.example/download?token=abc".to_string());
        assert_eq!(url.assert_redacted(), Err("target".to_string()));

        assert!(facts().assert_redacted().is_ok());
    }

    #[test]
    fn proposed_map_fixture_matches_the_enum() {
        let raw = include_str!("../tests/fixtures/registry-resolution/boundary-error-map.json");
        let map: serde_json::Value = serde_json::from_str(raw).expect("fixture map parses");
        assert_eq!(map["governing_spec"], "1258-offline-cache-activation");
        assert_eq!(map["diagnosis_issue"], 1273);
        assert_eq!(map["implementation_issue"], 1272);

        let boundaries = map["boundaries"]
            .as_array()
            .expect("boundaries is an array");
        assert_eq!(
            boundaries.len(),
            RegistryResolutionStage::ALL.len(),
            "fixture boundary count must match the enum"
        );
        for (index, stage) in RegistryResolutionStage::ALL.into_iter().enumerate() {
            let entry = &boundaries[index];
            assert_eq!(entry["evaluation_order"], (index as u64) + 1);
            assert_eq!(
                entry["stage"],
                stage.slug(),
                "stage slug mismatch at {index}"
            );
            assert_eq!(
                entry["proposed_code"],
                stage.proposed_code(),
                "proposed_code mismatch for {stage:?}"
            );
        }

        let allowed = map["allowed_redacted_evidence_fields"]
            .as_array()
            .expect("allowed field list is an array");
        for field in [
            "namespace",
            "id",
            "requested_range",
            "stage",
            "code",
            "summary",
        ] {
            assert!(
                allowed.iter().any(|value| value == field),
                "fixture allowed-field list is missing {field}"
            );
        }
    }

    /// Shared cross-host projection matrix (issue #1300). The Web host asserts
    /// the *same* fixture in
    /// `packages/web/TraverseEmbedder/tests/registryCacheCrossHostProjection.test.mjs`,
    /// so an inequality on either side is a cross-host conformance failure.
    const CROSS_HOST_PROJECTION_MATRIX: &str = include_str!(
        "../../../fixtures/cross-host/registry-cache-projections/projection-matrix.json"
    );

    const CROSS_HOST_CATEGORIES: [&str; 6] = [
        "missing",
        "altered",
        "lifecycle-rejected",
        "signature-invalid",
        "abi-incompatible",
        "target-incompatible",
    ];

    fn projection_matrix() -> serde_json::Value {
        serde_json::from_str(CROSS_HOST_PROJECTION_MATRIX).expect("projection matrix parses")
    }

    fn allowed_evidence_fields(matrix: &serde_json::Value) -> BTreeSet<String> {
        matrix["allowed_evidence_fields"]
            .as_array()
            .expect("allowed_evidence_fields is an array")
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .expect("allowed field is a string")
                    .to_string()
            })
            .collect()
    }

    #[test]
    fn cross_host_matrix_declares_the_spec_1258_evidence_contract() {
        let matrix = projection_matrix();
        assert_eq!(matrix["governing_spec"], "1258-offline-cache-activation");
        assert_eq!(
            matrix["conformance_matrix_spec"],
            "107-cross-host-embedded-registry-cache"
        );
        assert_eq!(matrix["implementation_issue"], 1272);
        assert_eq!(matrix["evidence_issue"], 1300);

        let listed: Vec<&str> = matrix["categories"]
            .as_array()
            .expect("categories is an array")
            .iter()
            .map(|entry| entry["category"].as_str().expect("category is a string"))
            .collect();
        assert_eq!(
            listed, CROSS_HOST_CATEGORIES,
            "the matrix must list every spec 1258 FR-004 failure category once, in order"
        );

        // The spec 107 FR-009 native equivalence set is recorded, not widened.
        let native = &matrix["native_matrix"];
        let equivalence_set: BTreeSet<&str> = native["spec_107_fr_009_equivalence_set"]
            .as_array()
            .expect("equivalence set is an array")
            .iter()
            .map(|value| value.as_str().expect("equivalence entry is a string"))
            .collect();
        assert_eq!(
            equivalence_set,
            BTreeSet::from([
                "preparation_success",
                "missing_cache",
                "yanked_dependency",
                "artifact_digest_mismatch",
            ]),
            "spec 107 FR-009 fixes the native equivalence set"
        );
        for category in ["missing", "altered", "lifecycle-rejected"] {
            assert!(
                native["coverage"][category]["native_code"].is_string(),
                "{category} must record an observable native code"
            );
        }
        for category in [
            "signature-invalid",
            "abi-incompatible",
            "target-incompatible",
        ] {
            assert_eq!(
                native["coverage"][category]["status"], "rust_web_projection_only",
                "{category} is outside the spec 107 FR-009 native set"
            );
        }
    }

    #[test]
    fn cross_host_matrix_projections_are_redacted_and_key_sorted() {
        let matrix = projection_matrix();
        let allowed = allowed_evidence_fields(&matrix);

        for entry in matrix["categories"]
            .as_array()
            .expect("categories is an array")
        {
            let category = entry["category"].as_str().expect("category is a string");
            let code = entry["code"].as_str().expect("code is a string");
            let stage_slug = entry["stage"].as_str().expect("stage is a string");
            let evidence = entry["redacted_evidence"]
                .as_object()
                .expect("redacted_evidence is an object");

            assert_eq!(
                evidence.get("code").and_then(serde_json::Value::as_str),
                Some(code),
                "{category}: redacted_evidence.code must equal the category code"
            );
            assert_eq!(
                evidence.get("stage").and_then(serde_json::Value::as_str),
                Some(stage_slug),
                "{category}: redacted_evidence.stage must equal the category stage"
            );

            let keys: Vec<&str> = evidence.keys().map(String::as_str).collect();
            for key in &keys {
                assert!(
                    allowed.contains(*key),
                    "{category}: evidence field {key} is not in allowed_evidence_fields"
                );
            }
            let mut sorted_keys = keys.clone();
            sorted_keys.sort_unstable();
            assert_eq!(
                keys, sorted_keys,
                "{category}: redacted_evidence keys must be sorted for cross-host comparison"
            );

            let lower = serde_json::to_string(&entry["redacted_evidence"])
                .expect("redacted_evidence serializes")
                .to_ascii_lowercase();
            for probe in [
                "/",
                "\\",
                "://",
                "authorization",
                "bearer ",
                "secret",
                "token=",
            ] {
                assert!(
                    !lower.contains(probe),
                    "{category}: canonical projection leaked {probe:?}: {lower}"
                );
            }
        }
    }

    #[test]
    fn cross_host_matrix_matches_the_rust_resolution_projection() {
        let matrix = projection_matrix();
        let allowed = allowed_evidence_fields(&matrix);

        for entry in matrix["categories"]
            .as_array()
            .expect("categories is an array")
        {
            let category = entry["category"].as_str().expect("category is a string");
            let code = entry["code"].as_str().expect("code is a string");
            let stage_slug = entry["stage"].as_str().expect("stage is a string");
            let evidence = &entry["redacted_evidence"];

            // Categories 2..6 are resolution boundaries with a Rust projection in
            // this module; `missing` is the offline-activation outcome and has no
            // resolution stage.
            let Some(stage) = RegistryResolutionStage::ALL
                .into_iter()
                .find(|stage| stage.slug() == stage_slug)
            else {
                assert_eq!(
                    category, "missing",
                    "only the missing category has no resolution stage"
                );
                assert_eq!(
                    code, "registry_cache_entry_missing",
                    "missing must project the stable offline-activation code"
                );
                continue;
            };

            let facts = RegistryReferenceFacts {
                namespace: evidence["namespace"]
                    .as_str()
                    .expect("namespace present")
                    .to_string(),
                id: evidence["id"].as_str().expect("id present").to_string(),
                requested_range: evidence["requested_range"]
                    .as_str()
                    .expect("requested_range present")
                    .to_string(),
                selected_version: evidence["selected_version"].as_str().map(str::to_string),
                contract_digest: None,
                artifact_digest: evidence["artifact_digest"].as_str().map(str::to_string),
                target: evidence["target"].as_str().map(str::to_string),
            };
            let observation = RegistryResolutionObservation::all_passed(facts)
                .with(stage, BoundaryOutcome::Failed);
            let diagnostic = classify(&observation).expect("a failed boundary yields a diagnostic");

            assert_eq!(
                diagnostic.code, code,
                "{category}: Rust projection code must match the matrix"
            );
            assert_eq!(
                diagnostic.stage.slug(),
                stage_slug,
                "{category}: Rust projection stage must match the matrix"
            );
            assert_eq!(
                diagnostic.summary,
                evidence["summary"].as_str().expect("summary present"),
                "{category}: Rust projection summary must match the matrix"
            );
            for key in diagnostic
                .redacted_evidence()
                .as_object()
                .expect("projection is an object")
                .keys()
            {
                assert!(
                    allowed.contains(key.as_str()),
                    "{category}: Rust projection field {key} is not permitted"
                );
            }
        }
    }
}
