use crate::executor::ExecutorError;
#[cfg(feature = "wasmtime-executor")]
use crate::executor::{ArtifactType, CapabilityExecutor, ExecutorCapability, WasmExecutor};
use crate::{LocalExecutionFailure, LocalExecutionFailureCode, LocalExecutor};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;
use traverse_registry::ResolvedCapability;

type NativeHandler = dyn Fn(&Value) -> Result<Value, LocalExecutionFailure> + Send + Sync;

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
        F: Fn(&Value) -> Result<Value, LocalExecutionFailure> + Send + Sync + 'static,
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
    ) -> Result<Value, LocalExecutionFailure> {
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
                };
                return self
                    .wasm
                    .execute(&executor_capability, input)
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
    LocalExecutionFailure {
        code,
        message: "registered artifact execution failed".to_string(),
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
            Ok(serde_json::json!({"ok": true}))
        });
        assert_eq!(
            router.execute(&capability, &serde_json::json!({})),
            Ok(serde_json::json!({"ok": true}))
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
        assert_eq!(failure.message, "registered artifact execution failed");
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
            assert_eq!(failure.message, "registered artifact execution failed");
        }
    }
}
