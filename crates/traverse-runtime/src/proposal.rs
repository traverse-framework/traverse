//! Runtime workflow proposal cross-validation, authorization, quotas, and
//! sequential execution (spec `109-runtime-workflow-proposals`, P1).
//!
//! Builds on the portable proposal types and structural validation in
//! `traverse_contracts::proposal`. This module owns everything that needs
//! live host state: cross-checking a proposal against a loaded application
//! manifest and capability registry, deciding whether it is automatic-eligible
//! or requires a verified approval token, tracking per-principal/app/workspace
//! quotas, and executing the bounded sequential DAG one node at a time.

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use traverse_contracts::{
    CanonicalProposal, CapabilityContract, DataClassification, EffectClass, EgressPolicy,
    MappingSource, ProposalNode, is_automatic_eligible,
};
use traverse_registry::{ApplicationBundleManifest, CapabilityRegistry, LookupScope};

use crate::{
    PlacementTarget, Runtime, RuntimeContext, RuntimeIntent, RuntimeLookup, RuntimeLookupScope,
    RuntimeRequest, RuntimeResultStatus,
};

// ---------------------------------------------------------------------------
// Cross-validation against the loaded manifest and capability registry
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProposalCrossValidationError {
    pub code: ProposalCrossValidationErrorCode,
    pub message: String,
    pub path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalCrossValidationErrorCode {
    UndeclaredCapability,
    CapabilityNotFound,
    ArtifactDigestMismatch,
    IncompatibleMappingSchema,
    UndeclaredDataClassification,
    DataClassificationOverAccepted,
    EgressDeniedForClassifiedMapping,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposalCrossValidationFailure {
    pub errors: Vec<ProposalCrossValidationError>,
}

/// A proposal node resolved to its exact, pinned capability contract.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedProposalNode {
    pub node_id: String,
    pub contract: CapabilityContract,
}

/// Cross-checks a structurally valid proposal against the loaded application
/// manifest and capability registry (spec 109 FR-004, FR-011):
///
/// - every node's capability must be declared in the manifest (never an
///   undeclared capability, matching the existing "declared set is the only
///   permitted set" pattern used for manifest connector bindings), and its
///   exact pinned `artifact_digest` must match the registry's record;
/// - every mapping's source/target JSON-schema fragments must be
///   structurally compatible;
/// - every mapping's field-level data classification must be explicitly
///   declared and accepted (fail closed: an undeclared classification is
///   never treated as safe), and must not flow into a capability whose
///   egress policy denies all connectors when the classification is above
///   `Public`.
///
/// # Errors
///
/// Returns [`ProposalCrossValidationFailure`] on any undeclared capability,
/// digest mismatch, incompatible mapping schema, or disallowed data flow.
#[allow(clippy::too_many_lines)]
pub fn validate_proposal_against_host_state(
    canonical: &CanonicalProposal,
    manifest: &ApplicationBundleManifest,
    registry: &CapabilityRegistry,
) -> Result<Vec<ResolvedProposalNode>, ProposalCrossValidationFailure> {
    let mut errors = Vec::new();
    let mut resolved: BTreeMap<String, ResolvedProposalNode> = BTreeMap::new();

    for (index, node) in canonical.proposal.nodes.iter().enumerate() {
        let path = format!("$.nodes[{index}]");
        if !manifest_declares_capability(manifest, node) {
            errors.push(cross_error(
                ProposalCrossValidationErrorCode::UndeclaredCapability,
                &path,
                &format!(
                    "capability '{}@{}' is not declared in the application manifest",
                    node.capability_id, node.capability_version
                ),
            ));
            continue;
        }

        let Some(capability) = registry.find_exact(
            LookupScope::PreferPrivate,
            &node.capability_id,
            &node.capability_version,
        ) else {
            errors.push(cross_error(
                ProposalCrossValidationErrorCode::CapabilityNotFound,
                &path,
                &format!(
                    "capability '{}@{}' was not found in the registry",
                    node.capability_id, node.capability_version
                ),
            ));
            continue;
        };

        let registry_digest = capability
            .artifact
            .digests
            .binary_digest
            .clone()
            .unwrap_or_else(|| capability.artifact.digests.source_digest.clone());
        if registry_digest != node.artifact_digest {
            errors.push(cross_error(
                ProposalCrossValidationErrorCode::ArtifactDigestMismatch,
                &format!("{path}.artifact_digest"),
                &format!(
                    "proposal pins digest '{}' but the registry resolves '{}@{}' to digest '{registry_digest}'",
                    node.artifact_digest, node.capability_id, node.capability_version
                ),
            ));
            continue;
        }

        resolved.insert(
            node.node_id.clone(),
            ResolvedProposalNode {
                node_id: node.node_id.clone(),
                contract: capability.contract,
            },
        );
    }

    if !errors.is_empty() {
        return Err(ProposalCrossValidationFailure { errors });
    }

    for (index, mapping) in canonical.proposal.mappings.iter().enumerate() {
        let path = format!("$.mappings[{index}]");
        let Some(target) = resolved.get(&mapping.target_node_id) else {
            continue;
        };

        let source_output_schema = match &mapping.source {
            MappingSource::InitialInput => None,
            MappingSource::Node { node_id } => {
                resolved.get(node_id).map(|n| &n.contract.outputs.schema)
            }
        };
        if let Some(source_schema) = source_output_schema
            && !mapping_schema_compatible(
                source_schema,
                &mapping.source_path,
                &target.contract.inputs.schema,
                &mapping.target_path,
            )
        {
            errors.push(cross_error(
                ProposalCrossValidationErrorCode::IncompatibleMappingSchema,
                &path,
                &format!(
                    "source path '{}' and target path '{}' declare incompatible JSON Schema types",
                    mapping.source_path, mapping.target_path
                ),
            ));
        }

        let produced_classification = match &mapping.source {
            MappingSource::InitialInput => None,
            MappingSource::Node { node_id } => resolved.get(node_id).and_then(|source| {
                classification_at_path(
                    &source.contract.risk.data_flow.produced_data_classifications,
                    &mapping.source_path,
                )
            }),
        };
        let Some(produced_classification) = produced_classification else {
            // Initial-input-sourced mappings carry no capability-declared
            // classification to check; FR-011 governs capability-to-capability
            // data flow, not caller-supplied initial input.
            continue;
        };
        let accepted_classification = classification_at_path(
            &target.contract.risk.data_flow.accepted_data_classifications,
            &mapping.target_path,
        );
        let Some(accepted_classification) = accepted_classification else {
            errors.push(cross_error(
                ProposalCrossValidationErrorCode::UndeclaredDataClassification,
                &path,
                &format!(
                    "target path '{}' on node '{}' has no declared accepted_data_classifications entry; \
                     schema compatibility alone does not authorize disclosure (spec 109 FR-011)",
                    mapping.target_path, mapping.target_node_id
                ),
            ));
            continue;
        };
        if produced_classification > accepted_classification {
            errors.push(cross_error(
                ProposalCrossValidationErrorCode::DataClassificationOverAccepted,
                &path,
                &format!(
                    "source path '{}' produces data classified above what target path '{}' on node '{}' accepts",
                    mapping.source_path, mapping.target_path, mapping.target_node_id
                ),
            ));
            continue;
        }
        let is_external_or_irreversible_effect = matches!(
            target.contract.risk.effect_class,
            EffectClass::ExternalEffect | EffectClass::IrreversibleEffect
        );
        let egress_is_denied = target.contract.risk.data_flow.egress_policy == EgressPolicy::Denied;
        if produced_classification > DataClassification::Public
            && is_external_or_irreversible_effect
            && egress_is_denied
        {
            errors.push(cross_error(
                ProposalCrossValidationErrorCode::EgressDeniedForClassifiedMapping,
                &path,
                &format!(
                    "node '{}' has an external/irreversible effect with a denied egress policy and \
                     cannot receive classified data from '{}'",
                    mapping.target_node_id, mapping.source_path
                ),
            ));
        }
    }

    if !errors.is_empty() {
        return Err(ProposalCrossValidationFailure { errors });
    }

    Ok(canonical
        .execution_order
        .iter()
        .filter_map(|node_id| resolved.get(node_id).cloned())
        .collect())
}

fn manifest_declares_capability(manifest: &ApplicationBundleManifest, node: &ProposalNode) -> bool {
    manifest.components.iter().any(|component| {
        component.manifest.capability_id == node.capability_id
            && component.manifest.capability_version == node.capability_version
    })
}

fn classification_at_path(
    classifications: &[traverse_contracts::FieldDataClassification],
    path: &str,
) -> Option<DataClassification> {
    classifications
        .iter()
        .find(|entry| entry.field_path == path)
        .map(|entry| entry.classification)
}

/// Bounded structural compatibility check between two JSON Schema fragments
/// addressed by JSON Pointer within `outputs.schema`/`inputs.schema`. Walks
/// `properties`/`items` segments to find the fragment at each path; when
/// both fragments declare a `type`, the types must match. A path that cannot
/// be resolved in either schema, or whose fragment declares no `type`, is
/// treated as compatible — schemas here document shape for humans and are
/// not held to full JSON Schema validation semantics.
fn mapping_schema_compatible(
    source_schema: &Value,
    source_path: &str,
    target_schema: &Value,
    target_path: &str,
) -> bool {
    let source_fragment = resolve_schema_pointer(source_schema, source_path);
    let target_fragment = resolve_schema_pointer(target_schema, target_path);
    match (
        source_fragment
            .and_then(|f| f.get("type"))
            .and_then(Value::as_str),
        target_fragment
            .and_then(|f| f.get("type"))
            .and_then(Value::as_str),
    ) {
        (Some(source_type), Some(target_type)) => source_type == target_type,
        _ => true,
    }
}

fn resolve_schema_pointer<'a>(schema: &'a Value, pointer: &str) -> Option<&'a Value> {
    let mut current = schema;
    for segment in pointer.split('/').filter(|s| !s.is_empty()) {
        current = current
            .get("properties")
            .and_then(|properties| properties.get(segment))
            .or_else(|| current.get("items"))?;
    }
    Some(current)
}

fn cross_error(
    code: ProposalCrossValidationErrorCode,
    path: &str,
    message: &str,
) -> ProposalCrossValidationError {
    ProposalCrossValidationError {
        code,
        message: message.to_string(),
        path: path.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Authorization: automatic-eligible vs. verified approval token (FR-006, FR-006a)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthorizationDecision {
    Automatic,
    Approved(Box<ApprovalTokenClaims>),
}

/// Whether every resolved node's declared risk classes permit automatic
/// execution (spec 109 FR-006). A proposal requires an approval token the
/// moment any single node does not.
#[must_use]
pub fn proposal_is_automatic_eligible(resolved_nodes: &[ResolvedProposalNode]) -> bool {
    resolved_nodes
        .iter()
        .all(|node| is_automatic_eligible(&node.contract.risk))
}

/// An approval token's verified claims (spec 109 FR-006a). Only produced by
/// [`verify_approval_token`] after signature, time, and binding checks pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalTokenClaims {
    pub token_id: String,
    pub issuer: String,
    pub key_id: String,
    pub audience: String,
    pub principal: String,
    pub workspace_id: String,
    pub proposal_digest: String,
    pub snapshot_digest: String,
    pub permitted_effects: Vec<EffectClass>,
    pub permitted_connectors: Vec<String>,
    pub max_use_count: u32,
    pub expiry_unix: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalTokenErrorCode {
    Malformed,
    AlgorithmNotAllowed,
    UnknownKeyId,
    SignatureVerificationFailed,
    IssuerMismatch,
    AudienceMismatch,
    Expired,
    WorkspaceMismatch,
    ProposalDigestMismatch,
    SnapshotDigestMismatch,
    UseCountExhausted,
    Revoked,
    StoreUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ApprovalTokenError {
    pub code: ApprovalTokenErrorCode,
    pub message: String,
}

/// The context an approval token is verified against: expected issuer,
/// audience, the exact proposal/snapshot digests it must be bound to, and
/// the key registry it may be signed with (spec 109 FR-006a).
pub struct ApprovalTokenVerificationContext<'a> {
    pub expected_issuer: &'a str,
    pub expected_audience: &'a str,
    pub expected_workspace_id: &'a str,
    pub expected_proposal_digest: &'a str,
    pub expected_snapshot_digest: &'a str,
    pub verifying_keys_by_key_id: &'a HashMap<String, VerifyingKey>,
}

const APPROVAL_TOKEN_ALLOWED_ALG: &str = "EdDSA";

/// Verifies an approval token's Ed25519 signature, algorithm, key identity,
/// issuer, audience, expiry, and exact binding to the proposal/snapshot
/// digest and workspace (spec 109 FR-006a). Does not check use-count or
/// revocation — see [`ApprovalTokenStore`].
///
/// # Errors
///
/// Returns [`ApprovalTokenError`] on any malformed, unverifiable, expired, or
/// mis-scoped token.
#[allow(clippy::too_many_lines)]
pub fn verify_approval_token(
    token: &str,
    context: &ApprovalTokenVerificationContext<'_>,
) -> Result<ApprovalTokenClaims, ApprovalTokenError> {
    let mut parts = token.split('.');
    let (Some(header_b64), Some(payload_b64), Some(signature_b64), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(token_error(
            ApprovalTokenErrorCode::Malformed,
            "approval token must have exactly three dot-separated segments",
        ));
    };

    let header_bytes = base64url_decode(header_b64)
        .map_err(|msg| token_error(ApprovalTokenErrorCode::Malformed, &msg))?;
    let header: Value = serde_json::from_slice(&header_bytes).map_err(|e| {
        token_error(
            ApprovalTokenErrorCode::Malformed,
            &format!("invalid header: {e}"),
        )
    })?;
    let alg = header
        .get("alg")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if alg != APPROVAL_TOKEN_ALLOWED_ALG {
        return Err(token_error(
            ApprovalTokenErrorCode::AlgorithmNotAllowed,
            &format!("alg '{alg}' is not allowed; only {APPROVAL_TOKEN_ALLOWED_ALG} is accepted"),
        ));
    }
    let key_id = header
        .get("kid")
        .and_then(Value::as_str)
        .ok_or_else(|| token_error(ApprovalTokenErrorCode::Malformed, "header missing 'kid'"))?
        .to_string();
    let verifying_key = context
        .verifying_keys_by_key_id
        .get(&key_id)
        .ok_or_else(|| {
            token_error(
                ApprovalTokenErrorCode::UnknownKeyId,
                &format!("no verification key configured for key id '{key_id}'"),
            )
        })?;

    let signature_bytes = base64url_decode(signature_b64)
        .map_err(|msg| token_error(ApprovalTokenErrorCode::SignatureVerificationFailed, &msg))?;
    let signature_array = <[u8; 64]>::try_from(signature_bytes.as_slice()).map_err(|_| {
        token_error(
            ApprovalTokenErrorCode::SignatureVerificationFailed,
            "signature must be 64 bytes",
        )
    })?;
    let signature = Signature::from_bytes(&signature_array);
    let signing_input = format!("{header_b64}.{payload_b64}");
    verifying_key
        .verify(signing_input.as_bytes(), &signature)
        .map_err(|_| {
            token_error(
                ApprovalTokenErrorCode::SignatureVerificationFailed,
                "signature verification failed",
            )
        })?;

    let payload_bytes = base64url_decode(payload_b64)
        .map_err(|msg| token_error(ApprovalTokenErrorCode::Malformed, &msg))?;
    let payload: Value = serde_json::from_slice(&payload_bytes).map_err(|e| {
        token_error(
            ApprovalTokenErrorCode::Malformed,
            &format!("invalid payload: {e}"),
        )
    })?;

    let claims = parse_approval_token_claims(&payload, &key_id)?;

    if claims.issuer != context.expected_issuer {
        return Err(token_error(
            ApprovalTokenErrorCode::IssuerMismatch,
            "token issuer does not match the expected issuer",
        ));
    }
    if claims.audience != context.expected_audience {
        return Err(token_error(
            ApprovalTokenErrorCode::AudienceMismatch,
            "token audience does not match the expected audience",
        ));
    }
    if claims.workspace_id != context.expected_workspace_id {
        return Err(token_error(
            ApprovalTokenErrorCode::WorkspaceMismatch,
            "token workspace_id does not match the proposal's workspace_id",
        ));
    }
    if claims.proposal_digest != context.expected_proposal_digest {
        return Err(token_error(
            ApprovalTokenErrorCode::ProposalDigestMismatch,
            "token is not bound to this exact proposal digest",
        ));
    }
    if claims.snapshot_digest != context.expected_snapshot_digest {
        return Err(token_error(
            ApprovalTokenErrorCode::SnapshotDigestMismatch,
            "token is not bound to the current pinned snapshot digest",
        ));
    }
    let now = unix_now();
    if claims.expiry_unix <= now {
        return Err(token_error(
            ApprovalTokenErrorCode::Expired,
            "token is expired",
        ));
    }

    Ok(claims)
}

fn parse_approval_token_claims(
    payload: &Value,
    key_id: &str,
) -> Result<ApprovalTokenClaims, ApprovalTokenError> {
    let get_str = |field: &str| -> Result<String, ApprovalTokenError> {
        payload
            .get(field)
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .map(ToString::to_string)
            .ok_or_else(|| {
                token_error(
                    ApprovalTokenErrorCode::Malformed,
                    &format!("payload missing required non-empty claim '{field}'"),
                )
            })
    };
    let permitted_effects = payload
        .get("permitted_effects")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .filter_map(parse_effect_class)
                .collect()
        })
        .unwrap_or_default();
    let permitted_connectors = payload
        .get("permitted_connectors")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default();
    let max_use_count = payload
        .get("max_use_count")
        .and_then(Value::as_u64)
        .and_then(|v| u32::try_from(v).ok())
        .ok_or_else(|| {
            token_error(
                ApprovalTokenErrorCode::Malformed,
                "payload missing required u32 claim 'max_use_count'",
            )
        })?;
    let expiry_unix = payload.get("exp").and_then(Value::as_i64).ok_or_else(|| {
        token_error(
            ApprovalTokenErrorCode::Malformed,
            "payload missing required claim 'exp'",
        )
    })?;

    Ok(ApprovalTokenClaims {
        token_id: get_str("jti")?,
        issuer: get_str("iss")?,
        key_id: key_id.to_string(),
        audience: get_str("aud")?,
        principal: get_str("sub")?,
        workspace_id: get_str("workspace_id")?,
        proposal_digest: get_str("proposal_digest")?,
        snapshot_digest: get_str("snapshot_digest")?,
        permitted_effects,
        permitted_connectors,
        max_use_count,
        expiry_unix,
    })
}

fn parse_effect_class(value: &str) -> Option<EffectClass> {
    match value {
        "pure_read" => Some(EffectClass::PureRead),
        "state_write" => Some(EffectClass::StateWrite),
        "external_effect" => Some(EffectClass::ExternalEffect),
        "irreversible_effect" => Some(EffectClass::IrreversibleEffect),
        _ => None,
    }
}

fn token_error(code: ApprovalTokenErrorCode, message: &str) -> ApprovalTokenError {
    ApprovalTokenError {
        code,
        message: message.to_string(),
    }
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

fn base64url_decode(input: &str) -> Result<Vec<u8>, String> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut lookup = [255u8; 256];
    for (index, byte) in ALPHABET.iter().enumerate() {
        lookup[*byte as usize] = u8::try_from(index).unwrap_or(0);
    }

    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4 + 3);
    let mut buffer: u32 = 0;
    let mut bits: u32 = 0;
    for &byte in bytes {
        let value = lookup[byte as usize];
        if value == 255 {
            return Err("invalid base64url character".to_string());
        }
        buffer = (buffer << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(u8::try_from((buffer >> bits) & 0xFF).unwrap_or(0));
        }
    }
    Ok(out)
}

/// Tracks approval-token use-count and revocation across calls (spec 109
/// FR-006a: max use count, revocation, replay prevention). In-memory only —
/// tokens do not survive a process restart, which is safe since they are
/// short-lived and bound to a specific pinned snapshot.
pub struct ApprovalTokenStore {
    used: Mutex<HashMap<String, TokenUsageRecord>>,
}

#[derive(Debug, Clone, Copy, Default)]
struct TokenUsageRecord {
    use_count: u32,
    revoked: bool,
}

impl Default for ApprovalTokenStore {
    fn default() -> Self {
        Self::new()
    }
}

impl ApprovalTokenStore {
    #[must_use]
    pub fn new() -> Self {
        Self {
            used: Mutex::new(HashMap::new()),
        }
    }

    /// Atomically checks revocation and use-count, then records one use.
    ///
    /// # Errors
    ///
    /// Returns [`ApprovalTokenError`] with [`ApprovalTokenErrorCode::Revoked`]
    /// or [`ApprovalTokenErrorCode::UseCountExhausted`] when the token cannot
    /// be used again, or [`ApprovalTokenErrorCode::StoreUnavailable`] (fail
    /// closed) if the internal store is poisoned by a prior panicking holder.
    pub fn check_and_record_use(
        &self,
        claims: &ApprovalTokenClaims,
    ) -> Result<(), ApprovalTokenError> {
        let mut used = self.used.lock().map_err(|_| {
            token_error(
                ApprovalTokenErrorCode::StoreUnavailable,
                "approval token store is unavailable; failing closed",
            )
        })?;
        let record = used.entry(claims.token_id.clone()).or_default();
        if record.revoked {
            return Err(token_error(
                ApprovalTokenErrorCode::Revoked,
                "token has been revoked",
            ));
        }
        if record.use_count >= claims.max_use_count {
            return Err(token_error(
                ApprovalTokenErrorCode::UseCountExhausted,
                "token has reached its maximum use count",
            ));
        }
        record.use_count += 1;
        Ok(())
    }

    /// Marks a token id as permanently unusable, regardless of remaining
    /// uses (spec 109 FR-006a revocation). A poisoned store is a no-op —
    /// there is nothing further to record, and `check_and_record_use` fails
    /// closed independently.
    pub fn revoke(&self, token_id: &str) {
        if let Ok(mut used) = self.used.lock() {
            used.entry(token_id.to_string()).or_default().revoked = true;
        }
    }
}

// ---------------------------------------------------------------------------
// Per-principal/app/workspace quota tracking (spec 109 FR-007b)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuotaLimits {
    pub max_concurrent_per_principal: u32,
    pub max_concurrent_per_app: u32,
    pub max_concurrent_per_workspace: u32,
}

pub const DEFAULT_MAX_CONCURRENT_PER_PRINCIPAL: u32 = 4;
pub const DEFAULT_MAX_CONCURRENT_PER_APP: u32 = 16;
pub const DEFAULT_MAX_CONCURRENT_PER_WORKSPACE: u32 = 32;

impl Default for QuotaLimits {
    fn default() -> Self {
        Self {
            max_concurrent_per_principal: DEFAULT_MAX_CONCURRENT_PER_PRINCIPAL,
            max_concurrent_per_app: DEFAULT_MAX_CONCURRENT_PER_APP,
            max_concurrent_per_workspace: DEFAULT_MAX_CONCURRENT_PER_WORKSPACE,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuotaDenial {
    pub scope: &'static str,
    pub message: String,
}

/// A live concurrency reservation. Releases automatically on drop so a
/// caller can never forget to release it, even on an early error return.
#[derive(Debug)]
pub struct QuotaReservation<'a> {
    tracker: &'a QuotaTracker,
    principal: String,
    app_id: String,
    workspace_id: String,
}

impl Drop for QuotaReservation<'_> {
    fn drop(&mut self) {
        self.tracker
            .release(&self.principal, &self.app_id, &self.workspace_id);
    }
}

/// In-memory concurrency quota ledger keyed by principal, app, and workspace
/// (spec 109 FR-007b). Each dimension is tracked and enforced independently.
#[derive(Debug, Default)]
#[allow(clippy::struct_field_names)]
pub struct QuotaTracker {
    principal_slots: Mutex<HashMap<String, u32>>,
    app_slots: Mutex<HashMap<String, u32>>,
    workspace_slots: Mutex<HashMap<String, u32>>,
}

impl QuotaTracker {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Reserves one concurrent execution slot across all three dimensions,
    /// or denies and reserves nothing if any dimension is already at its
    /// limit.
    ///
    /// # Errors
    ///
    /// Returns [`QuotaDenial`] naming the first exhausted dimension, or a
    /// `"store"`-scoped denial (fail closed) if an internal lock is poisoned
    /// by a prior panicking holder.
    pub fn reserve(
        &self,
        principal: &str,
        app_id: &str,
        workspace_id: &str,
        limits: &QuotaLimits,
    ) -> Result<QuotaReservation<'_>, QuotaDenial> {
        reserve_dimension(
            &self.principal_slots,
            principal,
            limits.max_concurrent_per_principal,
            "principal",
        )?;
        if let Err(denial) = reserve_dimension(
            &self.app_slots,
            app_id,
            limits.max_concurrent_per_app,
            "app",
        ) {
            release_dimension(&self.principal_slots, principal);
            return Err(denial);
        }
        if let Err(denial) = reserve_dimension(
            &self.workspace_slots,
            workspace_id,
            limits.max_concurrent_per_workspace,
            "workspace",
        ) {
            release_dimension(&self.principal_slots, principal);
            release_dimension(&self.app_slots, app_id);
            return Err(denial);
        }

        Ok(QuotaReservation {
            tracker: self,
            principal: principal.to_string(),
            app_id: app_id.to_string(),
            workspace_id: workspace_id.to_string(),
        })
    }

    fn release(&self, principal: &str, app_id: &str, workspace_id: &str) {
        release_dimension(&self.principal_slots, principal);
        release_dimension(&self.app_slots, app_id);
        release_dimension(&self.workspace_slots, workspace_id);
    }
}

fn reserve_dimension(
    counts: &Mutex<HashMap<String, u32>>,
    key: &str,
    limit: u32,
    scope: &'static str,
) -> Result<(), QuotaDenial> {
    let mut counts = counts.lock().map_err(|_| QuotaDenial {
        scope: "store",
        message: "quota tracker is unavailable; failing closed".to_string(),
    })?;
    let count = counts.entry(key.to_string()).or_insert(0);
    if *count >= limit {
        return Err(QuotaDenial {
            scope,
            message: format!("{scope} '{key}' is at its concurrency limit of {limit}"),
        });
    }
    *count += 1;
    Ok(())
}

fn release_dimension(counts: &Mutex<HashMap<String, u32>>, key: &str) {
    if let Ok(mut counts) = counts.lock()
        && let Some(count) = counts.get_mut(key)
    {
        *count = count.saturating_sub(1);
    }
}

// ---------------------------------------------------------------------------
// Sequential execution and redacted trace (spec 109 FR-007, FR-008, FR-008a, FR-009)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalNodeStatus {
    Succeeded,
    Failed,
    SkippedAfterEarlierFailure,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProposalNodeOutcome {
    pub node_id: String,
    pub capability_id: String,
    pub capability_version: String,
    pub artifact_digest: String,
    pub status: ProposalNodeStatus,
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalTerminalState {
    Succeeded,
    Failed,
    Cancelled,
    Expired,
    AuthorizationRevoked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AuthorizationSummary {
    pub automatic: bool,
    pub approval_token_id: Option<String>,
}

/// A bounded, redacted, immutable projection of one proposal execution (spec
/// 109 FR-009). Carries mapping *paths*, never mapped values; a token id,
/// never the raw token; and no raw node input/output payloads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProposalTrace {
    pub proposal_id: String,
    pub proposal_digest: String,
    pub snapshot_digest: String,
    pub authorization: AuthorizationSummary,
    pub node_outcomes: Vec<ProposalNodeOutcome>,
    pub mapping_paths: Vec<(String, String)>,
    pub terminal_state: ProposalTerminalState,
}

/// Executes a structurally and cross-validated proposal one node at a time in
/// its deterministic execution order, threading data between nodes solely
/// through the proposal's explicit mappings. Stops at the first failed node
/// with no retry, compensation, or graph mutation (spec 109 FR-008).
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn execute_proposal<E: crate::LocalExecutor>(
    runtime: &Runtime<E>,
    canonical: &CanonicalProposal,
    resolved_nodes: &[ResolvedProposalNode],
    authorization: AuthorizationSummary,
    proposal_digest: &str,
    snapshot_digest: &str,
) -> ProposalTrace {
    let contracts_by_node: HashMap<&str, &CapabilityContract> = resolved_nodes
        .iter()
        .map(|n| (n.node_id.as_str(), &n.contract))
        .collect();
    let nodes_by_id: HashMap<&str, &ProposalNode> = canonical
        .proposal
        .nodes
        .iter()
        .map(|n| (n.node_id.as_str(), n))
        .collect();

    let mut outputs: HashMap<String, Value> = HashMap::new();
    let mut outcomes = Vec::with_capacity(canonical.execution_order.len());
    let mut failed = false;

    for node_id in &canonical.execution_order {
        let Some(node) = nodes_by_id.get(node_id.as_str()) else {
            continue;
        };
        if failed {
            outcomes.push(ProposalNodeOutcome {
                node_id: node_id.clone(),
                capability_id: node.capability_id.clone(),
                capability_version: node.capability_version.clone(),
                artifact_digest: node.artifact_digest.clone(),
                status: ProposalNodeStatus::SkippedAfterEarlierFailure,
                error_code: None,
            });
            continue;
        }

        let input = assemble_node_input(canonical, node_id, &outputs);
        let request = build_node_execution_request(canonical, node, node_id, input);
        let outcome = runtime.execute(request);
        match outcome.result.status {
            RuntimeResultStatus::Completed => {
                if let Some(output) = outcome.result.output.clone() {
                    outputs.insert(node_id.clone(), output);
                }
                outcomes.push(ProposalNodeOutcome {
                    node_id: node_id.clone(),
                    capability_id: node.capability_id.clone(),
                    capability_version: node.capability_version.clone(),
                    artifact_digest: node.artifact_digest.clone(),
                    status: ProposalNodeStatus::Succeeded,
                    error_code: None,
                });
            }
            RuntimeResultStatus::Error => {
                failed = true;
                outcomes.push(ProposalNodeOutcome {
                    node_id: node_id.clone(),
                    capability_id: node.capability_id.clone(),
                    capability_version: node.capability_version.clone(),
                    artifact_digest: node.artifact_digest.clone(),
                    status: ProposalNodeStatus::Failed,
                    error_code: outcome
                        .result
                        .error
                        .as_ref()
                        .map(|error| format!("{:?}", error.code)),
                });
            }
        }
        let _ = contracts_by_node.get(node_id.as_str());
    }

    let terminal_state = if failed {
        ProposalTerminalState::Failed
    } else {
        ProposalTerminalState::Succeeded
    };

    ProposalTrace {
        proposal_id: canonical.proposal.proposal_id.clone(),
        proposal_digest: proposal_digest.to_string(),
        snapshot_digest: snapshot_digest.to_string(),
        authorization,
        node_outcomes: outcomes,
        mapping_paths: canonical
            .proposal
            .mappings
            .iter()
            .map(|m| (m.source_path.clone(), m.target_path.clone()))
            .collect(),
        terminal_state,
    }
}

/// Resolves every mapping targeting `node_id` against `outputs` (and the
/// proposal's `initial_input`) into that node's assembled input object.
/// Shared by the sequential P1 executor and the P2 wave executor.
pub(crate) fn assemble_node_input(
    canonical: &CanonicalProposal,
    node_id: &str,
    outputs: &HashMap<String, Value>,
) -> Value {
    let mut input = Value::Object(serde_json::Map::new());
    for mapping in &canonical.proposal.mappings {
        if mapping.target_node_id != *node_id {
            continue;
        }
        let value = match &mapping.source {
            MappingSource::InitialInput => {
                pointer_get(&canonical.proposal.initial_input, &mapping.source_path)
            }
            MappingSource::Node { node_id: source_id } => outputs
                .get(source_id)
                .and_then(|output| pointer_get(output, &mapping.source_path)),
        };
        if let Some(value) = value {
            pointer_set(&mut input, &mapping.target_path, value.clone());
        }
    }
    input
}

/// Builds the `006-runtime-request-execution` request for one proposal node.
/// Shared by the sequential P1 executor and the P2 wave executor.
pub(crate) fn build_node_execution_request(
    canonical: &CanonicalProposal,
    node: &ProposalNode,
    node_id: &str,
    input: Value,
) -> RuntimeRequest {
    RuntimeRequest {
        kind: "runtime_request".to_string(),
        schema_version: "1.0.0".to_string(),
        request_id: format!("{}-{node_id}", canonical.proposal.proposal_id),
        intent: RuntimeIntent {
            capability_id: Some(node.capability_id.clone()),
            capability_version: Some(node.capability_version.clone()),
            version_range: None,
            intent_key: None,
        },
        input,
        lookup: RuntimeLookup {
            scope: RuntimeLookupScope::PreferPrivate,
            allow_ambiguity: false,
        },
        context: RuntimeContext {
            requested_target: PlacementTarget::Local,
            correlation_id: Some(canonical.proposal.proposal_id.clone()),
            caller: Some("workflow_proposal".to_string()),
            traceparent: None,
            tracestate: None,
            metadata: None,
            identity: None,
        },
        governing_spec: "006-runtime-request-execution".to_string(),
    }
}

pub(crate) fn pointer_get<'a>(value: &'a Value, pointer: &str) -> Option<&'a Value> {
    value.pointer(pointer)
}

pub(crate) fn pointer_set(target: &mut Value, pointer: &str, new_value: Value) {
    let segments: Vec<&str> = pointer.split('/').filter(|s| !s.is_empty()).collect();
    *target = set_at_segments(std::mem::take(target), &segments, new_value);
}

/// Recursively rebuilds `current` with `new_value` inserted at `segments`,
/// creating intermediate objects as needed and overwriting any non-object
/// value found along the path. Every branch here is a real semantic case
/// (root replacement, descend-into-existing-object, or create-fresh-object)
/// rather than a defensive fallback, so there is no unreachable arm to guard.
fn set_at_segments(current: Value, segments: &[&str], new_value: Value) -> Value {
    let Some((head, rest)) = segments.split_first() else {
        return new_value;
    };
    let mut map = match current {
        Value::Object(map) => map,
        _ => serde_json::Map::new(),
    };
    let child = map.remove(*head).unwrap_or(Value::Null);
    map.insert((*head).to_string(), set_at_segments(child, rest, new_value));
    Value::Object(map)
}

#[cfg(test)]
#[allow(clippy::expect_used)]
#[allow(clippy::panic)]
mod tests {
    use super::*;
    use crate::security::RuntimeSecurityConfig;
    use crate::{
        LocalExecutionFailure, LocalExecutionFailureCode, LocalExecutionOutput, LocalExecutor,
        Runtime,
    };
    use ed25519_dalek::{Signer, SigningKey};
    use serde_json::json;
    use traverse_contracts::{
        BinaryFormat as ContractBinaryFormat, CanonicalProposal, CapabilityContract,
        DataClassification, DataFlowPolicy, DeterminismClass, EffectClass, EgressPolicy,
        Entrypoint, EntrypointKind, Execution, ExecutionConstraints, ExecutionTarget,
        FieldDataClassification, FilesystemAccess, HostApiAccess, Lifecycle, ManifestReference,
        MappingSource, NetworkAccess, Owner, ProposalEdge, ProposalLimits, ProposalMapping,
        ProposalNode, Provenance, ProvenanceSource, ReliabilityMetadata, RiskMetadata,
        SchemaContainer, ServiceType, SideEffect, SideEffectKind, WorkflowProposal,
        canonicalize_proposal, proposal_digest,
    };
    use traverse_registry::{
        ApplicationBundleManifest, ApplicationComponent, ApplicationComponentRef,
        ApplicationEffectiveConfig, ArtifactDigests, BinaryFormat as RegistryBinaryFormat,
        BinaryReference, CapabilityArtifactRecord, CapabilityRegistration, CapabilityRegistry,
        ComponentExecutionMode, ComposabilityMetadata, CompositionKind, CompositionPattern,
        ImplementationKind, RegistryProvenance, RegistryScope, SourceKind, SourceReference,
        WasmComponentManifest,
    };

    fn automatic_risk() -> RiskMetadata {
        RiskMetadata {
            effect_class: EffectClass::PureRead,
            determinism_class: DeterminismClass::Deterministic,
            data_flow: DataFlowPolicy::default(),
            reliability: ReliabilityMetadata {
                idempotency_required: false,
                retryable: true,
                compensation_available: false,
            },
        }
    }

    fn contract(
        id: &str,
        version: &str,
        outputs_schema: Value,
        inputs_schema: Value,
        risk: RiskMetadata,
    ) -> CapabilityContract {
        let (namespace, name) = id.rsplit_once('.').unwrap_or(("test", id));
        CapabilityContract {
            kind: "capability_contract".to_string(),
            schema_version: "1.0.0".to_string(),
            id: id.to_string(),
            namespace: namespace.to_string(),
            name: name.to_string(),
            version: version.to_string(),
            lifecycle: Lifecycle::Active,
            owner: Owner {
                team: "traverse-core".to_string(),
                contact: "enrico.piovesan10@gmail.com".to_string(),
            },
            summary: "Test capability for proposal lifecycle validation.".to_string(),
            description: "Portable test capability used to validate proposal cross-checks."
                .to_string(),
            inputs: SchemaContainer {
                schema: inputs_schema,
            },
            outputs: SchemaContainer {
                schema: outputs_schema,
            },
            preconditions: Vec::new(),
            postconditions: Vec::new(),
            side_effects: vec![SideEffect {
                kind: SideEffectKind::MemoryOnly,
                description: "No durable side effect.".to_string(),
            }],
            emits: Vec::new(),
            consumes: Vec::new(),
            permissions: Vec::new(),
            execution: Execution {
                binary_format: ContractBinaryFormat::Wasm,
                entrypoint: Entrypoint {
                    kind: EntrypointKind::WasiCommand,
                    command: "run".to_string(),
                },
                preferred_targets: vec![ExecutionTarget::Local],
                constraints: ExecutionConstraints {
                    host_api_access: HostApiAccess::None,
                    network_access: NetworkAccess::Forbidden,
                    filesystem_access: FilesystemAccess::None,
                },
                resources: None,
            },
            policies: Vec::new(),
            dependencies: Vec::new(),
            provenance: Provenance {
                source: ProvenanceSource::Greenfield,
                author: "test".to_string(),
                created_at: "2026-08-23T00:00:00Z".to_string(),
                spec_ref: None,
                adr_refs: Vec::new(),
                exception_refs: Vec::new(),
            },
            evidence: Vec::new(),
            service_type: ServiceType::Stateless,
            permitted_targets: vec![ExecutionTarget::Local],
            event_trigger: None,
            connector_requirements: Vec::new(),
            state_schema: None,
            use_cases: Vec::new(),
            risk,
        }
    }

    fn artifact(digest: &str) -> CapabilityArtifactRecord {
        CapabilityArtifactRecord {
            artifact_ref: format!("artifact:{digest}"),
            implementation_kind: ImplementationKind::Executable,
            source: SourceReference {
                kind: SourceKind::Git,
                location: "https://example.invalid/repo".to_string(),
            },
            binary: Some(BinaryReference {
                format: RegistryBinaryFormat::Wasm,
                location: format!("artifacts/{digest}/capability.wasm"),
                signature: None,
            }),
            workflow_ref: None,
            digests: ArtifactDigests {
                source_digest: format!("src-{digest}"),
                binary_digest: Some(digest.to_string()),
            },
            provenance: RegistryProvenance {
                source: "test".to_string(),
                author: "test".to_string(),
                created_at: "2026-08-23T00:00:00Z".to_string(),
            },
        }
    }

    fn registry_with(
        entries: Vec<(CapabilityContract, CapabilityArtifactRecord)>,
    ) -> CapabilityRegistry {
        let mut registry = CapabilityRegistry::new();
        for (contract, artifact) in entries {
            let outcome = registry.register(CapabilityRegistration {
                scope: RegistryScope::Public,
                contract,
                contract_path: "registry/test/contract.json".to_string(),
                artifact,
                registered_at: "2026-08-23T00:00:00Z".to_string(),
                tags: Vec::new(),
                composability: ComposabilityMetadata {
                    kind: CompositionKind::Atomic,
                    patterns: vec![CompositionPattern::Sequential],
                    provides: Vec::new(),
                    requires: Vec::new(),
                },
                governing_spec: "005-capability-registry".to_string(),
                validator_version: "0.1.0".to_string(),
            });
            assert!(outcome.is_ok(), "registration must succeed: {outcome:?}");
        }
        registry
    }

    fn manifest_declaring(components: &[(&str, &str)]) -> ApplicationBundleManifest {
        ApplicationBundleManifest {
            app_id: "test-app".to_string(),
            version: "1.0.0".to_string(),
            schema_version: "1.0.0".to_string(),
            workspace_defaults: json!({}),
            components: components
                .iter()
                .map(|(capability_id, capability_version)| ApplicationComponent {
                    reference: ApplicationComponentRef {
                        component_id: capability_id.to_string(),
                        version: (*capability_version).to_string(),
                        digest: "sha256:component-digest".to_string(),
                        manifest_path: "component.manifest.json".to_string(),
                    },
                    manifest_path: "component.manifest.json".into(),
                    manifest: WasmComponentManifest {
                        component_id: capability_id.to_string(),
                        version: (*capability_version).to_string(),
                        schema_version: "1.0.0".to_string(),
                        execution_mode: ComponentExecutionMode::Wasm,
                        capability_id: (*capability_id).to_string(),
                        capability_version: (*capability_version).to_string(),
                        contract_path: None,
                        registry_ref: None,
                        wasm_binary_path: None,
                        wasm_digest: None,
                        platforms: vec!["local".to_string()],
                        wrapper_path: None,
                        runtime_constraints: json!({}),
                        permitted_targets: vec![ExecutionTarget::Local],
                        dependencies: Vec::new(),
                        connector_requirements: Vec::new(),
                        validation_evidence: Vec::new(),
                        executable_pin: None,
                    },
                    contract_path: "contract.json".into(),
                    contract: contract(
                        capability_id,
                        capability_version,
                        json!({"type": "object"}),
                        json!({"type": "object"}),
                        automatic_risk(),
                    ),
                    wasm_binary_path: None,
                    verified_wasm_digest: None,
                })
                .collect(),
            workflows: Vec::new(),
            connector_bindings: Vec::new(),
            model_dependencies: Vec::new(),
            config_schema: json!({}),
            default_config: json!({}),
            effective_config: ApplicationEffectiveConfig {
                values: json!({}),
                redacted_secret_keys: Vec::new(),
            },
            placement_policy: json!({}),
            public_surfaces: Vec::new(),
            state_machine: None,
        }
    }

    fn linear_proposal_source() -> WorkflowProposal {
        WorkflowProposal {
            kind: "workflow_proposal".to_string(),
            schema_version: "1.0.0".to_string(),
            proposal_id: "proposal-001".to_string(),
            workspace_id: "workspace-001".to_string(),
            app_manifest: ManifestReference {
                app_id: "test-app".to_string(),
                app_version: "1.0.0".to_string(),
                manifest_digest: "sha256:manifest-digest".to_string(),
            },
            nodes: vec![
                ProposalNode {
                    node_id: "a".to_string(),
                    capability_id: "test.produce".to_string(),
                    capability_version: "1.0.0".to_string(),
                    artifact_digest: "digest-a".to_string(),
                },
                ProposalNode {
                    node_id: "b".to_string(),
                    capability_id: "test.consume".to_string(),
                    capability_version: "1.0.0".to_string(),
                    artifact_digest: "digest-b".to_string(),
                },
            ],
            edges: vec![ProposalEdge {
                from_node_id: "a".to_string(),
                to_node_id: "b".to_string(),
            }],
            mappings: vec![ProposalMapping {
                source: MappingSource::Node {
                    node_id: "a".to_string(),
                },
                source_path: "/value".to_string(),
                target_node_id: "b".to_string(),
                target_path: "/value".to_string(),
            }],
            initial_input: json!({}),
        }
    }

    fn linear_canonical() -> CanonicalProposal {
        canonicalize_proposal(linear_proposal_source(), &ProposalLimits::default())
            .expect("linear proposal must canonicalize")
    }

    fn initial_input_proposal_source() -> WorkflowProposal {
        WorkflowProposal {
            kind: "workflow_proposal".to_string(),
            schema_version: "1.0.0".to_string(),
            proposal_id: "proposal-002".to_string(),
            workspace_id: "workspace-001".to_string(),
            app_manifest: ManifestReference {
                app_id: "test-app".to_string(),
                app_version: "1.0.0".to_string(),
                manifest_digest: "sha256:manifest-digest".to_string(),
            },
            nodes: vec![ProposalNode {
                node_id: "x".to_string(),
                capability_id: "test.consume".to_string(),
                capability_version: "1.0.0".to_string(),
                artifact_digest: "digest-x".to_string(),
            }],
            edges: Vec::new(),
            mappings: vec![ProposalMapping {
                source: MappingSource::InitialInput,
                source_path: "/value".to_string(),
                target_node_id: "x".to_string(),
                target_path: "/nested/value".to_string(),
            }],
            initial_input: json!({"value": "from-caller"}),
        }
    }

    fn initial_input_canonical() -> CanonicalProposal {
        canonicalize_proposal(initial_input_proposal_source(), &ProposalLimits::default())
            .expect("initial-input proposal must canonicalize")
    }

    // -- Cross-validation ---------------------------------------------------

    #[test]
    fn validates_a_well_formed_proposal_against_manifest_and_registry() {
        let manifest = manifest_declaring(&[("test.produce", "1.0.0"), ("test.consume", "1.0.0")]);
        let registry = registry_with(vec![
            (
                contract(
                    "test.produce",
                    "1.0.0",
                    json!({"type": "object"}),
                    json!({"type": "object"}),
                    automatic_risk(),
                ),
                artifact("digest-a"),
            ),
            (
                contract(
                    "test.consume",
                    "1.0.0",
                    json!({"type": "object"}),
                    json!({"type": "object"}),
                    automatic_risk(),
                ),
                artifact("digest-b"),
            ),
        ]);

        let resolved =
            validate_proposal_against_host_state(&linear_canonical(), &manifest, &registry)
                .expect("well-formed proposal must validate");
        assert_eq!(resolved.len(), 2);
    }

    #[test]
    fn rejects_a_capability_not_declared_in_the_manifest() {
        let manifest = manifest_declaring(&[("test.produce", "1.0.0")]); // missing test.consume
        let registry = registry_with(vec![
            (
                contract(
                    "test.produce",
                    "1.0.0",
                    json!({"type": "object"}),
                    json!({"type": "object"}),
                    automatic_risk(),
                ),
                artifact("digest-a"),
            ),
            (
                contract(
                    "test.consume",
                    "1.0.0",
                    json!({"type": "object"}),
                    json!({"type": "object"}),
                    automatic_risk(),
                ),
                artifact("digest-b"),
            ),
        ]);

        let failure =
            validate_proposal_against_host_state(&linear_canonical(), &manifest, &registry)
                .expect_err("undeclared capability must be rejected");
        assert!(
            failure
                .errors
                .iter()
                .any(|e| e.code == ProposalCrossValidationErrorCode::UndeclaredCapability)
        );
    }

    #[test]
    fn rejects_a_capability_not_found_in_the_registry() {
        let manifest = manifest_declaring(&[("test.produce", "1.0.0"), ("test.consume", "1.0.0")]);
        let registry = registry_with(vec![(
            contract(
                "test.produce",
                "1.0.0",
                json!({"type": "object"}),
                json!({"type": "object"}),
                automatic_risk(),
            ),
            artifact("digest-a"),
        )]); // test.consume never registered

        let failure =
            validate_proposal_against_host_state(&linear_canonical(), &manifest, &registry)
                .expect_err("unregistered capability must be rejected");
        assert!(
            failure
                .errors
                .iter()
                .any(|e| e.code == ProposalCrossValidationErrorCode::CapabilityNotFound)
        );
    }

    #[test]
    fn rejects_an_artifact_digest_mismatch() {
        let manifest = manifest_declaring(&[("test.produce", "1.0.0"), ("test.consume", "1.0.0")]);
        let registry = registry_with(vec![
            (
                contract(
                    "test.produce",
                    "1.0.0",
                    json!({"type": "object"}),
                    json!({"type": "object"}),
                    automatic_risk(),
                ),
                artifact("wrong-digest"),
            ),
            (
                contract(
                    "test.consume",
                    "1.0.0",
                    json!({"type": "object"}),
                    json!({"type": "object"}),
                    automatic_risk(),
                ),
                artifact("digest-b"),
            ),
        ]);

        let failure =
            validate_proposal_against_host_state(&linear_canonical(), &manifest, &registry)
                .expect_err("digest mismatch must be rejected");
        assert!(
            failure
                .errors
                .iter()
                .any(|e| e.code == ProposalCrossValidationErrorCode::ArtifactDigestMismatch)
        );
    }

    #[test]
    fn rejects_incompatible_mapping_schema_types() {
        let manifest = manifest_declaring(&[("test.produce", "1.0.0"), ("test.consume", "1.0.0")]);
        let registry = registry_with(vec![
            (
                contract(
                    "test.produce",
                    "1.0.0",
                    json!({"type": "object", "properties": {"value": {"type": "string"}}}),
                    json!({"type": "object"}),
                    automatic_risk(),
                ),
                artifact("digest-a"),
            ),
            (
                contract(
                    "test.consume",
                    "1.0.0",
                    json!({"type": "object"}),
                    json!({"type": "object", "properties": {"value": {"type": "integer"}}}),
                    automatic_risk(),
                ),
                artifact("digest-b"),
            ),
        ]);

        let failure =
            validate_proposal_against_host_state(&linear_canonical(), &manifest, &registry)
                .expect_err("incompatible mapping schema types must be rejected");
        assert!(
            failure
                .errors
                .iter()
                .any(|e| e.code == ProposalCrossValidationErrorCode::IncompatibleMappingSchema)
        );
    }

    fn classification_risk(
        produced: &[(&str, DataClassification)],
        accepted: &[(&str, DataClassification)],
        egress_policy: EgressPolicy,
        effect_class: EffectClass,
    ) -> RiskMetadata {
        RiskMetadata {
            effect_class,
            determinism_class: DeterminismClass::Deterministic,
            data_flow: DataFlowPolicy {
                accepted_data_classifications: accepted
                    .iter()
                    .map(|(path, classification)| FieldDataClassification {
                        field_path: (*path).to_string(),
                        classification: *classification,
                    })
                    .collect(),
                produced_data_classifications: produced
                    .iter()
                    .map(|(path, classification)| FieldDataClassification {
                        field_path: (*path).to_string(),
                        classification: *classification,
                    })
                    .collect(),
                egress_policy,
            },
            reliability: ReliabilityMetadata {
                idempotency_required: false,
                retryable: true,
                compensation_available: false,
            },
        }
    }

    #[test]
    fn rejects_a_mapping_target_with_no_declared_accepted_classification() {
        let manifest = manifest_declaring(&[("test.produce", "1.0.0"), ("test.consume", "1.0.0")]);
        let registry = registry_with(vec![
            (
                contract(
                    "test.produce",
                    "1.0.0",
                    json!({"type": "object"}),
                    json!({"type": "object"}),
                    classification_risk(
                        &[("/value", DataClassification::Public)],
                        &[],
                        EgressPolicy::Denied,
                        EffectClass::PureRead,
                    ),
                ),
                artifact("digest-a"),
            ),
            (
                contract(
                    "test.consume",
                    "1.0.0",
                    json!({"type": "object"}),
                    json!({"type": "object"}),
                    automatic_risk(), // no accepted_data_classifications declared
                ),
                artifact("digest-b"),
            ),
        ]);

        let failure =
            validate_proposal_against_host_state(&linear_canonical(), &manifest, &registry)
                .expect_err("undeclared target classification must be rejected");
        assert!(
            failure
                .errors
                .iter()
                .any(|e| e.code == ProposalCrossValidationErrorCode::UndeclaredDataClassification)
        );
    }

    #[test]
    fn rejects_a_mapping_whose_produced_classification_exceeds_accepted() {
        let manifest = manifest_declaring(&[("test.produce", "1.0.0"), ("test.consume", "1.0.0")]);
        let registry = registry_with(vec![
            (
                contract(
                    "test.produce",
                    "1.0.0",
                    json!({"type": "object"}),
                    json!({"type": "object"}),
                    classification_risk(
                        &[("/value", DataClassification::Confidential)],
                        &[],
                        EgressPolicy::Denied,
                        EffectClass::PureRead,
                    ),
                ),
                artifact("digest-a"),
            ),
            (
                contract(
                    "test.consume",
                    "1.0.0",
                    json!({"type": "object"}),
                    json!({"type": "object"}),
                    classification_risk(
                        &[],
                        &[("/value", DataClassification::Public)],
                        EgressPolicy::Denied,
                        EffectClass::PureRead,
                    ),
                ),
                artifact("digest-b"),
            ),
        ]);

        let failure =
            validate_proposal_against_host_state(&linear_canonical(), &manifest, &registry)
                .expect_err("over-classified mapping must be rejected");
        assert!(
            failure
                .errors
                .iter()
                .any(|e| e.code == ProposalCrossValidationErrorCode::DataClassificationOverAccepted)
        );
    }

    #[test]
    fn rejects_classified_data_into_an_egress_denied_external_effect_node() {
        let manifest = manifest_declaring(&[("test.produce", "1.0.0"), ("test.consume", "1.0.0")]);
        let registry = registry_with(vec![
            (
                contract(
                    "test.produce",
                    "1.0.0",
                    json!({"type": "object"}),
                    json!({"type": "object"}),
                    classification_risk(
                        &[("/value", DataClassification::Internal)],
                        &[],
                        EgressPolicy::Denied,
                        EffectClass::PureRead,
                    ),
                ),
                artifact("digest-a"),
            ),
            (
                contract(
                    "test.consume",
                    "1.0.0",
                    json!({"type": "object"}),
                    json!({"type": "object"}),
                    classification_risk(
                        &[],
                        &[("/value", DataClassification::Internal)],
                        EgressPolicy::Denied,
                        EffectClass::ExternalEffect,
                    ),
                ),
                artifact("digest-b"),
            ),
        ]);

        let failure =
            validate_proposal_against_host_state(&linear_canonical(), &manifest, &registry)
                .expect_err(
                    "classified data into an egress-denied external-effect node must be rejected",
                );
        assert!(
            failure
                .errors
                .iter()
                .any(|e| e.code
                    == ProposalCrossValidationErrorCode::EgressDeniedForClassifiedMapping)
        );
    }

    #[test]
    fn accepts_a_mapping_sourced_from_initial_input_with_no_classification_check() {
        let manifest = manifest_declaring(&[("test.consume", "1.0.0")]);
        let registry = registry_with(vec![(
            contract(
                "test.consume",
                "1.0.0",
                json!({"type": "object"}),
                json!({"type": "object"}),
                automatic_risk(),
            ),
            artifact("digest-x"),
        )]);

        let resolved =
            validate_proposal_against_host_state(&initial_input_canonical(), &manifest, &registry)
                .expect("an initial-input-sourced mapping needs no capability-to-capability classification check");
        assert_eq!(resolved.len(), 1);
    }

    #[test]
    fn validate_proposal_against_host_state_skips_a_mapping_whose_target_node_is_not_resolved() {
        // `mapping.target_node_id` is normally guaranteed to reference a
        // declared node by `canonicalize_proposal`'s own validation, but
        // `CanonicalProposal` has public fields, so a caller could construct
        // one by hand bypassing that guarantee. This proves the function
        // degrades gracefully (skips the dangling mapping) rather than
        // panicking on such a malformed value.
        let manifest = manifest_declaring(&[("test.produce", "1.0.0")]);
        let registry = registry_with(vec![(
            contract(
                "test.produce",
                "1.0.0",
                json!({"type": "object"}),
                json!({"type": "object"}),
                automatic_risk(),
            ),
            artifact("digest-a"),
        )]);

        let mut proposal = linear_proposal_source();
        proposal.nodes.truncate(1);
        proposal.edges.clear();
        proposal.mappings[0].target_node_id = "ghost".to_string();
        let canonical = CanonicalProposal {
            execution_order: vec!["a".to_string()],
            proposal,
        };

        let resolved = validate_proposal_against_host_state(&canonical, &manifest, &registry)
            .expect("a dangling mapping target must not itself fail cross-validation");
        assert_eq!(resolved.len(), 1);
    }

    // -- Automatic eligibility -----------------------------------------------

    #[test]
    fn proposal_is_automatic_eligible_true_when_every_node_is_automatic_eligible() {
        let nodes = vec![
            ResolvedProposalNode {
                node_id: "a".to_string(),
                contract: contract(
                    "test.produce",
                    "1.0.0",
                    json!({}),
                    json!({}),
                    automatic_risk(),
                ),
            },
            ResolvedProposalNode {
                node_id: "b".to_string(),
                contract: contract(
                    "test.consume",
                    "1.0.0",
                    json!({}),
                    json!({}),
                    automatic_risk(),
                ),
            },
        ];
        assert!(proposal_is_automatic_eligible(&nodes));
    }

    #[test]
    fn proposal_is_automatic_eligible_false_when_any_node_is_not() {
        let mut non_automatic = automatic_risk();
        non_automatic.effect_class = EffectClass::StateWrite;
        let nodes = vec![
            ResolvedProposalNode {
                node_id: "a".to_string(),
                contract: contract(
                    "test.produce",
                    "1.0.0",
                    json!({}),
                    json!({}),
                    automatic_risk(),
                ),
            },
            ResolvedProposalNode {
                node_id: "b".to_string(),
                contract: contract("test.consume", "1.0.0", json!({}), json!({}), non_automatic),
            },
        ];
        assert!(!proposal_is_automatic_eligible(&nodes));
    }

    // -- Approval token verification -----------------------------------------

    fn signing_key() -> SigningKey {
        SigningKey::from_bytes(&[9_u8; 32])
    }

    fn base64url_encode(input: &[u8]) -> String {
        const ALPHABET: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let mut out = String::new();
        let mut i = 0;
        while i + 3 <= input.len() {
            let n = (u32::from(input[i]) << 16)
                | (u32::from(input[i + 1]) << 8)
                | u32::from(input[i + 2]);
            out.push(ALPHABET[((n >> 18) & 63) as usize] as char);
            out.push(ALPHABET[((n >> 12) & 63) as usize] as char);
            out.push(ALPHABET[((n >> 6) & 63) as usize] as char);
            out.push(ALPHABET[(n & 63) as usize] as char);
            i += 3;
        }
        let remainder = input.len() - i;
        if remainder == 1 {
            let n = u32::from(input[i]) << 16;
            out.push(ALPHABET[((n >> 18) & 63) as usize] as char);
            out.push(ALPHABET[((n >> 12) & 63) as usize] as char);
        } else if remainder == 2 {
            let n = (u32::from(input[i]) << 16) | (u32::from(input[i + 1]) << 8);
            out.push(ALPHABET[((n >> 18) & 63) as usize] as char);
            out.push(ALPHABET[((n >> 12) & 63) as usize] as char);
            out.push(ALPHABET[((n >> 6) & 63) as usize] as char);
        }
        out
    }

    fn sign_token(payload: &Value, key: &SigningKey, key_id: &str) -> String {
        let header = base64url_encode(format!(r#"{{"alg":"EdDSA","kid":"{key_id}"}}"#).as_bytes());
        let payload_b64 = base64url_encode(payload.to_string().as_bytes());
        let signing_input = format!("{header}.{payload_b64}");
        let signature = key.sign(signing_input.as_bytes());
        let signature_b64 = base64url_encode(&signature.to_bytes());
        format!("{header}.{payload_b64}.{signature_b64}")
    }

    fn valid_claims_payload() -> Value {
        json!({
            "jti": "token-001",
            "iss": "traverse-approval-service",
            "aud": "traverse-runtime",
            "sub": "principal-001",
            "workspace_id": "workspace-001",
            "proposal_digest": "digest-p",
            "snapshot_digest": "digest-s",
            "permitted_effects": ["external_effect"],
            "permitted_connectors": ["traverse.http"],
            "max_use_count": 1,
            "exp": 4_102_444_800_i64, // 2100-01-01, far future
        })
    }

    fn verification_context(
        keys: &HashMap<String, ed25519_dalek::VerifyingKey>,
    ) -> ApprovalTokenVerificationContext<'_> {
        ApprovalTokenVerificationContext {
            expected_issuer: "traverse-approval-service",
            expected_audience: "traverse-runtime",
            expected_workspace_id: "workspace-001",
            expected_proposal_digest: "digest-p",
            expected_snapshot_digest: "digest-s",
            verifying_keys_by_key_id: keys,
        }
    }

    fn keys_with_signing_key() -> HashMap<String, ed25519_dalek::VerifyingKey> {
        let mut keys = HashMap::new();
        keys.insert("key-1".to_string(), signing_key().verifying_key());
        keys
    }

    #[test]
    fn verifies_a_well_formed_approval_token() {
        let token = sign_token(&valid_claims_payload(), &signing_key(), "key-1");
        let keys = keys_with_signing_key();
        let claims = verify_approval_token(&token, &verification_context(&keys))
            .expect("well-formed token must verify");
        assert_eq!(claims.token_id, "token-001");
        assert_eq!(claims.principal, "principal-001");
        assert_eq!(claims.permitted_effects, vec![EffectClass::ExternalEffect]);
    }

    #[test]
    fn rejects_malformed_token_shape() {
        let keys = keys_with_signing_key();
        let failure = verify_approval_token("not-a-token", &verification_context(&keys))
            .expect_err("malformed token must be rejected");
        assert_eq!(failure.code, ApprovalTokenErrorCode::Malformed);
    }

    #[test]
    fn rejects_disallowed_algorithm() {
        let header = base64url_encode(br#"{"alg":"HS256","kid":"key-1"}"#);
        let payload = base64url_encode(valid_claims_payload().to_string().as_bytes());
        let token = format!("{header}.{payload}.sig");
        let keys = keys_with_signing_key();
        let failure = verify_approval_token(&token, &verification_context(&keys))
            .expect_err("disallowed alg must be rejected");
        assert_eq!(failure.code, ApprovalTokenErrorCode::AlgorithmNotAllowed);
    }

    #[test]
    fn rejects_unknown_key_id() {
        let token = sign_token(&valid_claims_payload(), &signing_key(), "unknown-key");
        let keys = keys_with_signing_key();
        let failure = verify_approval_token(&token, &verification_context(&keys))
            .expect_err("unknown key id must be rejected");
        assert_eq!(failure.code, ApprovalTokenErrorCode::UnknownKeyId);
    }

    #[test]
    fn rejects_bad_signature() {
        let other_key = SigningKey::from_bytes(&[3_u8; 32]);
        let token = sign_token(&valid_claims_payload(), &other_key, "key-1");
        let keys = keys_with_signing_key();
        let failure = verify_approval_token(&token, &verification_context(&keys))
            .expect_err("signature from the wrong key must be rejected");
        assert_eq!(
            failure.code,
            ApprovalTokenErrorCode::SignatureVerificationFailed
        );
    }

    #[test]
    fn rejects_issuer_mismatch() {
        let mut payload = valid_claims_payload();
        payload["iss"] = json!("someone-else");
        let token = sign_token(&payload, &signing_key(), "key-1");
        let keys = keys_with_signing_key();
        let failure = verify_approval_token(&token, &verification_context(&keys))
            .expect_err("issuer mismatch must be rejected");
        assert_eq!(failure.code, ApprovalTokenErrorCode::IssuerMismatch);
    }

    #[test]
    fn rejects_audience_mismatch() {
        let mut payload = valid_claims_payload();
        payload["aud"] = json!("someone-else");
        let token = sign_token(&payload, &signing_key(), "key-1");
        let keys = keys_with_signing_key();
        let failure = verify_approval_token(&token, &verification_context(&keys))
            .expect_err("audience mismatch must be rejected");
        assert_eq!(failure.code, ApprovalTokenErrorCode::AudienceMismatch);
    }

    #[test]
    fn rejects_workspace_mismatch() {
        let mut payload = valid_claims_payload();
        payload["workspace_id"] = json!("someone-elses-workspace");
        let token = sign_token(&payload, &signing_key(), "key-1");
        let keys = keys_with_signing_key();
        let failure = verify_approval_token(&token, &verification_context(&keys))
            .expect_err("workspace mismatch must be rejected");
        assert_eq!(failure.code, ApprovalTokenErrorCode::WorkspaceMismatch);
    }

    #[test]
    fn rejects_proposal_digest_mismatch() {
        let mut payload = valid_claims_payload();
        payload["proposal_digest"] = json!("different-digest");
        let token = sign_token(&payload, &signing_key(), "key-1");
        let keys = keys_with_signing_key();
        let failure = verify_approval_token(&token, &verification_context(&keys))
            .expect_err("proposal digest mismatch must be rejected");
        assert_eq!(failure.code, ApprovalTokenErrorCode::ProposalDigestMismatch);
    }

    #[test]
    fn rejects_snapshot_digest_mismatch() {
        let mut payload = valid_claims_payload();
        payload["snapshot_digest"] = json!("different-digest");
        let token = sign_token(&payload, &signing_key(), "key-1");
        let keys = keys_with_signing_key();
        let failure = verify_approval_token(&token, &verification_context(&keys))
            .expect_err("snapshot digest mismatch must be rejected");
        assert_eq!(failure.code, ApprovalTokenErrorCode::SnapshotDigestMismatch);
    }

    #[test]
    fn rejects_expired_token() {
        let mut payload = valid_claims_payload();
        payload["exp"] = json!(1); // 1970
        let token = sign_token(&payload, &signing_key(), "key-1");
        let keys = keys_with_signing_key();
        let failure = verify_approval_token(&token, &verification_context(&keys))
            .expect_err("expired token must be rejected");
        assert_eq!(failure.code, ApprovalTokenErrorCode::Expired);
    }

    #[test]
    fn rejects_a_token_with_an_invalid_base64url_character() {
        let payload = base64url_encode(valid_claims_payload().to_string().as_bytes());
        let token = format!("not!valid!base64.{payload}.sig");
        let keys = keys_with_signing_key();
        let failure = verify_approval_token(&token, &verification_context(&keys))
            .expect_err("an invalid base64url character must be rejected");
        assert_eq!(failure.code, ApprovalTokenErrorCode::Malformed);
    }

    #[test]
    fn rejects_a_token_whose_header_is_not_valid_json() {
        let header = base64url_encode(b"not-json");
        let payload = base64url_encode(valid_claims_payload().to_string().as_bytes());
        let token = format!("{header}.{payload}.sig");
        let keys = keys_with_signing_key();
        let failure = verify_approval_token(&token, &verification_context(&keys))
            .expect_err("a non-JSON header must be rejected");
        assert_eq!(failure.code, ApprovalTokenErrorCode::Malformed);
    }

    #[test]
    fn rejects_a_token_with_a_wrong_length_signature() {
        let header = base64url_encode(br#"{"alg":"EdDSA","kid":"key-1"}"#);
        let payload_b64 = base64url_encode(valid_claims_payload().to_string().as_bytes());
        let short_signature = base64url_encode(b"too-short");
        let token = format!("{header}.{payload_b64}.{short_signature}");
        let keys = keys_with_signing_key();
        let failure = verify_approval_token(&token, &verification_context(&keys))
            .expect_err("a signature that is not 64 bytes must be rejected");
        assert_eq!(
            failure.code,
            ApprovalTokenErrorCode::SignatureVerificationFailed
        );
    }

    #[test]
    fn rejects_a_token_whose_payload_is_not_valid_json() {
        let key = signing_key();
        let header = base64url_encode(br#"{"alg":"EdDSA","kid":"key-1"}"#);
        let payload_b64 = base64url_encode(b"not-json");
        let signing_input = format!("{header}.{payload_b64}");
        let signature = key.sign(signing_input.as_bytes());
        let signature_b64 = base64url_encode(&signature.to_bytes());
        let token = format!("{header}.{payload_b64}.{signature_b64}");
        let keys = keys_with_signing_key();
        let failure = verify_approval_token(&token, &verification_context(&keys))
            .expect_err("a non-JSON payload must be rejected");
        assert_eq!(failure.code, ApprovalTokenErrorCode::Malformed);
    }

    #[test]
    fn rejects_a_token_missing_a_required_string_claim() {
        let mut payload = valid_claims_payload();
        payload
            .as_object_mut()
            .expect("payload fixture is an object")
            .remove("jti");
        let token = sign_token(&payload, &signing_key(), "key-1");
        let keys = keys_with_signing_key();
        let failure = verify_approval_token(&token, &verification_context(&keys))
            .expect_err("a missing required string claim must be rejected");
        assert_eq!(failure.code, ApprovalTokenErrorCode::Malformed);
    }

    #[test]
    fn rejects_a_token_missing_max_use_count() {
        let mut payload = valid_claims_payload();
        payload
            .as_object_mut()
            .expect("payload fixture is an object")
            .remove("max_use_count");
        let token = sign_token(&payload, &signing_key(), "key-1");
        let keys = keys_with_signing_key();
        let failure = verify_approval_token(&token, &verification_context(&keys))
            .expect_err("a missing max_use_count claim must be rejected");
        assert_eq!(failure.code, ApprovalTokenErrorCode::Malformed);
    }

    #[test]
    fn rejects_a_token_missing_exp() {
        let mut payload = valid_claims_payload();
        payload
            .as_object_mut()
            .expect("payload fixture is an object")
            .remove("exp");
        let token = sign_token(&payload, &signing_key(), "key-1");
        let keys = keys_with_signing_key();
        let failure = verify_approval_token(&token, &verification_context(&keys))
            .expect_err("a missing exp claim must be rejected");
        assert_eq!(failure.code, ApprovalTokenErrorCode::Malformed);
    }

    #[test]
    fn accepts_a_token_permitting_an_irreversible_effect() {
        let mut payload = valid_claims_payload();
        payload["permitted_effects"] = json!(["irreversible_effect"]);
        let token = sign_token(&payload, &signing_key(), "key-1");
        let keys = keys_with_signing_key();
        let claims = verify_approval_token(&token, &verification_context(&keys))
            .expect("a token permitting an irreversible effect must verify");
        assert_eq!(
            claims.permitted_effects,
            vec![EffectClass::IrreversibleEffect]
        );
    }

    #[test]
    fn ignores_an_unrecognized_permitted_effect_string() {
        let mut payload = valid_claims_payload();
        payload["permitted_effects"] = json!(["not_a_real_effect", "external_effect"]);
        let token = sign_token(&payload, &signing_key(), "key-1");
        let keys = keys_with_signing_key();
        let claims = verify_approval_token(&token, &verification_context(&keys))
            .expect("an unrecognized effect string must be filtered out, not rejected");
        assert_eq!(claims.permitted_effects, vec![EffectClass::ExternalEffect]);
    }

    #[test]
    fn approval_token_store_default_starts_with_no_recorded_uses() {
        let store = ApprovalTokenStore::default();
        store
            .check_and_record_use(&claims_with_use_count(1))
            .expect("a freshly defaulted store has no recorded uses");
    }

    #[test]
    fn check_and_record_use_fails_closed_when_the_store_mutex_is_poisoned() {
        let store = ApprovalTokenStore::new();

        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = store
                .used
                .lock()
                .expect("lock must be acquirable to poison it");
            panic!("poison approval token store lock for test");
        }));

        let failure = store
            .check_and_record_use(&claims_with_use_count(10))
            .expect_err("a poisoned store must fail closed");
        assert_eq!(failure.code, ApprovalTokenErrorCode::StoreUnavailable);
    }

    // -- Token store -----------------------------------------------------------

    fn claims_with_use_count(max_use_count: u32) -> ApprovalTokenClaims {
        ApprovalTokenClaims {
            token_id: "token-001".to_string(),
            issuer: "traverse-approval-service".to_string(),
            key_id: "key-1".to_string(),
            audience: "traverse-runtime".to_string(),
            principal: "principal-001".to_string(),
            workspace_id: "workspace-001".to_string(),
            proposal_digest: "digest-p".to_string(),
            snapshot_digest: "digest-s".to_string(),
            permitted_effects: Vec::new(),
            permitted_connectors: Vec::new(),
            max_use_count,
            expiry_unix: 4_102_444_800,
        }
    }

    #[test]
    fn token_store_enforces_max_use_count() {
        let store = ApprovalTokenStore::new();
        let claims = claims_with_use_count(2);
        store
            .check_and_record_use(&claims)
            .expect("first use must succeed");
        store
            .check_and_record_use(&claims)
            .expect("second use must succeed");
        let failure = store
            .check_and_record_use(&claims)
            .expect_err("third use beyond max_use_count must be rejected");
        assert_eq!(failure.code, ApprovalTokenErrorCode::UseCountExhausted);
    }

    #[test]
    fn token_store_denies_a_revoked_token() {
        let store = ApprovalTokenStore::new();
        let claims = claims_with_use_count(10);
        store.revoke(&claims.token_id);
        let failure = store
            .check_and_record_use(&claims)
            .expect_err("revoked token must be rejected");
        assert_eq!(failure.code, ApprovalTokenErrorCode::Revoked);
    }

    // -- Quota tracker -----------------------------------------------------------

    #[test]
    fn quota_tracker_denies_when_principal_limit_reached() {
        let tracker = QuotaTracker::new();
        let limits = QuotaLimits {
            max_concurrent_per_principal: 1,
            max_concurrent_per_app: 10,
            max_concurrent_per_workspace: 10,
        };
        let _first = tracker
            .reserve("principal-1", "app-1", "workspace-1", &limits)
            .expect("first reservation must succeed");
        let denial = tracker
            .reserve("principal-1", "app-2", "workspace-2", &limits)
            .expect_err("second reservation for the same principal must be denied");
        assert_eq!(denial.scope, "principal");
    }

    #[test]
    fn quota_tracker_denies_when_workspace_limit_reached() {
        let tracker = QuotaTracker::new();
        let limits = QuotaLimits {
            max_concurrent_per_principal: 10,
            max_concurrent_per_app: 10,
            max_concurrent_per_workspace: 1,
        };
        let _first = tracker
            .reserve("principal-1", "app-1", "workspace-1", &limits)
            .expect("first reservation must succeed");
        let denial = tracker
            .reserve("principal-2", "app-2", "workspace-1", &limits)
            .expect_err("second reservation for the same workspace must be denied");
        assert_eq!(denial.scope, "workspace");
    }

    #[test]
    fn quota_tracker_releases_on_drop_allowing_reuse() {
        let tracker = QuotaTracker::new();
        let limits = QuotaLimits {
            max_concurrent_per_principal: 1,
            max_concurrent_per_app: 1,
            max_concurrent_per_workspace: 1,
        };
        {
            let _reservation = tracker
                .reserve("principal-1", "app-1", "workspace-1", &limits)
                .expect("first reservation must succeed");
        }
        tracker
            .reserve("principal-1", "app-1", "workspace-1", &limits)
            .expect("reservation must succeed again after the first is dropped");
    }

    #[test]
    fn quota_tracker_denies_when_app_limit_reached_and_rolls_back_the_principal_reservation() {
        let tracker = QuotaTracker::new();
        let limits = QuotaLimits {
            max_concurrent_per_principal: 1,
            max_concurrent_per_app: 1,
            max_concurrent_per_workspace: 10,
        };
        let _first = tracker
            .reserve("principal-1", "app-1", "workspace-1", &limits)
            .expect("first reservation must succeed");

        let denial = tracker
            .reserve("principal-2", "app-1", "workspace-2", &limits)
            .expect_err("second reservation against the same app must be denied");
        assert_eq!(denial.scope, "app");

        // If the principal-dimension reservation taken just before the
        // app-dimension denial were not rolled back, principal-2 would
        // already be at its limit and this would incorrectly fail too.
        tracker
            .reserve("principal-2", "app-2", "workspace-3", &limits)
            .expect("principal-2's slot must have been released by the app-limit rollback");
    }

    #[test]
    fn quota_limits_default_matches_the_documented_defaults() {
        let limits = QuotaLimits::default();
        assert_eq!(
            limits.max_concurrent_per_principal,
            DEFAULT_MAX_CONCURRENT_PER_PRINCIPAL
        );
        assert_eq!(
            limits.max_concurrent_per_app,
            DEFAULT_MAX_CONCURRENT_PER_APP
        );
        assert_eq!(
            limits.max_concurrent_per_workspace,
            DEFAULT_MAX_CONCURRENT_PER_WORKSPACE
        );
    }

    #[test]
    fn reserve_fails_closed_when_a_quota_dimension_mutex_is_poisoned() {
        let tracker = QuotaTracker::new();

        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = tracker
                .principal_slots
                .lock()
                .expect("lock must be acquirable to poison it");
            panic!("poison quota tracker principal_slots lock for test");
        }));

        let limits = QuotaLimits {
            max_concurrent_per_principal: 10,
            max_concurrent_per_app: 10,
            max_concurrent_per_workspace: 10,
        };
        let denial = tracker
            .reserve("principal-1", "app-1", "workspace-1", &limits)
            .expect_err("a poisoned quota dimension must fail closed");
        assert_eq!(denial.scope, "store");
    }

    // -- Execution engine -----------------------------------------------------------

    #[derive(Default)]
    struct MappingAwareExecutor {
        consumer_saw_input: std::sync::Arc<Mutex<Option<Value>>>,
    }

    impl LocalExecutor for MappingAwareExecutor {
        fn execute(
            &self,
            capability: &traverse_registry::ResolvedCapability,
            input: &Value,
        ) -> Result<LocalExecutionOutput, LocalExecutionFailure> {
            if capability.contract.id == "test.produce" {
                Ok(LocalExecutionOutput {
                    value: json!({"value": "produced-by-a"}),
                    emitted_events: Vec::new(),
                })
            } else {
                if let Ok(mut seen) = self.consumer_saw_input.lock() {
                    *seen = Some(input.clone());
                }
                Ok(LocalExecutionOutput {
                    value: json!({"received": input.clone()}),
                    emitted_events: Vec::new(),
                })
            }
        }
    }

    struct AlwaysFailingExecutor;

    impl LocalExecutor for AlwaysFailingExecutor {
        fn execute(
            &self,
            _capability: &traverse_registry::ResolvedCapability,
            _input: &Value,
        ) -> Result<LocalExecutionOutput, LocalExecutionFailure> {
            Err(LocalExecutionFailure {
                code: LocalExecutionFailureCode::ExecutionFailed,
                message: "always fails".to_string(),
            })
        }
    }

    fn resolved_nodes() -> Vec<ResolvedProposalNode> {
        vec![
            ResolvedProposalNode {
                node_id: "a".to_string(),
                contract: contract(
                    "test.produce",
                    "1.0.0",
                    json!({}),
                    json!({}),
                    automatic_risk(),
                ),
            },
            ResolvedProposalNode {
                node_id: "b".to_string(),
                contract: contract(
                    "test.consume",
                    "1.0.0",
                    json!({}),
                    json!({}),
                    automatic_risk(),
                ),
            },
        ]
    }

    #[test]
    fn executes_a_linear_proposal_threading_mapped_data_between_nodes() {
        let registry = registry_with(vec![
            (
                contract(
                    "test.produce",
                    "1.0.0",
                    json!({}),
                    json!({}),
                    automatic_risk(),
                ),
                artifact("digest-a"),
            ),
            (
                contract(
                    "test.consume",
                    "1.0.0",
                    json!({}),
                    json!({}),
                    automatic_risk(),
                ),
                artifact("digest-b"),
            ),
        ]);
        let consumer_saw_input = std::sync::Arc::new(Mutex::new(None));
        let executor = MappingAwareExecutor {
            consumer_saw_input: consumer_saw_input.clone(),
        };
        let runtime = Runtime::new(registry, executor)
            .with_security_config(RuntimeSecurityConfig::development());

        let canonical = linear_canonical();
        let digest = proposal_digest(&canonical.proposal);
        let trace = execute_proposal(
            &runtime,
            &canonical,
            &resolved_nodes(),
            AuthorizationSummary {
                automatic: true,
                approval_token_id: None,
            },
            &digest,
            "snapshot-digest",
        );

        assert_eq!(trace.terminal_state, ProposalTerminalState::Succeeded);
        assert_eq!(trace.node_outcomes.len(), 2);
        assert_eq!(trace.node_outcomes[0].status, ProposalNodeStatus::Succeeded);
        assert_eq!(trace.node_outcomes[1].status, ProposalNodeStatus::Succeeded);

        let seen_input = consumer_saw_input
            .lock()
            .expect("test mutex must not be poisoned")
            .clone()
            .expect("consumer node must have executed and recorded its input");
        assert_eq!(
            seen_input,
            json!({"value": "produced-by-a"}),
            "node b's input must be assembled solely from the declared mapping, not node a's full output"
        );
    }

    #[test]
    fn stops_at_first_failure_and_skips_remaining_nodes() {
        let registry = registry_with(vec![
            (
                contract(
                    "test.produce",
                    "1.0.0",
                    json!({}),
                    json!({}),
                    automatic_risk(),
                ),
                artifact("digest-a"),
            ),
            (
                contract(
                    "test.consume",
                    "1.0.0",
                    json!({}),
                    json!({}),
                    automatic_risk(),
                ),
                artifact("digest-b"),
            ),
        ]);
        let runtime = Runtime::new(registry, AlwaysFailingExecutor)
            .with_security_config(RuntimeSecurityConfig::development());

        let canonical = linear_canonical();
        let digest = proposal_digest(&canonical.proposal);
        let trace = execute_proposal(
            &runtime,
            &canonical,
            &resolved_nodes(),
            AuthorizationSummary {
                automatic: true,
                approval_token_id: None,
            },
            &digest,
            "snapshot-digest",
        );

        assert_eq!(trace.terminal_state, ProposalTerminalState::Failed);
        assert_eq!(trace.node_outcomes[0].status, ProposalNodeStatus::Failed);
        assert_eq!(
            trace.node_outcomes[1].status,
            ProposalNodeStatus::SkippedAfterEarlierFailure
        );
    }

    #[test]
    fn executes_a_mapping_sourced_from_initial_input_into_a_nested_target_path() {
        let registry = registry_with(vec![(
            contract(
                "test.consume",
                "1.0.0",
                json!({}),
                json!({}),
                automatic_risk(),
            ),
            artifact("digest-x"),
        )]);
        let consumer_saw_input = std::sync::Arc::new(Mutex::new(None));
        let executor = MappingAwareExecutor {
            consumer_saw_input: consumer_saw_input.clone(),
        };
        let runtime = Runtime::new(registry, executor)
            .with_security_config(RuntimeSecurityConfig::development());

        let canonical = initial_input_canonical();
        let digest = proposal_digest(&canonical.proposal);
        let trace = execute_proposal(
            &runtime,
            &canonical,
            &[ResolvedProposalNode {
                node_id: "x".to_string(),
                contract: contract(
                    "test.consume",
                    "1.0.0",
                    json!({}),
                    json!({}),
                    automatic_risk(),
                ),
            }],
            AuthorizationSummary {
                automatic: true,
                approval_token_id: None,
            },
            &digest,
            "snapshot-digest",
        );

        assert_eq!(trace.terminal_state, ProposalTerminalState::Succeeded);
        let seen_input = consumer_saw_input
            .lock()
            .expect("test mutex must not be poisoned")
            .clone()
            .expect("consumer must have received input");
        assert_eq!(seen_input["nested"]["value"], json!("from-caller"));
    }

    #[test]
    fn execute_proposal_skips_an_execution_order_entry_with_no_matching_node() {
        // `execution_order` is normally guaranteed to be a permutation of
        // `proposal.nodes`' ids by `canonicalize_proposal`, but
        // `CanonicalProposal` has public fields, so a caller could construct
        // one by hand with a mismatched entry. This proves execution degrades
        // gracefully (skips the unmatched entry) rather than panicking.
        let registry = registry_with(vec![(
            contract(
                "test.produce",
                "1.0.0",
                json!({}),
                json!({}),
                automatic_risk(),
            ),
            artifact("digest-a"),
        )]);
        let runtime = Runtime::new(registry, MappingAwareExecutor::default())
            .with_security_config(RuntimeSecurityConfig::development());

        let mut proposal = linear_proposal_source();
        proposal.nodes.truncate(1);
        proposal.edges.clear();
        proposal.mappings.clear();
        let canonical = CanonicalProposal {
            execution_order: vec!["a".to_string(), "ghost".to_string()],
            proposal,
        };
        let digest = proposal_digest(&canonical.proposal);
        let trace = execute_proposal(
            &runtime,
            &canonical,
            &[ResolvedProposalNode {
                node_id: "a".to_string(),
                contract: contract(
                    "test.produce",
                    "1.0.0",
                    json!({}),
                    json!({}),
                    automatic_risk(),
                ),
            }],
            AuthorizationSummary {
                automatic: true,
                approval_token_id: None,
            },
            &digest,
            "snapshot-digest",
        );

        assert_eq!(trace.terminal_state, ProposalTerminalState::Succeeded);
        assert_eq!(trace.node_outcomes.len(), 1);
    }

    #[test]
    fn pointer_set_with_an_empty_pointer_replaces_the_entire_target() {
        let mut target = json!({"unused": true});
        pointer_set(&mut target, "", json!({"replaced": true}));
        assert_eq!(target, json!({"replaced": true}));
    }
}
