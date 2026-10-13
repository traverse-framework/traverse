//! Bounded deterministic parallel proposal execution (spec
//! `110-bounded-parallel-workflow-scheduling`, P2, ADR-0042).
//!
//! Extends the P1 sequential proposal executor in `crate::proposal` with a
//! wave-based concurrent dispatcher over the same `CanonicalProposal`/
//! `ResolvedProposalNode` shapes. A [`traverse_contracts::ParallelSchedule`]
//! (computed by `traverse_contracts::compute_parallel_schedule`) levelizes
//! the already-validated acyclic graph into waves of node ids whose
//! dependencies are satisfied by earlier waves; this module authorizes and
//! executes that schedule.
//!
//! FR-004a (spec 110): the first P2 implementation permits a wave with more
//! than one member only when every member's declared `effect_class` is
//! `pure_read` — [`enforce_pure_read_only_parallelism`] checks this before
//! any dispatch. Once authorized, each wave runs on real OS threads via
//! [`std::thread::scope`], bounded to `max_concurrent_nodes` per batch;
//! outcomes are folded back into the trace in the wave's lexicographic
//! order (not completion order), so the observable trace is deterministic
//! regardless of real scheduling (FR-002).
//!
//! Wall-clock and payload-size bounds (FR-001, FR-005) are checked *between*
//! waves, before committing to the next one — Rust gives no safe way to
//! preemptively interrupt an in-flight OS thread without `unsafe`, and
//! FR-004a already restricts concurrent work to side-effect-free local reads,
//! so a wave that is already dispatched is always allowed to finish; a
//! budget that is already exhausted simply stops further waves from
//! starting, reported as [`traverse_contracts::CanonicalProposal`]'s trace
//! terminal state `cancelled`.

use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::time::{Duration, Instant};

use traverse_contracts::{CanonicalProposal, EffectClass, ParallelSchedule, ProposalNode};

use crate::proposal::{
    AuthorizationSummary, ProposalNodeOutcome, ProposalNodeStatus, ProposalTerminalState,
    ProposalTrace, ResolvedProposalNode, assemble_node_input, build_node_execution_request,
};
use crate::{Runtime, RuntimeResultStatus};

// ---------------------------------------------------------------------------
// FR-004a: pure_read-only concurrency authorization
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ParallelAuthorizationError {
    pub code: ParallelAuthorizationErrorCode,
    pub message: String,
    pub path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ParallelAuthorizationErrorCode {
    ConcurrentSideEffectDenied,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParallelAuthorizationFailure {
    pub errors: Vec<ParallelAuthorizationError>,
}

/// Enforces spec 110 FR-004a: the first P2 implementation permits parallel
/// execution only for `pure_read` nodes. Any wave with more than one member
/// is denied outright unless every member in it is `pure_read`.
///
/// # Errors
///
/// Returns [`ParallelAuthorizationFailure`] listing every offending node.
pub fn enforce_pure_read_only_parallelism(
    schedule: &ParallelSchedule,
    resolved_nodes: &[ResolvedProposalNode],
) -> Result<(), ParallelAuthorizationFailure> {
    let effect_class_by_node: HashMap<&str, EffectClass> = resolved_nodes
        .iter()
        .map(|node| (node.node_id.as_str(), node.contract.risk.effect_class))
        .collect();

    let mut errors = Vec::new();
    for (wave_index, wave) in schedule.waves.iter().enumerate() {
        if wave.len() <= 1 {
            continue;
        }
        for node_id in wave {
            if effect_class_by_node.get(node_id.as_str()) != Some(&EffectClass::PureRead) {
                errors.push(ParallelAuthorizationError {
                    code: ParallelAuthorizationErrorCode::ConcurrentSideEffectDenied,
                    message: format!(
                        "node '{node_id}' does not have effect_class pure_read and cannot run \
                         concurrently with other nodes in wave {wave_index}"
                    ),
                    path: format!("$.schedule.waves[{wave_index}]"),
                });
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(ParallelAuthorizationFailure { errors })
    }
}

// ---------------------------------------------------------------------------
// Execution-time bounds (spec 110 FR-001, FR-005): wall time and payload size
// ---------------------------------------------------------------------------

/// Execution-time bounds that structural schedule validation cannot express
/// (spec 110 FR-001: "time, memory" bounds). Checked between waves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParallelExecutionLimits {
    /// Total wall-clock budget for the whole parallel execution. Checked
    /// before starting each wave, not preemptively mid-wave.
    pub max_wall_time: Duration,
    /// Max total serialized JSON byte size of one wave's assembled node
    /// inputs — a bounded, honest proxy for a per-wave memory budget.
    pub max_wave_payload_bytes: usize,
    /// Max node executions the runtime dispatches at once within a wave —
    /// the execution-time enforcement of
    /// `ParallelScheduleLimits::max_concurrent_nodes`.
    pub max_concurrent_nodes: usize,
}

pub const DEFAULT_MAX_WALL_TIME_MS: u64 = 30_000;
pub const DEFAULT_MAX_WAVE_PAYLOAD_BYTES: usize = 1_048_576;

impl Default for ParallelExecutionLimits {
    fn default() -> Self {
        Self {
            max_wall_time: Duration::from_millis(DEFAULT_MAX_WALL_TIME_MS),
            max_wave_payload_bytes: DEFAULT_MAX_WAVE_PAYLOAD_BYTES,
            max_concurrent_nodes: traverse_contracts::DEFAULT_MAX_CONCURRENT_NODES,
        }
    }
}

// ---------------------------------------------------------------------------
// Execution
// ---------------------------------------------------------------------------

/// Executes an authorized [`ParallelSchedule`] wave by wave, dispatching
/// each wave's nodes concurrently (bounded to
/// `limits.max_concurrent_nodes` batches) when the wave has more than one
/// member, and folding results back in lexicographic order regardless of
/// real completion order (spec 110 FR-002).
///
/// Stops advancing to further waves — but never interrupts an
/// already-dispatched one — at the first node failure, exhausted wall-time
/// budget, or exceeded wave payload budget (spec 110 FR-005, FR-008
/// carried over from P1).
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn execute_parallel_proposal<E: crate::LocalExecutor>(
    runtime: &Runtime<E>,
    canonical: &CanonicalProposal,
    schedule: &ParallelSchedule,
    authorization: AuthorizationSummary,
    proposal_digest: &str,
    snapshot_digest: &str,
    limits: &ParallelExecutionLimits,
) -> ProposalTrace
where
    Runtime<E>: Sync,
{
    let nodes_by_id: HashMap<&str, &ProposalNode> = canonical
        .proposal
        .nodes
        .iter()
        .map(|node| (node.node_id.as_str(), node))
        .collect();

    let mut outputs: HashMap<String, Value> = HashMap::new();
    let mut outcomes: Vec<ProposalNodeOutcome> = Vec::with_capacity(canonical.proposal.nodes.len());
    let mut failed = false;
    let mut cancelled = false;
    let start = Instant::now();
    let batch_size = limits.max_concurrent_nodes.max(1);

    for wave in &schedule.waves {
        if failed || cancelled {
            push_skipped_wave(&mut outcomes, wave, &nodes_by_id);
            continue;
        }
        if start.elapsed() > limits.max_wall_time {
            cancelled = true;
            push_skipped_wave(&mut outcomes, wave, &nodes_by_id);
            continue;
        }

        let wave_inputs: Vec<(String, Value)> = wave
            .iter()
            .map(|node_id| {
                (
                    node_id.clone(),
                    assemble_node_input(canonical, node_id, &outputs),
                )
            })
            .collect();
        let wave_payload_bytes: usize = wave_inputs
            .iter()
            .map(|(_, input)| input.to_string().len())
            .sum();
        if wave_payload_bytes > limits.max_wave_payload_bytes {
            cancelled = true;
            push_skipped_wave(&mut outcomes, wave, &nodes_by_id);
            continue;
        }

        for batch in wave_inputs.chunks(batch_size) {
            for (node_id, outcome, output) in
                dispatch_batch(runtime, canonical, &nodes_by_id, batch)
            {
                if outcome.status == ProposalNodeStatus::Failed {
                    failed = true;
                }
                if let Some(output) = output {
                    outputs.insert(node_id, output);
                }
                outcomes.push(outcome);
            }
        }
    }

    let terminal_state = if failed {
        ProposalTerminalState::Failed
    } else if cancelled {
        ProposalTerminalState::Cancelled
    } else {
        ProposalTerminalState::Succeeded
    };

    ProposalTrace {
        proposal_id: canonical.proposal.proposal_id.clone(),
        proposal_digest: proposal_digest.to_string(),
        snapshot_digest: snapshot_digest.to_string(),
        authorization,
        node_outcomes: outcomes,
        mapping_paths: canonical
            .proposal
            .mappings
            .iter()
            .map(|m| (m.source_path.clone(), m.target_path.clone()))
            .collect(),
        terminal_state,
    }
}

/// Dispatches one bounded batch of a wave concurrently on real OS threads
/// and returns each entry's outcome (plus its output to store, on success)
/// in the batch's original lexicographic order, independent of actual
/// completion order. A node id absent from `nodes_by_id` (only reachable by
/// hand-constructing a [`ParallelSchedule`] that disagrees with `canonical`
/// — never produced by `traverse_contracts::compute_parallel_schedule`) is
/// silently skipped. A panicking executor is surfaced as a `Failed` outcome
/// rather than silently dropped, matching this crate's fail-closed
/// convention for host-side faults.
fn dispatch_batch<'a, E: crate::LocalExecutor>(
    runtime: &Runtime<E>,
    canonical: &CanonicalProposal,
    nodes_by_id: &HashMap<&'a str, &'a ProposalNode>,
    batch: &[(String, Value)],
) -> Vec<(String, ProposalNodeOutcome, Option<Value>)>
where
    Runtime<E>: Sync,
{
    let dispatchable: Vec<(&str, &'a ProposalNode, &Value)> = batch
        .iter()
        .filter_map(|(node_id, input)| {
            nodes_by_id
                .get(node_id.as_str())
                .map(|node| (node_id.as_str(), *node, input))
        })
        .collect();

    let joined: Vec<(
        String,
        &'a ProposalNode,
        std::thread::Result<crate::RuntimeExecutionOutcome>,
    )> = std::thread::scope(|scope| {
        let handles: Vec<(
            String,
            &'a ProposalNode,
            std::thread::ScopedJoinHandle<'_, crate::RuntimeExecutionOutcome>,
        )> = dispatchable
            .iter()
            .map(|(node_id, node, input)| {
                let owned_node_id = (*node_id).to_string();
                let spawn_node_id = owned_node_id.clone();
                let node = *node;
                let input = (*input).clone();
                let handle = scope.spawn(move || {
                    let request =
                        build_node_execution_request(canonical, node, &spawn_node_id, input);
                    runtime.execute(request)
                });
                (owned_node_id, node, handle)
            })
            .collect();
        handles
            .into_iter()
            .map(|(node_id, node, handle)| (node_id, node, handle.join()))
            .collect()
    });

    joined
        .into_iter()
        .map(|(node_id, node, joined_result)| match joined_result {
            Ok(outcome) => match outcome.result.status {
                RuntimeResultStatus::Completed => (
                    node_id.clone(),
                    succeeded_outcome(&node_id, node),
                    outcome.result.output.clone(),
                ),
                RuntimeResultStatus::Error => (
                    node_id.clone(),
                    failed_outcome(&node_id, node, &outcome),
                    None,
                ),
            },
            Err(_) => (node_id.clone(), panicked_outcome(&node_id, node), None),
        })
        .collect()
}

fn succeeded_outcome(node_id: &str, node: &ProposalNode) -> ProposalNodeOutcome {
    ProposalNodeOutcome {
        node_id: node_id.to_string(),
        capability_id: node.capability_id.clone(),
        capability_version: node.capability_version.clone(),
        artifact_digest: node.artifact_digest.clone(),
        status: ProposalNodeStatus::Succeeded,
        error_code: None,
    }
}

fn failed_outcome(
    node_id: &str,
    node: &ProposalNode,
    outcome: &crate::RuntimeExecutionOutcome,
) -> ProposalNodeOutcome {
    ProposalNodeOutcome {
        node_id: node_id.to_string(),
        capability_id: node.capability_id.clone(),
        capability_version: node.capability_version.clone(),
        artifact_digest: node.artifact_digest.clone(),
        status: ProposalNodeStatus::Failed,
        error_code: outcome
            .result
            .error
            .as_ref()
            .map(|error| format!("{:?}", error.code)),
    }
}

/// A node execution thread panicked. Surfaced as a `Failed` outcome — never
/// silently dropped — matching this crate's fail-closed convention for
/// host-side faults (e.g. `events/broker.rs`'s poisoned-lock handling).
fn panicked_outcome(node_id: &str, node: &ProposalNode) -> ProposalNodeOutcome {
    ProposalNodeOutcome {
        node_id: node_id.to_string(),
        capability_id: node.capability_id.clone(),
        capability_version: node.capability_version.clone(),
        artifact_digest: node.artifact_digest.clone(),
        status: ProposalNodeStatus::Failed,
        error_code: Some("executor_panicked".to_string()),
    }
}

fn push_skipped_wave(
    outcomes: &mut Vec<ProposalNodeOutcome>,
    wave: &[String],
    nodes_by_id: &HashMap<&str, &ProposalNode>,
) {
    for node_id in wave {
        let Some(node) = nodes_by_id.get(node_id.as_str()) else {
            continue;
        };
        outcomes.push(ProposalNodeOutcome {
            node_id: node_id.clone(),
            capability_id: node.capability_id.clone(),
            capability_version: node.capability_version.clone(),
            artifact_digest: node.artifact_digest.clone(),
            status: ProposalNodeStatus::SkippedAfterEarlierFailure,
            error_code: None,
        });
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
#[allow(clippy::panic)]
mod tests {
    use super::*;
    use crate::security::RuntimeSecurityConfig;
    use crate::{
        LocalExecutionFailure, LocalExecutionFailureCode, LocalExecutionOutput, LocalExecutor,
    };
    use serde_json::json;
    use std::collections::{HashMap as StdHashMap, HashSet};
    use std::sync::{Arc, Mutex};
    use traverse_contracts::{
        BinaryFormat as ContractBinaryFormat, CapabilityContract, DataFlowPolicy, DeterminismClass,
        Entrypoint, EntrypointKind, Execution, ExecutionConstraints, ExecutionTarget,
        FilesystemAccess, HostApiAccess, Lifecycle, ManifestReference, NetworkAccess, Owner,
        ParallelScheduleLimits, ProposalEdge, ProposalLimits, ReliabilityMetadata, RiskMetadata,
        SchemaContainer, ServiceType, SideEffect, SideEffectKind, WorkflowProposal,
        canonicalize_proposal, compute_parallel_schedule,
    };
    use traverse_registry::{
        ArtifactDigests, BinaryFormat as RegistryBinaryFormat, BinaryReference,
        CapabilityArtifactRecord, CapabilityRegistration, CapabilityRegistry,
        ComposabilityMetadata, CompositionKind, CompositionPattern, ImplementationKind,
        RegistryProvenance, RegistryScope, SourceKind, SourceReference,
    };

    fn risk(effect_class: EffectClass) -> RiskMetadata {
        RiskMetadata {
            effect_class,
            determinism_class: DeterminismClass::Deterministic,
            data_flow: DataFlowPolicy::default(),
            reliability: ReliabilityMetadata {
                idempotency_required: false,
                retryable: true,
                compensation_available: false,
            },
        }
    }

    fn contract(capability_id: &str, effect_class: EffectClass) -> CapabilityContract {
        let (namespace, name) = capability_id
            .rsplit_once('.')
            .unwrap_or(("test", capability_id));
        CapabilityContract {
            kind: "capability_contract".to_string(),
            schema_version: "1.0.0".to_string(),
            id: capability_id.to_string(),
            namespace: namespace.to_string(),
            name: name.to_string(),
            version: "1.0.0".to_string(),
            lifecycle: Lifecycle::Active,
            owner: Owner {
                team: "traverse-core".to_string(),
                contact: "enrico.piovesan10@gmail.com".to_string(),
            },
            summary: "Test capability for parallel proposal scheduling.".to_string(),
            description: "Portable test capability used to validate parallel scheduling."
                .to_string(),
            inputs: SchemaContainer { schema: json!({}) },
            outputs: SchemaContainer { schema: json!({}) },
            preconditions: Vec::new(),
            postconditions: Vec::new(),
            side_effects: vec![SideEffect {
                kind: SideEffectKind::MemoryOnly,
                description: "No durable side effect.".to_string(),
            }],
            emits: Vec::new(),
            consumes: Vec::new(),
            permissions: Vec::new(),
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
                resources: None,
            },
            policies: Vec::new(),
            dependencies: Vec::new(),
            provenance: traverse_contracts::Provenance {
                source: traverse_contracts::ProvenanceSource::Greenfield,
                author: "test".to_string(),
                created_at: "2026-08-23T00:00:00Z".to_string(),
                spec_ref: None,
                adr_refs: Vec::new(),
                exception_refs: Vec::new(),
            },
            evidence: Vec::new(),
            service_type: ServiceType::Stateless,
            permitted_targets: vec![ExecutionTarget::Local],
            event_trigger: None,
            connector_requirements: Vec::new(),
            state_schema: None,
            use_cases: Vec::new(),
            risk: risk(effect_class),
        }
    }

    fn artifact(digest: &str) -> CapabilityArtifactRecord {
        CapabilityArtifactRecord {
            artifact_ref: format!("artifact:{digest}"),
            implementation_kind: ImplementationKind::Executable,
            source: SourceReference {
                kind: SourceKind::Git,
                location: "https://example.invalid/repo".to_string(),
            },
            binary: Some(BinaryReference {
                format: RegistryBinaryFormat::Wasm,
                location: format!("artifacts/{digest}/capability.wasm"),
                signature: None,
            }),
            workflow_ref: None,
            digests: ArtifactDigests {
                source_digest: format!("src-{digest}"),
                binary_digest: Some(digest.to_string()),
            },
            provenance: RegistryProvenance {
                source: "test".to_string(),
                author: "test".to_string(),
                created_at: "2026-08-23T00:00:00Z".to_string(),
            },
        }
    }

    fn registry_with(
        entries: Vec<(CapabilityContract, CapabilityArtifactRecord)>,
    ) -> CapabilityRegistry {
        let mut registry = CapabilityRegistry::new();
        for (contract, artifact) in entries {
            let outcome = registry.register(CapabilityRegistration {
                scope: RegistryScope::Public,
                contract,
                contract_path: "registry/test/contract.json".to_string(),
                artifact,
                registered_at: "2026-08-23T00:00:00Z".to_string(),
                tags: Vec::new(),
                composability: ComposabilityMetadata {
                    kind: CompositionKind::Atomic,
                    patterns: vec![CompositionPattern::Sequential],
                    provides: Vec::new(),
                    requires: Vec::new(),
                },
                governing_spec: "005-capability-registry".to_string(),
                validator_version: "0.1.0".to_string(),
            });
            assert!(outcome.is_ok(), "registration must succeed: {outcome:?}");
        }
        registry
    }

    fn node(node_id: &str, capability_id: &str) -> ProposalNode {
        ProposalNode {
            node_id: node_id.to_string(),
            capability_id: capability_id.to_string(),
            capability_version: "1.0.0".to_string(),
            artifact_digest: format!("digest-{node_id}"),
        }
    }

    fn resolved(node_id: &str, effect_class: EffectClass) -> ResolvedProposalNode {
        ResolvedProposalNode {
            node_id: node_id.to_string(),
            contract: contract(&format!("test.{node_id}"), effect_class),
        }
    }

    /// a fans out to b and c (both independently mapped from a's output),
    /// which both feed the join node d.
    fn diamond_workflow_proposal() -> WorkflowProposal {
        WorkflowProposal {
            kind: "workflow_proposal".to_string(),
            schema_version: "1.0.0".to_string(),
            proposal_id: "proposal-p2-001".to_string(),
            workspace_id: "workspace-001".to_string(),
            app_manifest: ManifestReference {
                app_id: "test-app".to_string(),
                app_version: "1.0.0".to_string(),
                manifest_digest: "sha256:manifest-digest".to_string(),
            },
            nodes: vec![
                node("a", "test.a"),
                node("b", "test.b"),
                node("c", "test.c"),
                node("d", "test.d"),
            ],
            edges: vec![
                ProposalEdge {
                    from_node_id: "a".to_string(),
                    to_node_id: "b".to_string(),
                },
                ProposalEdge {
                    from_node_id: "a".to_string(),
                    to_node_id: "c".to_string(),
                },
                ProposalEdge {
                    from_node_id: "b".to_string(),
                    to_node_id: "d".to_string(),
                },
                ProposalEdge {
                    from_node_id: "c".to_string(),
                    to_node_id: "d".to_string(),
                },
            ],
            mappings: Vec::new(),
            initial_input: json!({}),
        }
    }

    fn diamond_canonical() -> CanonicalProposal {
        canonicalize_proposal(diamond_workflow_proposal(), &ProposalLimits::default())
            .expect("diamond proposal must canonicalize")
    }

    fn diamond_resolved_nodes(non_read_effect: Option<&str>) -> Vec<ResolvedProposalNode> {
        ["a", "b", "c", "d"]
            .iter()
            .map(|id| {
                let effect_class = if Some(*id) == non_read_effect {
                    EffectClass::ExternalEffect
                } else {
                    EffectClass::PureRead
                };
                resolved(id, effect_class)
            })
            .collect()
    }

    // -- FR-004a: pure_read-only concurrency authorization -------------------

    #[test]
    fn allows_a_diamond_schedule_when_every_concurrent_wave_is_pure_read() -> Result<(), String> {
        let canonical = diamond_canonical();
        let schedule = compute_parallel_schedule(&canonical, &ParallelScheduleLimits::default())
            .map_err(|e| format!("{e:?}"))?;
        enforce_pure_read_only_parallelism(&schedule, &diamond_resolved_nodes(None))
            .map_err(|e| format!("{e:?}"))
    }

    #[test]
    fn denies_a_concurrent_wave_containing_a_non_pure_read_node() -> Result<(), String> {
        let canonical = diamond_canonical();
        let schedule = compute_parallel_schedule(&canonical, &ParallelScheduleLimits::default())
            .map_err(|e| format!("{e:?}"))?;
        let failure =
            enforce_pure_read_only_parallelism(&schedule, &diamond_resolved_nodes(Some("c")))
                .expect_err("a non-pure_read node in a concurrent wave must be denied");
        assert!(
            failure
                .errors
                .iter()
                .any(|e| e.code == ParallelAuthorizationErrorCode::ConcurrentSideEffectDenied)
        );
        Ok(())
    }

    #[test]
    fn allows_a_non_pure_read_node_when_its_wave_has_no_sibling() -> Result<(), String> {
        // b and c are pure_read and run concurrently in wave 1; a and d are
        // singleton waves and may be any effect class.
        let canonical = diamond_canonical();
        let schedule = compute_parallel_schedule(&canonical, &ParallelScheduleLimits::default())
            .map_err(|e| format!("{e:?}"))?;
        enforce_pure_read_only_parallelism(&schedule, &diamond_resolved_nodes(Some("d")))
            .map_err(|e| format!("{e:?}"))
    }

    // -- Execution -------------------------------------------------------------

    #[derive(Default)]
    struct ConcurrencyTracker {
        current: Mutex<usize>,
        max_seen: Mutex<usize>,
    }

    impl ConcurrencyTracker {
        fn enter(&self) {
            let mut current = self
                .current
                .lock()
                .expect("tracker lock must not be poisoned");
            *current += 1;
            let mut max_seen = self
                .max_seen
                .lock()
                .expect("tracker lock must not be poisoned");
            if *current > *max_seen {
                *max_seen = *current;
            }
        }

        fn exit(&self) {
            let mut current = self
                .current
                .lock()
                .expect("tracker lock must not be poisoned");
            *current -= 1;
        }

        fn max_seen(&self) -> usize {
            *self
                .max_seen
                .lock()
                .expect("tracker lock must not be poisoned")
        }
    }

    #[derive(Default, Clone)]
    struct ScriptedExecutor {
        fail_capability_ids: HashSet<String>,
        panic_capability_ids: HashSet<String>,
        sleep_capability_ids: StdHashMap<String, std::time::Duration>,
        concurrency: Arc<ConcurrencyTracker>,
    }

    impl LocalExecutor for ScriptedExecutor {
        fn execute(
            &self,
            capability: &traverse_registry::ResolvedCapability,
            input: &Value,
        ) -> Result<LocalExecutionOutput, LocalExecutionFailure> {
            self.concurrency.enter();
            if self.panic_capability_ids.contains(&capability.contract.id) {
                self.concurrency.exit();
                panic!("scripted executor panic for test");
            }
            if let Some(duration) = self.sleep_capability_ids.get(&capability.contract.id) {
                std::thread::sleep(*duration);
            }
            let result = if self.fail_capability_ids.contains(&capability.contract.id) {
                Err(LocalExecutionFailure {
                    code: LocalExecutionFailureCode::ExecutionFailed,
                    message: "scripted failure".to_string(),
                })
            } else {
                Ok(LocalExecutionOutput {
                    value: json!({"node": capability.contract.id, "received": input.clone()}),
                    emitted_events: Vec::new(),
                })
            };
            self.concurrency.exit();
            result
        }
    }

    fn diamond_registry() -> CapabilityRegistry {
        registry_with(vec![
            (
                contract("test.a", EffectClass::PureRead),
                artifact("digest-a"),
            ),
            (
                contract("test.b", EffectClass::PureRead),
                artifact("digest-b"),
            ),
            (
                contract("test.c", EffectClass::PureRead),
                artifact("digest-c"),
            ),
            (
                contract("test.d", EffectClass::PureRead),
                artifact("digest-d"),
            ),
        ])
    }

    #[test]
    fn executes_independent_branches_concurrently_with_a_deterministic_trace_order()
    -> Result<(), String> {
        let canonical = diamond_canonical();
        let schedule = compute_parallel_schedule(&canonical, &ParallelScheduleLimits::default())
            .map_err(|e| format!("{e:?}"))?;

        let mut sleep_capability_ids = StdHashMap::new();
        sleep_capability_ids.insert("test.c".to_string(), std::time::Duration::from_millis(20));
        let executor = ScriptedExecutor {
            sleep_capability_ids,
            ..ScriptedExecutor::default()
        };
        let runtime = Runtime::new(diamond_registry(), executor)
            .with_security_config(RuntimeSecurityConfig::development());

        let trace = execute_parallel_proposal(
            &runtime,
            &canonical,
            &schedule,
            AuthorizationSummary {
                automatic: true,
                approval_token_id: None,
            },
            "digest",
            "snapshot-digest",
            &ParallelExecutionLimits::default(),
        );

        assert_eq!(trace.terminal_state, ProposalTerminalState::Succeeded);
        // b finishes before c (c sleeps), but the trace order is always
        // lexicographic (b, c), never completion order.
        let node_order: Vec<&str> = trace
            .node_outcomes
            .iter()
            .map(|o| o.node_id.as_str())
            .collect();
        assert_eq!(node_order, vec!["a", "b", "c", "d"]);
        assert!(
            trace
                .node_outcomes
                .iter()
                .all(|o| o.status == ProposalNodeStatus::Succeeded)
        );
        Ok(())
    }

    #[test]
    fn bounds_real_concurrency_to_the_configured_max_concurrent_nodes() -> Result<(), String> {
        let canonical = diamond_canonical();
        let schedule = compute_parallel_schedule(&canonical, &ParallelScheduleLimits::default())
            .map_err(|e| format!("{e:?}"))?;

        let mut sleep_capability_ids = StdHashMap::new();
        sleep_capability_ids.insert("test.b".to_string(), std::time::Duration::from_millis(15));
        sleep_capability_ids.insert("test.c".to_string(), std::time::Duration::from_millis(15));
        let executor = ScriptedExecutor {
            sleep_capability_ids,
            ..ScriptedExecutor::default()
        };
        let concurrency = Arc::clone(&executor.concurrency);
        let runtime = Runtime::new(diamond_registry(), executor)
            .with_security_config(RuntimeSecurityConfig::development());

        let limits = ParallelExecutionLimits {
            max_concurrent_nodes: 1,
            ..ParallelExecutionLimits::default()
        };
        let trace = execute_parallel_proposal(
            &runtime,
            &canonical,
            &schedule,
            AuthorizationSummary {
                automatic: true,
                approval_token_id: None,
            },
            "digest",
            "snapshot-digest",
            &limits,
        );

        assert_eq!(trace.terminal_state, ProposalTerminalState::Succeeded);
        assert_eq!(concurrency.max_seen(), 1);
        Ok(())
    }

    #[test]
    fn stops_advancing_after_a_node_failure_but_lets_the_dispatched_wave_finish()
    -> Result<(), String> {
        let canonical = diamond_canonical();
        let schedule = compute_parallel_schedule(&canonical, &ParallelScheduleLimits::default())
            .map_err(|e| format!("{e:?}"))?;

        let mut fail_capability_ids = HashSet::new();
        fail_capability_ids.insert("test.c".to_string());
        let executor = ScriptedExecutor {
            fail_capability_ids,
            ..ScriptedExecutor::default()
        };
        let runtime = Runtime::new(diamond_registry(), executor)
            .with_security_config(RuntimeSecurityConfig::development());

        let trace = execute_parallel_proposal(
            &runtime,
            &canonical,
            &schedule,
            AuthorizationSummary {
                automatic: true,
                approval_token_id: None,
            },
            "digest",
            "snapshot-digest",
            &ParallelExecutionLimits::default(),
        );

        assert_eq!(trace.terminal_state, ProposalTerminalState::Failed);
        let outcome_by_node: StdHashMap<&str, ProposalNodeStatus> = trace
            .node_outcomes
            .iter()
            .map(|o| (o.node_id.as_str(), o.status.clone()))
            .collect();
        assert_eq!(outcome_by_node["a"], ProposalNodeStatus::Succeeded);
        assert_eq!(outcome_by_node["b"], ProposalNodeStatus::Succeeded);
        assert_eq!(outcome_by_node["c"], ProposalNodeStatus::Failed);
        assert_eq!(
            outcome_by_node["d"],
            ProposalNodeStatus::SkippedAfterEarlierFailure
        );
        Ok(())
    }

    #[test]
    fn surfaces_an_executor_panic_as_a_failed_outcome_instead_of_dropping_it() -> Result<(), String>
    {
        let canonical = diamond_canonical();
        let schedule = compute_parallel_schedule(&canonical, &ParallelScheduleLimits::default())
            .map_err(|e| format!("{e:?}"))?;

        let mut panic_capability_ids = HashSet::new();
        panic_capability_ids.insert("test.c".to_string());
        let executor = ScriptedExecutor {
            panic_capability_ids,
            ..ScriptedExecutor::default()
        };
        let runtime = Runtime::new(diamond_registry(), executor)
            .with_security_config(RuntimeSecurityConfig::development());

        let trace = execute_parallel_proposal(
            &runtime,
            &canonical,
            &schedule,
            AuthorizationSummary {
                automatic: true,
                approval_token_id: None,
            },
            "digest",
            "snapshot-digest",
            &ParallelExecutionLimits::default(),
        );

        assert_eq!(trace.terminal_state, ProposalTerminalState::Failed);
        let c_outcome = trace
            .node_outcomes
            .iter()
            .find(|o| o.node_id == "c")
            .expect("c must have an outcome, not be silently dropped");
        assert_eq!(c_outcome.status, ProposalNodeStatus::Failed);
        assert_eq!(c_outcome.error_code, Some("executor_panicked".to_string()));
        Ok(())
    }

    #[test]
    fn cancels_further_waves_once_the_wall_time_budget_is_exhausted() -> Result<(), String> {
        let canonical = diamond_canonical();
        let schedule = compute_parallel_schedule(&canonical, &ParallelScheduleLimits::default())
            .map_err(|e| format!("{e:?}"))?;

        let mut sleep_capability_ids = StdHashMap::new();
        sleep_capability_ids.insert("test.a".to_string(), std::time::Duration::from_millis(40));
        let executor = ScriptedExecutor {
            sleep_capability_ids,
            ..ScriptedExecutor::default()
        };
        let runtime = Runtime::new(diamond_registry(), executor)
            .with_security_config(RuntimeSecurityConfig::development());

        let limits = ParallelExecutionLimits {
            max_wall_time: std::time::Duration::from_millis(10),
            ..ParallelExecutionLimits::default()
        };
        let trace = execute_parallel_proposal(
            &runtime,
            &canonical,
            &schedule,
            AuthorizationSummary {
                automatic: true,
                approval_token_id: None,
            },
            "digest",
            "snapshot-digest",
            &limits,
        );

        assert_eq!(trace.terminal_state, ProposalTerminalState::Cancelled);
        let outcome_by_node: StdHashMap<&str, ProposalNodeStatus> = trace
            .node_outcomes
            .iter()
            .map(|o| (o.node_id.as_str(), o.status.clone()))
            .collect();
        assert_eq!(outcome_by_node["a"], ProposalNodeStatus::Succeeded);
        assert_eq!(
            outcome_by_node["b"],
            ProposalNodeStatus::SkippedAfterEarlierFailure
        );
        Ok(())
    }

    #[test]
    fn cancels_a_wave_whose_assembled_payload_exceeds_the_configured_byte_budget()
    -> Result<(), String> {
        let canonical = diamond_canonical();
        let schedule = compute_parallel_schedule(&canonical, &ParallelScheduleLimits::default())
            .map_err(|e| format!("{e:?}"))?;
        let executor = ScriptedExecutor::default();
        let runtime = Runtime::new(diamond_registry(), executor)
            .with_security_config(RuntimeSecurityConfig::development());

        let limits = ParallelExecutionLimits {
            max_wave_payload_bytes: 0,
            ..ParallelExecutionLimits::default()
        };
        let trace = execute_parallel_proposal(
            &runtime,
            &canonical,
            &schedule,
            AuthorizationSummary {
                automatic: true,
                approval_token_id: None,
            },
            "digest",
            "snapshot-digest",
            &limits,
        );

        assert_eq!(trace.terminal_state, ProposalTerminalState::Cancelled);
        Ok(())
    }

    #[test]
    fn execute_parallel_proposal_skips_an_unresolved_node_id_within_a_dispatched_wave()
    -> Result<(), String> {
        // `ParallelSchedule` has public fields, so a caller could hand-build
        // one that disagrees with `canonical` — never produced by
        // `compute_parallel_schedule` itself. This proves the executor
        // degrades gracefully (silently skips the unresolved id) rather than
        // panicking.
        let mut proposal = diamond_workflow_proposal();
        proposal.nodes.retain(|n| n.node_id == "a");
        proposal.edges.clear();
        let canonical = canonicalize_proposal(proposal, &ProposalLimits::default())
            .map_err(|e| format!("{e:?}"))?;
        let schedule = ParallelSchedule {
            waves: vec![vec!["a".to_string(), "ghost".to_string()]],
        };

        let executor = ScriptedExecutor::default();
        let runtime = Runtime::new(
            registry_with(vec![(
                contract("test.a", EffectClass::PureRead),
                artifact("digest-a"),
            )]),
            executor,
        )
        .with_security_config(RuntimeSecurityConfig::development());

        let trace = execute_parallel_proposal(
            &runtime,
            &canonical,
            &schedule,
            AuthorizationSummary {
                automatic: true,
                approval_token_id: None,
            },
            "digest",
            "snapshot-digest",
            &ParallelExecutionLimits::default(),
        );

        assert_eq!(trace.terminal_state, ProposalTerminalState::Succeeded);
        assert_eq!(trace.node_outcomes.len(), 1);
        assert_eq!(trace.node_outcomes[0].node_id, "a");
        Ok(())
    }

    #[test]
    fn execute_parallel_proposal_skips_an_unresolved_node_id_in_a_skipped_wave()
    -> Result<(), String> {
        let mut proposal = diamond_workflow_proposal();
        proposal.nodes.retain(|n| n.node_id == "a");
        proposal.edges.clear();
        let canonical = canonicalize_proposal(proposal, &ProposalLimits::default())
            .map_err(|e| format!("{e:?}"))?;
        let schedule = ParallelSchedule {
            waves: vec![vec!["a".to_string()], vec!["ghost".to_string()]],
        };

        let mut fail_capability_ids = HashSet::new();
        fail_capability_ids.insert("test.a".to_string());
        let executor = ScriptedExecutor {
            fail_capability_ids,
            ..ScriptedExecutor::default()
        };
        let runtime = Runtime::new(
            registry_with(vec![(
                contract("test.a", EffectClass::PureRead),
                artifact("digest-a"),
            )]),
            executor,
        )
        .with_security_config(RuntimeSecurityConfig::development());

        let trace = execute_parallel_proposal(
            &runtime,
            &canonical,
            &schedule,
            AuthorizationSummary {
                automatic: true,
                approval_token_id: None,
            },
            "digest",
            "snapshot-digest",
            &ParallelExecutionLimits::default(),
        );

        assert_eq!(trace.terminal_state, ProposalTerminalState::Failed);
        assert_eq!(trace.node_outcomes.len(), 1);
        assert_eq!(trace.node_outcomes[0].node_id, "a");
        Ok(())
    }

    #[test]
    fn parallel_execution_limits_default_matches_the_documented_defaults() {
        let limits = ParallelExecutionLimits::default();
        assert_eq!(
            limits.max_wall_time,
            std::time::Duration::from_millis(DEFAULT_MAX_WALL_TIME_MS)
        );
        assert_eq!(
            limits.max_wave_payload_bytes,
            DEFAULT_MAX_WAVE_PAYLOAD_BYTES
        );
        assert_eq!(
            limits.max_concurrent_nodes,
            traverse_contracts::DEFAULT_MAX_CONCURRENT_NODES
        );
    }
}
