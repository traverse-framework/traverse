//! Spec `045` v1.1.0 FR-021–FR-024: host policy over a resolved public
//! registry record's lifecycle status and registry spec 026 model rights.
//!
//! Range selection already skips revoked versions (`is_active`). An exact
//! pin still resolves a revoked record (registry decision-log entry 127
//! Q11), so this check is the fail-closed do-not-run gate every consumer
//! applies before it prepares or activates the record. `conditional` rights
//! are denied unless the host opts in explicitly; `unknown` is always
//! denied. A model reference that predates spec 026 (no rights declared,
//! or the legacy string shape) is grandfathered and passes.

use traverse_registry::{
    ModelRight, PublicModelReference, PublicRecordStatus, PublicRegistryCapabilityRecord,
};

/// Host-owned rights policy. The default denies `conditional` rights.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ModelRightsPolicy {
    /// Allow models whose declared rights include `conditional`.
    pub allow_conditional: bool,
}

/// Stable do-not-run decision codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistryRightsDenialCode {
    /// The resolved version is revoked.
    Revoked,
    /// A model declares a `conditional` right and the host did not opt in.
    ConditionalRightsDenied,
    /// A model declares an `unknown` right.
    UnknownRightsDenied,
}

impl RegistryRightsDenialCode {
    /// Stable `snake_case` wire representation.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Revoked => "registry_dependency_revoked",
            Self::ConditionalRightsDenied => "model_rights_conditional_denied",
            Self::UnknownRightsDenied => "model_rights_unknown_denied",
        }
    }
}

/// A fail-closed decision with a deterministic, secret-free message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryRightsDenial {
    pub code: RegistryRightsDenialCode,
    pub message: String,
}

/// Decide whether `record` may be prepared or activated under `policy`.
///
/// # Errors
///
/// Returns the first denial in a fixed order: revocation, then each model
/// in declaration order, checking `commercial_use`, `redistribution`, then
/// `derivatives`.
pub fn check_registry_record_rights(
    record: &PublicRegistryCapabilityRecord,
    policy: ModelRightsPolicy,
) -> Result<(), RegistryRightsDenial> {
    let subject = format!("{}:{}@{}", record.namespace, record.id, record.version);
    if record.lifecycle_status() == PublicRecordStatus::Revoked {
        let detail = record
            .revocation
            .as_ref()
            .map_or_else(String::new, |revocation| {
                format!(
                    ": {} (revoked_at {}, evidence {})",
                    revocation.reason, revocation.revoked_at, revocation.evidence_url
                )
            });
        return Err(RegistryRightsDenial {
            code: RegistryRightsDenialCode::Revoked,
            message: format!("{subject} is revoked and must not run{detail}"),
        });
    }
    for model in record.ai.iter().flat_map(|ai| ai.models.iter()) {
        let PublicModelReference::Object(model_ref) = model else {
            continue;
        };
        for (field, right) in [
            ("commercial_use", model_ref.commercial_use),
            ("redistribution", model_ref.redistribution),
            ("derivatives", model_ref.derivatives),
        ] {
            let (code, value) = match right {
                Some(ModelRight::Unknown) => {
                    (RegistryRightsDenialCode::UnknownRightsDenied, "unknown")
                }
                Some(ModelRight::Conditional) if !policy.allow_conditional => (
                    RegistryRightsDenialCode::ConditionalRightsDenied,
                    "conditional",
                ),
                _ => continue,
            };
            return Err(RegistryRightsDenial {
                code,
                message: format!(
                    "{subject} model {} declares {field}: {value}, which the host rights policy denies",
                    model_ref.id
                ),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;
    use serde_json::json;
    use traverse_registry::PublicRevocation;

    #[allow(clippy::needless_pass_by_value)]
    fn record(models: serde_json::Value) -> PublicRegistryCapabilityRecord {
        serde_json::from_value(json!({
            "namespace": "demo",
            "id": "gate",
            "version": "1.0.0",
            "digest": "sha256:a",
            "artifact_url": "https://example.test/a.wasm",
            "contract_digest": "sha256:c",
            "contract_url": "https://example.test/contract.json",
            "deprecated": false,
            "ai": {"model_backed": true, "models": models}
        }))
        .expect("fixture record")
    }

    fn model(commercial: &str, redistribution: &str, derivatives: &str) -> serde_json::Value {
        json!({
            "id": "audio.yamnet",
            "spdx_expression": "Apache-2.0",
            "attribution_required": true,
            "source_url": "https://example.test/commit/abc",
            "commercial_use": commercial,
            "redistribution": redistribution,
            "derivatives": derivatives
        })
    }

    #[test]
    fn allowed_forbidden_legacy_and_undeclared_rights_pass() {
        let policy = ModelRightsPolicy::default();
        let allowed = record(json!([model("allowed", "allowed", "allowed")]));
        assert_eq!(check_registry_record_rights(&allowed, policy), Ok(()));
        let evaluation_only = record(json!([model("forbidden", "forbidden", "forbidden")]));
        assert_eq!(
            check_registry_record_rights(&evaluation_only, policy),
            Ok(())
        );
        let legacy = record(json!(["old-model"]));
        assert_eq!(check_registry_record_rights(&legacy, policy), Ok(()));
        let undeclared = record(json!([{
            "id": "pre-026",
            "spdx_expression": "MIT",
            "attribution_required": false,
            "source_url": "https://example.test/commit/abc"
        }]));
        assert_eq!(check_registry_record_rights(&undeclared, policy), Ok(()));
    }

    #[test]
    fn conditional_is_denied_by_default_and_allowed_by_opt_in() {
        let conditional = record(json!([model("allowed", "allowed", "conditional")]));
        let denial = check_registry_record_rights(&conditional, ModelRightsPolicy::default())
            .expect_err("conditional must be denied by default");
        assert_eq!(
            denial.code,
            RegistryRightsDenialCode::ConditionalRightsDenied
        );
        assert_eq!(denial.code.as_str(), "model_rights_conditional_denied");
        assert_eq!(
            denial.message,
            "demo:gate@1.0.0 model audio.yamnet declares derivatives: conditional, which the host rights policy denies"
        );
        let opt_in = ModelRightsPolicy {
            allow_conditional: true,
        };
        assert_eq!(check_registry_record_rights(&conditional, opt_in), Ok(()));
    }

    #[test]
    fn unknown_is_denied_even_with_opt_in() {
        let unknown = record(json!([model("unknown", "allowed", "allowed")]));
        let denial = check_registry_record_rights(
            &unknown,
            ModelRightsPolicy {
                allow_conditional: true,
            },
        )
        .expect_err("unknown must be denied");
        assert_eq!(denial.code, RegistryRightsDenialCode::UnknownRightsDenied);
        assert_eq!(denial.code.as_str(), "model_rights_unknown_denied");
        assert!(denial.message.contains("commercial_use: unknown"));
    }

    #[test]
    fn revoked_is_denied_with_its_revocation_record_before_rights() {
        let mut revoked = record(json!([model("unknown", "allowed", "allowed")]));
        revoked.status = Some(PublicRecordStatus::Revoked);
        let denial = check_registry_record_rights(&revoked, ModelRightsPolicy::default())
            .expect_err("revoked must be denied");
        assert_eq!(denial.code.as_str(), "registry_dependency_revoked");
        assert_eq!(
            denial.message,
            "demo:gate@1.0.0 is revoked and must not run"
        );

        revoked.revocation = Some(PublicRevocation {
            reason: "license withdrawn".to_string(),
            evidence_url: "https://example.test/e".to_string(),
            revoked_at: "2026-10-01T00:00:00Z".to_string(),
        });
        let denial = check_registry_record_rights(&revoked, ModelRightsPolicy::default())
            .expect_err("revoked must be denied");
        assert_eq!(denial.code, RegistryRightsDenialCode::Revoked);
        assert_eq!(
            denial.message,
            "demo:gate@1.0.0 is revoked and must not run: license withdrawn (revoked_at 2026-10-01T00:00:00Z, evidence https://example.test/e)"
        );
    }

    #[test]
    fn deprecated_is_not_a_rights_denial() {
        let mut deprecated = record(json!([model("allowed", "allowed", "allowed")]));
        deprecated.deprecated = true;
        assert_eq!(
            check_registry_record_rights(&deprecated, ModelRightsPolicy::default()),
            Ok(())
        );
    }
}
