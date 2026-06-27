#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use serde_json::json;
use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::thread;
use traverse_contracts::{
    ExecutionTarget, governed_content_digest, parse_contract, reference_connector_contracts,
};
use traverse_registry::{
    ApplicationModelDependency, ArtifactDigests, BinaryFormat, BinaryReference,
    CapabilityArtifactRecord, CapabilityRegistration, CapabilityRegistry, ComposabilityMetadata,
    CompositionKind, CompositionPattern, ConnectorRegistration, ImplementationKind, ModelCandidate,
    ModelCandidateRejectionCode, ModelResolutionPhase, ModelResolutionRequest,
    ModelSelectionPolicy, RegistryProvenance, RegistryScope, SourceKind, SourceReference,
};
use traverse_runtime::inference::{
    GovernedModelExecutionError, GovernedModelExecutionErrorCode, GovernedModelExecutionRequest,
    OllamaInferenceErrorCode, OllamaInferenceProvider, OllamaInferenceRequest,
    OllamaModelAvailabilityProbe, OllamaProviderConfig, execute_governed_ollama_model_dependency,
    resolve_ollama_model_dependency,
};

#[test]
fn ollama_provider_generates_real_response_through_local_http_endpoint() {
    let base_url = start_ollama_server(vec![
        json!({"models": [{"name": "llama3.2:3b"}]}).to_string(),
        json!({"model": "llama3.2:3b", "response": "readiness looks good", "done": true})
            .to_string(),
    ]);
    let provider = provider(&base_url);

    let output = provider
        .generate(&OllamaInferenceRequest {
            model: "llama3.2:3b".to_string(),
            prompt: "Summarize readiness.".to_string(),
            system_prompt: Some("Be concise.".to_string()),
            options: json!({"temperature": 0}),
        })
        .expect("real local server should return generation output");

    assert_eq!(output.interface_id, "traverse.inference.generate");
    assert_eq!(output.provider, "ollama");
    assert_eq!(output.provider_implementation_id, "ollama.local.generate");
    assert_eq!(output.model, "llama3.2:3b");
    assert_eq!(output.response, "readiness looks good");
    assert!(output.done);
    assert_eq!(output.evidence.selected_provider, "ollama");
    assert_eq!(output.evidence.selected_model, "llama3.2:3b");
}

#[test]
fn ollama_provider_reports_model_unavailable_before_generation() {
    let base_url = start_ollama_server(vec![
        json!({"models": [{"name": "mistral:7b"}]}).to_string(),
    ]);
    let provider = provider(&base_url);

    let failure = provider
        .generate(&OllamaInferenceRequest {
            model: "llama3.2:3b".to_string(),
            prompt: "Summarize readiness.".to_string(),
            system_prompt: None,
            options: json!({}),
        })
        .expect_err("missing model must fail before generate call");

    assert_eq!(failure.code, OllamaInferenceErrorCode::ModelUnavailable);
    assert_eq!(failure.machine_code(), "model_candidate_unavailable");
}

#[test]
fn ollama_provider_accepts_model_field_from_tags_and_fallback_generate_fields() {
    let base_url = start_ollama_server(vec![
        json!({"models": [{"model": "llama3.2:3b"}]}).to_string(),
        json!({"response": "fallback fields work"}).to_string(),
    ]);
    let provider = provider(&base_url);

    let output = provider
        .generate(&OllamaInferenceRequest {
            model: "llama3.2:3b".to_string(),
            prompt: "Summarize readiness.".to_string(),
            system_prompt: None,
            options: serde_json::Value::Null,
        })
        .expect("model-key tags and missing optional generate fields should work");

    assert_eq!(output.model, "llama3.2:3b");
    assert_eq!(output.response, "fallback fields work");
    assert!(!output.done);
}

#[test]
fn ollama_provider_supports_base_path_and_default_port_config() {
    let base_url = start_ollama_server(vec![
        json!({"models": [{"name": "llama3.2:3b"}]}).to_string(),
    ]);
    let provider = provider(&format!("{base_url}/ollama"));

    provider
        .check_model_available("llama3.2:3b")
        .expect("base path endpoint should still call tags");

    let default_port_provider = provider_with_timeout("http://127.0.0.1", 100);
    assert_eq!(
        default_port_provider.provider_implementation_id(),
        "ollama.local.generate"
    );
}

#[test]
fn ollama_provider_reports_invalid_tags_response() {
    let base_url = start_ollama_server(vec![json!({"models": null}).to_string()]);
    let provider = provider(&base_url);

    let failure = provider
        .check_model_available("llama3.2:3b")
        .expect_err("missing models array must fail");

    assert_eq!(failure.code, OllamaInferenceErrorCode::InvalidResponse);
    assert_eq!(failure.machine_code(), "model_provider_invalid_response");
}

#[test]
fn ollama_provider_reports_invalid_generate_response() {
    let base_url = start_ollama_server(vec![
        json!({"models": [{"name": "llama3.2:3b"}]}).to_string(),
        json!({"model": "llama3.2:3b", "done": true}).to_string(),
    ]);
    let provider = provider(&base_url);

    let failure = provider
        .generate(&OllamaInferenceRequest {
            model: "llama3.2:3b".to_string(),
            prompt: "Summarize readiness.".to_string(),
            system_prompt: None,
            options: json!({}),
        })
        .expect_err("generate response without response text must fail");

    assert_eq!(failure.code, OllamaInferenceErrorCode::InvalidResponse);
}

#[test]
fn ollama_provider_reports_http_failure_and_malformed_json() {
    let http_failure = provider(&start_raw_server(vec![http_response(
        500,
        &json!({"error": "model load failed"}).to_string(),
    )]))
    .check_model_available("llama3.2:3b")
    .expect_err("non-success status must fail");
    assert_eq!(http_failure.code, OllamaInferenceErrorCode::ProviderFailure);

    let malformed_json = provider(&start_raw_server(vec![http_response(200, "{not-json}")]))
        .check_model_available("llama3.2:3b")
        .expect_err("malformed JSON must fail");
    assert_eq!(
        malformed_json.code,
        OllamaInferenceErrorCode::InvalidResponse
    );
}

#[test]
fn ollama_provider_reports_malformed_http_responses() {
    let missing_separator = provider(&start_raw_server(vec!["HTTP/1.1 200 OK".to_string()]))
        .check_model_available("llama3.2:3b")
        .expect_err("missing header separator must fail");
    assert_eq!(
        missing_separator.code,
        OllamaInferenceErrorCode::InvalidResponse
    );

    let missing_status = provider(&start_raw_server(vec![http_response_with_status(
        "HTTP/1.1", "{}",
    )]))
    .check_model_available("llama3.2:3b")
    .expect_err("missing status code must fail");
    assert_eq!(
        missing_status.code,
        OllamaInferenceErrorCode::InvalidResponse
    );

    let empty_status = provider(&start_raw_server(vec!["\r\n\r\n{}".to_string()]))
        .check_model_available("llama3.2:3b")
        .expect_err("empty status line must fail");
    assert_eq!(empty_status.code, OllamaInferenceErrorCode::InvalidResponse);

    let invalid_status = provider(&start_raw_server(vec![http_response_with_status(
        "HTTP/1.1 OK",
        "{}",
    )]))
    .check_model_available("llama3.2:3b")
    .expect_err("invalid status code must fail");
    assert_eq!(
        invalid_status.code,
        OllamaInferenceErrorCode::InvalidResponse
    );
}

#[test]
fn ollama_provider_reports_provider_unavailable() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("ephemeral port should bind");
    let port = listener
        .local_addr()
        .expect("listener address should be readable")
        .port();
    drop(listener);
    let provider = provider_with_timeout(&format!("http://127.0.0.1:{port}"), 100);

    let failure = provider
        .check_model_available("llama3.2:3b")
        .expect_err("closed local port must fail as provider unavailable");

    assert_eq!(
        failure.code,
        OllamaInferenceErrorCode::ModelProviderUnavailable
    );
    assert_eq!(failure.machine_code(), "model_provider_unavailable");
}

#[test]
fn ollama_provider_rejects_invalid_config_and_prompt() {
    let config_failure = OllamaInferenceProvider::new(OllamaProviderConfig {
        base_url: "https://127.0.0.1:11434".to_string(),
        request_timeout_ms: None,
    })
    .expect_err("https endpoint is not supported by stdlib local provider");
    assert_eq!(config_failure.code, OllamaInferenceErrorCode::InvalidConfig);

    let provider = provider("http://127.0.0.1:11434");
    let prompt_failure = provider
        .generate(&OllamaInferenceRequest {
            model: "llama3.2:3b".to_string(),
            prompt: " ".to_string(),
            system_prompt: None,
            options: json!({}),
        })
        .expect_err("blank prompt must fail before provider call");
    assert_eq!(prompt_failure.code, OllamaInferenceErrorCode::InvalidConfig);

    let generate_model_failure = provider
        .generate(&OllamaInferenceRequest {
            model: " ".to_string(),
            prompt: "Summarize readiness.".to_string(),
            system_prompt: None,
            options: json!({}),
        })
        .expect_err("blank model in generate request must fail before provider call");
    assert_eq!(
        generate_model_failure.code,
        OllamaInferenceErrorCode::InvalidConfig
    );

    let model_failure = provider
        .check_model_available(" ")
        .expect_err("blank model must fail before provider call");
    assert_eq!(model_failure.code, OllamaInferenceErrorCode::InvalidConfig);

    let options_failure = provider
        .generate(&OllamaInferenceRequest {
            model: "llama3.2:3b".to_string(),
            prompt: "Summarize readiness.".to_string(),
            system_prompt: None,
            options: json!("invalid"),
        })
        .expect_err("non-object options must fail before provider call");
    assert_eq!(
        options_failure.code,
        OllamaInferenceErrorCode::InvalidConfig
    );
}

#[test]
fn ollama_provider_rejects_invalid_endpoint_shapes() {
    for base_url in ["http://", "http://:11434", "http://127.0.0.1:not-a-port"] {
        let failure = OllamaInferenceProvider::new(OllamaProviderConfig {
            base_url: base_url.to_string(),
            request_timeout_ms: None,
        })
        .expect_err("invalid endpoint should fail");
        assert_eq!(failure.code, OllamaInferenceErrorCode::InvalidConfig);
    }
}

#[test]
fn ollama_error_codes_are_stable() {
    assert_eq!(
        OllamaInferenceErrorCode::InvalidConfig.as_str(),
        "model_candidate_config_invalid"
    );
    assert_eq!(
        OllamaInferenceErrorCode::ProviderFailure.as_str(),
        "model_provider_failure"
    );
    assert_eq!(
        GovernedModelExecutionErrorCode::InterfaceNotDeclared.as_str(),
        "model_interface_not_declared"
    );
    assert_eq!(
        GovernedModelExecutionErrorCode::ModelDependencyUnsatisfied.as_str(),
        "model_dependency_unsatisfied"
    );
    assert_eq!(
        GovernedModelExecutionErrorCode::ProviderExecutionFailed.as_str(),
        "model_provider_failure"
    );

    let display = GovernedModelExecutionError::new(
        GovernedModelExecutionErrorCode::InterfaceNotDeclared,
        "missing interface",
    )
    .to_string();
    assert!(display.contains("model_interface_not_declared"));
    assert!(display.contains("missing interface"));
}

#[test]
fn ollama_provider_reports_resolution_failure() {
    let provider = provider_with_timeout("http://invalid host:11434", 100);

    let failure = provider
        .check_model_available("llama3.2:3b")
        .expect_err("invalid hostname should fail during provider resolution");

    assert_eq!(
        failure.code,
        OllamaInferenceErrorCode::ModelProviderUnavailable
    );
    assert!(failure.to_string().contains("model_provider_unavailable"));
}

#[test]
fn inference_contract_validates_and_registers_with_http_connector() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../contracts/inference/traverse.inference.generate/contract.json");
    let contract_text =
        fs::read_to_string(&path).expect("inference contract fixture should be readable");
    let contract = parse_contract(&contract_text).expect("inference contract should validate");
    assert_eq!(contract.id, "traverse.inference.generate");
    assert_eq!(
        contract.connector_requirements[0].connector_id,
        "traverse.http"
    );

    let mut registry = CapabilityRegistry::new();
    let connector = reference_connector_contracts()
        .into_iter()
        .find(|candidate| candidate.connector_id == "traverse.http")
        .expect("reference http connector should exist");
    registry
        .register_connector(ConnectorRegistration {
            scope: RegistryScope::Public,
            contract: connector,
            contract_path: "contracts/connectors/traverse.http/connector_contract.json".to_string(),
            registered_at: "2026-06-19T00:00:00Z".to_string(),
            governing_spec: "045-governed-model-dependency-resolution".to_string(),
            validator_version: "test".to_string(),
        })
        .expect("http connector should register");

    let outcome = registry
        .register(CapabilityRegistration {
            scope: RegistryScope::Public,
            contract: contract.clone(),
            contract_path: path.display().to_string(),
            artifact: CapabilityArtifactRecord {
                artifact_ref: "ollama.local.generate".to_string(),
                implementation_kind: ImplementationKind::Executable,
                source: SourceReference {
                    kind: SourceKind::Local,
                    location: "crates/traverse-runtime/src/inference.rs".to_string(),
                },
                binary: Some(BinaryReference {
                    format: BinaryFormat::Wasm,
                    location: "providers/ollama.local.generate.wasm".to_string(),
                    signature: None,
                }),
                workflow_ref: None,
                digests: ArtifactDigests {
                    source_digest: governed_content_digest(&contract),
                    binary_digest: Some(
                        "sha256:ollama-local-generate-provider-artifact".to_string(),
                    ),
                },
                provenance: RegistryProvenance {
                    source: "greenfield".to_string(),
                    author: "enricopiovesan".to_string(),
                    created_at: "2026-06-19T00:00:00Z".to_string(),
                },
            },
            registered_at: "2026-06-19T00:00:00Z".to_string(),
            tags: vec!["inference".to_string(), "ollama".to_string()],
            composability: ComposabilityMetadata {
                kind: CompositionKind::Atomic,
                patterns: vec![CompositionPattern::Enrichment],
                provides: vec!["traverse.inference.generate".to_string()],
                requires: vec!["traverse.http".to_string()],
            },
            governing_spec: "045-governed-model-dependency-resolution".to_string(),
            validator_version: "test".to_string(),
        })
        .expect("inference capability should register when http connector exists");

    assert_eq!(outcome.record.id, "traverse.inference.generate");
}

#[test]
fn ollama_model_resolution_selects_available_candidate_at_setup() {
    let tags = json!({
        "models": [{"name": "mistral:7b"}]
    })
    .to_string();
    let base_url = start_ollama_server(vec![tags.clone(), tags]);
    let dependency = model_dependency(vec![
        model_candidate("preferred", "llama3.2:3b", 20, 8192),
        model_candidate("fallback", "mistral:7b", 10, 8192),
    ]);

    let evidence = resolve_ollama_model_dependency(
        &dependency,
        &model_request(ModelResolutionPhase::Setup),
        &OllamaModelAvailabilityProbe::new(provider_config(&base_url, 1_000)),
    );

    assert_eq!(
        evidence
            .selected
            .expect("available fallback should be selected")
            .candidate_id,
        "fallback"
    );
    assert_eq!(
        evidence.candidates[0].rejection_code,
        Some(ModelCandidateRejectionCode::ModelCandidateUnavailable)
    );
    assert!(evidence.failure_code.is_none());
}

#[test]
fn ollama_model_resolution_revalidates_at_execution_time() {
    let setup_tags = json!({
        "models": [{"name": "llama3.2:3b"}, {"name": "mistral:7b"}]
    })
    .to_string();
    let execution_tags = json!({
        "models": [{"name": "mistral:7b"}]
    })
    .to_string();
    let setup_base_url = start_ollama_server(vec![setup_tags.clone(), setup_tags]);
    let execution_base_url = start_ollama_server(vec![execution_tags.clone(), execution_tags]);
    let dependency = model_dependency(vec![
        model_candidate("setup-choice", "llama3.2:3b", 20, 8192),
        model_candidate("execution-choice", "mistral:7b", 10, 8192),
    ]);

    let setup = resolve_ollama_model_dependency(
        &dependency,
        &model_request(ModelResolutionPhase::Setup),
        &OllamaModelAvailabilityProbe::new(provider_config(&setup_base_url, 1_000)),
    );
    let execution = resolve_ollama_model_dependency(
        &dependency,
        &model_request(ModelResolutionPhase::Execution),
        &OllamaModelAvailabilityProbe::new(provider_config(&execution_base_url, 1_000)),
    );

    assert_eq!(
        setup.selected.expect("setup should select").candidate_id,
        "setup-choice"
    );
    assert_eq!(
        execution
            .selected
            .expect("execution should select fallback")
            .candidate_id,
        "execution-choice"
    );
    assert_eq!(execution.phase, ModelResolutionPhase::Execution);
}

#[test]
fn ollama_model_resolution_reports_unsatisfied_dependency() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("ephemeral port should bind");
    let port = listener
        .local_addr()
        .expect("listener address should be readable")
        .port();
    drop(listener);
    let dependency = model_dependency(vec![model_candidate(
        "unreachable",
        "llama3.2:3b",
        20,
        8192,
    )]);

    let evidence = resolve_ollama_model_dependency(
        &dependency,
        &model_request(ModelResolutionPhase::Execution),
        &OllamaModelAvailabilityProbe::new(provider_config(
            &format!("http://127.0.0.1:{port}"),
            100,
        )),
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
}

#[test]
fn ollama_model_resolution_maps_provider_config_and_http_failures() {
    let tags = json!({
        "models": [{"name": "llama3.2:3b"}]
    })
    .to_string();
    let custom_base_url = start_ollama_server(vec![tags]);
    let mut custom_candidate = model_candidate("custom", "llama3.2:3b", 20, 8192);
    custom_candidate.provider_implementation_id = "ollama.custom.generate".to_string();
    let custom = resolve_ollama_model_dependency(
        &model_dependency(vec![custom_candidate]),
        &model_request(ModelResolutionPhase::Setup),
        &OllamaModelAvailabilityProbe::default().with_provider_config(
            "ollama.custom.generate",
            provider_config(&custom_base_url, 1_000),
        ),
    );
    assert_eq!(
        custom
            .selected
            .expect("custom config should select")
            .candidate_id,
        "custom"
    );

    let mut missing_config_candidate = model_candidate("missing-config", "llama3.2:3b", 20, 8192);
    missing_config_candidate.provider_implementation_id = "ollama.missing.generate".to_string();
    let missing_config = resolve_ollama_model_dependency(
        &model_dependency(vec![missing_config_candidate]),
        &model_request(ModelResolutionPhase::Setup),
        &OllamaModelAvailabilityProbe::new(provider_config("http://127.0.0.1:11434", 100)),
    );
    assert_eq!(
        missing_config.candidates[0].rejection_code,
        Some(ModelCandidateRejectionCode::ModelCandidateConfigInvalid)
    );

    let invalid_config = resolve_ollama_model_dependency(
        &model_dependency(vec![model_candidate(
            "invalid-config",
            "llama3.2:3b",
            20,
            8192,
        )]),
        &model_request(ModelResolutionPhase::Setup),
        &OllamaModelAvailabilityProbe::new(provider_config("https://127.0.0.1:11434", 100)),
    );
    assert_eq!(
        invalid_config.candidates[0].rejection_code,
        Some(ModelCandidateRejectionCode::ModelCandidateConfigInvalid)
    );

    let provider_failure = resolve_ollama_model_dependency(
        &model_dependency(vec![model_candidate(
            "provider-failure",
            "llama3.2:3b",
            20,
            8192,
        )]),
        &model_request(ModelResolutionPhase::Setup),
        &OllamaModelAvailabilityProbe::new(provider_config(
            &start_raw_server(vec![http_response(500, "{}")]),
            1_000,
        )),
    );
    assert_eq!(
        provider_failure.candidates[0].rejection_code,
        Some(ModelCandidateRejectionCode::ModelProviderUnavailable)
    );
}

#[test]
fn governed_model_execution_invokes_selected_provider_and_returns_evidence() {
    let base_url = start_ollama_server(vec![
        json!({"models": [{"name": "llama3.2:3b"}]}).to_string(),
        json!({"models": [{"name": "llama3.2:3b"}]}).to_string(),
        json!({"model": "llama3.2:3b", "response": "ready", "done": true}).to_string(),
    ]);
    let dependency = model_dependency(vec![model_candidate(
        "ready-local",
        "llama3.2:3b",
        20,
        8192,
    )]);

    let outcome = execute_governed_ollama_model_dependency(
        &dependency,
        &governed_model_request("traverse.inference.generate", &base_url),
    )
    .expect("available app-declared model should execute");

    assert_eq!(outcome.output.response, "ready");
    assert_eq!(
        outcome
            .model_resolution
            .selected
            .expect("selected candidate should be recorded")
            .candidate_id,
        "ready-local"
    );
}

#[test]
fn governed_model_execution_rejects_undeclared_interface() {
    let dependency = model_dependency(vec![model_candidate(
        "ready-local",
        "llama3.2:3b",
        20,
        8192,
    )]);

    let error = execute_governed_ollama_model_dependency(
        &dependency,
        &governed_model_request("traverse.inference.embed", "http://127.0.0.1:11434"),
    )
    .expect_err("undeclared interface should fail before provider access");

    assert_eq!(
        error.code,
        GovernedModelExecutionErrorCode::InterfaceNotDeclared
    );
    assert!(error.model_resolution.is_none());
}

#[test]
fn governed_model_execution_reports_unsatisfied_dependency_evidence() {
    let dependency = model_dependency(vec![model_candidate(
        "missing-config",
        "llama3.2:3b",
        20,
        8192,
    )]);
    let mut request =
        governed_model_request("traverse.inference.generate", "http://127.0.0.1:11434");
    request.provider_configs.clear();

    let error = execute_governed_ollama_model_dependency(&dependency, &request)
        .expect_err("missing provider config should leave dependency unsatisfied");

    assert_eq!(
        error.code,
        GovernedModelExecutionErrorCode::ModelDependencyUnsatisfied
    );
    assert_eq!(
        error
            .model_resolution
            .expect("resolution evidence should be attached")
            .machine_failure_code(),
        Some("model_dependency_unsatisfied")
    );
}

#[test]
fn governed_model_execution_reports_provider_execution_failure_with_evidence() {
    let base_url = start_ollama_server(vec![
        json!({"models": [{"name": "llama3.2:3b"}]}).to_string(),
        json!({"models": [{"name": "llama3.2:3b"}]}).to_string(),
        json!({"model": "llama3.2:3b", "done": true}).to_string(),
    ]);
    let dependency = model_dependency(vec![model_candidate(
        "bad-generate",
        "llama3.2:3b",
        20,
        8192,
    )]);

    let error = execute_governed_ollama_model_dependency(
        &dependency,
        &governed_model_request("traverse.inference.generate", &base_url),
    )
    .expect_err("invalid provider output should fail execution");

    assert_eq!(
        error.code,
        GovernedModelExecutionErrorCode::ProviderExecutionFailed
    );
    assert!(
        error
            .model_resolution
            .expect("provider failure should retain selected model evidence")
            .selected
            .is_some()
    );
}

fn provider(base_url: &str) -> OllamaInferenceProvider {
    provider_with_timeout(base_url, 1_000)
}

fn provider_with_timeout(base_url: &str, timeout_ms: u64) -> OllamaInferenceProvider {
    OllamaInferenceProvider::new(provider_config(base_url, timeout_ms))
        .expect("provider config should be valid")
}

fn provider_config(base_url: &str, timeout_ms: u64) -> OllamaProviderConfig {
    OllamaProviderConfig {
        base_url: base_url.to_string(),
        request_timeout_ms: Some(timeout_ms),
    }
}

fn model_request(phase: ModelResolutionPhase) -> ModelResolutionRequest {
    ModelResolutionRequest {
        phase,
        requested_interface_id: "traverse.inference.generate".to_string(),
        requested_placement: ExecutionTarget::Local,
    }
}

fn model_dependency(candidates: Vec<ModelCandidate>) -> ApplicationModelDependency {
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

fn model_candidate(
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

fn governed_model_request(interface_id: &str, base_url: &str) -> GovernedModelExecutionRequest {
    let mut provider_configs = BTreeMap::new();
    provider_configs.insert(
        "ollama.local.generate".to_string(),
        provider_config(base_url, 1_000),
    );
    GovernedModelExecutionRequest {
        interface_id: interface_id.to_string(),
        prompt: "Summarize readiness.".to_string(),
        system_prompt: Some("Be concise.".to_string()),
        options: json!({"temperature": 0}),
        requested_placement: ExecutionTarget::Local,
        provider_configs,
    }
}

fn start_ollama_server(bodies: Vec<String>) -> String {
    start_raw_server(
        bodies
            .into_iter()
            .map(|body| http_response(200, &body))
            .collect(),
    )
}

fn start_raw_server(responses: Vec<String>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("test server should bind");
    let address = listener
        .local_addr()
        .expect("test server address should be available");
    thread::spawn(move || {
        for response in responses {
            let (mut stream, _) = listener.accept().expect("test request should arrive");
            let mut buffer = [0_u8; 2048];
            let _ = stream
                .read(&mut buffer)
                .expect("test request should be readable");
            stream
                .write_all(response.as_bytes())
                .expect("test response should write");
        }
    });

    format!("http://{address}")
}

fn http_response(status: u16, body: &str) -> String {
    http_response_with_status(&format!("HTTP/1.1 {status} OK"), body)
}

fn http_response_with_status(status_line: &str, body: &str) -> String {
    format!(
        "{status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}
