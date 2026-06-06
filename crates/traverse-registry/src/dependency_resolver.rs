//! Dependency resolution for the capability registry (spec 043).
//!
//! Resolves the `Capability`-typed entries in a capability contract's
//! `dependencies` field against the workspace registry, detects cycles,
//! enforces a maximum transitive depth of 5, and returns an immutable
//! dependency lock record on success.

use crate::{CapabilityRegistry, CapabilityRegistryRecord, LookupScope, resolve_version_range};
use traverse_contracts::DependencyArtifactType;

/// Maximum transitive resolution depth (spec FR-007).
pub const MAX_TRANSITIVE_DEPTH: usize = 5;

/// A single entry in a resolved dependency lock record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedDependencyLock {
    /// The resolved capability id.
    pub capability_id: String,
    /// The resolved semver version string.
    pub version: String,
    /// The contract digest at the time of resolution.
    pub digest: String,
}

/// Errors that can be returned by [`resolve_dependencies`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolutionError {
    /// One or more declared dependencies could not be satisfied.
    MissingDependency {
        capability_id: String,
        required_version: String,
    },
    /// A directed cycle was detected in the dependency graph.
    CircularDependency { cycle: Vec<String> },
    /// Transitive depth exceeded [`MAX_TRANSITIVE_DEPTH`].
    MaxTransitiveDepthExceeded { depth: usize, chain: Vec<String> },
}

/// Resolves all `Capability`-typed dependencies declared in `dependencies`,
/// including transitive dependencies, up to [`MAX_TRANSITIVE_DEPTH`].
///
/// Returns an immutable lock record containing `(capability_id, version,
/// digest)` tuples for every directly and transitively resolved dependency.
///
/// # Errors
///
/// Returns [`ResolutionError`] when any dependency cannot be resolved,
/// a cycle is detected, a depth limit is exceeded, or a version range is
/// invalid.
pub fn resolve_dependencies(
    registry: &CapabilityRegistry,
    registering_id: &str,
    dependencies: &[traverse_contracts::DependencyReference],
    lookup_scope: LookupScope,
) -> Result<Vec<ResolvedDependencyLock>, ResolutionError> {
    let mut lock: Vec<ResolvedDependencyLock> = Vec::new();
    let mut visiting: Vec<String> = vec![registering_id.to_string()];
    let mut visited: std::collections::HashSet<String> = std::collections::HashSet::new();
    visited.insert(registering_id.to_string());

    resolve_layer(
        registry,
        dependencies,
        lookup_scope,
        &mut visiting,
        &mut visited,
        &mut lock,
        0,
    )?;

    Ok(lock)
}

fn resolve_layer(
    registry: &CapabilityRegistry,
    dependencies: &[traverse_contracts::DependencyReference],
    lookup_scope: LookupScope,
    visiting: &mut Vec<String>,
    visited: &mut std::collections::HashSet<String>,
    lock: &mut Vec<ResolvedDependencyLock>,
    depth: usize,
) -> Result<(), ResolutionError> {
    for dep in dependencies {
        if dep.artifact_type != DependencyArtifactType::Capability {
            continue;
        }

        let dep_id = &dep.id;
        let version_range = &dep.version;

        // Cycle detection: if this id is already in the current visit stack,
        // we have a cycle.
        if visiting.contains(dep_id) {
            let mut cycle = visiting.clone();
            cycle.push(dep_id.clone());
            return Err(ResolutionError::CircularDependency { cycle });
        }

        // Resolve via semver range.  Invalid range syntax is treated as a
        // missing dependency since the registry pre-validates version formats.
        let resolved = resolve_version_range(registry, dep_id, version_range, lookup_scope)
            .map_err(|_| ResolutionError::MissingDependency {
                capability_id: dep_id.clone(),
                required_version: version_range.clone(),
            })?;

        // Retrieve the full record to get the contract digest.
        let scope_lookup = match resolved.scope {
            crate::RegistryScope::Public => LookupScope::PublicOnly,
            crate::RegistryScope::Private => LookupScope::PreferPrivate,
        };
        let full = registry
            .find_exact(scope_lookup, &resolved.capability_id, &resolved.version)
            .ok_or(ResolutionError::MissingDependency {
                capability_id: dep_id.clone(),
                required_version: version_range.clone(),
            })?;

        let lock_entry = ResolvedDependencyLock {
            capability_id: resolved.capability_id.clone(),
            version: resolved.version.clone(),
            digest: full.record.contract_digest.clone(),
        };

        // Only add to lock if not already present (dedup transitive entries).
        if !lock
            .iter()
            .any(|e| e.capability_id == lock_entry.capability_id && e.version == lock_entry.version)
        {
            lock.push(lock_entry);
        }

        // Skip transitive resolution if already fully visited.
        if !visited.insert(dep_id.clone()) {
            continue;
        }

        // Recurse into transitive dependencies.
        if depth + 1 > MAX_TRANSITIVE_DEPTH {
            return Err(ResolutionError::MaxTransitiveDepthExceeded {
                depth: depth + 1,
                chain: {
                    let mut c = visiting.clone();
                    c.push(dep_id.clone());
                    c
                },
            });
        }

        visiting.push(dep_id.clone());
        resolve_layer(
            registry,
            &full.contract.dependencies,
            lookup_scope,
            visiting,
            visited,
            lock,
            depth + 1,
        )?;
        visiting.pop();
    }

    Ok(())
}

/// Verifies that each entry in the lock record still matches the currently
/// registered digest for the locked `(capability_id, version)`.
///
/// Returns the first mismatch found, or `None` if all digests are consistent.
#[must_use]
pub fn verify_lock_digests(
    registry: &CapabilityRegistry,
    lock: &[ResolvedDependencyLock],
    lookup_scope: LookupScope,
) -> Option<DigestMismatch> {
    for entry in lock {
        let scope_lookup = lookup_scope;
        let current = registry.find_exact(scope_lookup, &entry.capability_id, &entry.version)?;
        if current.record.contract_digest != entry.digest {
            return Some(DigestMismatch {
                capability_id: entry.capability_id.clone(),
                version: entry.version.clone(),
                locked_digest: entry.digest.clone(),
                current_digest: current.record.contract_digest.clone(),
            });
        }
    }
    None
}

/// Details of a digest mismatch detected at execution time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DigestMismatch {
    pub capability_id: String,
    pub version: String,
    pub locked_digest: String,
    pub current_digest: String,
}

/// Looks up the full registry record for a given lock entry.
///
/// Returns `None` when the entry is no longer present in the registry.
#[must_use]
pub fn lookup_lock_record(
    registry: &CapabilityRegistry,
    entry: &ResolvedDependencyLock,
    lookup_scope: LookupScope,
) -> Option<CapabilityRegistryRecord> {
    registry
        .find_exact(lookup_scope, &entry.capability_id, &entry.version)
        .map(|r| r.record)
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
        BinaryFormat as ContractBinaryFormat, Condition, DependencyArtifactType,
        DependencyReference, Entrypoint, EntrypointKind, EvidenceStatus, EvidenceType, Execution,
        ExecutionConstraints, ExecutionTarget, FilesystemAccess, HostApiAccess, Lifecycle,
        NetworkAccess, Owner, Provenance, ProvenanceSource, SchemaContainer, ServiceType,
        SideEffect, SideEffectKind, ValidationEvidence,
    };

    // ── helpers ────────────────────────────────────────────────────────────

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
                contact: "test@example.com".to_string(),
            },
            summary: "Test capability for dep resolution.".to_string(),
            description: "Test capability for dependency resolution testing purposes.".to_string(),
            inputs: SchemaContainer {
                schema: json!({"type": "object"}),
            },
            outputs: SchemaContainer {
                schema: json!({"type": "object"}),
            },
            preconditions: vec![Condition {
                id: "auth".to_string(),
                description: "Caller is authenticated.".to_string(),
            }],
            postconditions: vec![Condition {
                id: "done".to_string(),
                description: "Output produced.".to_string(),
            }],
            side_effects: vec![SideEffect {
                kind: SideEffectKind::MemoryOnly,
                description: "In-memory only.".to_string(),
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
                spec_ref: Some("043-module-dependency-management".to_string()),
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

    fn contract_with_deps(
        id: &str,
        version: &str,
        deps: Vec<DependencyReference>,
    ) -> traverse_contracts::CapabilityContract {
        traverse_contracts::CapabilityContract {
            dependencies: deps,
            ..base_contract(id, version)
        }
    }

    fn executable_artifact(
        contract: &traverse_contracts::CapabilityContract,
    ) -> CapabilityArtifactRecord {
        CapabilityArtifactRecord {
            artifact_ref: format!("artifact:{}:{}", contract.name, contract.version),
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
                source_digest: format!("sha256:src-{}-{}", contract.name, contract.version),
                binary_digest: Some(format!("sha256:bin-{}-{}", contract.name, contract.version)),
            },
            provenance: RegistryProvenance {
                source: "greenfield".to_string(),
                author: "test".to_string(),
                created_at: "2026-04-20T00:00:00Z".to_string(),
            },
        }
    }

    fn register(
        registry: &mut CapabilityRegistry,
        contract: traverse_contracts::CapabilityContract,
    ) {
        let artifact = executable_artifact(&contract);
        registry
            .register(CapabilityRegistration {
                scope: RegistryScope::Public,
                contract_path: format!(
                    "registry/public/{}/{}/contract.json",
                    contract.id, contract.version
                ),
                artifact,
                registered_at: "2026-04-20T00:00:00Z".to_string(),
                tags: vec!["test".to_string()],
                composability: ComposabilityMetadata {
                    kind: CompositionKind::Atomic,
                    patterns: vec![CompositionPattern::Sequential],
                    provides: vec!["output".to_string()],
                    requires: vec!["input".to_string()],
                },
                governing_spec: "043-module-dependency-management".to_string(),
                validator_version: "test".to_string(),
                contract,
            })
            .expect("registration should succeed");
    }

    fn cap_dep(id: &str, version_range: &str) -> DependencyReference {
        DependencyReference {
            artifact_type: DependencyArtifactType::Capability,
            id: id.to_string(),
            version: version_range.to_string(),
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

    // ── tests ──────────────────────────────────────────────────────────────

    #[test]
    fn resolves_capability_with_no_dependencies() {
        let registry = CapabilityRegistry::new();
        let result = resolve_dependencies(&registry, "test.a.cap", &[], LookupScope::PublicOnly)
            .expect("empty deps should resolve");
        assert!(result.is_empty());
    }

    #[test]
    fn resolves_satisfied_dependency() {
        let mut registry = CapabilityRegistry::new();
        register(&mut registry, base_contract("test.logging.logger", "1.2.0"));

        let deps = vec![cap_dep("test.logging.logger", "^1.0.0")];
        let lock = resolve_dependencies(
            &registry,
            "test.app.consumer",
            &deps,
            LookupScope::PublicOnly,
        )
        .expect("satisfiable dep should resolve");

        assert_eq!(lock.len(), 1);
        assert_eq!(lock[0].capability_id, "test.logging.logger");
        assert_eq!(lock[0].version, "1.2.0");
        assert!(!lock[0].digest.is_empty());
    }

    #[test]
    fn returns_error_for_missing_dependency() {
        let registry = CapabilityRegistry::new();
        let deps = vec![cap_dep("test.missing.capability", "^1.0.0")];
        let err = resolve_dependencies(
            &registry,
            "test.app.consumer",
            &deps,
            LookupScope::PublicOnly,
        )
        .expect_err("missing dep should fail");

        assert!(
            matches!(
                err,
                ResolutionError::MissingDependency { ref capability_id, .. }
                if capability_id == "test.missing.capability"
            ),
            "expected MissingDependency, got {err:?}"
        );
    }

    #[test]
    fn detects_circular_dependency() {
        // A depends on B (exact version), B depends on A (exact version).
        // The contracts use exact semver "1.0.0" to pass contract validation.
        // The resolver then traverses transitively and detects the cycle.
        let mut registry = CapabilityRegistry::new();
        register(
            &mut registry,
            contract_with_deps(
                "test.cycle.cap-a",
                "1.0.0",
                vec![cap_dep("test.cycle.cap-b", "1.0.0")],
            ),
        );
        register(
            &mut registry,
            contract_with_deps(
                "test.cycle.cap-b",
                "1.0.0",
                vec![cap_dep("test.cycle.cap-a", "1.0.0")],
            ),
        );

        // Resolving from outside: root depends on A, A depends on B, B depends
        // on A — cycle detected.
        let deps = vec![cap_dep("test.cycle.cap-a", "1.0.0")];
        let err = resolve_dependencies(
            &registry,
            "test.root.caller",
            &deps,
            LookupScope::PublicOnly,
        )
        .expect_err("circular dep should fail");

        assert!(
            matches!(err, ResolutionError::CircularDependency { .. }),
            "expected CircularDependency, got {err:?}"
        );
    }

    #[test]
    fn ignores_non_capability_dependencies() {
        let registry = CapabilityRegistry::new();
        let deps = vec![DependencyReference {
            artifact_type: DependencyArtifactType::Event,
            id: "test.events.some-event".to_string(),
            version: "^1.0.0".to_string(),
        }];
        let lock = resolve_dependencies(
            &registry,
            "test.app.consumer",
            &deps,
            LookupScope::PublicOnly,
        )
        .expect("non-capability deps should be ignored");
        assert!(lock.is_empty());
    }

    #[test]
    fn returns_error_for_invalid_version_range() {
        let mut registry = CapabilityRegistry::new();
        register(&mut registry, base_contract("test.logging.logger", "1.0.0"));

        let deps = vec![cap_dep("test.logging.logger", ">>invalid")];
        let err = resolve_dependencies(
            &registry,
            "test.app.consumer",
            &deps,
            LookupScope::PublicOnly,
        )
        .expect_err("invalid range should fail");

        assert!(
            matches!(err, ResolutionError::MissingDependency { .. }),
            "expected MissingDependency for invalid range, got {err:?}"
        );
    }

    #[test]
    fn verify_lock_digests_detects_changed_digest() {
        let mut registry = CapabilityRegistry::new();
        register(&mut registry, base_contract("test.logging.logger", "1.0.0"));

        let deps = vec![cap_dep("test.logging.logger", "1.0.0")];
        let lock = resolve_dependencies(
            &registry,
            "test.app.consumer",
            &deps,
            LookupScope::PublicOnly,
        )
        .expect("should resolve");

        // Lock captures the original digest — no mismatch yet.
        assert!(
            verify_lock_digests(&registry, &lock, LookupScope::PublicOnly).is_none(),
            "freshly locked digest should match"
        );

        // Simulate a stale lock by manually altering the captured digest.
        let stale_lock = vec![ResolvedDependencyLock {
            capability_id: lock[0].capability_id.clone(),
            version: lock[0].version.clone(),
            digest: "stale:000000000000dead".to_string(),
        }];

        let mismatch = verify_lock_digests(&registry, &stale_lock, LookupScope::PublicOnly)
            .expect("stale digest should yield a mismatch");
        assert_eq!(mismatch.capability_id, "test.logging.logger");
        assert_eq!(mismatch.locked_digest, "stale:000000000000dead");
    }

    #[test]
    fn resolves_transitive_dependency() {
        let mut registry = CapabilityRegistry::new();
        // C has no deps; B depends on C (exact version, passes contract validation).
        register(&mut registry, base_contract("test.chain.cap-c", "1.0.0"));
        register(
            &mut registry,
            contract_with_deps(
                "test.chain.cap-b",
                "1.0.0",
                vec![cap_dep("test.chain.cap-c", "1.0.0")],
            ),
        );

        // A (the caller) declares a range dep on B — this is passed directly to
        // resolve_dependencies, not through contract registration, so ranges work.
        let deps = vec![cap_dep("test.chain.cap-b", "1.0.0")];
        let lock = resolve_dependencies(
            &registry,
            "test.chain.cap-a",
            &deps,
            LookupScope::PublicOnly,
        )
        .expect("transitive chain should resolve");

        // Lock should contain both B and C.
        let ids: Vec<&str> = lock.iter().map(|e| e.capability_id.as_str()).collect();
        assert!(ids.contains(&"test.chain.cap-b"), "B should be in lock");
        assert!(ids.contains(&"test.chain.cap-c"), "C should be in lock");
    }

    #[test]
    fn resolves_private_scoped_dependency() {
        let mut registry = CapabilityRegistry::new();
        let contract = base_contract("test.logging.logger", "1.0.0");
        let artifact = executable_artifact(&contract);
        registry
            .register(CapabilityRegistration {
                scope: RegistryScope::Private,
                contract_path: "registry/private/test.logging.logger/1.0.0/contract.json"
                    .to_string(),
                artifact,
                registered_at: "2026-04-20T00:00:00Z".to_string(),
                tags: vec![],
                composability: ComposabilityMetadata {
                    kind: CompositionKind::Atomic,
                    patterns: vec![CompositionPattern::Sequential],
                    provides: vec!["output".to_string()],
                    requires: vec!["input".to_string()],
                },
                governing_spec: "043-module-dependency-management".to_string(),
                validator_version: "test".to_string(),
                contract,
            })
            .expect("private registration should succeed");

        let deps = vec![cap_dep("test.logging.logger", "1.0.0")];
        let lock = resolve_dependencies(
            &registry,
            "test.app.consumer",
            &deps,
            LookupScope::PreferPrivate,
        )
        .expect("private dep should resolve");

        assert_eq!(lock.len(), 1);
        assert_eq!(lock[0].capability_id, "test.logging.logger");
    }

    #[test]
    fn deduplicates_diamond_dependency() {
        let mut registry = CapabilityRegistry::new();
        register(&mut registry, base_contract("test.diamond.c", "1.0.0"));
        register(
            &mut registry,
            contract_with_deps(
                "test.diamond.a",
                "1.0.0",
                vec![cap_dep("test.diamond.c", "1.0.0")],
            ),
        );
        register(
            &mut registry,
            contract_with_deps(
                "test.diamond.b",
                "1.0.0",
                vec![cap_dep("test.diamond.c", "1.0.0")],
            ),
        );

        let deps = vec![
            cap_dep("test.diamond.a", "1.0.0"),
            cap_dep("test.diamond.b", "1.0.0"),
        ];
        let lock = resolve_dependencies(&registry, "test.consumer", &deps, LookupScope::PublicOnly)
            .expect("diamond dep should resolve");

        let c_count = lock
            .iter()
            .filter(|e| e.capability_id == "test.diamond.c")
            .count();
        assert_eq!(c_count, 1, "C should appear exactly once");
        assert_eq!(lock.len(), 3, "lock should have A, B, C");
    }

    #[test]
    fn rejects_when_max_transitive_depth_exceeded() {
        let chain = [
            "test.dep.l1",
            "test.dep.l2",
            "test.dep.l3",
            "test.dep.l4",
            "test.dep.l5",
            "test.dep.l6",
        ];
        let mut registry = CapabilityRegistry::new();
        for i in (0..chain.len()).rev() {
            let next_deps = if i + 1 < chain.len() {
                vec![cap_dep(chain[i + 1], "1.0.0")]
            } else {
                vec![]
            };
            register(
                &mut registry,
                contract_with_deps(chain[i], "1.0.0", next_deps),
            );
        }

        let err = resolve_dependencies(
            &registry,
            "test.consumer.root",
            &[cap_dep(chain[0], "1.0.0")],
            LookupScope::PublicOnly,
        )
        .expect_err("depth exceeded should fail");

        assert!(
            matches!(err, ResolutionError::MaxTransitiveDepthExceeded { .. }),
            "expected MaxTransitiveDepthExceeded, got {err:?}"
        );
    }

    #[test]
    fn lookup_lock_record_returns_registry_record() {
        let mut registry = CapabilityRegistry::new();
        register(&mut registry, base_contract("test.logging.logger", "1.0.0"));

        let lock = resolve_dependencies(
            &registry,
            "test.app.consumer",
            &[cap_dep("test.logging.logger", "1.0.0")],
            LookupScope::PublicOnly,
        )
        .expect("should resolve");

        assert!(
            lookup_lock_record(&registry, &lock[0], LookupScope::PublicOnly).is_some(),
            "should find the record"
        );
        assert!(
            lookup_lock_record(
                &registry,
                &ResolvedDependencyLock {
                    capability_id: "test.nonexistent".to_string(),
                    version: "9.9.9".to_string(),
                    digest: "none".to_string(),
                },
                LookupScope::PublicOnly
            )
            .is_none(),
            "missing entry should return None"
        );
    }
}
