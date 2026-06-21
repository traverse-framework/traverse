use serde::{Deserialize, Serialize};
use serde_json::Value;
use traverse_contracts::ExecutionTarget;

use crate::{ApplicationModelDependency, ModelCandidate};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelResolutionPhase {
    Setup,
    Execution,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModelResolutionRequest {
    pub phase: ModelResolutionPhase,
    pub requested_interface_id: String,
    pub requested_placement: ExecutionTarget,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelCandidateAvailability {
    pub provider_available: bool,
    pub model_available: bool,
    pub failure_code: Option<ModelCandidateRejectionCode>,
    pub reason: Option<String>,
}

impl ModelCandidateAvailability {
    #[must_use]
    pub fn ready() -> Self {
        Self {
            provider_available: true,
            model_available: true,
            failure_code: None,
            reason: None,
        }
    }

    #[must_use]
    pub fn rejected(code: ModelCandidateRejectionCode, reason: impl Into<String>) -> Self {
        let provider_available = code != ModelCandidateRejectionCode::ModelProviderUnavailable;
        let model_available = code != ModelCandidateRejectionCode::ModelCandidateUnavailable;
        Self {
            provider_available,
            model_available,
            failure_code: Some(code),
            reason: Some(reason.into()),
        }
    }
}

pub trait ModelAvailabilityProbe {
    fn check_candidate(
        &self,
        dependency: &ApplicationModelDependency,
        candidate: &ModelCandidate,
    ) -> ModelCandidateAvailability;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelCandidateReadiness {
    Ready,
    Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelCandidateRejectionCode {
    ModelProviderUnavailable,
    ModelCandidateUnavailable,
    ModelInterfaceUnsupported,
    ModelContextWindowInsufficient,
    ModelCandidateConfigInvalid,
    ModelDependencyUnsatisfied,
}

impl ModelCandidateRejectionCode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ModelProviderUnavailable => "model_provider_unavailable",
            Self::ModelCandidateUnavailable => "model_candidate_unavailable",
            Self::ModelInterfaceUnsupported => "model_interface_unsupported",
            Self::ModelContextWindowInsufficient => "model_context_window_insufficient",
            Self::ModelCandidateConfigInvalid => "model_candidate_config_invalid",
            Self::ModelDependencyUnsatisfied => "model_dependency_unsatisfied",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModelCandidateEvaluation {
    pub candidate_id: String,
    pub provider_capability_id: String,
    pub provider_implementation_id: String,
    pub model_identifier: String,
    pub placement_target: ExecutionTarget,
    pub priority: u32,
    pub readiness: ModelCandidateReadiness,
    pub rejection_code: Option<ModelCandidateRejectionCode>,
    pub reason: String,
    pub manifest_order: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SelectedModelCandidate {
    pub candidate_id: String,
    pub provider_capability_id: String,
    pub provider_implementation_id: String,
    pub model_identifier: String,
    pub placement_target: ExecutionTarget,
    pub priority: u32,
    pub selection_reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModelResolutionEvidence {
    pub phase: ModelResolutionPhase,
    pub interface_id: String,
    pub requested_interface_id: String,
    pub requested_placement: ExecutionTarget,
    pub selected: Option<SelectedModelCandidate>,
    pub candidates: Vec<ModelCandidateEvaluation>,
    pub failure_code: Option<ModelCandidateRejectionCode>,
}

impl ModelResolutionEvidence {
    #[must_use]
    pub fn is_ready(&self) -> bool {
        self.selected.is_some()
    }

    #[must_use]
    pub fn machine_failure_code(&self) -> Option<&'static str> {
        self.failure_code.map(ModelCandidateRejectionCode::as_str)
    }
}

#[must_use]
pub fn resolve_model_dependency(
    dependency: &ApplicationModelDependency,
    request: &ModelResolutionRequest,
    probe: &impl ModelAvailabilityProbe,
) -> ModelResolutionEvidence {
    let mut evaluations = Vec::new();
    let mut passing = Vec::new();

    for (manifest_order, candidate) in dependency.candidates.iter().enumerate() {
        let evaluation = evaluate_candidate(dependency, request, candidate, manifest_order, probe);
        if evaluation.readiness == ModelCandidateReadiness::Ready {
            passing.push(evaluation.clone());
        }
        evaluations.push(evaluation);
    }

    passing.sort_by(|left, right| {
        right
            .priority
            .cmp(&left.priority)
            .then_with(|| left.manifest_order.cmp(&right.manifest_order))
    });
    let selected = passing.first().map(|candidate| SelectedModelCandidate {
        candidate_id: candidate.candidate_id.clone(),
        provider_capability_id: candidate.provider_capability_id.clone(),
        provider_implementation_id: candidate.provider_implementation_id.clone(),
        model_identifier: candidate.model_identifier.clone(),
        placement_target: candidate.placement_target.clone(),
        priority: candidate.priority,
        selection_reason: if passing.len() > 1
            && passing[0].priority == passing.get(1).map_or(0, |next| next.priority)
        {
            "selected by priority tie using manifest order".to_string()
        } else {
            "selected highest-priority passing candidate".to_string()
        },
    });

    let failure_code = selected
        .is_none()
        .then_some(ModelCandidateRejectionCode::ModelDependencyUnsatisfied);

    ModelResolutionEvidence {
        phase: request.phase,
        interface_id: dependency.interface_id.clone(),
        requested_interface_id: request.requested_interface_id.clone(),
        requested_placement: request.requested_placement.clone(),
        selected,
        candidates: evaluations,
        failure_code,
    }
}

fn evaluate_candidate(
    dependency: &ApplicationModelDependency,
    request: &ModelResolutionRequest,
    candidate: &ModelCandidate,
    manifest_order: usize,
    probe: &impl ModelAvailabilityProbe,
) -> ModelCandidateEvaluation {
    let availability = probe.check_candidate(dependency, candidate);
    if let Some(code) = availability.failure_code {
        return rejected_candidate(
            candidate,
            manifest_order,
            code,
            availability
                .reason
                .unwrap_or_else(|| code.as_str().to_string()),
        );
    }
    if !availability.provider_available {
        return rejected_candidate(
            candidate,
            manifest_order,
            ModelCandidateRejectionCode::ModelProviderUnavailable,
            "provider unavailable",
        );
    }
    if !availability.model_available {
        return rejected_candidate(
            candidate,
            manifest_order,
            ModelCandidateRejectionCode::ModelCandidateUnavailable,
            "model unavailable",
        );
    }
    if candidate.provider_capability_id != request.requested_interface_id
        || candidate.provider_capability_id != dependency.interface_id
        || !supports_required_capabilities(&candidate.metadata, &dependency.required_capabilities)
    {
        return rejected_candidate(
            candidate,
            manifest_order,
            ModelCandidateRejectionCode::ModelInterfaceUnsupported,
            "candidate does not support requested inference interface",
        );
    }
    if candidate.placement_target != request.requested_placement {
        return rejected_candidate(
            candidate,
            manifest_order,
            ModelCandidateRejectionCode::ModelCandidateConfigInvalid,
            "candidate placement does not match requested placement policy",
        );
    }
    let context_window = candidate
        .metadata
        .get("model_context_window")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    if context_window < dependency.minimum_context_window {
        return rejected_candidate(
            candidate,
            manifest_order,
            ModelCandidateRejectionCode::ModelContextWindowInsufficient,
            format!(
                "model context window {context_window} is below required {}",
                dependency.minimum_context_window
            ),
        );
    }

    ModelCandidateEvaluation {
        candidate_id: candidate.candidate_id.clone(),
        provider_capability_id: candidate.provider_capability_id.clone(),
        provider_implementation_id: candidate.provider_implementation_id.clone(),
        model_identifier: candidate.model_identifier.clone(),
        placement_target: candidate.placement_target.clone(),
        priority: candidate.priority,
        readiness: ModelCandidateReadiness::Ready,
        rejection_code: None,
        reason: "candidate passed availability, interface, placement, and context checks"
            .to_string(),
        manifest_order,
    }
}

fn rejected_candidate(
    candidate: &ModelCandidate,
    manifest_order: usize,
    code: ModelCandidateRejectionCode,
    reason: impl Into<String>,
) -> ModelCandidateEvaluation {
    ModelCandidateEvaluation {
        candidate_id: candidate.candidate_id.clone(),
        provider_capability_id: candidate.provider_capability_id.clone(),
        provider_implementation_id: candidate.provider_implementation_id.clone(),
        model_identifier: candidate.model_identifier.clone(),
        placement_target: candidate.placement_target.clone(),
        priority: candidate.priority,
        readiness: ModelCandidateReadiness::Rejected,
        rejection_code: Some(code),
        reason: reason.into(),
        manifest_order,
    }
}

fn supports_required_capabilities(metadata: &Value, required: &[String]) -> bool {
    if required.is_empty() {
        return true;
    }
    let Some(capabilities) = metadata.get("capabilities").and_then(Value::as_array) else {
        return false;
    };
    required.iter().all(|required_capability| {
        capabilities
            .iter()
            .filter_map(Value::as_str)
            .any(|capability| capability == required_capability)
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use std::collections::BTreeMap;
    use traverse_contracts::ExecutionTarget;

    use super::*;
    use crate::ModelSelectionPolicy;

    #[test]
    fn selects_highest_priority_available_candidate() {
        let dependency = dependency(vec![
            candidate("lower", "llama3.2:3b", 10, 8192),
            candidate("higher", "mistral:7b", 20, 8192),
        ]);
        let evidence = resolve_model_dependency(&dependency, &setup_request(), &ReadyProbe);

        assert_eq!(
            evidence
                .selected
                .as_ref()
                .map(|selected| selected.candidate_id.as_str()),
            Some("higher")
        );
        assert!(evidence.is_ready());
        assert!(evidence.failure_code.is_none());
    }

    #[test]
    fn falls_back_to_lower_priority_passing_candidate() {
        let dependency = dependency(vec![
            candidate("preferred", "llama3.2:3b", 20, 8192),
            candidate("fallback", "mistral:7b", 10, 8192),
        ]);
        let evidence = resolve_model_dependency(
            &dependency,
            &setup_request(),
            &MapProbe::new([(
                "preferred",
                ModelCandidateAvailability::rejected(
                    ModelCandidateRejectionCode::ModelCandidateUnavailable,
                    "model missing",
                ),
            )]),
        );

        assert_eq!(
            evidence
                .selected
                .as_ref()
                .map(|selected| selected.candidate_id.as_str()),
            Some("fallback")
        );
        assert_eq!(
            evidence.candidates[0].rejection_code,
            Some(ModelCandidateRejectionCode::ModelCandidateUnavailable)
        );
    }

    #[test]
    fn reports_unsatisfied_when_no_candidate_passes() {
        let dependency = dependency(vec![
            candidate("missing", "llama3.2:3b", 20, 8192),
            candidate("small", "tiny:1b", 10, 4096),
        ]);
        let evidence = resolve_model_dependency(
            &dependency,
            &setup_request(),
            &MapProbe::new([(
                "missing",
                ModelCandidateAvailability::rejected(
                    ModelCandidateRejectionCode::ModelProviderUnavailable,
                    "provider stopped",
                ),
            )]),
        );

        assert!(evidence.selected.is_none());
        assert_eq!(
            evidence.machine_failure_code(),
            Some("model_dependency_unsatisfied")
        );
        assert_eq!(
            evidence.candidates[0].rejection_code,
            Some(ModelCandidateRejectionCode::ModelProviderUnavailable)
        );
        assert_eq!(
            evidence.candidates[1].rejection_code,
            Some(ModelCandidateRejectionCode::ModelContextWindowInsufficient)
        );
    }

    #[test]
    fn revalidates_execution_and_records_selection_change() {
        let dependency = dependency(vec![
            candidate("setup-choice", "llama3.2:3b", 20, 8192),
            candidate("execution-choice", "mistral:7b", 10, 8192),
        ]);
        let setup = resolve_model_dependency(&dependency, &setup_request(), &ReadyProbe);
        let execution = resolve_model_dependency(
            &dependency,
            &ModelResolutionRequest {
                phase: ModelResolutionPhase::Execution,
                requested_interface_id: "traverse.inference.generate".to_string(),
                requested_placement: ExecutionTarget::Local,
            },
            &MapProbe::new([(
                "setup-choice",
                ModelCandidateAvailability::rejected(
                    ModelCandidateRejectionCode::ModelProviderUnavailable,
                    "provider stopped after setup",
                ),
            )]),
        );

        assert_eq!(
            setup
                .selected
                .as_ref()
                .map(|selected| selected.candidate_id.as_str()),
            Some("setup-choice")
        );
        assert_eq!(
            execution
                .selected
                .as_ref()
                .map(|selected| selected.candidate_id.as_str()),
            Some("execution-choice")
        );
        assert_eq!(execution.phase, ModelResolutionPhase::Execution);
    }

    #[test]
    fn rejects_unsupported_interface_and_uses_manifest_order_for_priority_tie() {
        let mut unsupported = candidate("unsupported", "llama3.2:3b", 20, 8192);
        unsupported.provider_capability_id = "private.generate".to_string();
        let dependency = dependency(vec![
            unsupported,
            candidate("first-tie", "mistral:7b", 10, 8192),
            candidate("second-tie", "gemma:2b", 10, 8192),
        ]);
        let evidence = resolve_model_dependency(&dependency, &setup_request(), &ReadyProbe);

        assert_eq!(
            evidence.candidates[0].rejection_code,
            Some(ModelCandidateRejectionCode::ModelInterfaceUnsupported)
        );
        assert_eq!(
            evidence
                .selected
                .as_ref()
                .map(|selected| selected.candidate_id.as_str()),
            Some("first-tie")
        );
        assert_eq!(
            evidence
                .selected
                .as_ref()
                .map(|selected| selected.selection_reason.as_str()),
            Some("selected by priority tie using manifest order")
        );
    }

    #[test]
    fn covers_stable_codes_direct_availability_and_fit_guards() {
        assert_eq!(
            ModelCandidateRejectionCode::ModelProviderUnavailable.as_str(),
            "model_provider_unavailable"
        );
        assert_eq!(
            ModelCandidateRejectionCode::ModelCandidateUnavailable.as_str(),
            "model_candidate_unavailable"
        );
        assert_eq!(
            ModelCandidateRejectionCode::ModelInterfaceUnsupported.as_str(),
            "model_interface_unsupported"
        );
        assert_eq!(
            ModelCandidateRejectionCode::ModelContextWindowInsufficient.as_str(),
            "model_context_window_insufficient"
        );
        assert_eq!(
            ModelCandidateRejectionCode::ModelCandidateConfigInvalid.as_str(),
            "model_candidate_config_invalid"
        );
        assert_eq!(
            ModelCandidateRejectionCode::ModelDependencyUnsatisfied.as_str(),
            "model_dependency_unsatisfied"
        );

        let availability_dependency = dependency(vec![
            candidate("provider-bool", "llama3.2:3b", 30, 8192),
            candidate("model-bool", "mistral:7b", 20, 8192),
        ]);
        let evidence = resolve_model_dependency(
            &availability_dependency,
            &setup_request(),
            &MapProbe::new([
                (
                    "provider-bool",
                    ModelCandidateAvailability {
                        provider_available: false,
                        model_available: false,
                        failure_code: None,
                        reason: None,
                    },
                ),
                (
                    "model-bool",
                    ModelCandidateAvailability {
                        provider_available: true,
                        model_available: false,
                        failure_code: None,
                        reason: None,
                    },
                ),
            ]),
        );
        assert_eq!(
            evidence.candidates[0].rejection_code,
            Some(ModelCandidateRejectionCode::ModelProviderUnavailable)
        );
        assert_eq!(
            evidence.candidates[1].rejection_code,
            Some(ModelCandidateRejectionCode::ModelCandidateUnavailable)
        );

        let mut cloud = candidate("cloud", "llama3.2:3b", 10, 8192);
        cloud.placement_target = ExecutionTarget::Cloud;
        let placement =
            resolve_model_dependency(&dependency(vec![cloud]), &setup_request(), &ReadyProbe);
        assert_eq!(
            placement.candidates[0].rejection_code,
            Some(ModelCandidateRejectionCode::ModelCandidateConfigInvalid)
        );

        let mut no_capabilities = candidate("no-capabilities", "llama3.2:3b", 10, 8192);
        no_capabilities.metadata = json!({"model_context_window": 8192});
        let missing_capabilities = resolve_model_dependency(
            &dependency(vec![no_capabilities.clone()]),
            &setup_request(),
            &ReadyProbe,
        );
        assert_eq!(
            missing_capabilities.candidates[0].rejection_code,
            Some(ModelCandidateRejectionCode::ModelInterfaceUnsupported)
        );

        let mut no_required = dependency(vec![no_capabilities]);
        no_required.required_capabilities = Vec::new();
        let selected = resolve_model_dependency(&no_required, &setup_request(), &ReadyProbe);
        assert!(selected.is_ready());
    }

    #[derive(Debug, Clone, Copy)]
    struct ReadyProbe;

    impl ModelAvailabilityProbe for ReadyProbe {
        fn check_candidate(
            &self,
            _dependency: &ApplicationModelDependency,
            _candidate: &ModelCandidate,
        ) -> ModelCandidateAvailability {
            ModelCandidateAvailability::ready()
        }
    }

    #[derive(Debug, Clone)]
    struct MapProbe {
        failures: BTreeMap<String, ModelCandidateAvailability>,
    }

    impl MapProbe {
        fn new<const N: usize>(failures: [(&str, ModelCandidateAvailability); N]) -> Self {
            Self {
                failures: failures
                    .into_iter()
                    .map(|(candidate_id, availability)| (candidate_id.to_string(), availability))
                    .collect(),
            }
        }
    }

    impl ModelAvailabilityProbe for MapProbe {
        fn check_candidate(
            &self,
            _dependency: &ApplicationModelDependency,
            candidate: &ModelCandidate,
        ) -> ModelCandidateAvailability {
            self.failures
                .get(&candidate.candidate_id)
                .cloned()
                .unwrap_or_else(ModelCandidateAvailability::ready)
        }
    }

    fn setup_request() -> ModelResolutionRequest {
        ModelResolutionRequest {
            phase: ModelResolutionPhase::Setup,
            requested_interface_id: "traverse.inference.generate".to_string(),
            requested_placement: ExecutionTarget::Local,
        }
    }

    fn dependency(candidates: Vec<ModelCandidate>) -> ApplicationModelDependency {
        ApplicationModelDependency {
            interface_id: "traverse.inference.generate".to_string(),
            version_range: "^1.0".to_string(),
            selection_policy: ModelSelectionPolicy {
                strategy: "priority".to_string(),
                allow_fallback: true,
            },
            required_capabilities: vec!["text_generation".to_string()],
            minimum_context_window: 8192,
            candidates,
        }
    }

    fn candidate(
        candidate_id: &str,
        model_identifier: &str,
        priority: u32,
        context_window: u64,
    ) -> ModelCandidate {
        ModelCandidate {
            candidate_id: candidate_id.to_string(),
            provider_capability_id: "traverse.inference.generate".to_string(),
            provider_implementation_id: "ollama.local.generate".to_string(),
            model_identifier: model_identifier.to_string(),
            placement_target: ExecutionTarget::Local,
            priority,
            required_provider_config_keys: vec!["ollama_base_url".to_string()],
            metadata: json!({
                "implementation_kind": "real_local_provider",
                "provider": "ollama",
                "capabilities": ["text_generation"],
                "model_context_window": context_window
            }),
        }
    }
}
