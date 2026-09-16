//! Publishes a built `runtime.wasm`'s metadata into the native runtime
//! artifact distribution registry (spec 075). Consumes the
//! `runtime-release.json` the primary `traverse-native-bridge` binary
//! writes, plus a caller-supplied host-certification evidence file (spec
//! 075's Host Certification Evidence, produced from a real conformance run —
//! see `scripts/ci/native_artifact_certification.sh`), and records one
//! immutable release into `<workspace_root>/runtime/native-runtime-registry.json`.

use serde::Deserialize;
use std::env;
use std::fs;
use std::path::PathBuf;
use traverse_registry::{
    HostCertification, NativeRuntimeArtifactRecord, load_native_runtime_registry,
    publish_native_runtime_artifact, write_native_runtime_registry,
};

#[derive(Deserialize)]
struct RuntimeRelease {
    runtime_version: String,
    bridge_version: String,
    sha256: String,
}

/// Spec 075's `supported_bridge_range`: every release sharing `bridge_version`'s
/// major component satisfies compatibility, matching spec 071's own ABI
/// stability contract (a major bump is the only breaking-change boundary).
fn supported_bridge_range(bridge_version: &str) -> Result<String, String> {
    let major_str = bridge_version
        .split('.')
        .next()
        .ok_or_else(|| format!("bridge_version {bridge_version} has no major component"))?;
    let major: u64 = major_str
        .parse()
        .map_err(|_| format!("bridge_version {bridge_version} major component is not numeric"))?;
    Ok(format!(">={bridge_version},<{}.0.0", major + 1))
}

fn main() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let usage = "usage: publish_native_runtime_artifact <runtime_dir> <host_certifications_json> <artifact_url> [workspace_root]";
    let runtime_dir = PathBuf::from(args.next().ok_or_else(|| usage.to_string())?);
    let host_certifications_path = PathBuf::from(args.next().ok_or_else(|| usage.to_string())?);
    let artifact_url = args.next().ok_or_else(|| usage.to_string())?;
    let workspace_root = args
        .next()
        .map_or_else(|| PathBuf::from("."), PathBuf::from);

    let release_bytes = fs::read(runtime_dir.join("runtime-release.json"))
        .map_err(|error| format!("read runtime-release.json: {error}"))?;
    let release: RuntimeRelease = serde_json::from_slice(&release_bytes)
        .map_err(|error| format!("parse runtime-release.json: {error}"))?;

    let host_certifications_bytes = fs::read(&host_certifications_path)
        .map_err(|error| format!("read {}: {error}", host_certifications_path.display()))?;
    let host_certifications: Vec<HostCertification> =
        serde_json::from_slice(&host_certifications_bytes)
            .map_err(|error| format!("parse {}: {error}", host_certifications_path.display()))?;

    let record = NativeRuntimeArtifactRecord {
        runtime_version: release.runtime_version.clone(),
        bridge_version: release.bridge_version.clone(),
        supported_bridge_range: supported_bridge_range(&release.bridge_version)?,
        sha256: release.sha256,
        artifact_url,
        host_certifications,
    };

    let mut index = load_native_runtime_registry(&workspace_root)
        .map_err(|error| format!("load registry: {error:?}"))?;
    publish_native_runtime_artifact(&mut index, record)
        .map_err(|error| format!("publish: {error:?}"))?;
    write_native_runtime_registry(&workspace_root, &index)
        .map_err(|error| format!("write registry: {error:?}"))?;

    println!(
        "published runtime_version {} (bridge {}) to {}",
        release.runtime_version,
        release.bridge_version,
        workspace_root
            .join("runtime/native-runtime-registry.json")
            .display(),
    );
    Ok(())
}
