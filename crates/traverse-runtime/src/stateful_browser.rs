//! Stateful Browser activation attestation (Spec `132-stateful-browser-placement`).
//!
//! Contract validation may permit `Stateful` + `Browser`. Before guest
//! execution on Browser, the host must prove a Spec `085` `IndexedDB` DataStore
//! is bound and open under its exclusive-lock and public-integrity guarantees.
//! Failures use the stable code `stateful_browser_store_unavailable`.
//!
//! Spec `132` activation attestation.

use serde_json::{Value, json};
use traverse_contracts::{ExecutionTarget, ServiceType};

/// Stable activation failure code (Spec 132 FR-003).
pub const STATEFUL_BROWSER_STORE_UNAVAILABLE: &str = "stateful_browser_store_unavailable";

/// Runtime-verifiable facts about a bound open store (Spec 132 FR-004).
///
/// Callers MUST derive these from the live store handle. An embedder honor
/// flag alone is not attestation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexedDbOpenAttestation {
    /// Exclusive Web Lock / ownership acquired for this open handle.
    pub exclusive_lock_held: bool,
    /// Public integrity envelope path is available on this open handle.
    pub public_integrity_available: bool,
}

/// Bound store presented for Browser Stateful activation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatefulBrowserBoundStore {
    /// No `DataStore` is bound to the activation.
    Missing,
    /// A non-`IndexedDB` `DataStore` backend is bound.
    NonIndexedDb,
    /// `IndexedDB` backend with open-guarantee facts.
    IndexedDb(IndexedDbOpenAttestation),
}

/// Secret-free evidence for a successful Spec 132 activation check (FR-005).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatefulBrowserActivationEvidence {
    pub governing_spec: &'static str,
    pub backend: &'static str,
    pub exclusive_lock_held: bool,
    pub public_integrity_available: bool,
    pub outcome: &'static str,
}

/// Fail-closed activation error (Spec 132 FR-003/FR-005).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatefulBrowserActivationError {
    pub code: &'static str,
    pub governing_spec: &'static str,
    pub outcome: &'static str,
    pub reason: &'static str,
}

impl StatefulBrowserActivationEvidence {
    /// Secret-free JSON projection (Spec 132 FR-005).
    #[must_use]
    pub fn to_secret_free_json(&self) -> Value {
        json!({
            "governing_spec": self.governing_spec,
            "backend": self.backend,
            "exclusive_lock_held": self.exclusive_lock_held,
            "public_integrity_available": self.public_integrity_available,
            "outcome": self.outcome,
        })
    }
}

impl StatefulBrowserActivationError {
    /// Secret-free JSON projection (Spec 132 FR-005).
    #[must_use]
    pub fn to_secret_free_json(&self) -> Value {
        json!({
            "code": self.code,
            "governing_spec": self.governing_spec,
            "outcome": self.outcome,
            "reason": self.reason,
        })
    }
}

/// Attests Browser activation of a Stateful capability against a bound store.
///
/// Non-Browser targets and non-Stateful service types are no-ops (Spec 132
/// FR-006 / User Story 4).
///
/// # Errors
///
/// Returns [`StatefulBrowserActivationError`] with
/// [`STATEFUL_BROWSER_STORE_UNAVAILABLE`] when Browser Stateful activation
/// lacks a qualifying open `IndexedDB` store.
pub fn attest_stateful_browser_activation(
    service_type: &ServiceType,
    target: &ExecutionTarget,
    bound_store: StatefulBrowserBoundStore,
) -> Result<Option<StatefulBrowserActivationEvidence>, StatefulBrowserActivationError> {
    if *service_type != ServiceType::Stateful || *target != ExecutionTarget::Browser {
        return Ok(None);
    }

    match bound_store {
        StatefulBrowserBoundStore::IndexedDb(facts)
            if facts.exclusive_lock_held && facts.public_integrity_available =>
        {
            Ok(Some(StatefulBrowserActivationEvidence {
                governing_spec: "132-stateful-browser-placement",
                backend: "indexeddb",
                exclusive_lock_held: true,
                public_integrity_available: true,
                outcome: "attested",
            }))
        }
        StatefulBrowserBoundStore::Missing => Err(activation_error("missing_bound_store")),
        StatefulBrowserBoundStore::NonIndexedDb => Err(activation_error("non_indexeddb_backend")),
        StatefulBrowserBoundStore::IndexedDb(facts) if !facts.exclusive_lock_held => {
            Err(activation_error("exclusive_lock_not_held"))
        }
        StatefulBrowserBoundStore::IndexedDb(_) => {
            Err(activation_error("public_integrity_unavailable"))
        }
    }
}

fn activation_error(reason: &'static str) -> StatefulBrowserActivationError {
    StatefulBrowserActivationError {
        code: STATEFUL_BROWSER_STORE_UNAVAILABLE,
        governing_spec: "132-stateful-browser-placement",
        outcome: "denied",
        reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn qualifying() -> StatefulBrowserBoundStore {
        StatefulBrowserBoundStore::IndexedDb(IndexedDbOpenAttestation {
            exclusive_lock_held: true,
            public_integrity_available: true,
        })
    }

    fn denied(reason: &'static str) -> StatefulBrowserActivationError {
        activation_error(reason)
    }

    #[test]
    fn non_browser_stateful_is_unchanged() {
        assert_eq!(
            attest_stateful_browser_activation(
                &ServiceType::Stateful,
                &ExecutionTarget::Local,
                StatefulBrowserBoundStore::Missing,
            ),
            Ok(None)
        );
    }

    #[test]
    fn non_stateful_browser_is_unchanged() {
        assert_eq!(
            attest_stateful_browser_activation(
                &ServiceType::Stateless,
                &ExecutionTarget::Browser,
                StatefulBrowserBoundStore::Missing,
            ),
            Ok(None)
        );
    }

    #[test]
    fn qualifying_indexeddb_attests() {
        let evidence = StatefulBrowserActivationEvidence {
            governing_spec: "132-stateful-browser-placement",
            backend: "indexeddb",
            exclusive_lock_held: true,
            public_integrity_available: true,
            outcome: "attested",
        };
        assert_eq!(
            attest_stateful_browser_activation(
                &ServiceType::Stateful,
                &ExecutionTarget::Browser,
                qualifying(),
            ),
            Ok(Some(evidence.clone()))
        );
        let text = evidence.to_secret_free_json().to_string();
        assert!(!text.contains("database"));
        assert!(!text.contains('/'));
        assert!(!text.contains("payload"));
        assert!(format!("{evidence:?}").contains("attested"));
    }

    #[test]
    fn missing_store_fails_closed() {
        assert_eq!(
            attest_stateful_browser_activation(
                &ServiceType::Stateful,
                &ExecutionTarget::Browser,
                StatefulBrowserBoundStore::Missing,
            ),
            Err(denied("missing_bound_store"))
        );
    }

    #[test]
    fn non_indexeddb_fails_closed() {
        assert_eq!(
            attest_stateful_browser_activation(
                &ServiceType::Stateful,
                &ExecutionTarget::Browser,
                StatefulBrowserBoundStore::NonIndexedDb,
            ),
            Err(denied("non_indexeddb_backend"))
        );
    }

    #[test]
    fn open_without_lock_fails_closed() {
        assert_eq!(
            attest_stateful_browser_activation(
                &ServiceType::Stateful,
                &ExecutionTarget::Browser,
                StatefulBrowserBoundStore::IndexedDb(IndexedDbOpenAttestation {
                    exclusive_lock_held: false,
                    public_integrity_available: true,
                }),
            ),
            Err(denied("exclusive_lock_not_held"))
        );
    }

    #[test]
    fn open_without_public_integrity_fails_closed() {
        assert_eq!(
            attest_stateful_browser_activation(
                &ServiceType::Stateful,
                &ExecutionTarget::Browser,
                StatefulBrowserBoundStore::IndexedDb(IndexedDbOpenAttestation {
                    exclusive_lock_held: true,
                    public_integrity_available: false,
                }),
            ),
            Err(denied("public_integrity_unavailable"))
        );
    }

    #[test]
    fn denial_evidence_is_secret_free() {
        let error = denied("missing_bound_store");
        let text = error.to_secret_free_json().to_string();
        assert!(!text.contains("payload"));
        assert!(!text.contains("databaseName"));
        assert!(!text.contains("/var/"));
        assert!(!text.contains("indexeddb://"));
        assert_eq!(error, error.clone());
        assert!(format!("{error:?}").contains(STATEFUL_BROWSER_STORE_UNAVAILABLE));
        assert_ne!(
            StatefulBrowserBoundStore::Missing,
            StatefulBrowserBoundStore::NonIndexedDb
        );
        let facts = IndexedDbOpenAttestation {
            exclusive_lock_held: true,
            public_integrity_available: true,
        };
        assert_eq!(facts, facts);
        assert_eq!(
            StatefulBrowserBoundStore::IndexedDb(facts),
            StatefulBrowserBoundStore::IndexedDb(facts)
        );
    }
}
