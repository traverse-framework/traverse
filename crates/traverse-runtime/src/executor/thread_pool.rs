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
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use std::thread;
    use std::time::Duration;

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

    fn clone_input(input: &Value) -> Result<Value, String> {
        if input.get("__thread_pool_test_error").is_some() {
            return Err("requested test error".to_string());
        }
        Ok(input.clone())
    }

    fn result_debug<T, E: std::fmt::Debug>(result: Result<T, E>) -> Result<T, String> {
        result.map_err(|err| format!("{err:?}"))
    }

    fn executor(capacity: usize) -> Result<ThreadPoolExecutor, String> {
        result_debug(new_executor(
            capacity,
            Box::new(NativeExecutor::new(clone_input)),
        ))
    }

    fn execute_json(executor: &ThreadPoolExecutor, input: &Value) -> Result<Value, ExecutorError> {
        executor.execute(&native_capability(), input)
    }

    #[test]
    fn clone_input_helper_can_return_error() {
        let result = clone_input(&json!({ "__thread_pool_test_error": true }));

        assert_eq!(result, Err("requested test error".to_string()));
    }

    #[test]
    fn config_capacity_zero_returns_error() {
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
    fn config_capacity_max_valid() {
        let result = new_executor(256, Box::new(NativeExecutor::new(clone_input)));

        assert!(result.is_ok());
    }

    #[test]
    fn config_capacity_over_max_returns_error() {
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
    fn config_capacity_one_valid() {
        let result = new_executor(1, Box::new(NativeExecutor::new(clone_input)));

        assert!(result.is_ok());
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

        let display = build_error.map_err(|err| err.to_string()).err();
        assert_eq!(
            display,
            Some("thread pool build failed: spawn denied".to_string())
        );
    }

    #[test]
    fn thread_pool_executor_debug_is_non_exhaustive() -> Result<(), String> {
        let debug = executor(1).map(|executor| format!("{executor:?}"))?;

        assert_eq!(debug, "ThreadPoolExecutor { .. }");
        Ok(())
    }

    #[test]
    fn execute_native_returns_correct_output() -> Result<(), String> {
        let executor = executor(2)?;
        let result = execute_json(&executor, &json!({ "value": 42 }));

        assert_eq!(result, Ok(json!({ "value": 42 })));
        Ok(())
    }

    #[test]
    fn execute_native_error_propagates() {
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
    fn execute_wasm_artifact_type_returns_unsupported() {
        let result = new_executor(2, Box::new(PanickingExecutor::new(1)))
            .map(|executor| executor.execute(&wasm_capability(), &json!({})));

        assert_eq!(result, Ok(Err(ExecutorError::UnsupportedArtifactType)));
    }

    #[test]
    fn concurrent_calls_run_in_parallel() {
        let active_calls = Arc::new(AtomicUsize::new(0));
        let max_active_calls = Arc::new(AtomicUsize::new(0));
        let active_for_handler = Arc::clone(&active_calls);
        let max_for_handler = Arc::clone(&max_active_calls);

        let result = new_executor(
            2,
            Box::new(NativeExecutor::new(move |input| {
                let current = active_for_handler.fetch_add(1, Ordering::SeqCst) + 1;
                max_for_handler.fetch_max(current, Ordering::SeqCst);
                thread::sleep(Duration::from_millis(100));
                active_for_handler.fetch_sub(1, Ordering::SeqCst);
                Ok(input.clone())
            })),
        )
        .map(|executor| {
            let executor = Arc::new(executor);
            let first = {
                let executor = Arc::clone(&executor);
                thread::spawn(move || execute_json(&executor, &json!({ "call": 1 })))
            };
            let second = {
                let executor = Arc::clone(&executor);
                thread::spawn(move || execute_json(&executor, &json!({ "call": 2 })))
            };
            (result_debug(first.join()), result_debug(second.join()))
        });

        let first_result = result
            .as_ref()
            .map(|(first, _)| first.as_ref().map_err(String::as_str));
        let second_result = result
            .as_ref()
            .map(|(_, second)| second.as_ref().map_err(String::as_str));

        assert_eq!(first_result, Ok(Ok(&Ok(json!({ "call": 1 })))));
        assert_eq!(second_result, Ok(Ok(&Ok(json!({ "call": 2 })))));
        assert!(
            max_active_calls.load(Ordering::SeqCst) >= 2,
            "expected two active calls to overlap"
        );
    }

    #[test]
    fn pool_size_one_serialises_calls() {
        let result = new_executor(
            1,
            Box::new(NativeExecutor::new(|input| {
                thread::sleep(Duration::from_millis(50));
                Ok(input.clone())
            })),
        )
        .map(|executor| {
            let executor = Arc::new(executor);
            let first = {
                let executor = Arc::clone(&executor);
                thread::spawn(move || execute_json(&executor, &json!({ "call": 1 })))
            };
            let second = {
                let executor = Arc::clone(&executor);
                thread::spawn(move || execute_json(&executor, &json!({ "call": 2 })))
            };
            (result_debug(first.join()), result_debug(second.join()))
        });
        let first_result = result
            .as_ref()
            .map(|(first, _)| first.as_ref().map_err(String::as_str));
        let second_result = result
            .as_ref()
            .map(|(_, second)| second.as_ref().map_err(String::as_str));

        assert_eq!(first_result, Ok(Ok(&Ok(json!({ "call": 1 })))));
        assert_eq!(second_result, Ok(Ok(&Ok(json!({ "call": 2 })))));
    }

    #[test]
    fn independent_calls_do_not_share_state() -> Result<(), String> {
        let executor = Arc::new(executor(4)?);
        let mut handles = Vec::new();

        for value in 0..10 {
            let executor = Arc::clone(&executor);
            handles.push(thread::spawn(move || {
                execute_json(&executor, &json!({ "value": value }))
            }));
        }

        for (value, handle) in handles.into_iter().enumerate() {
            let result = result_debug(handle.join())?;
            assert_eq!(result, Ok(json!({ "value": value })));
        }
        Ok(())
    }

    struct PanickingExecutor {
        remaining_panics: AtomicUsize,
    }

    impl PanickingExecutor {
        fn new(remaining_panics: usize) -> Self {
            Self {
                remaining_panics: AtomicUsize::new(remaining_panics),
            }
        }
    }

    impl CapabilityExecutor for PanickingExecutor {
        fn execute(
            &self,
            _capability: &ExecutorCapability,
            input: &Value,
        ) -> Result<Value, ExecutorError> {
            let remaining = self.remaining_panics.load(Ordering::SeqCst);
            if remaining > 0 {
                self.remaining_panics.fetch_sub(1, Ordering::SeqCst);
                std::panic::resume_unwind(Box::new("boom"));
            }
            Ok(input.clone())
        }
    }

    #[test]
    fn panicking_handler_returns_execution_failed() -> Result<(), String> {
        let executor = result_debug(new_executor(1, Box::new(PanickingExecutor::new(1))))?;
        let result = executor.execute(&native_capability(), &json!({}));

        assert_eq!(
            result,
            Err(ExecutorError::ExecutionFailed(
                "capability panicked".to_string()
            ))
        );
        Ok(())
    }

    #[test]
    fn pool_usable_after_panic() -> Result<(), String> {
        let executor = result_debug(new_executor(1, Box::new(PanickingExecutor::new(1))))?;
        let failed = executor.execute(&native_capability(), &json!({ "first": true }));
        let recovered = executor.execute(&native_capability(), &json!({ "second": true }));

        assert_eq!(
            failed,
            Err(ExecutorError::ExecutionFailed(
                "capability panicked".to_string()
            ))
        );
        assert_eq!(recovered, Ok(json!({ "second": true })));
        Ok(())
    }

    #[test]
    fn multiple_sequential_panics_do_not_exhaust_pool() -> Result<(), String> {
        let executor = result_debug(new_executor(2, Box::new(PanickingExecutor::new(5))))?;

        for _ in 0..5 {
            let result = executor.execute(&native_capability(), &json!({}));
            assert_eq!(
                result,
                Err(ExecutorError::ExecutionFailed(
                    "capability panicked".to_string()
                ))
            );
        }

        let recovered = executor.execute(&native_capability(), &json!({ "ok": true }));

        assert_eq!(recovered, Ok(json!({ "ok": true })));
        Ok(())
    }

    #[test]
    fn executor_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}

        assert_send_sync::<ThreadPoolExecutor>();
    }

    #[test]
    fn arc_wrapped_executor_callable_from_multiple_threads() -> Result<(), String> {
        let executor = Arc::new(executor(4)?);
        let mut handles = Vec::new();

        for value in 0..4 {
            let executor = Arc::clone(&executor);
            handles.push(thread::spawn(move || {
                execute_json(&executor, &json!({ "thread": value }))
            }));
        }

        for (value, handle) in handles.into_iter().enumerate() {
            let result = result_debug(handle.join())?;
            assert_eq!(result, Ok(json!({ "thread": value })));
        }
        Ok(())
    }

    #[test]
    fn executor_drops_cleanly_after_use() -> Result<(), String> {
        let executor = executor(1)?;
        let result = execute_json(&executor, &json!({ "used": true }));

        assert_eq!(result, Ok(json!({ "used": true })));
        drop(executor);
        Ok(())
    }

    #[test]
    fn executor_drops_cleanly_with_no_calls() -> Result<(), String> {
        let executor = executor(1)?;

        drop(executor);
        Ok(())
    }
}
