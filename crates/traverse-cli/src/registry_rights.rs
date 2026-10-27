//! Consumer-side view of a synced public registry record's model rights and
//! lifecycle status (Spec 055 v1.1.0 FR-014–FR-017, registry spec 026).
//!
//! `registry sync` already keeps the spec 026 `ai`, `status`, `model_usage`
//! and `revocation` fields in local state. This module only projects them
//! for `registry inspect` and the `registry search` filters. It never
//! decides permission: `conditional`, `forbidden`, `unknown` and an
//! undeclared right are all shown as declared, and enforcement belongs to
//! the runtime.

use serde_json::{Value, json};
use std::fmt::Write as _;
use traverse_registry::{
    ModelDerivationDeclaration, ModelRight, ModelUsageClass, PublicModelRef, PublicModelReference,
    PublicRecordStatus, PublicRegistryCapabilityRecord,
};

/// Printed wherever the publisher declared nothing (FR-015).
pub(crate) const NOT_STATED: &str = "not stated";
/// Printed for a grandfathered `string[]` model reference (FR-016).
pub(crate) const LEGACY_RIGHTS: &str = "not recorded (legacy shape)";

pub(crate) const STATUS_VALUES: [&str; 3] = ["active", "deprecated", "revoked"];

pub(crate) fn status_label(status: PublicRecordStatus) -> &'static str {
    match status {
        PublicRecordStatus::Active => "active",
        PublicRecordStatus::Deprecated => "deprecated",
        PublicRecordStatus::Revoked => "revoked",
    }
}

fn right_label(right: Option<ModelRight>) -> &'static str {
    match right {
        Some(ModelRight::Allowed) => "allowed",
        Some(ModelRight::Forbidden) => "forbidden",
        Some(ModelRight::Conditional) => "conditional",
        Some(ModelRight::Unknown) => "unknown",
        None => NOT_STATED,
    }
}

fn usage_class_label(class: Option<ModelUsageClass>) -> &'static str {
    match class {
        Some(ModelUsageClass::Unrestricted) => "unrestricted",
        Some(ModelUsageClass::EvaluationOnly) => "evaluation-only",
        Some(ModelUsageClass::Conditional) => "conditional",
        None => NOT_STATED,
    }
}

/// Whether the record's own lifecycle status equals `value`.
pub(crate) fn matches_status(record: &PublicRegistryCapabilityRecord, value: &str) -> bool {
    status_label(record.lifecycle_status()) == value
}

/// Whether any object-shaped model on the record declares `commercial_use`
/// equal to `value`. Legacy and undeclared models never match.
pub(crate) fn matches_model_commercial_use(
    record: &PublicRegistryCapabilityRecord,
    value: &str,
) -> bool {
    object_models(record)
        .any(|model| model.commercial_use.is_some() && right_label(model.commercial_use) == value)
}

fn object_models(record: &PublicRegistryCapabilityRecord) -> impl Iterator<Item = &PublicModelRef> {
    record
        .ai
        .iter()
        .flat_map(|ai| ai.models.iter())
        .filter_map(|model| match model {
            PublicModelReference::Object(model_ref) => Some(model_ref.as_ref()),
            PublicModelReference::Legacy(_) => None,
        })
}

/// Lifecycle status, revocation record and per-model rights, as JSON.
pub(crate) fn record_rights_json(record: &PublicRegistryCapabilityRecord) -> Value {
    json!({
        "status": status_label(record.lifecycle_status()),
        "revocation": record.revocation.as_ref().map(|revocation| json!({
            "reason": revocation.reason,
            "evidence_url": revocation.evidence_url,
            "revoked_at": revocation.revoked_at,
        })),
        "model_backed": record.ai.as_ref().is_some_and(|ai| ai.model_backed),
        "models": record
            .ai
            .iter()
            .flat_map(|ai| ai.models.iter())
            .map(model_json)
            .collect::<Vec<_>>(),
    })
}

fn model_json(model: &PublicModelReference) -> Value {
    let model_ref = match model {
        PublicModelReference::Legacy(id) => {
            return json!({ "id": id, "shape": "legacy", "rights": LEGACY_RIGHTS });
        }
        PublicModelReference::Object(model_ref) => model_ref,
    };
    json!({
        "id": model_ref.id,
        "shape": "object",
        "spdx_expression": model_ref.spdx_expression,
        "attribution_required": model_ref.attribution_required,
        "commercial_use": right_label(model_ref.commercial_use),
        "redistribution": right_label(model_ref.redistribution),
        "derivatives": right_label(model_ref.derivatives),
        "usage_class": usage_class_label(model_ref.usage_class()),
        "license_files": model_ref.license_files.iter().map(|file| json!({
            "url": file.url,
            "sha256": file.sha256,
        })).collect::<Vec<_>>(),
        "notice_files": model_ref.notice_files.iter().map(|file| json!({
            "url": file.url,
            "sha256": file.sha256,
        })).collect::<Vec<_>>(),
        "derivation": derivation_json(model_ref.derivation.as_ref()),
        "data_obligations": model_ref.data_obligations.as_ref().map_or_else(
            || json!(NOT_STATED),
            |obligations| json!(obligations),
        ),
        "verification_status": model_ref
            .verification
            .as_ref()
            .map_or(NOT_STATED, |verification| verification.status.as_str()),
    })
}

fn derivation_json(derivation: Option<&ModelDerivationDeclaration>) -> Value {
    match derivation {
        None => json!({ "kind": NOT_STATED }),
        Some(ModelDerivationDeclaration::Verbatim) => json!({ "kind": "verbatim" }),
        Some(ModelDerivationDeclaration::Converted(converted)) => json!({
            "kind": "converted",
            "transformations": converted.transformations,
            "description": converted.description,
            "tool_url": converted.tool_url,
            "converted_sha256": converted.converted_sha256,
        }),
    }
}

/// Human-readable rendering of [`record_rights_json`], one record per block.
pub(crate) fn render_record_rights(
    record: &PublicRegistryCapabilityRecord,
    output: &mut String,
) -> std::fmt::Result {
    writeln!(
        output,
        "{}:{}@{}",
        record.namespace, record.id, record.version
    )?;
    writeln!(
        output,
        "  status: {}",
        status_label(record.lifecycle_status())
    )?;
    if let Some(revocation) = &record.revocation {
        writeln!(
            output,
            "  revocation: {} (revoked_at {}, evidence {})",
            revocation.reason, revocation.revoked_at, revocation.evidence_url
        )?;
    }
    let models = record
        .ai
        .iter()
        .flat_map(|ai| ai.models.iter())
        .collect::<Vec<_>>();
    if models.is_empty() {
        return writeln!(output, "  models: none");
    }
    writeln!(output, "  models:")?;
    for model in models {
        render_model(model, output)?;
    }
    Ok(())
}

fn render_model(model: &PublicModelReference, output: &mut String) -> std::fmt::Result {
    let model_ref = match model {
        PublicModelReference::Legacy(id) => {
            writeln!(output, "  - id: {id}")?;
            return writeln!(output, "    rights: {LEGACY_RIGHTS}");
        }
        PublicModelReference::Object(model_ref) => model_ref,
    };
    writeln!(output, "  - id: {}", model_ref.id)?;
    writeln!(output, "    spdx_expression: {}", model_ref.spdx_expression)?;
    writeln!(
        output,
        "    attribution_required: {}",
        model_ref.attribution_required
    )?;
    writeln!(
        output,
        "    commercial_use: {}",
        right_label(model_ref.commercial_use)
    )?;
    writeln!(
        output,
        "    redistribution: {}",
        right_label(model_ref.redistribution)
    )?;
    writeln!(
        output,
        "    derivatives: {}",
        right_label(model_ref.derivatives)
    )?;
    writeln!(
        output,
        "    usage_class: {}",
        usage_class_label(model_ref.usage_class())
    )?;
    for (label, files) in [
        ("license_files", &model_ref.license_files),
        ("notice_files", &model_ref.notice_files),
    ] {
        if files.is_empty() {
            writeln!(output, "    {label}: none")?;
        }
        for file in files {
            writeln!(output, "    {label}: {} (sha256 {})", file.url, file.sha256)?;
        }
    }
    match &model_ref.derivation {
        None => writeln!(output, "    derivation: {NOT_STATED}")?,
        Some(ModelDerivationDeclaration::Verbatim) => writeln!(output, "    derivation: verbatim")?,
        Some(ModelDerivationDeclaration::Converted(converted)) => writeln!(
            output,
            "    derivation: converted ({}; converted_sha256 {})",
            converted.description, converted.converted_sha256
        )?,
    }
    match &model_ref.data_obligations {
        None => writeln!(output, "    data_obligations: {NOT_STATED}")?,
        Some(obligations) if obligations.is_empty() => {
            writeln!(output, "    data_obligations: none")?;
        }
        Some(obligations) => {
            for obligation in obligations {
                writeln!(
                    output,
                    "    data_obligation: {} ({:?}): {}",
                    obligation.dataset, obligation.kind, obligation.obligation
                )?;
            }
        }
    }
    writeln!(
        output,
        "    verification_status: {}",
        model_ref
            .verification
            .as_ref()
            .map_or(NOT_STATED, |verification| verification.status.as_str())
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;
    use traverse_registry::{PublicAiDeclaration, PublicRevocation};

    fn plain_record() -> PublicRegistryCapabilityRecord {
        serde_json::from_value(json!({
            "namespace": "demo",
            "id": "classify",
            "version": "1.0.0",
            "digest": "sha256:a",
            "artifact_url": "https://example.test/a.wasm",
            "contract_digest": "sha256:c",
            "contract_url": "https://example.test/contract.json",
            "deprecated": false
        }))
        .expect("fixture record")
    }

    fn full_model() -> PublicModelReference {
        serde_json::from_value(json!({
            "id": "audio.yamnet",
            "spdx_expression": "Apache-2.0",
            "attribution_required": true,
            "huggingface_id": "google/yamnet",
            "revision": "0123456789abcdef0123456789abcdef01234567",
            "commercial_use": "allowed",
            "redistribution": "allowed",
            "derivatives": "conditional",
            "license_files": [{"url": "https://example.test/LICENSE", "sha256": "aa"}],
            "notice_files": [{"url": "https://example.test/NOTICE", "sha256": "bb"}],
            "derivation": {
                "transformations": ["format-convert"],
                "description": "tflite to onnx",
                "converted_sha256": "cc"
            },
            "data_obligations": [{
                "dataset": "AudioSet",
                "kind": "labels",
                "obligation": "CC BY-4.0 attribution",
                "source_url": "https://example.test/audioset"
            }],
            "verification": {"status": "maintainer-declared"}
        }))
        .expect("fixture model")
    }

    fn with_models(models: Vec<PublicModelReference>) -> PublicRegistryCapabilityRecord {
        let mut record = plain_record();
        record.ai = Some(PublicAiDeclaration {
            model_backed: true,
            models,
        });
        record
    }

    #[test]
    fn full_rights_record_is_projected_to_json() {
        let json = record_rights_json(&with_models(vec![full_model()]));
        assert_eq!(json["status"], "active");
        assert_eq!(json["revocation"], Value::Null);
        assert_eq!(json["model_backed"], true);
        let model = &json["models"][0];
        assert_eq!(model["shape"], "object");
        assert_eq!(model["spdx_expression"], "Apache-2.0");
        assert_eq!(model["commercial_use"], "allowed");
        assert_eq!(model["derivatives"], "conditional");
        assert_eq!(model["usage_class"], "conditional");
        assert_eq!(model["license_files"][0]["sha256"], "aa");
        assert_eq!(
            model["notice_files"][0]["url"],
            "https://example.test/NOTICE"
        );
        assert_eq!(model["derivation"]["kind"], "converted");
        assert_eq!(model["derivation"]["transformations"][0], "format-convert");
        assert_eq!(model["data_obligations"][0]["kind"], "labels");
        assert_eq!(model["verification_status"], "maintainer-declared");
    }

    #[test]
    fn absent_fields_render_as_not_stated_and_legacy_is_not_an_error() {
        let bare: PublicModelReference = serde_json::from_value(json!({
            "id": "bare",
            "spdx_expression": "MIT",
            "attribution_required": false,
            "source_url": "https://example.test/commit/abc"
        }))
        .expect("bare model");
        let verbatim: PublicModelReference = serde_json::from_value(json!({
            "id": "verbatim",
            "spdx_expression": "MIT",
            "attribution_required": false,
            "source_url": "https://example.test/commit/abc",
            "derivation": null,
            "data_obligations": []
        }))
        .expect("verbatim model");
        let record = with_models(vec![
            PublicModelReference::Legacy("legacy-model".to_string()),
            bare,
            verbatim,
        ]);
        let json = record_rights_json(&record);
        assert_eq!(json["models"][0]["rights"], LEGACY_RIGHTS);
        assert_eq!(json["models"][1]["commercial_use"], NOT_STATED);
        assert_eq!(json["models"][1]["usage_class"], NOT_STATED);
        assert_eq!(json["models"][1]["derivation"]["kind"], NOT_STATED);
        assert_eq!(json["models"][1]["data_obligations"], NOT_STATED);
        assert_eq!(json["models"][1]["verification_status"], NOT_STATED);
        assert_eq!(json["models"][2]["derivation"]["kind"], "verbatim");
        assert_eq!(json["models"][2]["data_obligations"], json!([]));

        let mut human = String::new();
        render_record_rights(&record, &mut human).expect("render");
        assert!(human.contains("rights: not recorded (legacy shape)"));
        assert!(human.contains("data_obligations: not stated"));
        assert!(human.contains("data_obligations: none"));
        assert!(human.contains("derivation: not stated"));
        assert!(human.contains("derivation: verbatim"));
        assert!(human.contains("license_files: none"));
        assert!(human.contains("usage_class: not stated"));
    }

    #[test]
    fn revoked_record_carries_revocation_and_matches_status_filter() {
        let mut record = with_models(vec![full_model()]);
        record.status = Some(PublicRecordStatus::Revoked);
        record.revocation = Some(PublicRevocation {
            reason: "upstream license withdrawn".to_string(),
            evidence_url: "https://example.test/evidence".to_string(),
            revoked_at: "2026-10-01T00:00:00Z".to_string(),
        });
        let json = record_rights_json(&record);
        assert_eq!(json["status"], "revoked");
        assert_eq!(json["revocation"]["reason"], "upstream license withdrawn");
        assert!(matches_status(&record, "revoked"));
        assert!(!matches_status(&record, "active"));

        let mut human = String::new();
        render_record_rights(&record, &mut human).expect("render");
        assert!(human.starts_with("demo:classify@1.0.0\n  status: revoked\n"));
        assert!(human.contains("revocation: upstream license withdrawn"));
        assert!(human.contains("derivation: converted (tflite to onnx"));
        assert!(human.contains("data_obligation: AudioSet (Labels): CC BY-4.0 attribution"));
        assert!(human.contains("license_files: https://example.test/LICENSE (sha256 aa)"));

        let mut deprecated = record.clone();
        deprecated.status = None;
        deprecated.deprecated = true;
        assert!(matches_status(&deprecated, "deprecated"));
    }

    #[test]
    fn model_commercial_use_filter_ignores_legacy_and_undeclared() {
        let record = with_models(vec![full_model()]);
        assert!(matches_model_commercial_use(&record, "allowed"));
        assert!(!matches_model_commercial_use(&record, "forbidden"));

        let legacy = with_models(vec![PublicModelReference::Legacy("x".to_string())]);
        assert!(!matches_model_commercial_use(&legacy, "allowed"));
        assert!(!matches_model_commercial_use(&legacy, NOT_STATED));
        assert!(!matches_model_commercial_use(&plain_record(), "allowed"));
    }

    #[test]
    fn plain_capability_renders_no_models() {
        let json = record_rights_json(&plain_record());
        assert_eq!(json["model_backed"], false);
        assert_eq!(json["models"], json!([]));
        let mut human = String::new();
        render_record_rights(&plain_record(), &mut human).expect("render");
        assert!(human.ends_with("  models: none\n"));
    }

    #[test]
    fn every_right_and_usage_class_has_a_stable_label() {
        assert_eq!(right_label(Some(ModelRight::Forbidden)), "forbidden");
        assert_eq!(right_label(Some(ModelRight::Unknown)), "unknown");
        assert_eq!(
            usage_class_label(Some(ModelUsageClass::Unrestricted)),
            "unrestricted"
        );
        assert_eq!(
            usage_class_label(Some(ModelUsageClass::EvaluationOnly)),
            "evaluation-only"
        );
        assert_eq!(status_label(PublicRecordStatus::Deprecated), "deprecated");
    }
}
