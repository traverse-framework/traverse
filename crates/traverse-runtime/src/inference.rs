//! Governed local inference providers for Traverse.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;
use traverse_contracts::ExecutionTarget;
use traverse_registry::{
    ApplicationModelDependency, ModelAvailabilityProbe, ModelCandidate, ModelCandidateAvailability,
    ModelCandidateRejectionCode, ModelResolutionEvidence, ModelResolutionPhase,
    ModelResolutionRequest, resolve_model_dependency,
};

use crate::exact_model::{
    ExactModelHostConnector, ExactModelPin, decode_guest_frame, encode_guest_frame,
};
use crate::host_connector_dispatch::{
    HostConnectorHostRequest, HostConnectorPort, MODEL_EXECUTE_OPERATION, MODEL_RUNTIME_CONNECTOR,
};

const OLLAMA_PROVIDER: &str = "ollama";
const GENERATE_INTERFACE: &str = "traverse.inference.generate";
const DEFAULT_TIMEOUT_MS: u64 = 30_000;
/// Default cap on the buffered HTTP response from the (operator-configurable,
/// possibly untrusted) inference endpoint, guarding against unbounded memory
/// growth (spec 045-governed-model-dependency-resolution).
const DEFAULT_MAX_RESPONSE_BYTES: u64 = 8 * 1024 * 1024;

/// Spec 045/138 bridge (Decision 95): candidate kind delegating execution to
/// Spec 138's `model.execute` instead of a native provider daemon.
const EXACT_REF_PROVIDER: &str = "exact-ref.wasm-cpu";
/// Guest frame dtype for the bridge's opaque UTF-8 prompt/response payloads.
const EXACT_REF_TEXT_DTYPE: u8 = 4;
const EXACT_REF_STAGE_MAX_BYTES: usize = 1_048_576;
const EXACT_REF_MAX_OUTPUT_BYTES: usize = 65_536;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OllamaProviderConfig {
    pub base_url: String,
    #[serde(default)]
    pub request_timeout_ms: Option<u64>,
    /// Maximum number of response bytes read from the endpoint before the
    /// request is rejected. Defaults to [`DEFAULT_MAX_RESPONSE_BYTES`].
    #[serde(default)]
    pub max_response_bytes: Option<u64>,
}

impl OllamaProviderConfig {
    #[must_use]
    pub fn timeout(&self) -> Duration {
        Duration::from_millis(self.request_timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS))
    }

    #[must_use]
    pub fn max_response_bytes(&self) -> u64 {
        self.max_response_bytes
            .unwrap_or(DEFAULT_MAX_RESPONSE_BYTES)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OllamaInferenceRequest {
    pub model: String,
    pub prompt: String,
    #[serde(default)]
    pub system_prompt: Option<String>,
    #[serde(default)]
    pub options: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenerateOutput {
    pub interface_id: String,
    pub provider: String,
    pub provider_implementation_id: String,
    pub model: String,
    pub response: String,
    pub done: bool,
    pub evidence: GenerateEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenerateEvidence {
    pub placement_target: String,
    pub selected_provider: String,
    pub selected_model: String,
    /// Spec 045/138 bridge (Decision 95, FR-012a): distinguishes a
    /// digest-verified exact-ref package execution from a local, unsigned
    /// daemon execution, regardless of which candidate kind was selected.
    pub trust_class: InferenceTrustClass,
}

/// Spec 045/138 bridge (Decision 95, FR-012a).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InferenceTrustClass {
    /// The response came from a Spec 138 exact-ref signed WASM package,
    /// digest-verified before execution.
    VerifiedSignedPackage,
    /// The response came from a local, unsigned provider daemon (Ollama).
    LocalUnverifiedDaemon,
}

impl InferenceTrustClass {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::VerifiedSignedPackage => "verified_signed_package",
            Self::LocalUnverifiedDaemon => "local_unverified_daemon",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GovernedModelExecutionRequest {
    pub interface_id: String,
    pub prompt: String,
    #[serde(default)]
    pub system_prompt: Option<String>,
    #[serde(default)]
    pub options: Value,
    pub requested_placement: ExecutionTarget,
    #[serde(default)]
    pub provider_configs: BTreeMap<String, OllamaProviderConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GovernedModelExecutionOutcome {
    pub output: GenerateOutput,
    pub model_resolution: ModelResolutionEvidence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GovernedModelExecutionErrorCode {
    InterfaceNotDeclared,
    ModelDependencyUnsatisfied,
    ProviderExecutionFailed,
}

impl GovernedModelExecutionErrorCode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InterfaceNotDeclared => "model_interface_not_declared",
            Self::ModelDependencyUnsatisfied => "model_dependency_unsatisfied",
            Self::ProviderExecutionFailed => "model_provider_failure",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GovernedModelExecutionError {
    pub code: GovernedModelExecutionErrorCode,
    pub message: String,
    pub model_resolution: Option<Box<ModelResolutionEvidence>>,
}

impl GovernedModelExecutionError {
    #[must_use]
    pub fn new(code: GovernedModelExecutionErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            model_resolution: None,
        }
    }

    #[must_use]
    pub fn with_model_resolution(mut self, evidence: ModelResolutionEvidence) -> Self {
        self.model_resolution = Some(Box::new(evidence));
        self
    }
}

impl fmt::Display for GovernedModelExecutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for GovernedModelExecutionError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OllamaInferenceProvider {
    config: OllamaProviderConfig,
}

impl OllamaInferenceProvider {
    /// Creates a local Ollama provider backed by the configured HTTP endpoint.
    ///
    /// # Errors
    ///
    /// Returns [`OllamaInferenceError`] when the endpoint config is invalid.
    pub fn new(config: OllamaProviderConfig) -> Result<Self, OllamaInferenceError> {
        parse_base_url(&config.base_url)?;
        Ok(Self { config })
    }

    fn from_validated_config(config: OllamaProviderConfig) -> Self {
        Self { config }
    }

    #[must_use]
    pub fn provider_implementation_id(&self) -> &'static str {
        "ollama.local.generate"
    }

    /// Checks whether the configured Ollama endpoint lists the requested model.
    ///
    /// # Errors
    ///
    /// Returns [`OllamaInferenceError`] when the provider is unavailable, the
    /// response is invalid, or the model is not listed by Ollama.
    pub fn check_model_available(&self, model: &str) -> Result<(), OllamaInferenceError> {
        if model.trim().is_empty() {
            return Err(OllamaInferenceError::new(
                OllamaInferenceErrorCode::InvalidConfig,
                "model_identifier is required",
            ));
        }

        let response = self.request("GET", "/api/tags", None)?;
        let models = response
            .get("models")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                OllamaInferenceError::new(
                    OllamaInferenceErrorCode::InvalidResponse,
                    "Ollama tags response must contain models array",
                )
            })?;

        if models.iter().any(|entry| model_entry_matches(entry, model)) {
            return Ok(());
        }

        Err(OllamaInferenceError::new(
            OllamaInferenceErrorCode::ModelUnavailable,
            format!("Ollama model {model} is not installed"),
        ))
    }

    /// Invokes real local generation through Ollama.
    ///
    /// # Errors
    ///
    /// Returns [`OllamaInferenceError`] when config, provider availability,
    /// model availability, provider execution, or response validation fails.
    pub fn generate(
        &self,
        request: &OllamaInferenceRequest,
    ) -> Result<GenerateOutput, OllamaInferenceError> {
        validate_generate_request(request)?;
        self.check_model_available(&request.model)?;

        let mut body = Map::new();
        body.insert("model".to_string(), json!(request.model));
        body.insert("prompt".to_string(), json!(request.prompt));
        body.insert("stream".to_string(), json!(false));
        if let Some(system_prompt) = &request.system_prompt {
            body.insert("system".to_string(), json!(system_prompt));
        }
        if !request.options.is_null() {
            body.insert("options".to_string(), request.options.clone());
        }

        let response = self.request("POST", "/api/generate", Some(Value::Object(body)))?;
        parse_generate_response(self.provider_implementation_id(), &request.model, &response)
    }

    fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value, OllamaInferenceError> {
        let endpoint = parse_base_url(&self.config.base_url)?;
        let body_text = body.map_or_else(String::new, |value| value.to_string());
        let request_path = endpoint.path_for(path);
        let response_text = send_http_json(
            &endpoint.host,
            endpoint.port,
            &request_path,
            method,
            &body_text,
            self.config.timeout(),
            self.config.max_response_bytes(),
        )?;
        parse_http_json_response(&response_text)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OllamaModelAvailabilityProbe {
    configs_by_implementation: BTreeMap<String, OllamaProviderConfig>,
}

impl OllamaModelAvailabilityProbe {
    #[must_use]
    pub fn new(config: OllamaProviderConfig) -> Self {
        let mut configs_by_implementation = BTreeMap::new();
        configs_by_implementation.insert("ollama.local.generate".to_string(), config);
        Self {
            configs_by_implementation,
        }
    }

    #[must_use]
    pub fn with_provider_config(
        mut self,
        provider_implementation_id: impl Into<String>,
        config: OllamaProviderConfig,
    ) -> Self {
        self.configs_by_implementation
            .insert(provider_implementation_id.into(), config);
        self
    }
}

impl ModelAvailabilityProbe for OllamaModelAvailabilityProbe {
    fn check_candidate(
        &self,
        _dependency: &ApplicationModelDependency,
        candidate: &ModelCandidate,
    ) -> ModelCandidateAvailability {
        let Some(config) = self
            .configs_by_implementation
            .get(&candidate.provider_implementation_id)
        else {
            return ModelCandidateAvailability::rejected(
                ModelCandidateRejectionCode::ModelCandidateConfigInvalid,
                "missing provider config for model candidate",
            );
        };
        let provider = match OllamaInferenceProvider::new(config.clone()) {
            Ok(provider) => provider,
            Err(error) => return model_candidate_availability_error(&error),
        };
        match provider.check_model_available(&candidate.model_identifier) {
            Ok(()) => ModelCandidateAvailability::ready(),
            Err(error) => model_candidate_availability_error(&error),
        }
    }
}

#[must_use]
pub fn resolve_ollama_model_dependency(
    dependency: &ApplicationModelDependency,
    request: &ModelResolutionRequest,
    probe: &OllamaModelAvailabilityProbe,
) -> ModelResolutionEvidence {
    resolve_model_dependency(dependency, request, probe)
}

/// Resolves and executes one app-declared model dependency through Traverse.
///
/// The caller supplies runtime-local provider configuration, while the selected
/// provider/model must come from the registered app dependency declaration.
///
/// # Errors
///
/// Returns [`GovernedModelExecutionError`] when the dependency does not match
/// the requested interface, no model candidate can be selected, selected
/// provider config is unavailable, or real provider execution fails.
pub fn execute_governed_ollama_model_dependency(
    dependency: &ApplicationModelDependency,
    request: &GovernedModelExecutionRequest,
) -> Result<GovernedModelExecutionOutcome, GovernedModelExecutionError> {
    if dependency.interface_id != request.interface_id {
        return Err(GovernedModelExecutionError::new(
            GovernedModelExecutionErrorCode::InterfaceNotDeclared,
            "requested inference interface is not declared by this app dependency",
        ));
    }

    let probe = request.provider_configs.iter().fold(
        OllamaModelAvailabilityProbe::default(),
        |probe, (implementation_id, config)| {
            probe.with_provider_config(implementation_id.clone(), config.clone())
        },
    );
    let resolution_request = ModelResolutionRequest {
        phase: ModelResolutionPhase::Execution,
        requested_interface_id: request.interface_id.clone(),
        requested_placement: request.requested_placement.clone(),
    };
    let evidence = resolve_ollama_model_dependency(dependency, &resolution_request, &probe);
    let Some(selected) = evidence.selected.as_ref() else {
        return Err(GovernedModelExecutionError::new(
            GovernedModelExecutionErrorCode::ModelDependencyUnsatisfied,
            "no app-declared model candidate satisfied execution-time resolution",
        )
        .with_model_resolution(evidence));
    };
    let provider = OllamaInferenceProvider::from_validated_config(
        request.provider_configs[&selected.provider_implementation_id].clone(),
    );
    let output = provider
        .generate(&OllamaInferenceRequest {
            model: selected.model_identifier.clone(),
            prompt: request.prompt.clone(),
            system_prompt: request.system_prompt.clone(),
            options: request.options.clone(),
        })
        .map_err(|error| {
            GovernedModelExecutionError::new(
                GovernedModelExecutionErrorCode::ProviderExecutionFailed,
                error.to_string(),
            )
            .with_model_resolution(evidence.clone())
        })?;

    Ok(GovernedModelExecutionOutcome {
        output,
        model_resolution: evidence,
    })
}

/// Spec 045/138 bridge (Decision 95): resolves a model dependency whose
/// candidates may be `ollama.local.generate` or `exact-ref.wasm-cpu`, and
/// executes whichever kind is selected. Exact-ref execution is delegated
/// entirely to `138-governed-exact-model-execution`'s `model.execute`; this
/// function only wires the prompt/response bytes through that existing path.
///
/// # Errors
///
/// Returns [`GovernedModelExecutionError`] under the same conditions as
/// [`execute_governed_ollama_model_dependency`], plus provider execution
/// failure when the selected exact-ref candidate's referenced pin is missing
/// or `model.execute` itself fails.
pub fn execute_governed_bridged_model_dependency(
    dependency: &ApplicationModelDependency,
    request: &GovernedModelExecutionRequest,
    exact_model_pins: &[ExactModelPin],
    activated_exact_ref_pins: &BTreeSet<(String, String)>,
    exact_model_host: &mut ExactModelHostConnector,
    exact_ref_policy_ref: &str,
    exact_ref_data_classification: &str,
) -> Result<GovernedModelExecutionOutcome, GovernedModelExecutionError> {
    if dependency.interface_id != request.interface_id {
        return Err(GovernedModelExecutionError::new(
            GovernedModelExecutionErrorCode::InterfaceNotDeclared,
            "requested inference interface is not declared by this app dependency",
        ));
    }

    let ollama_probe = request.provider_configs.iter().fold(
        OllamaModelAvailabilityProbe::default(),
        |probe, (implementation_id, config)| {
            probe.with_provider_config(implementation_id.clone(), config.clone())
        },
    );
    let probe = BridgedModelAvailabilityProbe::new(ollama_probe, activated_exact_ref_pins.clone());
    let resolution_request = ModelResolutionRequest {
        phase: ModelResolutionPhase::Execution,
        requested_interface_id: request.interface_id.clone(),
        requested_placement: request.requested_placement.clone(),
    };
    let evidence = resolve_model_dependency(dependency, &resolution_request, &probe);
    let Some(selected) = evidence.selected.as_ref() else {
        return Err(GovernedModelExecutionError::new(
            GovernedModelExecutionErrorCode::ModelDependencyUnsatisfied,
            "no app-declared model candidate satisfied execution-time resolution",
        )
        .with_model_resolution(evidence));
    };

    if selected.provider_implementation_id == EXACT_REF_PROVIDER {
        // `selected.candidate_id` always names an entry in `dependency.candidates`:
        // resolution derives it by evaluating this same list, never any other
        // source. Direct indexing mirrors the existing
        // `request.provider_configs[&selected.provider_implementation_id]`
        // access below for the same reason.
        let candidates_by_id: BTreeMap<&str, &ModelCandidate> = dependency
            .candidates
            .iter()
            .map(|candidate| (candidate.candidate_id.as_str(), candidate))
            .collect();
        let candidate = candidates_by_id[selected.candidate_id.as_str()];
        let output = execute_exact_ref_candidate(
            candidate,
            request,
            exact_model_pins,
            exact_model_host,
            exact_ref_policy_ref,
            exact_ref_data_classification,
        )
        .map_err(|message| {
            GovernedModelExecutionError::new(
                GovernedModelExecutionErrorCode::ProviderExecutionFailed,
                message,
            )
            .with_model_resolution(evidence.clone())
        })?;
        return Ok(GovernedModelExecutionOutcome {
            output,
            model_resolution: evidence,
        });
    }

    let provider = OllamaInferenceProvider::from_validated_config(
        request.provider_configs[&selected.provider_implementation_id].clone(),
    );
    let output = provider
        .generate(&OllamaInferenceRequest {
            model: selected.model_identifier.clone(),
            prompt: request.prompt.clone(),
            system_prompt: request.system_prompt.clone(),
            options: request.options.clone(),
        })
        .map_err(|error| {
            GovernedModelExecutionError::new(
                GovernedModelExecutionErrorCode::ProviderExecutionFailed,
                error.to_string(),
            )
            .with_model_resolution(evidence.clone())
        })?;

    Ok(GovernedModelExecutionOutcome {
        output,
        model_resolution: evidence,
    })
}

/// Spec 045/138 bridge (Decision 95): evaluates both `ollama.local.generate`
/// and `exact-ref.wasm-cpu` candidates in one resolution pass.
#[derive(Debug, Clone)]
pub struct BridgedModelAvailabilityProbe {
    ollama: OllamaModelAvailabilityProbe,
    activated_exact_ref_pins: BTreeSet<(String, String)>,
}

impl BridgedModelAvailabilityProbe {
    #[must_use]
    pub fn new(
        ollama: OllamaModelAvailabilityProbe,
        activated_exact_ref_pins: BTreeSet<(String, String)>,
    ) -> Self {
        Self {
            ollama,
            activated_exact_ref_pins,
        }
    }
}

impl ModelAvailabilityProbe for BridgedModelAvailabilityProbe {
    fn check_candidate(
        &self,
        dependency: &ApplicationModelDependency,
        candidate: &ModelCandidate,
    ) -> ModelCandidateAvailability {
        if candidate.provider_implementation_id == EXACT_REF_PROVIDER {
            return check_exact_ref_candidate_availability(
                candidate,
                &self.activated_exact_ref_pins,
            );
        }
        self.ollama.check_candidate(dependency, candidate)
    }
}

/// FR-002a: an `exact-ref.wasm-cpu` candidate is available only when its
/// referenced Spec 044 pin (by `model_id`+`version`, carried in
/// `metadata.exact_ref_model_id`/`metadata.exact_ref_version`) has an
/// activated `traverse.model-runtime` binding. This does not check whether
/// the underlying signed package bytes are physically cached — that is
/// re-verified fail-closed by `model.execute` itself at invocation time.
fn check_exact_ref_candidate_availability(
    candidate: &ModelCandidate,
    activated_exact_ref_pins: &BTreeSet<(String, String)>,
) -> ModelCandidateAvailability {
    let Some(model_id) = candidate
        .metadata
        .get("exact_ref_model_id")
        .and_then(Value::as_str)
    else {
        return ModelCandidateAvailability::rejected(
            ModelCandidateRejectionCode::ModelCandidateConfigInvalid,
            "exact-ref candidate is missing exact_ref_model_id metadata",
        );
    };
    let Some(version) = candidate
        .metadata
        .get("exact_ref_version")
        .and_then(Value::as_str)
    else {
        return ModelCandidateAvailability::rejected(
            ModelCandidateRejectionCode::ModelCandidateConfigInvalid,
            "exact-ref candidate is missing exact_ref_version metadata",
        );
    };
    if activated_exact_ref_pins.contains(&(model_id.to_string(), version.to_string())) {
        ModelCandidateAvailability::ready()
    } else {
        ModelCandidateAvailability::rejected(
            ModelCandidateRejectionCode::ModelProviderUnavailable,
            "referenced exact_model_dependencies pin has no activated traverse.model-runtime binding",
        )
    }
}

/// Encodes the prompt, stages it, invokes `model.execute` on the referenced
/// exact-ref pin, and decodes the response frame's payload as UTF-8 text.
fn execute_exact_ref_candidate(
    candidate: &ModelCandidate,
    request: &GovernedModelExecutionRequest,
    exact_model_pins: &[ExactModelPin],
    host: &mut ExactModelHostConnector,
    policy_ref: &str,
    data_classification: &str,
) -> Result<GenerateOutput, String> {
    let model_id = candidate
        .metadata
        .get("exact_ref_model_id")
        .and_then(Value::as_str)
        .ok_or("exact-ref candidate is missing exact_ref_model_id metadata")?;
    let version = candidate
        .metadata
        .get("exact_ref_version")
        .and_then(Value::as_str)
        .ok_or("exact-ref candidate is missing exact_ref_version metadata")?;
    let input_schema_ref = candidate
        .metadata
        .get("exact_ref_input_schema_ref")
        .and_then(Value::as_str)
        .unwrap_or("schema:bridged-generate-in");
    let input_schema_version = candidate
        .metadata
        .get("exact_ref_input_schema_version")
        .and_then(Value::as_str)
        .unwrap_or("1.0.0");
    let pin = exact_model_pins
        .iter()
        .find(|pin| pin.model_id == model_id && pin.version == version)
        .ok_or_else(|| format!("no exact_model_dependencies pin found for {model_id}@{version}"))?;

    let prompt_bytes = request.prompt.as_bytes();
    let dims = [u32::try_from(prompt_bytes.len()).unwrap_or(u32::MAX)];
    let frame = encode_guest_frame(EXACT_REF_TEXT_DTYPE, &dims, prompt_bytes);
    let input_ref = host
        .io
        .stage_model_input(&frame, EXACT_REF_STAGE_MAX_BYTES)
        .map_err(|error| error.to_string())?;

    let host_request = HostConnectorHostRequest {
        connector_id: MODEL_RUNTIME_CONNECTOR.to_string(),
        operation: MODEL_EXECUTE_OPERATION.to_string(),
        binding_id: "bridged-exact-ref".to_string(),
        target_family: format!("{:?}", request.requested_placement),
        correlation_id: format!("bridge-{model_id}-{version}"),
        payload: json!({
            "model_ref": {
                "model_id": pin.model_id,
                "version": pin.version,
                "digest": pin.digest,
            },
            "input_ref": input_ref,
            "policy_ref": policy_ref,
            "data_classification": data_classification,
            "input_schema_ref": input_schema_ref,
            "input_schema_version": input_schema_version,
            "max_output_bytes": EXACT_REF_MAX_OUTPUT_BYTES,
        }),
        cancel_requested: false,
    };
    let result = host
        .invoke(&host_request)
        .map_err(|error| error.to_string())?;
    let output_ref = result
        .artifact_ref
        .as_deref()
        .ok_or_else(|| "exact-ref host returned no artifact_ref".to_string())?;
    let output_bytes = host
        .io
        .read_model_output(output_ref, EXACT_REF_MAX_OUTPUT_BYTES)
        .map_err(|error| error.to_string())?;
    let (_, _, payload) = decode_guest_frame(&output_bytes).map_err(|error| error.to_string())?;
    let text = String::from_utf8(payload)
        .map_err(|error| format!("exact-ref response was not valid UTF-8: {error}"))?;
    let model_display = format!("{model_id}@{version}");

    Ok(GenerateOutput {
        interface_id: GENERATE_INTERFACE.to_string(),
        provider: "exact-ref".to_string(),
        provider_implementation_id: EXACT_REF_PROVIDER.to_string(),
        model: model_display.clone(),
        response: text,
        done: true,
        evidence: GenerateEvidence {
            placement_target: format!("{:?}", request.requested_placement),
            selected_provider: "exact-ref".to_string(),
            selected_model: model_display,
            trust_class: InferenceTrustClass::VerifiedSignedPackage,
        },
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OllamaInferenceErrorCode {
    InvalidConfig,
    ModelProviderUnavailable,
    ModelUnavailable,
    ProviderFailure,
    InvalidResponse,
}

impl OllamaInferenceErrorCode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidConfig => "model_candidate_config_invalid",
            Self::ModelProviderUnavailable => "model_provider_unavailable",
            Self::ModelUnavailable => "model_candidate_unavailable",
            Self::ProviderFailure => "model_provider_failure",
            Self::InvalidResponse => "model_provider_invalid_response",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OllamaInferenceError {
    pub code: OllamaInferenceErrorCode,
    pub message: String,
}

impl OllamaInferenceError {
    #[must_use]
    pub fn new(code: OllamaInferenceErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    #[must_use]
    pub fn machine_code(&self) -> &'static str {
        self.code.as_str()
    }
}

impl fmt::Display for OllamaInferenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.machine_code(), self.message)
    }
}

impl std::error::Error for OllamaInferenceError {}

impl From<std::io::Error> for OllamaInferenceError {
    fn from(error: std::io::Error) -> Self {
        provider_unavailable(format!("Ollama I/O failed: {error}"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HttpEndpoint {
    host: String,
    port: u16,
    base_path: String,
}

impl HttpEndpoint {
    fn path_for(&self, path: &str) -> String {
        let base = self.base_path.trim_end_matches('/');
        let suffix = path.trim_start_matches('/');
        if base.is_empty() {
            format!("/{suffix}")
        } else {
            format!("{base}/{suffix}")
        }
    }
}

fn validate_generate_request(request: &OllamaInferenceRequest) -> Result<(), OllamaInferenceError> {
    if request.model.trim().is_empty() {
        return Err(OllamaInferenceError::new(
            OllamaInferenceErrorCode::InvalidConfig,
            "model_identifier is required",
        ));
    }
    if request.prompt.trim().is_empty() {
        return Err(OllamaInferenceError::new(
            OllamaInferenceErrorCode::InvalidConfig,
            "prompt is required for traverse.inference.generate",
        ));
    }
    if !request.options.is_null() && !request.options.is_object() {
        return Err(OllamaInferenceError::new(
            OllamaInferenceErrorCode::InvalidConfig,
            "options must be a JSON object when provided",
        ));
    }
    Ok(())
}

fn parse_generate_response(
    provider_implementation_id: &str,
    requested_model: &str,
    response: &Value,
) -> Result<GenerateOutput, OllamaInferenceError> {
    let text = response
        .get("response")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            OllamaInferenceError::new(
                OllamaInferenceErrorCode::InvalidResponse,
                "Ollama generate response must contain response text",
            )
        })?;
    let done = response
        .get("done")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let model = response
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or(requested_model);

    Ok(GenerateOutput {
        interface_id: GENERATE_INTERFACE.to_string(),
        provider: OLLAMA_PROVIDER.to_string(),
        provider_implementation_id: provider_implementation_id.to_string(),
        model: model.to_string(),
        response: text.to_string(),
        done,
        evidence: GenerateEvidence {
            placement_target: "local".to_string(),
            selected_provider: OLLAMA_PROVIDER.to_string(),
            selected_model: model.to_string(),
            trust_class: InferenceTrustClass::LocalUnverifiedDaemon,
        },
    })
}

fn model_entry_matches(entry: &Value, model: &str) -> bool {
    entry
        .get("name")
        .or_else(|| entry.get("model"))
        .and_then(Value::as_str)
        .is_some_and(|name| name == model)
}

fn parse_base_url(base_url: &str) -> Result<HttpEndpoint, OllamaInferenceError> {
    let trimmed = base_url.trim();
    let remainder = trimmed.strip_prefix("http://").ok_or_else(|| {
        OllamaInferenceError::new(
            OllamaInferenceErrorCode::InvalidConfig,
            "ollama_base_url must use http:// for local Ollama",
        )
    })?;
    let remainder = remainder.trim_end_matches('/');
    let (authority, path) = remainder.split_once('/').unwrap_or((remainder, ""));
    if authority.is_empty() {
        return Err(OllamaInferenceError::new(
            OllamaInferenceErrorCode::InvalidConfig,
            "ollama_base_url must include host",
        ));
    }

    let (host, port) = parse_authority(authority)?;
    Ok(HttpEndpoint {
        host,
        port,
        base_path: format!("/{path}").trim_end_matches('/').to_string(),
    })
}

fn parse_authority(authority: &str) -> Result<(String, u16), OllamaInferenceError> {
    let (host, port) = if let Some((host, port_text)) = authority.rsplit_once(':') {
        let port = port_text.parse::<u16>().map_err(|error| {
            OllamaInferenceError::new(
                OllamaInferenceErrorCode::InvalidConfig,
                format!("ollama_base_url port is invalid: {error}"),
            )
        })?;
        (host.to_string(), port)
    } else {
        (authority.to_string(), 11_434)
    };

    if host.is_empty() {
        return Err(OllamaInferenceError::new(
            OllamaInferenceErrorCode::InvalidConfig,
            "ollama_base_url host is required",
        ));
    }

    Ok((host, port))
}

fn send_http_json(
    host: &str,
    port: u16,
    path: &str,
    method: &str,
    body: &str,
    timeout: Duration,
    max_response_bytes: u64,
) -> Result<String, OllamaInferenceError> {
    let address = format!("{host}:{port}");
    let socket_address = address
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| provider_unavailable("Ollama endpoint did not resolve"))?;
    let mut stream = TcpStream::connect_timeout(&socket_address, timeout)?;
    // Bound the whole exchange, not just connect: a peer that accepts and then
    // stalls (or never sends EOF) would otherwise hang the thread forever
    // despite request_timeout_ms (spec 045-governed-model-dependency-resolution).
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host}:{port}\r\nContent-Type: application/json\r\nAccept: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes())?;

    read_capped_response(&mut stream, max_response_bytes)
}

/// Read the response with an upper size bound so an endpoint that streams
/// unbounded data cannot drive the host out of memory. The read is one byte
/// over the cap to distinguish "exactly at cap" from "over cap".
fn read_capped_response(
    stream: &mut TcpStream,
    max_response_bytes: u64,
) -> Result<String, OllamaInferenceError> {
    let mut buffer = Vec::new();
    let limit = max_response_bytes.saturating_add(1);
    stream.take(limit).read_to_end(&mut buffer)?;
    if buffer.len() as u64 > max_response_bytes {
        return Err(OllamaInferenceError::new(
            OllamaInferenceErrorCode::InvalidResponse,
            format!("Ollama response exceeded the {max_response_bytes}-byte limit"),
        ));
    }
    String::from_utf8(buffer).map_err(|error| {
        OllamaInferenceError::new(
            OllamaInferenceErrorCode::InvalidResponse,
            format!("Ollama response was not valid UTF-8: {error}"),
        )
    })
}

fn parse_http_json_response(response: &str) -> Result<Value, OllamaInferenceError> {
    let (head, body) = response.split_once("\r\n\r\n").ok_or_else(|| {
        OllamaInferenceError::new(
            OllamaInferenceErrorCode::InvalidResponse,
            "Ollama HTTP response is missing header separator",
        )
    })?;
    let status_line = head.lines().next().ok_or_else(|| {
        OllamaInferenceError::new(
            OllamaInferenceErrorCode::InvalidResponse,
            "Ollama HTTP response is missing status line",
        )
    })?;
    let status = parse_status_code(status_line)?;
    if !(200..300).contains(&status) {
        return Err(OllamaInferenceError::new(
            OllamaInferenceErrorCode::ProviderFailure,
            format!("Ollama returned HTTP {status}"),
        ));
    }

    serde_json::from_str(body).map_err(|error| {
        OllamaInferenceError::new(
            OllamaInferenceErrorCode::InvalidResponse,
            format!("Ollama response body is not valid JSON: {error}"),
        )
    })
}

fn parse_status_code(status_line: &str) -> Result<u16, OllamaInferenceError> {
    status_line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| {
            OllamaInferenceError::new(
                OllamaInferenceErrorCode::InvalidResponse,
                "Ollama HTTP status line is malformed",
            )
        })?
        .parse::<u16>()
        .map_err(|error| {
            OllamaInferenceError::new(
                OllamaInferenceErrorCode::InvalidResponse,
                format!("Ollama HTTP status code is invalid: {error}"),
            )
        })
}

fn provider_unavailable(message: impl Into<String>) -> OllamaInferenceError {
    OllamaInferenceError::new(OllamaInferenceErrorCode::ModelProviderUnavailable, message)
}

fn model_candidate_availability_error(error: &OllamaInferenceError) -> ModelCandidateAvailability {
    match error.code {
        OllamaInferenceErrorCode::InvalidConfig => ModelCandidateAvailability::rejected(
            ModelCandidateRejectionCode::ModelCandidateConfigInvalid,
            error.message.clone(),
        ),
        OllamaInferenceErrorCode::ModelProviderUnavailable => ModelCandidateAvailability::rejected(
            ModelCandidateRejectionCode::ModelProviderUnavailable,
            error.message.clone(),
        ),
        OllamaInferenceErrorCode::ModelUnavailable => ModelCandidateAvailability::rejected(
            ModelCandidateRejectionCode::ModelCandidateUnavailable,
            error.message.clone(),
        ),
        OllamaInferenceErrorCode::ProviderFailure | OllamaInferenceErrorCode::InvalidResponse => {
            ModelCandidateAvailability::rejected(
                ModelCandidateRejectionCode::ModelProviderUnavailable,
                error.message.clone(),
            )
        }
    }
}
