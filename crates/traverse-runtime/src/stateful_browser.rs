//! Stateful Browser activation attestation (Spec `132-stateful-browser-placement`).
//!
//! Contract validation may permit `Stateful` + `Browser`. Before guest
//! execution on Browser, the host must prove a Spec `085` `IndexedDB` DataStore
//! is bound and open under its exclusive-lock and public-integrity guarantees.
//! Failures use the stable code `stateful_browser_store_unavailable`.
//!
//! Spec `132` activation attestation.

use serde::Serialize;
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StatefulBrowserActivationEvidence {
    pub governing_spec: &'static str,
    pub backend: &'static str,
    pub exclusive_lock_held: bool,
    pub public_integrity_available: bool,
    pub outcome: &'static str,
}

/// Fail-closed activation error (Spec 132 FR-003/FR-005).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StatefulBrowserActivationError {
    pub code: &'static str,
    pub governing_spec: &'static str,
    pub outcome: &'static str,
    pub reason: &'static str,
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
    use serde_json::to_value;

    fn qualifying() -> StatefulBrowserBoundStore {
        StatefulBrowserBoundStore::IndexedDb(IndexedDbOpenAttestation {
            exclusive_lock_held: true,
            public_integrity_available: true,
        })
    }

    #[test]
    fn non_browser_stateful_is_unchanged() {
        let result = attest_stateful_browser_activation(
            &ServiceType::Stateful,
            &ExecutionTarget::Local,
            StatefulBrowserBoundStore::Missing,
        );
        assert_eq!(result, Ok(None));
    }

    #[test]
    fn non_stateful_browser_is_unchanged() {
        let result = attest_stateful_browser_activation(
            &ServiceType::Stateless,
            &ExecutionTarget::Browser,
            StatefulBrowserBoundStore::Missing,
        );
        assert_eq!(result, Ok(None));
    }

    #[test]
    fn qualifying_indexeddb_attests() {
        let result = attest_stateful_browser_activation(
            &ServiceType::Stateful,
            &ExecutionTarget::Browser,
            qualifying(),
        );
        assert!(matches!(
            result,
            Ok(Some(StatefulBrowserActivationEvidence {
                backend: "indexeddb",
                outcome: "attested",
                exclusive_lock_held: true,
                public_integrity_available: true,
                ..
            }))
        ));
        if let Ok(Some(evidence)) = result {
            let serialized = to_value(&evidence);
            assert!(serialized.is_ok());
            if let Ok(value) = serialized {
                let text = value.to_string();
                assert!(!text.contains("database"));
                assert!(!text.contains('/'));
                assert!(!text.contains("payload"));
            }
        }
    }

    #[test]
    fn missing_store_fails_closed() {
        let result = attest_stateful_browser_activation(
            &ServiceType::Stateful,
            &ExecutionTarget::Browser,
            StatefulBrowserBoundStore::Missing,
        );
        assert!(result.is_err());
        if let Err(error) = result {
            assert_eq!(error.code, STATEFUL_BROWSER_STORE_UNAVAILABLE);
            assert_eq!(error.reason, "missing_bound_store");
        }
    }

    #[test]
    fn non_indexeddb_fails_closed() {
        let result = attest_stateful_browser_activation(
            &ServiceType::Stateful,
            &ExecutionTarget::Browser,
            StatefulBrowserBoundStore::NonIndexedDb,
        );
        assert!(result.is_err());
        if let Err(error) = result {
            assert_eq!(error.code, STATEFUL_BROWSER_STORE_UNAVAILABLE);
            assert_eq!(error.reason, "non_indexeddb_backend");
        }
    }

    #[test]
    fn open_without_lock_fails_closed() {
        let result = attest_stateful_browser_activation(
            &ServiceType::Stateful,
            &ExecutionTarget::Browser,
            StatefulBrowserBoundStore::IndexedDb(IndexedDbOpenAttestation {
                exclusive_lock_held: false,
                public_integrity_available: true,
            }),
        );
        assert!(result.is_err());
        if let Err(error) = result {
            assert_eq!(error.code, STATEFUL_BROWSER_STORE_UNAVAILABLE);
            assert_eq!(error.reason, "exclusive_lock_not_held");
        }
    }

    #[test]
    fn open_without_public_integrity_fails_closed() {
        let result = attest_stateful_browser_activation(
            &ServiceType::Stateful,
            &ExecutionTarget::Browser,
            StatefulBrowserBoundStore::IndexedDb(IndexedDbOpenAttestation {
                exclusive_lock_held: true,
                public_integrity_available: false,
            }),
        );
        assert!(result.is_err());
        if let Err(error) = result {
            assert_eq!(error.code, STATEFUL_BROWSER_STORE_UNAVAILABLE);
            assert_eq!(error.reason, "public_integrity_unavailable");
        }
    }

    #[test]
    fn denial_evidence_is_secret_free() {
        let error = activation_error("missing_bound_store");
        let serialized = to_value(&error);
        assert!(serialized.is_ok());
        if let Ok(value) = serialized {
            let text = value.to_string();
            assert!(!text.contains("payload"));
            assert!(!text.contains("databaseName"));
            assert!(!text.contains("/var/"));
            assert!(!text.contains("indexeddb://"));
        }
    }

    #[test]
    fn public_types_round_trip_debug_clone_and_equality() {
        let facts = IndexedDbOpenAttestation {
            exclusive_lock_held: true,
            public_integrity_available: true,
        };
        let copied = facts;
        assert_eq!(facts, copied);
        assert_eq!(
            StatefulBrowserBoundStore::IndexedDb(facts),
            StatefulBrowserBoundStore::IndexedDb(copied)
        );
        assert_ne!(
            StatefulBrowserBoundStore::Missing,
            StatefulBrowserBoundStore::NonIndexedDb
        );

        let evidence = StatefulBrowserActivationEvidence {
            governing_spec: "132-stateful-browser-placement",
            backend: "indexeddb",
            exclusive_lock_held: true,
            public_integrity_available: true,
            outcome: "attested",
        };
        let cloned = evidence.clone();
        assert_eq!(evidence, cloned);
        assert!(format!("{evidence:?}").contains("attested"));
        assert!(to_value(&evidence).is_ok());

        let error = activation_error("missing_bound_store");
        let cloned_error = error.clone();
        assert_eq!(error, cloned_error);
        assert!(format!("{error:?}").contains(STATEFUL_BROWSER_STORE_UNAVAILABLE));
        assert!(to_value(&error).is_ok());
    }
}
