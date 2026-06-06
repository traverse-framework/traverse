//! Semver range resolution for the capability registry (spec 037).

use semver::{Version, VersionReq};

use crate::{CapabilityRegistry, LookupScope, RegistryScope};

/// A fully resolved capability registration returned by range resolution.
///
/// This is a lightweight view — it carries the registry record and the
/// `CapabilityRegistration` snapshot needed by callers.  When only the
/// resolved version string is needed, inspect `record.version`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRangeCapability {
    /// The `id` of the capability.
    pub capability_id: String,
    /// The resolved semver version string.
    pub version: String,
    /// The artifact reference of the resolved registration.
    pub artifact_ref: String,
    /// The registry scope in which the capability was found.
    pub scope: RegistryScope,
}

/// Errors that can be returned by [`resolve_version_range`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RangeResolutionError {
    /// `capability_id` was not present in the registry at all.
    CapabilityNotFound { id: String },
    /// The capability exists but no registered version satisfies `range`.
    NoVersionSatisfies { range: String },
    /// Multiple registrations at the highest satisfying version with different
    /// digests — the resolver cannot pick one deterministically.
    AmbiguousMatch { candidates: Vec<AmbiguousCandidate> },
    /// `range_str` could not be parsed as a valid semver range expression.
    InvalidRangeSyntax { range: String, reason: String },
}

/// One entry in an [`RangeResolutionError::AmbiguousMatch`] set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AmbiguousCandidate {
    pub capability_id: String,
    pub version: String,
    pub artifact_ref: String,
    pub scope: RegistryScope,
}

/// Resolves the highest version of `capability_id` registered in
/// `workspace_id` (lookup scope) that satisfies `range_str`.
///
/// Resolution order:
/// 1. Parse `range_str` — [`RangeResolutionError::InvalidRangeSyntax`] on failure.
/// 2. Collect all registered versions of `capability_id` in `lookup_scope`.
/// 3. Filter to those satisfying the range.
/// 4. Select the highest satisfying version.
/// 5. If multiple registrations at that version have different digests →
///    [`RangeResolutionError::AmbiguousMatch`].
/// 6. If no versions satisfy → [`RangeResolutionError::NoVersionSatisfies`].
/// 7. If `capability_id` is absent entirely → [`RangeResolutionError::CapabilityNotFound`].
///
/// # Errors
///
/// Returns [`RangeResolutionError`] for any of the cases described above.
pub fn resolve_version_range(
    registry: &CapabilityRegistry,
    capability_id: &str,
    range_str: &str,
    lookup_scope: LookupScope,
) -> Result<ResolvedRangeCapability, RangeResolutionError> {
    let req =
        VersionReq::parse(range_str).map_err(|err| RangeResolutionError::InvalidRangeSyntax {
            range: range_str.to_string(),
            reason: err.to_string(),
        })?;

    // Collect all versions of this capability across the lookup scope.
    // We probe each applicable scope independently so that cross-scope
    // ambiguity (same version, different artifact_ref in Public vs. Private)
    // is visible before `discover`'s shadowing collapses them.
    let mut all_records: Vec<(RegistryScope, String, String)> = Vec::new(); // (scope, version, artifact_ref)
    let mut found_any = false;

    let probe_scopes: &[RegistryScope] = match lookup_scope {
        LookupScope::PublicOnly => &[RegistryScope::Public],
        LookupScope::PreferPrivate => &[RegistryScope::Private, RegistryScope::Public],
    };

    for &scope in probe_scopes {
        let scope_lookup = match scope {
            RegistryScope::Public => LookupScope::PublicOnly,
            RegistryScope::Private => LookupScope::PreferPrivate,
        };
        let entries = registry.discover(scope_lookup, &crate::DiscoveryQuery::default());
        for entry in &entries {
            if entry.id != capability_id || entry.scope != scope {
                continue;
            }
            found_any = true;
            if let Some(resolved) = registry.find_exact(scope_lookup, &entry.id, &entry.version) {
                all_records.push((
                    scope,
                    resolved.record.version.clone(),
                    resolved.record.artifact_ref.clone(),
                ));
            }
        }
    }

    if !found_any && all_records.is_empty() {
        return Err(RangeResolutionError::CapabilityNotFound {
            id: capability_id.to_string(),
        });
    }

    // Filter to versions that satisfy the range.
    let mut satisfying: Vec<(RegistryScope, Version, String)> = all_records
        .into_iter()
        .filter_map(|(scope, version_str, artifact_ref)| {
            let version = Version::parse(&version_str).ok()?;
            if req.matches(&version) {
                Some((scope, version, artifact_ref))
            } else {
                None
            }
        })
        .collect();

    if satisfying.is_empty() {
        return Err(RangeResolutionError::NoVersionSatisfies {
            range: range_str.to_string(),
        });
    }

    // Find the highest satisfying version.
    satisfying.sort_by(|(_, va, _), (_, vb, _)| va.cmp(vb));
    // satisfying is non-empty (guarded above); .last() and .next() are safe.
    let highest = satisfying
        .last()
        .ok_or(RangeResolutionError::NoVersionSatisfies {
            range: range_str.to_string(),
        })
        .map(|(_, v, _)| v.clone())?;

    // Collect all registrations at the highest version.
    let at_highest: Vec<(RegistryScope, Version, String)> = satisfying
        .into_iter()
        .filter(|(_, v, _)| *v == highest)
        .collect();

    // Check for ambiguity: same version, different artifact digests.
    let unique_refs: std::collections::BTreeSet<&str> =
        at_highest.iter().map(|(_, _, r)| r.as_str()).collect();

    if unique_refs.len() > 1 {
        let candidates = at_highest
            .iter()
            .map(|(scope, version, artifact_ref)| AmbiguousCandidate {
                capability_id: capability_id.to_string(),
                version: version.to_string(),
                artifact_ref: artifact_ref.clone(),
                scope: *scope,
            })
            .collect();
        return Err(RangeResolutionError::AmbiguousMatch { candidates });
    }

    // at_highest is non-empty because satisfying is non-empty and we filter to
    // the highest version which is present by construction.
    let (scope, version, artifact_ref) =
        at_highest
            .into_iter()
            .next()
            .ok_or(RangeResolutionError::NoVersionSatisfies {
                range: range_str.to_string(),
            })?;

    Ok(ResolvedRangeCapability {
        capability_id: capability_id.to_string(),
        version: version.to_string(),
        artifact_ref,
        scope,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;
    use crate::{
        ArtifactDigests, BinaryFormat, BinaryReference, CapabilityArtifactRecord,
        CapabilityRegistration, ComposabilityMetadata, CompositionKind, CompositionPattern,
        ImplementationKind, RegistryProvenance, RegistryScope, SourceKind, SourceReference,
    };
    use serde_json::json;
    use traverse_contracts::{
        BinaryFormat as ContractBinaryFormat, Condition, Entrypoint, EntrypointKind,
        EvidenceStatus, EvidenceType, Execution, ExecutionConstraints, ExecutionTarget,
        FilesystemAccess, HostApiAccess, Lifecycle, NetworkAccess, Owner, Provenance,
        ProvenanceSource, SchemaContainer, ServiceType, SideEffect, SideEffectKind,
        ValidationEvidence,
    };

    fn base_contract(id: &str, version: &str) -> traverse_contracts::CapabilityContract {
        let (namespace, name) = split_id(id);
        traverse_contracts::CapabilityContract {
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
            summary: "Test capability.".to_string(),
            description: "Test capability for range resolution.".to_string(),
            inputs: SchemaContainer {
                schema: json!({"type": "object"}),
            },
            outputs: SchemaContainer {
                schema: json!({"type": "object"}),
            },
            preconditions: vec![Condition {
                id: "request-authenticated".to_string(),
                description: "Caller identity has been established.".to_string(),
            }],
            postconditions: vec![Condition {
                id: "draft-created".to_string(),
                description: "A draft payload is produced.".to_string(),
            }],
            side_effects: vec![SideEffect {
                kind: SideEffectKind::MemoryOnly,
                description: "In-memory state only.".to_string(),
            }],
            emits: vec![],
            consumes: vec![],
            permissions: vec![traverse_contracts::IdReference {
                id: "test.read".to_string(),
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
            policies: vec![],
            dependencies: vec![],
            provenance: Provenance {
                source: ProvenanceSource::Greenfield,
                author: "test".to_string(),
                created_at: "2026-04-20T00:00:00Z".to_string(),
                spec_ref: Some("037-semver-range-resolution".to_string()),
                adr_refs: vec![],
                exception_refs: vec![],
            },
            evidence: vec![ValidationEvidence {
                evidence_id: "validation:contract".to_string(),
                evidence_type: EvidenceType::ContractValidation,
                status: EvidenceStatus::Passed,
            }],
            service_type: ServiceType::Stateless,
            permitted_targets: vec![ExecutionTarget::Local],
            event_trigger: None,
            connector_requirements: Vec::new(),
            state_schema: None,
        }
    }

    fn executable_artifact(
        contract: &traverse_contracts::CapabilityContract,
        digest_suffix: &str,
    ) -> CapabilityArtifactRecord {
        CapabilityArtifactRecord {
            artifact_ref: format!(
                "artifact:{}:{}:{}",
                contract.name, contract.version, digest_suffix
            ),
            implementation_kind: ImplementationKind::Executable,
            source: SourceReference {
                kind: SourceKind::Git,
                location: format!("https://github.com/test/{}", contract.name),
            },
            binary: Some(BinaryReference {
                format: BinaryFormat::Wasm,
                location: format!("artifacts/{}/{}.wasm", contract.name, contract.version),
            }),
            workflow_ref: None,
            digests: ArtifactDigests {
                source_digest: format!(
                    "sha256:source-{}-{}-{}",
                    contract.name, contract.version, digest_suffix
                ),
                binary_digest: Some(format!(
                    "sha256:binary-{}-{}-{}",
                    contract.name, contract.version, digest_suffix
                )),
            },
            provenance: RegistryProvenance {
                source: "greenfield".to_string(),
                author: "test".to_string(),
                created_at: "2026-04-20T00:00:00Z".to_string(),
            },
        }
    }

    fn registration(
        scope: RegistryScope,
        contract: traverse_contracts::CapabilityContract,
        digest_suffix: &str,
    ) -> CapabilityRegistration {
        CapabilityRegistration {
            scope,
            contract_path: format!(
                "registry/{}/{}/{}",
                scope_label(scope),
                contract.id,
                contract.version
            ) + "/contract.json",
            artifact: executable_artifact(&contract, digest_suffix),
            registered_at: "2026-04-20T00:00:00Z".to_string(),
            tags: vec!["test".to_string()],
            composability: ComposabilityMetadata {
                kind: CompositionKind::Atomic,
                patterns: vec![CompositionPattern::Sequential],
                provides: vec!["test-output".to_string()],
                requires: vec!["test-input".to_string()],
            },
            governing_spec: "037-semver-range-resolution".to_string(),
            validator_version: "test".to_string(),
            contract,
        }
    }

    fn scope_label(scope: RegistryScope) -> &'static str {
        match scope {
            RegistryScope::Public => "public",
            RegistryScope::Private => "private",
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

    fn registry_with_versions(id: &str, versions: &[&str]) -> CapabilityRegistry {
        let mut registry = CapabilityRegistry::new();
        for version in versions {
            registry
                .register(registration(
                    RegistryScope::Public,
                    base_contract(id, version),
                    "default",
                ))
                .expect("registration should succeed");
        }
        registry
    }

    const CAP_ID: &str = "test.range.resolver";

    // Scenario 1: ^1.0.0 selects highest satisfying version
    #[test]
    fn resolves_highest_satisfying_version() {
        let registry = registry_with_versions(CAP_ID, &["1.0.0", "1.1.0", "1.2.0"]);
        let result = resolve_version_range(&registry, CAP_ID, "^1.0.0", LookupScope::PublicOnly)
            .expect("^1.0.0 should resolve");
        assert_eq!(result.version, "1.2.0");
        assert_eq!(result.capability_id, CAP_ID);
    }

    // Scenario 2: ^1.0.0 with only 2.0.0 → NoVersionSatisfies
    #[test]
    fn no_version_satisfies_range() {
        let registry = registry_with_versions(CAP_ID, &["2.0.0"]);
        let err = resolve_version_range(&registry, CAP_ID, "^1.0.0", LookupScope::PublicOnly)
            .expect_err("should fail with NoVersionSatisfies");
        assert!(
            matches!(err, RangeResolutionError::NoVersionSatisfies { range } if range == "^1.0.0")
        );
    }

    // Scenario 3: two 1.2.0 registrations with different digests → AmbiguousMatch
    #[test]
    fn ambiguous_match_on_different_digests() {
        let mut registry = CapabilityRegistry::new();
        // Register 1.2.0 in Public scope
        registry
            .register(registration(
                RegistryScope::Public,
                base_contract(CAP_ID, "1.2.0"),
                "digest-a",
            ))
            .expect("public registration should succeed");
        // Register 1.2.0 in Private scope (different digest, different artifact_ref)
        registry
            .register(registration(
                RegistryScope::Private,
                base_contract(CAP_ID, "1.2.0"),
                "digest-b",
            ))
            .expect("private registration should succeed");

        let err = resolve_version_range(&registry, CAP_ID, "^1.0.0", LookupScope::PreferPrivate)
            .expect_err("should fail with AmbiguousMatch");
        assert!(matches!(
            err,
            RangeResolutionError::AmbiguousMatch { candidates } if candidates.len() == 2
        ));
    }

    // Scenario 4: exact version string resolves via exact lookup in runtime — tested
    // at the registry level by confirming `^1.2.3` (caret is not exact) and checking
    // the resolver selects 1.2.3 when only that version is registered.
    #[test]
    fn exact_version_string_resolves() {
        let registry = registry_with_versions(CAP_ID, &["1.2.3"]);
        let result = resolve_version_range(&registry, CAP_ID, "1.2.3", LookupScope::PublicOnly)
            .expect("exact version string should resolve");
        assert_eq!(result.version, "1.2.3");
    }

    // Scenario 5: * selects highest overall
    #[test]
    fn wildcard_selects_highest_version() {
        let registry = registry_with_versions(CAP_ID, &["1.0.0", "1.1.0", "2.3.0", "0.9.0"]);
        let result = resolve_version_range(&registry, CAP_ID, "*", LookupScope::PublicOnly)
            .expect("* should resolve to highest");
        assert_eq!(result.version, "2.3.0");
    }

    // Scenario 6: malformed range → InvalidRangeSyntax
    #[test]
    fn malformed_range_returns_invalid_syntax_error() {
        let registry = registry_with_versions(CAP_ID, &["1.0.0"]);
        let err = resolve_version_range(&registry, CAP_ID, ">>1.0", LookupScope::PublicOnly)
            .expect_err("malformed range should fail");
        assert!(
            matches!(err, RangeResolutionError::InvalidRangeSyntax { range, .. } if range == ">>1.0")
        );
    }

    // Scenario 7: unknown capability → CapabilityNotFound
    #[test]
    fn unknown_capability_id_returns_not_found() {
        let registry = CapabilityRegistry::new();
        let err = resolve_version_range(
            &registry,
            "test.range.nonexistent",
            "^1.0.0",
            LookupScope::PublicOnly,
        )
        .expect_err("unknown capability should fail");
        assert!(
            matches!(err, RangeResolutionError::CapabilityNotFound { id } if id == "test.range.nonexistent")
        );
    }

    // Scenario 8: PreferPrivate probe with a Public-only registration — the Private probe
    // leg's inner loop hits `continue` (line 96) because entry.scope (Public) != scope (Private).
    #[test]
    fn prefer_private_resolves_public_only_capability() {
        let registry = registry_with_versions(CAP_ID, &["1.3.0", "1.4.0"]);
        let result = resolve_version_range(&registry, CAP_ID, "^1.0.0", LookupScope::PreferPrivate)
            .expect("should fall back to Public and resolve 1.4.0");
        assert_eq!(result.version, "1.4.0");
        assert_eq!(result.scope, RegistryScope::Public);
    }

    // Scenario 9: AmbiguousMatch candidates list is inspectable.
    #[test]
    fn ambiguous_match_exposes_candidate_details() {
        let mut registry = CapabilityRegistry::new();
        registry
            .register(registration(
                RegistryScope::Public,
                base_contract(CAP_ID, "2.0.0"),
                "alpha",
            ))
            .expect("public registration should succeed");
        registry
            .register(registration(
                RegistryScope::Private,
                base_contract(CAP_ID, "2.0.0"),
                "beta",
            ))
            .expect("private registration should succeed");
        let err = resolve_version_range(&registry, CAP_ID, "^2.0.0", LookupScope::PreferPrivate)
            .expect_err("should return AmbiguousMatch");
        assert!(
            matches!(err, RangeResolutionError::AmbiguousMatch { ref candidates } if candidates.len() == 2
                && candidates.iter().all(|c| c.capability_id == CAP_ID)
                && candidates.iter().all(|c| c.version == "2.0.0")),
            "expected AmbiguousMatch with 2 candidates, got {err:?}"
        );
    }
}
