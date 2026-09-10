//! Regression for issue #1336: released registry `wasi-command` audio planners
//! must execute through `ArtifactRouter` on the registered `target: local` path
//! with concrete failure messages (not a collapsed generic string).
#![cfg(feature = "wasmtime-executor")]
#![allow(clippy::expect_used)]

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use traverse_registry::{
    ArtifactDigests, BinaryFormat, BinaryReference, CapabilityArtifactRecord,
    CapabilityRegistration, CapabilityRegistry, ComposabilityMetadata, CompositionKind,
    CompositionPattern, ImplementationKind, LookupScope, RegistryProvenance, RegistryScope,
    ResolvedCapability, SourceKind, SourceReference,
};
use traverse_runtime::{ArtifactRouter, LocalExecutor};

const FIXTURE_DIR: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/registry-audio-plans"
);

const CAPTURE_DIGEST: &str = "10fcd7c6e53d0f16cf396b2601864c85436d686e79dee4ae8ffec50b083a6baf";
const TRANSFORM_DIGEST: &str = "b706c69e1c945efa12e968479a6959dc551b7dba2b376bb48f5bb0f4fddacb60";
const WINDOW_DIGEST: &str = "ecc7aba24eee918106999ab89cbcacda212502aae32f8692d35b00565a95dca4";
const VALIDATE_DIGEST: &str = "ad440e4782bc9418c59d422e4a34d3524130978a4bef049212a2ad346cd8775d";

#[test]
fn artifact_router_runs_released_audio_capture_plan_and_negative_bounds() {
    let path = load_released_artifact(
        "core.create-audio-capture-request-plan",
        "core-create-audio-capture-request-plan.wasm",
        CAPTURE_DIGEST,
    );
    let capability = resolved_capability(
        "core.create-audio-capture-request-plan",
        &path,
        CAPTURE_DIGEST,
    );
    let router = ArtifactRouter::new().expect("ArtifactRouter should initialize");

    let planned = router
        .execute(
            &capability,
            &json!({
                "request_id": "capture-e2e-001",
                "source_profile_ref": "profile:outdoor-mono-48k",
                "duration_seconds": 900,
                "idempotency_key": "capture-e2e-001"
            }),
        )
        .expect("capture plan should succeed through ArtifactRouter");
    assert_eq!(planned.value["result_class"], "planned");
    assert_eq!(planned.value["connector_id"], "traverse.audio-input");
    assert_eq!(planned.value["operation"], "capture_segment");

    let rejected = router
        .execute(
            &capability,
            &json!({
                "request_id": "capture-e2e-001",
                "source_profile_ref": "profile:outdoor-mono-48k",
                "duration_seconds": 3601,
                "idempotency_key": "capture-e2e-001"
            }),
        )
        .expect("out-of-bounds capture plan should still return JSON");
    assert_eq!(rejected.value["result_class"], "duration_out_of_bounds");
}

#[test]
fn artifact_router_runs_sibling_audio_plan_artifacts() {
    let router = ArtifactRouter::new().expect("ArtifactRouter should initialize");

    let transform_path = load_released_artifact(
        "core.create-audio-transform-plan",
        "core-create-audio-transform-plan.wasm",
        TRANSFORM_DIGEST,
    );
    let transform = router
        .execute(
            &resolved_capability(
                "core.create-audio-transform-plan",
                &transform_path,
                TRANSFORM_DIGEST,
            ),
            &json!({
                "source_artifact_ref": "asset:x",
                "source_media_type": "audio/flac",
                "target_sample_rate_hz": 48_000,
                "target_channel_count": 1,
                "target_encoding": "pcm_s16le",
                "idempotency_key": "transform-e2e-001"
            }),
        )
        .expect("transform plan should succeed");
    assert_eq!(transform.value["result_class"], "planned");
    assert_eq!(transform.value["operation"], "transform");

    let window_path = load_released_artifact(
        "core.create-audio-window-plan",
        "core-create-audio-window-plan.wasm",
        WINDOW_DIGEST,
    );
    let window = router
        .execute(
            &resolved_capability(
                "core.create-audio-window-plan",
                &window_path,
                WINDOW_DIGEST,
            ),
            &json!({
                "artifact_ref": "asset:pcm-17",
                "duration_millis": 60_000,
                "window_policy": {
                    "policy_version": "v1",
                    "window_millis": 5_000,
                    "hop_millis": 2_500
                }
            }),
        )
        .expect("window plan should succeed");
    assert_eq!(window.value["result_class"], "planned");
    assert_eq!(window.value["window_count"], 23.0);

    let validate_path = load_released_artifact(
        "core.validate-audio-source-profile",
        "core-validate-audio-source-profile.wasm",
        VALIDATE_DIGEST,
    );
    let validate = router
        .execute(
            &resolved_capability(
                "core.validate-audio-source-profile",
                &validate_path,
                VALIDATE_DIGEST,
            ),
            &json!({
                "source_profile": {
                    "sample_rate_hz": 48_000,
                    "channel_count": 1,
                    "bit_depth": 16,
                    "encoding": "pcm_s16le"
                },
                "capture_policy": {
                    "policy_version": "v1",
                    "max_segment_seconds": 900,
                    "requested_segment_seconds": 900
                }
            }),
        )
        .expect("validate profile should succeed");
    assert_eq!(validate.value["valid"], true);
    assert_eq!(validate.value["reason_code"], "source_profile_compatible");
}

#[test]
fn wasm_executor_still_traps_released_capture_plan_under_eight_mib_cap() {
    use traverse_runtime::executor::{ExecutorError, WasmExecutionLimits, WasmExecutor};

    let path = load_released_artifact(
        "core.create-audio-capture-request-plan",
        "core-create-audio-capture-request-plan.wasm",
        CAPTURE_DIGEST,
    );
    let bytes = fs::read(&path).expect("fixture bytes");
    let executor = WasmExecutor::with_limits(WasmExecutionLimits {
        memory_bytes: 8 * 1024 * 1024,
        ..WasmExecutionLimits::default()
    })
    .expect("executor");
    let err = executor
        .run_bytes(
            &bytes,
            &json!({
                "request_id": "capture-e2e-001",
                "source_profile_ref": "profile:outdoor-mono-48k",
                "duration_seconds": 900,
                "idempotency_key": "capture-e2e-001"
            }),
        )
        .expect_err("8 MiB must still trap this module");
    match err {
        ExecutorError::ResourceExhausted(detail) => {
            assert!(
                detail.contains("17891328") || detail.contains("forcing trap"),
                "expected concrete memory trap detail, got {detail}"
            );
        }
        other => panic!("expected ResourceExhausted, got {other:?}"),
    }
}

fn load_released_artifact(capability_id: &str, asset_name: &str, expected_digest: &str) -> PathBuf {
    let fixture = PathBuf::from(FIXTURE_DIR).join(asset_name);
    if std::env::var_os("TRAVERSE_FETCH_REGISTRY_ARTIFACTS").is_some() {
        let url = format!(
            "https://github.com/traverse-framework/registry/releases/download/artifacts/{capability_id}-1.0.0/{asset_name}"
        );
        let status = Command::new("curl")
            .args(["-fsSL", "-o"])
            .arg(&fixture)
            .arg(&url)
            .status()
            .unwrap_or_else(|error| panic!("curl failed to start for {url}: {error}"));
        assert!(
            status.success(),
            "failed to fetch released artifact {url} (exit {status})"
        );
    }
    assert!(
        fixture.is_file(),
        "missing released artifact fixture at {}",
        fixture.display()
    );
    let bytes = fs::read(&fixture).unwrap_or_else(|error| {
        panic!("failed to read {}: {error}", fixture.display());
    });
    let actual = sha256_hex(&bytes);
    assert_eq!(
        actual, expected_digest,
        "digest mismatch for {} ({})",
        fixture.display(),
        capability_id
    );
    fixture
}

fn resolved_capability(
    capability_id: &str,
    wasm_path: &Path,
    digest_hex: &str,
) -> ResolvedCapability {
    let (namespace, name) = capability_id
        .split_once('.')
        .unwrap_or(("core", capability_id));
    let mut contract_value: Value = serde_json::from_str(include_str!(
        "../../../contracts/examples/hello-world/capabilities/say-hello/contract.json"
    ))
    .expect("hello-world contract should parse");
    contract_value["id"] = Value::String(capability_id.to_string());
    contract_value["namespace"] = Value::String(namespace.to_string());
    contract_value["name"] = Value::String(name.to_string());
    contract_value["version"] = Value::String("1.0.0".to_string());

    let contract: traverse_contracts::CapabilityContract =
        serde_json::from_value(contract_value).expect("patched contract should deserialize");

    let mut registry = CapabilityRegistry::new();
    registry
        .register(CapabilityRegistration {
            scope: RegistryScope::Public,
            contract,
            contract_path: format!("registry://{capability_id}/1.0.0/contract.json"),
            artifact: CapabilityArtifactRecord {
                artifact_ref: format!("artifact:{capability_id}:1.0.0"),
                implementation_kind: ImplementationKind::Executable,
                source: SourceReference {
                    kind: SourceKind::Local,
                    location: "registry-audio-plans".to_string(),
                },
                binary: Some(BinaryReference {
                    format: BinaryFormat::Wasm,
                    location: wasm_path.to_string_lossy().into_owned(),
                    signature: None,
                }),
                workflow_ref: None,
                digests: ArtifactDigests {
                    source_digest: format!("sha256:{digest_hex}"),
                    binary_digest: Some(format!("sha256:{digest_hex}")),
                },
                provenance: RegistryProvenance {
                    source: "traverse-framework/registry".to_string(),
                    author: "registry".to_string(),
                    created_at: "2026-09-10T00:00:00Z".to_string(),
                },
            },
            registered_at: "2026-09-10T00:00:00Z".to_string(),
            tags: Vec::new(),
            composability: ComposabilityMetadata {
                kind: CompositionKind::Atomic,
                patterns: vec![CompositionPattern::Sequential],
                provides: Vec::new(),
                requires: Vec::new(),
            },
            governing_spec: "064-production-artifact-execution".to_string(),
            validator_version: "test".to_string(),
        })
        .expect("fixture capability should register");
    registry
        .find_exact(LookupScope::PublicOnly, capability_id, "1.0.0")
        .expect("registered fixture capability should resolve")
}

fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher
        .finalize()
        .iter()
        .fold(String::new(), |mut acc, byte| {
            let _ = write!(acc, "{byte:02x}");
            acc
        })
}
