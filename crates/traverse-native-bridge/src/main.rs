use sha2::{Digest, Sha256};
use std::env;
use std::fmt::Write;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

// Tracks `crates/traverse-runtime-wasm`'s `BRIDGE_ABI_VERSION` /
// `runtime-wasm-bridge/1.0.0` identity (spec 071 FR-006). Not read back from
// the built artifact itself: both are declared, independently-versioned
// constants of the same governed ABI, so a drift between them is a bug this
// builder does not paper over.
const BRIDGE_VERSION: &str = "1.1.0";
const ABI_VERSION: u32 = 10_100;

fn workspace_root() -> Result<PathBuf, String> {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .map(std::path::Path::to_path_buf)
        .ok_or_else(|| "CARGO_MANIFEST_DIR has no workspace-root ancestor".to_string())
}

/// Builds the real `crates/traverse-runtime-wasm` `wasm32-unknown-unknown`
/// artifact and returns its bytes. Uses a dedicated `CARGO_TARGET_DIR` and
/// strips rustc-wrapper env so this nested build never races the outer
/// build's own target-dir lock or inherits an outer `cargo llvm-cov`
/// coverage-instrumentation wrapper, which fails wasm32 cross-compiles (no
/// `profiler_builtins`) — the same hazard
/// `crates/traverse-runtime/tests/runtime_wasm_host_tests.rs` guards against.
fn build_runtime_wasm_bytes() -> Result<Vec<u8>, String> {
    let root = workspace_root()?;
    let target_dir = root.join("target/native-bridge-build");
    let status = Command::new("cargo")
        .args([
            "build",
            "--release",
            "-p",
            "traverse-runtime-wasm",
            "--target",
            "wasm32-unknown-unknown",
        ])
        .env("CARGO_TARGET_DIR", &target_dir)
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("RUSTC_WRAPPER")
        .env_remove("RUSTC_WORKSPACE_WRAPPER")
        .current_dir(&root)
        .status()
        .map_err(|error| format!("cargo build -p traverse-runtime-wasm: {error}"))?;
    if !status.success() {
        return Err("building traverse-runtime-wasm failed".to_string());
    }
    let artifact = target_dir
        .join("wasm32-unknown-unknown")
        .join("release")
        .join("traverse_runtime_wasm.wasm");
    fs::read(&artifact).map_err(|error| format!("read {}: {error}", artifact.display()))
}

fn main() -> Result<(), String> {
    let destination = env::args()
        .nth(1)
        .map_or_else(|| PathBuf::from("runtime"), PathBuf::from);
    fs::create_dir_all(&destination).map_err(|error| error.to_string())?;
    let bytes = build_runtime_wasm_bytes()?;
    let mut digest = String::with_capacity(64);
    for byte in Sha256::digest(&bytes) {
        write!(&mut digest, "{byte:02x}").map_err(|error| error.to_string())?;
    }
    fs::write(destination.join("runtime.wasm"), bytes).map_err(|error| error.to_string())?;
    fs::write(
        destination.join("runtime-release.json"),
        format!(
            "{{\"runtime_version\":\"{}\",\"bridge_version\":\"{}\",\"bridge_abi_version\":{},\"sha256\":\"{}\"}}\n",
            env!("CARGO_PKG_VERSION"),
            BRIDGE_VERSION,
            ABI_VERSION,
            digest
        ),
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}
