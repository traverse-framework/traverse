//! Thin CLI wrappers for Decision 80 authoring helpers (specs 113 / 112).
//!
//! `workflow plan` and `workflow promote` are deterministic file→JSON surfaces
//! over the existing MCP library tools. They never auto-seal, never mutate the
//! catalog, and never accept natural-language goals.

use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;
use traverse_contracts::{Lifecycle, ManifestReference, Owner};
use traverse_mcp::tools::workflow_plan::{PlanRequest, PlanTarget, plan_workflow};
use traverse_mcp::tools::workflow_promotion::{
    PromotedWorkflowIdentity, WorkflowCandidateArtifact, finalize_candidate_into_definition,
};
use traverse_registry::load_application_bundle_manifest;

use crate::{CliError, RegisteredBundle, load_registered_bundle};

#[derive(Debug, Deserialize)]
struct PromoteIdentityFile {
    id: String,
    name: String,
    version: String,
    owner: Owner,
    lifecycle: Lifecycle,
    summary: String,
    #[serde(default)]
    tags: Vec<String>,
}

/// Runs `plan_workflow` against an application manifest + capability registry
/// bundle and returns pretty-printed candidate JSON.
pub(crate) fn workflow_plan(
    app_manifest_path: &Path,
    registry_bundle_path: &Path,
    starting_facts_path: &Path,
    target_capability: Option<&str>,
    target_event: Option<&str>,
    workspace_id: &str,
) -> Result<String, CliError> {
    let target = parse_plan_target(target_capability, target_event)?;
    let starting_facts = read_json_value(starting_facts_path)?;
    let manifest_bytes = fs::read(app_manifest_path).map_err(|error| {
        CliError::IoError(format!(
            "failed to read app manifest {}: {error}",
            app_manifest_path.display()
        ))
    })?;
    let manifest = load_application_bundle_manifest(app_manifest_path).map_err(|failure| {
        CliError::ValidationFailed(format!(
            "invalid application manifest {}: {}",
            app_manifest_path.display(),
            failure
                .errors
                .first()
                .map_or("unknown validation failure", |error| error.message.as_str())
        ))
    })?;
    let RegisteredBundle {
        capability_registry: registry,
        ..
    } = load_registered_bundle(registry_bundle_path)?;
    let app_manifest = ManifestReference {
        app_id: manifest.app_id.clone(),
        app_version: manifest.version.clone(),
        manifest_digest: digest_bytes(&manifest_bytes),
    };
    let response = plan_workflow(&PlanRequest {
        target: &target,
        starting_facts: &starting_facts,
        manifest: &manifest,
        app_manifest: &app_manifest,
        registry: &registry,
        workspace_id,
    });
    serde_json::to_string_pretty(&response)
        .map_err(|error| CliError::IoError(format!("failed to serialize plan response: {error}")))
}

/// Finalizes a reviewed candidate into a registrable workflow definition JSON.
/// Does not register; callers must run `workflow register` separately.
pub(crate) fn workflow_promote_finalize(
    candidate_path: &Path,
    identity_path: &Path,
    acknowledge_unconfirmed: bool,
) -> Result<String, CliError> {
    let candidate: WorkflowCandidateArtifact = read_json(candidate_path)?;
    if !candidate.unconfirmed_mappings.is_empty() && !acknowledge_unconfirmed {
        return Err(CliError::ValidationFailed(format!(
            "candidate has {} unconfirmed mapping(s); pass --acknowledge-unconfirmed after review",
            candidate.unconfirmed_mappings.len()
        )));
    }
    let identity_file: PromoteIdentityFile = read_json(identity_path)?;
    let identity = PromotedWorkflowIdentity {
        id: identity_file.id,
        name: identity_file.name,
        version: identity_file.version,
        owner: identity_file.owner,
        lifecycle: identity_file.lifecycle,
        summary: identity_file.summary,
        tags: identity_file.tags,
    };
    let definition = finalize_candidate_into_definition(&candidate, identity);
    serde_json::to_string_pretty(&definition).map_err(|error| {
        CliError::IoError(format!("failed to serialize workflow definition: {error}"))
    })
}

fn parse_plan_target(
    target_capability: Option<&str>,
    target_event: Option<&str>,
) -> Result<PlanTarget, CliError> {
    match (target_capability, target_event) {
        (Some(capability), None) => {
            let (capability_id, capability_version) =
                capability.split_once('@').ok_or_else(|| {
                    CliError::ValidationFailed(
                        "--target-capability must be <capability_id>@<version>".to_string(),
                    )
                })?;
            if capability_id.is_empty() || capability_version.is_empty() {
                return Err(CliError::ValidationFailed(
                    "--target-capability must be <capability_id>@<version>".to_string(),
                ));
            }
            Ok(PlanTarget::Capability {
                capability_id: capability_id.to_string(),
                capability_version: capability_version.to_string(),
            })
        }
        (None, Some(event_type)) if !event_type.trim().is_empty() => Ok(PlanTarget::EmitsEvent {
            event_type: event_type.to_string(),
        }),
        (Some(_), Some(_)) => Err(CliError::ValidationFailed(
            "provide exactly one of --target-capability or --target-event".to_string(),
        )),
        _ => Err(CliError::ValidationFailed(
            "provide --target-capability <id@version> or --target-event <event_type>".to_string(),
        )),
    }
}

fn read_json_value(path: &Path) -> Result<Value, CliError> {
    let bytes = fs::read(path).map_err(|error| {
        CliError::IoError(format!("failed to read {}: {error}", path.display()))
    })?;
    serde_json::from_slice(&bytes).map_err(|error| {
        CliError::ValidationFailed(format!("invalid JSON in {}: {error}", path.display()))
    })
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, CliError> {
    let bytes = fs::read(path).map_err(|error| {
        CliError::IoError(format!("failed to read {}: {error}", path.display()))
    })?;
    serde_json::from_slice(&bytes).map_err(|error| {
        CliError::ValidationFailed(format!("invalid JSON in {}: {error}", path.display()))
    })
}

fn digest_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = std::fmt::Write::write_fmt(&mut hex, format_args!("{byte:02x}"));
    }
    format!("sha256:{hex}")
}

#[cfg(test)]
mod tests {
    use super::{parse_plan_target, workflow_promote_finalize};
    use std::fs;
    use traverse_mcp::tools::workflow_promotion::WorkflowCandidateArtifact;

    #[test]
    fn plan_target_requires_exactly_one_selector() {
        assert!(parse_plan_target(None, None).is_err());
        assert!(parse_plan_target(Some("a@1"), Some("evt")).is_err());
        assert!(parse_plan_target(Some("missing-version"), None).is_err());
        assert!(parse_plan_target(Some("cap@1.0.0"), None).is_ok());
        assert!(parse_plan_target(None, Some("evt.type")).is_ok());
    }

    #[test]
    fn promote_finalize_rejects_unconfirmed_without_ack() {
        let dir = std::env::temp_dir().join(format!(
            "traverse-promote-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        assert!(
            fs::create_dir_all(&dir).is_ok(),
            "temp dir must be creatable"
        );
        let candidate_path = dir.join("candidate.json");
        let identity_path = dir.join("identity.json");
        let candidate = WorkflowCandidateArtifact {
            source_proposal_id: "p1".to_string(),
            source_proposal_digest: "d1".to_string(),
            source_snapshot_digest: "s1".to_string(),
            nodes: Vec::new(),
            edges: Vec::new(),
            start_node: "a".to_string(),
            terminal_nodes: vec!["a".to_string()],
            unconfirmed_mappings: vec![
                traverse_mcp::tools::workflow_promotion::UnconfirmedMapping {
                    source_path: "/a".to_string(),
                    target_node_id: "a".to_string(),
                    target_path: "/b".to_string(),
                    reason: "rename".to_string(),
                },
            ],
            excluded_fields: Vec::new(),
        };
        let candidate_bytes = match serde_json::to_vec_pretty(&candidate) {
            Ok(bytes) => bytes,
            Err(_) => return,
        };
        if fs::write(&candidate_path, candidate_bytes).is_err() {
            return;
        }
        if fs::write(
            &identity_path,
            br#"{
              "id": "demo.workflow",
              "name": "Demo",
              "version": "1.0.0",
              "owner": {"team":"demo","contact":"demo@example.com"},
              "lifecycle": "active",
              "summary": "demo",
              "tags": []
            }"#,
        )
        .is_err()
        {
            return;
        }
        let result = workflow_promote_finalize(&candidate_path, &identity_path, false);
        assert!(
            result
                .as_ref()
                .err()
                .is_some_and(|err| err.to_string().contains("acknowledge-unconfirmed")),
            "finalize without acknowledge-unconfirmed must fail: {result:?}"
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
