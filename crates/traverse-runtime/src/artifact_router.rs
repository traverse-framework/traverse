use crate::executor::ExecutorError;
#[cfg(feature = "wasmtime-executor")]
use crate::executor::{ArtifactType, CapabilityExecutor, ExecutorCapability, WasmExecutor};
use crate::{LocalExecutionFailure, LocalExecutionFailureCode, LocalExecutor};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;
use traverse_registry::{BinaryFormat, ResolvedCapability};

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
                if binary.format != BinaryFormat::Wasm {
                    return Err(constraint_failure(
                        "registered artifact format is not supported",
                    ));
                }
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
