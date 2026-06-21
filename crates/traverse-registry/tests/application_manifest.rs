#![allow(clippy::expect_used)]

use serde_json::json;
use sha2::{Digest, Sha256};
use std::fmt::Write;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use traverse_contracts::{ExecutionTarget, parse_event_contract};
use traverse_registry::{
    ApplicationManifestErrorCode, ApplicationRegistrationErrorCode, ApplicationRegistrationRequest,
    ApplicationRegistrationStatus, ApplicationRegistry, CapabilityRegistry, EventRegistration,
    EventRegistry, LookupScope, ModelCandidateRejectionCode, ModelResolutionEvidence,
    ModelResolutionPhase, RegistryScope, SelectedModelCandidate, WorkflowRegistry,
    load_application_bundle_manifest,
};

#[test]
fn loads_checked_in_application_manifest_with_real_wasm_component() {
    let manifest_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/applications/expedition-readiness/app.manifest.json");

    let bundle = load_application_bundle_manifest(&manifest_path)
        .expect("checked-in application bundle should validate");

    assert_eq!(bundle.app_id, "expedition.readiness");
    assert_eq!(bundle.version, "1.0.0");
    assert_eq!(bundle.components.len(), 1);
    assert_eq!(
        bundle.components[0].manifest.component_id,
        "expedition.readiness.validate-team-readiness-component"
    );
    assert_eq!(
        bundle.components[0].contract.id,
        "expedition.planning.validate-team-readiness"
    );
    assert_eq!(
        bundle.components[0].verified_wasm_digest,
        "sha256:5647c39a1d25d8728350f9619025292a62e78a602068a2ad9b6f075751c93d99"
    );
}

#[test]
fn rejects_manifest_path_without_parent_directory() {
    let failure = load_application_bundle_manifest(PathBuf::new().as_path())
        .expect_err("empty manifest path should fail before reading");

    assert_eq!(
        failure.errors[0].code,
        ApplicationManifestErrorCode::ManifestParentMissing
    );
}

#[test]
fn rejects_missing_application_manifest_file() {
    let fixture = AppFixture::new("missing-app-manifest");

    let failure = load_application_bundle_manifest(&fixture.root.join("missing.manifest.json"))
        .expect_err("missing app manifest should fail");

    assert_eq!(
        failure.errors[0].code,
        ApplicationManifestErrorCode::ManifestReadFailed
    );
}

#[test]
fn rejects_invalid_application_manifest_json() {
    let fixture = AppFixture::new("invalid-app-manifest");
    fs::write(fixture.app_manifest_path(), "{ not valid json ")
        .expect("invalid app manifest fixture should write");

    let failure = load_application_bundle_manifest(&fixture.app_manifest_path())
        .expect_err("invalid app manifest json should fail");

    assert_eq!(
        failure.errors[0].code,
        ApplicationManifestErrorCode::ManifestParseFailed
    );
}

#[test]
fn rejects_missing_component_manifest() {
    let fixture = AppFixture::new("missing-component");
    fixture.write_app_manifest(&json!([component_ref(
        "expedition.readiness.validate-team-readiness-component",
        "1.0.0",
        "sha256:5647c39a1d25d8728350f9619025292a62e78a602068a2ad9b6f075751c93d99",
        "components/missing/component.manifest.json",
    )]));

    let failure = load_application_bundle_manifest(&fixture.app_manifest_path())
        .expect_err("missing component manifest must fail");

    assert_eq!(
        failure.errors[0].code,
        ApplicationManifestErrorCode::AppComponentManifestMissing
    );
}

#[test]
fn rejects_duplicate_component_references() {
    let fixture = AppFixture::new("duplicate-components");
    let refs = json!([
        component_ref(
            "expedition.readiness.validate-team-readiness-component",
            "1.0.0",
            "sha256:5647c39a1d25d8728350f9619025292a62e78a602068a2ad9b6f075751c93d99",
            "components/a/component.manifest.json",
        ),
        component_ref(
            "expedition.readiness.validate-team-readiness-component",
            "1.0.0",
            "sha256:5647c39a1d25d8728350f9619025292a62e78a602068a2ad9b6f075751c93d99",
            "components/b/component.manifest.json",
        )
    ]);
    fixture.write_app_manifest(&refs);

    let failure = load_application_bundle_manifest(&fixture.app_manifest_path())
        .expect_err("duplicate component references must fail");

    assert_eq!(
        failure.errors[0].code,
        ApplicationManifestErrorCode::DuplicateComponentReference
    );
}

#[test]
fn loads_valid_governed_model_dependency_schema() {
    let fixture = AppFixture::new("valid-model-dependency");
    fixture.write_app_manifest_with_model_dependencies(
        &json!([]),
        &json!([model_dependency(json!([model_candidate(
            "ollama-llama-3-2",
            json!({})
        )]))]),
    );

    let bundle = load_application_bundle_manifest(&fixture.app_manifest_path())
        .expect("model dependency schema should validate");

    assert_eq!(bundle.model_dependencies.len(), 1);
    let dependency = &bundle.model_dependencies[0];
    assert_eq!(dependency.interface_id, "traverse.inference.generate");
    assert_eq!(dependency.version_range, "^1.0");
    assert_eq!(dependency.selection_policy.strategy, "priority");
    assert!(dependency.selection_policy.allow_fallback);
    assert_eq!(dependency.minimum_context_window, 8192);
    assert_eq!(dependency.candidates[0].candidate_id, "ollama-llama-3-2");
    assert_eq!(dependency.candidates[0].priority, 10);
}

#[test]
fn rejects_unsupported_model_dependency_interface() {
    let fixture = AppFixture::new("unsupported-model-interface");
    fixture.write_app_manifest_with_model_dependencies(
        &json!([]),
        &json!([
            model_dependency(json!([model_candidate("ollama-llama-3-2", json!({}))]))
                .as_object()
                .map(|object| {
                    let mut object = object.clone();
                    object.insert(
                        "interface_id".to_string(),
                        json!("downstream.private.generate"),
                    );
                    serde_json::Value::Object(object)
                })
                .expect("model dependency object")
        ]),
    );

    let failure = load_application_bundle_manifest(&fixture.app_manifest_path())
        .expect_err("unsupported model interface must fail");

    assert_eq!(
        failure.errors[0].code,
        ApplicationManifestErrorCode::UnsupportedModelInterface
    );
}

#[test]
fn rejects_model_dependency_without_candidates() {
    let fixture = AppFixture::new("missing-model-candidates");
    fixture.write_app_manifest_with_model_dependencies(
        &json!([]),
        &json!([model_dependency(json!([]))]),
    );

    let failure = load_application_bundle_manifest(&fixture.app_manifest_path())
        .expect_err("missing model candidates must fail");

    assert_eq!(
        failure.errors[0].code,
        ApplicationManifestErrorCode::ModelDependencyMissingCandidates
    );
}

#[test]
fn rejects_duplicate_model_candidate_ids() {
    let fixture = AppFixture::new("duplicate-model-candidates");
    fixture.write_app_manifest_with_model_dependencies(
        &json!([]),
        &json!([model_dependency(json!([
            model_candidate("ollama-llama-3-2", json!({})),
            model_candidate("ollama-llama-3-2", json!({"model_context_window": 16384}))
        ]))]),
    );

    let failure = load_application_bundle_manifest(&fixture.app_manifest_path())
        .expect_err("duplicate model candidate must fail");

    assert_eq!(
        failure.errors[0].code,
        ApplicationManifestErrorCode::DuplicateModelCandidate
    );
}

#[test]
fn rejects_invalid_model_candidate_config() {
    let fixture = AppFixture::new("invalid-model-candidate-config");
    let mut invalid_candidate = model_candidate("ollama-llama-3-2", json!({}));
    invalid_candidate
        .as_object_mut()
        .expect("candidate object")
        .insert("priority".to_string(), json!(0));
    fixture.write_app_manifest_with_model_dependencies(
        &json!([]),
        &json!([model_dependency(json!([invalid_candidate]))]),
    );

    let failure = load_application_bundle_manifest(&fixture.app_manifest_path())
        .expect_err("invalid model candidate config must fail");

    assert_eq!(
        failure.errors[0].code,
        ApplicationManifestErrorCode::ModelCandidateConfigInvalid
    );
}

#[test]
fn workspace_config_overrides_declared_fields_and_redacts_secret_values() {
    let fixture = AppFixture::new("workspace-config-merge");
    fixture.write_app_manifest_with_config(
        &json!([]),
        &config_schema(),
        &json!({
            "ollama_base_url": "http://127.0.0.1:11434",
            "browser_origin": "http://127.0.0.1:5173"
        }),
    );
    fixture.write_workspace_config(&json!({
        "overrides": {
            "ollama_base_url": "http://localhost:11434"
        },
        "secrets": {
            "ollama_api_key": "sk-local-secret"
        }
    }));

    let bundle = load_application_bundle_manifest(&fixture.app_manifest_path())
        .expect("workspace config should merge with manifest defaults");

    assert_eq!(
        bundle.effective_config.values["ollama_base_url"],
        "http://localhost:11434"
    );
    assert_eq!(
        bundle.effective_config.values["browser_origin"],
        "http://127.0.0.1:5173"
    );
    assert_eq!(
        bundle.effective_config.redacted_secret_keys,
        vec!["ollama_api_key"]
    );
    let serialized =
        serde_json::to_string(&bundle.effective_config).expect("effective config serializes");
    assert!(!serialized.contains("sk-local-secret"));
}

#[test]
fn rejects_workspace_config_immutable_override() {
    let fixture = AppFixture::new("workspace-config-immutable");
    fixture.write_app_manifest_with_config(&json!([]), &config_schema(), &json!({}));
    fixture.write_workspace_config(&json!({
        "overrides": {
            "app_id": "other.app"
        },
        "secrets": {}
    }));

    let failure = load_application_bundle_manifest(&fixture.app_manifest_path())
        .expect_err("immutable config override must fail");

    assert_eq!(
        failure.errors[0].code,
        ApplicationManifestErrorCode::AppConfigImmutableOverride
    );
}

#[test]
fn rejects_workspace_config_value_that_violates_schema() {
    let fixture = AppFixture::new("workspace-config-invalid");
    fixture.write_app_manifest_with_config(&json!([]), &config_schema(), &json!({}));
    fixture.write_workspace_config(&json!({
        "overrides": {
            "ollama_base_url": false
        },
        "secrets": {}
    }));

    let failure = load_application_bundle_manifest(&fixture.app_manifest_path())
        .expect_err("invalid workspace config type must fail");

    assert_eq!(
        failure.errors[0].code,
        ApplicationManifestErrorCode::AppConfigInvalid
    );
}

#[test]
fn rejects_workspace_config_malformed_json() {
    let fixture = AppFixture::new("workspace-config-malformed");
    fixture.write_app_manifest_with_config(&json!([]), &config_schema(), &json!({}));
    fs::write(fixture.root.join("workspace.config.json"), "{ not json")
        .expect("malformed workspace config should write");

    let failure = load_application_bundle_manifest(&fixture.app_manifest_path())
        .expect_err("malformed workspace config must fail");

    assert_eq!(
        failure.errors[0].code,
        ApplicationManifestErrorCode::AppConfigInvalid
    );
}

#[test]
fn rejects_workspace_config_sections_that_are_not_objects() {
    let cases = vec![
        json!({
            "overrides": "not-object",
            "secrets": {}
        }),
        json!({
            "overrides": {},
            "secrets": "not-object"
        }),
    ];
    for config in cases {
        let fixture = AppFixture::new("workspace-config-section-shape");
        fixture.write_app_manifest_with_config(&json!([]), &config_schema(), &json!({}));
        fixture.write_workspace_config(&config);

        let failure = load_application_bundle_manifest(&fixture.app_manifest_path())
            .expect_err("workspace config sections must be objects");

        assert_eq!(
            failure.errors[0].code,
            ApplicationManifestErrorCode::AppConfigInvalid
        );
    }
}

#[test]
fn rejects_workspace_config_undeclared_and_non_overrideable_fields() {
    let cases = vec![
        (
            json!({
                "overrides": {
                    "private_endpoint": "http://localhost"
                },
                "secrets": {}
            }),
            ApplicationManifestErrorCode::AppConfigInvalid,
        ),
        (
            json!({
                "overrides": {
                    "app_theme": "dark"
                },
                "secrets": {}
            }),
            ApplicationManifestErrorCode::AppConfigImmutableOverride,
        ),
    ];
    for (config, expected_code) in cases {
        let fixture = AppFixture::new("workspace-config-field-guard");
        fixture.write_app_manifest_with_config(&json!([]), &config_schema(), &json!({}));
        fixture.write_workspace_config(&config);

        let failure = load_application_bundle_manifest(&fixture.app_manifest_path())
            .expect_err("workspace config field guard must fail");

        assert_eq!(failure.errors[0].code, expected_code);
    }
}

#[test]
fn rejects_workspace_config_missing_required_effective_value() {
    let fixture = AppFixture::new("workspace-config-required");
    fixture.write_app_manifest_with_config(&json!([]), &config_schema(), &json!({}));
    fixture.write_workspace_config(&json!({
        "overrides": {},
        "secrets": {}
    }));

    let failure = load_application_bundle_manifest(&fixture.app_manifest_path())
        .expect_err("missing required effective config must fail");

    assert_eq!(
        failure.errors[0].code,
        ApplicationManifestErrorCode::AppConfigInvalid
    );
}

#[test]
fn rejects_manifest_default_config_that_is_not_an_object() {
    let fixture = AppFixture::new("workspace-config-default-shape");
    fixture.write_app_manifest_with_config(&json!([]), &config_schema(), &json!("not-object"));

    let failure = load_application_bundle_manifest(&fixture.app_manifest_path())
        .expect_err("manifest default_config must be an object");

    assert_eq!(
        failure.errors[0].code,
        ApplicationManifestErrorCode::AppConfigInvalid
    );
}

#[test]
fn absent_workspace_config_uses_manifest_defaults() {
    let fixture = AppFixture::new("workspace-config-absent");
    fixture.write_app_manifest_without_workspace_config(
        &config_schema(),
        &json!({
            "ollama_base_url": "http://127.0.0.1:11434"
        }),
    );

    let bundle = load_application_bundle_manifest(&fixture.app_manifest_path())
        .expect("absent workspace config should use manifest defaults");

    assert_eq!(
        bundle.effective_config.values["ollama_base_url"],
        "http://127.0.0.1:11434"
    );
    assert!(bundle.effective_config.redacted_secret_keys.is_empty());
}

#[test]
fn missing_workspace_config_file_uses_manifest_defaults() {
    let fixture = AppFixture::new("workspace-config-missing-file");
    fixture.write_app_manifest_with_config(
        &json!([]),
        &config_schema(),
        &json!({
            "ollama_base_url": "http://127.0.0.1:11434"
        }),
    );

    let bundle = load_application_bundle_manifest(&fixture.app_manifest_path())
        .expect("missing workspace config file should use manifest defaults");

    assert_eq!(
        bundle.effective_config.values["ollama_base_url"],
        "http://127.0.0.1:11434"
    );
}

#[test]
fn rejects_unreadable_workspace_config() {
    let fixture = AppFixture::new("workspace-config-unreadable");
    fixture.write_app_manifest_with_config(&json!([]), &config_schema(), &json!({}));
    fixture.write_workspace_config(&json!({
        "overrides": {},
        "secrets": {}
    }));
    make_unreadable(&fixture.root.join("workspace.config.json"));

    let failure = load_application_bundle_manifest(&fixture.app_manifest_path())
        .expect_err("unreadable workspace config must fail");

    assert_eq!(
        failure.errors[0].code,
        ApplicationManifestErrorCode::AppConfigInvalid
    );
}

#[test]
fn accepts_workspace_config_supported_schema_value_types() {
    let fixture = AppFixture::new("workspace-config-types");
    fixture.write_app_manifest_with_config(
        &json!([]),
        &json!({
            "type": "object",
            "properties": {
                "enabled": {"type": "boolean", "x-traverse-overrideable": true},
                "limit": {"type": "integer", "x-traverse-overrideable": true},
                "temperature": {"type": "number", "x-traverse-overrideable": true},
                "metadata": {"type": "object", "x-traverse-overrideable": true},
                "origins": {"type": "array", "x-traverse-overrideable": true},
                "schema_free": {"x-traverse-overrideable": true},
                "future_type": {"type": "duration", "x-traverse-overrideable": true}
            }
        }),
        &json!({}),
    );
    fixture.write_workspace_config(&json!({
        "overrides": {
            "enabled": true,
            "limit": 3,
            "temperature": 0.2,
            "metadata": {"tier": "local"},
            "origins": ["http://localhost:5173"],
            "schema_free": false,
            "future_type": "PT1S"
        },
        "secrets": {}
    }));

    let bundle = load_application_bundle_manifest(&fixture.app_manifest_path())
        .expect("supported config schema types should validate");

    assert_eq!(bundle.effective_config.values["limit"], 3);
    assert_eq!(bundle.effective_config.values["future_type"], "PT1S");
}

#[test]
fn rejects_missing_model_dependency_fields_with_stable_errors() {
    let cases = vec![
        (
            {
                let mut dependency =
                    model_dependency(json!([model_candidate("ollama-llama-3-2", json!({}),)]));
                dependency
                    .as_object_mut()
                    .expect("dependency object")
                    .remove("interface_id");
                dependency
            },
            ApplicationManifestErrorCode::ModelCandidateConfigInvalid,
        ),
        (
            {
                let mut dependency =
                    model_dependency(json!([model_candidate("ollama-llama-3-2", json!({}),)]));
                dependency
                    .as_object_mut()
                    .expect("dependency object")
                    .remove("version_range");
                dependency
            },
            ApplicationManifestErrorCode::ModelCandidateConfigInvalid,
        ),
        (
            {
                let mut dependency =
                    model_dependency(json!([model_candidate("ollama-llama-3-2", json!({}),)]));
                dependency
                    .as_object_mut()
                    .expect("dependency object")
                    .remove("required_capabilities");
                dependency
            },
            ApplicationManifestErrorCode::ModelCandidateConfigInvalid,
        ),
        (
            {
                let mut dependency =
                    model_dependency(json!([model_candidate("ollama-llama-3-2", json!({}),)]));
                dependency
                    .as_object_mut()
                    .expect("dependency object")
                    .remove("selection_policy");
                dependency
            },
            ApplicationManifestErrorCode::ModelCandidateConfigInvalid,
        ),
        (
            {
                let mut dependency =
                    model_dependency(json!([model_candidate("ollama-llama-3-2", json!({}),)]));
                dependency["selection_policy"]
                    .as_object_mut()
                    .expect("selection policy object")
                    .remove("strategy");
                dependency
            },
            ApplicationManifestErrorCode::ModelCandidateConfigInvalid,
        ),
        (
            {
                let mut dependency =
                    model_dependency(json!([model_candidate("ollama-llama-3-2", json!({}),)]));
                dependency
                    .as_object_mut()
                    .expect("dependency object")
                    .remove("candidates");
                dependency
            },
            ApplicationManifestErrorCode::ModelDependencyMissingCandidates,
        ),
    ];

    assert_model_dependency_rejections("missing-model-dependency", cases);
}

#[test]
fn rejects_invalid_model_dependency_values_with_stable_errors() {
    let cases = vec![
        (
            {
                let mut dependency =
                    model_dependency(json!([model_candidate("ollama-llama-3-2", json!({}),)]));
                dependency["selection_policy"]["strategy"] = json!("random");
                dependency
            },
            ApplicationManifestErrorCode::ModelCandidateConfigInvalid,
        ),
        (
            {
                let mut dependency =
                    model_dependency(json!([model_candidate("ollama-llama-3-2", json!({}),)]));
                dependency["minimum_context_window"] = json!(0);
                dependency
            },
            ApplicationManifestErrorCode::ModelCandidateConfigInvalid,
        ),
        (
            {
                let mut dependency =
                    model_dependency(json!([model_candidate("ollama-llama-3-2", json!({}),)]));
                dependency["required_capabilities"] = json!([""]);
                dependency
            },
            ApplicationManifestErrorCode::ModelCandidateConfigInvalid,
        ),
    ];

    assert_model_dependency_rejections("invalid-model-dependency", cases);
}

#[test]
fn rejects_incomplete_model_candidate_fields_with_stable_errors() {
    let candidate_fields = vec![
        "candidate_id",
        "provider_capability_id",
        "provider_implementation_id",
        "model_identifier",
        "placement_target",
    ];

    for field in candidate_fields {
        let fixture = AppFixture::new(&format!("incomplete-model-candidate-{field}"));
        let mut candidate = model_candidate("ollama-llama-3-2", json!({}));
        candidate
            .as_object_mut()
            .expect("candidate object")
            .remove(field);
        fixture.write_app_manifest_with_model_dependencies(
            &json!([]),
            &json!([model_dependency(json!([candidate]))]),
        );

        let failure = load_application_bundle_manifest(&fixture.app_manifest_path())
            .expect_err("incomplete model candidate must fail");

        assert_eq!(
            failure.errors[0].code,
            ApplicationManifestErrorCode::ModelCandidateConfigInvalid
        );
    }
}

#[test]
fn rejects_model_candidate_without_metadata_object() {
    let fixture = AppFixture::new("invalid-model-candidate-metadata");
    let mut candidate = model_candidate("ollama-llama-3-2", json!({}));
    candidate
        .as_object_mut()
        .expect("candidate object")
        .insert("metadata".to_string(), json!(null));
    fixture.write_app_manifest_with_model_dependencies(
        &json!([]),
        &json!([model_dependency(json!([candidate]))]),
    );

    let failure = load_application_bundle_manifest(&fixture.app_manifest_path())
        .expect_err("model candidate without metadata object must fail");

    assert_eq!(
        failure.errors[0].code,
        ApplicationManifestErrorCode::ModelCandidateConfigInvalid
    );
}

#[test]
fn rejects_placeholder_model_candidate_metadata() {
    let fixture = AppFixture::new("placeholder-model-metadata");
    fixture.write_app_manifest_with_model_dependencies(
        &json!([]),
        &json!([model_dependency(json!([model_candidate(
            "ollama-llama-3-2",
            json!({"implementation_kind": "documentation-only"})
        )]))]),
    );

    let failure = load_application_bundle_manifest(&fixture.app_manifest_path())
        .expect_err("placeholder model candidate metadata must fail");

    assert_eq!(
        failure.errors[0].code,
        ApplicationManifestErrorCode::ModelCandidateImplementationInvalid
    );
}

#[test]
fn rejects_placeholder_model_candidate_implementation() {
    let fixture = AppFixture::new("placeholder-model-implementation");
    let mut candidate = model_candidate("ollama-llama-3-2", json!({}));
    candidate.as_object_mut().expect("candidate object").insert(
        "provider_implementation_id".to_string(),
        json!("placeholder.ollama-provider"),
    );
    fixture.write_app_manifest_with_model_dependencies(
        &json!([]),
        &json!([model_dependency(json!([candidate]))]),
    );

    let failure = load_application_bundle_manifest(&fixture.app_manifest_path())
        .expect_err("placeholder model candidate must fail");

    assert_eq!(
        failure.errors[0].code,
        ApplicationManifestErrorCode::ModelCandidateImplementationInvalid
    );
}

#[test]
fn rejects_unreadable_component_manifest() {
    let fixture = AppFixture::new("unreadable-component-manifest");
    fixture.write_app_manifest(&json!([component_ref(
        "expedition.readiness.validate-team-readiness-component",
        "1.0.0",
        "sha256:5647c39a1d25d8728350f9619025292a62e78a602068a2ad9b6f075751c93d99",
        "components/validate-team-readiness/component.manifest.json",
    )]));
    fs::write(fixture.component_manifest_path(), "{}")
        .expect("component manifest fixture should write");
    make_unreadable(&fixture.component_manifest_path());

    let failure = load_application_bundle_manifest(&fixture.app_manifest_path())
        .expect_err("unreadable component manifest must fail");

    assert_eq!(
        failure.errors[0].code,
        ApplicationManifestErrorCode::ComponentManifestReadFailed
    );
}

#[test]
fn rejects_invalid_component_manifest_json() {
    let fixture = AppFixture::new("invalid-component-manifest");
    fixture.write_app_manifest(&json!([component_ref(
        "expedition.readiness.validate-team-readiness-component",
        "1.0.0",
        "sha256:5647c39a1d25d8728350f9619025292a62e78a602068a2ad9b6f075751c93d99",
        "components/validate-team-readiness/component.manifest.json",
    )]));
    fs::write(fixture.component_manifest_path(), "{ not valid json ")
        .expect("invalid component manifest fixture should write");

    let failure = load_application_bundle_manifest(&fixture.app_manifest_path())
        .expect_err("invalid component manifest json must fail");

    assert_eq!(
        failure.errors[0].code,
        ApplicationManifestErrorCode::ComponentManifestParseFailed
    );
}

#[test]
fn rejects_component_reference_identity_mismatch() {
    let fixture = AppFixture::new("component-reference-mismatch");
    let wasm_digest = fixture.write_wasm("component bytes");
    fixture.write_component_manifest(&json!({
        "component_id": "expedition.readiness.other-component",
        "wasm_digest": format!("sha256:{wasm_digest}"),
        "dependencies": []
    }));
    fixture.write_app_manifest(&json!([component_ref(
        "expedition.readiness.validate-team-readiness-component",
        "1.0.0",
        &format!("sha256:{wasm_digest}"),
        "components/validate-team-readiness/component.manifest.json",
    )]));

    let failure = load_application_bundle_manifest(&fixture.app_manifest_path())
        .expect_err("component reference mismatch must fail");

    assert_eq!(
        failure.errors[0].code,
        ApplicationManifestErrorCode::ComponentReferenceMismatch
    );
}

#[test]
fn rejects_component_contract_identity_mismatch() {
    let fixture = AppFixture::new("contract-mismatch");
    let wasm_digest = fixture.write_wasm("component bytes");
    fixture.write_component_manifest(&json!({
        "capability_id": "expedition.planning.not-the-contract",
        "capability_version": "1.0.0",
        "wasm_digest": format!("sha256:{wasm_digest}"),
        "dependencies": []
    }));
    fixture.write_app_manifest(&json!([component_ref(
        "expedition.readiness.validate-team-readiness-component",
        "1.0.0",
        &format!("sha256:{wasm_digest}"),
        "components/validate-team-readiness/component.manifest.json",
    )]));

    let failure = load_application_bundle_manifest(&fixture.app_manifest_path())
        .expect_err("contract mismatch must fail");

    assert_eq!(
        failure.errors[0].code,
        ApplicationManifestErrorCode::ComponentContractMismatch
    );
}

#[test]
fn rejects_component_manifest_digest_mismatch() {
    let fixture = AppFixture::new("component-manifest-digest-mismatch");
    let wasm_digest = fixture.write_wasm("component bytes");
    fixture.write_component_manifest(&json!({
        "wasm_digest": format!("sha256:{wasm_digest}"),
        "dependencies": []
    }));
    fixture.write_app_manifest(&json!([component_ref(
        "expedition.readiness.validate-team-readiness-component",
        "1.0.0",
        "sha256:5647c39a1d25d8728350f9619025292a62e78a602068a2ad9b6f075751c93d99",
        "components/validate-team-readiness/component.manifest.json",
    )]));

    let failure = load_application_bundle_manifest(&fixture.app_manifest_path())
        .expect_err("app/component digest mismatch must fail");

    assert_eq!(
        failure.errors[0].code,
        ApplicationManifestErrorCode::ComponentDigestMismatch
    );
}

#[test]
fn rejects_invalid_component_manifest_digest_metadata() {
    let fixture = AppFixture::new("invalid-component-digest");
    fixture.write_component_manifest(&json!({
        "wasm_digest": "fnv1a64:dffc31d6401c84d6",
        "dependencies": []
    }));
    fixture.write_app_manifest(&json!([component_ref(
        "expedition.readiness.validate-team-readiness-component",
        "1.0.0",
        "sha256:5647c39a1d25d8728350f9619025292a62e78a602068a2ad9b6f075751c93d99",
        "components/validate-team-readiness/component.manifest.json",
    )]));

    let failure = load_application_bundle_manifest(&fixture.app_manifest_path())
        .expect_err("invalid component digest metadata must fail");

    assert_eq!(
        failure.errors[0].code,
        ApplicationManifestErrorCode::InvalidDigestMetadata
    );
}

#[test]
fn rejects_invalid_digest_metadata() {
    let fixture = AppFixture::new("invalid-digest");
    fixture.write_app_manifest(&json!([component_ref(
        "expedition.readiness.validate-team-readiness-component",
        "1.0.0",
        "fnv1a64:dffc31d6401c84d6",
        "components/validate-team-readiness/component.manifest.json",
    )]));
    fixture.write_component_manifest(&json!({
        "wasm_digest": "fnv1a64:dffc31d6401c84d6",
        "dependencies": []
    }));

    let failure = load_application_bundle_manifest(&fixture.app_manifest_path())
        .expect_err("invalid digest metadata must fail");

    assert_eq!(
        failure.errors[0].code,
        ApplicationManifestErrorCode::InvalidDigestMetadata
    );
}

#[test]
fn rejects_missing_component_contract() {
    let fixture = AppFixture::new("missing-contract");
    let wasm_digest = fixture.write_wasm("component bytes");
    fixture.write_component_manifest(&json!({
        "contract_path": "missing-contract.json",
        "wasm_digest": format!("sha256:{wasm_digest}"),
        "dependencies": []
    }));
    fixture.write_app_manifest(&json!([component_ref(
        "expedition.readiness.validate-team-readiness-component",
        "1.0.0",
        &format!("sha256:{wasm_digest}"),
        "components/validate-team-readiness/component.manifest.json",
    )]));

    let failure = load_application_bundle_manifest(&fixture.app_manifest_path())
        .expect_err("missing component contract must fail");

    assert_eq!(
        failure.errors[0].code,
        ApplicationManifestErrorCode::ComponentContractMissing
    );
}

#[test]
fn rejects_unreadable_component_contract() {
    let fixture = AppFixture::new("unreadable-contract");
    let wasm_digest = fixture.write_wasm("component bytes");
    let contract_path = fixture
        .root
        .join("components/validate-team-readiness/unreadable-contract.json");
    fs::write(&contract_path, "{}").expect("contract fixture should write");
    make_unreadable(&contract_path);
    fixture.write_component_manifest(&json!({
        "contract_path": "unreadable-contract.json",
        "wasm_digest": format!("sha256:{wasm_digest}"),
        "dependencies": []
    }));
    fixture.write_app_manifest(&json!([component_ref(
        "expedition.readiness.validate-team-readiness-component",
        "1.0.0",
        &format!("sha256:{wasm_digest}"),
        "components/validate-team-readiness/component.manifest.json",
    )]));

    let failure = load_application_bundle_manifest(&fixture.app_manifest_path())
        .expect_err("unreadable component contract must fail");

    assert_eq!(
        failure.errors[0].code,
        ApplicationManifestErrorCode::ComponentContractMissing
    );
}

#[test]
fn rejects_invalid_component_contract_json() {
    let fixture = AppFixture::new("invalid-contract");
    let wasm_digest = fixture.write_wasm("component bytes");
    let contract_path = fixture
        .root
        .join("components/validate-team-readiness/invalid-contract.json");
    fs::write(&contract_path, "{}").expect("contract fixture should write");
    fixture.write_component_manifest(&json!({
        "contract_path": "invalid-contract.json",
        "wasm_digest": format!("sha256:{wasm_digest}"),
        "dependencies": []
    }));
    fixture.write_app_manifest(&json!([component_ref(
        "expedition.readiness.validate-team-readiness-component",
        "1.0.0",
        &format!("sha256:{wasm_digest}"),
        "components/validate-team-readiness/component.manifest.json",
    )]));

    let failure = load_application_bundle_manifest(&fixture.app_manifest_path())
        .expect_err("invalid component contract must fail");

    assert_eq!(
        failure.errors[0].code,
        ApplicationManifestErrorCode::ComponentContractParseFailed
    );
}

#[test]
fn rejects_missing_wasm_binary() {
    let fixture = AppFixture::new("missing-wasm");
    let digest = "sha256:5647c39a1d25d8728350f9619025292a62e78a602068a2ad9b6f075751c93d99";
    fixture.write_component_manifest(&json!({
        "wasm_digest": digest,
        "dependencies": []
    }));
    fixture.write_app_manifest(&json!([component_ref(
        "expedition.readiness.validate-team-readiness-component",
        "1.0.0",
        digest,
        "components/validate-team-readiness/component.manifest.json",
    )]));

    let failure = load_application_bundle_manifest(&fixture.app_manifest_path())
        .expect_err("missing WASM binary must fail");

    assert_eq!(
        failure.errors[0].code,
        ApplicationManifestErrorCode::ComponentWasmMissing
    );
}

#[test]
fn rejects_unreadable_wasm_binary() {
    let fixture = AppFixture::new("unreadable-wasm");
    let wasm_digest = fixture.write_wasm("component bytes");
    make_unreadable(&fixture.wasm_path());
    fixture.write_component_manifest(&json!({
        "wasm_digest": format!("sha256:{wasm_digest}"),
        "dependencies": []
    }));
    fixture.write_app_manifest(&json!([component_ref(
        "expedition.readiness.validate-team-readiness-component",
        "1.0.0",
        &format!("sha256:{wasm_digest}"),
        "components/validate-team-readiness/component.manifest.json",
    )]));

    let failure = load_application_bundle_manifest(&fixture.app_manifest_path())
        .expect_err("unreadable WASM binary must fail");

    assert_eq!(
        failure.errors[0].code,
        ApplicationManifestErrorCode::ComponentWasmMissing
    );
}

#[test]
fn rejects_wasm_digest_mismatch() {
    let fixture = AppFixture::new("digest-mismatch");
    let _digest = fixture.write_wasm("different bytes");
    let wrong_digest = "sha256:5647c39a1d25d8728350f9619025292a62e78a602068a2ad9b6f075751c93d99";
    fixture.write_component_manifest(&json!({
        "wasm_digest": wrong_digest,
        "dependencies": []
    }));
    fixture.write_app_manifest(&json!([component_ref(
        "expedition.readiness.validate-team-readiness-component",
        "1.0.0",
        wrong_digest,
        "components/validate-team-readiness/component.manifest.json",
    )]));

    let failure = load_application_bundle_manifest(&fixture.app_manifest_path())
        .expect_err("digest mismatch must fail");

    assert_eq!(
        failure.errors[0].code,
        ApplicationManifestErrorCode::ComponentDigestMismatch
    );
}

#[test]
fn rejects_version_range_component_dependencies() {
    let fixture = AppFixture::new("range-dependency");
    let wasm_digest = fixture.write_wasm("component bytes");
    fixture.write_component_manifest(&json!({
        "wasm_digest": format!("sha256:{wasm_digest}"),
        "dependencies": [
            {
                "component_id": "expedition.readiness.other-component",
                "version_range": "^1.0.0"
            }
        ]
    }));
    fixture.write_app_manifest(&json!([component_ref(
        "expedition.readiness.validate-team-readiness-component",
        "1.0.0",
        &format!("sha256:{wasm_digest}"),
        "components/validate-team-readiness/component.manifest.json",
    )]));

    let failure = load_application_bundle_manifest(&fixture.app_manifest_path())
        .expect_err("version range dependency must fail");

    assert_eq!(
        failure.errors[0].code,
        ApplicationManifestErrorCode::ComponentDependencyMustBeConcrete
    );
}

#[test]
fn accepts_concrete_component_dependencies() {
    let fixture = AppFixture::new("concrete-dependency");
    let wasm_digest = fixture.write_wasm("component bytes");
    fixture.write_component_manifest(&json!({
        "wasm_digest": format!("sha256:{wasm_digest}"),
        "dependencies": [
            {
                "component_id": "expedition.readiness.other-component",
                "version": "1.0.0",
                "digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            }
        ]
    }));
    fixture.write_app_manifest(&json!([component_ref(
        "expedition.readiness.validate-team-readiness-component",
        "1.0.0",
        &format!("sha256:{wasm_digest}"),
        "components/validate-team-readiness/component.manifest.json",
    )]));

    let bundle = load_application_bundle_manifest(&fixture.app_manifest_path())
        .expect("concrete component dependencies should validate");

    assert_eq!(bundle.components[0].manifest.dependencies.len(), 1);
}

#[test]
fn rejects_component_dependencies_without_concrete_version_and_digest() {
    let fixture = AppFixture::new("missing-concrete-dependency");
    let wasm_digest = fixture.write_wasm("component bytes");
    fixture.write_component_manifest(&json!({
        "wasm_digest": format!("sha256:{wasm_digest}"),
        "dependencies": [
            {
                "component_id": "expedition.readiness.other-component",
                "version": " "
            }
        ]
    }));
    fixture.write_app_manifest(&json!([component_ref(
        "expedition.readiness.validate-team-readiness-component",
        "1.0.0",
        &format!("sha256:{wasm_digest}"),
        "components/validate-team-readiness/component.manifest.json",
    )]));

    let failure = load_application_bundle_manifest(&fixture.app_manifest_path())
        .expect_err("non-concrete dependency must fail");

    assert_eq!(
        failure.errors[0].code,
        ApplicationManifestErrorCode::ComponentDependencyMustBeConcrete
    );
}

#[test]
fn registers_application_bundle_atomically_with_created_status() {
    let fixture = AppFixture::new("register-created");
    fixture.write_hello_world_bundle();
    let mut app_registry = ApplicationRegistry::new();
    let mut capability_registry = CapabilityRegistry::new();
    let event_registry = EventRegistry::new();
    let mut workflow_registry = WorkflowRegistry::new();

    let outcome = app_registry
        .register_bundle(
            &mut capability_registry,
            &event_registry,
            &mut workflow_registry,
            &fixture.registration_request(),
        )
        .expect("valid application bundle should register atomically");

    assert_eq!(outcome.status, ApplicationRegistrationStatus::Created);
    assert_eq!(outcome.status.http_status(), 201);
    assert_eq!(outcome.record.app_id, "hello.world.app");
    assert!(outcome.record.model_readiness.is_empty());
    assert_eq!(outcome.record.components.len(), 1);
    assert_eq!(outcome.record.workflows.len(), 1);
    assert!(
        app_registry
            .find_exact(RegistryScope::Private, "hello.world.app", "1.0.0")
            .is_some()
    );
    assert!(
        capability_registry
            .find_exact(LookupScope::PreferPrivate, "hello.world.say-hello", "1.0.0")
            .is_some()
    );
    assert!(
        workflow_registry
            .find_exact(LookupScope::PreferPrivate, "hello.world.say-hello", "1.0.0")
            .is_some()
    );
}

#[test]
fn application_registration_records_model_readiness_evidence() {
    let fixture = AppFixture::new("register-model-readiness");
    fixture.write_hello_world_bundle();
    let mut app_registry = ApplicationRegistry::new();
    let mut capability_registry = CapabilityRegistry::new();
    let event_registry = EventRegistry::new();
    let mut workflow_registry = WorkflowRegistry::new();

    let outcome = app_registry
        .register_bundle_with_model_readiness(
            &mut capability_registry,
            &event_registry,
            &mut workflow_registry,
            &fixture.registration_request(),
            vec![model_readiness_evidence("ollama-llama-3-2")],
        )
        .expect("valid application bundle should register with readiness evidence");

    assert_eq!(outcome.record.model_readiness.len(), 1);
    let readiness = &outcome.record.model_readiness[0];
    assert_eq!(readiness.phase, ModelResolutionPhase::Setup);
    assert_eq!(
        readiness
            .selected
            .as_ref()
            .expect("selected candidate should be recorded")
            .candidate_id,
        "ollama-llama-3-2"
    );
    assert!(readiness.failure_code.is_none());
}

#[test]
fn application_registration_exposes_only_non_sensitive_effective_config() {
    let fixture = AppFixture::new("register-effective-config");
    let wasm_digest = fixture.write_hello_component("hello world executable bytes");
    let workflow_path = fixture.write_workflow("hello.world.say-hello", "hello.world.say-hello");
    fixture.write_app_manifest_full_with_config(
        &json!([component_ref(
            "hello.world.say-hello-component",
            "1.0.0",
            &format!("sha256:{wasm_digest}"),
            "components/validate-team-readiness/component.manifest.json",
        )]),
        &json!([{
            "workflow_id": "hello.world.say-hello",
            "workflow_version": "1.0.0",
            "path": workflow_path
        }]),
        &json!([]),
        &config_schema(),
        &json!({
            "ollama_base_url": "http://127.0.0.1:11434"
        }),
    );
    fixture.write_workspace_config(&json!({
        "overrides": {
            "ollama_base_url": "http://localhost:11434"
        },
        "secrets": {
            "ollama_api_key": "sk-local-secret"
        }
    }));
    let mut app_registry = ApplicationRegistry::new();
    let mut capability_registry = CapabilityRegistry::new();
    let event_registry = EventRegistry::new();
    let mut workflow_registry = WorkflowRegistry::new();

    let outcome = app_registry
        .register_bundle(
            &mut capability_registry,
            &event_registry,
            &mut workflow_registry,
            &fixture.registration_request(),
        )
        .expect("registration record should expose redacted effective config");

    assert_eq!(
        outcome.record.effective_config.values["ollama_base_url"],
        "http://localhost:11434"
    );
    assert_eq!(
        outcome.record.effective_config.redacted_secret_keys,
        vec!["ollama_api_key"]
    );
    assert_ne!(
        outcome.record.effective_config.values["ollama_base_url"],
        "sk-local-secret"
    );
}

#[test]
fn application_registration_returns_stable_config_error_codes() {
    let cases = vec![
        (
            json!({
                "overrides": {
                    "app_id": "other.app"
                },
                "secrets": {}
            }),
            ApplicationRegistrationErrorCode::AppConfigImmutableOverride,
        ),
        (
            json!({
                "overrides": {
                    "ollama_base_url": false
                },
                "secrets": {}
            }),
            ApplicationRegistrationErrorCode::AppConfigInvalid,
        ),
    ];
    for (config, expected_code) in cases {
        let fixture = AppFixture::new("register-invalid-config");
        fixture.write_app_manifest_with_config(&json!([]), &config_schema(), &json!({}));
        fixture.write_workspace_config(&config);
        let mut app_registry = ApplicationRegistry::new();
        let mut capability_registry = CapabilityRegistry::new();
        let event_registry = EventRegistry::new();
        let mut workflow_registry = WorkflowRegistry::new();

        let failure = app_registry
            .register_bundle(
                &mut capability_registry,
                &event_registry,
                &mut workflow_registry,
                &fixture.registration_request(),
            )
            .expect_err("registration must expose stable config error code");

        assert_eq!(failure.errors[0].code, expected_code);
    }
}

#[test]
fn reregistering_unchanged_application_bundle_returns_stable_200() {
    let fixture = AppFixture::new("register-idempotent");
    fixture.write_hello_world_bundle();
    let mut app_registry = ApplicationRegistry::new();
    let mut capability_registry = CapabilityRegistry::new();
    let event_registry = EventRegistry::new();
    let mut workflow_registry = WorkflowRegistry::new();

    let first = app_registry
        .register_bundle(
            &mut capability_registry,
            &event_registry,
            &mut workflow_registry,
            &fixture.registration_request(),
        )
        .expect("first application registration should succeed");
    let second = app_registry
        .register_bundle(
            &mut capability_registry,
            &event_registry,
            &mut workflow_registry,
            &fixture.registration_request(),
        )
        .expect("unchanged application registration should be idempotent");

    assert_eq!(first.status, ApplicationRegistrationStatus::Created);
    assert_eq!(
        second.status,
        ApplicationRegistrationStatus::AlreadyRegistered
    );
    assert_eq!(second.status.http_status(), 200);
    assert_eq!(first.record.bundle_digest, second.record.bundle_digest);
}

#[test]
fn failed_application_registration_writes_no_partial_state() {
    let fixture = AppFixture::new("register-rollback");
    fixture.write_hello_world_bundle();
    let mut app_registry = ApplicationRegistry::new();
    let mut capability_registry = CapabilityRegistry::new();
    let event_registry = EventRegistry::new();
    let mut workflow_registry = WorkflowRegistry::new();
    let first = app_registry
        .register_bundle(
            &mut capability_registry,
            &event_registry,
            &mut workflow_registry,
            &fixture.registration_request(),
        )
        .expect("baseline registration should succeed");
    fixture.write_bad_workflow_bundle();

    let failure = app_registry
        .register_bundle(
            &mut capability_registry,
            &event_registry,
            &mut workflow_registry,
            &fixture.registration_request(),
        )
        .expect_err("changed app bundle with invalid workflow must fail");

    assert_eq!(
        failure.errors[0].code,
        ApplicationRegistrationErrorCode::WorkflowReferenceMismatch
    );
    let record = app_registry
        .find_exact(RegistryScope::Private, "hello.world.app", "1.0.0")
        .expect("existing app record should remain after failed registration");
    assert_eq!(record.bundle_digest, first.record.bundle_digest);
    assert!(
        capability_registry
            .find_exact(LookupScope::PreferPrivate, "hello.world.say-hello", "1.0.0")
            .is_some()
    );
    assert!(
        workflow_registry
            .find_exact(LookupScope::PreferPrivate, "hello.world.say-hello", "1.0.0")
            .is_some()
    );
    assert!(
        workflow_registry
            .find_exact(LookupScope::PreferPrivate, "hello.world.changed", "1.0.0")
            .is_none()
    );
}

#[test]
fn application_registration_reports_missing_manifest_as_validation_failure() {
    let fixture = AppFixture::new("register-missing-manifest");
    let mut app_registry = ApplicationRegistry::new();
    let mut capability_registry = CapabilityRegistry::new();
    let event_registry = EventRegistry::new();
    let mut workflow_registry = WorkflowRegistry::new();

    let failure = app_registry
        .register_bundle(
            &mut capability_registry,
            &event_registry,
            &mut workflow_registry,
            &fixture.registration_request(),
        )
        .expect_err("missing app manifest should fail through registration");

    assert_eq!(
        failure.errors[0].code,
        ApplicationRegistrationErrorCode::ManifestValidationFailed
    );
}

#[test]
fn application_registration_rejects_missing_workflow_file() {
    let fixture = AppFixture::new("register-missing-workflow");
    let wasm_digest = fixture.write_hello_component("hello world executable bytes");
    fixture.write_hello_app_manifest(
        &wasm_digest,
        &json!([{
            "workflow_id": "hello.world.say-hello",
            "workflow_version": "1.0.0",
            "path": "missing-workflow.json"
        }]),
    );
    let mut app_registry = ApplicationRegistry::new();
    let mut capability_registry = CapabilityRegistry::new();
    let event_registry = EventRegistry::new();
    let mut workflow_registry = WorkflowRegistry::new();

    let failure = app_registry
        .register_bundle(
            &mut capability_registry,
            &event_registry,
            &mut workflow_registry,
            &fixture.registration_request(),
        )
        .expect_err("missing workflow should fail registration");

    assert_eq!(
        failure.errors[0].code,
        ApplicationRegistrationErrorCode::WorkflowReadFailed
    );
}

#[test]
fn application_registration_rejects_invalid_workflow_json() {
    let fixture = AppFixture::new("register-invalid-workflow");
    let wasm_digest = fixture.write_hello_component("hello world executable bytes");
    let workflow_path = fixture.root.join("invalid-workflow.json");
    fs::write(&workflow_path, "{ not json").expect("invalid workflow fixture should write");
    fixture.write_hello_app_manifest(
        &wasm_digest,
        &json!([{
            "workflow_id": "hello.world.say-hello",
            "workflow_version": "1.0.0",
            "path": workflow_path
        }]),
    );
    let mut app_registry = ApplicationRegistry::new();
    let mut capability_registry = CapabilityRegistry::new();
    let event_registry = EventRegistry::new();
    let mut workflow_registry = WorkflowRegistry::new();

    let failure = app_registry
        .register_bundle(
            &mut capability_registry,
            &event_registry,
            &mut workflow_registry,
            &fixture.registration_request(),
        )
        .expect_err("invalid workflow JSON should fail registration");

    assert_eq!(
        failure.errors[0].code,
        ApplicationRegistrationErrorCode::WorkflowParseFailed
    );
}

#[test]
fn application_registration_rejects_missing_event_reference() {
    let fixture = AppFixture::new("register-missing-event");
    let wasm_digest = fixture.write_wasm("component bytes");
    fixture.write_component_manifest(&json!({
        "wasm_digest": format!("sha256:{wasm_digest}"),
        "dependencies": []
    }));
    fixture.write_app_manifest(&json!([component_ref(
        "expedition.readiness.validate-team-readiness-component",
        "1.0.0",
        &format!("sha256:{wasm_digest}"),
        "components/validate-team-readiness/component.manifest.json",
    )]));
    let mut app_registry = ApplicationRegistry::new();
    let mut capability_registry = CapabilityRegistry::new();
    let event_registry = EventRegistry::new();
    let mut workflow_registry = WorkflowRegistry::new();

    let failure = app_registry
        .register_bundle(
            &mut capability_registry,
            &event_registry,
            &mut workflow_registry,
            &fixture.registration_request(),
        )
        .expect_err("missing referenced event should fail registration");

    assert_eq!(
        failure.errors[0].code,
        ApplicationRegistrationErrorCode::MissingRequiredEvent
    );
}

#[test]
fn application_registration_checks_all_component_event_references() {
    let fixture = AppFixture::new("register-partial-event");
    let wasm_digest = fixture.write_wasm("component bytes");
    fixture.write_component_manifest(&json!({
        "wasm_digest": format!("sha256:{wasm_digest}"),
        "dependencies": []
    }));
    fixture.write_app_manifest(&json!([component_ref(
        "expedition.readiness.validate-team-readiness-component",
        "1.0.0",
        &format!("sha256:{wasm_digest}"),
        "components/validate-team-readiness/component.manifest.json",
    )]));
    let mut app_registry = ApplicationRegistry::new();
    let mut capability_registry = CapabilityRegistry::new();
    let mut event_registry = EventRegistry::new();
    register_event_fixture(
        &mut event_registry,
        "team-readiness-validated",
        "expedition.planning.team-readiness-validated",
    );
    let mut workflow_registry = WorkflowRegistry::new();

    let failure = app_registry
        .register_bundle(
            &mut capability_registry,
            &event_registry,
            &mut workflow_registry,
            &fixture.registration_request(),
        )
        .expect_err("second missing event reference should fail registration");

    assert_eq!(
        failure.errors[0].code,
        ApplicationRegistrationErrorCode::MissingRequiredEvent
    );
    assert!(
        failure.errors[0]
            .message
            .contains("conditions-summary-assessed")
    );
}

#[test]
fn application_registration_maps_workflow_registration_failure() {
    let fixture = AppFixture::new("register-workflow-failure");
    let wasm_digest = fixture.write_hello_component("hello world executable bytes");
    let workflow_path = fixture.write_workflow("hello.world.say-hello", "hello.world.missing");
    fixture.write_hello_app_manifest(
        &wasm_digest,
        &json!([{
            "workflow_id": "hello.world.say-hello",
            "workflow_version": "1.0.0",
            "path": workflow_path
        }]),
    );
    let mut app_registry = ApplicationRegistry::new();
    let mut capability_registry = CapabilityRegistry::new();
    let event_registry = EventRegistry::new();
    let mut workflow_registry = WorkflowRegistry::new();

    let failure = app_registry
        .register_bundle(
            &mut capability_registry,
            &event_registry,
            &mut workflow_registry,
            &fixture.registration_request(),
        )
        .expect_err("workflow missing capability reference should fail");

    assert_eq!(
        failure.errors[0].code,
        ApplicationRegistrationErrorCode::WorkflowRegistrationFailed
    );
}

#[test]
fn application_registration_maps_capability_registration_failure() {
    let fixture = AppFixture::new("register-capability-failure");
    fixture.write_hello_world_bundle();
    let mut seed_app_registry = ApplicationRegistry::new();
    let mut capability_registry = CapabilityRegistry::new();
    let event_registry = EventRegistry::new();
    let mut workflow_registry = WorkflowRegistry::new();
    seed_app_registry
        .register_bundle(
            &mut capability_registry,
            &event_registry,
            &mut workflow_registry,
            &fixture.registration_request(),
        )
        .expect("seed application registration should succeed");
    let changed_digest = fixture.write_hello_component("changed executable bytes");
    let workflow_path = fixture.write_workflow("hello.world.say-hello", "hello.world.say-hello");
    fixture.write_hello_app_manifest(
        &changed_digest,
        &json!([{
            "workflow_id": "hello.world.say-hello",
            "workflow_version": "1.0.0",
            "path": workflow_path
        }]),
    );
    let mut new_app_registry = ApplicationRegistry::new();

    let failure = new_app_registry
        .register_bundle(
            &mut capability_registry,
            &event_registry,
            &mut workflow_registry,
            &fixture.registration_request(),
        )
        .expect_err("changed component metadata must fail capability registration");

    assert_eq!(
        failure.errors[0].code,
        ApplicationRegistrationErrorCode::CapabilityRegistrationFailed
    );
    assert!(
        new_app_registry
            .find_exact(RegistryScope::Private, "hello.world.app", "1.0.0")
            .is_none()
    );
}

#[test]
fn application_registration_rejects_changed_bundle_for_same_app_version() {
    let fixture = AppFixture::new("register-app-conflict");
    fixture.write_hello_world_bundle();
    let mut app_registry = ApplicationRegistry::new();
    let mut capability_registry = CapabilityRegistry::new();
    let event_registry = EventRegistry::new();
    let mut workflow_registry = WorkflowRegistry::new();
    app_registry
        .register_bundle(
            &mut capability_registry,
            &event_registry,
            &mut workflow_registry,
            &fixture.registration_request(),
        )
        .expect("baseline registration should succeed");
    let wasm_digest = fixture.write_hello_component("hello world executable bytes");
    let workflow_path = fixture.write_workflow("hello.world.say-hello", "hello.world.say-hello");
    fixture.write_hello_app_manifest(
        &wasm_digest,
        &json!([
            {
                "workflow_id": "hello.world.say-hello",
                "workflow_version": "1.0.0",
                "path": workflow_path
            },
            {
                "workflow_id": "hello.world.say-hello",
                "workflow_version": "1.0.0",
                "path": workflow_path
            }
        ]),
    );

    let failure = app_registry
        .register_bundle(
            &mut capability_registry,
            &event_registry,
            &mut workflow_registry,
            &fixture.registration_request(),
        )
        .expect_err("changed app bundle for same version must fail");

    assert_eq!(
        failure.errors[0].code,
        ApplicationRegistrationErrorCode::ImmutableApplicationVersionConflict
    );
}

#[test]
fn application_registration_supports_public_scope_lookup_path() {
    let fixture = AppFixture::new("register-public");
    fixture.write_hello_world_bundle();
    let mut app_registry = ApplicationRegistry::new();
    let mut capability_registry = CapabilityRegistry::new();
    let event_registry = EventRegistry::new();
    let mut workflow_registry = WorkflowRegistry::new();
    let mut request = fixture.registration_request();
    request.scope = RegistryScope::Public;

    let outcome = app_registry
        .register_bundle(
            &mut capability_registry,
            &event_registry,
            &mut workflow_registry,
            &request,
        )
        .expect("public application bundle should register");

    assert_eq!(outcome.status, ApplicationRegistrationStatus::Created);
    assert!(
        app_registry
            .find_exact(RegistryScope::Public, "hello.world.app", "1.0.0")
            .is_some()
    );
}

struct AppFixture {
    root: PathBuf,
}

impl AppFixture {
    fn new(name: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("traverse-app-manifest-{name}-{nanos}"));
        fs::create_dir_all(root.join("components/validate-team-readiness"))
            .expect("fixture directories should be created");
        Self { root }
    }

    fn app_manifest_path(&self) -> PathBuf {
        self.root.join("app.manifest.json")
    }

    fn component_manifest_path(&self) -> PathBuf {
        self.root
            .join("components/validate-team-readiness/component.manifest.json")
    }

    fn wasm_path(&self) -> PathBuf {
        self.root
            .join("components/validate-team-readiness/component.wasm")
    }

    fn write_app_manifest(&self, components: &serde_json::Value) {
        self.write_app_manifest_with_workflows(components, &json!([]));
    }

    fn write_app_manifest_with_model_dependencies(
        &self,
        components: &serde_json::Value,
        model_dependencies: &serde_json::Value,
    ) {
        self.write_app_manifest_full(components, &json!([]), model_dependencies);
    }

    fn write_app_manifest_with_workflows(
        &self,
        components: &serde_json::Value,
        workflows: &serde_json::Value,
    ) {
        self.write_app_manifest_full(components, workflows, &json!([]));
    }

    fn write_app_manifest_with_config(
        &self,
        components: &serde_json::Value,
        config_schema: &serde_json::Value,
        default_config: &serde_json::Value,
    ) {
        self.write_app_manifest_full_with_config(
            components,
            &json!([]),
            &json!([]),
            config_schema,
            default_config,
        );
    }

    fn write_app_manifest_full(
        &self,
        components: &serde_json::Value,
        workflows: &serde_json::Value,
        model_dependencies: &serde_json::Value,
    ) {
        self.write_app_manifest_full_with_config(
            components,
            workflows,
            model_dependencies,
            &json!({
                "type": "object"
            }),
            &json!({}),
        );
    }

    fn write_app_manifest_full_with_config(
        &self,
        components: &serde_json::Value,
        workflows: &serde_json::Value,
        model_dependencies: &serde_json::Value,
        config_schema: &serde_json::Value,
        default_config: &serde_json::Value,
    ) {
        let app = json!({
            "app_id": "hello.world.app",
            "version": "1.0.0",
            "schema_version": "1.0.0",
            "workspace_defaults": {
                "workspace_id": "test",
                "config_path": "workspace.config.json"
            },
            "components": components,
            "workflows": workflows,
            "model_dependencies": model_dependencies,
            "config_schema": config_schema,
            "default_config": default_config,
            "placement_policy": {
                "preferred_targets": ["local"]
            },
            "public_surfaces": ["cli"]
        });
        fs::write(self.app_manifest_path(), app.to_string()).expect("app manifest should write");
    }

    fn write_app_manifest_without_workspace_config(
        &self,
        config_schema: &serde_json::Value,
        default_config: &serde_json::Value,
    ) {
        let app = json!({
            "app_id": "hello.world.app",
            "version": "1.0.0",
            "schema_version": "1.0.0",
            "workspace_defaults": {
                "workspace_id": "test"
            },
            "components": [],
            "workflows": [],
            "model_dependencies": [],
            "config_schema": config_schema,
            "default_config": default_config,
            "placement_policy": {
                "preferred_targets": ["local"]
            },
            "public_surfaces": ["cli"]
        });
        fs::write(self.app_manifest_path(), app.to_string()).expect("app manifest should write");
    }

    fn write_workspace_config(&self, config: &serde_json::Value) {
        fs::write(self.root.join("workspace.config.json"), config.to_string())
            .expect("workspace config should write");
    }

    fn write_component_manifest(&self, overrides: &serde_json::Value) {
        let component_id = overrides
            .get("component_id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("expedition.readiness.validate-team-readiness-component");
        let version = overrides
            .get("version")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("1.0.0");
        let wasm_digest = overrides
            .get("wasm_digest")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("sha256:5647c39a1d25d8728350f9619025292a62e78a602068a2ad9b6f075751c93d99");
        let capability_id = overrides
            .get("capability_id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("expedition.planning.validate-team-readiness");
        let capability_version = overrides
            .get("capability_version")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("1.0.0");
        let dependencies = overrides
            .get("dependencies")
            .cloned()
            .unwrap_or_else(|| json!([]));
        let default_contract_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
            "../../contracts/examples/expedition/capabilities/validate-team-readiness/contract.json",
        );
        let contract_path = overrides
            .get("contract_path")
            .cloned()
            .unwrap_or_else(|| json!(default_contract_path));
        let wasm_binary_path = overrides
            .get("wasm_binary_path")
            .cloned()
            .unwrap_or_else(|| json!("component.wasm"));
        let component = json!({
            "component_id": component_id,
            "version": version,
            "schema_version": "1.0.0",
            "capability_id": capability_id,
            "capability_version": capability_version,
            "contract_path": contract_path,
            "wasm_binary_path": wasm_binary_path,
            "wasm_digest": wasm_digest,
            "runtime_constraints": {
                "host_api_access": "none",
                "network_access": "forbidden",
                "filesystem_access": "none"
            },
            "permitted_targets": ["local"],
            "dependencies": dependencies,
            "connector_requirements": [],
            "validation_evidence": []
        });
        fs::write(self.component_manifest_path(), component.to_string())
            .expect("component manifest should write");
    }

    fn write_wasm(&self, contents: &str) -> String {
        fs::write(self.wasm_path(), contents.as_bytes()).expect("wasm fixture should write");
        sha256_hex(contents.as_bytes())
    }

    fn registration_request(&self) -> ApplicationRegistrationRequest {
        ApplicationRegistrationRequest {
            scope: RegistryScope::Private,
            workspace_id: "test-workspace".to_string(),
            manifest_path: self.app_manifest_path(),
            registered_at: "2026-06-13T00:00:00Z".to_string(),
            validator_version: "test".to_string(),
        }
    }

    fn write_hello_world_bundle(&self) {
        let wasm_digest = self.write_hello_component("hello world executable bytes");
        let workflow_path = self.write_workflow("hello.world.say-hello", "hello.world.say-hello");
        self.write_hello_app_manifest(
            &wasm_digest,
            &json!([{
                "workflow_id": "hello.world.say-hello",
                "workflow_version": "1.0.0",
                "path": workflow_path
            }]),
        );
    }

    fn write_bad_workflow_bundle(&self) {
        let wasm_digest = self.write_hello_component("hello world executable bytes");
        let workflow_path = self.write_workflow("hello.world.changed", "hello.world.say-hello");
        self.write_hello_app_manifest(
            &wasm_digest,
            &json!([{
                "workflow_id": "hello.world.say-hello",
                "workflow_version": "1.0.0",
                "path": workflow_path
            }]),
        );
    }

    fn write_hello_component(&self, contents: &str) -> String {
        let wasm_digest = self.write_wasm(contents);
        let contract_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../contracts/examples/hello-world/capabilities/say-hello/contract.json");
        self.write_component_manifest(&json!({
            "component_id": "hello.world.say-hello-component",
            "capability_id": "hello.world.say-hello",
            "wasm_digest": format!("sha256:{wasm_digest}"),
            "contract_path": contract_path,
            "dependencies": []
        }));
        wasm_digest
    }

    fn write_hello_app_manifest(&self, wasm_digest: &str, workflows: &serde_json::Value) {
        self.write_app_manifest_with_workflows(
            &json!([component_ref(
                "hello.world.say-hello-component",
                "1.0.0",
                &format!("sha256:{wasm_digest}"),
                "components/validate-team-readiness/component.manifest.json",
            )]),
            workflows,
        );
    }

    fn write_workflow(&self, workflow_id: &str, node_capability_id: &str) -> String {
        let path = self.root.join("workflow.json");
        let workflow = json!({
            "kind": "workflow_definition",
            "schema_version": "1.0.0",
            "id": workflow_id,
            "name": "say-hello",
            "version": "1.0.0",
            "lifecycle": "active",
            "owner": {
                "team": "traverse-core",
                "contact": "enrico.piovesan10@gmail.com"
            },
            "summary": "Run the minimal hello-world greeting flow as one governed workflow-backed example.",
            "inputs": {
                "schema": {
                    "type": "object",
                    "required": ["name"],
                    "properties": {
                        "name": {
                            "type": "string"
                        }
                    },
                    "additionalProperties": false
                }
            },
            "outputs": {
                "schema": {
                    "type": "object",
                    "required": ["name", "greeting"],
                    "properties": {
                        "name": {
                            "type": "string"
                        },
                        "greeting": {
                            "type": "string"
                        }
                    },
                    "additionalProperties": false
                }
            },
            "nodes": [
                {
                    "node_id": "say_hello",
                    "capability_id": node_capability_id,
                    "capability_version": "1.0.0",
                    "input": {
                        "from_workflow_input": ["name"]
                    },
                    "output": {
                        "to_workflow_state": ["name", "greeting"]
                    }
                }
            ],
            "edges": [],
            "start_node": "say_hello",
            "terminal_nodes": ["say_hello"],
            "tags": ["hello-world", "registration-test"],
            "governing_spec": "007-workflow-registry-traversal"
        });
        fs::write(&path, workflow.to_string()).expect("workflow fixture should write");
        path.display().to_string()
    }
}

fn make_unreadable(path: &PathBuf) {
    let mut permissions = fs::metadata(path)
        .expect("fixture metadata should be available")
        .permissions();
    permissions.set_mode(0o000);
    fs::set_permissions(path, permissions).expect("fixture should become unreadable");
}

fn component_ref(id: &str, version: &str, digest: &str, manifest_path: &str) -> serde_json::Value {
    json!({
        "component_id": id,
        "version": version,
        "digest": digest,
        "manifest_path": manifest_path
    })
}

fn config_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "required": ["ollama_base_url"],
        "properties": {
            "ollama_base_url": {
                "type": "string",
                "x-traverse-overrideable": true
            },
            "browser_origin": {
                "type": "string",
                "x-traverse-overrideable": true
            },
            "app_theme": {
                "type": "string",
                "x-traverse-overrideable": false
            }
        },
        "additionalProperties": false
    })
}

fn assert_model_dependency_rejections(
    prefix: &str,
    cases: Vec<(serde_json::Value, ApplicationManifestErrorCode)>,
) {
    for (index, (dependency, expected_code)) in cases.into_iter().enumerate() {
        let fixture = AppFixture::new(&format!("{prefix}-{index}"));
        fixture.write_app_manifest_with_model_dependencies(&json!([]), &json!([dependency]));

        let failure = load_application_bundle_manifest(&fixture.app_manifest_path())
            .expect_err("model dependency rejection case must fail");

        assert_eq!(failure.errors[0].code, expected_code);
    }
}

fn model_dependency(candidates: impl Into<serde_json::Value>) -> serde_json::Value {
    let candidates = candidates.into();
    json!({
        "interface_id": "traverse.inference.generate",
        "version_range": "^1.0",
        "selection_policy": {
            "strategy": "priority",
            "allow_fallback": true
        },
        "required_capabilities": ["text_generation"],
        "minimum_context_window": 8192,
        "candidates": candidates
    })
}

fn model_candidate(
    candidate_id: &str,
    metadata_overrides: impl Into<serde_json::Value>,
) -> serde_json::Value {
    let metadata_overrides = metadata_overrides.into();
    let mut metadata = json!({
        "implementation_kind": "real_local_provider",
        "provider": "ollama",
        "model_context_window": 8192,
        "supports_streaming": true
    });
    if let (Some(base), Some(overrides)) =
        (metadata.as_object_mut(), metadata_overrides.as_object())
    {
        for (key, value) in overrides {
            base.insert(key.clone(), value.clone());
        }
    }
    json!({
        "candidate_id": candidate_id,
        "provider_capability_id": "traverse.inference.generate",
        "provider_implementation_id": "ollama.local.generate",
        "model_identifier": "llama3.2:3b",
        "placement_target": "local",
        "priority": 10,
        "required_provider_config_keys": ["ollama_base_url"],
        "metadata": metadata
    })
}

fn model_readiness_evidence(candidate_id: &str) -> ModelResolutionEvidence {
    ModelResolutionEvidence {
        phase: ModelResolutionPhase::Setup,
        interface_id: "traverse.inference.generate".to_string(),
        requested_interface_id: "traverse.inference.generate".to_string(),
        requested_placement: ExecutionTarget::Local,
        selected: Some(SelectedModelCandidate {
            candidate_id: candidate_id.to_string(),
            provider_capability_id: "traverse.inference.generate".to_string(),
            provider_implementation_id: "ollama.local.generate".to_string(),
            model_identifier: "llama3.2:3b".to_string(),
            placement_target: ExecutionTarget::Local,
            priority: 10,
            selection_reason: "selected highest-priority passing candidate".to_string(),
        }),
        candidates: Vec::new(),
        failure_code: Option::<ModelCandidateRejectionCode>::None,
    }
}

fn register_event_fixture(registry: &mut EventRegistry, event_dir: &str, expected_id: &str) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../../contracts/examples/expedition/events/{event_dir}/contract.json"
    ));
    let contents = fs::read_to_string(&path).expect("event contract fixture should read");
    let contract = parse_event_contract(&contents).expect("event contract fixture should parse");
    assert_eq!(contract.id, expected_id);
    registry
        .register(EventRegistration {
            scope: RegistryScope::Private,
            contract,
            contract_path: path.display().to_string(),
            registered_at: "2026-06-13T00:00:00Z".to_string(),
            governing_spec: "011-event-registry".to_string(),
            validator_version: "test".to_string(),
        })
        .expect("event fixture should register");
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(output, "{byte:02x}");
    }
    output
}
