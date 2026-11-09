//! The compatible-capability lifecycle rules (spec
//! `072-native-bridge-compatible-lifecycle` FR-002 to FR-008, Decisions 127
//! and 129).
//!
//! A compatible capability runs in a host platform module, and the runtime
//! owns its lifecycle: the platform allowlist, one running instance at a
//! time, instance identifiers that are never reused, and the Spec 1722
//! canonical input the platform wrapper receives. `runtime.wasm` and the
//! Rust `traverse-embedder` both apply these rules from this module, so
//! every host follows one copy of them. Callers own their transport: request
//! parsing, events and status codes.

use crate::canonical_json::{CanonicalJsonError, to_canonical_json};
use serde_json::Value;
use std::fmt;

/// Why a compatible lifecycle operation did not happen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompatibleLifecycleError {
    /// The capability's `platforms[]` does not include the host's platform.
    PlatformNotSupported,
    /// An instance is already running (Spec 072 FR-003).
    InstanceActive,
    /// No running instance matches the request.
    InstanceNotActive,
    /// The start input cannot be canonicalized (Spec 1722 FR-003).
    InputNotCanonicalizable(CanonicalJsonError),
}

impl CompatibleLifecycleError {
    /// The stable error code.
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::PlatformNotSupported => "platform_not_supported",
            Self::InstanceActive => "compatible_instance_active",
            Self::InstanceNotActive => "compatible_instance_not_active",
            Self::InputNotCanonicalizable(error) => error.code(),
        }
    }
}

impl fmt::Display for CompatibleLifecycleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InputNotCanonicalizable(error) => error.fmt(f),
            _ => f.write_str(self.code()),
        }
    }
}

impl std::error::Error for CompatibleLifecycleError {}

/// How a running instance ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompatibleFinish {
    /// Graceful stop (Spec 072 FR-006).
    Stopped,
    /// Forced termination (Spec 072 FR-007).
    Killed,
}

impl CompatibleFinish {
    /// The lifecycle state the instance ends in.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stopped => "stopped",
            Self::Killed => "killed",
        }
    }
}

/// A started instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompatibleStart {
    /// `{capability_id}-compatible-{n}`, never reused.
    pub instance_id: String,
    /// The Spec 1722 bytes the host hands to the platform wrapper unchanged.
    pub canonical_input: String,
}

/// One compatible capability's platform allowlist and lifecycle state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompatibleLifecycle {
    capability_id: String,
    platforms: Vec<String>,
    host_platform: String,
    /// Instances started so far; numbers the next instance identifier.
    started: u64,
    /// The running instance. At most one runs at a time.
    active: Option<String>,
}

impl CompatibleLifecycle {
    /// A lifecycle with no instance started yet.
    #[must_use]
    pub fn new(capability_id: String, platforms: Vec<String>, host_platform: String) -> Self {
        Self {
            capability_id,
            platforms,
            host_platform,
            started: 0,
            active: None,
        }
    }

    /// The capability's `platforms[]` allowlist.
    #[must_use]
    pub fn platforms(&self) -> &[String] {
        &self.platforms
    }

    /// The platform the host runs on.
    #[must_use]
    pub fn host_platform(&self) -> &str {
        &self.host_platform
    }

    /// The running instance, if any.
    #[must_use]
    pub fn active(&self) -> Option<&str> {
        self.active.as_deref()
    }

    /// Starts an instance with `input` (Spec 072 FR-002).
    ///
    /// # Errors
    ///
    /// Input JCS cannot represent, a host platform outside the allowlist,
    /// or an instance already running.
    pub fn start(&mut self, input: &Value) -> Result<CompatibleStart, CompatibleLifecycleError> {
        let canonical_input =
            to_canonical_json(input).map_err(CompatibleLifecycleError::InputNotCanonicalizable)?;
        if !self.platforms.contains(&self.host_platform) {
            return Err(CompatibleLifecycleError::PlatformNotSupported);
        }
        if self.active.is_some() {
            return Err(CompatibleLifecycleError::InstanceActive);
        }
        self.started += 1;
        let instance_id = format!("{}-compatible-{}", self.capability_id, self.started);
        self.active = Some(instance_id.clone());
        Ok(CompatibleStart {
            instance_id,
            canonical_input,
        })
    }

    /// Ends the running instance and returns its identifier (Spec 072
    /// FR-006, FR-007). `None` selects the running instance.
    ///
    /// # Errors
    ///
    /// [`CompatibleLifecycleError::InstanceNotActive`] when nothing runs or
    /// `instance_id` names a different instance.
    pub fn finish(
        &mut self,
        instance_id: Option<&str>,
    ) -> Result<String, CompatibleLifecycleError> {
        match (self.active.as_deref(), instance_id) {
            (Some(active), Some(requested)) if active != requested => {
                Err(CompatibleLifecycleError::InstanceNotActive)
            }
            (Some(_), _) => self
                .active
                .take()
                .ok_or(CompatibleLifecycleError::InstanceNotActive),
            (None, _) => Err(CompatibleLifecycleError::InstanceNotActive),
        }
    }

    /// Kills the running instance on shutdown and returns its identifier
    /// (Spec 072 FR-008).
    pub fn shutdown(&mut self) -> Option<String> {
        self.active.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn lifecycle(host_platform: &str) -> CompatibleLifecycle {
        CompatibleLifecycle::new(
            "demo.camera".to_string(),
            vec!["ios".to_string(), "android".to_string()],
            host_platform.to_string(),
        )
    }

    #[test]
    fn start_returns_canonical_input_and_a_fresh_instance() {
        let mut lifecycle = lifecycle("ios");
        assert_eq!(lifecycle.platforms(), ["ios", "android"]);
        assert_eq!(lifecycle.host_platform(), "ios");
        let first = lifecycle.start(&json!({"zoom": 2, "lens": "wide"}));
        assert_eq!(
            first,
            Ok(CompatibleStart {
                instance_id: "demo.camera-compatible-1".to_string(),
                canonical_input: r#"{"lens":"wide","zoom":2}"#.to_string(),
            })
        );
        assert_eq!(lifecycle.active(), Some("demo.camera-compatible-1"));
        assert_eq!(
            lifecycle.start(&json!({})),
            Err(CompatibleLifecycleError::InstanceActive)
        );
        assert_eq!(
            lifecycle.finish(None),
            Ok("demo.camera-compatible-1".to_string())
        );
        let second = lifecycle.start(&json!({})).map(|start| start.instance_id);
        assert_eq!(second, Ok("demo.camera-compatible-2".to_string()));
    }

    #[test]
    fn start_rejects_unsupported_platforms_and_unsafe_input() {
        assert_eq!(
            lifecycle("web").start(&json!({})),
            Err(CompatibleLifecycleError::PlatformNotSupported)
        );
        let unsafe_input = lifecycle("ios").start(&json!({"n": 9_007_199_254_740_992_u64}));
        assert_eq!(
            unsafe_input,
            Err(CompatibleLifecycleError::InputNotCanonicalizable(
                CanonicalJsonError::UnsafeInteger
            ))
        );
    }

    #[test]
    fn finish_requires_the_running_instance() {
        let mut lifecycle = lifecycle("ios");
        assert_eq!(
            lifecycle.finish(None),
            Err(CompatibleLifecycleError::InstanceNotActive)
        );
        let started = lifecycle.start(&json!({})).map(|start| start.instance_id);
        assert_eq!(
            lifecycle.finish(Some("demo.camera-compatible-9")),
            Err(CompatibleLifecycleError::InstanceNotActive)
        );
        assert_eq!(lifecycle.active(), Some("demo.camera-compatible-1"));
        assert_eq!(lifecycle.finish(Some("demo.camera-compatible-1")), started);
        assert_eq!(
            lifecycle.finish(Some("demo.camera-compatible-1")),
            Err(CompatibleLifecycleError::InstanceNotActive)
        );
    }

    #[test]
    fn shutdown_takes_the_running_instance() {
        let mut lifecycle = lifecycle("ios");
        assert_eq!(lifecycle.shutdown(), None);
        let _ = lifecycle.start(&json!({}));
        assert_eq!(
            lifecycle.shutdown(),
            Some("demo.camera-compatible-1".to_string())
        );
        assert_eq!(lifecycle.active(), None);
    }

    #[test]
    fn errors_carry_stable_codes() {
        let cases = [
            (
                CompatibleLifecycleError::PlatformNotSupported,
                "platform_not_supported",
                "platform_not_supported",
            ),
            (
                CompatibleLifecycleError::InstanceActive,
                "compatible_instance_active",
                "compatible_instance_active",
            ),
            (
                CompatibleLifecycleError::InstanceNotActive,
                "compatible_instance_not_active",
                "compatible_instance_not_active",
            ),
            (
                CompatibleLifecycleError::InputNotCanonicalizable(CanonicalJsonError::DuplicateKey),
                "input_not_canonicalizable",
                "input_not_canonicalizable: input contains an object with a duplicate member name",
            ),
        ];
        for (error, code, display) in cases {
            assert_eq!(error.code(), code);
            assert_eq!(error.to_string(), display);
        }
        assert_eq!(CompatibleFinish::Stopped.as_str(), "stopped");
        assert_eq!(CompatibleFinish::Killed.as_str(), "killed");
    }
}
