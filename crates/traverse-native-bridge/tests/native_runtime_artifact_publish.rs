#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! Proves a real, built `runtime.wasm` (not a synthetic fixture) can be
//! published into the native runtime artifact distribution registry (spec
//! 075) via `publish_native_runtime_artifact` and resolved back out via
//! `resolve_native_runtime_artifact` — end to end, against real artifact
//! bytes and a real on-disk registry file.

use std::fs;
use std::process::Command;

use traverse_registry::{load_native_runtime_registry, resolve_native_runtime_artifact};

#[test]
fn publishes_and_resolves_the_real_built_artifact() {
    let workspace_root = std::env::temp_dir().join(format!(
        "traverse-native-runtime-publish-test-{}",
        std::process::id()
    ));
    fs::create_dir_all(&workspace_root).expect("scratch workspace root should be created");

    let runtime_dir = workspace_root.join("scratch-runtime");
    let build_status = Command::new(env!("CARGO_BIN_EXE_traverse-native-bridge"))
        .arg(&runtime_dir)
        .status()
        .expect("traverse-native-bridge must run");
    assert!(build_status.success(), "building runtime.wasm failed");

    let host_certifications_path = workspace_root.join("host-certifications.json");
    fs::write(
        &host_certifications_path,
        serde_json::json!([
            {"host": "swift", "engine_name": "wasmi", "engine_version": "2.0.0", "conformance_passed": true},
            {"host": "kotlin", "engine_name": "Chicory", "engine_version": "1.7.5", "conformance_passed": true},
            {"host": "dotnet", "engine_name": "Wasmtime", "engine_version": "44.0.0", "conformance_passed": true},
        ])
        .to_string(),
    )
    .expect("host certifications fixture should write");

    let publish_status = Command::new(env!("CARGO_BIN_EXE_publish_native_runtime_artifact"))
        .arg(&runtime_dir)
        .arg(&host_certifications_path)
        .arg("https://example.invalid/runtime.wasm")
        .arg(&workspace_root)
        .status()
        .expect("publish_native_runtime_artifact must run");
    assert!(publish_status.success(), "publishing the artifact failed");

    let release_bytes =
        fs::read(runtime_dir.join("runtime-release.json")).expect("release metadata should read");
    let release: serde_json::Value =
        serde_json::from_slice(&release_bytes).expect("release metadata should parse");
    let runtime_version = release["runtime_version"].as_str().unwrap();
    let sha256 = release["sha256"].as_str().unwrap();

    let index = load_native_runtime_registry(&workspace_root).expect("registry should load");
    assert_eq!(index.releases.len(), 1);

    for host in ["swift", "kotlin", "dotnet"] {
        let resolved = resolve_native_runtime_artifact(
            &index,
            runtime_version,
            sha256,
            ">=1.1.0,<2.0.0",
            host,
        )
        .unwrap_or_else(|error| panic!("resolution should pass for {host}: {error:?}"));
        assert_eq!(resolved.runtime_version, runtime_version);
        assert_eq!(resolved.sha256, sha256);
    }

    let tampered = resolve_native_runtime_artifact(
        &index,
        runtime_version,
        "0".repeat(64).as_str(),
        ">=1.1.0,<2.0.0",
        "swift",
    );
    assert!(
        tampered.is_err(),
        "a mismatched digest must fail resolution"
    );

    fs::remove_dir_all(&workspace_root).ok();
}
