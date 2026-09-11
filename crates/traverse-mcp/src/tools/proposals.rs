//! MCP tool surfaces for the runtime workflow proposal lifecycle.
//!
//! Governed by spec `109-runtime-workflow-proposals`. Mirrors the
//! `tools::capabilities` pattern: plain, fully-tested Rust functions form the
//! public MCP surface (spec 015's precedent), independent of the separate
//! `stdio_server.rs` reference host transport.

use ed25519_dalek::VerifyingKey;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;

use traverse_contracts::{
    ProposalLimits, ProposalValidationError as StructuralError, SnapshotDigests, WorkflowProposal,
    canonicalize_proposal, proposal_digest, proposal_snapshot_digest,
};
use traverse_registry::{ApplicationBundleManifest, CapabilityRegistry};
use traverse_runtime::proposal::{
    ApprovalTokenStore, ApprovalTokenVerificationContext, AuthorizationDecision,
    AuthorizationSummary, ProposalCrossValidationError as CrossError, ProposalTrace, QuotaLimits,
    QuotaReservation, QuotaTracker, ResolvedProposalNode, execute_proposal,
    proposal_is_automatic_eligible, validate_proposal_against_host_state, verify_approval_token,
};
use traverse_runtime::{LocalExecutor, Runtime};

use crate::{McpError, McpErrorCode};

/// Decision 80 dual-path: sealed workflows are the default production path;
/// live proposal validate/submit/execute requires explicit `adaptive` opt-in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CompositionMode {
    #[default]
    Sealed,
    Adaptive,
}

impl CompositionMode {
    /// Parses an optional request/app field. Missing or empty means sealed.
    ///
    /// # Errors
    ///
    /// Returns a stable secret-free message when the value is present but not
    /// `sealed` or `adaptive`.
    pub fn parse(raw: Option<&str>) -> Result<Self, &'static str> {
        match raw.map(str::trim) {
            None | Some("") | Some("sealed") => Ok(Self::Sealed),
            Some("adaptive") => Ok(Self::Adaptive),
            Some(_) => Err("composition_mode must be sealed or adaptive when present"),
        }
    }

    #[must_use]
    pub const fn allows_proposal_surfaces(self) -> bool {
        matches!(self, Self::Adaptive)
    }
}

/// Stable denial when adaptive proposal surfaces are used without opt-in.
pub const ADAPTIVE_COMPOSITION_OPT_IN_REQUIRED: &str = "adaptive_composition_opt_in_required";

#[must_use]
pub fn adaptive_composition_opt_in_message() -> &'static str {
    "adaptive composition requires explicit composition_mode=adaptive; sealed workflows are the default — author and validate a pinned workflow instead"
}

/// Fail-closed gate for proposal validate/submit/execute (Decision 80 §6).
#[must_use]
pub fn deny_unless_adaptive(mode: CompositionMode) -> Option<ProposalDenial> {
    if mode.allows_proposal_surfaces() {
        None
    } else {
        Some(ProposalDenial {
            code: ADAPTIVE_COMPOSITION_OPT_IN_REQUIRED.to_string(),
            path: "/composition_mode".to_string(),
            message: adaptive_composition_opt_in_message().to_string(),
        })
    }
}

/// One stable, machine-readable, secret-free denial (spec 109 FR-010).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProposalDenial {
    pub code: String,
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProposalValidationResponse {
    pub proposal_id: String,
    pub proposal_digest: String,
    pub valid: bool,
    pub errors: Vec<ProposalDenial>,
}

/// Parses, canonicalizes, and cross-validates a proposal against the loaded
/// manifest and registry (spec 109 FR-001 validation feedback / compatibility
/// inspection). A structurally or semantically invalid proposal is a normal,
/// structured response, never an [`McpError`] — matching FR-010's stable
/// denial-code contract.
///
/// # Errors
///
/// Returns [`McpError`] only when `proposal_json` is not valid JSON or does
/// not deserialize into the proposal wire shape at all.
pub fn validate_proposal(
    proposal_json: &str,
    manifest: &ApplicationBundleManifest,
    registry: &CapabilityRegistry,
    limits: &ProposalLimits,
) -> Result<ProposalValidationResponse, McpError> {
    let (proposal, proposal_id, digest) = parse_and_digest(proposal_json)?;

    let canonical = match canonicalize_proposal(proposal, limits) {
        Ok(canonical) => canonical,
        Err(failure) => {
            return Ok(ProposalValidationResponse {
                proposal_id,
                proposal_digest: digest,
                valid: false,
                errors: failure.errors.into_iter().map(structural_denial).collect(),
            });
        }
    };

    match validate_proposal_against_host_state(&canonical, manifest, registry) {
        Ok(_resolved) => Ok(ProposalValidationResponse {
            proposal_id,
            proposal_digest: digest,
            valid: true,
            errors: Vec::new(),
        }),
        Err(failure) => Ok(ProposalValidationResponse {
            proposal_id,
            proposal_digest: digest,
            valid: false,
            errors: failure.errors.into_iter().map(cross_denial).collect(),
        }),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ProposalSubmissionResponse {
    pub proposal_id: String,
    pub proposal_digest: String,
    pub snapshot_digest: String,
    pub valid: bool,
    pub errors: Vec<ProposalDenial>,
    pub automatic_eligible: bool,
}

/// Submits a proposal: validates it, then binds its digest to the pinned
/// governing snapshots (spec 109 FR-003) and decides automatic eligibility
/// (FR-006). P1 has no server-side proposal catalog — submission does not
/// persist anything; the caller re-presents the same JSON to `execute`.
///
/// # Errors
///
/// Returns [`McpError`] only when `proposal_json` is not valid JSON.
pub fn submit_proposal(
    proposal_json: &str,
    manifest: &ApplicationBundleManifest,
    registry: &CapabilityRegistry,
    limits: &ProposalLimits,
    snapshots: &SnapshotDigests,
) -> Result<ProposalSubmissionResponse, McpError> {
    let (proposal, proposal_id, digest) = parse_and_digest(proposal_json)?;
    let snapshot_digest = proposal_snapshot_digest(&digest, snapshots);

    let canonical = match canonicalize_proposal(proposal, limits) {
        Ok(canonical) => canonical,
        Err(failure) => {
            return Ok(ProposalSubmissionResponse {
                proposal_id,
                proposal_digest: digest,
                snapshot_digest,
                valid: false,
                errors: failure.errors.into_iter().map(structural_denial).collect(),
                automatic_eligible: false,
            });
        }
    };

    match validate_proposal_against_host_state(&canonical, manifest, registry) {
        Ok(resolved) => Ok(ProposalSubmissionResponse {
            proposal_id,
            proposal_digest: digest,
            snapshot_digest,
            valid: true,
            errors: Vec::new(),
            automatic_eligible: proposal_is_automatic_eligible(&resolved),
        }),
        Err(failure) => Ok(ProposalSubmissionResponse {
            proposal_id,
            proposal_digest: digest,
            snapshot_digest,
            valid: false,
            errors: failure.errors.into_iter().map(cross_denial).collect(),
            automatic_eligible: false,
        }),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum AuthorizationState {
    Invalid { errors: Vec<ProposalDenial> },
    Automatic,
    RequiresApprovalToken,
}

/// Reports whether a proposal is automatic-eligible or requires a verified
/// approval token (spec 109 FR-006), without executing anything.
///
/// # Errors
///
/// Returns [`McpError`] only when `proposal_json` is not valid JSON.
pub fn authorization_state(
    proposal_json: &str,
    manifest: &ApplicationBundleManifest,
    registry: &CapabilityRegistry,
    limits: &ProposalLimits,
) -> Result<AuthorizationState, McpError> {
    let (proposal, _proposal_id, _digest) = parse_and_digest(proposal_json)?;

    let canonical = match canonicalize_proposal(proposal, limits) {
        Ok(canonical) => canonical,
        Err(failure) => {
            return Ok(AuthorizationState::Invalid {
                errors: failure.errors.into_iter().map(structural_denial).collect(),
            });
        }
    };

    match validate_proposal_against_host_state(&canonical, manifest, registry) {
        Ok(resolved) => Ok(if proposal_is_automatic_eligible(&resolved) {
            AuthorizationState::Automatic
        } else {
            AuthorizationState::RequiresApprovalToken
        }),
        Err(failure) => Ok(AuthorizationState::Invalid {
            errors: failure.errors.into_iter().map(cross_denial).collect(),
        }),
    }
}

/// Everything [`execute_proposal_via_mcp`] needs beyond the caller's runtime
/// and shared authorization/quota state — bundled to keep the function
/// signature reasonable given spec 109's many independently-required checks.
pub struct ProposalExecutionRequest<'a> {
    pub proposal_json: &'a str,
    pub manifest: &'a ApplicationBundleManifest,
    pub registry: &'a CapabilityRegistry,
    pub limits: &'a ProposalLimits,
    pub snapshots: &'a SnapshotDigests,
    pub approval_token: Option<&'a str>,
    pub expected_token_issuer: &'a str,
    pub expected_token_audience: &'a str,
    pub token_verifying_keys_by_key_id: &'a HashMap<String, VerifyingKey>,
    pub principal: &'a str,
    pub app_id: &'a str,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProposalExecutionResponse {
    Trace(ProposalTrace),
    Denied { code: String, message: String },
}

/// Authorizes and executes a proposal end to end (spec 109 FR-006 through
/// FR-009): validates it, decides or verifies authorization, reserves a
/// per-principal/app/workspace quota slot (FR-007b), and runs the bounded
/// sequential DAG (FR-007, FR-008). A denial for any reason — invalid
/// proposal, missing/invalid approval token, exhausted quota — is a normal
/// structured response, never an [`McpError`].
///
/// # Errors
///
/// Returns [`McpError`] only when `proposal_json` is not valid JSON.
pub fn execute_proposal_via_mcp<E: LocalExecutor>(
    runtime: &Runtime<E>,
    request: &ProposalExecutionRequest<'_>,
    token_store: &ApprovalTokenStore,
    quota_tracker: &QuotaTracker,
    quota_limits: &QuotaLimits,
) -> Result<ProposalExecutionResponse, McpError> {
    let (proposal, _proposal_id, digest) = parse_and_digest(request.proposal_json)?;
    let workspace_id = proposal.workspace_id.clone();
    let snapshot_digest = proposal_snapshot_digest(&digest, request.snapshots);

    let canonical = match canonicalize_proposal(proposal, request.limits) {
        Ok(canonical) => canonical,
        Err(failure) => {
            return Ok(ProposalExecutionResponse::Denied {
                code: "invalid_proposal".to_string(),
                message: format!("{} structural validation error(s)", failure.errors.len()),
            });
        }
    };

    let resolved = match validate_proposal_against_host_state(
        &canonical,
        request.manifest,
        request.registry,
    ) {
        Ok(resolved) => resolved,
        Err(failure) => {
            return Ok(ProposalExecutionResponse::Denied {
                code: "invalid_proposal".to_string(),
                message: format!("{} cross-validation error(s)", failure.errors.len()),
            });
        }
    };

    let authorized = match authorize_and_reserve_quota(&AuthorizeAndReserveQuotaRequest {
        resolved: &resolved,
        digest: &digest,
        snapshot_digest: &snapshot_digest,
        workspace_id: &workspace_id,
        approval_token: request.approval_token,
        expected_token_issuer: request.expected_token_issuer,
        expected_token_audience: request.expected_token_audience,
        token_verifying_keys_by_key_id: request.token_verifying_keys_by_key_id,
        token_store,
        quota_tracker,
        quota_limits,
        principal: request.principal,
        app_id: request.app_id,
    }) {
        Ok(authorized) => authorized,
        Err(denial) => return Ok(*denial),
    };

    let trace = execute_proposal(
        runtime,
        &canonical,
        &resolved,
        authorized.summary,
        &digest,
        &snapshot_digest,
    );
    drop(authorized.reservation);
    Ok(ProposalExecutionResponse::Trace(trace))
}

/// Everything [`authorize_and_reserve_quota`] needs to decide automatic vs.
/// approval-token authorization and reserve a concurrency quota slot.
/// Shared by the P1 sequential and P2 parallel MCP execute paths.
pub(crate) struct AuthorizeAndReserveQuotaRequest<'a> {
    pub resolved: &'a [ResolvedProposalNode],
    pub digest: &'a str,
    pub snapshot_digest: &'a str,
    pub workspace_id: &'a str,
    pub approval_token: Option<&'a str>,
    pub expected_token_issuer: &'a str,
    pub expected_token_audience: &'a str,
    pub token_verifying_keys_by_key_id: &'a HashMap<String, VerifyingKey>,
    pub token_store: &'a ApprovalTokenStore,
    pub quota_tracker: &'a QuotaTracker,
    pub quota_limits: &'a QuotaLimits,
    pub principal: &'a str,
    pub app_id: &'a str,
}

pub(crate) struct AuthorizedQuota<'a> {
    pub summary: AuthorizationSummary,
    pub reservation: QuotaReservation<'a>,
}

/// Decides automatic-vs-approval-token authorization (spec 109 FR-006,
/// FR-006a) and reserves a per-principal/app/workspace concurrency quota
/// slot (FR-007b). On any denial, returns the exact
/// [`ProposalExecutionResponse::Denied`] the caller should return unchanged.
pub(crate) fn authorize_and_reserve_quota<'a>(
    request: &AuthorizeAndReserveQuotaRequest<'a>,
) -> Result<AuthorizedQuota<'a>, Box<ProposalExecutionResponse>> {
    let authorization = if proposal_is_automatic_eligible(request.resolved) {
        AuthorizationDecision::Automatic
    } else {
        let Some(token) = request.approval_token else {
            return Err(Box::new(ProposalExecutionResponse::Denied {
                code: "approval_token_required".to_string(),
                message: "this proposal requires a verified approval token".to_string(),
            }));
        };
        let verification_context = ApprovalTokenVerificationContext {
            expected_issuer: request.expected_token_issuer,
            expected_audience: request.expected_token_audience,
            expected_workspace_id: request.workspace_id,
            expected_proposal_digest: request.digest,
            expected_snapshot_digest: request.snapshot_digest,
            verifying_keys_by_key_id: request.token_verifying_keys_by_key_id,
        };
        let claims = match verify_approval_token(token, &verification_context) {
            Ok(claims) => claims,
            Err(error) => {
                return Err(Box::new(ProposalExecutionResponse::Denied {
                    code: token_error_code(&error.code),
                    message: error.message,
                }));
            }
        };
        if let Err(error) = request.token_store.check_and_record_use(&claims) {
            return Err(Box::new(ProposalExecutionResponse::Denied {
                code: token_error_code(&error.code),
                message: error.message,
            }));
        }
        AuthorizationDecision::Approved(Box::new(claims))
    };

    let reservation = match request.quota_tracker.reserve(
        request.principal,
        request.app_id,
        request.workspace_id,
        request.quota_limits,
    ) {
        Ok(reservation) => reservation,
        Err(denial) => {
            return Err(Box::new(ProposalExecutionResponse::Denied {
                code: format!("quota_exhausted_{}", denial.scope),
                message: denial.message,
            }));
        }
    };

    let summary = match &authorization {
        AuthorizationDecision::Automatic => AuthorizationSummary {
            automatic: true,
            approval_token_id: None,
        },
        AuthorizationDecision::Approved(claims) => AuthorizationSummary {
            automatic: false,
            approval_token_id: Some(claims.token_id.clone()),
        },
    };

    Ok(AuthorizedQuota {
        summary,
        reservation,
    })
}

/// Renders a completed execution's redacted trace for MCP observation (spec
/// 109 FR-001 observation, FR-009). The trace itself already excludes raw
/// payloads and secrets — this is a plain JSON projection, not a second
/// redaction pass.
#[must_use]
pub fn observe_proposal(trace: &ProposalTrace) -> Value {
    serde_json::to_value(trace).unwrap_or(Value::Null)
}

#[derive(Debug, Clone, Serialize)]
pub struct ProposalExportResponse {
    pub proposal: WorkflowProposal,
    pub proposal_digest: String,
    pub execution_order: Vec<String>,
}

/// Exports a proposal's canonical form and digest so an external party can
/// independently re-derive the identical digest (spec 109 FR-001 export,
/// FR-007a: "re-submitting pinned identical inputs produces the same
/// proposal digest"). Performs structural validation only — no manifest or
/// registry cross-check.
///
/// # Errors
///
/// Returns [`McpError`] when `proposal_json` is not valid JSON or is not
/// structurally valid.
pub fn export_proposal(
    proposal_json: &str,
    limits: &ProposalLimits,
) -> Result<ProposalExportResponse, McpError> {
    let (proposal, _proposal_id, digest) = parse_and_digest(proposal_json)?;
    let canonical = canonicalize_proposal(proposal, limits).map_err(|failure| McpError {
        code: McpErrorCode::ValidationFailed,
        message: format!(
            "proposal is not structurally valid ({} error(s))",
            failure.errors.len()
        ),
    })?;
    Ok(ProposalExportResponse {
        proposal: canonical.proposal,
        proposal_digest: digest,
        execution_order: canonical.execution_order,
    })
}

pub(crate) fn parse_and_digest(
    proposal_json: &str,
) -> Result<(WorkflowProposal, String, String), McpError> {
    let proposal: WorkflowProposal = serde_json::from_str(proposal_json).map_err(|e| McpError {
        code: McpErrorCode::InvalidRequest,
        message: format!("proposal is not valid JSON: {e}"),
    })?;
    let digest = proposal_digest(&proposal);
    let proposal_id = proposal.proposal_id.clone();
    Ok((proposal, proposal_id, digest))
}

pub(crate) fn structural_denial(error: StructuralError) -> ProposalDenial {
    ProposalDenial {
        code: debug_enum_to_snake_case(&format!("{:?}", error.code)),
        path: error.path,
        message: error.message,
    }
}

pub(crate) fn cross_denial(error: CrossError) -> ProposalDenial {
    ProposalDenial {
        code: debug_enum_to_snake_case(&format!("{:?}", error.code)),
        path: error.path,
        message: error.message,
    }
}

pub(crate) fn token_error_code(
    code: &traverse_runtime::proposal::ApprovalTokenErrorCode,
) -> String {
    debug_enum_to_snake_case(&format!("{code:?}"))
}

/// Converts a Rust `Debug`-formatted `PascalCase` enum variant into the
/// stable `snake_case` string used for every machine-readable code this
/// module emits (spec 109 FR-010).
fn debug_enum_to_snake_case(value: &str) -> String {
    let mut output = String::with_capacity(value.len() + 4);
    for (index, ch) in value.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if index > 0 {
                output.push('_');
            }
            output.push(ch.to_ascii_lowercase());
        } else {
            output.push(ch);
        }
    }
    output
}

#[cfg(test)]
mod composition_mode_tests {
    use super::{
        ADAPTIVE_COMPOSITION_OPT_IN_REQUIRED, CompositionMode, deny_unless_adaptive,
    };

    #[test]
    fn missing_or_sealed_denies_proposal_surfaces() {
        assert_eq!(CompositionMode::parse(None).ok(), Some(CompositionMode::Sealed));
        assert_eq!(
            CompositionMode::parse(Some("sealed")).ok(),
            Some(CompositionMode::Sealed)
        );
        let denial = deny_unless_adaptive(CompositionMode::Sealed).expect("sealed denies");
        assert_eq!(denial.code, ADAPTIVE_COMPOSITION_OPT_IN_REQUIRED);
    }

    #[test]
    fn adaptive_allows_proposal_surfaces() {
        assert_eq!(
            CompositionMode::parse(Some("adaptive")).ok(),
            Some(CompositionMode::Adaptive)
        );
        assert!(deny_unless_adaptive(CompositionMode::Adaptive).is_none());
    }

    #[test]
    fn rejects_unknown_composition_mode_values() {
        assert!(CompositionMode::parse(Some("auto")).is_err());
    }
}
