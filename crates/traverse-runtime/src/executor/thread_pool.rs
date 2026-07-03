//! Bounded native capability executor.
//!
//! Governed by spec `047-thread-pool-executor`.

use std::panic::{AssertUnwindSafe, catch_unwind};

use rayon::{ThreadPool, ThreadPoolBuildError, ThreadPoolBuilder};
use serde_json::Value;

use super::{ArtifactType, CapabilityExecutor, ExecutorCapability, ExecutorError};

const MIN_CAPACITY: usize = 1;
const MAX_CAPACITY: usize = 256;

/// Configuration for [`ThreadPoolExecutor`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThreadPoolExecutorConfig {
    /// Number of worker threads available to native capability execution.
    pub capacity: usize,
}

/// Construction-time configuration error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    /// The configured capacity is outside the supported inclusive range.
    InvalidCapacity {
        /// Requested capacity.
        given: usize,
        /// Minimum supported capacity.
        min: usize,
        /// Maximum supported capacity.
        max: usize,
    },
    /// Rayon could not build the worker pool.
    PoolBuildFailed(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidCapacity { given, min, max } => {
                write!(
                    f,
                    "invalid thread pool capacity {given}; expected {min}..={max}"
                )
            }
            Self::PoolBuildFailed(msg) => write!(f, "thread pool build failed: {msg}"),
        }
    }
}

impl std::error::Error for ConfigError {}

impl From<ThreadPoolBuildError> for ConfigError {
    fn from(value: ThreadPoolBuildError) -> Self {
        Self::PoolBuildFailed(value.to_string())
    }
}

/// Dispatches native capability execution onto a bounded worker pool.
pub struct ThreadPoolExecutor {
    pool: ThreadPool,
    inner: Box<dyn CapabilityExecutor>,
}

impl std::fmt::Debug for ThreadPoolExecutor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ThreadPoolExecutor").finish_non_exhaustive()
    }
}

impl ThreadPoolExecutor {
    /// Create a new bounded thread-pool executor.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] when capacity is outside `1..=256` or the
    /// underlying pool cannot be built.
    pub fn new(
        config: ThreadPoolExecutorConfig,
        inner: Box<dyn CapabilityExecutor>,
    ) -> Result<Self, ConfigError> {
        if !(MIN_CAPACITY..=MAX_CAPACITY).contains(&config.capacity) {
            return Err(ConfigError::InvalidCapacity {
                given: config.capacity,
                min: MIN_CAPACITY,
                max: MAX_CAPACITY,
            });
        }

        let pool = ThreadPoolBuilder::new()
            .num_threads(config.capacity)
            .build()?;

        Ok(Self { pool, inner })
    }
}

impl CapabilityExecutor for ThreadPoolExecutor {
    fn execute(
        &self,
        capability: &ExecutorCapability,
        input: &Value,
    ) -> Result<Value, ExecutorError> {
        if capability.artifact_type == ArtifactType::Wasm {
            return Err(ExecutorError::UnsupportedArtifactType);
        }

        let capability = capability.clone();
        let input = input.clone();
        let result = self
            .pool
            .install(|| catch_unwind(AssertUnwindSafe(|| self.inner.execute(&capability, &input))));

        match result {
            Ok(inner_result) => inner_result,
            Err(_) => Err(ExecutorError::ExecutionFailed(
                "capability panicked".to_string(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io;

    use serde_json::json;

    use super::*;
    use crate::executor::NativeExecutor;

    fn native_capability() -> ExecutorCapability {
        ExecutorCapability {
            capability_id: "test.native".to_string(),
            artifact_type: ArtifactType::Native,
            wasm_binary_path: None,
            wasm_checksum: None,
            host_abi_version: None,
        }
    }

    fn wasm_capability() -> ExecutorCapability {
        ExecutorCapability {
            artifact_type: ArtifactType::Wasm,
            ..native_capability()
        }
    }

    fn new_executor(
        capacity: usize,
        inner: Box<dyn CapabilityExecutor>,
    ) -> Result<ThreadPoolExecutor, ConfigError> {
        ThreadPoolExecutor::new(ThreadPoolExecutorConfig { capacity }, inner)
    }

    #[allow(clippy::unnecessary_wraps)]
    fn clone_input(input: &Value) -> Result<Value, String> {
        if input.get("__thread_pool_test_error").is_some() {
            return Err("requested test error".to_string());
        }
        Ok(input.clone())
    }

    #[test]
    fn clone_input_helper_can_return_error() {
        let result = clone_input(&json!({ "__thread_pool_test_error": true }));

        assert_eq!(result, Err("requested test error".to_string()));
    }

    #[test]
    fn config_rejects_capacity_below_minimum() {
        let result = new_executor(0, Box::new(NativeExecutor::new(clone_input)));

        assert_eq!(
            result.err(),
            Some(ConfigError::InvalidCapacity {
                given: 0,
                min: 1,
                max: 256
            })
        );
    }

    #[test]
    fn config_accepts_capacity_bounds() {
        let min_result = new_executor(1, Box::new(NativeExecutor::new(clone_input)));
        let max_result = new_executor(256, Box::new(NativeExecutor::new(clone_input)));

        assert!(min_result.is_ok());
        assert!(max_result.is_ok());
    }

    #[test]
    fn config_rejects_capacity_above_maximum() {
        let result = new_executor(257, Box::new(NativeExecutor::new(clone_input)));

        assert_eq!(
            result.err(),
            Some(ConfigError::InvalidCapacity {
                given: 257,
                min: 1,
                max: 256
            })
        );
    }

    #[test]
    fn config_error_display_and_from_cover_failure_shapes() {
        let invalid = ConfigError::InvalidCapacity {
            given: 0,
            min: 1,
            max: 256,
        };

        assert_eq!(
            invalid.to_string(),
            "invalid thread pool capacity 0; expected 1..=256"
        );

        let build_error = ThreadPoolBuilder::new()
            .num_threads(1)
            .spawn_handler(|_| Err(io::Error::other("spawn denied")))
            .build()
            .map_err(ConfigError::from);

        assert!(matches!(build_error, Err(ConfigError::PoolBuildFailed(_))));

        let display = build_error.err().map(|err| err.to_string());
        assert_eq!(
            display,
            Some("thread pool build failed: spawn denied".to_string())
        );
    }

    #[test]
    fn thread_pool_executor_debug_is_non_exhaustive() {
        let debug = new_executor(1, Box::new(NativeExecutor::new(clone_input)))
            .map(|executor| format!("{executor:?}"));

        assert_eq!(debug, Ok("ThreadPoolExecutor { .. }".to_string()));
    }

    #[test]
    fn execute_native_returns_inner_output() {
        let result = new_executor(2, Box::new(NativeExecutor::new(clone_input)))
            .map(|executor| executor.execute(&native_capability(), &json!({ "value": 42 })));

        assert_eq!(result, Ok(Ok(json!({ "value": 42 }))));
    }

    #[test]
    fn execute_native_propagates_inner_error() {
        let result = new_executor(
            2,
            Box::new(NativeExecutor::new(|_| Err("inner failed".to_string()))),
        )
        .map(|executor| executor.execute(&native_capability(), &json!({})));

        assert_eq!(
            result,
            Ok(Err(ExecutorError::ExecutionFailed(
                "inner failed".to_string()
            )))
        );
    }

    #[test]
    fn execute_wasm_returns_unsupported() {
        let result = new_executor(2, Box::new(NativeExecutor::new(clone_input)))
            .map(|executor| executor.execute(&wasm_capability(), &json!({})));

        assert_eq!(result, Ok(Err(ExecutorError::UnsupportedArtifactType)));
    }

    #[test]
    fn panicking_inner_returns_execution_failed_and_pool_remains_usable() {
        struct PanickingExecutor;

        impl CapabilityExecutor for PanickingExecutor {
            fn execute(
                &self,
                _capability: &ExecutorCapability,
                _input: &Value,
            ) -> Result<Value, ExecutorError> {
                std::panic::resume_unwind(Box::new("boom"));
            }
        }

        let failed = new_executor(1, Box::new(PanickingExecutor))
            .map(|executor| executor.execute(&native_capability(), &json!({})));

        assert_eq!(
            failed,
            Ok(Err(ExecutorError::ExecutionFailed(
                "capability panicked".to_string()
            )))
        );

        let result = new_executor(1, Box::new(NativeExecutor::new(|_| Ok(json!("ok")))))
            .map(|executor| executor.execute(&native_capability(), &json!({})));

        assert_eq!(result, Ok(Ok(json!("ok"))));
    }
}
