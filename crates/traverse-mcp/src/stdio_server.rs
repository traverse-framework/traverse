//! Dedicated Traverse MCP stdio server package entrypoint.

use crate::{
    McpLifecycleStatus, McpObservationMessage, TraverseMcp,
    youaskm3_mcp_consumption_validation_path,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::fmt;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use traverse_embedder::{
    HostRegistryCache, PublicCapabilityMetadata, PublicMetadataRead, RegistryCacheError,
    read_public_metadata, resolve_registry_component,
};
use traverse_registry::{
    ArtifactDigests, BinaryFormat as RegistryBinaryFormat, BinaryReference,
    CapabilityArtifactRecord, CapabilityRegistration, CapabilityRegistry, ComposabilityMetadata,
    CompositionKind, CompositionPattern, EventRegistration, EventRegistry, ImplementationKind,
    RegistryBundle, RegistryProvenance, RegistryReference, RegistryScope,
    ResolvedRegistryComponent, SourceKind, SourceReference, WorkflowReference,
    WorkflowRegistration, WorkflowRegistry, load_registry_bundle,
};
use traverse_runtime::security::RuntimeSecurityConfig;
use traverse_runtime::{
    ArtifactRouter, LocalExecutor, Runtime, RuntimeRequest, parse_runtime_request,
};

const SERVER_NAME: &str = "traverse-mcp";
const HOST_MODE: &str = "stdio";
const GOVERNING_SPEC: &str = "022-mcp-wasm-server";
/// Governing spec for the verified public-registry Mode A discovery/execution
/// surface (env `TRAVERSE_MCP_REGISTRY_CACHE`).
const MODE_A_GOVERNING_SPEC: &str = "119-verified-registry-mcp-mode-a";
/// Environment variable naming the host-owned, digest-verified registry cache
/// root that Mode A consumes. When unset, the server runs the contributor-only
/// expedition example path.
const MODE_A_CACHE_ENV: &str = "TRAVERSE_MCP_REGISTRY_CACHE";
const PUBLIC_SURFACE_ID: &str = "traverse.mcp.stdio-server";
const SUPPORTING_COMMANDS: &[&str] = &[
    "describe_server",
    "list_content_groups",
    "describe_content_group",
    "list_entrypoints",
    "search_capabilities",
    "describe_entrypoint",
    "validate_entrypoint",
    "execute_entrypoint",
    "render_execution_report",
    "shutdown",
];

#[derive(Debug, Deserialize)]
struct StdioCommandEnvelope {
    command: String,
    #[serde(default)]
    auth: Option<StdioAuthEnvelope>,
    #[serde(default)]
    bearer_token: Option<String>,
    #[serde(default)]
    content_group_id: Option<String>,
    #[serde(default)]
    entrypoint_kind: Option<String>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    request_path: Option<String>,
    /// Spec 119 FR-002: inline `RuntimeRequest` object, mutually exclusive with
    /// `request_path`. When present the server never touches the filesystem to
    /// materialize the request.
    #[serde(default)]
    request: Option<Value>,
    #[serde(default)]
    query: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StdioAuthEnvelope {
    #[serde(default)]
    r#type: Option<String>,
    #[serde(default)]
    token: Option<String>,
}

#[derive(Clone, PartialEq, Eq)]
pub struct StdioAuthConfig {
    mode: StdioAuthMode,
}

#[derive(Clone, PartialEq, Eq)]
enum StdioAuthMode {
    LocalTrust,
    BearerRequired { token: String },
}

impl fmt::Debug for StdioAuthConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.mode {
            StdioAuthMode::LocalTrust => f
                .debug_struct("StdioAuthConfig")
                .field("mode", &"local_trust")
                .finish(),
            StdioAuthMode::BearerRequired { .. } => f
                .debug_struct("StdioAuthConfig")
                .field("mode", &"bearer_required")
                .field("token", &"<redacted>")
                .finish(),
        }
    }
}

impl StdioAuthConfig {
    #[must_use]
    pub fn local_trust() -> Self {
        Self {
            mode: StdioAuthMode::LocalTrust,
        }
    }

    #[must_use]
    pub fn bearer_required(token: impl Into<String>) -> Self {
        Self {
            mode: StdioAuthMode::BearerRequired {
                token: token.into(),
            },
        }
    }

    #[must_use]
    pub fn from_env() -> Self {
        Self::from_env_var(std::env::var("TRAVERSE_MCP_STDIO_BEARER_TOKEN"))
    }

    /// Testable core of [`Self::from_env`]: takes the lookup result directly
    /// instead of reading the process environment, since mutating env vars
    /// from a test is `unsafe` and this workspace forbids `unsafe` entirely.
    fn from_env_var(token: Result<String, std::env::VarError>) -> Self {
        match token {
            Ok(token) if !token.is_empty() => Self::bearer_required(token),
            _ => Self::local_trust(),
        }
    }

    fn mode_name(&self) -> &'static str {
        match self.mode {
            StdioAuthMode::LocalTrust => "local_trust",
            StdioAuthMode::BearerRequired { .. } => "bearer_required",
        }
    }

    fn verify_execute_command(
        &self,
        command: &StdioCommandEnvelope,
    ) -> Result<(), StdioServerFailure> {
        let StdioAuthMode::BearerRequired { token } = &self.mode else {
            return Ok(());
        };

        let supplied = command
            .auth
            .as_ref()
            .and_then(|auth| {
                let auth_type = auth.r#type.as_deref().unwrap_or("bearer");
                (auth_type == "bearer")
                    .then_some(auth.token.as_deref())
                    .flatten()
            })
            .or(command.bearer_token.as_deref());

        match supplied {
            Some(candidate) if candidate == token => Ok(()),
            _ => Err(StdioServerFailure::new(
                "auth_required",
                "execute command requires a valid MCP stdio bearer token.",
            )),
        }
    }
}

#[derive(Debug)]
pub struct McpDiscoveryCatalog {
    bundle: RegistryBundle,
}

#[derive(Debug)]
struct CanonicalExecutionContext {
    capabilities: CapabilityRegistry,
    events: EventRegistry,
    workflows: WorkflowRegistry,
}

impl McpDiscoveryCatalog {
    /// Load the canonical discovery catalog used by the stdio server.
    ///
    /// # Errors
    ///
    /// Returns `catalog_load_failed` when the expedition registry bundle cannot be loaded.
    pub fn load_canonical() -> Result<Self, StdioServerFailure> {
        Self::load_from_manifest_path(&canonical_expedition_bundle_path())
    }

    fn load_from_manifest_path(manifest_path: &Path) -> Result<Self, StdioServerFailure> {
        let bundle = load_registry_bundle(manifest_path).map_err(|failure| {
            StdioServerFailure::new(
                "catalog_load_failed",
                format!(
                    "Failed to load expedition registry bundle {}: {}",
                    manifest_path.display(),
                    failure.errors[0].message
                ),
            )
        })?;

        Ok(Self { bundle })
    }

    #[must_use]
    pub fn capability_count(&self) -> usize {
        self.bundle.capabilities.len()
    }

    #[must_use]
    pub fn workflow_count(&self) -> usize {
        self.bundle.workflows.len()
    }

    #[must_use]
    pub fn event_count(&self) -> usize {
        self.bundle.events.len()
    }
}

impl CanonicalExecutionContext {
    fn load_canonical() -> Result<Self, StdioServerFailure> {
        Self::load_from_manifest_path(&canonical_expedition_bundle_path())
    }

    fn load_from_manifest_path(manifest_path: &Path) -> Result<Self, StdioServerFailure> {
        let bundle = load_registry_bundle(manifest_path).map_err(|failure| {
            StdioServerFailure::new(
                "catalog_load_failed",
                format!(
                    "Failed to load expedition registry bundle {}: {}",
                    manifest_path.display(),
                    failure.errors[0].message
                ),
            )
        })?;

        let mut capabilities = CapabilityRegistry::new();
        let mut events = EventRegistry::new();
        let mut workflows = WorkflowRegistry::new();

        for capability in &bundle.capabilities {
            let request = build_capability_registration(&bundle, capability)?;
            capabilities.register(request).map_err(|failure| {
                StdioServerFailure::new(
                    "registry_registration_failed",
                    format!(
                        "Failed to register capability {}@{} for stdio execution: {}",
                        capability.contract.id,
                        capability.contract.version,
                        failure.errors[0].message,
                    ),
                )
            })?;
        }

        for event in &bundle.events {
            let request = EventRegistration {
                scope: bundle.scope,
                contract: event.contract.clone(),
                contract_path: event.path.display().to_string(),
                registered_at: bundle_registered_at(&bundle),
                governing_spec: "011-event-registry".to_string(),
                validator_version: env!("CARGO_PKG_VERSION").to_string(),
            };
            events.register(request).map_err(|failure| {
                StdioServerFailure::new(
                    "registry_registration_failed",
                    format!(
                        "Failed to register event {}@{} for stdio execution: {}",
                        event.contract.id, event.contract.version, failure.errors[0].message,
                    ),
                )
            })?;
        }

        for workflow in &bundle.workflows {
            workflows
                .register(
                    &capabilities,
                    WorkflowRegistration {
                        scope: bundle.scope,
                        definition: workflow.definition.clone(),
                        workflow_path: workflow.path.display().to_string(),
                        registered_at: bundle_registered_at(&bundle),
                        validator_version: env!("CARGO_PKG_VERSION").to_string(),
                    },
                )
                .map_err(|failure| {
                    StdioServerFailure::new(
                        "registry_registration_failed",
                        format!(
                            "Failed to register workflow {}@{} for stdio execution: {}",
                            workflow.definition.id,
                            workflow.definition.version,
                            failure.errors[0].message,
                        ),
                    )
                })?;
        }

        Ok(Self {
            capabilities,
            events,
            workflows,
        })
    }
}

/// Spec 119 Mode A: verified public-registry discovery and execution state.
///
/// Holds the host-owned, digest-verified registry cache and its published
/// public-metadata generation. All discovery reads come from this generation;
/// every execution resolves the exact digest-verified WASM artifact from the
/// same cache. There is no expedition, private, in-process, or network
/// fallback (FR-001, FR-003, FR-004).
#[derive(Debug)]
pub struct ModeAContext {
    cache: HostRegistryCache,
    metadata: PublicMetadataRead,
}

impl ModeAContext {
    /// Load Mode A state from a host-owned verified registry cache root.
    ///
    /// # Errors
    ///
    /// Returns a stable `registry_sync_missing` / `registry_metadata_cache_invalid`
    /// failure when the prepared verified state is absent, malformed, or carries
    /// invalid verification bindings. A prepared-but-empty generation is valid
    /// state and yields an empty catalog rather than an error.
    pub fn load(cache_root: impl Into<PathBuf>) -> Result<Self, StdioServerFailure> {
        let cache = HostRegistryCache::new(cache_root);
        let metadata = read_public_metadata(&cache).map_err(mode_a_cache_failure)?;
        Ok(Self { cache, metadata })
    }

    /// Exact-identity lookup against the verified public-metadata generation.
    fn record(&self, id: &str, version: &str) -> Option<&PublicCapabilityMetadata> {
        self.metadata
            .records
            .iter()
            .find(|record| record.id == id && record.version == version)
    }
}

/// Map a verified-registry-cache failure to a stable, secret-free stdio failure.
fn mode_a_cache_failure(error: RegistryCacheError) -> StdioServerFailure {
    StdioServerFailure::new(error.code.as_str(), error.message)
}

#[derive(Debug)]
pub struct TraverseMcpStdioServer<'a, E> {
    mcp: &'a TraverseMcp<'a, E>,
    catalog: &'a McpDiscoveryCatalog,
    public_metadata_cache: Option<HostRegistryCache>,
    mode_a: Option<&'a ModeAContext>,
}

impl<'a, E> TraverseMcpStdioServer<'a, E>
where
    E: LocalExecutor,
{
    #[must_use]
    pub fn new(mcp: &'a TraverseMcp<'a, E>, catalog: &'a McpDiscoveryCatalog) -> Self {
        Self {
            mcp,
            catalog,
            public_metadata_cache: None,
            mode_a: None,
        }
    }

    #[must_use]
    pub fn with_public_metadata_cache(mut self, cache: HostRegistryCache) -> Self {
        self.public_metadata_cache = Some(cache);
        self
    }

    /// Engage Spec 119 Mode A: discovery and execution are served exclusively
    /// from the supplied verified public-registry state.
    #[must_use]
    pub fn with_mode_a(mut self, context: &'a ModeAContext) -> Self {
        self.mode_a = Some(context);
        self
    }

    /// The governing spec reported in envelopes — Mode A when engaged.
    fn governing_spec(&self) -> &'static str {
        if self.mode_a.is_some() {
            MODE_A_GOVERNING_SPEC
        } else {
            GOVERNING_SPEC
        }
    }

    /// The active discovery/execution mode label.
    fn mode_label(&self) -> &'static str {
        if self.mode_a.is_some() {
            "verified_public"
        } else {
            "expedition_contributor"
        }
    }

    fn search_capabilities_envelope(&self, query: &str) -> Result<Value, StdioServerFailure> {
        let cache = self
            .mode_a
            .map(|context| &context.cache)
            .or(self.public_metadata_cache.as_ref())
            .ok_or_else(|| {
                StdioServerFailure::new(
                    "registry_sync_missing",
                    "search requires verified public registry state.",
                )
            })?;
        let result = crate::tools::capabilities::search_capabilities(cache, query)
            .map_err(|error| StdioServerFailure::new(&error.message, &error.message))?;
        Ok(
            json!({"kind":"mcp_capability_search", "records": result.records, "stale": result.stale}),
        )
    }

    #[must_use]
    pub fn startup_envelope(&self) -> Value {
        json!({
            "kind": "mcp_stdio_server_startup",
            "server_name": SERVER_NAME,
            "host_mode": HOST_MODE,
            "governing_spec": self.governing_spec(),
            "mode": self.mode_label(),
            "status": "ready",
            "supported_commands": SUPPORTING_COMMANDS,
            "public_surface_id": PUBLIC_SURFACE_ID,
            "auth_boundary": {
                "default_mode": "local_trust",
                "required_token_env": "TRAVERSE_MCP_STDIO_BEARER_TOKEN",
                "required_commands": ["execute_entrypoint", "render_execution_report"],
            },
            "discovery_source": self.discovery_source_summary(),
            "content_group_count": self.content_group_count(),
        })
    }

    /// Provenance of the active discovery surface.
    fn discovery_source_summary(&self) -> Value {
        match self.mode_a {
            Some(context) => json!({
                "kind": "host_verified_public_registry",
                "source_release": context.metadata.source_release,
                "index_digest": context.metadata.index_digest,
                "stale": context.metadata.stale,
                "capability_count": context.metadata.records.len(),
            }),
            None => json!({
                "kind": "bundled_expedition_example",
                "note": "contributor-only source-run catalog; not a Mode A product surface",
            }),
        }
    }

    /// Content-group count — always zero under Mode A (FR-007: no hard-coded or
    /// inferred content groups before registry-governed grouping metadata).
    fn content_group_count(&self) -> usize {
        if self.mode_a.is_some() {
            0
        } else {
            McpDiscoveryCatalog::content_group_count()
        }
    }

    /// Content-group summaries — empty under Mode A (FR-007).
    fn content_group_summaries(&self) -> Value {
        if self.mode_a.is_some() {
            json!([])
        } else {
            json!(McpDiscoveryCatalog::content_group_summaries())
        }
    }

    #[must_use]
    pub fn describe_envelope(&self) -> Value {
        let validation_path = youaskm3_mcp_consumption_validation_path();
        let governed_surface_counts = match self.mode_a {
            Some(context) => json!({
                "capabilities": context.metadata.records.len(),
                "events": 0,
                "workflows": 0,
            }),
            None => json!({
                "capabilities": self.catalog.capability_count(),
                "events": self.catalog.event_count(),
                "workflows": self.catalog.workflow_count(),
            }),
        };
        json!({
            "kind": "mcp_stdio_server_description",
            "server_name": SERVER_NAME,
            "host_mode": HOST_MODE,
            "governing_spec": self.governing_spec(),
            "mode": self.mode_label(),
            "runtime_authority": "Traverse runtime authority",
            "public_surface_id": PUBLIC_SURFACE_ID,
            "supported_commands": SUPPORTING_COMMANDS,
            "auth_boundary": {
                "default_mode": "local_trust",
                "required_token_env": "TRAVERSE_MCP_STDIO_BEARER_TOKEN",
                "required_commands": ["execute_entrypoint", "render_execution_report"],
                "token_output_policy": "never_echo_raw_token",
            },
            "discovery_source": self.discovery_source_summary(),
            "governed_surface_counts": governed_surface_counts,
            "content_groups": self.content_group_summaries(),
            "downstream_validation_path": {
                "consumer_name": validation_path.consumer_name,
                "validated_flow_id": validation_path.validated_flow_id,
                "public_surface_id": validation_path.public_surface_id,
                "governing_specs": validation_path.governing_specs,
            },
        })
    }

    #[must_use]
    pub fn list_entrypoints_envelope(&self) -> Value {
        if let Some(context) = self.mode_a {
            return self.mode_a_list_entrypoints_envelope(context);
        }
        let capability_entries = self
            .catalog
            .bundle
            .capabilities
            .iter()
            .map(capability_entrypoint_summary)
            .collect::<Vec<_>>();
        let event_entries = self
            .catalog
            .bundle
            .events
            .iter()
            .map(event_entrypoint_summary)
            .collect::<Vec<_>>();
        let workflow_entries = self
            .catalog
            .bundle
            .workflows
            .iter()
            .map(workflow_entrypoint_summary)
            .collect::<Vec<_>>();

        json!({
            "kind": "mcp_stdio_server_entrypoint_list",
            "server_name": SERVER_NAME,
            "host_mode": HOST_MODE,
            "governing_spec": self.governing_spec(),
            "content_groups": McpDiscoveryCatalog::content_group_summaries(),
            "entrypoints": {
                "capabilities": capability_entries,
                "events": event_entries,
                "workflows": workflow_entries,
            },
        })
    }

    #[must_use]
    pub fn list_content_groups_envelope(&self) -> Value {
        json!({
            "kind": "mcp_stdio_server_content_group_list",
            "server_name": SERVER_NAME,
            "host_mode": HOST_MODE,
            "governing_spec": self.governing_spec(),
            "mode": self.mode_label(),
            "content_groups": self.content_group_summaries(),
        })
    }

    /// # Errors
    ///
    /// Returns `invalid_request` when the content group id is missing or unsupported.
    pub fn describe_content_group_envelope(
        &self,
        content_group_id: &str,
    ) -> Result<Value, StdioServerFailure> {
        if self.mode_a.is_some() {
            // FR-007: Mode A exposes no content groups until registry-governed
            // grouping metadata exists.
            return Err(not_found("content group", content_group_id, "1.0.0"));
        }
        McpDiscoveryCatalog::content_group_detail(content_group_id)
            .map(|content_group| {
                json!({
                    "kind": "mcp_stdio_server_content_group_description",
                    "server_name": SERVER_NAME,
                    "host_mode": HOST_MODE,
                    "governing_spec": self.governing_spec(),
                    "content_group": content_group,
                })
            })
            .ok_or_else(|| not_found("content group", content_group_id, "1.0.0"))
    }

    /// # Errors
    ///
    /// Returns `invalid_request` when the entrypoint kind is unsupported or the id/version is malformed.
    /// Returns `not_found` when the requested entrypoint does not exist in the canonical bundle.
    pub fn describe_entrypoint_envelope(
        &self,
        entrypoint_kind: &str,
        id: &str,
        version: &str,
    ) -> Result<Value, StdioServerFailure> {
        if let Some(context) = self.mode_a {
            return self.mode_a_describe_entrypoint_envelope(context, entrypoint_kind, id, version);
        }
        match entrypoint_kind {
            "capability" => self
                .catalog
                .bundle
                .capabilities
                .iter()
                .find(|artifact| artifact.contract.id == id && artifact.contract.version == version)
                .map(|artifact| {
                    json!({
                        "kind": "mcp_stdio_server_entrypoint_description",
                        "server_name": SERVER_NAME,
                        "host_mode": HOST_MODE,
                        "governing_spec": self.governing_spec(),
                        "entrypoint": capability_entrypoint_detail(artifact),
                    })
                })
                .ok_or_else(|| not_found("capability entrypoint", id, version)),
            "workflow" => self
                .catalog
                .bundle
                .workflows
                .iter()
                .find(|artifact| {
                    artifact.definition.id == id && artifact.definition.version == version
                })
                .map(|artifact| {
                    json!({
                        "kind": "mcp_stdio_server_entrypoint_description",
                        "server_name": SERVER_NAME,
                        "host_mode": HOST_MODE,
                        "governing_spec": self.governing_spec(),
                        "entrypoint": workflow_entrypoint_detail(artifact),
                    })
                })
                .ok_or_else(|| not_found("workflow entrypoint", id, version)),
            other => Err(StdioServerFailure::new(
                "invalid_request",
                format!("Unsupported entrypoint_kind: {other}"),
            )),
        }
    }

    fn validate_entrypoint_envelope(
        &self,
        command: &StdioCommandEnvelope,
    ) -> Result<Value, StdioServerFailure> {
        if let Some(context) = self.mode_a {
            return self.mode_a_run(context, command, ModeAAction::Validate);
        }
        let artifacts = self.entrypoint_artifacts(command)?;
        Ok(json!({
            "kind": "mcp_stdio_server_entrypoint_validation",
            "server_name": SERVER_NAME,
            "host_mode": HOST_MODE,
            "governing_spec": self.governing_spec(),
            "status": "valid",
            "request_path": artifacts.request_path,
            "request_source": artifacts.request_source,
            "entrypoint": artifacts.entrypoint,
            "request": runtime_request_summary(&artifacts.request),
        }))
    }

    fn execute_entrypoint_envelope(
        &self,
        command: &StdioCommandEnvelope,
        auth_config: &StdioAuthConfig,
    ) -> Result<Value, StdioServerFailure> {
        auth_config.verify_execute_command(command)?;
        if let Some(context) = self.mode_a {
            return self.mode_a_run(context, command, ModeAAction::Execute { auth_config });
        }
        let artifacts = self.entrypoint_artifacts(command)?;
        let response = self
            .mcp
            .execute(artifacts.request)
            .map_err(|error| StdioServerFailure::new("execution_failed", format!("{error:?}")))?;
        let result = response.result.clone();
        let trace = response.trace.clone();
        let request_id = result.request_id.clone();
        let execution_id = result.execution_id.clone();
        let observation_messages = response
            .observation_messages
            .into_iter()
            .map(observation_message_summary)
            .collect::<Vec<_>>();

        Ok(json!({
            "kind": "mcp_stdio_server_entrypoint_execution",
            "server_name": SERVER_NAME,
            "host_mode": HOST_MODE,
            "governing_spec": self.governing_spec(),
            "status": "completed",
            "request_path": artifacts.request_path,
            "request_source": artifacts.request_source,
            "entrypoint": artifacts.entrypoint,
            "request_id": request_id,
            "execution_id": execution_id,
            "result": result,
            "trace": public_trace_summary(&trace),
            "trace_redaction": trace_redaction_policy(auth_config),
            "observation_messages": observation_messages,
        }))
    }

    fn render_execution_report_envelope(
        &self,
        command: &StdioCommandEnvelope,
        auth_config: &StdioAuthConfig,
    ) -> Result<Value, StdioServerFailure> {
        auth_config.verify_execute_command(command)?;
        if let Some(context) = self.mode_a {
            return self.mode_a_run(context, command, ModeAAction::Report { auth_config });
        }
        let artifacts = self.entrypoint_artifacts(command)?;
        let response = self
            .mcp
            .execute(artifacts.request)
            .map_err(|error| StdioServerFailure::new("execution_failed", format!("{error:?}")))?;
        let result = response.result.clone();
        let trace = response.trace.clone();
        let request_id = result.request_id.clone();
        let execution_id = result.execution_id.clone();
        let observation_messages = response
            .observation_messages
            .into_iter()
            .map(observation_message_summary)
            .collect::<Vec<_>>();

        Ok(json!({
            "kind": "mcp_stdio_server_execution_report",
            "server_name": SERVER_NAME,
            "host_mode": HOST_MODE,
            "governing_spec": self.governing_spec(),
            "status": "rendered",
            "request_path": artifacts.request_path,
            "request_source": artifacts.request_source,
            "entrypoint": artifacts.entrypoint,
            "execution": {
                "request_id": request_id.clone(),
                "execution_id": execution_id.clone(),
                "result": result,
                "trace": public_trace_summary(&trace),
                "trace_redaction": trace_redaction_policy(auth_config),
                "observation_messages": observation_messages,
            },
            "report": {
                "summary": "Rendered execution report from governed runtime output",
                "execution_id": execution_id,
                "request_id": request_id,
                "result_status": result.status,
                "trace_kind": trace.kind,
                "trace_redacted": true,
                "observation_message_count": observation_messages.len(),
            },
        }))
    }

    // --- Spec 119 Mode A: verified public-registry discovery and execution ---

    fn mode_a_list_entrypoints_envelope(&self, context: &ModeAContext) -> Value {
        let capabilities = context
            .metadata
            .records
            .iter()
            .map(mode_a_entrypoint_summary)
            .collect::<Vec<_>>();
        json!({
            "kind": "mcp_stdio_server_entrypoint_list",
            "server_name": SERVER_NAME,
            "host_mode": HOST_MODE,
            "governing_spec": self.governing_spec(),
            "mode": self.mode_label(),
            "discovery_source": self.discovery_source_summary(),
            "content_groups": [],
            "entrypoints": {
                "capabilities": capabilities,
                "events": [],
                "workflows": [],
            },
        })
    }

    fn mode_a_describe_entrypoint_envelope(
        &self,
        context: &ModeAContext,
        entrypoint_kind: &str,
        id: &str,
        version: &str,
    ) -> Result<Value, StdioServerFailure> {
        if entrypoint_kind != "capability" {
            return Err(StdioServerFailure::new(
                "invalid_request",
                format!(
                    "Mode A exposes verified capability entrypoints only; got entrypoint_kind {entrypoint_kind}"
                ),
            ));
        }
        let record = context
            .record(id, version)
            .ok_or_else(|| not_found("capability entrypoint", id, version))?;
        Ok(json!({
            "kind": "mcp_stdio_server_entrypoint_description",
            "server_name": SERVER_NAME,
            "host_mode": HOST_MODE,
            "governing_spec": self.governing_spec(),
            "mode": self.mode_label(),
            "entrypoint": mode_a_entrypoint_detail(record),
        }))
    }

    /// Shared validate / execute / render-report path for Mode A. Discovery,
    /// artifact selection, and execution all come from the verified public
    /// state and its digest-verified cache — never the expedition bundle.
    #[allow(clippy::too_many_lines)]
    fn mode_a_run(
        &self,
        context: &ModeAContext,
        command: &StdioCommandEnvelope,
        action: ModeAAction<'_>,
    ) -> Result<Value, StdioServerFailure> {
        let entrypoint_kind = command.entrypoint_kind.as_deref().ok_or_else(|| {
            StdioServerFailure::new("invalid_request", "command requires entrypoint_kind.")
        })?;
        if entrypoint_kind != "capability" {
            return Err(StdioServerFailure::new(
                "invalid_request",
                "Mode A executes verified capability entrypoints only.",
            ));
        }
        let id = command
            .id
            .as_deref()
            .ok_or_else(|| StdioServerFailure::new("invalid_request", "command requires id."))?;
        let version = command.version.as_deref().ok_or_else(|| {
            StdioServerFailure::new("invalid_request", "command requires version.")
        })?;

        let RequestInput {
            request,
            request_path,
            request_source,
        } = resolve_runtime_request_input(command)?;
        self.validate_runtime_request("capability", id, version, &request)?;

        // Discovery guard (FR-001): the target must be present in the verified
        // public-metadata generation.
        let record = context
            .record(id, version)
            .ok_or_else(|| not_found("capability entrypoint", id, version))?;

        // Artifact selection (FR-004): resolve the exact digest-verified WASM
        // and contract from the host-owned cache. Offline only; fails closed.
        let reference = RegistryReference {
            namespace: record.namespace.clone(),
            id: id.to_string(),
            version_range: format!("={version}"),
        };
        let component =
            resolve_registry_component(&context.cache, &reference).map_err(mode_a_cache_failure)?;
        if component.contract.id != id || component.contract.version != version {
            return Err(StdioServerFailure::new(
                "invalid_request",
                format!(
                    "verified artifact identity {}@{} does not match requested entrypoint {id}@{version}",
                    component.contract.id, component.contract.version
                ),
            ));
        }

        let entrypoint = mode_a_entrypoint_detail(record);
        let artifact = mode_a_artifact_summary(context, record, &component);

        match action {
            ModeAAction::Validate => Ok(json!({
                "kind": "mcp_stdio_server_entrypoint_validation",
                "server_name": SERVER_NAME,
                "host_mode": HOST_MODE,
                "governing_spec": self.governing_spec(),
                "mode": self.mode_label(),
                "status": "valid",
                "request_path": request_path,
                "request_source": request_source,
                "entrypoint": entrypoint,
                "artifact": artifact,
                "request": runtime_request_summary(&request),
            })),
            ModeAAction::Execute { auth_config } => {
                let response = Self::mode_a_execute_component(&component, request)?;
                let ExecutionParts {
                    result,
                    trace,
                    request_id,
                    execution_id,
                    observation_messages,
                } = ExecutionParts::from_response(response);
                Ok(json!({
                    "kind": "mcp_stdio_server_entrypoint_execution",
                    "server_name": SERVER_NAME,
                    "host_mode": HOST_MODE,
                    "governing_spec": self.governing_spec(),
                    "mode": self.mode_label(),
                    "status": "completed",
                    "request_path": request_path,
                    "request_source": request_source,
                    "entrypoint": entrypoint,
                    "artifact": artifact,
                    "request_id": request_id,
                    "execution_id": execution_id,
                    "result": result,
                    "trace": public_trace_summary(&trace),
                    "trace_redaction": trace_redaction_policy(auth_config),
                    "observation_messages": observation_messages,
                }))
            }
            ModeAAction::Report { auth_config } => {
                let response = Self::mode_a_execute_component(&component, request)?;
                let ExecutionParts {
                    result,
                    trace,
                    request_id,
                    execution_id,
                    observation_messages,
                } = ExecutionParts::from_response(response);
                Ok(json!({
                    "kind": "mcp_stdio_server_execution_report",
                    "server_name": SERVER_NAME,
                    "host_mode": HOST_MODE,
                    "governing_spec": self.governing_spec(),
                    "mode": self.mode_label(),
                    "status": "rendered",
                    "request_path": request_path,
                    "request_source": request_source,
                    "entrypoint": entrypoint,
                    "artifact": artifact,
                    "execution": {
                        "request_id": request_id.clone(),
                        "execution_id": execution_id.clone(),
                        "result": result,
                        "trace": public_trace_summary(&trace),
                        "trace_redaction": trace_redaction_policy(auth_config),
                        "observation_messages": observation_messages,
                    },
                    "report": {
                        "summary": "Rendered execution report from governed runtime output",
                        "execution_id": execution_id,
                        "request_id": request_id,
                        "result_status": result.status,
                        "trace_kind": trace.kind,
                        "trace_redacted": true,
                        "observation_message_count": observation_messages.len(),
                    },
                }))
            }
        }
    }

    /// Execute one verified component through a freshly built runtime whose only
    /// registered capability is the digest-verified artifact. Uses the real
    /// [`ArtifactRouter`] WASM executor — never the expedition Rust executor
    /// (FR-004).
    fn mode_a_execute_component(
        component: &ResolvedRegistryComponent,
        request: RuntimeRequest,
    ) -> Result<crate::McpExecutionResponse, StdioServerFailure> {
        let registration = mode_a_capability_registration(component);
        let mut capability_registry = CapabilityRegistry::new();
        capability_registry
            .register(registration)
            .map_err(|failure| {
                StdioServerFailure::new(
                    "registry_registration_failed",
                    failure
                        .errors
                        .first()
                        .map_or("verified capability registration failed", |error| {
                            error.message.as_str()
                        }),
                )
            })?;

        let router = ArtifactRouter::new().map_err(|failure| {
            StdioServerFailure::new("execution_failed", format!("{failure:?}"))
        })?;
        // v0.1.0 posture: the Spec 080 verified registry cache is digest-based
        // and carries no artifact signature, so execution runs in development
        // security mode with the digest checked twice (cache resolve + WASM
        // executor checksum). Ed25519 provenance is a tracked follow-up.
        let runtime = Runtime::new(capability_registry, router)
            .with_security_config(RuntimeSecurityConfig::development());
        let event_registry = EventRegistry::new();
        let workflow_registry = WorkflowRegistry::new();
        let discovery_registry = CapabilityRegistry::new();
        let mcp = TraverseMcp::new(
            &discovery_registry,
            &event_registry,
            &workflow_registry,
            &runtime,
        );
        mcp.execute(request)
            .map_err(|error| StdioServerFailure::new("execution_failed", format!("{error:?}")))
    }

    fn entrypoint_artifacts(
        &self,
        command: &StdioCommandEnvelope,
    ) -> Result<EntrypointArtifacts, StdioServerFailure> {
        let entrypoint_kind = command.entrypoint_kind.as_deref().ok_or_else(|| {
            StdioServerFailure::new("invalid_request", "command requires entrypoint_kind.")
        })?;
        let id = command
            .id
            .as_deref()
            .ok_or_else(|| StdioServerFailure::new("invalid_request", "command requires id."))?;
        let version = command.version.as_deref().ok_or_else(|| {
            StdioServerFailure::new("invalid_request", "command requires version.")
        })?;

        let RequestInput {
            request,
            request_path,
            request_source,
        } = resolve_runtime_request_input(command)?;
        self.validate_runtime_request(entrypoint_kind, id, version, &request)?;

        Ok(EntrypointArtifacts {
            request_path,
            request_source,
            entrypoint: self.describe_entrypoint_envelope(entrypoint_kind, id, version)?,
            request,
        })
    }

    fn validate_runtime_request(
        &self,
        entrypoint_kind: &str,
        id: &str,
        version: &str,
        request: &RuntimeRequest,
    ) -> Result<(), StdioServerFailure> {
        match entrypoint_kind {
            "capability" => {
                let Some(capability_id) = request.intent.capability_id.as_deref() else {
                    return Err(StdioServerFailure::new(
                        "invalid_request",
                        "runtime request must include intent.capability_id for capability entrypoints.",
                    ));
                };
                let Some(capability_version) = request.intent.capability_version.as_deref() else {
                    return Err(StdioServerFailure::new(
                        "invalid_request",
                        "runtime request must include intent.capability_version for capability entrypoints.",
                    ));
                };

                if capability_id != id || capability_version != version {
                    return Err(StdioServerFailure::new(
                        "invalid_request",
                        format!(
                            "runtime request target {capability_id}@{capability_version} does not match capability entrypoint {id}@{version}"
                        ),
                    ));
                }
            }
            "workflow" => {
                let Some(capability_id) = request.intent.capability_id.as_deref() else {
                    return Err(StdioServerFailure::new(
                        "invalid_request",
                        "runtime request must include intent.capability_id for workflow entrypoints.",
                    ));
                };
                let Some(capability_version) = request.intent.capability_version.as_deref() else {
                    return Err(StdioServerFailure::new(
                        "invalid_request",
                        "runtime request must include intent.capability_version for workflow entrypoints.",
                    ));
                };

                let Some(workflow) = self.catalog.bundle.workflows.iter().find(|artifact| {
                    artifact.definition.id == id && artifact.definition.version == version
                }) else {
                    return Err(not_found("workflow entrypoint", id, version));
                };

                let _ = workflow;
                if capability_id != id || capability_version != version {
                    return Err(StdioServerFailure::new(
                        "invalid_request",
                        format!(
                            "runtime request target {capability_id}@{capability_version} does not match workflow entrypoint {id}@{version}"
                        ),
                    ));
                }
            }
            other => {
                return Err(StdioServerFailure::new(
                    "invalid_request",
                    format!("Unsupported entrypoint_kind: {other}"),
                ));
            }
        }

        Ok(())
    }

    #[must_use]
    pub fn shutdown_envelope(&self, reason: &str) -> Value {
        json!({
            "kind": "mcp_stdio_server_shutdown",
            "server_name": SERVER_NAME,
            "host_mode": HOST_MODE,
            "governing_spec": self.governing_spec(),
            "status": "complete",
            "reason": reason,
        })
    }

    #[allow(clippy::too_many_lines)]
    /// # Errors
    ///
    /// Returns `io_error` when writing or reading stdio fails.
    /// Returns `invalid_request` when a command envelope omits required fields.
    /// Returns `unsupported_command` when the command name is not recognized.
    pub fn run_stdio<R, W, EWrite>(
        &self,
        input: R,
        stdout: &mut W,
        stderr: &mut EWrite,
        simulate_startup_failure: bool,
    ) -> Result<(), StdioServerFailure>
    where
        R: BufRead,
        W: Write,
        EWrite: Write,
    {
        self.run_stdio_with_auth(
            input,
            stdout,
            stderr,
            simulate_startup_failure,
            &StdioAuthConfig::local_trust(),
        )
    }

    #[allow(clippy::too_many_lines)]
    /// # Errors
    ///
    /// Returns `auth_required` when an execution command is submitted without the
    /// required local bearer token.
    pub fn run_stdio_with_auth<R, W, EWrite>(
        &self,
        input: R,
        stdout: &mut W,
        stderr: &mut EWrite,
        simulate_startup_failure: bool,
        auth_config: &StdioAuthConfig,
    ) -> Result<(), StdioServerFailure>
    where
        R: BufRead,
        W: Write,
        EWrite: Write,
    {
        if simulate_startup_failure {
            let failure = StdioServerFailure::new(
                "startup_failed",
                "Simulated startup failure for deterministic validation.",
            );
            write_json_line(stderr, &failure.envelope()).map_err(|error| {
                StdioServerFailure::new(
                    "io_error",
                    format!("Failed to write startup failure envelope: {error}"),
                )
            })?;
            return Err(failure);
        }

        write_json_line(stdout, &self.startup_envelope()).map_err(|error| {
            StdioServerFailure::new(
                "io_error",
                format!("Failed to write startup envelope: {error}"),
            )
        })?;

        for line in input.lines() {
            let line = line.map_err(|error| {
                StdioServerFailure::new(
                    "io_error",
                    format!("Failed to read stdio command line: {error}"),
                )
            })?;

            if line.trim().is_empty() {
                continue;
            }

            let command = match parse_command(&line) {
                Ok(command) => command,
                Err(failure) => {
                    let _ = write_json_line(stderr, &failure.envelope());
                    return Err(failure);
                }
            };
            match command.command.as_str() {
                "describe_server" | "describe" => {
                    write_json_line(stdout, &self.describe_envelope()).map_err(|error| {
                        StdioServerFailure::new(
                            "io_error",
                            format!("Failed to write server description envelope: {error}"),
                        )
                    })?;
                }
                "list_content_groups" => {
                    write_json_line(stdout, &self.list_content_groups_envelope()).map_err(
                        |error| {
                            StdioServerFailure::new(
                                "io_error",
                                format!("Failed to write content group list envelope: {error}"),
                            )
                        },
                    )?;
                }
                "describe_content_group" => {
                    let Some(content_group_id) = command.content_group_id.as_deref() else {
                        let failure = StdioServerFailure::new(
                            "invalid_request",
                            "describe_content_group requires content_group_id.",
                        );
                        let _ = write_json_line(stderr, &failure.envelope());
                        return Err(failure);
                    };

                    let envelope = self.describe_content_group_envelope(content_group_id)?;
                    write_json_line(stdout, &envelope).map_err(|error| {
                        StdioServerFailure::new(
                            "io_error",
                            format!("Failed to write content group description envelope: {error}"),
                        )
                    })?;
                }
                "list_entrypoints" | "list" => {
                    write_json_line(stdout, &self.list_entrypoints_envelope()).map_err(
                        |error| {
                            StdioServerFailure::new(
                                "io_error",
                                format!("Failed to write entrypoint list envelope: {error}"),
                            )
                        },
                    )?;
                }
                "search_capabilities" => {
                    let Some(query) = command.query.as_deref() else {
                        let failure = StdioServerFailure::new(
                            "invalid_query",
                            "search_capabilities requires query.",
                        );
                        let _ = write_json_line(stderr, &failure.envelope());
                        return Err(failure);
                    };
                    let envelope = match self.search_capabilities_envelope(query) {
                        Ok(envelope) => envelope,
                        Err(failure) => {
                            let _ = write_json_line(stderr, &failure.envelope());
                            return Err(failure);
                        }
                    };
                    write_json_line(stdout, &envelope).map_err(|error| {
                        StdioServerFailure::new(
                            "io_error",
                            format!("Failed to write search envelope: {error}"),
                        )
                    })?;
                }
                "describe_entrypoint" => {
                    let Some(entrypoint_kind) = command.entrypoint_kind.as_deref() else {
                        let failure = StdioServerFailure::new(
                            "invalid_request",
                            "describe_entrypoint requires entrypoint_kind.",
                        );
                        let _ = write_json_line(stderr, &failure.envelope());
                        return Err(failure);
                    };
                    let Some(id) = command.id.as_deref() else {
                        let failure = StdioServerFailure::new(
                            "invalid_request",
                            "describe_entrypoint requires id.",
                        );
                        let _ = write_json_line(stderr, &failure.envelope());
                        return Err(failure);
                    };
                    let Some(version) = command.version.as_deref() else {
                        let failure = StdioServerFailure::new(
                            "invalid_request",
                            "describe_entrypoint requires version.",
                        );
                        let _ = write_json_line(stderr, &failure.envelope());
                        return Err(failure);
                    };

                    let envelope =
                        self.describe_entrypoint_envelope(entrypoint_kind, id, version)?;
                    write_json_line(stdout, &envelope).map_err(|error| {
                        StdioServerFailure::new(
                            "io_error",
                            format!("Failed to write entrypoint description envelope: {error}"),
                        )
                    })?;
                }
                "validate_entrypoint" => {
                    let envelope = match self.validate_entrypoint_envelope(&command) {
                        Ok(envelope) => envelope,
                        Err(failure) => {
                            let _ = write_json_line(stderr, &failure.envelope());
                            return Err(failure);
                        }
                    };
                    write_json_line(stdout, &envelope).map_err(|error| {
                        StdioServerFailure::new(
                            "io_error",
                            format!("Failed to write entrypoint validation envelope: {error}"),
                        )
                    })?;
                }
                "execute_entrypoint" => {
                    let envelope = match self.execute_entrypoint_envelope(&command, auth_config) {
                        Ok(envelope) => envelope,
                        Err(failure) => {
                            let _ = write_json_line(stderr, &failure.envelope());
                            return Err(failure);
                        }
                    };
                    write_json_line(stdout, &envelope).map_err(|error| {
                        StdioServerFailure::new(
                            "io_error",
                            format!("Failed to write entrypoint execution envelope: {error}"),
                        )
                    })?;
                }
                "render_execution_report" => {
                    let envelope =
                        match self.render_execution_report_envelope(&command, auth_config) {
                            Ok(envelope) => envelope,
                            Err(failure) => {
                                let _ = write_json_line(stderr, &failure.envelope());
                                return Err(failure);
                            }
                        };
                    write_json_line(stdout, &envelope).map_err(|error| {
                        StdioServerFailure::new(
                            "io_error",
                            format!("Failed to write execution report envelope: {error}"),
                        )
                    })?;
                }
                "shutdown" => {
                    write_json_line(stdout, &self.shutdown_envelope("shutdown_command")).map_err(
                        |error| {
                            StdioServerFailure::new(
                                "io_error",
                                format!("Failed to write shutdown envelope: {error}"),
                            )
                        },
                    )?;
                    return Ok(());
                }
                other => {
                    let failure = StdioServerFailure::new(
                        "unsupported_command",
                        format!("Unsupported stdio command: {other}"),
                    );
                    let _ = write_json_line(stderr, &failure.envelope());
                    return Err(failure);
                }
            }
        }

        write_json_line(stdout, &self.shutdown_envelope("stdin_closed")).map_err(|error| {
            StdioServerFailure::new(
                "io_error",
                format!("Failed to write shutdown envelope: {error}"),
            )
        })?;
        Ok(())
    }
}

#[derive(Debug)]
struct EntrypointArtifacts {
    /// `Some` only when the legacy `request_path` compatibility input was used;
    /// `None` for a Spec 119 FR-002 inline `request`.
    request_path: Option<String>,
    /// `"inline"` or `"request_path"` — surfaced in every response envelope so
    /// a client can confirm no filesystem materialization occurred.
    request_source: &'static str,
    entrypoint: Value,
    request: RuntimeRequest,
}

fn public_trace_summary(trace: &traverse_runtime::RuntimeTrace) -> Value {
    json!({
        "kind": trace.kind,
        "schema_version": trace.schema_version,
        "trace_id": trace.trace_id,
        "execution_id": trace.execution_id,
        "request_id": trace.request_id,
        "governing_spec": trace.governing_spec,
        "selected_capability_id": trace.selected_capability_id(),
        "runtime_status": trace.terminal_outcome.runtime_status,
        "emitted_event_count": trace.emitted_events.len(),
        "state_transition_count": trace.state_transitions.len(),
        "model_resolution_count": trace.model_resolution.len(),
        "private_fields_omitted": [
            "request",
            "decision_evidence",
            "state_progression",
            "candidate_collection",
            "selection",
            "execution",
            "result",
            "otel_trace",
        ],
    })
}

fn trace_redaction_policy(auth_config: &StdioAuthConfig) -> Value {
    json!({
        "enabled": true,
        "tier": "public_summary",
        "reason": "MCP stdio responses omit full runtime traces by default.",
        "auth_mode": auth_config.mode_name(),
    })
}

fn observation_message_summary(message: McpObservationMessage) -> Value {
    match message {
        McpObservationMessage::Lifecycle(message) => json!({
            "kind": "lifecycle",
            "sequence": message.sequence,
            "execution_id": message.execution_id,
            "request_id": message.request_id,
            "status": match message.status {
                McpLifecycleStatus::StreamStarted => "stream_started",
                McpLifecycleStatus::StreamCompleted => "stream_completed",
            },
        }),
        McpObservationMessage::State(message) => json!({
            "kind": "state",
            "sequence": message.sequence,
            "state_event": message.state_event,
        }),
        McpObservationMessage::Trace(message) => json!({
            "kind": "trace",
            "sequence": message.sequence,
            "trace": public_trace_summary(&message.trace),
            "model_resolution_count": message.model_resolution.len(),
            "trace_redacted": true,
        }),
        McpObservationMessage::Terminal(message) => json!({
            "kind": "terminal",
            "sequence": message.sequence,
            "result": message.result,
        }),
    }
}

/// # Errors
///
/// Returns `catalog_load_failed` when the canonical expedition bundle cannot be loaded.
pub fn run_stdio_server(
    simulate_startup_failure: bool,
    cache_root: Option<PathBuf>,
) -> Result<(), StdioServerFailure> {
    let canonical_execution = CanonicalExecutionContext::load_canonical()?;
    let catalog = McpDiscoveryCatalog::load_canonical()?;

    let capability_registry = Box::leak(Box::new(CapabilityRegistry::new()));
    let event_registry = Box::leak(Box::new(canonical_execution.events));
    let workflow_registry = Box::leak(Box::new(WorkflowRegistry::new()));

    // The stdio server only executes the canonical bundled expedition example,
    // whose artifacts are unsigned local-dev bundles (SourceKind::Local); the
    // development security mode allows them with a warning, matching every
    // other example-executing runtime in this crate and the CLI (spec 030
    // FR-013 keeps production the default for embedders).
    let runtime = Box::leak(Box::new(
        Runtime::new(canonical_execution.capabilities, ExpeditionExampleExecutor)
            .with_workflow_registry(canonical_execution.workflows)
            .with_security_config(traverse_runtime::security::RuntimeSecurityConfig::development()),
    ));
    let mcp = Box::leak(Box::new(TraverseMcp::new(
        capability_registry,
        event_registry,
        workflow_registry,
        runtime,
    )));
    let catalog = Box::leak(Box::new(catalog));
    let mut server = TraverseMcpStdioServer::new(mcp, catalog);

    let stdin = io::stdin();
    let stdout = io::stdout();
    let stderr = io::stderr();

    let mut stdout = stdout.lock();
    let mut stderr = stderr.lock();

    // Spec 119: engage Mode A when the host names a verified registry cache.
    // Loading fails closed — a missing or invalid prepared state stops startup
    // with a stable error envelope rather than silently falling back to the
    // expedition catalog (FR-003).
    let cache_root = cache_root.or_else(|| std::env::var_os(MODE_A_CACHE_ENV).map(PathBuf::from));
    if let Some(cache_root) = cache_root {
        match ModeAContext::load(cache_root) {
            Ok(context) => {
                server = server.with_mode_a(Box::leak(Box::new(context)));
            }
            Err(failure) => {
                let _ = write_json_line(&mut stderr, &failure.envelope());
                return Err(failure);
            }
        }
    }
    server.run_stdio_with_auth(
        stdin.lock(),
        &mut stdout,
        &mut stderr,
        simulate_startup_failure,
        &StdioAuthConfig::from_env(),
    )
}

#[derive(Debug, Default, Clone, Copy)]
struct ExpeditionExampleExecutor;

impl LocalExecutor for ExpeditionExampleExecutor {
    fn execute(
        &self,
        capability: &traverse_registry::ResolvedCapability,
        input: &Value,
    ) -> Result<traverse_runtime::LocalExecutionOutput, traverse_runtime::LocalExecutionFailure>
    {
        let value = match capability.contract.id.as_str() {
            "expedition.planning.capture-expedition-objective" => {
                execute_capture_expedition_objective(input)
            }
            "expedition.planning.interpret-expedition-intent" => {
                execute_interpret_expedition_intent(input)
            }
            "expedition.planning.assess-conditions-summary" => {
                execute_assess_conditions_summary(input)
            }
            "expedition.planning.validate-team-readiness" => execute_validate_team_readiness(input),
            "expedition.planning.assemble-expedition-plan" => {
                execute_assemble_expedition_plan(input)
            }
            other => Err(executor_failure(&format!(
                "unsupported expedition capability for stdio execution: {other}"
            ))),
        }?;
        Ok(traverse_runtime::LocalExecutionOutput {
            value,
            emitted_events: Vec::new(),
        })
    }
}

fn build_capability_registration(
    bundle: &RegistryBundle,
    capability: &traverse_registry::CapabilityBundleArtifact,
) -> Result<CapabilityRegistration, StdioServerFailure> {
    let raw_contract = read_text_file(&capability.path, "capability contract")?;
    let envelope = serde_json::from_str::<Value>(&raw_contract).map_err(|error| {
        StdioServerFailure::new(
            "invalid_request",
            format!(
                "failed to parse capability registration metadata {}: {error}",
                capability.path.display()
            ),
        )
    })?;
    let implementation_kind = derive_implementation_kind(envelope.get("composability"));
    let workflow_ref = derive_workflow_ref(envelope.get("composability"))?;
    let composability =
        derive_composability_metadata(implementation_kind, workflow_ref.as_ref(), capability)?;
    let artifact = build_capability_artifact(bundle, capability, implementation_kind, workflow_ref);

    Ok(CapabilityRegistration {
        scope: bundle.scope,
        contract: capability.contract.clone(),
        contract_path: capability.path.display().to_string(),
        artifact,
        registered_at: bundle_registered_at(bundle),
        tags: Vec::new(),
        composability,
        governing_spec: "005-capability-registry".to_string(),
        validator_version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

fn build_capability_artifact(
    bundle: &RegistryBundle,
    capability: &traverse_registry::CapabilityBundleArtifact,
    implementation_kind: ImplementationKind,
    workflow_ref: Option<WorkflowReference>,
) -> CapabilityArtifactRecord {
    CapabilityArtifactRecord {
        artifact_ref: format!(
            "bundle:{}:{}:{}",
            bundle.bundle_id, capability.contract.id, capability.contract.version
        ),
        implementation_kind,
        source: SourceReference {
            kind: SourceKind::Local,
            location: capability.path.display().to_string(),
        },
        binary: match implementation_kind {
            ImplementationKind::Executable => Some(BinaryReference {
                format: RegistryBinaryFormat::Wasm,
                location: format!(
                    "bundled://{}/{}/module.wasm",
                    capability.contract.id, capability.contract.version
                ),
                signature: None,
            }),
            ImplementationKind::Workflow => None,
        },
        workflow_ref,
        digests: traverse_registry::ArtifactDigests {
            source_digest: format!(
                "source:{}:{}",
                capability.contract.id, capability.contract.version
            ),
            binary_digest: match implementation_kind {
                ImplementationKind::Executable => Some(format!(
                    "binary:{}:{}",
                    capability.contract.id, capability.contract.version
                )),
                ImplementationKind::Workflow => None,
            },
        },
        provenance: RegistryProvenance {
            source: provenance_source_label(&capability.contract.provenance.source),
            author: capability.contract.provenance.author.clone(),
            created_at: capability.contract.provenance.created_at.clone(),
        },
    }
}

fn derive_implementation_kind(composability_value: Option<&Value>) -> ImplementationKind {
    match composability_value
        .and_then(|composability| composability.get("implementation_kind"))
        .and_then(Value::as_str)
    {
        Some("workflow") => ImplementationKind::Workflow,
        _ => ImplementationKind::Executable,
    }
}

fn derive_workflow_ref(
    composability_value: Option<&Value>,
) -> Result<Option<WorkflowReference>, StdioServerFailure> {
    composability_value
        .and_then(|composability| composability.get("workflow_ref"))
        .map(parse_workflow_ref)
        .transpose()
}

fn derive_composability_metadata(
    implementation_kind: ImplementationKind,
    workflow_ref: Option<&WorkflowReference>,
    capability: &traverse_registry::CapabilityBundleArtifact,
) -> Result<ComposabilityMetadata, StdioServerFailure> {
    let requires = capability
        .contract
        .consumes
        .iter()
        .map(|event| event.event_id.clone())
        .collect();

    match implementation_kind {
        ImplementationKind::Workflow => {
            if workflow_ref.is_none() {
                return Err(StdioServerFailure::new(
                    "invalid_request",
                    format!(
                        "workflow-backed capability {} must declare workflow_ref",
                        capability.contract.id
                    ),
                ));
            }
            Ok(ComposabilityMetadata {
                kind: CompositionKind::Composite,
                patterns: vec![CompositionPattern::Sequential],
                provides: vec![capability.contract.id.clone()],
                requires,
            })
        }
        ImplementationKind::Executable => Ok(ComposabilityMetadata {
            kind: CompositionKind::Atomic,
            patterns: vec![CompositionPattern::Sequential],
            provides: vec![capability.contract.id.clone()],
            requires,
        }),
    }
}

fn parse_workflow_ref(value: &Value) -> Result<WorkflowReference, StdioServerFailure> {
    let workflow_id = value
        .get("workflow_id")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            StdioServerFailure::new(
                "invalid_request",
                "workflow_ref.workflow_id must be a string.",
            )
        })?;
    let workflow_version = value
        .get("workflow_version")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            StdioServerFailure::new(
                "invalid_request",
                "workflow_ref.workflow_version must be a string.",
            )
        })?;
    Ok(WorkflowReference {
        workflow_id: workflow_id.to_string(),
        workflow_version: workflow_version.to_string(),
    })
}

fn load_runtime_request(request_path: &str) -> Result<RuntimeRequest, StdioServerFailure> {
    let path = resolve_relative_path(request_path);
    let contents = read_text_file(&path, "runtime request")?;
    parse_runtime_request(&contents).map_err(|error| {
        StdioServerFailure::new(
            "invalid_request",
            format!(
                "failed to parse runtime request {}: {}",
                path.display(),
                error.message
            ),
        )
    })
}

/// Resolved runtime request plus how it was supplied.
struct RequestInput {
    request: RuntimeRequest,
    /// `Some` only for the legacy `request_path` input.
    request_path: Option<String>,
    /// `"inline"` or `"request_path"`.
    request_source: &'static str,
}

/// Spec 119 FR-002: resolve exactly one of the mutually exclusive `request`
/// (inline) or `request_path` (legacy compatibility) inputs.
fn resolve_runtime_request_input(
    command: &StdioCommandEnvelope,
) -> Result<RequestInput, StdioServerFailure> {
    match (command.request.as_ref(), command.request_path.as_deref()) {
        (Some(_), Some(_)) => Err(StdioServerFailure::new(
            "invalid_request",
            "command accepts either request or request_path, not both.",
        )),
        (None, None) => Err(StdioServerFailure::new(
            "invalid_request",
            "command requires request or request_path.",
        )),
        (Some(inline), None) => Ok(RequestInput {
            request: parse_inline_runtime_request(inline)?,
            request_path: None,
            request_source: "inline",
        }),
        (None, Some(path)) => Ok(RequestInput {
            request: load_runtime_request(path)?,
            request_path: Some(path.to_string()),
            request_source: "request_path",
        }),
    }
}

/// Parse a Spec 119 FR-002 inline `request` object into a [`RuntimeRequest`],
/// applying the exact same validation as the legacy `request_path` load and
/// without any filesystem access.
fn parse_inline_runtime_request(inline: &Value) -> Result<RuntimeRequest, StdioServerFailure> {
    if !inline.is_object() {
        return Err(StdioServerFailure::new(
            "invalid_request",
            "inline request must be a JSON object serialized as a RuntimeRequest.",
        ));
    }
    let serialized = serde_json::to_string(inline).map_err(|error| {
        StdioServerFailure::new(
            "invalid_request",
            format!("failed to serialize inline runtime request: {error}"),
        )
    })?;
    parse_runtime_request(&serialized).map_err(|error| {
        StdioServerFailure::new(
            "invalid_request",
            format!("failed to parse inline runtime request: {}", error.message),
        )
    })
}

/// Which Mode A operation `mode_a_run` should perform.
#[derive(Clone, Copy)]
enum ModeAAction<'a> {
    Validate,
    Execute { auth_config: &'a StdioAuthConfig },
    Report { auth_config: &'a StdioAuthConfig },
}

/// Flattened, already-redaction-safe pieces of one runtime execution.
struct ExecutionParts {
    result: traverse_runtime::RuntimeResult,
    trace: traverse_runtime::RuntimeTrace,
    request_id: String,
    execution_id: String,
    observation_messages: Vec<Value>,
}

impl ExecutionParts {
    fn from_response(response: crate::McpExecutionResponse) -> Self {
        let result = response.result.clone();
        let trace = response.trace.clone();
        let request_id = result.request_id.clone();
        let execution_id = result.execution_id.clone();
        let observation_messages = response
            .observation_messages
            .into_iter()
            .map(observation_message_summary)
            .collect::<Vec<_>>();
        Self {
            result,
            trace,
            request_id,
            execution_id,
            observation_messages,
        }
    }
}

/// Flat discovery-list entry for one verified public capability.
fn mode_a_entrypoint_summary(record: &PublicCapabilityMetadata) -> Value {
    json!({
        "kind": "capability",
        "namespace": record.namespace,
        "id": record.id,
        "version": record.version,
        "service_type": record.service_type,
        "lifecycle": record.lifecycle,
        "summary": record.summary,
        "artifact_digest": record.artifact_digest,
    })
}

/// Redacted detail for one verified public capability. Never exposes the raw
/// contract, private records, request paths, or credentials (FR-005).
fn mode_a_entrypoint_detail(record: &PublicCapabilityMetadata) -> Value {
    json!({
        "artifact_kind": "capability",
        "namespace": record.namespace,
        "id": record.id,
        "version": record.version,
        "service_type": record.service_type,
        "permitted_targets": record.permitted_targets,
        "lifecycle": record.lifecycle,
        "summary": record.summary,
        "description": record.description,
        "scenarios": record.scenarios,
        "artifact_digest": record.artifact_digest,
        "source_release": record.source_release,
        "index_digest": record.index_digest,
        "provenance": record.provenance,
    })
}

/// Verification evidence surfaced alongside every Mode A validate/execute
/// response — the digest handed to the executor and where it was selected from.
fn mode_a_artifact_summary(
    context: &ModeAContext,
    record: &PublicCapabilityMetadata,
    component: &ResolvedRegistryComponent,
) -> Value {
    json!({
        "digest": component.wasm_digest,
        "digest_matches_public_state": component.wasm_digest == record.artifact_digest,
        "verified": true,
        "verification": "digest",
        "source_release": context.metadata.source_release,
        "index_digest": context.metadata.index_digest,
        "stale": context.metadata.stale,
    })
}

/// Build a one-capability registration bound to the digest-verified WASM path.
fn mode_a_capability_registration(component: &ResolvedRegistryComponent) -> CapabilityRegistration {
    let contract = component.contract.clone();
    let id = contract.id.clone();
    let version = contract.version.clone();
    let binary_location = component.wasm_binary_path.display().to_string();
    CapabilityRegistration {
        scope: RegistryScope::Public,
        contract_path: component.contract_path.display().to_string(),
        artifact: CapabilityArtifactRecord {
            artifact_ref: format!("verified-registry:{id}:{version}"),
            implementation_kind: ImplementationKind::Executable,
            source: SourceReference {
                kind: SourceKind::Local,
                location: binary_location.clone(),
            },
            binary: Some(BinaryReference {
                format: RegistryBinaryFormat::Wasm,
                location: binary_location,
                signature: None,
            }),
            workflow_ref: None,
            digests: ArtifactDigests {
                source_digest: format!("verified-registry-contract:{id}:{version}"),
                binary_digest: Some(component.wasm_digest.clone()),
            },
            provenance: RegistryProvenance {
                source: provenance_source_label(&contract.provenance.source),
                author: contract.provenance.author.clone(),
                created_at: contract.provenance.created_at.clone(),
            },
        },
        registered_at: format!("verified-registry:{id}@{version}"),
        tags: Vec::new(),
        composability: ComposabilityMetadata {
            kind: CompositionKind::Atomic,
            patterns: vec![CompositionPattern::Sequential],
            provides: vec![id.clone()],
            requires: Vec::new(),
        },
        governing_spec: MODE_A_GOVERNING_SPEC.to_string(),
        validator_version: env!("CARGO_PKG_VERSION").to_string(),
        contract,
    }
}

fn runtime_request_summary(runtime_request: &RuntimeRequest) -> Value {
    json!({
        "kind": runtime_request.kind,
        "schema_version": runtime_request.schema_version,
        "request_id": runtime_request.request_id,
        "governing_spec": runtime_request.governing_spec,
        "intent": {
            "capability_id": runtime_request.intent.capability_id,
            "capability_version": runtime_request.intent.capability_version,
            "intent_key": runtime_request.intent.intent_key,
        },
        "lookup": {
            "scope": runtime_request.lookup.scope,
            "allow_ambiguity": runtime_request.lookup.allow_ambiguity,
        },
        "requested_target": format!("{:?}", runtime_request.context.requested_target).to_lowercase(),
        "correlation_id": runtime_request.context.correlation_id,
        "caller": runtime_request.context.caller,
    })
}

fn capability_entrypoint_summary(artifact: &traverse_registry::CapabilityBundleArtifact) -> Value {
    let contract = &artifact.contract;
    json!({
        "artifact_kind": "capability",
        "id": contract.id,
        "version": contract.version,
        "lifecycle": format!("{:?}", contract.lifecycle).to_lowercase(),
        "summary": contract.summary,
    })
}

fn event_entrypoint_summary(artifact: &traverse_registry::EventBundleArtifact) -> Value {
    let contract = &artifact.contract;
    json!({
        "artifact_kind": "event",
        "id": contract.id,
        "version": contract.version,
        "lifecycle": format!("{:?}", contract.lifecycle).to_lowercase(),
        "summary": contract.summary,
    })
}

fn workflow_entrypoint_summary(artifact: &traverse_registry::WorkflowBundleArtifact) -> Value {
    let definition = &artifact.definition;
    json!({
        "artifact_kind": "workflow",
        "id": definition.id,
        "version": definition.version,
        "lifecycle": format!("{:?}", definition.lifecycle).to_lowercase(),
        "summary": definition.summary,
    })
}

fn capability_entrypoint_detail(artifact: &traverse_registry::CapabilityBundleArtifact) -> Value {
    let contract = &artifact.contract;
    json!({
        "artifact_kind": "capability",
        "id": contract.id,
        "version": contract.version,
        "lifecycle": format!("{:?}", contract.lifecycle).to_lowercase(),
        "summary": contract.summary,
        "owner_team": contract.owner.team,
        "artifact_path": artifact.path.display().to_string(),
    })
}

fn workflow_entrypoint_detail(artifact: &traverse_registry::WorkflowBundleArtifact) -> Value {
    let definition = &artifact.definition;
    json!({
        "artifact_kind": "workflow",
        "id": definition.id,
        "version": definition.version,
        "lifecycle": format!("{:?}", definition.lifecycle).to_lowercase(),
        "summary": definition.summary,
        "owner_team": definition.owner.team,
        "artifact_path": artifact.path.display().to_string(),
    })
}

fn write_json_line<W: Write>(writer: &mut W, value: &Value) -> io::Result<()> {
    serde_json::to_writer(&mut *writer, value)?;
    writer.write_all(b"\n")
}

fn parse_command(raw: &str) -> Result<StdioCommandEnvelope, StdioServerFailure> {
    serde_json::from_str(raw).map_err(|error| {
        StdioServerFailure::new(
            "invalid_request",
            format!("failed to parse stdio command envelope: {error}"),
        )
    })
}

fn read_text_file(path: &Path, artifact_kind: &str) -> Result<String, StdioServerFailure> {
    fs::read_to_string(path).map_err(|error| {
        StdioServerFailure::new(
            "io_error",
            format!("failed to read {artifact_kind} {}: {error}", path.display()),
        )
    })
}

fn resolve_relative_path(relative_path: &str) -> PathBuf {
    let candidate = PathBuf::from(relative_path);
    if candidate.is_absolute() {
        candidate
    } else {
        repo_root().join(candidate)
    }
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn canonical_expedition_bundle_path() -> PathBuf {
    repo_root().join("examples/expedition/registry-bundle/manifest.json")
}

fn not_found(kind: &str, id: &str, version: &str) -> StdioServerFailure {
    StdioServerFailure::new("not_found", format!("{kind} {id}@{version} was not found"))
}

fn bundle_registered_at(bundle: &RegistryBundle) -> String {
    format!("bundle:{}@{}", bundle.bundle_id, bundle.version)
}

fn provenance_source_label(source: &traverse_contracts::ProvenanceSource) -> String {
    match source {
        traverse_contracts::ProvenanceSource::Greenfield => "greenfield",
        traverse_contracts::ProvenanceSource::BrownfieldExtracted => "brownfield-extracted",
        traverse_contracts::ProvenanceSource::AiGenerated => "ai-generated",
        traverse_contracts::ProvenanceSource::AiAssisted => "ai-assisted",
    }
    .to_string()
}

impl McpDiscoveryCatalog {
    #[must_use]
    fn content_group_count() -> usize {
        Self::content_group_summaries().len()
    }

    #[must_use]
    fn content_group_summaries() -> Vec<Value> {
        vec![core_runtime_example_content_group_summary()]
    }

    fn content_group_detail(content_group_id: &str) -> Option<Value> {
        Self::content_group_summaries()
            .into_iter()
            .find(|group| group["content_group_id"].as_str() == Some(content_group_id))
    }
}

fn core_runtime_example_content_group_summary() -> Value {
    json!({
        "content_group_id": "core-runtime-example",
        "summary": "Traverse-neutral executable capability package template and local runtime shape.",
        "display_name": "Core runtime example",
        "governed_paths": [
            "examples/templates/executable-capability-package/manifest.template.json",
            "docs/executable-package-template.md",
            "docs/local-runtime-home.md",
            "scripts/ci/executable_package_template_smoke.sh"
        ],
        "validation_commands": [
            "bash scripts/ci/executable_package_template_smoke.sh"
        ],
        "invocable_entrypoints": [
            "describe_content_group"
        ],
    })
}

fn execute_capture_expedition_objective(
    input: &Value,
) -> Result<Value, traverse_runtime::LocalExecutionFailure> {
    let map = input
        .as_object()
        .ok_or_else(|| executor_failure("executor input must be an object"))?;
    let destination = required_value(map, "destination")?;
    let target_window = required_value(map, "target_window")?;
    let preferences = required_value(map, "preferences")?;
    let notes = required_value(map, "notes")?;
    let objective_id = format!("objective-{}", slug(required_string(map, "destination")?));
    let objective = serde_json::json!({
        "objective_id": objective_id,
        "destination": destination.clone(),
        "target_window": target_window.clone(),
        "preferences": preferences.clone(),
        "notes": notes.clone()
    });

    Ok(serde_json::json!({
        "objective_id": objective_id,
        "destination": destination.clone(),
        "target_window": target_window.clone(),
        "preferences": preferences.clone(),
        "notes": notes.clone(),
        "objective": objective,
        "emitted_events": [event_ref("expedition.planning.expedition-objective-captured")]
    }))
}

fn execute_interpret_expedition_intent(
    input: &Value,
) -> Result<Value, traverse_runtime::LocalExecutionFailure> {
    let map = input
        .as_object()
        .ok_or_else(|| executor_failure("executor input must be an object"))?;
    let objective = required_object(map, "objective")?;
    let objective_id = required_string(objective, "objective_id")?;
    let preferences = required_object(objective, "preferences")?;
    let style = required_string(preferences, "style")?;
    let priority = required_string(preferences, "priority")?;
    let planning_intent = required_string(map, "planning_intent")?;
    let interpreted_intent = serde_json::json!({
        "intent_id": format!("intent-{objective_id}"),
        "objective_id": objective_id,
        "route_preferences": [style, priority],
        "constraints": [format!("priority:{priority}")],
        "assumptions": [planning_intent],
        "confidence": 0.87
    });

    Ok(serde_json::json!({
        "intent_id": format!("intent-{objective_id}"),
        "objective_id": objective_id,
        "route_preferences": [style, priority],
        "constraints": [format!("priority:{priority}")],
        "assumptions": [planning_intent],
        "confidence": 0.87,
        "interpreted_intent": interpreted_intent,
        "emitted_events": [event_ref("expedition.planning.expedition-intent-interpreted")]
    }))
}

fn execute_assess_conditions_summary(
    input: &Value,
) -> Result<Value, traverse_runtime::LocalExecutionFailure> {
    let map = input
        .as_object()
        .ok_or_else(|| executor_failure("executor input must be an object"))?;
    let objective = required_object(map, "objective")?;
    let objective_id = required_string(objective, "objective_id")?;
    let destination = required_string(objective, "destination")?;
    let interpreted = required_object(map, "interpreted_intent")?;
    let route_preferences = required_string_array(interpreted, "route_preferences")?;
    let conditions_summary = serde_json::json!({
        "conditions_summary_id": format!("conditions-{objective_id}"),
        "objective_id": objective_id,
        "overall_rating": "watchful",
        "key_findings": [format!("stable morning window for {destination}"), format!("preferred style: {}", route_preferences.first().cloned().unwrap_or_else(|| "conservative".to_string()))],
        "blocking_concerns": []
    });

    Ok(serde_json::json!({
        "conditions_summary_id": format!("conditions-{objective_id}"),
        "objective_id": objective_id,
        "overall_rating": "watchful",
        "key_findings": [format!("stable morning window for {destination}"), format!("preferred style: {}", route_preferences.first().cloned().unwrap_or_else(|| "conservative".to_string()))],
        "blocking_concerns": [],
        "conditions_summary": conditions_summary,
        "emitted_events": [event_ref("expedition.planning.conditions-summary-assessed")]
    }))
}

fn execute_validate_team_readiness(
    input: &Value,
) -> Result<Value, traverse_runtime::LocalExecutionFailure> {
    let map = input
        .as_object()
        .ok_or_else(|| executor_failure("executor input must be an object"))?;
    let objective = required_object(map, "objective")?;
    let objective_id = required_string(objective, "objective_id")?;
    let team_profile = required_object(map, "team_profile")?;
    let equipment_ready = required_bool(team_profile, "equipment_ready")?;
    let status = if equipment_ready {
        "ready"
    } else {
        "needs_action"
    };
    let required_actions = if equipment_ready {
        Vec::<String>::new()
    } else {
        vec!["complete equipment verification".to_string()]
    };
    let readiness_result = serde_json::json!({
        "readiness_result_id": format!("readiness-{objective_id}"),
        "objective_id": objective_id,
        "status": status,
        "reasons": ["team profile satisfies baseline expedition requirements"],
        "required_actions": required_actions.clone()
    });

    Ok(serde_json::json!({
        "readiness_result_id": format!("readiness-{objective_id}"),
        "objective_id": objective_id,
        "status": status,
        "reasons": ["team profile satisfies baseline expedition requirements"],
        "required_actions": required_actions,
        "readiness_result": readiness_result,
        "emitted_events": [event_ref("expedition.planning.team-readiness-validated")]
    }))
}

fn execute_assemble_expedition_plan(
    input: &Value,
) -> Result<Value, traverse_runtime::LocalExecutionFailure> {
    let map = input
        .as_object()
        .ok_or_else(|| executor_failure("executor input must be an object"))?;
    let objective = required_object(map, "objective")?;
    let objective_id = required_string(objective, "objective_id")?;
    let interpreted = required_object(map, "interpreted_intent")?;
    let route_preferences = required_string_array(interpreted, "route_preferences")?;
    let constraints = required_string_array(interpreted, "constraints")?;
    let readiness = required_object(map, "readiness_result")?;
    let readiness_status = required_string(readiness, "status")?;
    let readiness_reasons = required_string_array(readiness, "reasons")?;
    let required_actions = required_string_array(readiness, "required_actions")?;
    let route_style = route_preferences
        .first()
        .cloned()
        .unwrap_or_else(|| "conservative-alpine-push".to_string());

    let mut readiness_notes = readiness_reasons;
    readiness_notes.extend(required_actions);

    Ok(serde_json::json!({
        "plan_id": format!("plan-{objective_id}"),
        "objective_id": objective_id,
        "status": if readiness_status == "ready" { "ready" } else { "requires_attention" },
        "recommended_route_style": route_style,
        "key_steps": [
            "depart before sunrise",
            "reassess winds at mid-route checkpoint",
            "apply conservative turnaround time"
        ],
        "constraints": constraints,
        "readiness_notes": readiness_notes,
        "summary": "Proceed with a conservative same-day ascent plan under a limited morning weather window.",
        "emitted_events": [event_ref("expedition.planning.expedition-plan-assembled")]
    }))
}

fn event_ref(event_id: &str) -> Value {
    json!({
        "event_id": event_id,
        "version": "1.0.0"
    })
}

fn required_object<'a>(
    map: &'a serde_json::Map<String, Value>,
    key: &str,
) -> Result<&'a serde_json::Map<String, Value>, traverse_runtime::LocalExecutionFailure> {
    map.get(key)
        .and_then(Value::as_object)
        .ok_or_else(|| executor_failure(&format!("missing object field: {key}")))
}

fn required_value<'a>(
    map: &'a serde_json::Map<String, Value>,
    key: &str,
) -> Result<&'a Value, traverse_runtime::LocalExecutionFailure> {
    map.get(key)
        .ok_or_else(|| executor_failure(&format!("missing field: {key}")))
}

fn required_string<'a>(
    map: &'a serde_json::Map<String, Value>,
    key: &str,
) -> Result<&'a str, traverse_runtime::LocalExecutionFailure> {
    map.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| executor_failure(&format!("missing string field: {key}")))
}

fn required_bool(
    map: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<bool, traverse_runtime::LocalExecutionFailure> {
    map.get(key)
        .and_then(Value::as_bool)
        .ok_or_else(|| executor_failure(&format!("missing boolean field: {key}")))
}

fn required_string_array(
    map: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<Vec<String>, traverse_runtime::LocalExecutionFailure> {
    let items = map
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| executor_failure(&format!("missing string array field: {key}")))?;

    items
        .iter()
        .map(|item| {
            item.as_str()
                .map(ToString::to_string)
                .ok_or_else(|| executor_failure(&format!("invalid string array field: {key}")))
        })
        .collect()
}

fn executor_failure(message: &str) -> traverse_runtime::LocalExecutionFailure {
    traverse_runtime::LocalExecutionFailure {
        code: traverse_runtime::LocalExecutionFailureCode::ExecutionFailed,
        message: message.to_string(),
    }
}

fn slug(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StdioServerFailure {
    code: String,
    message: String,
}

impl StdioServerFailure {
    #[must_use]
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }

    #[must_use]
    pub fn envelope(&self) -> Value {
        json!({
            "kind": "mcp_stdio_server_error",
            "server_name": SERVER_NAME,
            "host_mode": HOST_MODE,
            "governing_spec": GOVERNING_SPEC,
            "code": self.code,
            "message": self.message,
        })
    }
}

impl fmt::Display for StdioServerFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for StdioServerFailure {}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;

    #[test]
    fn canonical_bundle_load_failures_are_stable_for_catalog_and_execution() {
        let missing_manifest = std::env::temp_dir().join(format!(
            "traverse-mcp-missing-bundle-{}-manifest.json",
            std::process::id()
        ));

        for failure in [
            McpDiscoveryCatalog::load_from_manifest_path(&missing_manifest)
                .expect_err("missing catalog manifest should fail"),
            CanonicalExecutionContext::load_from_manifest_path(&missing_manifest)
                .expect_err("missing execution manifest should fail"),
        ] {
            assert_eq!(failure.code, "catalog_load_failed");
            assert!(
                failure
                    .message
                    .contains("Failed to load expedition registry bundle")
            );
        }
    }

    #[test]
    fn emits_deterministic_startup_list_validate_execute_and_shutdown_envelopes() {
        let server = build_test_server();
        let input = std::io::Cursor::new(
            br#"{"command":"describe_server"}
{"command":"list_content_groups"}
{"command":"describe_content_group","content_group_id":"core-runtime-example"}
{"command":"list_entrypoints"}
{"command":"describe_entrypoint","entrypoint_kind":"workflow","id":"expedition.planning.plan-expedition","version":"1.0.0"}
{"command":"validate_entrypoint","entrypoint_kind":"workflow","id":"expedition.planning.plan-expedition","version":"1.0.0","request_path":"examples/expedition/runtime-requests/plan-expedition.json"}
{"command":"execute_entrypoint","entrypoint_kind":"workflow","id":"expedition.planning.plan-expedition","version":"1.0.0","request_path":"examples/expedition/runtime-requests/plan-expedition.json"}
{"command":"render_execution_report","entrypoint_kind":"workflow","id":"expedition.planning.plan-expedition","version":"1.0.0","request_path":"examples/expedition/runtime-requests/plan-expedition.json"}
{"command":"shutdown"}
"#,
        );
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        assert!(
            server
                .run_stdio(input, &mut stdout, &mut stderr, false)
                .is_ok()
        );

        let output = String::from_utf8(stdout).expect("stdout must be valid UTF-8");
        assert!(output.contains("\"kind\":\"mcp_stdio_server_startup\""));
        assert!(output.contains("\"kind\":\"mcp_stdio_server_description\""));
        assert!(output.contains("\"kind\":\"mcp_stdio_server_content_group_list\""));
        assert!(output.contains("\"kind\":\"mcp_stdio_server_content_group_description\""));
        assert!(output.contains("\"content_group_id\":\"core-runtime-example\""));
        assert!(output.contains("\"kind\":\"mcp_stdio_server_entrypoint_list\""));
        assert!(output.contains("\"kind\":\"mcp_stdio_server_entrypoint_validation\""));
        assert!(output.contains("\"kind\":\"mcp_stdio_server_entrypoint_execution\""));
        assert!(output.contains("\"kind\":\"mcp_stdio_server_execution_report\""));
        assert!(output.contains("\"status\":\"rendered\""));
        assert!(output.contains("\"kind\":\"mcp_stdio_server_shutdown\""));
        assert!(stderr.is_empty());
    }

    #[test]
    fn search_command_requires_query_and_verified_public_metadata() {
        let server = build_test_server();
        let missing = server
            .search_capabilities_envelope("catalog")
            .expect_err("server without cache must fail closed");
        assert_eq!(missing.code, "registry_sync_missing");

        let cache = HostRegistryCache::new(
            std::env::temp_dir().join(format!("traverse-mcp-stdio-search-{}", std::process::id())),
        );
        let server = build_test_server().with_public_metadata_cache(cache);
        let input = std::io::Cursor::new(
            br#"{"command":"search_capabilities"}
{"command":"search_capabilities","query":"catalog"}
"#,
        );
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let error = server
            .run_stdio(input, &mut stdout, &mut stderr, false)
            .expect_err("missing query must fail");
        assert_eq!(error.code, "invalid_query");
        assert!(
            String::from_utf8(stderr)
                .expect("stderr utf8")
                .contains("invalid_query")
        );
    }

    #[test]
    fn required_stdio_bearer_token_denies_unauthenticated_execution() -> Result<(), String> {
        let server = build_test_server();
        let input = std::io::Cursor::new(
            br#"{"command":"execute_entrypoint","entrypoint_kind":"workflow","id":"expedition.planning.plan-expedition","version":"1.0.0","request_path":"examples/expedition/runtime-requests/plan-expedition.json"}
"#,
        );
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        let result = server.run_stdio_with_auth(
            input,
            &mut stdout,
            &mut stderr,
            false,
            &StdioAuthConfig::bearer_required("test-token"),
        );

        assert!(result.is_err());
        let output = String::from_utf8(stdout).map_err(|error| error.to_string())?;
        let errors = String::from_utf8(stderr).map_err(|error| error.to_string())?;
        assert!(output.contains("\"kind\":\"mcp_stdio_server_startup\""));
        assert!(errors.contains("\"code\":\"auth_required\""));
        assert!(!errors.contains("test-token"));
        Ok(())
    }

    #[test]
    fn required_stdio_bearer_token_allows_authenticated_execution_with_redacted_trace()
    -> Result<(), String> {
        let server = build_test_server();
        let input = std::io::Cursor::new(
            br#"{"command":"execute_entrypoint","auth":{"type":"bearer","token":"test-token"},"entrypoint_kind":"workflow","id":"expedition.planning.plan-expedition","version":"1.0.0","request_path":"examples/expedition/runtime-requests/plan-expedition.json"}
{"command":"shutdown"}
"#,
        );
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        assert!(
            server
                .run_stdio_with_auth(
                    input,
                    &mut stdout,
                    &mut stderr,
                    false,
                    &StdioAuthConfig::bearer_required("test-token"),
                )
                .is_ok()
        );

        let output = String::from_utf8(stdout).map_err(|error| error.to_string())?;
        assert!(output.contains("\"kind\":\"mcp_stdio_server_entrypoint_execution\""));
        assert!(output.contains("\"trace_redaction\""));
        assert!(output.contains("\"private_fields_omitted\""));
        assert!(output.contains("\"trace_redacted\":true"));
        assert!(!output.contains("test-token"));
        assert!(stderr.is_empty());
        Ok(())
    }

    fn build_test_server() -> TraverseMcpStdioServer<'static, ExpeditionExampleExecutor> {
        let execution = CanonicalExecutionContext::load_canonical()
            .expect("failed to load canonical execution context");
        let capability_registry = Box::leak(Box::new(CapabilityRegistry::new()));
        let event_registry = Box::leak(Box::new(EventRegistry::new()));
        let workflow_registry = Box::leak(Box::new(WorkflowRegistry::new()));
        let runtime = Box::leak(Box::new(
            Runtime::new(execution.capabilities, ExpeditionExampleExecutor)
                .with_workflow_registry(execution.workflows)
                .with_security_config(
                    traverse_runtime::security::RuntimeSecurityConfig::development(),
                ),
        ));
        let mcp = Box::leak(Box::new(TraverseMcp::new(
            capability_registry,
            event_registry,
            workflow_registry,
            runtime,
        )));
        let catalog = Box::leak(Box::new(
            McpDiscoveryCatalog::load_canonical()
                .expect("failed to load canonical discovery catalog"),
        ));
        TraverseMcpStdioServer::new(mcp, catalog)
    }

    const PLAN_EXPEDITION_REQUEST_PATH: &str =
        "examples/expedition/runtime-requests/plan-expedition.json";

    #[test]
    fn stdio_auth_config_debug_redacts_bearer_token_and_shows_local_trust() {
        let local = format!("{:?}", StdioAuthConfig::local_trust());
        assert!(local.contains("local_trust"));

        let bearer = format!("{:?}", StdioAuthConfig::bearer_required("super-secret"));
        assert!(bearer.contains("bearer_required"));
        assert!(bearer.contains("<redacted>"));
        assert!(!bearer.contains("super-secret"));
    }

    #[test]
    fn stdio_auth_config_from_env_reads_bearer_token_or_falls_back_to_local_trust() {
        assert_eq!(
            StdioAuthConfig::from_env_var(Ok("env-token".to_string())).mode_name(),
            "bearer_required"
        );
        assert_eq!(
            StdioAuthConfig::from_env_var(Err(std::env::VarError::NotPresent)).mode_name(),
            "local_trust"
        );
        assert_eq!(
            StdioAuthConfig::from_env_var(Ok(String::new())).mode_name(),
            "local_trust"
        );
    }

    #[test]
    fn stdio_auth_config_from_env_reads_real_process_environment() {
        // Exercises StdioAuthConfig::from_env's own body (it just delegates
        // to from_env_var, which the test above covers exhaustively); this
        // process normally has no TRAVERSE_MCP_STDIO_BEARER_TOKEN set, so it
        // falls back to local trust.
        let _ = StdioAuthConfig::from_env();
    }

    #[test]
    fn stdio_server_failure_display_formats_code_and_message() {
        let failure = StdioServerFailure::new("some_code", "some message");
        assert_eq!(format!("{failure}"), "some_code: some message");
    }

    #[test]
    fn describe_entrypoint_envelope_resolves_capability_kind() {
        let server = build_test_server();
        let envelope = server
            .describe_entrypoint_envelope(
                "capability",
                "expedition.planning.capture-expedition-objective",
                "1.0.0",
            )
            .expect("known capability entrypoint must resolve");
        assert_eq!(
            envelope["kind"],
            json!("mcp_stdio_server_entrypoint_description")
        );
        assert_eq!(envelope["entrypoint"]["artifact_kind"], json!("capability"));
    }

    #[test]
    fn describe_entrypoint_envelope_rejects_unsupported_kind() {
        let server = build_test_server();
        let error = server
            .describe_entrypoint_envelope("bogus", "id", "1.0.0")
            .expect_err("unsupported entrypoint kind must be rejected");
        assert_eq!(error.code, "invalid_request");
    }

    #[test]
    fn describe_content_group_envelope_reports_not_found_for_unknown_group() {
        let server = build_test_server();
        let error = server
            .describe_content_group_envelope("unknown-content-group")
            .expect_err("unknown content group must not resolve");
        assert_eq!(error.code, "not_found");
    }

    #[test]
    fn entrypoint_artifacts_requires_kind_id_version_and_request_path() {
        let server = build_test_server();

        let missing_kind =
            parse_command(r#"{"command":"validate_entrypoint"}"#).expect("command must parse");
        assert!(server.entrypoint_artifacts(&missing_kind).is_err());

        let missing_id =
            parse_command(r#"{"command":"validate_entrypoint","entrypoint_kind":"workflow"}"#)
                .expect("command must parse");
        assert!(server.entrypoint_artifacts(&missing_id).is_err());

        let missing_version = parse_command(
            r#"{"command":"validate_entrypoint","entrypoint_kind":"workflow","id":"expedition.planning.plan-expedition"}"#,
        )
        .expect("command must parse");
        assert!(server.entrypoint_artifacts(&missing_version).is_err());

        let missing_request_and_path = parse_command(
            r#"{"command":"validate_entrypoint","entrypoint_kind":"workflow","id":"expedition.planning.plan-expedition","version":"1.0.0"}"#,
        )
        .expect("command must parse");
        let error = server
            .entrypoint_artifacts(&missing_request_and_path)
            .expect_err("neither request nor request_path must fail");
        assert_eq!(error.code, "invalid_request");
        assert!(error.message.contains("request or request_path"));
    }

    fn plan_expedition_inline_request() -> String {
        std::fs::read_to_string(resolve_relative_path(PLAN_EXPEDITION_REQUEST_PATH))
            .expect("plan-expedition fixture must be readable")
    }

    #[test]
    fn entrypoint_artifacts_accepts_an_inline_runtime_request_without_touching_the_filesystem() {
        let server = build_test_server();
        let command = parse_command(&format!(
            r#"{{"command":"validate_entrypoint","entrypoint_kind":"workflow","id":"expedition.planning.plan-expedition","version":"1.0.0","request":{}}}"#,
            plan_expedition_inline_request()
        ))
        .expect("command must parse");

        let artifacts = server
            .entrypoint_artifacts(&command)
            .expect("inline request must resolve");
        assert_eq!(artifacts.request_source, "inline");
        assert!(artifacts.request_path.is_none());
    }

    #[test]
    fn entrypoint_artifacts_rejects_supplying_both_request_and_request_path() {
        let server = build_test_server();
        let command = parse_command(&format!(
            r#"{{"command":"execute_entrypoint","entrypoint_kind":"workflow","id":"expedition.planning.plan-expedition","version":"1.0.0","request_path":"{PLAN_EXPEDITION_REQUEST_PATH}","request":{}}}"#,
            plan_expedition_inline_request()
        ))
        .expect("command must parse");

        let error = server
            .entrypoint_artifacts(&command)
            .expect_err("request + request_path must be mutually exclusive");
        assert_eq!(error.code, "invalid_request");
        assert!(error.message.contains("not both"));
    }

    #[test]
    fn parse_inline_runtime_request_rejects_non_object_and_malformed_payloads() {
        let not_object = parse_inline_runtime_request(&json!("just a string"))
            .expect_err("non-object inline request must be rejected");
        assert_eq!(not_object.code, "invalid_request");
        assert!(not_object.message.contains("JSON object"));

        let malformed = parse_inline_runtime_request(&json!({"kind": "not-a-runtime-request"}))
            .expect_err("malformed inline request must be rejected");
        assert_eq!(malformed.code, "invalid_request");
        assert!(malformed.message.contains("inline runtime request"));
    }

    #[test]
    fn validate_entrypoint_envelope_reports_the_inline_request_source() {
        let server = build_test_server();
        let command = parse_command(&format!(
            r#"{{"command":"validate_entrypoint","entrypoint_kind":"workflow","id":"expedition.planning.plan-expedition","version":"1.0.0","request":{}}}"#,
            plan_expedition_inline_request()
        ))
        .expect("command must parse");

        let envelope = server
            .validate_entrypoint_envelope(&command)
            .expect("inline validation must succeed");
        assert_eq!(envelope["request_source"], json!("inline"));
        assert_eq!(envelope["request_path"], Value::Null);
        assert_eq!(envelope["status"], json!("valid"));
    }

    #[test]
    fn validate_runtime_request_covers_capability_and_workflow_branches() {
        let server = build_test_server();
        let mut request = load_runtime_request(PLAN_EXPEDITION_REQUEST_PATH)
            .expect("plan-expedition fixture must load");

        // Capability kind: missing capability_id / capability_version.
        request.intent.capability_id = None;
        assert!(
            server
                .validate_runtime_request("capability", "any.id", "1.0.0", &request)
                .is_err()
        );
        request.intent.capability_id = Some("expedition.planning.plan-expedition".to_string());
        request.intent.capability_version = None;
        assert!(
            server
                .validate_runtime_request("capability", "any.id", "1.0.0", &request)
                .is_err()
        );
        request.intent.capability_version = Some("1.0.0".to_string());
        assert!(
            server
                .validate_runtime_request("capability", "different.id", "1.0.0", &request)
                .is_err()
        );

        // Workflow kind: missing capability_id / capability_version, unknown
        // workflow, and target mismatch.
        request.intent.capability_id = None;
        assert!(
            server
                .validate_runtime_request(
                    "workflow",
                    "expedition.planning.plan-expedition",
                    "1.0.0",
                    &request
                )
                .is_err()
        );
        request.intent.capability_id = Some("expedition.planning.plan-expedition".to_string());
        request.intent.capability_version = None;
        assert!(
            server
                .validate_runtime_request(
                    "workflow",
                    "expedition.planning.plan-expedition",
                    "1.0.0",
                    &request
                )
                .is_err()
        );
        request.intent.capability_version = Some("1.0.0".to_string());
        assert!(
            server
                .validate_runtime_request("workflow", "unknown.workflow", "9.9.9", &request)
                .is_err()
        );
        // Workflow resolves by id/version, but the request's own intent target
        // does not match it.
        request.intent.capability_id = Some("different.workflow".to_string());
        assert!(
            server
                .validate_runtime_request(
                    "workflow",
                    "expedition.planning.plan-expedition",
                    "1.0.0",
                    &request
                )
                .is_err()
        );
        request.intent.capability_id = Some("expedition.planning.plan-expedition".to_string());
        assert!(
            server
                .validate_runtime_request(
                    "workflow",
                    "expedition.planning.plan-expedition",
                    "1.0.0",
                    &request
                )
                .is_ok()
        );

        // Unsupported entrypoint_kind.
        assert!(
            server
                .validate_runtime_request(
                    "bogus",
                    "expedition.planning.plan-expedition",
                    "1.0.0",
                    &request
                )
                .is_err()
        );
    }

    #[test]
    fn missing_content_group_id_and_entrypoint_fields_are_rejected_over_stdio() {
        let server = build_test_server();

        let cases: [&[u8]; 4] = [
            b"{\"command\":\"describe_content_group\"}\n",
            b"{\"command\":\"describe_entrypoint\"}\n",
            b"{\"command\":\"describe_entrypoint\",\"entrypoint_kind\":\"workflow\"}\n",
            b"{\"command\":\"describe_entrypoint\",\"entrypoint_kind\":\"workflow\",\"id\":\"expedition.planning.plan-expedition\"}\n",
        ];
        for case in cases {
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let result =
                server.run_stdio(std::io::Cursor::new(case), &mut stdout, &mut stderr, false);
            assert!(result.is_err(), "expected rejection for {case:?}");
        }
    }

    #[test]
    fn run_stdio_server_reports_simulated_startup_failure() {
        let result = run_stdio_server(true, None);
        assert!(result.is_err());
    }

    /// Writer that fails exactly once, on the `target`-th time it is asked to
    /// write a lone `\n` byte. `write_json_line` always ends an envelope with
    /// a dedicated `write_all(b"\n")` call, so counting those calls lets a
    /// test target one specific envelope write (and its `io_error` branch)
    /// without depending on how many raw `write` calls the JSON body itself
    /// takes.
    struct FailOnNthNewline {
        target: usize,
        seen: std::cell::Cell<usize>,
    }

    impl Write for FailOnNthNewline {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            if buf == b"\n" {
                let index = self.seen.get();
                self.seen.set(index + 1);
                if index == self.target {
                    return Err(io::Error::other("simulated stdout write failure"));
                }
            }
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn write_failures_are_reported_as_io_errors_for_every_stdio_response() {
        let server = build_test_server();
        let session: &[u8] = br#"{"command":"describe_server"}
{"command":"list_content_groups"}
{"command":"describe_content_group","content_group_id":"core-runtime-example"}
{"command":"list_entrypoints"}
{"command":"describe_entrypoint","entrypoint_kind":"workflow","id":"expedition.planning.plan-expedition","version":"1.0.0"}
{"command":"validate_entrypoint","entrypoint_kind":"workflow","id":"expedition.planning.plan-expedition","version":"1.0.0","request_path":"examples/expedition/runtime-requests/plan-expedition.json"}
{"command":"execute_entrypoint","entrypoint_kind":"workflow","id":"expedition.planning.plan-expedition","version":"1.0.0","request_path":"examples/expedition/runtime-requests/plan-expedition.json"}
{"command":"render_execution_report","entrypoint_kind":"workflow","id":"expedition.planning.plan-expedition","version":"1.0.0","request_path":"examples/expedition/runtime-requests/plan-expedition.json"}
{"command":"shutdown"}
"#;

        // index 0 = startup envelope, 1..=9 = the nine command responses in order.
        for target in 0..=9 {
            let mut stdout = FailOnNthNewline {
                target,
                seen: std::cell::Cell::new(0),
            };
            let mut stderr = Vec::new();
            let result = server.run_stdio(
                std::io::Cursor::new(session),
                &mut stdout,
                &mut stderr,
                false,
            );
            assert!(result.is_err(), "expected write failure at index {target}");
        }
    }

    #[test]
    fn write_failure_on_stdin_closed_shutdown_envelope_is_reported() {
        let server = build_test_server();
        let mut stdout = FailOnNthNewline {
            target: 1,
            seen: std::cell::Cell::new(0),
        };
        let mut stderr = Vec::new();
        let result = server.run_stdio(
            std::io::Cursor::new(b"" as &[u8]),
            &mut stdout,
            &mut stderr,
            false,
        );
        assert!(result.is_err());
        assert!(stdout.flush().is_ok());
    }

    #[test]
    fn simulated_startup_failure_stderr_write_error_is_reported() {
        let server = build_test_server();
        let mut stdout = Vec::new();
        let mut stderr = FailOnNthNewline {
            target: 0,
            seen: std::cell::Cell::new(0),
        };
        let result = server.run_stdio(
            std::io::Cursor::new(b"" as &[u8]),
            &mut stdout,
            &mut stderr,
            true,
        );
        assert!(result.is_err());
    }

    #[test]
    fn stdin_closed_shutdown_envelope_completes_successfully_without_a_shutdown_command() {
        let server = build_test_server();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        // Blank lines between commands must be skipped, not treated as commands.
        let input = std::io::Cursor::new(b"\n{\"command\":\"describe_server\"}\n\n" as &[u8]);
        let result = server.run_stdio(input, &mut stdout, &mut stderr, false);
        assert!(result.is_ok());
        let output = String::from_utf8(stdout).expect("stdout must be valid UTF-8");
        assert!(output.contains("\"kind\":\"mcp_stdio_server_shutdown\""));
    }

    #[test]
    fn malformed_json_command_line_is_rejected() {
        let server = build_test_server();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let input = std::io::Cursor::new(b"not json\n" as &[u8]);
        let result = server.run_stdio(input, &mut stdout, &mut stderr, false);
        assert!(result.is_err());
    }

    #[test]
    fn invalid_utf8_command_line_is_reported_as_io_error() {
        let server = build_test_server();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let input = std::io::Cursor::new(vec![0xFF, 0xFE, b'\n']);
        let result = server.run_stdio(input, &mut stdout, &mut stderr, false);
        assert!(result.is_err());
    }

    #[test]
    fn unsupported_stdio_command_is_rejected() {
        let server = build_test_server();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let input = std::io::Cursor::new(b"{\"command\":\"bogus_command\"}\n" as &[u8]);
        let result = server.run_stdio(input, &mut stdout, &mut stderr, false);
        assert!(result.is_err());
        let errors = String::from_utf8(stderr).expect("stderr must be valid UTF-8");
        assert!(errors.contains("\"code\":\"unsupported_command\""));
    }

    #[test]
    fn validate_and_render_execution_report_commands_report_underlying_failures() {
        let server = build_test_server();

        for command in ["validate_entrypoint", "render_execution_report"] {
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let input = std::io::Cursor::new(
                format!("{{\"command\":\"{command}\",\"entrypoint_kind\":\"workflow\"}}\n")
                    .into_bytes(),
            );
            let result = server.run_stdio(input, &mut stdout, &mut stderr, false);
            assert!(result.is_err(), "expected {command} to fail on missing id");
        }
    }

    #[test]
    fn derive_composability_metadata_requires_workflow_ref_for_workflow_kind() {
        let capability = test_capability_bundle_artifact();
        let error = derive_composability_metadata(ImplementationKind::Workflow, None, &capability)
            .expect_err("workflow-backed capability without workflow_ref must be rejected");
        assert_eq!(error.code, "invalid_request");
    }

    fn test_capability_bundle_artifact() -> traverse_registry::CapabilityBundleArtifact {
        let server = build_test_server();
        server.catalog.bundle.capabilities[0].clone()
    }

    #[test]
    fn parse_workflow_ref_requires_workflow_id_and_version() {
        let error = parse_workflow_ref(&json!({})).expect_err("missing workflow_id must fail");
        assert_eq!(error.code, "invalid_request");

        let error = parse_workflow_ref(&json!({"workflow_id": "wf"}))
            .expect_err("missing workflow_version must fail");
        assert_eq!(error.code, "invalid_request");

        let reference =
            parse_workflow_ref(&json!({"workflow_id": "wf", "workflow_version": "1.0.0"}))
                .expect("fully specified workflow_ref must parse");
        assert_eq!(reference.workflow_id, "wf");
        assert_eq!(reference.workflow_version, "1.0.0");
    }

    #[test]
    fn load_runtime_request_reports_missing_file_and_invalid_json() {
        let missing = load_runtime_request("/definitely/missing/runtime-request.json")
            .expect_err("missing file must fail");
        assert_eq!(missing.code, "io_error");

        let temp_path = std::env::temp_dir().join(format!(
            "traverse-mcp-invalid-runtime-request-{}.json",
            std::process::id()
        ));
        fs::write(&temp_path, b"not json").expect("temp file must write");
        let invalid = load_runtime_request(&temp_path.display().to_string())
            .expect_err("invalid JSON runtime request must fail");
        assert_eq!(invalid.code, "invalid_request");
        let _ = fs::remove_file(&temp_path);
    }

    #[test]
    fn provenance_source_label_covers_every_variant() {
        assert_eq!(
            provenance_source_label(&traverse_contracts::ProvenanceSource::Greenfield),
            "greenfield"
        );
        assert_eq!(
            provenance_source_label(&traverse_contracts::ProvenanceSource::BrownfieldExtracted),
            "brownfield-extracted"
        );
        assert_eq!(
            provenance_source_label(&traverse_contracts::ProvenanceSource::AiGenerated),
            "ai-generated"
        );
        assert_eq!(
            provenance_source_label(&traverse_contracts::ProvenanceSource::AiAssisted),
            "ai-assisted"
        );
    }

    #[test]
    fn execute_validate_team_readiness_covers_ready_and_needs_action_branches() {
        let not_ready = execute_validate_team_readiness(&json!({
            "objective": {"objective_id": "obj-1"},
            "team_profile": {"equipment_ready": false}
        }))
        .expect("valid input must execute");
        assert_eq!(
            not_ready["readiness_result"]["status"],
            json!("needs_action")
        );
        assert_eq!(
            not_ready["readiness_result"]["required_actions"],
            json!(["complete equipment verification"])
        );

        let missing_field = execute_validate_team_readiness(&json!({}))
            .expect_err("missing objective field must fail");
        assert_eq!(
            missing_field.code,
            traverse_runtime::LocalExecutionFailureCode::ExecutionFailed
        );
    }

    // ----------------------------------------------------------------------
    // Spec 119 Mode A: verified public-registry discovery and execution
    // ----------------------------------------------------------------------

    mod mode_a {
        #![allow(clippy::unwrap_used, clippy::panic)]

        use super::*;
        use sha2::{Digest, Sha256};
        use std::fmt::Write as _;
        use std::sync::atomic::{AtomicU64, Ordering};
        use traverse_embedder::{
            RegistryArtifactFetcher, prepare_registry_dependency, publish_public_metadata,
        };
        use traverse_registry::{
            PublicRegistryCapabilityRecord, PublicUseCaseSummary, SyncedPublicRegistryState,
        };

        const KIT_ID: &str = "core.normalize-participants";
        const KIT_NAMESPACE: &str = "core";
        const KIT_VERSION: &str = "1.1.0";
        const KIT_ARTIFACT_URL: &str =
            "https://registry.test/core/normalize-participants/1.1.0/module.wasm";
        const KIT_CONTRACT_URL: &str =
            "https://registry.test/core/normalize-participants/1.1.0/contract.json";

        fn repo_root() -> PathBuf {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .and_then(Path::parent)
                .expect("workspace root")
                .to_path_buf()
        }

        fn sha256_prefixed(bytes: &[u8]) -> String {
            let mut hasher = Sha256::new();
            hasher.update(bytes);
            let mut out = String::from("sha256:");
            for byte in hasher.finalize() {
                write!(out, "{byte:02x}").expect("writing to a String cannot fail");
            }
            out
        }

        fn kit_wasm_bytes() -> Vec<u8> {
            fs::read(repo_root().join(
                "examples/core-normalize-participants/artifacts/core-normalize-participants.wasm",
            ))
            .expect("kit wasm fixture must exist")
        }

        fn kit_contract_bytes() -> Vec<u8> {
            fs::read(repo_root().join("examples/core-normalize-participants/contract.json"))
                .expect("kit contract fixture must exist")
        }

        fn kit_request_json() -> String {
            let value: Value = serde_json::from_slice(
                &fs::read(repo_root().join(
                    "examples/core-normalize-participants/runtime-requests/uc01-mixed-match.json",
                ))
                .expect("kit runtime request fixture must exist"),
            )
            .expect("kit runtime request must be valid json");
            value.to_string()
        }

        fn kit_reference() -> RegistryReference {
            RegistryReference {
                namespace: KIT_NAMESPACE.to_string(),
                id: KIT_ID.to_string(),
                version_range: format!("={KIT_VERSION}"),
            }
        }

        fn kit_snapshot() -> SyncedPublicRegistryState {
            let record = PublicRegistryCapabilityRecord {
                namespace: KIT_NAMESPACE.to_string(),
                id: KIT_ID.to_string(),
                version: KIT_VERSION.to_string(),
                digest: sha256_prefixed(&kit_wasm_bytes()),
                artifact_url: KIT_ARTIFACT_URL.to_string(),
                contract_digest: sha256_prefixed(&kit_contract_bytes()),
                contract_url: KIT_CONTRACT_URL.to_string(),
                deprecated: false,
                summary: "Normalize raw participants into canonical records.".to_string(),
                description: "Verified public kit fixture for Mode A tests.".to_string(),
                use_cases: vec![PublicUseCaseSummary {
                    scenario: "Resolve extracted names and emails to workspace members."
                        .to_string(),
                }],
                service_type: "stateless".to_string(),
                permitted_targets: vec!["wasm".to_string()],
                lifecycle: "active".to_string(),
                provenance: None,
            };
            SyncedPublicRegistryState {
                schema_version: "1.0.0".to_string(),
                workspace_id: "mode-a-fixture".to_string(),
                state_scope: "public_registry_synced".to_string(),
                source_repo: "traverse-framework/registry".to_string(),
                release_tag: "index-v1".to_string(),
                index_version: 1,
                generated_at: "2026-08-25T00:00:00Z".to_string(),
                source_commit: None,
                synced_at: "2026-08-25T00:00:00Z".to_string(),
                record_count: 1,
                validation_status: "valid".to_string(),
                governing_spec: "055-registry-sync".to_string(),
                capabilities: vec![record],
                events: Vec::new(),
            }
        }

        struct LocalKitFetcher;
        impl RegistryArtifactFetcher for LocalKitFetcher {
            fn fetch(&self, url: &str) -> Result<Vec<u8>, String> {
                match url {
                    KIT_ARTIFACT_URL => Ok(kit_wasm_bytes()),
                    KIT_CONTRACT_URL => Ok(kit_contract_bytes()),
                    other => Err(format!("unexpected fetch url {other}")),
                }
            }
        }

        fn fresh_cache_root(tag: &str) -> PathBuf {
            static SEQ: AtomicU64 = AtomicU64::new(0);
            let seq = SEQ.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "traverse-mcp-mode-a-{tag}-{}-{seq}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&root);
            root
        }

        /// A fully prepared, digest-verified Mode A cache in a fresh temp dir.
        fn build_verified_cache(tag: &str) -> PathBuf {
            let root = fresh_cache_root(tag);
            let cache = HostRegistryCache::new(&root);
            let snapshot = kit_snapshot();
            prepare_registry_dependency(&cache, &snapshot, &kit_reference(), &LocalKitFetcher)
                .expect("prepare verified kit");
            publish_public_metadata(&cache, &snapshot, false).expect("publish public metadata");
            root
        }

        fn mode_a_context(root: &Path) -> &'static ModeAContext {
            Box::leak(Box::new(
                ModeAContext::load(root).expect("verified cache must load"),
            ))
        }

        fn mode_a_server(
            root: &Path,
        ) -> TraverseMcpStdioServer<'static, ExpeditionExampleExecutor> {
            build_test_server().with_mode_a(mode_a_context(root))
        }

        /// Drive one command line through the server and return every stdout
        /// envelope as parsed JSON.
        fn drive(
            server: &TraverseMcpStdioServer<'_, ExpeditionExampleExecutor>,
            line: &str,
            auth: &StdioAuthConfig,
        ) -> (Vec<Value>, String, Result<(), StdioServerFailure>) {
            let input = std::io::Cursor::new(format!("{line}\n").into_bytes());
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let outcome = server.run_stdio_with_auth(input, &mut stdout, &mut stderr, false, auth);
            let envelopes = String::from_utf8(stdout)
                .expect("stdout utf-8")
                .lines()
                .filter(|l| !l.trim().is_empty())
                .map(|l| serde_json::from_str::<Value>(l).expect("stdout line json"))
                .collect();
            (
                envelopes,
                String::from_utf8(stderr).expect("stderr utf-8"),
                outcome,
            )
        }

        fn envelope<'a>(envelopes: &'a [Value], kind: &str) -> &'a Value {
            envelopes
                .iter()
                .find(|value| value["kind"] == json!(kind))
                .unwrap_or_else(|| panic!("missing {kind} envelope in {envelopes:?}"))
        }

        #[test]
        #[ignore = "regenerates the committed Mode A fixture cache under tests/fixtures"]
        fn regenerate_committed_fixture() {
            let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mode-a-cache");
            let _ = fs::remove_dir_all(&root);
            let cache = HostRegistryCache::new(&root);
            let snapshot = kit_snapshot();
            prepare_registry_dependency(&cache, &snapshot, &kit_reference(), &LocalKitFetcher)
                .expect("prepare verified kit");
            publish_public_metadata(&cache, &snapshot, false).expect("publish public metadata");
        }

        #[test]
        fn startup_reports_verified_public_mode_and_no_content_groups() {
            let root = build_verified_cache("startup");
            let server = mode_a_server(&root);
            let (envelopes, stderr, outcome) = drive(
                &server,
                r#"{"command":"describe_server"}"#,
                &StdioAuthConfig::local_trust(),
            );
            assert!(outcome.is_ok());
            assert!(stderr.is_empty());
            let startup = envelope(&envelopes, "mcp_stdio_server_startup");
            assert_eq!(startup["mode"], json!("verified_public"));
            assert_eq!(startup["governing_spec"], json!(MODE_A_GOVERNING_SPEC));
            assert_eq!(startup["content_group_count"], json!(0));
            assert_eq!(
                startup["discovery_source"]["kind"],
                json!("host_verified_public_registry")
            );
            assert_eq!(startup["discovery_source"]["capability_count"], json!(1));
        }

        #[test]
        fn discovery_lists_only_verified_public_entries_without_expedition_records() {
            let root = build_verified_cache("discovery");
            let server = mode_a_server(&root);
            let (envelopes, _, outcome) = drive(
                &server,
                r#"{"command":"list_entrypoints"}"#,
                &StdioAuthConfig::local_trust(),
            );
            assert!(outcome.is_ok());
            let list = envelope(&envelopes, "mcp_stdio_server_entrypoint_list");
            assert_eq!(list["governing_spec"], json!(MODE_A_GOVERNING_SPEC));
            assert_eq!(list["content_groups"], json!([]));
            let capabilities = list["entrypoints"]["capabilities"]
                .as_array()
                .expect("capabilities array");
            assert_eq!(capabilities.len(), 1);
            assert_eq!(capabilities[0]["id"], json!(KIT_ID));
            assert_eq!(capabilities[0]["version"], json!(KIT_VERSION));
            assert_eq!(list["entrypoints"]["workflows"], json!([]));
            let serialized = serde_json::to_string(list).unwrap();
            assert!(!serialized.contains("expedition"));
        }

        #[test]
        fn describe_entrypoint_returns_redacted_public_detail() {
            let root = build_verified_cache("describe");
            let server = mode_a_server(&root);
            let (envelopes, _, outcome) = drive(
                &server,
                &format!(
                    r#"{{"command":"describe_entrypoint","entrypoint_kind":"capability","id":"{KIT_ID}","version":"{KIT_VERSION}"}}"#
                ),
                &StdioAuthConfig::local_trust(),
            );
            assert!(outcome.is_ok());
            let detail = envelope(&envelopes, "mcp_stdio_server_entrypoint_description");
            let entrypoint = &detail["entrypoint"];
            assert_eq!(entrypoint["id"], json!(KIT_ID));
            assert_eq!(entrypoint["artifact_kind"], json!("capability"));
            assert!(entrypoint.get("input_schema").is_none());
            assert!(entrypoint.get("contract").is_none());
        }

        #[test]
        fn describe_unknown_entrypoint_and_content_group_fail_closed() {
            let root = build_verified_cache("unknown");
            let (_, _, outcome) = drive(
                &mode_a_server(&root),
                r#"{"command":"describe_entrypoint","entrypoint_kind":"capability","id":"core.missing","version":"9.9.9"}"#,
                &StdioAuthConfig::local_trust(),
            );
            assert_eq!(
                outcome.expect_err("unknown entrypoint must fail").code,
                "not_found"
            );

            let (_, _, outcome) = drive(
                &mode_a_server(&root),
                r#"{"command":"describe_content_group","content_group_id":"core-runtime-example"}"#,
                &StdioAuthConfig::local_trust(),
            );
            assert_eq!(
                outcome.expect_err("no content groups in Mode A").code,
                "not_found"
            );
        }

        #[test]
        fn inline_execute_runs_the_exact_digest_verified_wasm_artifact() {
            let root = build_verified_cache("execute");
            let server = mode_a_server(&root);
            let line = format!(
                r#"{{"command":"execute_entrypoint","entrypoint_kind":"capability","id":"{KIT_ID}","version":"{KIT_VERSION}","request":{}}}"#,
                kit_request_json()
            );
            let (envelopes, stderr, outcome) =
                drive(&server, &line, &StdioAuthConfig::local_trust());
            assert!(outcome.is_ok(), "stderr: {stderr}");
            let execution = envelope(&envelopes, "mcp_stdio_server_entrypoint_execution");
            assert_eq!(execution["mode"], json!("verified_public"));
            assert_eq!(execution["status"], json!("completed"));
            assert_eq!(execution["request_source"], json!("inline"));
            assert_eq!(execution["request_path"], Value::Null);
            assert_eq!(execution["artifact"]["verified"], json!(true));
            assert_eq!(
                execution["artifact"]["digest_matches_public_state"],
                json!(true)
            );
            assert_eq!(execution["result"]["status"], json!("completed"));
            assert!(execution["result"].get("error").is_none_or(Value::is_null));
            // FR-005: redacted public trace only — the private trace fields are
            // named in `private_fields_omitted` and are absent as trace keys.
            let trace = &execution["trace"];
            assert!(trace["private_fields_omitted"].is_array());
            for private_key in ["decision_evidence", "otel_trace", "selection", "execution"] {
                assert!(
                    trace.get(private_key).is_none(),
                    "redacted trace must not carry {private_key}"
                );
            }
        }

        #[test]
        fn execute_rejects_supplying_both_request_and_request_path() {
            let root = build_verified_cache("xor");
            let server = mode_a_server(&root);
            let line = format!(
                r#"{{"command":"execute_entrypoint","entrypoint_kind":"capability","id":"{KIT_ID}","version":"{KIT_VERSION}","request_path":"examples/core-normalize-participants/runtime-requests/uc01-mixed-match.json","request":{}}}"#,
                kit_request_json()
            );
            let (_, stderr, outcome) = drive(&server, &line, &StdioAuthConfig::local_trust());
            assert!(outcome.is_err());
            assert!(stderr.contains("not both"));
        }

        #[test]
        fn execute_rejects_an_identity_mismatch_between_request_and_entrypoint() {
            let root = build_verified_cache("identity");
            let server = mode_a_server(&root);
            let mut request: Value = serde_json::from_str(&kit_request_json()).unwrap();
            request["intent"]["capability_version"] = json!("2.0.0");
            let line = format!(
                r#"{{"command":"execute_entrypoint","entrypoint_kind":"capability","id":"{KIT_ID}","version":"{KIT_VERSION}","request":{request}}}"#
            );
            let (_, stderr, outcome) = drive(&server, &line, &StdioAuthConfig::local_trust());
            assert!(outcome.is_err());
            assert!(stderr.contains("does not match"));
        }

        #[test]
        fn workflow_entrypoint_kind_is_rejected_in_mode_a() {
            let root = build_verified_cache("workflow");
            let server = mode_a_server(&root);
            let (_, stderr, outcome) = drive(
                &server,
                r#"{"command":"validate_entrypoint","entrypoint_kind":"workflow","id":"x","version":"1.0.0","request_path":"examples/core-normalize-participants/runtime-requests/uc01-mixed-match.json"}"#,
                &StdioAuthConfig::local_trust(),
            );
            assert!(outcome.is_err());
            assert!(stderr.contains("capability"));
        }

        #[test]
        fn missing_prepared_state_fails_closed_without_a_fallback_catalog() {
            let root = fresh_cache_root("absent");
            let failure = ModeAContext::load(&root).expect_err("absent state must fail closed");
            assert_eq!(failure.code, "registry_sync_missing");
        }

        #[test]
        fn malformed_public_metadata_generation_fails_closed() {
            let root = build_verified_cache("malformed");
            let generation = root.join("public-metadata").join("current.json");
            fs::write(&generation, b"{ not valid json").expect("overwrite generation");
            let failure = ModeAContext::load(&root).expect_err("malformed state must fail closed");
            assert_eq!(failure.code, "registry_metadata_cache_invalid");
        }

        #[test]
        fn execute_fails_closed_when_the_verified_artifact_entry_is_missing() {
            let root = build_verified_cache("tampered");
            // Remove the digest-verified artifact bytes but keep discovery state.
            let sha_dir = root.join("sha256");
            for entry in fs::read_dir(&sha_dir).expect("sha256 dir") {
                fs::remove_file(entry.expect("dir entry").path()).expect("remove artifact");
            }
            let server = mode_a_server(&root);
            let line = format!(
                r#"{{"command":"execute_entrypoint","entrypoint_kind":"capability","id":"{KIT_ID}","version":"{KIT_VERSION}","request":{}}}"#,
                kit_request_json()
            );
            let (_, stderr, outcome) = drive(&server, &line, &StdioAuthConfig::local_trust());
            assert!(outcome.is_err());
            assert!(
                stderr.contains("registry_cache_entry_missing")
                    || stderr.contains("registry_prepare_failed")
            );
        }

        #[test]
        fn committed_fixture_cache_serves_discovery_and_inline_execute() {
            let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mode-a-cache");
            assert!(
                root.join("public-metadata/current.json").is_file(),
                "run `cargo test -p traverse-mcp -- --ignored regenerate_committed_fixture` to rebuild the fixture"
            );
            let server = mode_a_server(&root);
            let (discovery, _, discovery_outcome) = drive(
                &server,
                r#"{"command":"list_entrypoints"}"#,
                &StdioAuthConfig::local_trust(),
            );
            assert!(discovery_outcome.is_ok());
            assert_eq!(
                envelope(&discovery, "mcp_stdio_server_entrypoint_list")["entrypoints"]["capabilities"]
                    [0]["id"],
                json!(KIT_ID)
            );

            let server = mode_a_server(&root);
            let line = format!(
                r#"{{"command":"execute_entrypoint","entrypoint_kind":"capability","id":"{KIT_ID}","version":"{KIT_VERSION}","request":{}}}"#,
                kit_request_json()
            );
            let (execution, stderr, outcome) =
                drive(&server, &line, &StdioAuthConfig::local_trust());
            assert!(outcome.is_ok(), "stderr: {stderr}");
            assert_eq!(
                envelope(&execution, "mcp_stdio_server_entrypoint_execution")["result"]["status"],
                json!("completed")
            );
        }

        #[test]
        fn validate_entrypoint_reports_verified_artifact_evidence_without_executing() {
            let root = build_verified_cache("validate");
            let server = mode_a_server(&root);
            let line = format!(
                r#"{{"command":"validate_entrypoint","entrypoint_kind":"capability","id":"{KIT_ID}","version":"{KIT_VERSION}","request":{}}}"#,
                kit_request_json()
            );
            let (envelopes, stderr, outcome) =
                drive(&server, &line, &StdioAuthConfig::local_trust());
            assert!(outcome.is_ok(), "stderr: {stderr}");
            let validation = envelope(&envelopes, "mcp_stdio_server_entrypoint_validation");
            assert_eq!(validation["status"], json!("valid"));
            assert_eq!(validation["mode"], json!("verified_public"));
            assert_eq!(validation["request_source"], json!("inline"));
            assert_eq!(
                validation["artifact"]["digest_matches_public_state"],
                json!(true)
            );
            assert_eq!(
                validation["request"]["intent"]["capability_id"],
                json!(KIT_ID)
            );
        }

        #[test]
        fn render_execution_report_returns_a_redacted_report_from_the_verified_artifact() {
            let root = build_verified_cache("report");
            let server = mode_a_server(&root);
            let line = format!(
                r#"{{"command":"render_execution_report","entrypoint_kind":"capability","id":"{KIT_ID}","version":"{KIT_VERSION}","request":{}}}"#,
                kit_request_json()
            );
            let (envelopes, stderr, outcome) =
                drive(&server, &line, &StdioAuthConfig::local_trust());
            assert!(outcome.is_ok(), "stderr: {stderr}");
            let report = envelope(&envelopes, "mcp_stdio_server_execution_report");
            assert_eq!(report["status"], json!("rendered"));
            assert_eq!(report["mode"], json!("verified_public"));
            assert_eq!(report["report"]["trace_redacted"], json!(true));
            assert_eq!(report["report"]["result_status"], json!("completed"));
            assert_eq!(report["execution"]["result"]["status"], json!("completed"));
        }

        #[test]
        fn execute_requires_the_local_bearer_token_when_configured() {
            let root = build_verified_cache("auth");
            let auth = StdioAuthConfig::bearer_required("mode-a-secret");
            let denied_line = format!(
                r#"{{"command":"execute_entrypoint","entrypoint_kind":"capability","id":"{KIT_ID}","version":"{KIT_VERSION}","request":{}}}"#,
                kit_request_json()
            );
            let (_, _, outcome) = drive(&mode_a_server(&root), &denied_line, &auth);
            assert_eq!(outcome.expect_err("missing token").code, "auth_required");

            let allowed_line = format!(
                r#"{{"command":"execute_entrypoint","auth":{{"type":"bearer","token":"mode-a-secret"}},"entrypoint_kind":"capability","id":"{KIT_ID}","version":"{KIT_VERSION}","request":{}}}"#,
                kit_request_json()
            );
            let (envelopes, stderr, outcome) = drive(&mode_a_server(&root), &allowed_line, &auth);
            assert!(outcome.is_ok(), "stderr: {stderr}");
            assert_eq!(
                envelope(&envelopes, "mcp_stdio_server_entrypoint_execution")["status"],
                json!("completed")
            );
        }

        #[test]
        fn mode_a_commands_reject_missing_id_or_version() {
            let root = build_verified_cache("fields");
            for line in [
                r#"{"command":"execute_entrypoint","entrypoint_kind":"capability","request":{}}"#,
                r#"{"command":"execute_entrypoint","entrypoint_kind":"capability","id":"core.x","request":{}}"#,
            ] {
                let (_, _, outcome) =
                    drive(&mode_a_server(&root), line, &StdioAuthConfig::local_trust());
                assert_eq!(outcome.expect_err("missing field").code, "invalid_request");
            }
        }

        #[test]
        fn mode_a_requires_request_or_request_path() {
            let root = build_verified_cache("norequest");
            let (_, _, outcome) = drive(
                &mode_a_server(&root),
                &format!(
                    r#"{{"command":"validate_entrypoint","entrypoint_kind":"capability","id":"{KIT_ID}","version":"{KIT_VERSION}"}}"#
                ),
                &StdioAuthConfig::local_trust(),
            );
            let failure = outcome.expect_err("no request must fail");
            assert_eq!(failure.code, "invalid_request");
        }

        #[test]
        fn search_capabilities_runs_against_the_verified_public_generation() {
            let root = build_verified_cache("search");
            let (envelopes, _, outcome) = drive(
                &mode_a_server(&root),
                r#"{"command":"search_capabilities","query":"resolve extracted names"}"#,
                &StdioAuthConfig::local_trust(),
            );
            assert!(outcome.is_ok());
            let search = envelope(&envelopes, "mcp_capability_search");
            let records = search["records"].as_array().expect("records array");
            assert_eq!(records.len(), 1);
            assert_eq!(records[0]["id"], json!(KIT_ID));
        }

        #[test]
        fn describe_server_and_list_content_groups_reflect_mode_a() {
            let root = build_verified_cache("describe-server");
            let (describe, _, _) = drive(
                &mode_a_server(&root),
                r#"{"command":"describe_server"}"#,
                &StdioAuthConfig::local_trust(),
            );
            let description = envelope(&describe, "mcp_stdio_server_description");
            assert_eq!(description["mode"], json!("verified_public"));
            assert_eq!(
                description["governed_surface_counts"]["capabilities"],
                json!(1)
            );
            assert_eq!(description["content_groups"], json!([]));

            let (groups, _, _) = drive(
                &mode_a_server(&root),
                r#"{"command":"list_content_groups"}"#,
                &StdioAuthConfig::local_trust(),
            );
            assert_eq!(
                envelope(&groups, "mcp_stdio_server_content_group_list")["content_groups"],
                json!([])
            );
        }

        #[test]
        fn describe_entrypoint_rejects_a_non_capability_kind() {
            let root = build_verified_cache("describe-kind");
            let (_, _, outcome) = drive(
                &mode_a_server(&root),
                &format!(
                    r#"{{"command":"describe_entrypoint","entrypoint_kind":"workflow","id":"{KIT_ID}","version":"{KIT_VERSION}"}}"#
                ),
                &StdioAuthConfig::local_trust(),
            );
            assert_eq!(outcome.expect_err("workflow kind").code, "invalid_request");
        }

        #[test]
        fn repeated_discovery_from_unchanged_state_is_deterministic() {
            let root = build_verified_cache("deterministic");
            let first = drive(
                &mode_a_server(&root),
                r#"{"command":"list_entrypoints"}"#,
                &StdioAuthConfig::local_trust(),
            )
            .0;
            let second = drive(
                &mode_a_server(&root),
                r#"{"command":"list_entrypoints"}"#,
                &StdioAuthConfig::local_trust(),
            )
            .0;
            assert_eq!(first, second);
        }
    }
}
