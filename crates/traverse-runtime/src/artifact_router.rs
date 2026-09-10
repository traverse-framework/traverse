use crate::executor::ExecutorError;
#[cfg(feature = "wasmtime-executor")]
use crate::executor::{ArtifactType, CapabilityExecutor, ExecutorCapability, WasmExecutor};
use crate::{
    LocalExecutionFailure, LocalExecutionFailureCode, LocalExecutionOutput, LocalExecutor,
};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;
use traverse_registry::ResolvedCapability;

type NativeHandler =
    dyn Fn(&Value) -> Result<LocalExecutionOutput, LocalExecutionFailure> + Send + Sync;

/// Production local-execution boundary for registered artifacts.
///
/// WASM executes only from the resolved registered artifact. Native execution
/// is limited to explicitly registered host handlers and never loads a binary
/// or command from artifact metadata.
#[derive(Clone)]
pub struct ArtifactRouter {
    #[cfg(feature = "wasmtime-executor")]
    wasm: Arc<WasmExecutor>,
    native_handlers: BTreeMap<String, Arc<NativeHandler>>,
}

impl ArtifactRouter {
    /// Creates a router using the default bounded Wasmtime configuration.
    ///
    /// # Errors
    ///
    /// Returns an execution failure when the Wasmtime runtime cannot initialize.
    pub fn new() -> Result<Self, LocalExecutionFailure> {
        #[cfg(feature = "wasmtime-executor")]
        {
            WasmExecutor::new()
                .map(|wasm| Self {
                    wasm: Arc::new(wasm),
                    native_handlers: BTreeMap::new(),
                })
                .map_err(|error| map_executor_error(&error))
        }
        #[cfg(not(feature = "wasmtime-executor"))]
        {
            Ok(Self {
                native_handlers: BTreeMap::new(),
            })
        }
    }

    /// Registers one host-provided native handler for an exact capability id.
    pub fn register_native_handler<F>(&mut self, capability_id: impl Into<String>, handler: F)
    where
        F: Fn(&Value) -> Result<LocalExecutionOutput, LocalExecutionFailure>
            + Send
            + Sync
            + 'static,
    {
        self.native_handlers
            .insert(capability_id.into(), Arc::new(handler));
    }
}

impl LocalExecutor for ArtifactRouter {
    fn execute(
        &self,
        capability: &ResolvedCapability,
        input: &Value,
    ) -> Result<LocalExecutionOutput, LocalExecutionFailure> {
        if let Some(binary) = &capability.artifact.binary {
            #[cfg(feature = "wasmtime-executor")]
            {
                let executor_capability = ExecutorCapability {
                    capability_id: capability.contract.id.clone(),
                    artifact_type: ArtifactType::Wasm,
                    wasm_binary_path: Some(binary.location.clone()),
                    wasm_checksum: capability
                        .artifact
                        .digests
                        .binary_digest
                        .as_deref()
                        .and_then(|digest| digest.strip_prefix("sha256:"))
                        .map(str::to_string),
                    host_abi_version: None,
                    emits: capability.contract.emits.clone(),
                    service_type: capability.contract.service_type.clone(),
                    state_schema: capability.contract.state_schema.clone(),
                };
                // Events emitted via `traverse_host::emit_event` during this
                // call are returned as real `LocalExecutionOutput.emitted_events`
                // (spec 101-local-executor-event-emission FR-003) — already
                // ABI-validated by `WasmExecutor`. `ArtifactRouter` itself
                // does not publish them (FR-004): it is used both directly
                // by `workflows.rs` and, via `BoundLocalExecutor`, by
                // `PlacementRouter` Step 5, so publishing here would
                // double-publish on the live `Runtime::execute()` path.
                return self
                    .wasm
                    .execute(&executor_capability, input)
                    .map(|output| LocalExecutionOutput {
                        value: output.value,
                        emitted_events: output.emitted_events,
                    })
                    .map_err(|error| map_executor_error(&error));
            }
            #[cfg(not(feature = "wasmtime-executor"))]
            {
                let _ = binary;
                return Err(constraint_failure(
                    "WASM execution is unavailable in this runtime build",
                ));
            }
        }
        self.native_handlers
            .get(&capability.contract.id)
            .ok_or_else(|| constraint_failure("native capability has no explicit host handler"))?(
            input,
        )
    }
}

fn map_executor_error(error: &ExecutorError) -> LocalExecutionFailure {
    let code = match error {
        ExecutorError::Timeout(_) => LocalExecutionFailureCode::Timeout,
        ExecutorError::ResourceExhausted(_) => LocalExecutionFailureCode::ResourceExhausted,
        ExecutorError::ChecksumMismatch { .. }
        | ExecutorError::MalformedWasmArtifact { .. }
        | ExecutorError::UnsupportedAbiVersion { .. }
        | ExecutorError::UnauthorizedHostImport { .. }
        | ExecutorError::BinaryLoadFailed(_)
        | ExecutorError::RuntimeSetupFailed(_) => LocalExecutionFailureCode::ConstraintViolated,
        ExecutorError::OutputDeserializationFailed(_) => LocalExecutionFailureCode::InvalidInput,
        ExecutorError::ExecutionFailed(_) | ExecutorError::UnsupportedArtifactType => {
            LocalExecutionFailureCode::ExecutionFailed
        }
    };
    // Keep stable LocalExecutionFailureCode classification (spec 064 FR-006)
    // while surfacing the concrete executor cause — trap text, guest exit,
    // instantiation/missing-export detail, or resource-limit message — so
    // registered `wasi-command` failures are diagnosable (issue #1336).
    LocalExecutionFailure {
        code,
        message: error.to_string(),
    }
}

fn constraint_failure(message: &str) -> LocalExecutionFailure {
    LocalExecutionFailure {
        code: LocalExecutionFailureCode::ConstraintViolated,
        message: message.to_string(),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;
    use traverse_registry::{
        ArtifactDigests, BinaryFormat, BinaryReference, CapabilityArtifactRecord,
        CapabilityRegistration, CapabilityRegistry, ComposabilityMetadata, CompositionKind,
        CompositionPattern, ImplementationKind, LookupScope, RegistryProvenance, RegistryScope,
        SourceKind, SourceReference,
    };

    fn resolved_capability(binary: Option<BinaryReference>) -> ResolvedCapability {
        let contract = serde_json::from_str(include_str!(
            "../../../contracts/examples/hello-world/capabilities/say-hello/contract.json"
        ))
        .expect("checked-in capability contract should parse");
        let mut registry = CapabilityRegistry::new();
        registry
            .register(CapabilityRegistration {
                scope: RegistryScope::Public,
                contract,
                contract_path:
                    "contracts/examples/hello-world/capabilities/say-hello/contract.json"
                        .to_string(),
                artifact: CapabilityArtifactRecord {
                    artifact_ref: "artifact:hello.world.say-hello:1.0.0".to_string(),
                    implementation_kind: ImplementationKind::Executable,
                    source: SourceReference {
                        kind: SourceKind::Local,
                        location: "examples".to_string(),
                    },
                    binary: Some(binary.unwrap_or(BinaryReference {
                        format: BinaryFormat::Wasm,
                        location: "registered-test-module.wasm".to_string(),
                        signature: None,
                    })),
                    workflow_ref: None,
                    digests: ArtifactDigests {
                        source_digest: "source-digest".to_string(),
                        binary_digest: Some("sha256:checksum".to_string()),
                    },
                    provenance: RegistryProvenance {
                        source: "test".to_string(),
                        author: "test".to_string(),
                        created_at: "2026-07-13T00:00:00Z".to_string(),
                    },
                },
                registered_at: "2026-07-13T00:00:00Z".to_string(),
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
            .expect("test capability should register");
        registry
            .find_exact(LookupScope::PublicOnly, "hello.world.say-hello", "1.0.0")
            .expect("registered capability should resolve")
    }

    #[test]
    fn native_execution_requires_an_explicit_handler() {
        let mut capability = resolved_capability(None);
        capability.artifact.binary = None;
        let mut router = ArtifactRouter::new().expect("router should initialize");
        let failure = router
            .execute(&capability, &serde_json::json!({}))
            .expect_err("unregistered native handler should fail closed");
        assert_eq!(failure.code, LocalExecutionFailureCode::ConstraintViolated);

        router.register_native_handler("hello.world.say-hello", |_| {
            Ok(LocalExecutionOutput {
                value: serde_json::json!({"ok": true}),
                emitted_events: Vec::new(),
            })
        });
        assert_eq!(
            router.execute(&capability, &serde_json::json!({})),
            Ok(LocalExecutionOutput {
                value: serde_json::json!({"ok": true}),
                emitted_events: Vec::new(),
            })
        );
    }

    #[cfg(feature = "wasmtime-executor")]
    #[test]
    fn wasm_artifacts_are_executed_only_from_registered_binary_metadata() {
        let capability = resolved_capability(Some(BinaryReference {
            format: BinaryFormat::Wasm,
            location: "missing-test-module.wasm".to_string(),
            signature: None,
        }));
        let failure = ArtifactRouter::new()
            .expect("router should initialize")
            .execute(&capability, &serde_json::json!({}))
            .expect_err("missing registered binary should fail");
        assert_eq!(failure.code, LocalExecutionFailureCode::ConstraintViolated);
        assert!(
            failure.message.contains("binary load failed")
                && failure.message.contains("missing-test-module.wasm"),
            "missing binary should surface the concrete load failure, got {}",
            failure.message
        );
    }

    #[cfg(feature = "wasmtime-executor")]
    #[test]
    fn wasm_execution_success_returns_real_value_and_emitted_events() {
        use sha2::{Digest, Sha256};
        use std::fmt::Write as _;

        // Spec 101-local-executor-event-emission FR-003: on a successful
        // WASM execution, `ArtifactRouter` must return the executor's real
        // `value`/`emitted_events`, not discard or reshape them.
        let wat_src = r#"
            (module
                (import "wasi_snapshot_preview1" "fd_write"
                    (func $fd_write (param i32 i32 i32 i32) (result i32)))
                (memory (export "memory") 1)
                (data (i32.const 8) "{}")
                (func $_start (export "_start")
                    (i32.store (i32.const 0) (i32.const 8))
                    (i32.store (i32.const 4) (i32.const 2))
                    (drop (call $fd_write (i32.const 1) (i32.const 0) (i32.const 1) (i32.const 4)))
                )
            )
        "#;
        let wasm_bytes = wat::parse_str(wat_src).expect("WAT source should parse");

        let mut hasher = Sha256::new();
        hasher.update(&wasm_bytes);
        let checksum = hasher
            .finalize()
            .iter()
            .fold(String::new(), |mut acc, byte| {
                let _ = write!(acc, "{byte:02x}");
                acc
            });

        let tmp = format!(
            "/tmp/traverse-artifact-router-test-{}.wasm",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        );
        std::fs::write(&tmp, &wasm_bytes).expect("temp wasm module should write");

        let mut capability = resolved_capability(Some(BinaryReference {
            format: BinaryFormat::Wasm,
            location: tmp.clone(),
            signature: None,
        }));
        capability.artifact.digests.binary_digest = Some(format!("sha256:{checksum}"));

        let result = ArtifactRouter::new()
            .expect("router should initialize")
            .execute(&capability, &serde_json::json!({}));
        std::fs::remove_file(&tmp).ok();

        assert_eq!(
            result,
            Ok(LocalExecutionOutput {
                value: serde_json::json!({}),
                emitted_events: Vec::new(),
            })
        );
    }

    #[test]
    fn executor_errors_map_to_stable_local_failure_codes() {
        let errors = [
            (
                ExecutorError::Timeout("x".to_string()),
                LocalExecutionFailureCode::Timeout,
            ),
            (
                ExecutorError::ResourceExhausted("x".to_string()),
                LocalExecutionFailureCode::ResourceExhausted,
            ),
            (
                ExecutorError::ChecksumMismatch {
                    expected: "a".to_string(),
                    actual: "b".to_string(),
                },
                LocalExecutionFailureCode::ConstraintViolated,
            ),
            (
                ExecutorError::MalformedWasmArtifact {
                    error_code: "x".to_string(),
                    detail: "x".to_string(),
                },
                LocalExecutionFailureCode::ConstraintViolated,
            ),
            (
                ExecutorError::UnsupportedAbiVersion {
                    error_code: "x".to_string(),
                    requested: "x".to_string(),
                    supported: "x".to_string(),
                },
                LocalExecutionFailureCode::ConstraintViolated,
            ),
            (
                ExecutorError::UnauthorizedHostImport {
                    error_code: "x".to_string(),
                    abi_version: "x".to_string(),
                    module: "x".to_string(),
                    name: "x".to_string(),
                },
                LocalExecutionFailureCode::ConstraintViolated,
            ),
            (
                ExecutorError::BinaryLoadFailed("x".to_string()),
                LocalExecutionFailureCode::ConstraintViolated,
            ),
            (
                ExecutorError::RuntimeSetupFailed("x".to_string()),
                LocalExecutionFailureCode::ConstraintViolated,
            ),
            (
                ExecutorError::OutputDeserializationFailed("x".to_string()),
                LocalExecutionFailureCode::InvalidInput,
            ),
            (
                ExecutorError::ExecutionFailed("x".to_string()),
                LocalExecutionFailureCode::ExecutionFailed,
            ),
            (
                ExecutorError::UnsupportedArtifactType,
                LocalExecutionFailureCode::ExecutionFailed,
            ),
        ];
        for (error, expected) in errors {
            let failure = map_executor_error(&error);
            assert_eq!(failure.code, expected);
            assert_eq!(failure.message, error.to_string());
            assert!(
                !failure.message.is_empty(),
                "executor failure message must not be collapsed to an empty string"
            );
        }
    }
}
