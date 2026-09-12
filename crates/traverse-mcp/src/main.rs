use std::path::PathBuf;
use std::process::ExitCode;
use traverse_mcp::{prepare_verified_cache, run_stdio_server};

fn main() -> ExitCode {
    run(std::env::args().skip(1))
}

const USAGE: &str = "Usage: traverse-mcp stdio [--cache <dir>] [--simulate-startup-failure]\n       traverse-mcp prepare-cache --synced-state <path> --cache <dir> [--ref <namespace/id@version_range>]... [--json]";

/// Testable core of [`main`]: takes the argument iterator directly instead of
/// reading `std::env::args()` so the CLI parsing branches can be exercised
/// without spawning a subprocess.
fn run(args: impl Iterator<Item = String>) -> ExitCode {
    match parse_command(args) {
        Err(message) => {
            eprintln!("{message}");
            ExitCode::from(1)
        }
        Ok(Command::Stdio {
            simulate_startup_failure,
            cache,
        }) => match run_stdio_server(simulate_startup_failure, cache) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("traverse-mcp stdio server failed: {error:?}");
                ExitCode::from(1)
            }
        },
        Ok(Command::PrepareCache {
            synced_state,
            cache,
            refs,
            json_output,
        }) => match prepare_verified_cache(&synced_state, &cache, &refs) {
            Ok(evidence) => {
                if json_output {
                    println!("{}", evidence.envelope());
                } else {
                    println!("{}", evidence.render());
                }
                ExitCode::SUCCESS
            }
            Err(error) => {
                if json_output {
                    println!("{}", error.envelope());
                } else {
                    eprintln!("{error}");
                }
                ExitCode::from(1)
            }
        },
    }
}

enum Command {
    Stdio {
        simulate_startup_failure: bool,
        cache: Option<PathBuf>,
    },
    PrepareCache {
        synced_state: PathBuf,
        cache: PathBuf,
        refs: Vec<String>,
        json_output: bool,
    },
}

fn parse_command(mut args: impl Iterator<Item = String>) -> Result<Command, String> {
    let Some(command) = args.next() else {
        return Err(USAGE.to_string());
    };
    match command.as_str() {
        "stdio" => parse_stdio(args),
        "prepare-cache" => parse_prepare_cache(args),
        other => Err(format!("Unsupported command: {other}")),
    }
}

fn parse_stdio(mut args: impl Iterator<Item = String>) -> Result<Command, String> {
    let mut simulate_startup_failure = false;
    let mut cache = None;
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--simulate-startup-failure" => simulate_startup_failure = true,
            "--cache" => {
                let value = args
                    .next()
                    .ok_or_else(|| "stdio --cache requires <dir>".to_string())?;
                cache = Some(PathBuf::from(value));
            }
            other => return Err(format!("Unsupported stdio flag: {other}")),
        }
    }
    Ok(Command::Stdio {
        simulate_startup_failure,
        cache,
    })
}

fn parse_prepare_cache(mut args: impl Iterator<Item = String>) -> Result<Command, String> {
    let mut synced_state = None;
    let mut cache = None;
    let mut refs = Vec::new();
    let mut json_output = false;
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--synced-state" => {
                let value = args
                    .next()
                    .ok_or_else(|| "prepare-cache --synced-state requires <path>".to_string())?;
                synced_state = Some(PathBuf::from(value));
            }
            "--cache" => {
                let value = args
                    .next()
                    .ok_or_else(|| "prepare-cache --cache requires <dir>".to_string())?;
                cache = Some(PathBuf::from(value));
            }
            "--ref" => {
                let value = args.next().ok_or_else(|| {
                    "prepare-cache --ref requires <namespace/id@version_range>".to_string()
                })?;
                refs.push(value);
            }
            "--json" => json_output = true,
            other => return Err(format!("Unsupported prepare-cache flag: {other}")),
        }
    }
    Ok(Command::PrepareCache {
        synced_state: synced_state
            .ok_or_else(|| "prepare-cache requires --synced-state <path>".to_string())?,
        cache: cache.ok_or_else(|| "prepare-cache requires --cache <dir>".to_string())?,
        refs,
        json_output,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn no_command_prints_usage_and_exits_with_failure() {
        assert_eq!(run(std::iter::empty()), ExitCode::from(1));
    }

    #[test]
    fn unsupported_command_exits_with_failure() {
        assert_eq!(run(["bogus".to_string()].into_iter()), ExitCode::from(1));
    }

    #[test]
    fn stdio_command_with_simulated_startup_failure_exits_with_failure() {
        assert_eq!(
            run([
                "stdio".to_string(),
                "--simulate-startup-failure".to_string()
            ]
            .into_iter()),
            ExitCode::from(1)
        );
    }

    #[test]
    fn stdio_rejects_unknown_flags() {
        assert_eq!(
            run(["stdio".to_string(), "--unknown".to_string()].into_iter()),
            ExitCode::from(1)
        );
    }

    #[test]
    fn stdio_cache_requires_a_directory() {
        assert_eq!(
            run(["stdio".to_string(), "--cache".to_string()].into_iter()),
            ExitCode::from(1)
        );
    }

    #[test]
    fn prepare_cache_requires_synced_state_and_cache() {
        assert_eq!(
            run(["prepare-cache".to_string()].into_iter()),
            ExitCode::from(1)
        );
        assert_eq!(
            run([
                "prepare-cache".to_string(),
                "--synced-state".to_string(),
                "/tmp/missing-synced-state.json".to_string()
            ]
            .into_iter()),
            ExitCode::from(1)
        );
        assert_eq!(
            run([
                "prepare-cache".to_string(),
                "--cache".to_string(),
                "/tmp/missing-mode-b-cache".to_string()
            ]
            .into_iter()),
            ExitCode::from(1)
        );
    }

    #[test]
    fn prepare_cache_rejects_unknown_flags_and_missing_ref_value() {
        assert_eq!(
            run([
                "prepare-cache".to_string(),
                "--synced-state".to_string(),
                "state.json".to_string(),
                "--cache".to_string(),
                "cache".to_string(),
                "--bogus".to_string()
            ]
            .into_iter()),
            ExitCode::from(1)
        );
        assert_eq!(
            run([
                "prepare-cache".to_string(),
                "--synced-state".to_string(),
                "state.json".to_string(),
                "--cache".to_string(),
                "cache".to_string(),
                "--ref".to_string()
            ]
            .into_iter()),
            ExitCode::from(1)
        );
    }

    #[test]
    fn parse_stdio_accepts_cache_flag() {
        let command = parse_command(
            [
                "stdio".to_string(),
                "--cache".to_string(),
                "/srv/cache".to_string(),
            ]
            .into_iter(),
        )
        .expect("parse");
        match command {
            Command::Stdio {
                simulate_startup_failure,
                cache,
            } => {
                assert!(!simulate_startup_failure);
                assert_eq!(cache, Some(PathBuf::from("/srv/cache")));
            }
            Command::PrepareCache { .. } => panic!("expected stdio"),
        }
    }

    #[test]
    fn prepare_cache_flag_values_are_required() {
        assert_eq!(
            run(["prepare-cache".to_string(), "--synced-state".to_string()].into_iter()),
            ExitCode::from(1)
        );
        assert_eq!(
            run([
                "prepare-cache".to_string(),
                "--synced-state".to_string(),
                "state.json".to_string(),
                "--cache".to_string()
            ]
            .into_iter()),
            ExitCode::from(1)
        );
    }

    #[test]
    fn prepare_cache_run_reports_json_and_text_failures() {
        assert_eq!(
            run([
                "prepare-cache".to_string(),
                "--synced-state".to_string(),
                "/tmp/missing-traverse-mode-b-state.json".to_string(),
                "--cache".to_string(),
                "/tmp/missing-traverse-mode-b-cache".to_string(),
                "--json".to_string()
            ]
            .into_iter()),
            ExitCode::from(1)
        );
        assert_eq!(
            run([
                "prepare-cache".to_string(),
                "--synced-state".to_string(),
                "/tmp/missing-traverse-mode-b-state.json".to_string(),
                "--cache".to_string(),
                "/tmp/missing-traverse-mode-b-cache".to_string()
            ]
            .into_iter()),
            ExitCode::from(1)
        );
    }

    #[test]
    fn prepare_cache_run_prepares_a_local_file_snapshot() {
        let root = std::env::temp_dir().join(format!(
            "traverse-mcp-mode-b-cli-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).expect("temp");
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("workspace")
            .to_path_buf();
        let wasm = repo.join(
            "examples/core-normalize-participants/artifacts/core-normalize-participants.wasm",
        );
        let contract = repo.join("examples/core-normalize-participants/contract.json");
        let wasm_bytes = std::fs::read(&wasm).expect("wasm");
        let contract_bytes = std::fs::read(&contract).expect("contract");
        let digest = |bytes: &[u8]| {
            use sha2::{Digest, Sha256};
            let hashed = Sha256::digest(bytes);
            let mut out = String::from("sha256:");
            for byte in hashed {
                use std::fmt::Write as _;
                let _ = write!(out, "{byte:02x}");
            }
            out
        };
        let snapshot = serde_json::json!({
            "schema_version": "1.0.0",
            "workspace_id": "mode-b-cli",
            "state_scope": "public_registry_synced",
            "source_repo": "traverse-framework/registry",
            "release_tag": "index-v1",
            "index_version": 1,
            "generated_at": "2026-09-11T00:00:00Z",
            "source_commit": null,
            "synced_at": "2026-09-11T00:00:00Z",
            "record_count": 1,
            "validation_status": "valid",
            "governing_spec": "055-registry-sync",
            "capabilities": [{
                "namespace": "core",
                "id": "core.normalize-participants",
                "version": "1.1.0",
                "digest": digest(&wasm_bytes),
                "artifact_url": format!("file://{}", wasm.display()),
                "contract_digest": digest(&contract_bytes),
                "contract_url": format!("file://{}", contract.display()),
                "deprecated": false
            }],
            "events": []
        });
        let state = root.join("state.json");
        std::fs::write(&state, serde_json::to_vec(&snapshot).expect("json")).expect("write");
        let cache = root.join("cache");
        assert_eq!(
            run([
                "prepare-cache".to_string(),
                "--synced-state".to_string(),
                state.display().to_string(),
                "--cache".to_string(),
                cache.display().to_string(),
                "--json".to_string()
            ]
            .into_iter()),
            ExitCode::SUCCESS
        );
        assert_eq!(
            run([
                "prepare-cache".to_string(),
                "--synced-state".to_string(),
                state.display().to_string(),
                "--cache".to_string(),
                cache.display().to_string()
            ]
            .into_iter()),
            ExitCode::SUCCESS
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn parse_prepare_cache_accepts_refs_and_json() {
        let command = parse_command(
            [
                "prepare-cache".to_string(),
                "--synced-state".to_string(),
                "state.json".to_string(),
                "--cache".to_string(),
                "cache".to_string(),
                "--ref".to_string(),
                "core/core.normalize-participants@=1.1.0".to_string(),
                "--json".to_string(),
            ]
            .into_iter(),
        )
        .expect("parse");
        match command {
            Command::PrepareCache {
                synced_state,
                cache,
                refs,
                json_output,
            } => {
                assert_eq!(synced_state, PathBuf::from("state.json"));
                assert_eq!(cache, PathBuf::from("cache"));
                assert_eq!(refs, vec!["core/core.normalize-participants@=1.1.0"]);
                assert!(json_output);
            }
            Command::Stdio { .. } => panic!("expected prepare-cache"),
        }
    }
}
