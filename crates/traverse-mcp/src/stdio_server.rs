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
use traverse_registry::{
    BinaryFormat as RegistryBinaryFormat, BinaryReference, CapabilityArtifactRecord,
    CapabilityRegistration, CapabilityRegistry, ComposabilityMetadata, CompositionKind,
    CompositionPattern, EventRegistration, EventRegistry, ImplementationKind, RegistryBundle,
    RegistryProvenance, SourceKind, SourceReference, WorkflowReference, WorkflowRegistration,
    WorkflowRegistry, load_registry_bundle,
};
use traverse_runtime::{LocalExecutor, Runtime, RuntimeRequest, parse_runtime_request};

const SERVER_NAME: &str = "traverse-mcp";
const HOST_MODE: &str = "stdio";
const GOVERNING_SPEC: &str = "022-mcp-wasm-server";
const PUBLIC_SURFACE_ID: &str = "traverse.mcp.stdio-server";
const SUPPORTING_COMMANDS: &[&str] = &[
    "describe_server",
    "list_content_groups",
    "describe_content_group",
    "list_entrypoints",
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
        let bundle = load_registry_bundle(&manifest_path).map_err(|failure| {
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
        let bundle = load_registry_bundle(&manifest_path).map_err(|failure| {
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

#[derive(Debug)]
pub struct TraverseMcpStdioServer<'a, E> {
    mcp: &'a TraverseMcp<'a, E>,
    catalog: &'a McpDiscoveryCatalog,
}

impl<'a, E> TraverseMcpStdioServer<'a, E>
where
    E: LocalExecutor,
{
    #[must_use]
    pub fn new(mcp: &'a TraverseMcp<'a, E>, catalog: &'a McpDiscoveryCatalog) -> Self {
        Self { mcp, catalog }
    }

    #[must_use]
    pub fn startup_envelope(&self) -> Value {
        json!({
            "kind": "mcp_stdio_server_startup",
            "server_name": SERVER_NAME,
            "host_mode": HOST_MODE,
            "governing_spec": GOVERNING_SPEC,
            "status": "ready",
            "supported_commands": SUPPORTING_COMMANDS,
            "public_surface_id": PUBLIC_SURFACE_ID,
            "auth_boundary": {
                "default_mode": "local_trust",
                "required_token_env": "TRAVERSE_MCP_STDIO_BEARER_TOKEN",
                "required_commands": ["execute_entrypoint", "render_execution_report"],
            },
            "content_group_count": McpDiscoveryCatalog::content_group_count(),
        })
    }

    #[must_use]
    pub fn describe_envelope(&self) -> Value {
        let validation_path = youaskm3_mcp_consumption_validation_path();
        json!({
            "kind": "mcp_stdio_server_description",
            "server_name": SERVER_NAME,
            "host_mode": HOST_MODE,
            "governing_spec": GOVERNING_SPEC,
            "runtime_authority": "Traverse runtime authority",
            "public_surface_id": PUBLIC_SURFACE_ID,
            "supported_commands": SUPPORTING_COMMANDS,
            "auth_boundary": {
                "default_mode": "local_trust",
                "required_token_env": "TRAVERSE_MCP_STDIO_BEARER_TOKEN",
                "required_commands": ["execute_entrypoint", "render_execution_report"],
                "token_output_policy": "never_echo_raw_token",
            },
            "governed_surface_counts": {
                "capabilities": self.catalog.capability_count(),
                "events": self.catalog.event_count(),
                "workflows": self.catalog.workflow_count(),
            },
            "content_groups": McpDiscoveryCatalog::content_group_summaries(),
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
            "governing_spec": GOVERNING_SPEC,
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
            "governing_spec": GOVERNING_SPEC,
            "content_groups": McpDiscoveryCatalog::content_group_summaries(),
        })
    }

    /// # Errors
    ///
    /// Returns `invalid_request` when the content group id is missing or unsupported.
    pub fn describe_content_group_envelope(
        &self,
        content_group_id: &str,
    ) -> Result<Value, StdioServerFailure> {
        McpDiscoveryCatalog::content_group_detail(content_group_id)
            .map(|content_group| {
                json!({
                    "kind": "mcp_stdio_server_content_group_description",
                    "server_name": SERVER_NAME,
                    "host_mode": HOST_MODE,
                    "governing_spec": GOVERNING_SPEC,
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
                        "governing_spec": GOVERNING_SPEC,
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
                        "governing_spec": GOVERNING_SPEC,
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
        let artifacts = self.entrypoint_artifacts(command)?;
        Ok(json!({
            "kind": "mcp_stdio_server_entrypoint_validation",
            "server_name": SERVER_NAME,
            "host_mode": HOST_MODE,
            "governing_spec": GOVERNING_SPEC,
            "status": "valid",
            "request_path": artifacts.request_path,
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
            "governing_spec": GOVERNING_SPEC,
            "status": "completed",
            "request_path": artifacts.request_path,
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
            "governing_spec": GOVERNING_SPEC,
            "status": "rendered",
            "request_path": artifacts.request_path,
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
        let request_path = command.request_path.as_deref().ok_or_else(|| {
            StdioServerFailure::new("invalid_request", "command requires request_path.")
        })?;
        let request = load_runtime_request(request_path)?;
        self.validate_runtime_request(entrypoint_kind, id, version, &request)?;

        Ok(EntrypointArtifacts {
            request_path: request_path.to_string(),
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
            "governing_spec": GOVERNING_SPEC,
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
    request_path: String,
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
pub fn run_stdio_server(simulate_startup_failure: bool) -> Result<(), StdioServerFailure> {
    let canonical_execution = CanonicalExecutionContext::load_canonical()?;
    let catalog = McpDiscoveryCatalog::load_canonical()?;

    let capability_registry = Box::leak(Box::new(CapabilityRegistry::new()));
    let event_registry = Box::leak(Box::new(canonical_execution.events));
    let workflow_registry = Box::leak(Box::new(WorkflowRegistry::new()));

    let runtime = Box::leak(Box::new(
        Runtime::new(canonical_execution.capabilities, ExpeditionExampleExecutor)
            .with_workflow_registry(canonical_execution.workflows),
    ));
    let mcp = Box::leak(Box::new(TraverseMcp::new(
        capability_registry,
        event_registry,
        workflow_registry,
        runtime,
    )));
    let catalog = Box::leak(Box::new(catalog));
    let server = TraverseMcpStdioServer::new(mcp, catalog);

    let stdin = io::stdin();
    let stdout = io::stdout();
    let stderr = io::stderr();

    let mut stdout = stdout.lock();
    let mut stderr = stderr.lock();
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
    ) -> Result<Value, traverse_runtime::LocalExecutionFailure> {
        match capability.contract.id.as_str() {
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
        }
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
                .with_workflow_registry(execution.workflows),
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

        let missing_request_path = parse_command(
            r#"{"command":"validate_entrypoint","entrypoint_kind":"workflow","id":"expedition.planning.plan-expedition","version":"1.0.0"}"#,
        )
        .expect("command must parse");
        assert!(server.entrypoint_artifacts(&missing_request_path).is_err());
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
        let result = run_stdio_server(true);
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
}
