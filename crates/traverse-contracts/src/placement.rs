//! Shared placement constraint evaluator (spec `024-placement-constraint-evaluator`).
//!
//! Engine-agnostic three-tier placement used by both native
//! `PlacementRouter` (`traverse-runtime`) and the nested-wasmi
//! `runtime.wasm` path (`traverse-runtime-wasm`) so constraint outcomes
//! cannot drift between hosts (spec `1402` FR-003/FR-004, Decision 88).

use std::collections::HashMap;

use crate::{CapabilityContract, ExecutionTarget};

/// A snapshot of runtime target load at a point in time.
#[derive(Debug, Clone, Default)]
pub struct RuntimeSnapshot {
    /// Load score per target (0.0 = idle, 1.0 = saturated).
    /// Targets absent from this map are treated as load 0.0.
    pub target_loads: HashMap<ExecutionTarget, f32>,
}

/// Input to the placement evaluator.
pub struct PlacementRequest {
    pub capability_id: String,
    pub target_hint: Option<ExecutionTarget>,
    pub runtime_snapshot: RuntimeSnapshot,
}

/// The result of a successful placement evaluation.
#[derive(Debug, Clone, PartialEq)]
pub struct PlacementDecision {
    pub target: ExecutionTarget,
    pub reason: PlacementReason,
    pub confidence: PlacementConfidence,
}

/// Why the selected target was chosen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlacementReason {
    /// The caller's hint was accepted because it is a permitted target.
    CallerHintAccepted,
    /// A single target remained after contract constraints were applied.
    ContractConstrained,
    /// The target was selected by load-based heuristics.
    HeuristicSelected,
}

/// Confidence level derived from the selected target's load score.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlacementConfidence {
    /// load < 0.5
    High,
    /// 0.5 <= load < 0.75
    Medium,
    /// 0.75 <= load < 0.9
    Low,
}

/// Errors that can occur during placement evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlacementError {
    /// No target survived all constraint tiers.
    NoEligibleTarget,
}

/// Stateless evaluator that applies the three-tier placement algorithm.
pub struct PlacementConstraintEvaluator;

impl PlacementConstraintEvaluator {
    /// Evaluate placement for `request` against `contract.permitted_targets`.
    ///
    /// # Errors
    ///
    /// Returns [`PlacementError::NoEligibleTarget`] when no target survives all
    /// three constraint tiers (caller hint, contract constraints, heuristics).
    pub fn evaluate(
        &self,
        request: &PlacementRequest,
        contract: &CapabilityContract,
    ) -> Result<PlacementDecision, PlacementError> {
        self.evaluate_targets(request, &contract.permitted_targets)
    }

    /// Same algorithm as [`Self::evaluate`], taking an explicit permitted-target
    /// list. Used by `runtime.wasm` which stores targets from init metadata
    /// without reconstructing a full [`CapabilityContract`].
    ///
    /// # Errors
    ///
    /// Returns [`PlacementError::NoEligibleTarget`] when no target survives.
    pub fn evaluate_targets(
        &self,
        request: &PlacementRequest,
        permitted_targets: &[ExecutionTarget],
    ) -> Result<PlacementDecision, PlacementError> {
        // --- Tier 1: Caller hint ---
        if let Some(ref hint) = request.target_hint
            && permitted_targets.contains(hint)
        {
            let load = load_for(&request.runtime_snapshot, hint);
            return Ok(PlacementDecision {
                target: hint.clone(),
                reason: PlacementReason::CallerHintAccepted,
                confidence: confidence_for(load),
            });
        }

        // --- Tier 2: Contract constraints ---
        // Spec `132` allows Stateful+Browser here; IndexedDB attestation is an
        // activation gate, not a placement filter.
        let mut eligible: Vec<ExecutionTarget> = permitted_targets.to_vec();
        let pre_heuristic_count = eligible.len();

        // --- Tier 3: Heuristics ---
        // Remove overloaded targets (load > 0.9).
        eligible.retain(|t| load_for(&request.runtime_snapshot, t) <= 0.9);

        if eligible.is_empty() {
            return Err(PlacementError::NoEligibleTarget);
        }

        let selected = eligible
            .into_iter()
            .min_by(|a, b| {
                let la = load_for(&request.runtime_snapshot, a);
                let lb = load_for(&request.runtime_snapshot, b);
                la.partial_cmp(&lb)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| format!("{a:?}").cmp(&format!("{b:?}")))
            })
            .ok_or(PlacementError::NoEligibleTarget)?;

        let load = load_for(&request.runtime_snapshot, &selected);
        let reason = if pre_heuristic_count == 1 {
            PlacementReason::ContractConstrained
        } else {
            PlacementReason::HeuristicSelected
        };

        Ok(PlacementDecision {
            target: selected,
            reason,
            confidence: confidence_for(load),
        })
    }
}

fn load_for(snapshot: &RuntimeSnapshot, target: &ExecutionTarget) -> f32 {
    snapshot.target_loads.get(target).copied().unwrap_or(0.0)
}

fn confidence_for(load: f32) -> PlacementConfidence {
    if load < 0.5 {
        PlacementConfidence::High
    } else if load < 0.75 {
        PlacementConfidence::Medium
    } else {
        PlacementConfidence::Low
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]

    use super::*;
    use crate::{
        BinaryFormat, CapabilityContract, Condition, Entrypoint, EntrypointKind, Execution,
        ExecutionConstraints, FilesystemAccess, HostApiAccess, Lifecycle, NetworkAccess, Owner,
        Provenance, ProvenanceSource, SchemaContainer, ServiceType, SideEffect, SideEffectKind,
        default_risk_metadata,
    };

    const GOVERNING_SPEC: &str = "002-capability-contracts@0.1.0";

    fn base_contract() -> CapabilityContract {
        CapabilityContract {
            kind: "capability_contract".to_string(),
            schema_version: "1.0.0".to_string(),
            id: "placement.tests.evaluator-subject".to_string(),
            namespace: "placement.tests".to_string(),
            name: "evaluator-subject".to_string(),
            version: "0.1.0".to_string(),
            lifecycle: Lifecycle::Draft,
            owner: Owner {
                team: "traverse-core".to_string(),
                contact: "test@example.com".to_string(),
            },
            summary: "Placement evaluator test subject capability.".to_string(),
            description: "Used only in placement evaluator tests.".to_string(),
            inputs: SchemaContainer {
                schema: serde_json::json!({ "type": "object" }),
            },
            outputs: SchemaContainer {
                schema: serde_json::json!({ "type": "object" }),
            },
            preconditions: vec![Condition {
                id: "always-met".to_string(),
                description: "No preconditions in test.".to_string(),
            }],
            postconditions: vec![Condition {
                id: "always-met".to_string(),
                description: "No postconditions in test.".to_string(),
            }],
            side_effects: vec![SideEffect {
                kind: SideEffectKind::MemoryOnly,
                description: "No durable side effect.".to_string(),
            }],
            emits: Vec::new(),
            consumes: Vec::new(),
            permissions: Vec::new(),
            execution: Execution {
                binary_format: BinaryFormat::Wasm,
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
            policies: Vec::new(),
            dependencies: Vec::new(),
            provenance: Provenance {
                source: ProvenanceSource::Greenfield,
                author: "test-author".to_string(),
                created_at: "2026-04-01T00:00:00Z".to_string(),
                spec_ref: Some(GOVERNING_SPEC.to_string()),
                adr_refs: Vec::new(),
                exception_refs: Vec::new(),
            },
            evidence: Vec::new(),
            service_type: ServiceType::Stateless,
            permitted_targets: vec![
                ExecutionTarget::Local,
                ExecutionTarget::Cloud,
                ExecutionTarget::Edge,
            ],
            event_trigger: None,
            connector_requirements: Vec::new(),
            state_schema: None,
            use_cases: Vec::new(),
            risk: default_risk_metadata(),
        }
    }

    fn snapshot_with(pairs: &[(ExecutionTarget, f32)]) -> RuntimeSnapshot {
        RuntimeSnapshot {
            target_loads: pairs.iter().cloned().collect(),
        }
    }

    #[test]
    fn tier1_hint_accepted_when_in_permitted_targets() {
        let contract = base_contract();
        let request = PlacementRequest {
            capability_id: contract.id.clone(),
            target_hint: Some(ExecutionTarget::Cloud),
            runtime_snapshot: snapshot_with(&[(ExecutionTarget::Cloud, 0.3)]),
        };
        let decision = PlacementConstraintEvaluator
            .evaluate(&request, &contract)
            .expect("hint accepted");
        assert_eq!(decision.target, ExecutionTarget::Cloud);
        assert_eq!(decision.reason, PlacementReason::CallerHintAccepted);
        assert_eq!(decision.confidence, PlacementConfidence::High);
    }

    #[test]
    fn tier1_hint_rejected_falls_through_to_tier3() {
        let contract = base_contract();
        let request = PlacementRequest {
            capability_id: contract.id.clone(),
            target_hint: Some(ExecutionTarget::Browser),
            runtime_snapshot: snapshot_with(&[
                (ExecutionTarget::Local, 0.2),
                (ExecutionTarget::Cloud, 0.5),
                (ExecutionTarget::Edge, 0.8),
            ]),
        };
        let decision = PlacementConstraintEvaluator
            .evaluate(&request, &contract)
            .expect("heuristic");
        assert_eq!(decision.target, ExecutionTarget::Local);
        assert_eq!(decision.reason, PlacementReason::HeuristicSelected);
    }

    #[test]
    fn tier2_allows_browser_for_stateful_when_lowest_load() {
        let mut contract = base_contract();
        contract.service_type = ServiceType::Stateful;
        contract.permitted_targets = vec![
            ExecutionTarget::Local,
            ExecutionTarget::Browser,
            ExecutionTarget::Cloud,
        ];
        let request = PlacementRequest {
            capability_id: contract.id.clone(),
            target_hint: None,
            runtime_snapshot: snapshot_with(&[
                (ExecutionTarget::Browser, 0.1),
                (ExecutionTarget::Local, 0.4),
                (ExecutionTarget::Cloud, 0.6),
            ]),
        };
        let decision = PlacementConstraintEvaluator
            .evaluate(&request, &contract)
            .expect("browser eligible");
        assert_eq!(decision.target, ExecutionTarget::Browser);
    }

    #[test]
    fn tier2_single_permitted_target_is_contract_constrained() {
        let mut contract = base_contract();
        contract.permitted_targets = vec![ExecutionTarget::Local];
        let request = PlacementRequest {
            capability_id: contract.id.clone(),
            target_hint: None,
            runtime_snapshot: snapshot_with(&[
                (ExecutionTarget::Cloud, 0.0),
                (ExecutionTarget::Local, 0.4),
            ]),
        };
        let decision = PlacementConstraintEvaluator
            .evaluate(&request, &contract)
            .expect("local only");
        assert_eq!(decision.target, ExecutionTarget::Local);
        assert_eq!(decision.reason, PlacementReason::ContractConstrained);
    }

    #[test]
    fn tier3_selects_lowest_load_target() {
        let contract = base_contract();
        let request = PlacementRequest {
            capability_id: contract.id.clone(),
            target_hint: None,
            runtime_snapshot: snapshot_with(&[
                (ExecutionTarget::Local, 0.6),
                (ExecutionTarget::Cloud, 0.2),
                (ExecutionTarget::Edge, 0.4),
            ]),
        };
        let decision = PlacementConstraintEvaluator
            .evaluate(&request, &contract)
            .expect("cloud lowest");
        assert_eq!(decision.target, ExecutionTarget::Cloud);
        assert_eq!(decision.reason, PlacementReason::HeuristicSelected);
        assert_eq!(decision.confidence, PlacementConfidence::High);
    }

    #[test]
    fn tier3_overloaded_targets_excluded() {
        let contract = base_contract();
        let request = PlacementRequest {
            capability_id: contract.id.clone(),
            target_hint: None,
            runtime_snapshot: snapshot_with(&[
                (ExecutionTarget::Local, 0.95),
                (ExecutionTarget::Cloud, 0.91),
                (ExecutionTarget::Edge, 0.85),
            ]),
        };
        let decision = PlacementConstraintEvaluator
            .evaluate(&request, &contract)
            .expect("edge remains");
        assert_eq!(decision.target, ExecutionTarget::Edge);
        assert_eq!(decision.confidence, PlacementConfidence::Low);
    }

    #[test]
    fn no_eligible_target_returns_error() {
        let contract = base_contract();
        let request = PlacementRequest {
            capability_id: contract.id.clone(),
            target_hint: None,
            runtime_snapshot: snapshot_with(&[
                (ExecutionTarget::Local, 0.95),
                (ExecutionTarget::Cloud, 0.92),
                (ExecutionTarget::Edge, 0.91),
            ]),
        };
        assert_eq!(
            PlacementConstraintEvaluator.evaluate(&request, &contract),
            Err(PlacementError::NoEligibleTarget)
        );
    }

    #[test]
    fn evaluate_targets_matches_evaluate_for_same_list() {
        let contract = base_contract();
        let request = PlacementRequest {
            capability_id: contract.id.clone(),
            target_hint: None,
            runtime_snapshot: snapshot_with(&[(ExecutionTarget::Cloud, 0.1)]),
        };
        let via_contract = PlacementConstraintEvaluator
            .evaluate(&request, &contract)
            .expect("contract path");
        let via_targets = PlacementConstraintEvaluator
            .evaluate_targets(&request, &contract.permitted_targets)
            .expect("targets path");
        assert_eq!(via_contract, via_targets);
    }

    #[test]
    fn confidence_medium_for_mid_load() {
        let mut contract = base_contract();
        contract.permitted_targets = vec![ExecutionTarget::Local];
        let request = PlacementRequest {
            capability_id: contract.id.clone(),
            target_hint: None,
            runtime_snapshot: snapshot_with(&[(ExecutionTarget::Local, 0.6)]),
        };
        let decision = PlacementConstraintEvaluator
            .evaluate(&request, &contract)
            .expect("medium");
        assert_eq!(decision.confidence, PlacementConfidence::Medium);
    }

    #[test]
    fn all_execution_targets_are_selectable_via_hint() {
        // Full constraint-type / target-variant coverage for nested-path parity.
        let targets = [
            ExecutionTarget::Local,
            ExecutionTarget::Browser,
            ExecutionTarget::Edge,
            ExecutionTarget::Cloud,
            ExecutionTarget::Worker,
            ExecutionTarget::Device,
        ];
        for target in &targets {
            let request = PlacementRequest {
                capability_id: "placement.parity".to_string(),
                target_hint: Some(target.clone()),
                runtime_snapshot: RuntimeSnapshot {
                    target_loads: HashMap::new(),
                },
            };
            let decision = PlacementConstraintEvaluator
                .evaluate_targets(&request, &targets)
                .expect("hint must accept every ExecutionTarget variant");
            assert_eq!(decision.target, *target);
            assert_eq!(decision.reason, PlacementReason::CallerHintAccepted);
        }
    }
}
