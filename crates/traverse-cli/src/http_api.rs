use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{IpAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use traverse_contracts::{CapabilityContract, EventContract, parse_contract, parse_event_contract};
use traverse_registry::{
    ArtifactDigests, BinaryFormat, BinaryReference, CapabilityArtifactRecord,
    CapabilityRegistration, CapabilityRegistry, ComposabilityMetadata, CompositionKind,
    CompositionPattern, DiscoveryQuery, EventRegistration, EventRegistry, ImplementationKind,
    LookupScope, RegistryProvenance, RegistryScope, SourceKind, SourceReference,
    WorkflowDefinition, WorkflowRegistration, WorkflowRegistry,
};
use traverse_runtime::{
    LocalExecutor, Runtime, RuntimeExecutionOutcome, RuntimeRequest, RuntimeResultStatus,
    RuntimeTrace, parse_runtime_request,
};

const MAX_REQUEST_BODY: usize = 4 * 1024 * 1024; // 4 MiB
const SYSTEM_WORKSPACE_ID: &str = "system";
const SYSTEM_ADMIN_SUBJECT: &str = "system_admin";
const PERSISTED_REGISTRY_SCHEMA_VERSION: &str = "1.0.0";
const WORKSPACE_METADATA_SCHEMA_VERSION: &str = "1.0.0";
const DEFAULT_WORKSPACE_ID: &str = "local-default";
const SERVER_DISCOVERY_SCHEMA_VERSION: &str = "1.0.0";
const DEFAULT_IDEMPOTENCY_RETENTION_SECONDS: u64 = 24 * 60 * 60;
const MIN_IDEMPOTENCY_RETENTION_SECONDS: u64 = 60;
const CORS_ALLOW_METHODS: &str = "GET, POST, OPTIONS";
const CORS_ALLOW_HEADERS: &str = "Authorization, Content-Type, Idempotency-Key, Prefer";
const CORS_MAX_AGE_SECONDS: &str = "600";

/// Errors that can occur while serving the HTTP/JSON API.
#[derive(Debug)]
pub enum ServeError {
    BindFailed(String),
    AcceptFailed(String),
}

impl std::fmt::Display for ServeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServeError::BindFailed(msg) => write!(f, "failed to bind HTTP/JSON API server: {msg}"),
            ServeError::AcceptFailed(msg) => {
                write!(f, "HTTP/JSON API server accept loop failed: {msg}")
            }
        }
    }
}

/// Configuration for the HTTP/JSON API server.
pub struct ApiServerConfig<E> {
    pub bind_address: String,
    pub allow_unauthenticated: bool,
    pub allowed_origins: Vec<String>,
    pub capability_registry: CapabilityRegistry,
    pub workflow_registry: WorkflowRegistry,
    pub registry_root: PathBuf,
    pub executor: E,
    /// Optional Idempotency-Key retention in seconds. Values below 60 seconds are floored to 60.
    pub idempotency_retention_seconds: Option<u64>,
}

struct ApiState<E> {
    allow_unauthenticated: bool,
    allowed_origins: Vec<String>,
    registry_root: PathBuf,
    executor: E,
    workspaces: RefCell<HashMap<String, WorkspaceState<E>>>,
    idempotency_records: RefCell<HashMap<String, IdempotencyRecord>>,
    idempotency_retention_seconds: u64,
}

struct WorkspaceState<E> {
    runtime: traverse_runtime::Runtime<E>,
    event_registry: EventRegistry,
    persisted: PersistedWorkspaceRegistryV1,
    loaded_from_disk: bool,
    executions: HashMap<String, ExecutionStatusRecord>,
    traces: HashMap<String, RuntimeTrace>,
}

#[derive(Debug, Clone)]
struct ExecutionStatusRecord {
    execution_id: String,
    status: String,
    created_at: String,
    updated_at: String,
}

#[derive(Debug, Clone)]
struct IdempotencyRecord {
    body_digest: String,
    status: u16,
    reason: String,
    content_type: String,
    body: Vec<u8>,
    stored_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedWorkspaceRegistryV1 {
    schema_version: String,
    registrations: Vec<PersistedCapabilityRegistrationV1>,
    #[serde(default)]
    events: Vec<PersistedEventRegistrationV1>,
    #[serde(default)]
    workflows: Vec<PersistedWorkflowRegistrationV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedCapabilityRegistrationV1 {
    registry_scope: String,
    contract: CapabilityContract,
    #[serde(default)]
    tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedEventRegistrationV1 {
    registry_scope: String,
    contract: EventContract,
    registered_at: String,
    validator_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedWorkflowRegistrationV1 {
    registry_scope: String,
    definition: WorkflowDefinition,
    registered_at: String,
    validator_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ServerDiscoveryV1 {
    schema_version: String,
    base_url: String,
    health_url: String,
    workspace_default: String,
    pid: u32,
    started_at: String,
    auth_mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    local_dev_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WorkspaceMetadataV1 {
    schema_version: String,
    workspace_id: String,
    owner_subject: String,
    shared: bool,
    #[serde(default)]
    members: Vec<String>,
}

#[derive(Debug, Clone)]
struct DerivedIdentity {
    subject_id: String,
    is_admin: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RegistrationScope {
    WorkspacePersisted,
    SessionEphemeral,
}

#[derive(Debug, Clone)]
struct ApiError {
    status: u16,
    reason: &'static str,
    code: &'static str,
    message: String,
}

enum WorkspaceOperation {
    Execute(String),
    RegisterCapability(String),
    RegisterEventContract(String),
    RegisterWorkflow(String),
    ExecutionStatus(String, String),
    Trace(String, String),
}

/// Start the HTTP/JSON API server, blocking until the listener fails.
///
/// # Errors
///
/// Returns [`ServeError`] when the server cannot bind or the accept loop fails.
pub fn serve_http_api<E>(config: ApiServerConfig<E>) -> Result<(), ServeError>
where
    E: LocalExecutor + Clone,
{
    let listener = TcpListener::bind(&config.bind_address)
        .map_err(|e| ServeError::BindFailed(format!("{}: {e}", config.bind_address)))?;

    let local_addr = listener
        .local_addr()
        .map_err(|e| ServeError::BindFailed(format!("could not read local address: {e}")))?;
    let auth_mode = if local_addr.ip().is_loopback() {
        "dev-loopback"
    } else {
        "bearer-required"
    };
    let local_dev_token = if local_addr.ip().is_loopback() {
        Some(mint_local_dev_token(&local_addr.to_string()))
    } else {
        None
    };

    if config.allow_unauthenticated {
        eprintln!(
            "WARNING: --allow-unauthenticated is set. Any caller on any network interface may \
             invoke this API without credentials. Do not use in production."
        );
    }

    eprintln!(
        "traverse-cli serve: HTTP/JSON API listening on http://{local_addr} (spec 033-http-json-api)"
    );
    let _ = std::io::stderr().flush();

    write_server_discovery(
        Path::new("."),
        &format!("http://{local_addr}"),
        auth_mode,
        local_dev_token.as_deref(),
    )
    .map_err(ServeError::BindFailed)?;

    let mut workspaces = HashMap::new();
    workspaces.insert(
        SYSTEM_WORKSPACE_ID.to_string(),
        WorkspaceState {
            runtime: Runtime::new(config.capability_registry, config.executor.clone())
                .with_workflow_registry(config.workflow_registry),
            event_registry: EventRegistry::new(),
            persisted: PersistedWorkspaceRegistryV1 {
                schema_version: PERSISTED_REGISTRY_SCHEMA_VERSION.to_string(),
                registrations: Vec::new(),
                events: Vec::new(),
                workflows: Vec::new(),
            },
            loaded_from_disk: true,
            executions: HashMap::new(),
            traces: HashMap::new(),
        },
    );

    let state = ApiState {
        allow_unauthenticated: config.allow_unauthenticated,
        allowed_origins: config.allowed_origins,
        registry_root: config.registry_root,
        executor: config.executor,
        workspaces: RefCell::new(workspaces),
        idempotency_records: RefCell::new(HashMap::new()),
        idempotency_retention_seconds: configured_idempotency_retention(
            config.idempotency_retention_seconds,
        ),
    };

    for connection in listener.incoming() {
        match connection {
            Ok(stream) => {
                if let Err(e) = handle_connection(stream, &state) {
                    eprintln!("traverse-cli serve: connection error: {e}");
                }
            }
            Err(e) => return Err(ServeError::AcceptFailed(e.to_string())),
        }
    }

    Ok(())
}

fn mint_local_dev_token(local_addr: &str) -> String {
    let now = unix_timestamp();
    format!(
        "trv_local_{}_{}",
        std::process::id(),
        crate::agent_packages::fnv1a64(format!("{local_addr}:{now}").as_bytes())
    )
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn configured_idempotency_retention(value: Option<u64>) -> u64 {
    value
        .unwrap_or(DEFAULT_IDEMPOTENCY_RETENTION_SECONDS)
        .max(MIN_IDEMPOTENCY_RETENTION_SECONDS)
}

fn write_server_discovery(
    repo_root: &Path,
    base_url: &str,
    auth_mode: &str,
    local_dev_token: Option<&str>,
) -> Result<PathBuf, String> {
    let traverse_dir = repo_root.join(".traverse");
    std::fs::create_dir_all(&traverse_dir)
        .map_err(|e| format!("failed to create .traverse directory: {e}"))?;
    let discovery_path = traverse_dir.join("server.json");
    let discovery = ServerDiscoveryV1 {
        schema_version: SERVER_DISCOVERY_SCHEMA_VERSION.to_string(),
        base_url: base_url.to_string(),
        health_url: format!("{base_url}/healthz"),
        workspace_default: DEFAULT_WORKSPACE_ID.to_string(),
        pid: std::process::id(),
        started_at: generated_registered_at().map_err(|e| e.message)?,
        auth_mode: auth_mode.to_string(),
        local_dev_token: local_dev_token.map(str::to_string),
    };
    let body = serde_json::to_vec_pretty(&discovery)
        .map_err(|e| format!("failed to serialize server discovery file: {e}"))?;
    std::fs::write(&discovery_path, body)
        .map_err(|e| format!("failed to write {}: {e}", discovery_path.display()))?;
    if local_dev_token.is_some() {
        set_owner_read_write(&discovery_path)?;
    }
    Ok(discovery_path)
}

#[cfg(unix)]
fn set_owner_read_write(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(|e| {
        format!(
            "failed to set owner-only permissions on {}: {e}",
            path.display()
        )
    })
}

#[cfg(not(unix))]
fn set_owner_read_write(_path: &Path) -> Result<(), String> {
    Ok(())
}

/// In-process wrapper around the HTTP/JSON API handlers, used by `traverse-cli`
/// subcommands that must delegate to the canonical server code paths.
pub struct InProcessApi<E> {
    state: ApiState<E>,
}

impl<E> InProcessApi<E>
where
    E: LocalExecutor + Clone,
{
    #[must_use]
    pub fn new(config: ApiServerConfig<E>) -> Self {
        let mut workspaces = HashMap::new();
        workspaces.insert(
            SYSTEM_WORKSPACE_ID.to_string(),
            WorkspaceState {
                runtime: Runtime::new(config.capability_registry, config.executor.clone())
                    .with_workflow_registry(config.workflow_registry),
                event_registry: EventRegistry::new(),
                persisted: PersistedWorkspaceRegistryV1 {
                    schema_version: PERSISTED_REGISTRY_SCHEMA_VERSION.to_string(),
                    registrations: Vec::new(),
                    events: Vec::new(),
                    workflows: Vec::new(),
                },
                loaded_from_disk: false,
                executions: HashMap::new(),
                traces: HashMap::new(),
            },
        );

        Self {
            state: ApiState {
                allow_unauthenticated: config.allow_unauthenticated,
                allowed_origins: config.allowed_origins,
                registry_root: config.registry_root,
                executor: config.executor,
                workspaces: RefCell::new(workspaces),
                idempotency_records: RefCell::new(HashMap::new()),
                idempotency_retention_seconds: configured_idempotency_retention(
                    config.idempotency_retention_seconds,
                ),
            },
        }
    }

    pub fn register_workflow(&self, body: Vec<u8>, loopback: bool) -> Result<(u16, Value), String> {
        let request = HttpRequest {
            method: "POST".to_string(),
            path: "/v1/workflows/register".to_string(),
            query: HashMap::new(),
            headers: HashMap::new(),
            body,
        };
        let mut out = Vec::new();
        handle_register_workflow(&mut out, &request, &self.state, loopback)?;
        parse_http_json_response(&out)
    }

    pub fn list_workflows(
        &self,
        workspace_id: &str,
        loopback: bool,
    ) -> Result<(u16, Value), String> {
        let mut query = HashMap::new();
        query.insert("workspace_id".to_string(), workspace_id.to_string());
        let request = HttpRequest {
            method: "GET".to_string(),
            path: "/v1/workflows".to_string(),
            query,
            headers: HashMap::new(),
            body: Vec::new(),
        };
        let mut out = Vec::new();
        handle_list_workflows(&mut out, &request, &self.state, loopback)?;
        parse_http_json_response(&out)
    }

    pub fn get_workflow(
        &self,
        workspace_id: &str,
        workflow_id: &str,
        version: Option<&str>,
        loopback: bool,
    ) -> Result<(u16, Value), String> {
        let mut query = HashMap::new();
        query.insert("workspace_id".to_string(), workspace_id.to_string());
        if let Some(version) = version {
            query.insert("version".to_string(), version.to_string());
        }
        let request = HttpRequest {
            method: "GET".to_string(),
            path: format!("/v1/workflows/{workflow_id}"),
            query,
            headers: HashMap::new(),
            body: Vec::new(),
        };
        let mut out = Vec::new();
        handle_get_workflow(&mut out, &request, &self.state, loopback, workflow_id)?;
        parse_http_json_response(&out)
    }
}

impl<E> ApiState<E>
where
    E: LocalExecutor + Clone,
{
    fn with_workspace_mut<R>(
        &self,
        workspace_id: &str,
        f: impl FnOnce(&mut WorkspaceState<E>) -> Result<R, String>,
    ) -> Result<R, String> {
        let mut workspaces = self.workspaces.borrow_mut();
        let entry = workspaces
            .entry(workspace_id.to_string())
            .or_insert_with(|| WorkspaceState {
                runtime: Runtime::new(CapabilityRegistry::new(), self.executor.clone())
                    .with_workflow_registry(WorkflowRegistry::new()),
                event_registry: EventRegistry::new(),
                persisted: PersistedWorkspaceRegistryV1 {
                    schema_version: PERSISTED_REGISTRY_SCHEMA_VERSION.to_string(),
                    registrations: Vec::new(),
                    events: Vec::new(),
                    workflows: Vec::new(),
                },
                loaded_from_disk: false,
                executions: HashMap::new(),
                traces: HashMap::new(),
            });

        if !entry.loaded_from_disk {
            entry.persisted = load_persisted_registry(&self.registry_root, workspace_id)?;
            for persisted in entry.persisted.registrations.clone() {
                let registration = derive_registration(workspace_id, &persisted).map_err(|e| {
                    format!("persisted registry contains invalid entry: {}", e.message)
                })?;
                let _ = entry
                    .runtime
                    .register_capability(registration)
                    .map_err(render_registry_failure_as_string)?;
            }
            for persisted in entry.persisted.events.clone() {
                let registration =
                    derive_event_registration(workspace_id, &persisted).map_err(|e| {
                        format!("persisted registry contains invalid event: {}", e.message)
                    })?;
                let _ = entry
                    .event_registry
                    .register(registration)
                    .map_err(render_event_registry_failure_as_string)?;
            }
            for persisted in entry.persisted.workflows.clone() {
                let registration = derive_workflow_registration(workspace_id, &persisted)
                    .map_err(|e| format!("persisted registry contains invalid workflow: {e:?}"))?;
                let _ = entry
                    .runtime
                    .register_workflow(registration)
                    .map_err(render_workflow_failure_as_string)?;
            }
            entry.loaded_from_disk = true;
        }

        f(entry)
    }
}

fn parse_http_json_response(bytes: &[u8]) -> Result<(u16, Value), String> {
    let text = std::str::from_utf8(bytes).map_err(|e| format!("response not UTF-8: {e}"))?;
    let status_line = text
        .lines()
        .next()
        .ok_or_else(|| "response missing status line".to_string())?;
    let mut parts = status_line.split_whitespace();
    let _proto = parts
        .next()
        .ok_or_else(|| "response status line missing protocol".to_string())?;
    let status = parts
        .next()
        .ok_or_else(|| "response status line missing status code".to_string())?
        .parse::<u16>()
        .map_err(|_| "response status code is not a u16".to_string())?;

    let header_end = text
        .find("\r\n\r\n")
        .ok_or_else(|| "response missing header terminator".to_string())?;
    let body = &bytes[header_end + 4..];
    let value: Value =
        serde_json::from_slice(body).map_err(|e| format!("invalid JSON response body: {e}"))?;
    Ok((status, value))
}

fn load_persisted_registry(
    registry_root: &Path,
    workspace_id: &str,
) -> Result<PersistedWorkspaceRegistryV1, String> {
    let path = persisted_registry_path(registry_root, workspace_id);
    if !path.exists() {
        return Ok(PersistedWorkspaceRegistryV1 {
            schema_version: PERSISTED_REGISTRY_SCHEMA_VERSION.to_string(),
            registrations: Vec::new(),
            events: Vec::new(),
            workflows: Vec::new(),
        });
    }

    let bytes =
        std::fs::read(&path).map_err(|e| format!("failed to read persisted registry: {e}"))?;
    let persisted: PersistedWorkspaceRegistryV1 = serde_json::from_slice(&bytes).map_err(|e| {
        format!(
            "failed to parse persisted registry at {}: {e}",
            path.display()
        )
    })?;
    Ok(persisted)
}

fn persisted_registry_path(registry_root: &Path, workspace_id: &str) -> PathBuf {
    registry_root
        .join("workspaces")
        .join(workspace_id)
        .join("capabilities.json")
}

fn workspace_metadata_path(registry_root: &Path, workspace_id: &str) -> PathBuf {
    registry_root
        .join("workspaces")
        .join(workspace_id)
        .join("workspace.json")
}

fn persist_registry(
    registry_root: &Path,
    workspace_id: &str,
    persisted: &PersistedWorkspaceRegistryV1,
) -> Result<(), String> {
    let path = persisted_registry_path(registry_root, workspace_id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create persisted registry directory: {e}"))?;
    }

    let bytes = serde_json::to_vec_pretty(persisted)
        .map_err(|e| format!("failed to serialize persisted registry: {e}"))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, &bytes)
        .map_err(|e| format!("failed to write persisted registry temp file: {e}"))?;
    std::fs::rename(&tmp, &path)
        .map_err(|e| format!("failed to atomically replace persisted registry: {e}"))?;
    Ok(())
}

fn render_registry_failure_as_string(failure: traverse_registry::RegistryFailure) -> String {
    use std::fmt::Write as _;

    let mut rendered = String::new();
    for err in failure.errors {
        let _ = write!(
            &mut rendered,
            "{:?} at {}: {}; ",
            err.code, err.target, err.message
        );
    }
    rendered
}

fn render_event_registry_failure_as_string(
    failure: traverse_registry::EventRegistryFailure,
) -> String {
    use std::fmt::Write as _;

    let mut rendered = String::new();
    for err in failure.errors {
        let _ = write!(
            &mut rendered,
            "{:?} at {}: {}; ",
            err.code, err.target, err.message
        );
    }
    rendered
}

fn render_workflow_failure_as_string(failure: traverse_registry::WorkflowFailure) -> String {
    use std::fmt::Write as _;

    let mut rendered = String::new();
    for err in failure.errors {
        let _ = write!(
            &mut rendered,
            "{:?} at {}: {}; ",
            err.code, err.path, err.message
        );
    }
    rendered
}

fn generated_registered_at() -> Result<String, ApiError> {
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| ApiError {
            status: 500,
            reason: "Internal Server Error",
            code: "internal_error",
            message: format!("failed to read system time: {e}"),
        })?
        .as_secs();
    Ok(format!("unix:{now_secs}"))
}

fn validate_workspace_id(workspace_id: &str) -> Result<(), String> {
    if workspace_id.trim().is_empty() {
        return Err("workspace_id must be non-empty".to_string());
    }
    if workspace_id.len() > 128 {
        return Err("workspace_id must be at most 128 characters".to_string());
    }
    if workspace_id.contains('\0') {
        return Err("workspace_id must not contain null bytes".to_string());
    }

    // Conservative allowlist: avoids path traversal and injection into on-disk layout.
    for ch in workspace_id.chars() {
        let ok = ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.');
        if !ok {
            return Err(
                "workspace_id may contain only ASCII letters, digits, '-', '_', and '.'"
                    .to_string(),
            );
        }
    }
    Ok(())
}

fn require_workspace_id_query(request: &HttpRequest) -> Result<String, ApiError> {
    request
        .query
        .get("workspace_id")
        .cloned()
        .ok_or_else(|| ApiError {
            status: 400,
            reason: "Bad Request",
            code: "workspace_id_required",
            message: "workspace_id is required (add ?workspace_id=<id>)".to_string(),
        })
}

fn subject_from_request(
    headers: &HashMap<String, String>,
    allow_unauthenticated: bool,
    loopback: bool,
) -> Result<DerivedIdentity, ApiError> {
    let token = headers
        .get("authorization")
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToString::to_string);

    if let Some(token) = token {
        if let Some(identity) = derive_identity_from_jwt(&token)? {
            return Ok(identity);
        }

        // Fallback: accept non-JWT bearer tokens as direct subject identifiers.
        // This is intended for local/dev environments that don't provide JWTs yet.
        validate_subject_id(&token).map_err(|msg| ApiError {
            status: 401,
            reason: "Unauthorized",
            code: "unauthorized",
            message: msg,
        })?;

        return Ok(DerivedIdentity {
            subject_id: token.clone(),
            is_admin: token == SYSTEM_ADMIN_SUBJECT,
        });
    }

    if allow_unauthenticated || loopback {
        return Ok(DerivedIdentity {
            subject_id: "local".to_string(),
            is_admin: false,
        });
    }

    Err(ApiError {
        status: 401,
        reason: "Unauthorized",
        code: "unauthorized",
        message: "Bearer token required".to_string(),
    })
}

fn validate_subject_id(subject_id: &str) -> Result<(), String> {
    if subject_id.trim().is_empty() {
        return Err("subject_id must be non-empty".to_string());
    }
    if subject_id.len() > 256 {
        return Err("subject_id must be at most 256 characters".to_string());
    }
    if subject_id.contains('\0') {
        return Err("subject_id must not contain null bytes".to_string());
    }
    Ok(())
}

fn derive_identity_from_jwt(token: &str) -> Result<Option<DerivedIdentity>, ApiError> {
    let mut parts = token.split('.');
    let header = parts.next();
    let payload = parts.next();
    let signature = parts.next();

    if header.is_none() || payload.is_none() || signature.is_none() || parts.next().is_some() {
        return Ok(None);
    }

    let Some(payload_b64) = payload else {
        return Ok(None);
    };
    let payload_bytes = base64url_decode(payload_b64).map_err(|msg| ApiError {
        status: 401,
        reason: "Unauthorized",
        code: "unauthorized",
        message: msg,
    })?;

    let value: Value = serde_json::from_slice(&payload_bytes).map_err(|e| ApiError {
        status: 401,
        reason: "Unauthorized",
        code: "unauthorized",
        message: format!("invalid JWT payload: {e}"),
    })?;

    let subject_id = value
        .get("sub")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| ApiError {
            status: 401,
            reason: "Unauthorized",
            code: "unauthorized",
            message: "JWT missing required 'sub' claim".to_string(),
        })?
        .to_string();

    validate_subject_id(&subject_id).map_err(|msg| ApiError {
        status: 401,
        reason: "Unauthorized",
        code: "unauthorized",
        message: msg,
    })?;

    if let Some(exp) = value.get("exp").and_then(Value::as_i64) {
        if exp <= 0 {
            return Err(ApiError {
                status: 401,
                reason: "Unauthorized",
                code: "token_expired",
                message: "token is expired".to_string(),
            });
        }

        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| ApiError {
                status: 500,
                reason: "Internal Server Error",
                code: "internal_error",
                message: format!("failed to read system time: {e}"),
            })?
            .as_secs();

        let now = i64::try_from(now_secs).map_err(|_| ApiError {
            status: 500,
            reason: "Internal Server Error",
            code: "internal_error",
            message: "system time overflow".to_string(),
        })?;

        if now > exp {
            return Err(ApiError {
                status: 401,
                reason: "Unauthorized",
                code: "token_expired",
                message: "token is expired".to_string(),
            });
        }
    }

    let is_admin = value
        .get("traverse_admin")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || value
            .get("roles")
            .and_then(Value::as_array)
            .is_some_and(|arr| {
                arr.iter().any(|v| {
                    v.as_str()
                        .is_some_and(|s| s == "traverse_admin" || s == SYSTEM_ADMIN_SUBJECT)
                })
            })
        || value
            .get("role")
            .and_then(|v| v.as_str())
            .is_some_and(|s| s == "traverse_admin" || s == SYSTEM_ADMIN_SUBJECT);

    Ok(Some(DerivedIdentity {
        subject_id,
        is_admin,
    }))
}

fn base64url_decode(input: &str) -> Result<Vec<u8>, String> {
    if input.is_empty() {
        return Ok(Vec::new());
    }
    if input.contains('=') {
        return Err("base64url input must not include '=' padding".to_string());
    }

    let mut sextets = Vec::with_capacity(input.len());
    for ch in input.chars() {
        let val = match ch {
            'A'..='Z' => (ch as u8) - b'A',
            'a'..='z' => (ch as u8) - b'a' + 26,
            '0'..='9' => (ch as u8) - b'0' + 52,
            '-' => 62,
            '_' => 63,
            _ => {
                return Err("base64url input contains invalid characters".to_string());
            }
        };
        sextets.push(val);
    }

    match sextets.len() % 4 {
        0 | 2 | 3 => {}
        _ => return Err("base64url input has invalid length".to_string()),
    }

    let mut out = Vec::with_capacity((sextets.len() * 3) / 4);
    let mut i = 0;
    while i + 4 <= sextets.len() {
        let n = (u32::from(sextets[i]) << 18)
            | (u32::from(sextets[i + 1]) << 12)
            | (u32::from(sextets[i + 2]) << 6)
            | u32::from(sextets[i + 3]);
        out.push(((n >> 16) & 0xff) as u8);
        out.push(((n >> 8) & 0xff) as u8);
        out.push((n & 0xff) as u8);
        i += 4;
    }

    let rem = sextets.len() - i;
    if rem == 2 {
        let n = (u32::from(sextets[i]) << 18) | (u32::from(sextets[i + 1]) << 12);
        out.push(((n >> 16) & 0xff) as u8);
    } else if rem == 3 {
        let n = (u32::from(sextets[i]) << 18)
            | (u32::from(sextets[i + 1]) << 12)
            | (u32::from(sextets[i + 2]) << 6);
        out.push(((n >> 16) & 0xff) as u8);
        out.push(((n >> 8) & 0xff) as u8);
    }

    Ok(out)
}

fn load_workspace_metadata(
    registry_root: &Path,
    workspace_id: &str,
) -> Result<Option<WorkspaceMetadataV1>, ApiError> {
    let path = workspace_metadata_path(registry_root, workspace_id);
    if !path.exists() {
        return Ok(None);
    }

    let bytes = std::fs::read(&path).map_err(|e| ApiError {
        status: 500,
        reason: "Internal Server Error",
        code: "workspace_metadata_read_failed",
        message: format!("failed to read workspace metadata: {e}"),
    })?;

    let metadata: WorkspaceMetadataV1 = serde_json::from_slice(&bytes).map_err(|e| ApiError {
        status: 500,
        reason: "Internal Server Error",
        code: "workspace_metadata_parse_failed",
        message: format!("failed to parse workspace metadata: {e}"),
    })?;

    Ok(Some(metadata))
}

fn persist_workspace_metadata(
    registry_root: &Path,
    workspace_id: &str,
    metadata: &WorkspaceMetadataV1,
) -> Result<(), ApiError> {
    let path = workspace_metadata_path(registry_root, workspace_id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| ApiError {
            status: 500,
            reason: "Internal Server Error",
            code: "workspace_metadata_write_failed",
            message: format!("failed to create workspace directory: {e}"),
        })?;
    }

    let bytes = serde_json::to_vec_pretty(metadata).map_err(|e| ApiError {
        status: 500,
        reason: "Internal Server Error",
        code: "workspace_metadata_write_failed",
        message: format!("failed to serialize workspace metadata: {e}"),
    })?;

    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, &bytes).map_err(|e| ApiError {
        status: 500,
        reason: "Internal Server Error",
        code: "workspace_metadata_write_failed",
        message: format!("failed to write workspace metadata temp file: {e}"),
    })?;
    std::fs::rename(&tmp, &path).map_err(|e| ApiError {
        status: 500,
        reason: "Internal Server Error",
        code: "workspace_metadata_write_failed",
        message: format!("failed to atomically replace workspace metadata: {e}"),
    })?;

    Ok(())
}

fn ensure_workspace_access(
    registry_root: &Path,
    workspace_id: &str,
    identity: &DerivedIdentity,
) -> Result<WorkspaceMetadataV1, ApiError> {
    if workspace_id == SYSTEM_WORKSPACE_ID && !identity.is_admin {
        return Err(ApiError {
            status: 403,
            reason: "Forbidden",
            code: "insufficient_privileges",
            message: "system workspace requires privileged role claim".to_string(),
        });
    }

    validate_workspace_id(workspace_id).map_err(|msg| ApiError {
        status: 400,
        reason: "Bad Request",
        code: "workspace_id_invalid",
        message: msg,
    })?;

    let existing = load_workspace_metadata(registry_root, workspace_id)?;
    let metadata = if let Some(metadata) = existing {
        metadata
    } else {
        let metadata = WorkspaceMetadataV1 {
            schema_version: WORKSPACE_METADATA_SCHEMA_VERSION.to_string(),
            workspace_id: workspace_id.to_string(),
            owner_subject: identity.subject_id.clone(),
            shared: false,
            members: Vec::new(),
        };
        persist_workspace_metadata(registry_root, workspace_id, &metadata)?;
        metadata
    };

    if metadata.shared {
        if metadata.owner_subject == identity.subject_id
            || metadata.members.iter().any(|m| m == &identity.subject_id)
        {
            return Ok(metadata);
        }
    } else if metadata.owner_subject == identity.subject_id {
        return Ok(metadata);
    }

    Err(ApiError {
        status: 403,
        reason: "Forbidden",
        code: "unauthorized_workspace",
        message: "subject is not authorized for workspace".to_string(),
    })
}

fn parse_registration_scope(value: Option<&Value>) -> Result<RegistrationScope, String> {
    let Some(value) = value else {
        return Ok(RegistrationScope::WorkspacePersisted);
    };
    let Some(scope) = value.as_str() else {
        return Err("scope must be a string".to_string());
    };
    match scope {
        "workspace_persisted" => Ok(RegistrationScope::WorkspacePersisted),
        "session_ephemeral" => Ok(RegistrationScope::SessionEphemeral),
        _ => Err("scope must be workspace_persisted or session_ephemeral".to_string()),
    }
}

fn map_registry_failure_http(
    failure: &traverse_registry::RegistryFailure,
) -> (u16, &'static str, &'static str) {
    use traverse_registry::RegistryErrorCode;

    let mut has_immutable = false;
    let mut has_registration_conflict = false;
    for err in &failure.errors {
        if err.code == RegistryErrorCode::ImmutableVersionConflict {
            has_immutable = true;
        }
        if err.code == RegistryErrorCode::ArtifactConflict
            || err
                .message
                .contains("published contract versions are immutable")
        {
            has_registration_conflict = true;
        }
    }

    if has_immutable {
        return (409, "immutable_version_conflict", "Conflict");
    }
    if has_registration_conflict {
        return (409, "registration_conflict", "Conflict");
    }

    (422, "registration_failed", "Unprocessable Entity")
}

fn map_event_registry_failure_http(
    failure: &traverse_registry::EventRegistryFailure,
) -> (u16, &'static str, &'static str) {
    use traverse_registry::EventRegistryErrorCode;

    if failure
        .errors
        .iter()
        .any(|err| err.code == EventRegistryErrorCode::ImmutableVersionConflict)
    {
        return (409, "registration_conflict", "Conflict");
    }

    (422, "event_registration_failed", "Unprocessable Entity")
}

fn map_workflow_failure_http(
    failure: &traverse_registry::WorkflowFailure,
    definition: &WorkflowDefinition,
) -> (u16, &'static str, &'static str, Option<Value>) {
    use traverse_registry::WorkflowErrorCode;

    let mut has_immutable = false;
    let mut has_cycle = false;
    let mut has_edge_schema_mismatch = false;
    let mut has_missing_reference = false;
    let mut has_empty_nodes = false;

    for err in &failure.errors {
        match err.code {
            WorkflowErrorCode::ImmutableVersionConflict => has_immutable = true,
            WorkflowErrorCode::DeterministicCycleNotAllowed => has_cycle = true,
            WorkflowErrorCode::EdgeSchemaMismatch => has_edge_schema_mismatch = true,
            WorkflowErrorCode::MissingReference => has_missing_reference = true,
            WorkflowErrorCode::MissingRequiredField => {
                if err.path == "$.nodes" {
                    has_empty_nodes = true;
                }
            }
            _ => {}
        }
    }

    if has_immutable {
        return (409, "immutable_version_conflict", "Conflict", None);
    }

    if has_cycle {
        let path = find_cycle_path(definition);
        return (
            422,
            "workflow_cycle_detected",
            "Unprocessable Entity",
            Some(json!({ "cycle_path": path })),
        );
    }

    if has_edge_schema_mismatch {
        return (422, "edge_schema_mismatch", "Unprocessable Entity", None);
    }

    if has_missing_reference {
        return (
            422,
            "unresolved_capability_reference",
            "Unprocessable Entity",
            None,
        );
    }

    if has_empty_nodes {
        return (422, "empty_workflow", "Unprocessable Entity", None);
    }

    (422, "registration_failed", "Unprocessable Entity", None)
}

fn find_cycle_path(definition: &WorkflowDefinition) -> Vec<String> {
    use std::collections::BTreeMap;

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Mark {
        Visiting,
        Done,
    }

    fn dfs(
        node: &str,
        adjacency: &std::collections::BTreeMap<String, Vec<String>>,
        marks: &mut std::collections::BTreeMap<String, Mark>,
        stack: &mut Vec<String>,
    ) -> Option<Vec<String>> {
        marks.insert(node.to_string(), Mark::Visiting);
        stack.push(node.to_string());

        if let Some(neighbors) = adjacency.get(node) {
            for next in neighbors {
                match marks.get(next.as_str()).copied() {
                    Some(Mark::Visiting) => {
                        if let Some(pos) = stack.iter().position(|v| v == next) {
                            let mut cycle = stack[pos..].to_vec();
                            cycle.push(next.clone());
                            return Some(cycle);
                        }
                        return Some(vec![next.clone(), next.clone()]);
                    }
                    Some(Mark::Done) => {}
                    None => {
                        if let Some(found) = dfs(next, adjacency, marks, stack) {
                            return Some(found);
                        }
                    }
                }
            }
        }

        stack.pop();
        marks.insert(node.to_string(), Mark::Done);
        None
    }

    let node_ids = definition
        .nodes
        .iter()
        .map(|node| node.node_id.clone())
        .collect::<std::collections::BTreeSet<_>>();
    let mut adjacency: BTreeMap<String, Vec<String>> =
        node_ids.iter().map(|id| (id.clone(), Vec::new())).collect();

    for edge in &definition.edges {
        if node_ids.contains(&edge.from) && node_ids.contains(&edge.to) {
            adjacency
                .entry(edge.from.clone())
                .or_default()
                .push(edge.to.clone());
        }
    }

    for neighbors in adjacency.values_mut() {
        neighbors.sort();
    }

    let mut marks = BTreeMap::<String, Mark>::new();
    for node in &node_ids {
        if marks.contains_key(node) {
            continue;
        }
        let mut stack = Vec::new();
        if let Some(found) = dfs(node, &adjacency, &mut marks, &mut stack) {
            return found;
        }
    }

    Vec::new()
}

fn derive_registration(
    workspace_id: &str,
    persisted: &PersistedCapabilityRegistrationV1,
) -> Result<CapabilityRegistration, ApiError> {
    let registry_scope = match persisted.registry_scope.as_str() {
        "public" => RegistryScope::Public,
        "private" => RegistryScope::Private,
        other => {
            return Err(ApiError {
                status: 422,
                reason: "Unprocessable Entity",
                code: "invalid_registry_scope",
                message: format!("registry_scope must be public or private (got {other})"),
            });
        }
    };

    let contract = persisted.contract.clone();
    let entrypoint = contract.execution.entrypoint.command.clone();
    let binary_path = PathBuf::from(&entrypoint);
    if !binary_path.exists() {
        return Err(ApiError {
            status: 422,
            reason: "Unprocessable Entity",
            code: "artifact_not_found",
            message: format!("binary artifact not found at {entrypoint}"),
        });
    }

    let artifact_ref = format!(
        "workspace:{workspace_id}:{}:{}",
        contract.id, contract.version
    );
    let source_digest = format!("sha256:source-{}-{}", contract.id, contract.version);
    let binary_digest = format!("sha256:binary-{}-{}", contract.id, contract.version);

    Ok(CapabilityRegistration {
        scope: registry_scope,
        contract_path: format!(
            "workspaces/{workspace_id}/registry/{}/{}@{}/contract.json",
            format!("{registry_scope:?}").to_lowercase(),
            contract.id,
            contract.version
        ),
        contract,
        artifact: CapabilityArtifactRecord {
            artifact_ref,
            implementation_kind: ImplementationKind::Executable,
            source: SourceReference {
                kind: SourceKind::Local,
                location: entrypoint.clone(),
            },
            binary: Some(BinaryReference {
                format: BinaryFormat::Wasm,
                location: entrypoint,
            }),
            workflow_ref: None,
            digests: ArtifactDigests {
                source_digest,
                binary_digest: Some(binary_digest),
            },
            provenance: RegistryProvenance {
                source: "programmatic_registration".to_string(),
                author: persisted.contract.provenance.author.clone(),
                created_at: persisted.contract.provenance.created_at.clone(),
            },
        },
        registered_at: persisted.contract.provenance.created_at.clone(),
        tags: persisted.tags.clone(),
        composability: ComposabilityMetadata {
            kind: CompositionKind::Atomic,
            patterns: vec![CompositionPattern::Sequential],
            provides: Vec::new(),
            requires: Vec::new(),
        },
        governing_spec: "034-programmatic-registration".to_string(),
        validator_version: "traverse-cli".to_string(),
    })
}

fn derive_event_registration(
    workspace_id: &str,
    persisted: &PersistedEventRegistrationV1,
) -> Result<EventRegistration, ApiError> {
    let registry_scope = match persisted.registry_scope.as_str() {
        "public" => RegistryScope::Public,
        "private" => RegistryScope::Private,
        other => {
            return Err(ApiError {
                status: 422,
                reason: "Unprocessable Entity",
                code: "invalid_registry_scope",
                message: format!("registry_scope must be public or private (got {other})"),
            });
        }
    };

    Ok(EventRegistration {
        scope: registry_scope,
        contract: persisted.contract.clone(),
        contract_path: format!(
            "workspaces/{workspace_id}/events/{}/{}@{}/event.json",
            format!("{registry_scope:?}").to_lowercase(),
            persisted.contract.id,
            persisted.contract.version
        ),
        registered_at: persisted.registered_at.clone(),
        governing_spec: "011-event-registry".to_string(),
        validator_version: persisted.validator_version.clone(),
    })
}

fn derive_workflow_registration(
    workspace_id: &str,
    persisted: &PersistedWorkflowRegistrationV1,
) -> Result<WorkflowRegistration, ApiError> {
    let registry_scope = match persisted.registry_scope.as_str() {
        "public" => RegistryScope::Public,
        "private" => RegistryScope::Private,
        other => {
            return Err(ApiError {
                status: 422,
                reason: "Unprocessable Entity",
                code: "invalid_registry_scope",
                message: format!("registry_scope must be public or private (got {other})"),
            });
        }
    };

    Ok(WorkflowRegistration {
        scope: registry_scope,
        definition: persisted.definition.clone(),
        workflow_path: format!(
            "workspaces/{workspace_id}/workflows/{}/{}@{}/workflow.json",
            format!("{registry_scope:?}").to_lowercase(),
            persisted.definition.id,
            persisted.definition.version
        ),
        registered_at: persisted.registered_at.clone(),
        validator_version: persisted.validator_version.clone(),
    })
}

fn parse_register_body(
    body: &[u8],
) -> Result<(String, RegistrationScope, PersistedCapabilityRegistrationV1), ApiError> {
    parse_register_body_with_workspace(body, None, false)
}

fn parse_register_body_for_workspace(
    body: &[u8],
    workspace_id: &str,
) -> Result<(RegistrationScope, PersistedCapabilityRegistrationV1), ApiError> {
    let (parsed_workspace_id, scope, registration) =
        parse_register_body_with_workspace(body, Some(workspace_id), true)?;
    if parsed_workspace_id != workspace_id {
        return Err(ApiError {
            status: 400,
            reason: "Bad Request",
            code: "invalid_workspace_id",
            message: "body workspace_id must match URL workspace_id".to_string(),
        });
    }
    Ok((scope, registration))
}

fn parse_event_register_body_for_workspace(
    body: &[u8],
    workspace_id: &str,
) -> Result<(RegistrationScope, PersistedEventRegistrationV1), ApiError> {
    let body_str = std::str::from_utf8(body).map_err(|e| ApiError {
        status: 400,
        reason: "Bad Request",
        code: "invalid_request",
        message: format!("request body is not valid UTF-8: {e}"),
    })?;

    let value: Value = serde_json::from_str(body_str).map_err(|e| ApiError {
        status: 400,
        reason: "Bad Request",
        code: "invalid_request",
        message: format!("invalid JSON body: {e}"),
    })?;

    reject_unknown_event_registration_wrapper_fields(&value)?;
    let parsed_workspace_id = registration_workspace_id(&value, Some(workspace_id))?;
    if parsed_workspace_id != workspace_id {
        return Err(ApiError {
            status: 400,
            reason: "Bad Request",
            code: "invalid_workspace_id",
            message: "body workspace_id must match URL workspace_id".to_string(),
        });
    }

    let scope = parse_registration_scope(value.get("scope")).map_err(|msg| ApiError {
        status: 422,
        reason: "Unprocessable Entity",
        code: "invalid_scope",
        message: msg,
    })?;

    let contract_value = value.get("event_contract").ok_or_else(|| ApiError {
        status: 422,
        reason: "Unprocessable Entity",
        code: "invalid_event_contract",
        message: "event_contract is required".to_string(),
    })?;
    let contract_json = serde_json::to_string(contract_value).map_err(|e| ApiError {
        status: 422,
        reason: "Unprocessable Entity",
        code: "invalid_event_contract",
        message: format!("failed to serialize event contract: {e}"),
    })?;
    let contract = parse_event_contract(&contract_json).map_err(|failure| ApiError {
        status: 422,
        reason: "Unprocessable Entity",
        code: "event_contract_validation_failed",
        message: format!("event contract could not be parsed: {failure:?}"),
    })?;

    let registry_scope = value
        .get("registry_scope")
        .and_then(|v| v.as_str())
        .unwrap_or("private")
        .to_string();
    let registered_at = generated_registered_at().unwrap_or_else(|_| "unix:0".to_string());

    Ok((
        scope,
        PersistedEventRegistrationV1 {
            registry_scope,
            contract,
            registered_at,
            validator_version: env!("CARGO_PKG_VERSION").to_string(),
        },
    ))
}

fn parse_register_body_with_workspace(
    body: &[u8],
    path_workspace_id: Option<&str>,
    reject_unknown_wrapper_fields: bool,
) -> Result<(String, RegistrationScope, PersistedCapabilityRegistrationV1), ApiError> {
    let body_str = std::str::from_utf8(body).map_err(|e| ApiError {
        status: 400,
        reason: "Bad Request",
        code: "invalid_request",
        message: format!("request body is not valid UTF-8: {e}"),
    })?;

    let value: Value = serde_json::from_str(body_str).map_err(|e| ApiError {
        status: 400,
        reason: "Bad Request",
        code: "invalid_request",
        message: format!("invalid JSON body: {e}"),
    })?;

    if reject_unknown_wrapper_fields && value.get("contract").is_some() {
        reject_unknown_registration_wrapper_fields(&value)?;
    }

    let workspace_id = registration_workspace_id(&value, path_workspace_id)?;

    let scope = parse_registration_scope(value.get("scope")).map_err(|msg| ApiError {
        status: 422,
        reason: "Unprocessable Entity",
        code: "invalid_scope",
        message: msg,
    })?;

    let contract_value = if value
        .get("kind")
        .and_then(|v| v.as_str())
        .is_some_and(|k| k == "capability_contract")
    {
        value.clone()
    } else if let Some(contract) = value.get("contract") {
        contract.clone()
    } else {
        return Err(ApiError {
            status: 422,
            reason: "Unprocessable Entity",
            code: "invalid_contract",
            message: "expected body to be a capability contract or to contain a `contract` field"
                .to_string(),
        });
    };

    let contract_json = serde_json::to_string(&contract_value).map_err(|e| ApiError {
        status: 422,
        reason: "Unprocessable Entity",
        code: "invalid_contract",
        message: format!("failed to serialize contract: {e}"),
    })?;

    let contract: CapabilityContract =
        parse_contract(&contract_json).map_err(|failure| ApiError {
            status: 422,
            reason: "Unprocessable Entity",
            code: "contract_validation_failed",
            message: format!("contract could not be parsed: {failure:?}"),
        })?;

    let registry_scope = value
        .get("registry_scope")
        .and_then(|v| v.as_str())
        .unwrap_or("private")
        .to_string();

    let tags = value
        .get("tags")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(ToString::to_string))
                .collect()
        })
        .unwrap_or_default();

    Ok((
        workspace_id,
        scope,
        PersistedCapabilityRegistrationV1 {
            registry_scope,
            contract,
            tags,
        },
    ))
}

fn reject_unknown_registration_wrapper_fields(value: &Value) -> Result<(), ApiError> {
    let Some(object) = value.as_object() else {
        return Err(ApiError {
            status: 400,
            reason: "Bad Request",
            code: "invalid_request",
            message: "registration body must be a JSON object".to_string(),
        });
    };

    for key in object.keys() {
        if !matches!(
            key.as_str(),
            "workspace_id" | "scope" | "registry_scope" | "tags" | "contract"
        ) {
            return Err(ApiError {
                status: 400,
                reason: "Bad Request",
                code: "unknown_field",
                message: format!("unknown registration field `{key}`"),
            });
        }
    }
    Ok(())
}

fn reject_unknown_event_registration_wrapper_fields(value: &Value) -> Result<(), ApiError> {
    let Some(object) = value.as_object() else {
        return Err(ApiError {
            status: 400,
            reason: "Bad Request",
            code: "invalid_request",
            message: "event registration body must be a JSON object".to_string(),
        });
    };

    for key in object.keys() {
        if !matches!(
            key.as_str(),
            "workspace_id" | "scope" | "registry_scope" | "event_contract"
        ) {
            return Err(ApiError {
                status: 400,
                reason: "Bad Request",
                code: "unknown_field",
                message: format!("unknown event registration field `{key}`"),
            });
        }
    }
    Ok(())
}

fn reject_unknown_workflow_registration_wrapper_fields(value: &Value) -> Result<(), ApiError> {
    let Some(object) = value.as_object() else {
        return Err(ApiError {
            status: 400,
            reason: "Bad Request",
            code: "invalid_request",
            message: "workflow registration body must be a JSON object".to_string(),
        });
    };

    for key in object.keys() {
        if !matches!(
            key.as_str(),
            "workspace_id"
                | "scope"
                | "registry_scope"
                | "workflow"
                | "registered_at"
                | "validator_version"
        ) {
            return Err(ApiError {
                status: 400,
                reason: "Bad Request",
                code: "unknown_field",
                message: format!("unknown workflow registration field `{key}`"),
            });
        }
    }
    Ok(())
}

fn registration_workspace_id(
    value: &Value,
    path_workspace_id: Option<&str>,
) -> Result<String, ApiError> {
    let workspace_id = if let Some(path_workspace_id) = path_workspace_id {
        if let Some(body_workspace_id) = value.get("workspace_id").and_then(|v| v.as_str())
            && body_workspace_id != path_workspace_id
        {
            return Err(ApiError {
                status: 400,
                reason: "Bad Request",
                code: "invalid_workspace_id",
                message: "body workspace_id must match URL workspace_id".to_string(),
            });
        }
        path_workspace_id.to_string()
    } else {
        value
            .get("workspace_id")
            .and_then(|v| v.as_str())
            .filter(|ws| !ws.trim().is_empty())
            .ok_or_else(|| ApiError {
                status: 400,
                reason: "Bad Request",
                code: "workspace_id_required",
                message: "workspace_id is required".to_string(),
            })?
            .to_string()
    };

    validate_workspace_id(&workspace_id).map_err(|msg| ApiError {
        status: 400,
        reason: "Bad Request",
        code: "invalid_workspace_id",
        message: msg,
    })?;
    Ok(workspace_id)
}

fn parse_workflow_register_body(
    body: &[u8],
) -> Result<(String, RegistrationScope, PersistedWorkflowRegistrationV1), ApiError> {
    parse_workflow_register_body_with_workspace(body, None, false)
}

fn parse_workflow_register_body_for_workspace(
    body: &[u8],
    workspace_id: &str,
) -> Result<(RegistrationScope, PersistedWorkflowRegistrationV1), ApiError> {
    let (parsed_workspace_id, scope, registration) =
        parse_workflow_register_body_with_workspace(body, Some(workspace_id), true)?;
    if parsed_workspace_id != workspace_id {
        return Err(ApiError {
            status: 400,
            reason: "Bad Request",
            code: "invalid_workspace_id",
            message: "body workspace_id must match URL workspace_id".to_string(),
        });
    }
    Ok((scope, registration))
}

fn parse_workflow_register_body_with_workspace(
    body: &[u8],
    path_workspace_id: Option<&str>,
    reject_unknown_wrapper_fields: bool,
) -> Result<(String, RegistrationScope, PersistedWorkflowRegistrationV1), ApiError> {
    let body_str = std::str::from_utf8(body).map_err(|e| ApiError {
        status: 400,
        reason: "Bad Request",
        code: "invalid_request",
        message: format!("request body is not valid UTF-8: {e}"),
    })?;

    let value: Value = serde_json::from_str(body_str).map_err(|e| ApiError {
        status: 400,
        reason: "Bad Request",
        code: "invalid_request",
        message: format!("invalid JSON body: {e}"),
    })?;

    if reject_unknown_wrapper_fields {
        reject_unknown_workflow_registration_wrapper_fields(&value)?;
    }

    let workspace_id = registration_workspace_id(&value, path_workspace_id)?;

    let scope = value
        .get("scope")
        .and_then(|v| v.as_str())
        .unwrap_or("workspace_persisted");
    let scope = match scope {
        "workspace_persisted" => RegistrationScope::WorkspacePersisted,
        "session_ephemeral" => RegistrationScope::SessionEphemeral,
        other => {
            return Err(ApiError {
                status: 422,
                reason: "Unprocessable Entity",
                code: "invalid_scope",
                message: format!(
                    "scope must be workspace_persisted or session_ephemeral (got {other})"
                ),
            });
        }
    };

    let registry_scope = value
        .get("registry_scope")
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| {
            if workspace_id == SYSTEM_WORKSPACE_ID {
                "public"
            } else {
                "private"
            }
        })
        .to_string();

    let workflow_value = value.get("workflow").ok_or_else(|| ApiError {
        status: 400,
        reason: "Bad Request",
        code: "missing_workflow",
        message: "workflow is required".to_string(),
    })?;

    let definition: WorkflowDefinition =
        serde_json::from_value(workflow_value.clone()).map_err(|e| ApiError {
            status: 422,
            reason: "Unprocessable Entity",
            code: "invalid_workflow",
            message: format!("workflow must be a valid workflow_definition: {e}"),
        })?;

    let registered_at = match value.get("registered_at").and_then(|v| v.as_str()) {
        Some(value) if !value.trim().is_empty() => value.to_string(),
        _ => match generated_registered_at() {
            Ok(value) => value,
            Err(_) => "unix:0".to_string(),
        },
    };
    let validator_version = value
        .get("validator_version")
        .and_then(|v| v.as_str())
        .unwrap_or(env!("CARGO_PKG_VERSION"))
        .to_string();

    Ok((
        workspace_id,
        scope,
        PersistedWorkflowRegistrationV1 {
            registry_scope,
            definition,
            registered_at,
            validator_version,
        },
    ))
}

fn ensure_workspace_loaded<E: LocalExecutor + Clone>(
    state: &ApiState<E>,
    workspace_id: &str,
    ws: &mut WorkspaceState<E>,
) -> Result<(), String> {
    if ws.loaded_from_disk {
        return Ok(());
    }

    ws.persisted = load_persisted_registry(&state.registry_root, workspace_id)?;
    for persisted in ws.persisted.registrations.clone() {
        let registration = derive_registration(workspace_id, &persisted)
            .map_err(|e| format!("persisted registry contains invalid entry: {}", e.message))?;
        let _ = ws
            .runtime
            .register_capability(registration)
            .map_err(render_registry_failure_as_string)?;
    }
    for persisted in ws.persisted.events.clone() {
        let registration = derive_event_registration(workspace_id, &persisted)
            .map_err(|e| format!("persisted registry contains invalid event: {}", e.message))?;
        let _ = ws
            .event_registry
            .register(registration)
            .map_err(render_event_registry_failure_as_string)?;
    }
    ws.loaded_from_disk = true;
    Ok(())
}

fn apply_registration<E: LocalExecutor + Clone>(
    state: &ApiState<E>,
    workspace_id: &str,
    scope: RegistrationScope,
    persisted_registration: PersistedCapabilityRegistrationV1,
    registration: CapabilityRegistration,
) -> Result<
    Result<traverse_registry::RegistrationOutcome, traverse_registry::RegistryFailure>,
    String,
> {
    let mut workspaces = state.workspaces.borrow_mut();
    let ws = workspaces
        .entry(workspace_id.to_string())
        .or_insert_with(|| WorkspaceState {
            runtime: Runtime::new(CapabilityRegistry::new(), state.executor.clone())
                .with_workflow_registry(WorkflowRegistry::new()),
            event_registry: EventRegistry::new(),
            persisted: PersistedWorkspaceRegistryV1 {
                schema_version: PERSISTED_REGISTRY_SCHEMA_VERSION.to_string(),
                registrations: Vec::new(),
                events: Vec::new(),
                workflows: Vec::new(),
            },
            loaded_from_disk: false,
            executions: HashMap::new(),
            traces: HashMap::new(),
        });

    ensure_workspace_loaded(state, workspace_id, ws)?;

    match ws.runtime.register_capability(registration) {
        Ok(outcome) => {
            if scope == RegistrationScope::WorkspacePersisted && !outcome.already_registered {
                ws.persisted.registrations.push(persisted_registration);
                persist_registry(&state.registry_root, workspace_id, &ws.persisted)?;
            }
            Ok(Ok(outcome))
        }
        Err(failure) => Ok(Err(failure)),
    }
}

#[derive(Debug, Clone)]
struct EventRegistrationHttpOutcome {
    already_registered: bool,
    event_id: String,
    event_version: String,
    digest: String,
}

fn apply_event_registration<E: LocalExecutor + Clone>(
    state: &ApiState<E>,
    workspace_id: &str,
    scope: RegistrationScope,
    persisted_registration: PersistedEventRegistrationV1,
    registration: EventRegistration,
) -> Result<Result<EventRegistrationHttpOutcome, traverse_registry::EventRegistryFailure>, String> {
    let mut workspaces = state.workspaces.borrow_mut();
    let ws = workspaces
        .entry(workspace_id.to_string())
        .or_insert_with(|| WorkspaceState {
            runtime: Runtime::new(CapabilityRegistry::new(), state.executor.clone())
                .with_workflow_registry(WorkflowRegistry::new()),
            event_registry: EventRegistry::new(),
            persisted: PersistedWorkspaceRegistryV1 {
                schema_version: PERSISTED_REGISTRY_SCHEMA_VERSION.to_string(),
                registrations: Vec::new(),
                events: Vec::new(),
                workflows: Vec::new(),
            },
            loaded_from_disk: false,
            executions: HashMap::new(),
            traces: HashMap::new(),
        });

    ensure_workspace_loaded(state, workspace_id, ws)?;

    let lookup_scope = match registration.scope {
        RegistryScope::Public => LookupScope::PublicOnly,
        RegistryScope::Private => LookupScope::PreferPrivate,
    };
    let existing = ws.event_registry.find_exact(
        lookup_scope,
        &registration.contract.id,
        &registration.contract.version,
    );

    match ws.event_registry.register(registration) {
        Ok(outcome) => {
            let already_registered = existing.is_some_and(|existing| {
                existing.record.contract_digest == outcome.record.contract_digest
            });
            if scope == RegistrationScope::WorkspacePersisted && !already_registered {
                ws.persisted.events.push(persisted_registration);
                persist_registry(&state.registry_root, workspace_id, &ws.persisted)?;
            }
            Ok(Ok(EventRegistrationHttpOutcome {
                already_registered,
                event_id: outcome.record.id,
                event_version: outcome.record.version,
                digest: outcome.record.contract_digest,
            }))
        }
        Err(failure) => Ok(Err(failure)),
    }
}

#[derive(Debug, Clone)]
struct WorkflowRegistrationHttpOutcome {
    already_registered: bool,
    workflow_id: String,
    workflow_version: String,
    digest: String,
    registry_scope: String,
    registered_at: String,
}

fn apply_workflow_registration<E: LocalExecutor + Clone>(
    state: &ApiState<E>,
    workspace_id: &str,
    scope: RegistrationScope,
    mut persisted: PersistedWorkflowRegistrationV1,
) -> Result<Result<WorkflowRegistrationHttpOutcome, traverse_registry::WorkflowFailure>, String> {
    state.with_workspace_mut(workspace_id, |ws| {
        let workflow_id = persisted.definition.id.clone();
        let workflow_version = persisted.definition.version.clone();
        let already = ws
            .runtime
            .workflow_registry()
            .find_exact(LookupScope::PreferPrivate, &workflow_id, &workflow_version)
            .is_some();

        if !already && persisted.registered_at.trim().is_empty() {
            persisted.registered_at = generated_registered_at().map_err(|e| e.message)?;
        }

        if persisted.validator_version.trim().is_empty() {
            persisted.validator_version = env!("CARGO_PKG_VERSION").to_string();
        }

        let registration =
            derive_workflow_registration(workspace_id, &persisted).map_err(|e| e.message)?;

        match ws.runtime.register_workflow(registration) {
            Ok(outcome) => {
                if scope == RegistrationScope::WorkspacePersisted && !already {
                    ws.persisted.workflows.push(persisted);
                    persist_registry(&state.registry_root, workspace_id, &ws.persisted)?;
                }

                Ok(Ok(WorkflowRegistrationHttpOutcome {
                    already_registered: already,
                    workflow_id: outcome.record.id,
                    workflow_version: outcome.record.version,
                    digest: outcome.record.workflow_digest,
                    registry_scope: format!("{:?}", outcome.record.scope).to_lowercase(),
                    registered_at: outcome.record.registered_at,
                }))
            }
            Err(failure) => Ok(Err(failure)),
        }
    })
}

// ---------------------------------------------------------------------------
// Connection handler
// ---------------------------------------------------------------------------

fn handle_connection<E: LocalExecutor + Clone>(
    mut stream: TcpStream,
    state: &ApiState<E>,
) -> Result<(), String> {
    let request = match read_http_request(&mut stream) {
        Ok(request) => request,
        Err(message) => {
            let (status, reason, code) = if message.contains("too large") {
                (413, "Payload Too Large", "payload_too_large")
            } else {
                (400, "Bad Request", "invalid_request")
            };
            return write_json(&mut stream, status, reason, &error_envelope(code, &message));
        }
    };

    let peer_ip = stream
        .peer_addr()
        .map(|a| a.ip())
        .unwrap_or(IpAddr::from([127, 0, 0, 1]));
    let loopback = peer_ip.is_loopback();

    if request.method == "OPTIONS" {
        return handle_cors_preflight(&mut stream, &request, state, loopback);
    }

    let cors_headers = match cors_response_headers(&request, state, loopback) {
        Ok(headers) => headers,
        Err(message) => {
            return write_json(
                &mut stream,
                403,
                "Forbidden",
                &error_envelope("cors_origin_forbidden", &message),
            );
        }
    };

    if request.path != "/healthz" && !state.allow_unauthenticated && !loopback {
        let has_bearer = request
            .headers
            .get("authorization")
            .is_some_and(|v| v.starts_with("Bearer "));

        if !has_bearer {
            let mut response = BufferedResponse::new();
            write_json(
                &mut response,
                401,
                "Unauthorized",
                &error_envelope("unauthorized", "Bearer token required"),
            )?;
            return response.write_to(&mut stream, &cors_headers);
        }
    }

    if let Some(err) = unsupported_media_type_error(&request) {
        let mut response = BufferedResponse::new();
        write_json(
            &mut response,
            err.status,
            err.reason,
            &error_envelope(err.code, &err.message),
        )?;
        return response.write_to(&mut stream, &cors_headers);
    }

    let mut response = BufferedResponse::new();
    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/healthz") => handle_health(&mut response, loopback),
        ("GET", "/v1/capabilities") => {
            handle_list_capabilities(&mut response, &request, state, loopback)
        }
        ("POST", "/v1/capabilities/register") => {
            handle_register_capability(&mut response, &request, state, loopback)
        }
        ("POST", "/v1/capabilities/execute") => {
            handle_execute(&mut response, &request, state, loopback)
        }
        (method, path) if workspace_operation_path(method, path).is_some() => {
            handle_workspace_operation(&mut response, &request, state, loopback)
        }
        ("POST", "/v1/workflows/register") => {
            handle_register_workflow(&mut response, &request, state, loopback)
        }
        ("GET", "/v1/workflows") => handle_list_workflows(&mut response, &request, state, loopback),
        ("GET", path) if path.starts_with("/v1/workflows/") => handle_get_workflow(
            &mut response,
            &request,
            state,
            loopback,
            path.trim_start_matches("/v1/workflows/"),
        ),
        _ => write_json(
            &mut response,
            404,
            "Not Found",
            &error_envelope("not_found", "route not found"),
        ),
    }?;
    response.write_to(&mut stream, &cors_headers)
}

fn handle_workspace_operation<W: Write, E: LocalExecutor + Clone>(
    w: &mut W,
    request: &HttpRequest,
    state: &ApiState<E>,
    loopback: bool,
) -> Result<(), String> {
    let Some(operation) = workspace_operation_path(&request.method, &request.path) else {
        return write_json(
            w,
            404,
            "Not Found",
            &error_envelope("not_found", "route not found"),
        );
    };

    match operation {
        WorkspaceOperation::Execute(workspace_id) => {
            handle_execute_workspace(w, request, state, loopback, &workspace_id)
        }
        WorkspaceOperation::RegisterCapability(workspace_id) => {
            handle_register_workspace_capability(w, request, state, loopback, &workspace_id)
        }
        WorkspaceOperation::RegisterEventContract(workspace_id) => {
            handle_register_workspace_event_contract(w, request, state, loopback, &workspace_id)
        }
        WorkspaceOperation::RegisterWorkflow(workspace_id) => {
            handle_register_workspace_workflow(w, request, state, loopback, &workspace_id)
        }
        WorkspaceOperation::ExecutionStatus(workspace_id, execution_id) => {
            handle_execution_status(w, request, state, loopback, &workspace_id, &execution_id)
        }
        WorkspaceOperation::Trace(workspace_id, execution_id) => {
            handle_trace_fetch(w, request, state, loopback, &workspace_id, &execution_id)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HeaderLine {
    name: String,
    value: String,
}

struct BufferedResponse {
    bytes: Vec<u8>,
}

impl BufferedResponse {
    fn new() -> Self {
        Self { bytes: Vec::new() }
    }

    fn write_to<W: Write>(&self, w: &mut W, extra_headers: &[HeaderLine]) -> Result<(), String> {
        if extra_headers.is_empty() {
            w.write_all(&self.bytes)
                .map_err(|e| format!("failed to write HTTP response: {e}"))?;
            return w
                .flush()
                .map_err(|e| format!("failed to flush HTTP response: {e}"));
        }

        let Some(header_end) = find_header_end(&self.bytes) else {
            w.write_all(&self.bytes)
                .map_err(|e| format!("failed to write HTTP response: {e}"))?;
            return w
                .flush()
                .map_err(|e| format!("failed to flush HTTP response: {e}"));
        };

        w.write_all(&self.bytes[..header_end])
            .map_err(|e| format!("failed to write HTTP response header: {e}"))?;
        for header in extra_headers {
            write!(w, "\r\n{}: {}", header.name, header.value)
                .map_err(|e| format!("failed to write HTTP response header: {e}"))?;
        }
        w.write_all(&self.bytes[header_end..])
            .map_err(|e| format!("failed to write HTTP response body: {e}"))?;
        w.flush()
            .map_err(|e| format!("failed to flush HTTP response: {e}"))
    }
}

impl Write for BufferedResponse {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.bytes.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn handle_cors_preflight<W: Write, E>(
    w: &mut W,
    request: &HttpRequest,
    state: &ApiState<E>,
    loopback: bool,
) -> Result<(), String> {
    let headers = match cors_response_headers(request, state, loopback) {
        Ok(headers) => headers,
        Err(message) => {
            return write_json(
                w,
                403,
                "Forbidden",
                &error_envelope("cors_origin_forbidden", &message),
            );
        }
    };
    if headers.is_empty() {
        return write_json(
            w,
            403,
            "Forbidden",
            &error_envelope("cors_origin_forbidden", "Origin is not allowed"),
        );
    }

    write_raw_with_headers(w, 204, "No Content", "application/json", &[], &headers)
}

fn cors_response_headers<E>(
    request: &HttpRequest,
    state: &ApiState<E>,
    loopback: bool,
) -> Result<Vec<HeaderLine>, String> {
    let Some(origin) = request.headers.get("origin") else {
        return Ok(Vec::new());
    };

    if !is_cors_origin_allowed(origin, &state.allowed_origins, loopback) {
        return Err("CORS origin is not allowed".to_string());
    }

    Ok(vec![
        HeaderLine {
            name: "Access-Control-Allow-Origin".to_string(),
            value: origin.clone(),
        },
        HeaderLine {
            name: "Vary".to_string(),
            value: "Origin".to_string(),
        },
        HeaderLine {
            name: "Access-Control-Allow-Methods".to_string(),
            value: CORS_ALLOW_METHODS.to_string(),
        },
        HeaderLine {
            name: "Access-Control-Allow-Headers".to_string(),
            value: CORS_ALLOW_HEADERS.to_string(),
        },
        HeaderLine {
            name: "Access-Control-Max-Age".to_string(),
            value: CORS_MAX_AGE_SECONDS.to_string(),
        },
    ])
}

fn is_cors_origin_allowed(origin: &str, configured_origins: &[String], loopback: bool) -> bool {
    if configured_origins
        .iter()
        .any(|configured| configured == origin)
    {
        return true;
    }

    loopback && is_loopback_browser_origin(origin)
}

fn is_loopback_browser_origin(origin: &str) -> bool {
    let Some(rest) = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))
    else {
        return false;
    };

    let host = if let Some(after_bracket) = rest.strip_prefix("[::1]") {
        if after_bracket.is_empty() || after_bracket.starts_with(':') {
            "::1"
        } else {
            return false;
        }
    } else {
        rest.split(':').next().unwrap_or_default()
    };

    matches!(host, "localhost" | "127.0.0.1" | "::1")
}

fn unsupported_media_type_error(request: &HttpRequest) -> Option<ApiError> {
    if !matches!(request.method.as_str(), "POST" | "PUT" | "PATCH") || request.body.is_empty() {
        return None;
    }

    let content_type = request.headers.get("content-type")?;

    let media_type = content_type
        .split(';')
        .next()
        .map(str::trim)
        .unwrap_or_default();
    if media_type.eq_ignore_ascii_case("application/json") {
        return None;
    }

    Some(ApiError {
        status: 415,
        reason: "Unsupported Media Type",
        code: "unsupported_media_type",
        message: "request body content-type must be application/json".to_string(),
    })
}

// ---------------------------------------------------------------------------
// Route handlers (pub(crate) so tests can call them directly)
// ---------------------------------------------------------------------------

fn handle_health<W: Write>(w: &mut W, loopback: bool) -> Result<(), String> {
    let auth_mode = if loopback {
        "dev-loopback"
    } else {
        "bearer-required"
    };

    write_json(
        w,
        200,
        "OK",
        &json!({
            "status": "ok",
            "version": env!("CARGO_PKG_VERSION"),
            "api_version": "v1",
            "workspace_default": "local-default",
            "auth_mode": auth_mode,
        }),
    )
}

fn handle_list_capabilities<W: Write, E: LocalExecutor + Clone>(
    w: &mut W,
    request: &HttpRequest,
    state: &ApiState<E>,
    loopback: bool,
) -> Result<(), String> {
    let workspace_id = match require_workspace_id_query(request) {
        Ok(value) => value,
        Err(err) => {
            return write_json(
                w,
                err.status,
                err.reason,
                &error_envelope(err.code, &err.message),
            );
        }
    };

    let identity =
        match subject_from_request(&request.headers, state.allow_unauthenticated, loopback) {
            Ok(identity) => identity,
            Err(err) => {
                return write_json(
                    w,
                    err.status,
                    err.reason,
                    &error_envelope(err.code, &err.message),
                );
            }
        };

    let _ = match ensure_workspace_access(&state.registry_root, &workspace_id, &identity) {
        Ok(metadata) => metadata,
        Err(err) => {
            return write_json(
                w,
                err.status,
                err.reason,
                &error_envelope(err.code, &err.message),
            );
        }
    };

    let entries = state.with_workspace_mut(&workspace_id, |ws| {
        Ok(ws
            .runtime
            .capability_registry()
            .discover(LookupScope::PreferPrivate, &DiscoveryQuery::default()))
    })?;

    let json_entries: Vec<Value> = entries
        .iter()
        .map(|e| {
            json!({
                "id": e.id,
                "version": e.version,
                "scope": format!("{:?}", e.scope).to_lowercase(),
                "lifecycle": format!("{:?}", e.lifecycle).to_lowercase(),
                "implementation_kind": format!("{:?}", e.implementation_kind).to_lowercase(),
                "summary": e.summary,
                "tags": e.tags,
            })
        })
        .collect();
    write_json(w, 200, "OK", &Value::Array(json_entries))
}

fn handle_register_capability<W: Write, E: LocalExecutor + Clone>(
    w: &mut W,
    request: &HttpRequest,
    state: &ApiState<E>,
    loopback: bool,
) -> Result<(), String> {
    let identity =
        match subject_from_request(&request.headers, state.allow_unauthenticated, loopback) {
            Ok(identity) => identity,
            Err(err) => {
                return write_json(
                    w,
                    err.status,
                    err.reason,
                    &error_envelope(err.code, &err.message),
                );
            }
        };

    let (workspace_id, scope, persisted_registration) = match parse_register_body(&request.body) {
        Ok(parsed) => parsed,
        Err(err) => {
            return write_json(
                w,
                err.status,
                err.reason,
                &error_envelope(err.code, &err.message),
            );
        }
    };

    let _ = match ensure_workspace_access(&state.registry_root, &workspace_id, &identity) {
        Ok(metadata) => metadata,
        Err(err) => {
            return write_json(
                w,
                err.status,
                err.reason,
                &error_envelope(err.code, &err.message),
            );
        }
    };

    let registration = match derive_registration(&workspace_id, &persisted_registration) {
        Ok(registration) => registration,
        Err(err) => {
            return write_json(
                w,
                err.status,
                err.reason,
                &error_envelope(err.code, &err.message),
            );
        }
    };

    match apply_registration(
        state,
        &workspace_id,
        scope,
        persisted_registration,
        registration,
    )? {
        Ok(outcome) => {
            let status = if outcome.already_registered { 200 } else { 201 };
            write_json(
                w,
                status,
                if status == 200 { "OK" } else { "Created" },
                &json!({
                    "workspace_id": workspace_id,
                    "scope": match scope {
                        RegistrationScope::WorkspacePersisted => "workspace_persisted",
                        RegistrationScope::SessionEphemeral => "session_ephemeral",
                    },
                    "already_registered": outcome.already_registered,
                    "capability": {
                        "id": outcome.record.id,
                        "version": outcome.record.version,
                        "digest": outcome.record.contract_digest,
                        "registry_scope": format!("{:?}", outcome.record.scope).to_lowercase(),
                    }
                }),
            )
        }
        Err(failure) => {
            let (status, code, reason) = map_registry_failure_http(&failure);
            write_json(
                w,
                status,
                reason,
                &error_envelope(code, &render_registry_failure_as_string(failure)),
            )
        }
    }
}

fn handle_register_workspace_capability<W: Write, E: LocalExecutor + Clone>(
    w: &mut W,
    request: &HttpRequest,
    state: &ApiState<E>,
    loopback: bool,
    workspace_id: &str,
) -> Result<(), String> {
    let identity =
        match subject_from_request(&request.headers, state.allow_unauthenticated, loopback) {
            Ok(identity) => identity,
            Err(err) => {
                return write_json(
                    w,
                    err.status,
                    err.reason,
                    &error_envelope(err.code, &err.message),
                );
            }
        };

    let (scope, persisted_registration) =
        match parse_register_body_for_workspace(&request.body, workspace_id) {
            Ok(parsed) => parsed,
            Err(err) => {
                return write_json(
                    w,
                    err.status,
                    err.reason,
                    &error_envelope(err.code, &err.message),
                );
            }
        };

    let _ = match ensure_workspace_access(&state.registry_root, workspace_id, &identity) {
        Ok(metadata) => metadata,
        Err(err) => {
            return write_json(
                w,
                err.status,
                err.reason,
                &error_envelope(err.code, &err.message),
            );
        }
    };

    let registration = match derive_registration(workspace_id, &persisted_registration) {
        Ok(registration) => registration,
        Err(err) => {
            return write_json(
                w,
                err.status,
                err.reason,
                &error_envelope(err.code, &err.message),
            );
        }
    };

    match apply_registration(
        state,
        workspace_id,
        scope,
        persisted_registration,
        registration,
    )? {
        Ok(outcome) => {
            let status = if outcome.already_registered { 200 } else { 201 };
            let scope_name = match scope {
                RegistrationScope::WorkspacePersisted => "workspace_persisted",
                RegistrationScope::SessionEphemeral => "session_ephemeral",
            };
            write_json(
                w,
                status,
                if status == 200 { "OK" } else { "Created" },
                &json!({
                    "api_version": "v1",
                    "registered": !outcome.already_registered,
                    "already_registered": outcome.already_registered,
                    "artifact_type": "capability",
                    "artifact_id": outcome.record.id,
                    "version": outcome.record.version,
                    "digest": outcome.record.contract_digest,
                    "scope": scope_name,
                    "links": {
                        "self": format!(
                            "/v1/workspaces/{workspace_id}/capabilities/{}/{}",
                            outcome.record.id,
                            outcome.record.version
                        ),
                        "execute": format!("/v1/workspaces/{workspace_id}/execute")
                    }
                }),
            )
        }
        Err(failure) => {
            let (status, code, reason) = map_registry_failure_http(&failure);
            write_json(
                w,
                status,
                reason,
                &error_envelope(code, &render_registry_failure_as_string(failure)),
            )
        }
    }
}

fn handle_register_workspace_event_contract<W: Write, E: LocalExecutor + Clone>(
    w: &mut W,
    request: &HttpRequest,
    state: &ApiState<E>,
    loopback: bool,
    workspace_id: &str,
) -> Result<(), String> {
    let identity =
        match subject_from_request(&request.headers, state.allow_unauthenticated, loopback) {
            Ok(identity) => identity,
            Err(err) => {
                return write_json(
                    w,
                    err.status,
                    err.reason,
                    &error_envelope(err.code, &err.message),
                );
            }
        };

    let (scope, persisted_registration) =
        match parse_event_register_body_for_workspace(&request.body, workspace_id) {
            Ok(parsed) => parsed,
            Err(err) => {
                return write_json(
                    w,
                    err.status,
                    err.reason,
                    &error_envelope(err.code, &err.message),
                );
            }
        };

    let _ = match ensure_workspace_access(&state.registry_root, workspace_id, &identity) {
        Ok(metadata) => metadata,
        Err(err) => {
            return write_json(
                w,
                err.status,
                err.reason,
                &error_envelope(err.code, &err.message),
            );
        }
    };

    let registration = match derive_event_registration(workspace_id, &persisted_registration) {
        Ok(registration) => registration,
        Err(err) => {
            return write_json(
                w,
                err.status,
                err.reason,
                &error_envelope(err.code, &err.message),
            );
        }
    };

    match apply_event_registration(
        state,
        workspace_id,
        scope,
        persisted_registration,
        registration,
    )? {
        Ok(outcome) => {
            let status = if outcome.already_registered { 200 } else { 201 };
            let scope_name = match scope {
                RegistrationScope::WorkspacePersisted => "workspace_persisted",
                RegistrationScope::SessionEphemeral => "session_ephemeral",
            };
            write_json(
                w,
                status,
                if status == 200 { "OK" } else { "Created" },
                &json!({
                    "api_version": "v1",
                    "registered": !outcome.already_registered,
                    "already_registered": outcome.already_registered,
                    "artifact_type": "event_contract",
                    "artifact_id": outcome.event_id,
                    "version": outcome.event_version,
                    "digest": outcome.digest,
                    "scope": scope_name,
                    "links": {
                        "self": format!(
                            "/v1/workspaces/{workspace_id}/event-contracts/{}/{}",
                            outcome.event_id,
                            outcome.event_version
                        )
                    }
                }),
            )
        }
        Err(failure) => {
            let (status, code, reason) = map_event_registry_failure_http(&failure);
            write_json(
                w,
                status,
                reason,
                &error_envelope(code, &render_event_registry_failure_as_string(failure)),
            )
        }
    }
}

fn handle_register_workspace_workflow<W: Write, E: LocalExecutor + Clone>(
    w: &mut W,
    request: &HttpRequest,
    state: &ApiState<E>,
    loopback: bool,
    workspace_id: &str,
) -> Result<(), String> {
    let identity =
        match subject_from_request(&request.headers, state.allow_unauthenticated, loopback) {
            Ok(identity) => identity,
            Err(err) => {
                return write_json(
                    w,
                    err.status,
                    err.reason,
                    &error_envelope(err.code, &err.message),
                );
            }
        };

    let (scope, persisted) =
        match parse_workflow_register_body_for_workspace(&request.body, workspace_id) {
            Ok(parsed) => parsed,
            Err(err) => {
                return write_json(
                    w,
                    err.status,
                    err.reason,
                    &error_envelope(err.code, &err.message),
                );
            }
        };
    let definition_for_errors = persisted.definition.clone();

    let _ = match ensure_workspace_access(&state.registry_root, workspace_id, &identity) {
        Ok(metadata) => metadata,
        Err(err) => {
            return write_json(
                w,
                err.status,
                err.reason,
                &error_envelope(err.code, &err.message),
            );
        }
    };

    match apply_workflow_registration(state, workspace_id, scope, persisted)? {
        Ok(outcome) => {
            let status = if outcome.already_registered { 200 } else { 201 };
            let scope_name = match scope {
                RegistrationScope::WorkspacePersisted => "workspace_persisted",
                RegistrationScope::SessionEphemeral => "session_ephemeral",
            };
            write_json(
                w,
                status,
                if status == 200 { "OK" } else { "Created" },
                &json!({
                    "api_version": "v1",
                    "registered": !outcome.already_registered,
                    "already_registered": outcome.already_registered,
                    "artifact_type": "workflow",
                    "artifact_id": outcome.workflow_id,
                    "version": outcome.workflow_version,
                    "digest": outcome.digest,
                    "scope": scope_name,
                    "links": {
                        "self": format!(
                            "/v1/workspaces/{workspace_id}/workflows/{}/{}",
                            outcome.workflow_id,
                            outcome.workflow_version
                        )
                    }
                }),
            )
        }
        Err(failure) => {
            let rendered = render_workflow_failure_as_string(failure.clone());
            let (status, code, reason, extra) =
                map_workflow_failure_http(&failure, &definition_for_errors);
            let mut body = error_envelope(code, &rendered);
            if let (Some(extra), Value::Object(root)) = (extra, &mut body) {
                root.insert("details".to_string(), extra);
            }
            write_json(w, status, reason, &body)
        }
    }
}

fn handle_execute<W: Write, E: LocalExecutor + Clone>(
    w: &mut W,
    request: &HttpRequest,
    state: &ApiState<E>,
    loopback: bool,
) -> Result<(), String> {
    let body = request.body.as_slice();
    let body_str = match std::str::from_utf8(body) {
        Ok(s) => s,
        Err(e) => {
            return write_json(
                w,
                400,
                "Bad Request",
                &error_envelope(
                    "invalid_request",
                    &format!("request body is not valid UTF-8: {e}"),
                ),
            );
        }
    };

    let runtime_request: RuntimeRequest = match parse_runtime_request(body_str) {
        Ok(r) => r,
        Err(e) => {
            return write_json(
                w,
                400,
                "Bad Request",
                &error_envelope(
                    "invalid_request",
                    &format!("failed to parse RuntimeRequest: {e}"),
                ),
            );
        }
    };

    let workspace_id = match require_workspace_id_query(request) {
        Ok(value) => value,
        Err(err) => {
            return write_json(
                w,
                err.status,
                err.reason,
                &error_envelope(err.code, &err.message),
            );
        }
    };

    let identity =
        match subject_from_request(&request.headers, state.allow_unauthenticated, loopback) {
            Ok(identity) => identity,
            Err(err) => {
                return write_json(
                    w,
                    err.status,
                    err.reason,
                    &error_envelope(err.code, &err.message),
                );
            }
        };

    let _ = match ensure_workspace_access(&state.registry_root, &workspace_id, &identity) {
        Ok(metadata) => metadata,
        Err(err) => {
            return write_json(
                w,
                err.status,
                err.reason,
                &error_envelope(err.code, &err.message),
            );
        }
    };

    let outcome: RuntimeExecutionOutcome =
        state.with_workspace_mut(&workspace_id, |ws| Ok(ws.runtime.execute(runtime_request)))?;

    match serialize_outcome(&outcome) {
        Ok(body_str) => write_json_raw(w, 200, "OK", &body_str),
        Err(e) => write_json(
            w,
            500,
            "Internal Server Error",
            &error_envelope("internal_error", &e),
        ),
    }
}

fn handle_execute_workspace<W: Write, E: LocalExecutor + Clone>(
    w: &mut W,
    request: &HttpRequest,
    state: &ApiState<E>,
    loopback: bool,
    workspace_id: &str,
) -> Result<(), String> {
    if let Some(replay) =
        idempotency_replay_or_conflict(request, state, workspace_id, "workspace_execute")?
    {
        return write_recorded_response(w, &replay);
    }

    let Ok(runtime_request) = parse_execute_runtime_request(w, request) else {
        return Ok(());
    };

    let identity =
        match subject_from_request(&request.headers, state.allow_unauthenticated, loopback) {
            Ok(identity) => identity,
            Err(err) => {
                return write_json(
                    w,
                    err.status,
                    err.reason,
                    &error_envelope(err.code, &err.message),
                );
            }
        };

    let _ = match ensure_workspace_access(&state.registry_root, workspace_id, &identity) {
        Ok(metadata) => metadata,
        Err(err) => {
            return write_json(
                w,
                err.status,
                err.reason,
                &error_envelope(err.code, &err.message),
            );
        }
    };

    if request_prefers_async(request) {
        let execution_id = format!("exec_{}", runtime_request.request_id);
        record_execution_status(state, workspace_id, &execution_id, "accepted")?;
        let body = json!({
            "api_version": "v1",
            "execution_id": execution_id,
            "status": "accepted",
            "links": execution_links(workspace_id, &execution_id, true),
        });
        record_idempotent_success(
            request,
            state,
            workspace_id,
            "workspace_execute",
            202,
            "Accepted",
            &body,
        )?;
        return write_json(w, 202, "Accepted", &body);
    }

    let outcome: RuntimeExecutionOutcome =
        state.with_workspace_mut(workspace_id, |ws| Ok(ws.runtime.execute(runtime_request)))?;
    let status = if outcome.result.status == RuntimeResultStatus::Error {
        "failed"
    } else {
        "succeeded"
    };
    record_execution_status(state, workspace_id, &outcome.result.execution_id, status)?;
    record_execution_trace(
        state,
        workspace_id,
        &outcome.result.execution_id,
        outcome.trace.clone(),
    )?;

    let body = json!({
        "api_version": "v1",
        "execution_id": outcome.result.execution_id,
        "status": status,
        "output": outcome.result.output,
        "error": outcome.result.error.as_ref().map(|e| json!({
            "code": format!("{:?}", e.code).to_lowercase(),
            "message": e.message,
        })),
        "links": execution_links(workspace_id, &outcome.result.execution_id, false),
    });
    record_idempotent_success(
        request,
        state,
        workspace_id,
        "workspace_execute",
        200,
        "OK",
        &body,
    )?;
    write_json(w, 200, "OK", &body)
}

fn parse_execute_runtime_request<W: Write>(
    w: &mut W,
    request: &HttpRequest,
) -> Result<RuntimeRequest, ()> {
    let body_str = match std::str::from_utf8(request.body.as_slice()) {
        Ok(value) => value,
        Err(e) => {
            let _ = write_json(
                w,
                400,
                "Bad Request",
                &error_envelope(
                    "invalid_request",
                    &format!("request body is not valid UTF-8: {e}"),
                ),
            );
            return Err(());
        }
    };

    match parse_runtime_request(body_str) {
        Ok(value) => Ok(value),
        Err(e) => {
            let _ = write_json(
                w,
                400,
                "Bad Request",
                &error_envelope(
                    "invalid_request",
                    &format!("failed to parse RuntimeRequest: {e}"),
                ),
            );
            Err(())
        }
    }
}

fn workspace_execute_path(path: &str) -> Option<String> {
    let suffix = path.strip_prefix("/v1/workspaces/")?;
    let workspace_id = suffix.strip_suffix("/execute")?;
    if workspace_id.trim().is_empty() || workspace_id.contains('/') {
        return None;
    }
    Some(workspace_id.to_string())
}

fn workspace_capabilities_path(path: &str) -> Option<String> {
    let suffix = path.strip_prefix("/v1/workspaces/")?;
    let workspace_id = suffix.strip_suffix("/capabilities")?;
    if workspace_id.trim().is_empty() || workspace_id.contains('/') {
        return None;
    }
    Some(workspace_id.to_string())
}

fn workspace_event_contracts_path(path: &str) -> Option<String> {
    let suffix = path.strip_prefix("/v1/workspaces/")?;
    let workspace_id = suffix.strip_suffix("/event-contracts")?;
    if workspace_id.trim().is_empty() || workspace_id.contains('/') {
        return None;
    }
    Some(workspace_id.to_string())
}

fn workspace_workflows_path(path: &str) -> Option<String> {
    let suffix = path.strip_prefix("/v1/workspaces/")?;
    let workspace_id = suffix.strip_suffix("/workflows")?;
    if workspace_id.trim().is_empty() || workspace_id.contains('/') {
        return None;
    }
    Some(workspace_id.to_string())
}

fn workspace_operation_path(method: &str, path: &str) -> Option<WorkspaceOperation> {
    match method {
        "POST" => workspace_execute_path(path)
            .map(WorkspaceOperation::Execute)
            .or_else(|| {
                workspace_capabilities_path(path).map(WorkspaceOperation::RegisterCapability)
            })
            .or_else(|| {
                workspace_event_contracts_path(path).map(WorkspaceOperation::RegisterEventContract)
            })
            .or_else(|| workspace_workflows_path(path).map(WorkspaceOperation::RegisterWorkflow)),
        "GET" => workspace_execution_status_path(path)
            .map(|(workspace_id, execution_id)| {
                WorkspaceOperation::ExecutionStatus(workspace_id, execution_id)
            })
            .or_else(|| {
                workspace_trace_path(path).map(|(workspace_id, execution_id)| {
                    WorkspaceOperation::Trace(workspace_id, execution_id)
                })
            }),
        _ => None,
    }
}

fn workspace_execution_status_path(path: &str) -> Option<(String, String)> {
    let suffix = path.strip_prefix("/v1/workspaces/")?;
    let (workspace_id, tail) = suffix.split_once("/executions/")?;
    if workspace_id.trim().is_empty() || tail.trim().is_empty() || tail.contains('/') {
        return None;
    }
    Some((workspace_id.to_string(), tail.to_string()))
}

fn workspace_trace_path(path: &str) -> Option<(String, String)> {
    let suffix = path.strip_prefix("/v1/workspaces/")?;
    let (workspace_id, tail) = suffix.split_once("/traces/")?;
    if workspace_id.trim().is_empty() || tail.trim().is_empty() || tail.contains('/') {
        return None;
    }
    Some((workspace_id.to_string(), tail.to_string()))
}

fn request_prefers_async(request: &HttpRequest) -> bool {
    let header_prefers_async = request
        .headers
        .get("prefer")
        .is_some_and(|value| value.split(',').any(|part| part.trim() == "respond-async"));
    let body_prefers_async = serde_json::from_slice::<Value>(&request.body)
        .ok()
        .and_then(|value| value.get("mode").cloned())
        .and_then(|value| value.as_str().map(str::to_string))
        .is_some_and(|mode| mode == "async");
    header_prefers_async || body_prefers_async
}

fn execution_links(workspace_id: &str, execution_id: &str, include_subscription: bool) -> Value {
    let status = format!("/v1/workspaces/{workspace_id}/executions/{execution_id}");
    let trace = format!("/v1/workspaces/{workspace_id}/traces/{execution_id}");
    let mut links = serde_json::Map::new();
    links.insert("self".to_string(), Value::String(status.clone()));
    links.insert("status".to_string(), Value::String(status.clone()));
    links.insert("trace".to_string(), Value::String(trace));
    if include_subscription {
        links.insert(
            "subscription".to_string(),
            Value::String(format!("{status}/events")),
        );
    }
    Value::Object(links)
}

fn record_execution_status<E: LocalExecutor + Clone>(
    state: &ApiState<E>,
    workspace_id: &str,
    execution_id: &str,
    status: &str,
) -> Result<(), String> {
    let now = generated_registered_at().map_err(|e| e.message)?;
    state.with_workspace_mut(workspace_id, |ws| {
        ws.executions.insert(
            execution_id.to_string(),
            ExecutionStatusRecord {
                execution_id: execution_id.to_string(),
                status: status.to_string(),
                created_at: now.clone(),
                updated_at: now,
            },
        );
        Ok(())
    })
}

fn record_execution_trace<E: LocalExecutor + Clone>(
    state: &ApiState<E>,
    workspace_id: &str,
    execution_id: &str,
    trace: RuntimeTrace,
) -> Result<(), String> {
    state.with_workspace_mut(workspace_id, |ws| {
        ws.traces.insert(execution_id.to_string(), trace);
        Ok(())
    })
}

fn idempotency_replay_or_conflict<E: LocalExecutor + Clone>(
    request: &HttpRequest,
    state: &ApiState<E>,
    workspace_id: &str,
    operation: &str,
) -> Result<Option<IdempotencyRecord>, String> {
    let Some(key) = idempotency_key(request) else {
        return Ok(None);
    };

    prune_idempotency_records(state);
    let cache_key = idempotency_cache_key(request, workspace_id, operation, key);
    let body_digest = idempotency_body_digest(request);
    let Some(record) = state.idempotency_records.borrow().get(&cache_key).cloned() else {
        return Ok(None);
    };

    if record.body_digest == body_digest {
        return Ok(Some(record));
    }

    let body = error_envelope(
        "idempotency_key_conflict",
        "Idempotency-Key was reused with a different request body",
    );
    let bytes = problem_response_bytes(409, "Conflict", &body)?;
    Ok(Some(IdempotencyRecord {
        body_digest,
        status: 409,
        reason: "Conflict".to_string(),
        content_type: "application/problem+json".to_string(),
        body: bytes,
        stored_at: unix_timestamp(),
    }))
}

fn record_idempotent_success<E: LocalExecutor + Clone>(
    request: &HttpRequest,
    state: &ApiState<E>,
    workspace_id: &str,
    operation: &str,
    status: u16,
    reason: &str,
    body: &Value,
) -> Result<(), String> {
    let Some(key) = idempotency_key(request) else {
        return Ok(());
    };

    prune_idempotency_records(state);
    let cache_key = idempotency_cache_key(request, workspace_id, operation, key);
    let bytes =
        serde_json::to_vec(body).map_err(|e| format!("failed to serialize response: {e}"))?;
    state.idempotency_records.borrow_mut().insert(
        cache_key,
        IdempotencyRecord {
            body_digest: idempotency_body_digest(request),
            status,
            reason: reason.to_string(),
            content_type: "application/json".to_string(),
            body: bytes,
            stored_at: unix_timestamp(),
        },
    );
    Ok(())
}

fn write_recorded_response<W: Write>(w: &mut W, record: &IdempotencyRecord) -> Result<(), String> {
    write_raw(
        w,
        record.status,
        &record.reason,
        &record.content_type,
        &record.body,
    )
}

fn idempotency_key(request: &HttpRequest) -> Option<&str> {
    request
        .headers
        .get("idempotency-key")
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn idempotency_cache_key(
    request: &HttpRequest,
    workspace_id: &str,
    operation: &str,
    key: &str,
) -> String {
    format!(
        "{operation}|{workspace_id}|{}|{}|{key}",
        idempotency_subject(request),
        request.path
    )
}

fn idempotency_subject(request: &HttpRequest) -> &str {
    request
        .headers
        .get("authorization")
        .map_or("local", String::as_str)
}

fn idempotency_body_digest(request: &HttpRequest) -> String {
    crate::agent_packages::fnv1a64(&request.body)
}

fn prune_idempotency_records<E: LocalExecutor + Clone>(state: &ApiState<E>) {
    let retention = state
        .idempotency_retention_seconds
        .max(MIN_IDEMPOTENCY_RETENTION_SECONDS);
    let now = unix_timestamp();
    state
        .idempotency_records
        .borrow_mut()
        .retain(|_, record| now.saturating_sub(record.stored_at) <= retention);
}

fn problem_response_bytes(status: u16, reason: &str, body: &Value) -> Result<Vec<u8>, String> {
    let mut body = body.clone();
    if let Value::Object(root) = &mut body {
        root.insert("title".to_string(), Value::String(reason.to_string()));
        root.insert(
            "status".to_string(),
            Value::Number(serde_json::Number::from(status)),
        );
    }
    serde_json::to_vec(&body).map_err(|e| format!("failed to serialize response: {e}"))
}

fn handle_execution_status<W: Write, E: LocalExecutor + Clone>(
    w: &mut W,
    request: &HttpRequest,
    state: &ApiState<E>,
    loopback: bool,
    workspace_id: &str,
    execution_id: &str,
) -> Result<(), String> {
    let identity =
        match subject_from_request(&request.headers, state.allow_unauthenticated, loopback) {
            Ok(identity) => identity,
            Err(err) => {
                return write_json(
                    w,
                    err.status,
                    err.reason,
                    &error_envelope(err.code, &err.message),
                );
            }
        };

    let _ = match ensure_workspace_access(&state.registry_root, workspace_id, &identity) {
        Ok(metadata) => metadata,
        Err(err) => {
            return write_json(
                w,
                err.status,
                err.reason,
                &error_envelope(err.code, &err.message),
            );
        }
    };

    let record = state.with_workspace_mut(workspace_id, |ws| {
        Ok(ws.executions.get(execution_id).cloned())
    })?;
    let Some(record) = record else {
        return write_json(
            w,
            404,
            "Not Found",
            &error_envelope("not_found", "execution was not found"),
        );
    };

    write_json(
        w,
        200,
        "OK",
        &json!({
            "api_version": "v1",
            "execution_id": record.execution_id,
            "status": record.status,
            "created_at": record.created_at,
            "updated_at": record.updated_at,
            "links": execution_links(workspace_id, execution_id, true),
        }),
    )
}

fn handle_trace_fetch<W: Write, E: LocalExecutor + Clone>(
    w: &mut W,
    request: &HttpRequest,
    state: &ApiState<E>,
    loopback: bool,
    workspace_id: &str,
    execution_id: &str,
) -> Result<(), String> {
    let identity =
        match subject_from_request(&request.headers, state.allow_unauthenticated, loopback) {
            Ok(identity) => identity,
            Err(err) => {
                return write_json(
                    w,
                    err.status,
                    err.reason,
                    &error_envelope(err.code, &err.message),
                );
            }
        };

    let _ = match ensure_workspace_access(&state.registry_root, workspace_id, &identity) {
        Ok(metadata) => metadata,
        Err(err) => {
            return write_json(
                w,
                err.status,
                err.reason,
                &error_envelope(err.code, &err.message),
            );
        }
    };

    let trace =
        state.with_workspace_mut(workspace_id, |ws| Ok(ws.traces.get(execution_id).cloned()))?;
    let Some(trace) = trace else {
        return write_json(
            w,
            404,
            "Not Found",
            &error_envelope("not_found", "trace was not found"),
        );
    };

    write_json(w, 200, "OK", &public_trace_envelope(workspace_id, &trace))
}

fn public_trace_envelope(workspace_id: &str, trace: &RuntimeTrace) -> Value {
    let spans: Vec<Value> = trace
        .state_transitions
        .iter()
        .enumerate()
        .map(|(index, transition)| {
            json!({
                "span_id": format!("{}:span:{index}", trace.trace_id),
                "name": "runtime.state_transition",
                "from_state": transition.from_state,
                "to_state": transition.to_state,
                "reason_code": transition.reason_code,
                "occurred_at": transition.occurred_at,
            })
        })
        .collect();

    let mut events: Vec<Value> = trace
        .state_progression
        .state_events
        .iter()
        .map(|event| {
            json!({
                "type": "runtime_state",
                "event_id": event.event_id,
                "state": event.state,
                "entered_at": event.entered_at,
            })
        })
        .collect();

    events.extend(trace.emitted_events.iter().map(|event| {
        json!({
            "type": "emitted_event",
            "event_id": event.event_id,
            "version": event.version,
        })
    }));

    json!({
        "api_version": "v1",
        "execution_id": trace.execution_id,
        "trace_id": trace.trace_id,
        "status": if trace.result.status == RuntimeResultStatus::Error {
            "failed"
        } else {
            "succeeded"
        },
        "spans": spans,
        "events": events,
        "links": {
            "self": format!("/v1/workspaces/{workspace_id}/traces/{}", trace.execution_id),
            "execution": format!("/v1/workspaces/{workspace_id}/executions/{}", trace.execution_id),
        },
    })
}

fn handle_register_workflow<W: Write, E: LocalExecutor + Clone>(
    w: &mut W,
    request: &HttpRequest,
    state: &ApiState<E>,
    loopback: bool,
) -> Result<(), String> {
    let identity =
        match subject_from_request(&request.headers, state.allow_unauthenticated, loopback) {
            Ok(identity) => identity,
            Err(err) => {
                return write_json(
                    w,
                    err.status,
                    err.reason,
                    &error_envelope(err.code, &err.message),
                );
            }
        };

    let (workspace_id, scope, persisted) = match parse_workflow_register_body(&request.body) {
        Ok(parsed) => parsed,
        Err(err) => {
            return write_json(
                w,
                err.status,
                err.reason,
                &error_envelope(err.code, &err.message),
            );
        }
    };
    let definition_for_errors = persisted.definition.clone();

    let _ = match ensure_workspace_access(&state.registry_root, &workspace_id, &identity) {
        Ok(metadata) => metadata,
        Err(err) => {
            return write_json(
                w,
                err.status,
                err.reason,
                &error_envelope(err.code, &err.message),
            );
        }
    };

    match apply_workflow_registration(state, &workspace_id, scope, persisted)? {
        Ok(outcome) => {
            let status = if outcome.already_registered { 200 } else { 201 };
            write_json(
                w,
                status,
                if status == 200 { "OK" } else { "Created" },
                &json!({
                    "workspace_id": workspace_id,
                    "scope": match scope {
                        RegistrationScope::WorkspacePersisted => "workspace_persisted",
                        RegistrationScope::SessionEphemeral => "session_ephemeral",
                    },
                    "already_registered": outcome.already_registered,
                    "workflow": {
                        "id": outcome.workflow_id,
                        "version": outcome.workflow_version,
                        "digest": outcome.digest,
                        "registry_scope": outcome.registry_scope,
                        "registered_at": outcome.registered_at,
                    }
                }),
            )
        }
        Err(failure) => {
            let rendered = render_workflow_failure_as_string(failure.clone());
            let (status, code, reason, extra) =
                map_workflow_failure_http(&failure, &definition_for_errors);
            let mut body = error_envelope(code, &rendered);
            if let (Some(extra), Value::Object(root)) = (extra, &mut body) {
                root.insert("details".to_string(), extra);
            }
            write_json(w, status, reason, &body)
        }
    }
}

fn handle_list_workflows<W: Write, E: LocalExecutor + Clone>(
    w: &mut W,
    request: &HttpRequest,
    state: &ApiState<E>,
    loopback: bool,
) -> Result<(), String> {
    let workspace_id = match require_workspace_id_query(request) {
        Ok(value) => value,
        Err(err) => {
            return write_json(
                w,
                err.status,
                err.reason,
                &error_envelope(err.code, &err.message),
            );
        }
    };

    let identity =
        match subject_from_request(&request.headers, state.allow_unauthenticated, loopback) {
            Ok(identity) => identity,
            Err(err) => {
                return write_json(
                    w,
                    err.status,
                    err.reason,
                    &error_envelope(err.code, &err.message),
                );
            }
        };

    let _ = match ensure_workspace_access(&state.registry_root, &workspace_id, &identity) {
        Ok(metadata) => metadata,
        Err(err) => {
            return write_json(
                w,
                err.status,
                err.reason,
                &error_envelope(err.code, &err.message),
            );
        }
    };

    let entries = state.with_workspace_mut(&workspace_id, |ws| {
        Ok(ws
            .runtime
            .workflow_registry()
            .discover(LookupScope::PreferPrivate))
    })?;

    let mut json_entries = Vec::new();
    for entry in entries {
        let resolved = state.with_workspace_mut(&workspace_id, |ws| {
            Ok(ws.runtime.workflow_registry().find_exact(
                LookupScope::PreferPrivate,
                &entry.id,
                &entry.version,
            ))
        })?;
        let digest = resolved
            .as_ref()
            .map(|wf| wf.record.workflow_digest.clone())
            .unwrap_or_default();
        let registered_at = resolved
            .as_ref()
            .map(|wf| wf.record.registered_at.clone())
            .unwrap_or_default();
        json_entries.push(json!({
            "id": entry.id,
            "version": entry.version,
            "digest": digest,
            "registered_at": registered_at,
            "scope": format!("{:?}", entry.scope).to_lowercase(),
            "lifecycle": format!("{:?}", entry.lifecycle).to_lowercase(),
            "summary": entry.summary,
            "tags": entry.tags,
        }));
    }

    write_json(w, 200, "OK", &Value::Array(json_entries))
}

fn handle_get_workflow<W: Write, E: LocalExecutor + Clone>(
    w: &mut W,
    request: &HttpRequest,
    state: &ApiState<E>,
    loopback: bool,
    workflow_id: &str,
) -> Result<(), String> {
    let workflow_id = workflow_id.trim();
    if workflow_id.is_empty() {
        return write_json(
            w,
            400,
            "Bad Request",
            &error_envelope("invalid_request", "workflow id must be non-empty"),
        );
    }

    let workspace_id = match require_workspace_id_query(request) {
        Ok(value) => value,
        Err(err) => {
            return write_json(
                w,
                err.status,
                err.reason,
                &error_envelope(err.code, &err.message),
            );
        }
    };

    let identity =
        match subject_from_request(&request.headers, state.allow_unauthenticated, loopback) {
            Ok(identity) => identity,
            Err(err) => {
                return write_json(
                    w,
                    err.status,
                    err.reason,
                    &error_envelope(err.code, &err.message),
                );
            }
        };

    let _ = match ensure_workspace_access(&state.registry_root, &workspace_id, &identity) {
        Ok(metadata) => metadata,
        Err(err) => {
            return write_json(
                w,
                err.status,
                err.reason,
                &error_envelope(err.code, &err.message),
            );
        }
    };

    let version = request.query.get("version").cloned();
    let resolved = state.with_workspace_mut(&workspace_id, |ws| {
        let registry = ws.runtime.workflow_registry();
        if let Some(version) = &version {
            return Ok(registry.find_exact(LookupScope::PreferPrivate, workflow_id, version));
        }

        let candidates = registry
            .discover(LookupScope::PreferPrivate)
            .into_iter()
            .filter(|entry| entry.id == workflow_id)
            .collect::<Vec<_>>();
        let mut ordered = candidates;
        ordered.sort_by(|left, right| {
            semver::Version::parse(&left.version)
                .ok()
                .cmp(&semver::Version::parse(&right.version).ok())
        });
        let latest = ordered.last().cloned();
        Ok(latest.and_then(|entry| {
            registry.find_exact(LookupScope::PreferPrivate, &entry.id, &entry.version)
        }))
    })?;

    let Some(resolved) = resolved else {
        return write_json(
            w,
            404,
            "Not Found",
            &error_envelope(
                "workflow_not_found",
                &format!("workflow {workflow_id} was not found"),
            ),
        );
    };

    write_json(
        w,
        200,
        "OK",
        &json!({
            "workflow": resolved.definition,
            "record": {
                "id": resolved.record.id,
                "version": resolved.record.version,
                "digest": resolved.record.workflow_digest,
                "registered_at": resolved.record.registered_at,
                "registry_scope": format!("{:?}", resolved.record.scope).to_lowercase(),
            }
        }),
    )
}

// ---------------------------------------------------------------------------
// Serialization helpers
// ---------------------------------------------------------------------------

fn serialize_outcome(outcome: &RuntimeExecutionOutcome) -> Result<String, String> {
    let trace_value = serde_json::to_value(&outcome.trace)
        .map_err(|e| format!("failed to serialize trace: {e}"))?;

    let status = if outcome.result.status == RuntimeResultStatus::Error {
        "error"
    } else {
        "completed"
    };

    let response = json!({
        "status": status,
        "request_id": outcome.result.request_id,
        "execution_id": outcome.result.execution_id,
        "trace_ref": outcome.result.trace_ref,
        "output": outcome.result.output,
        "error": outcome.result.error.as_ref().map(|e| json!({
            "code": format!("{:?}", e.code).to_lowercase(),
            "message": e.message,
        })),
        "trace": trace_value,
    });

    serde_json::to_string(&response).map_err(|e| format!("failed to serialize outcome: {e}"))
}

pub(crate) fn error_envelope(code: &str, message: &str) -> Value {
    json!({
        "type": format!("https://traverse.dev/problems/{code}"),
        "title": "",
        "status": 0,
        "detail": message,
        "traverse_code": code,
    })
}

// ---------------------------------------------------------------------------
// Raw HTTP helpers (same pattern as browser_adapter.rs)
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub(crate) struct HttpRequest {
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) query: HashMap<String, String>,
    pub(crate) headers: HashMap<String, String>,
    pub(crate) body: Vec<u8>,
}

fn read_http_request(stream: &mut TcpStream) -> Result<HttpRequest, String> {
    let mut buffer = Vec::new();
    let mut header_end = None;

    loop {
        let mut chunk = [0_u8; 1024];
        let n = stream
            .read(&mut chunk)
            .map_err(|e| format!("failed to read HTTP request: {e}"))?;
        if n == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..n]);
        if let Some(idx) = find_header_end(&buffer) {
            header_end = Some(idx);
            break;
        }
        if buffer.len() > MAX_REQUEST_BODY {
            return Err("HTTP request headers too large".to_string());
        }
    }

    let header_end = header_end
        .ok_or_else(|| "HTTP request missing \\r\\n\\r\\n header terminator".to_string())?;

    let headers_text = String::from_utf8(buffer[..header_end].to_vec())
        .map_err(|e| format!("HTTP request headers not valid UTF-8: {e}"))?;

    let mut lines = headers_text.split("\r\n");
    let request_line = lines
        .next()
        .ok_or_else(|| "HTTP request missing request line".to_string())?;

    let mut parts = request_line.split_whitespace();
    let method = parts
        .next()
        .ok_or_else(|| "HTTP request missing method".to_string())?
        .to_string();
    let raw_path = parts
        .next()
        .ok_or_else(|| "HTTP request missing path".to_string())?
        .to_string();
    let (path, query) = parse_path_and_query(&raw_path);

    let mut headers = HashMap::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }

    let content_length = headers
        .get("content-length")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0);

    if content_length > MAX_REQUEST_BODY {
        return Err(format!(
            "HTTP request body too large ({content_length} bytes, max {MAX_REQUEST_BODY})"
        ));
    }

    let mut body = buffer[header_end + 4..].to_vec();
    while body.len() < content_length {
        let mut chunk = vec![0_u8; content_length - body.len()];
        let n = stream
            .read(&mut chunk)
            .map_err(|e| format!("failed to read HTTP request body: {e}"))?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    body.truncate(content_length);

    Ok(HttpRequest {
        method,
        path,
        query,
        headers,
        body,
    })
}

fn parse_path_and_query(raw_path: &str) -> (String, HashMap<String, String>) {
    let (path, query) = match raw_path.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (raw_path, None),
    };

    let mut params = HashMap::new();
    if let Some(query) = query {
        for pair in query.split('&') {
            if pair.is_empty() {
                continue;
            }
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            params.insert(k.to_string(), v.to_string());
        }
    }
    (path.to_string(), params)
}

fn find_header_end(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|w| w == b"\r\n\r\n")
}

fn write_json<W: Write>(w: &mut W, status: u16, reason: &str, body: &Value) -> Result<(), String> {
    let mut body = body.clone();
    let content_type = if status >= 400 && body.get("traverse_code").is_some() {
        if let Value::Object(root) = &mut body {
            root.insert("title".to_string(), Value::String(reason.to_string()));
            root.insert(
                "status".to_string(),
                Value::Number(serde_json::Number::from(status)),
            );
        }
        "application/problem+json"
    } else {
        "application/json"
    };
    let bytes =
        serde_json::to_vec(&body).map_err(|e| format!("failed to serialize response: {e}"))?;
    write_raw(w, status, reason, content_type, &bytes)
}

fn write_json_raw<W: Write>(
    w: &mut W,
    status: u16,
    reason: &str,
    body: &str,
) -> Result<(), String> {
    write_raw(w, status, reason, "application/json", body.as_bytes())
}

fn write_raw<W: Write>(
    w: &mut W,
    status: u16,
    reason: &str,
    content_type: &str,
    body: &[u8],
) -> Result<(), String> {
    write_raw_with_headers(w, status, reason, content_type, body, &[])
}

fn write_raw_with_headers<W: Write>(
    w: &mut W,
    status: u16,
    reason: &str,
    content_type: &str,
    body: &[u8],
    extra_headers: &[HeaderLine],
) -> Result<(), String> {
    let header = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close",
        body.len()
    );
    w.write_all(header.as_bytes())
        .map_err(|e| format!("failed to write HTTP response header: {e}"))?;
    for header in extra_headers {
        write!(w, "\r\n{}: {}", header.name, header.value)
            .map_err(|e| format!("failed to write HTTP response header: {e}"))?;
    }
    w.write_all(b"\r\n\r\n")
        .map_err(|e| format!("failed to write HTTP response header: {e}"))?;
    w.write_all(body)
        .map_err(|e| format!("failed to write HTTP response body: {e}"))?;
    w.flush()
        .map_err(|e| format!("failed to flush HTTP response: {e}"))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;
    use serde_json::Value;
    use traverse_contracts::{
        BinaryFormat as ContractBinaryFormat, CapabilityContract, CapabilityReference, Entrypoint,
        EntrypointKind, EventClassification, EventPayload, EventProvenance, EventProvenanceSource,
        EventType, Execution, ExecutionConstraints, ExecutionTarget, FilesystemAccess,
        HostApiAccess, IdReference, Lifecycle, NetworkAccess, Owner, PayloadCompatibility,
        Provenance, ProvenanceSource, SchemaContainer, ServiceType, SideEffect, SideEffectKind,
    };
    use traverse_registry::ResolvedCapability;
    use traverse_registry::{
        ArtifactDigests, BinaryFormat, BinaryReference, CapabilityArtifactRecord,
        CapabilityRegistration, ComposabilityMetadata, CompositionKind, CompositionPattern,
        ImplementationKind, RegistryProvenance, RegistryScope, SourceKind, SourceReference,
        WorkflowEdge, WorkflowNode, WorkflowNodeInput, WorkflowNodeOutput,
    };
    use traverse_runtime::{LocalExecutionFailure, LocalExecutionFailureCode};

    // ------------------------------------------------------------------
    // Minimal test executor
    // ------------------------------------------------------------------

    #[derive(Clone)]
    struct TestExecutor {
        result: Result<Value, String>,
    }

    impl TestExecutor {
        fn ok(value: Value) -> Self {
            Self { result: Ok(value) }
        }
    }

    impl LocalExecutor for TestExecutor {
        fn execute(
            &self,
            _capability: &ResolvedCapability,
            _input: &Value,
        ) -> Result<Value, LocalExecutionFailure> {
            self.result.clone().map_err(|msg| LocalExecutionFailure {
                code: LocalExecutionFailureCode::ExecutionFailed,
                message: msg,
            })
        }
    }

    // ------------------------------------------------------------------
    // Helpers
    // ------------------------------------------------------------------

    fn test_registry_root() -> PathBuf {
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time must be valid")
            .as_nanos();
        std::env::temp_dir().join(format!("traverse-cli-http-api-tests-{suffix}"))
    }

    fn test_contract(id: &str, version: &str) -> CapabilityContract {
        let dot = id.rfind('.').unwrap_or(0);
        let namespace = id[..dot].to_string();
        let name = id[dot + 1..].to_string();
        CapabilityContract {
            kind: "capability_contract".to_string(),
            schema_version: "1.0.0".to_string(),
            id: id.to_string(),
            namespace,
            name,
            version: version.to_string(),
            lifecycle: Lifecycle::Active,
            owner: Owner {
                team: "test-team".to_string(),
                contact: "test@example.com".to_string(),
            },
            summary: "test capability".to_string(),
            description: "test capability for http_api unit tests".to_string(),
            inputs: SchemaContainer {
                schema: json!({"type": "object"}),
            },
            outputs: SchemaContainer {
                schema: json!({"type": "object"}),
            },
            preconditions: vec![],
            postconditions: vec![],
            side_effects: vec![SideEffect {
                kind: SideEffectKind::MemoryOnly,
                description: "none".to_string(),
            }],
            emits: vec![],
            consumes: vec![],
            permissions: vec![],
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
            },
            policies: vec![],
            dependencies: vec![],
            provenance: Provenance {
                source: ProvenanceSource::Greenfield,
                author: "test".to_string(),
                created_at: "2026-01-01T00:00:00Z".to_string(),
                spec_ref: None,
                adr_refs: vec![],
                exception_refs: vec![],
            },
            evidence: vec![],
            service_type: ServiceType::Stateless,
            permitted_targets: vec![ExecutionTarget::Local],
            event_trigger: None,
        }
    }

    fn test_registration(id: &str, version: &str) -> CapabilityRegistration {
        let contract = test_contract(id, version);
        CapabilityRegistration {
            scope: RegistryScope::Private,
            contract_path: format!("test/{id}/{version}/contract.json"),
            artifact: CapabilityArtifactRecord {
                artifact_ref: format!("test:{id}:{version}"),
                implementation_kind: ImplementationKind::Executable,
                source: SourceReference {
                    kind: SourceKind::Local,
                    location: format!("test/{id}/module.wasm"),
                },
                binary: Some(BinaryReference {
                    format: BinaryFormat::Wasm,
                    location: format!("test/{id}/module.wasm"),
                }),
                workflow_ref: None,
                digests: ArtifactDigests {
                    source_digest: "sha256:test".to_string(),
                    binary_digest: Some("sha256:test-bin".to_string()),
                },
                provenance: RegistryProvenance {
                    source: "greenfield".to_string(),
                    author: "test".to_string(),
                    created_at: "2026-01-01T00:00:00Z".to_string(),
                },
            },
            registered_at: "test-bundle@1.0.0".to_string(),
            tags: vec![],
            composability: ComposabilityMetadata {
                kind: CompositionKind::Atomic,
                patterns: vec![CompositionPattern::Sequential],
                provides: vec![id.to_string()],
                requires: vec![],
            },
            governing_spec: "005-capability-registry".to_string(),
            validator_version: "0.2.0".to_string(),
            contract,
        }
    }

    fn valid_registration_body(id: &str, version: &str, artifact_path: &Path) -> Vec<u8> {
        let mut contract = test_contract(id, version);
        contract.execution.entrypoint.command = artifact_path.to_string_lossy().to_string();
        json!({
            "scope": "workspace_persisted",
            "registry_scope": "private",
            "tags": ["http-api-test"],
            "contract": contract
        })
        .to_string()
        .into_bytes()
    }

    fn test_event_contract(id: &str, version: &str) -> EventContract {
        let dot = id.rfind('.').unwrap_or(0);
        let namespace = id[..dot].to_string();
        let name = id[dot + 1..].to_string();
        EventContract {
            kind: "event_contract".to_string(),
            schema_version: "1.0.0".to_string(),
            id: id.to_string(),
            namespace,
            name,
            version: version.to_string(),
            lifecycle: Lifecycle::Active,
            owner: Owner {
                team: "test-team".to_string(),
                contact: "test@example.com".to_string(),
            },
            summary: "test event".to_string(),
            description: "test event for http_api unit tests".to_string(),
            payload: EventPayload {
                schema: json!({
                    "type": "object",
                    "required": ["event_id"],
                    "properties": {
                        "event_id": {"type": "string"}
                    }
                }),
                compatibility: PayloadCompatibility::BackwardCompatible,
            },
            classification: EventClassification {
                domain: "test".to_string(),
                bounded_context: "api".to_string(),
                event_type: EventType::Domain,
                tags: vec!["test".to_string()],
            },
            publishers: vec![CapabilityReference {
                capability_id: "test.api.publisher".to_string(),
                version: "1.0.0".to_string(),
            }],
            subscribers: vec![CapabilityReference {
                capability_id: "test.api.subscriber".to_string(),
                version: "1.0.0".to_string(),
            }],
            policies: vec![IdReference {
                id: "test-policy".to_string(),
            }],
            tags: vec!["test".to_string()],
            provenance: EventProvenance {
                source: EventProvenanceSource::Greenfield,
                author: "test".to_string(),
                created_at: "2026-01-01T00:00:00Z".to_string(),
            },
            evidence: Vec::new(),
        }
    }

    fn valid_event_registration_body(id: &str, version: &str) -> Vec<u8> {
        json!({
            "scope": "workspace_persisted",
            "registry_scope": "private",
            "event_contract": test_event_contract(id, version)
        })
        .to_string()
        .into_bytes()
    }

    fn test_workflow_definition(
        id: &str,
        version: &str,
        capability_id: &str,
    ) -> WorkflowDefinition {
        let dot = id.rfind('.').unwrap_or(0);
        WorkflowDefinition {
            kind: "workflow_definition".to_string(),
            schema_version: "1.0.0".to_string(),
            id: id.to_string(),
            name: id[dot + 1..].to_string(),
            version: version.to_string(),
            lifecycle: Lifecycle::Active,
            owner: Owner {
                team: "test-team".to_string(),
                contact: "test@example.com".to_string(),
            },
            summary: "test workflow".to_string(),
            inputs: SchemaContainer {
                schema: json!({"type": "object"}),
            },
            outputs: SchemaContainer {
                schema: json!({"type": "object"}),
            },
            nodes: vec![WorkflowNode {
                node_id: "run_capability".to_string(),
                capability_id: capability_id.to_string(),
                capability_version: "1.0.0".to_string(),
                input: WorkflowNodeInput {
                    from_workflow_input: Vec::new(),
                },
                output: WorkflowNodeOutput {
                    to_workflow_state: Vec::new(),
                },
            }],
            edges: Vec::<WorkflowEdge>::new(),
            start_node: "run_capability".to_string(),
            terminal_nodes: vec!["run_capability".to_string()],
            tags: vec!["test".to_string()],
            governing_spec: "007-workflow-registry-traversal".to_string(),
        }
    }

    fn valid_workflow_registration_body(id: &str, version: &str, capability_id: &str) -> Vec<u8> {
        json!({
            "scope": "workspace_persisted",
            "registry_scope": "private",
            "workflow": test_workflow_definition(id, version, capability_id)
        })
        .to_string()
        .into_bytes()
    }

    fn test_state_with(id: &str, version: &str) -> ApiState<TestExecutor> {
        let mut registry = CapabilityRegistry::new();
        registry
            .register(test_registration(id, version))
            .expect("test registration must succeed");

        let executor = TestExecutor::ok(json!({"result": "ok"}));
        let registry_root = test_registry_root();
        std::fs::create_dir_all(&registry_root).expect("registry root must be created");

        let mut workspaces = HashMap::new();
        let workspace_id = "ws-test";
        workspaces.insert(
            workspace_id.to_string(),
            WorkspaceState {
                runtime: Runtime::new(registry, executor.clone())
                    .with_workflow_registry(WorkflowRegistry::new()),
                event_registry: EventRegistry::new(),
                persisted: PersistedWorkspaceRegistryV1 {
                    schema_version: PERSISTED_REGISTRY_SCHEMA_VERSION.to_string(),
                    registrations: Vec::new(),
                    events: Vec::new(),
                    workflows: Vec::new(),
                },
                loaded_from_disk: true,
                executions: HashMap::new(),
                traces: HashMap::new(),
            },
        );

        ApiState {
            allow_unauthenticated: true,
            allowed_origins: Vec::new(),
            registry_root,
            executor,
            workspaces: RefCell::new(workspaces),
            idempotency_records: RefCell::new(HashMap::new()),
            idempotency_retention_seconds: DEFAULT_IDEMPOTENCY_RETENTION_SECONDS,
        }
    }

    fn empty_state() -> ApiState<TestExecutor> {
        let executor = TestExecutor::ok(json!({}));
        let registry_root = test_registry_root();
        std::fs::create_dir_all(&registry_root).expect("registry root must be created");

        let mut workspaces = HashMap::new();
        let workspace_id = "ws-test";
        workspaces.insert(
            workspace_id.to_string(),
            WorkspaceState {
                runtime: Runtime::new(CapabilityRegistry::new(), executor.clone())
                    .with_workflow_registry(WorkflowRegistry::new()),
                event_registry: EventRegistry::new(),
                persisted: PersistedWorkspaceRegistryV1 {
                    schema_version: PERSISTED_REGISTRY_SCHEMA_VERSION.to_string(),
                    registrations: Vec::new(),
                    events: Vec::new(),
                    workflows: Vec::new(),
                },
                loaded_from_disk: true,
                executions: HashMap::new(),
                traces: HashMap::new(),
            },
        );

        ApiState {
            allow_unauthenticated: true,
            allowed_origins: Vec::new(),
            registry_root,
            executor,
            workspaces: RefCell::new(workspaces),
            idempotency_records: RefCell::new(HashMap::new()),
            idempotency_retention_seconds: DEFAULT_IDEMPOTENCY_RETENTION_SECONDS,
        }
    }

    fn make_http_request(method: &str, path: &str, body: Vec<u8>) -> HttpRequest {
        HttpRequest {
            method: method.to_string(),
            path: path.to_string(),
            query: HashMap::new(),
            headers: HashMap::new(),
            body,
        }
    }

    fn with_workspace_query(mut req: HttpRequest, workspace_id: &str) -> HttpRequest {
        req.query
            .insert("workspace_id".to_string(), workspace_id.to_string());
        req
    }

    fn with_bearer(mut req: HttpRequest, token: &str) -> HttpRequest {
        req.headers.insert(
            "authorization".to_string(),
            format!("Bearer {}", token.trim()),
        );
        req
    }

    fn with_idempotency_key(mut req: HttpRequest, key: &str) -> HttpRequest {
        req.headers
            .insert("idempotency-key".to_string(), key.to_string());
        req
    }

    fn base64url_encode(input: &[u8]) -> String {
        const ALPHABET: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

        if input.is_empty() {
            return String::new();
        }

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

        let rem = input.len() - i;
        if rem == 1 {
            let n = u32::from(input[i]) << 16;
            out.push(ALPHABET[((n >> 18) & 63) as usize] as char);
            out.push(ALPHABET[((n >> 12) & 63) as usize] as char);
        } else if rem == 2 {
            let n = (u32::from(input[i]) << 16) | (u32::from(input[i + 1]) << 8);
            out.push(ALPHABET[((n >> 18) & 63) as usize] as char);
            out.push(ALPHABET[((n >> 12) & 63) as usize] as char);
            out.push(ALPHABET[((n >> 6) & 63) as usize] as char);
        }

        out
    }

    fn make_jwt(sub: &str, exp: i64, admin: bool) -> String {
        let header = base64url_encode(br#"{"alg":"none","typ":"JWT"}"#);
        let mut payload = json!({ "sub": sub, "exp": exp });
        if admin {
            payload["traverse_admin"] = json!(true);
        }
        let payload_b64 = base64url_encode(payload.to_string().as_bytes());
        format!("{header}.{payload_b64}.sig")
    }

    fn make_runtime_request_body(capability_id: &str) -> Vec<u8> {
        json!({
            "kind": "runtime_request",
            "schema_version": "1.0.0",
            "request_id": "test-req-001",
            "intent": {
                "capability_id": capability_id,
                "capability_version": "1.0.0"
            },
            "input": {},
            "lookup": {
                "scope": "prefer_private",
                "allow_ambiguity": false
            },
            "context": {
                "requested_target": "local"
            },
            "governing_spec": "006-runtime-request-execution"
        })
        .to_string()
        .into_bytes()
    }

    fn parse_response_body(response: &[u8]) -> Value {
        let pos = response
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .expect("response must contain \\r\\n\\r\\n");
        serde_json::from_slice(&response[pos + 4..]).expect("response body must be valid JSON")
    }

    fn response_status(response: &[u8]) -> u16 {
        let text = std::str::from_utf8(response).expect("response must be UTF-8");
        let line = text
            .lines()
            .next()
            .expect("response must have a first line");
        let mut parts = line.splitn(3, ' ');
        parts.next();
        parts
            .next()
            .expect("status code must be present")
            .parse()
            .expect("status code must be numeric")
    }

    fn response_content_type(response: &[u8]) -> String {
        let text = std::str::from_utf8(response).expect("response must be UTF-8");
        text.lines()
            .find_map(|line| line.strip_prefix("Content-Type: ").map(ToString::to_string))
            .expect("content-type header must be present")
    }

    fn response_header(response: &[u8], name: &str) -> Option<String> {
        let text = std::str::from_utf8(response).expect("response must be UTF-8");
        let prefix = format!("{name}: ");
        text.lines()
            .find_map(|line| line.strip_prefix(&prefix).map(ToString::to_string))
    }

    // ------------------------------------------------------------------
    // CORS policy
    // ------------------------------------------------------------------

    #[test]
    fn cors_allows_loopback_browser_origins_for_dev_loopback_by_default() {
        let mut state = empty_state();
        state.allowed_origins.clear();
        let mut req = make_http_request("GET", "/healthz", Vec::new());
        req.headers
            .insert("origin".to_string(), "http://localhost:3000".to_string());

        let headers =
            cors_response_headers(&req, &state, true).expect("loopback origin must be allowed");

        assert!(headers.iter().any(|header| {
            header.name == "Access-Control-Allow-Origin" && header.value == "http://localhost:3000"
        }));
    }

    #[test]
    fn cors_requires_exact_configured_origin_for_non_loopback_callers() {
        let mut state = empty_state();
        state.allowed_origins = vec!["https://app.example".to_string()];
        let mut allowed = make_http_request("GET", "/healthz", Vec::new());
        allowed
            .headers
            .insert("origin".to_string(), "https://app.example".to_string());
        let mut denied = make_http_request("GET", "/healthz", Vec::new());
        denied
            .headers
            .insert("origin".to_string(), "https://other.example".to_string());

        assert!(cors_response_headers(&allowed, &state, false).is_ok());
        assert!(cors_response_headers(&denied, &state, false).is_err());
    }

    #[test]
    fn cors_preflight_returns_allow_headers_for_allowed_origin() {
        let state = empty_state();
        let mut req = make_http_request("OPTIONS", "/v1/capabilities", Vec::new());
        req.headers
            .insert("origin".to_string(), "http://127.0.0.1:5173".to_string());

        let mut out = Vec::new();
        handle_cors_preflight(&mut out, &req, &state, true)
            .expect("preflight must write a response");

        assert_eq!(response_status(&out), 204);
        assert_eq!(
            response_header(&out, "Access-Control-Allow-Origin"),
            Some("http://127.0.0.1:5173".to_string())
        );
        assert_eq!(
            response_header(&out, "Access-Control-Allow-Methods"),
            Some(CORS_ALLOW_METHODS.to_string())
        );
    }

    #[test]
    fn cors_preflight_rejects_unconfigured_non_loopback_origin() {
        let state = empty_state();
        let mut req = make_http_request("OPTIONS", "/v1/capabilities", Vec::new());
        req.headers
            .insert("origin".to_string(), "https://other.example".to_string());

        let mut out = Vec::new();
        handle_cors_preflight(&mut out, &req, &state, false)
            .expect("preflight must write a response");

        assert_eq!(response_status(&out), 403);
        assert_eq!(response_content_type(&out), "application/problem+json");
        assert_eq!(
            parse_response_body(&out)["traverse_code"],
            "cors_origin_forbidden"
        );
    }

    // ------------------------------------------------------------------
    // health endpoint
    // ------------------------------------------------------------------

    #[test]
    fn health_endpoint_returns_dev_loopback_envelope_for_loopback_callers() {
        let mut out = Vec::new();
        handle_health(&mut out, true).expect("health must succeed");

        assert_eq!(response_status(&out), 200);
        let body = parse_response_body(&out);
        assert_eq!(body["status"], "ok");
        assert_eq!(body["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(body["api_version"], "v1");
        assert_eq!(body["workspace_default"], "local-default");
        assert_eq!(body["auth_mode"], "dev-loopback");
    }

    #[test]
    fn health_endpoint_returns_bearer_required_envelope_for_non_loopback_callers() {
        let mut out = Vec::new();
        handle_health(&mut out, false).expect("health must succeed");

        assert_eq!(response_status(&out), 200);
        let body = parse_response_body(&out);
        assert_eq!(body["status"], "ok");
        assert_eq!(body["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(body["api_version"], "v1");
        assert_eq!(body["workspace_default"], "local-default");
        assert_eq!(body["auth_mode"], "bearer-required");
    }

    #[test]
    fn server_discovery_file_contains_health_url_and_local_token_metadata() {
        let repo_root = test_registry_root();
        let discovery_path = write_server_discovery(
            &repo_root,
            "http://127.0.0.1:8787",
            "dev-loopback",
            Some("local-token"),
        )
        .expect("discovery file must be written");

        assert_eq!(discovery_path, repo_root.join(".traverse/server.json"));
        let body = std::fs::read_to_string(&discovery_path).expect("discovery file must be read");
        let json: Value = serde_json::from_str(&body).expect("discovery must be valid json");
        assert_eq!(json["schema_version"], SERVER_DISCOVERY_SCHEMA_VERSION);
        assert_eq!(json["base_url"], "http://127.0.0.1:8787");
        assert_eq!(json["health_url"], "http://127.0.0.1:8787/healthz");
        assert_eq!(json["workspace_default"], DEFAULT_WORKSPACE_ID);
        assert_eq!(json["auth_mode"], "dev-loopback");
        assert_eq!(json["local_dev_token"], "local-token");
        assert!(json["pid"].as_u64().is_some());
        assert!(
            json["started_at"]
                .as_str()
                .is_some_and(|value| !value.is_empty())
        );
    }

    #[cfg(unix)]
    #[test]
    fn token_bearing_discovery_file_is_owner_read_write_only() {
        use std::os::unix::fs::PermissionsExt;

        let repo_root = test_registry_root();
        let discovery_path = write_server_discovery(
            &repo_root,
            "http://127.0.0.1:8787",
            "dev-loopback",
            Some("local-token"),
        )
        .expect("discovery file must be written");

        let mode = std::fs::metadata(&discovery_path)
            .expect("metadata must be readable")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }

    // ------------------------------------------------------------------
    // capabilities list endpoint
    // ------------------------------------------------------------------

    #[test]
    fn capabilities_endpoint_returns_registered_capability() {
        let state = test_state_with("test.api.do-something", "1.0.0");
        let req = with_workspace_query(
            make_http_request("GET", "/v1/capabilities", Vec::new()),
            "ws-test",
        );
        let mut out = Vec::new();
        handle_list_capabilities(&mut out, &req, &state, true).expect("list must succeed");

        let status = response_status(&out);
        let body = parse_response_body(&out);

        assert_eq!(status, 200);
        assert!(body.is_array());
        let arr = body.as_array().expect("body must be array");
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["id"], "test.api.do-something");
        assert_eq!(arr[0]["version"], "1.0.0");
    }

    #[test]
    fn capabilities_endpoint_returns_empty_array_for_empty_registry() {
        let state = empty_state();
        let req = with_workspace_query(
            make_http_request("GET", "/v1/capabilities", Vec::new()),
            "ws-test",
        );
        let mut out = Vec::new();
        handle_list_capabilities(&mut out, &req, &state, true).expect("list must succeed");

        let body = parse_response_body(&out);
        assert!(body.is_array());
        assert!(body.as_array().expect("array").is_empty());
    }

    #[test]
    fn capabilities_endpoint_requires_workspace_id() {
        let state = empty_state();
        let req = make_http_request("GET", "/v1/capabilities", Vec::new());
        let mut out = Vec::new();
        handle_list_capabilities(&mut out, &req, &state, true).expect("list must write a response");

        assert_eq!(response_status(&out), 400);
        let body = parse_response_body(&out);
        assert_eq!(body["traverse_code"], "workspace_id_required");
    }

    #[test]
    fn capabilities_endpoint_isolated_between_workspaces() {
        let state = empty_state();
        state
            .with_workspace_mut("ws-a", |ws| {
                ws.runtime
                    .register_capability(test_registration("cap.a", "1.0.0"))
                    .expect("registration must succeed");
                Ok(())
            })
            .expect("workspace insert must succeed");
        state
            .with_workspace_mut("ws-b", |ws| {
                ws.runtime
                    .register_capability(test_registration("cap.b", "1.0.0"))
                    .expect("registration must succeed");
                Ok(())
            })
            .expect("workspace insert must succeed");

        let mut out_a = Vec::new();
        let req_a = with_workspace_query(
            make_http_request("GET", "/v1/capabilities", Vec::new()),
            "ws-a",
        );
        handle_list_capabilities(&mut out_a, &req_a, &state, true).expect("list must succeed");
        assert_eq!(response_status(&out_a), 200);
        let body_a = parse_response_body(&out_a);
        let arr_a = body_a.as_array().expect("array");
        assert_eq!(arr_a.len(), 1);
        assert_eq!(arr_a[0]["id"], "cap.a");

        let mut out_b = Vec::new();
        let req_b = with_workspace_query(
            make_http_request("GET", "/v1/capabilities", Vec::new()),
            "ws-b",
        );
        handle_list_capabilities(&mut out_b, &req_b, &state, true).expect("list must succeed");
        assert_eq!(response_status(&out_b), 200);
        let body_b = parse_response_body(&out_b);
        let arr_b = body_b.as_array().expect("array");
        assert_eq!(arr_b.len(), 1);
        assert_eq!(arr_b[0]["id"], "cap.b");
    }

    #[test]
    fn capabilities_endpoint_rejects_unauthorized_workspace_access() {
        let state = empty_state();
        let metadata = WorkspaceMetadataV1 {
            schema_version: WORKSPACE_METADATA_SCHEMA_VERSION.to_string(),
            workspace_id: "ws-owned".to_string(),
            owner_subject: "alice".to_string(),
            shared: false,
            members: Vec::new(),
        };
        persist_workspace_metadata(&state.registry_root, "ws-owned", &metadata)
            .expect("metadata write must succeed");

        let req = with_bearer(
            with_workspace_query(
                make_http_request("GET", "/v1/capabilities", Vec::new()),
                "ws-owned",
            ),
            "bob",
        );
        let mut out = Vec::new();
        handle_list_capabilities(&mut out, &req, &state, true).expect("list must write a response");

        assert_eq!(response_status(&out), 403);
        assert_eq!(response_content_type(&out), "application/problem+json");
        let body = parse_response_body(&out);
        assert_eq!(body["traverse_code"], "unauthorized_workspace");
    }

    #[test]
    fn capabilities_endpoint_allows_shared_workspace_members() {
        let state = empty_state();
        let metadata = WorkspaceMetadataV1 {
            schema_version: WORKSPACE_METADATA_SCHEMA_VERSION.to_string(),
            workspace_id: "ws-shared".to_string(),
            owner_subject: "alice".to_string(),
            shared: true,
            members: vec!["bob".to_string()],
        };
        persist_workspace_metadata(&state.registry_root, "ws-shared", &metadata)
            .expect("metadata write must succeed");

        let req = with_bearer(
            with_workspace_query(
                make_http_request("GET", "/v1/capabilities", Vec::new()),
                "ws-shared",
            ),
            "bob",
        );
        let mut out = Vec::new();
        handle_list_capabilities(&mut out, &req, &state, true).expect("list must write a response");

        assert_eq!(response_status(&out), 200);
        let body = parse_response_body(&out);
        assert!(body.as_array().expect("array").is_empty());
    }

    #[test]
    fn system_workspace_requires_privileged_claim() {
        let state = empty_state();
        let req = with_bearer(
            with_workspace_query(
                make_http_request("GET", "/v1/capabilities", Vec::new()),
                SYSTEM_WORKSPACE_ID,
            ),
            "alice",
        );
        let mut out = Vec::new();
        handle_list_capabilities(&mut out, &req, &state, true).expect("list must write a response");

        assert_eq!(response_status(&out), 403);
        let body = parse_response_body(&out);
        assert_eq!(body["traverse_code"], "insufficient_privileges");
    }

    #[test]
    fn system_workspace_allows_admin_jwt() {
        let state = empty_state();
        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time must be valid")
            .as_secs();
        let now = i64::try_from(now_secs).expect("time must fit i64");
        let token = make_jwt("admin-user", now + 3600, true);
        let req = with_bearer(
            with_workspace_query(
                make_http_request("GET", "/v1/capabilities", Vec::new()),
                SYSTEM_WORKSPACE_ID,
            ),
            &token,
        );
        let mut out = Vec::new();
        handle_list_capabilities(&mut out, &req, &state, true).expect("list must write a response");
        assert_eq!(response_status(&out), 200);
    }

    // ------------------------------------------------------------------
    // execute endpoint — success
    // ------------------------------------------------------------------

    #[test]
    fn execute_endpoint_returns_completed_trace_on_success() {
        let body = make_runtime_request_body("test.api.do-something");
        let state = test_state_with("test.api.do-something", "1.0.0");
        let req = with_workspace_query(
            make_http_request("POST", "/v1/capabilities/execute", body),
            "ws-test",
        );

        let mut out = Vec::new();
        handle_execute(&mut out, &req, &state, true).expect("execute must write a response");

        let status = response_status(&out);
        let resp = parse_response_body(&out);

        assert_eq!(status, 200);
        assert_eq!(resp["status"], "completed");
        assert!(resp["trace"].is_object(), "trace must be an object");
        assert_eq!(resp["request_id"], "test-req-001");
    }

    #[test]
    fn workspace_execute_endpoint_returns_sync_execution_envelope() {
        let body = make_runtime_request_body("test.api.do-something");
        let state = test_state_with("test.api.do-something", "1.0.0");
        let req = make_http_request("POST", "/v1/workspaces/ws-test/execute", body);

        let mut out = Vec::new();
        handle_execute_workspace(&mut out, &req, &state, true, "ws-test")
            .expect("execute must write a response");

        let status = response_status(&out);
        let resp = parse_response_body(&out);

        assert_eq!(status, 200);
        assert_eq!(resp["api_version"], "v1");
        assert_eq!(resp["execution_id"], "exec_test-req-001");
        assert_eq!(resp["status"], "succeeded");
        assert_eq!(resp["output"]["result"], "ok");
        assert_eq!(
            resp["links"]["self"],
            "/v1/workspaces/ws-test/executions/exec_test-req-001"
        );
        assert_eq!(
            resp["links"]["trace"],
            "/v1/workspaces/ws-test/traces/exec_test-req-001"
        );
    }

    #[test]
    fn workspace_execute_endpoint_returns_async_accepted_envelope_for_prefer_header() {
        let body = make_runtime_request_body("test.api.do-something");
        let state = test_state_with("test.api.do-something", "1.0.0");
        let mut req = make_http_request("POST", "/v1/workspaces/ws-test/execute", body);
        req.headers
            .insert("prefer".to_string(), "respond-async".to_string());

        let mut out = Vec::new();
        handle_execute_workspace(&mut out, &req, &state, true, "ws-test")
            .expect("execute must write a response");

        let status = response_status(&out);
        let resp = parse_response_body(&out);

        assert_eq!(status, 202);
        assert_eq!(resp["api_version"], "v1");
        assert_eq!(resp["execution_id"], "exec_test-req-001");
        assert_eq!(resp["status"], "accepted");
        assert_eq!(
            resp["links"]["status"],
            "/v1/workspaces/ws-test/executions/exec_test-req-001"
        );
        assert_eq!(
            resp["links"]["trace"],
            "/v1/workspaces/ws-test/traces/exec_test-req-001"
        );
        assert_eq!(
            resp["links"]["subscription"],
            "/v1/workspaces/ws-test/executions/exec_test-req-001/events"
        );
    }

    #[test]
    fn workspace_execute_endpoint_returns_async_accepted_envelope_for_body_mode() {
        let mut body: Value =
            serde_json::from_slice(&make_runtime_request_body("test.api.do-something"))
                .expect("request body must be json");
        body["mode"] = Value::String("async".to_string());
        let state = test_state_with("test.api.do-something", "1.0.0");
        let req = make_http_request(
            "POST",
            "/v1/workspaces/ws-test/execute",
            serde_json::to_vec(&body).expect("request body must serialize"),
        );
        assert!(request_prefers_async(&req));

        let mut out = Vec::new();
        handle_execute_workspace(&mut out, &req, &state, true, "ws-test")
            .expect("execute must write a response");

        assert_eq!(response_status(&out), 202);
        assert_eq!(parse_response_body(&out)["status"], "accepted");
    }

    #[test]
    fn idempotency_key_same_body_replays_original_result() {
        let body = make_runtime_request_body("test.api.do-something");
        let state = test_state_with("test.api.do-something", "1.0.0");
        let req = with_idempotency_key(
            make_http_request("POST", "/v1/workspaces/ws-test/execute", body),
            "retry-001",
        );

        let mut first = Vec::new();
        handle_execute_workspace(&mut first, &req, &state, true, "ws-test")
            .expect("first execute must write a response");
        let mut second = Vec::new();
        handle_execute_workspace(&mut second, &req, &state, true, "ws-test")
            .expect("retry execute must write a response");

        assert_eq!(response_status(&first), 200);
        assert_eq!(response_status(&second), 200);
        assert_eq!(parse_response_body(&first), parse_response_body(&second));
        assert_eq!(state.idempotency_records.borrow().len(), 1);
    }

    #[test]
    fn idempotency_key_different_body_returns_conflict_problem_details() {
        let first_body = make_runtime_request_body("test.api.do-something");
        let second_body = make_runtime_request_body("unknown.capability.does-not-exist");
        let state = test_state_with("test.api.do-something", "1.0.0");
        let first_req = with_idempotency_key(
            make_http_request("POST", "/v1/workspaces/ws-test/execute", first_body),
            "retry-002",
        );
        let second_req = with_idempotency_key(
            make_http_request("POST", "/v1/workspaces/ws-test/execute", second_body),
            "retry-002",
        );

        let mut first = Vec::new();
        handle_execute_workspace(&mut first, &first_req, &state, true, "ws-test")
            .expect("first execute must write a response");
        let mut second = Vec::new();
        handle_execute_workspace(&mut second, &second_req, &state, true, "ws-test")
            .expect("conflict must write a response");

        assert_eq!(response_status(&first), 200);
        assert_eq!(response_status(&second), 409);
        assert_eq!(response_content_type(&second), "application/problem+json");
        let resp = parse_response_body(&second);
        assert_eq!(resp["traverse_code"], "idempotency_key_conflict");
        assert_eq!(resp["status"], 409);
    }

    #[test]
    fn idempotency_retention_defaults_to_24_hours_with_minimum_floor() {
        let state = empty_state();
        assert_eq!(
            state.idempotency_retention_seconds,
            DEFAULT_IDEMPOTENCY_RETENTION_SECONDS
        );

        state.idempotency_records.borrow_mut().insert(
            "old".to_string(),
            IdempotencyRecord {
                body_digest: "fnv1a64:old".to_string(),
                status: 200,
                reason: "OK".to_string(),
                content_type: "application/json".to_string(),
                body: b"{}".to_vec(),
                stored_at: unix_timestamp().saturating_sub(MIN_IDEMPOTENCY_RETENTION_SECONDS + 1),
            },
        );
        let mut state = state;
        state.idempotency_retention_seconds = 1;
        prune_idempotency_records(&state);

        assert!(state.idempotency_records.borrow().is_empty());
    }

    #[test]
    fn execution_status_endpoint_returns_running_status() {
        let state = empty_state();
        state
            .with_workspace_mut("ws-test", |ws| {
                ws.executions.insert(
                    "exec_running".to_string(),
                    ExecutionStatusRecord {
                        execution_id: "exec_running".to_string(),
                        status: "running".to_string(),
                        created_at: "2026-01-01T00:00:00Z".to_string(),
                        updated_at: "2026-01-01T00:00:01Z".to_string(),
                    },
                );
                Ok(())
            })
            .expect("execution status seed must succeed");
        let req = make_http_request(
            "GET",
            "/v1/workspaces/ws-test/executions/exec_running",
            Vec::new(),
        );

        let mut out = Vec::new();
        handle_execution_status(&mut out, &req, &state, true, "ws-test", "exec_running")
            .expect("status lookup must write a response");

        assert_eq!(response_status(&out), 200);
        let resp = parse_response_body(&out);
        assert_eq!(resp["api_version"], "v1");
        assert_eq!(resp["execution_id"], "exec_running");
        assert_eq!(resp["status"], "running");
        assert_eq!(resp["created_at"], "2026-01-01T00:00:00Z");
        assert_eq!(resp["updated_at"], "2026-01-01T00:00:01Z");
        assert_eq!(
            resp["links"]["self"],
            "/v1/workspaces/ws-test/executions/exec_running"
        );
        assert_eq!(
            resp["links"]["trace"],
            "/v1/workspaces/ws-test/traces/exec_running"
        );
        assert_eq!(
            resp["links"]["subscription"],
            "/v1/workspaces/ws-test/executions/exec_running/events"
        );
    }

    #[test]
    fn execution_status_endpoint_returns_succeeded_status_after_sync_execute() {
        let body = make_runtime_request_body("test.api.do-something");
        let state = test_state_with("test.api.do-something", "1.0.0");
        let execute_req = make_http_request("POST", "/v1/workspaces/ws-test/execute", body);
        let mut execute_out = Vec::new();
        handle_execute_workspace(&mut execute_out, &execute_req, &state, true, "ws-test")
            .expect("execute must write a response");

        let req = make_http_request(
            "GET",
            "/v1/workspaces/ws-test/executions/exec_test-req-001",
            Vec::new(),
        );
        let mut out = Vec::new();
        handle_execution_status(&mut out, &req, &state, true, "ws-test", "exec_test-req-001")
            .expect("status lookup must write a response");

        assert_eq!(response_status(&out), 200);
        let resp = parse_response_body(&out);
        assert_eq!(resp["execution_id"], "exec_test-req-001");
        assert_eq!(resp["status"], "succeeded");
        assert!(resp["created_at"].as_str().is_some_and(|v| !v.is_empty()));
        assert!(resp["updated_at"].as_str().is_some_and(|v| !v.is_empty()));
    }

    #[test]
    fn execution_status_endpoint_returns_failed_status_after_runtime_error() {
        let body = make_runtime_request_body("unknown.capability.does-not-exist");
        let state = empty_state();
        let execute_req = make_http_request("POST", "/v1/workspaces/ws-test/execute", body);
        let mut execute_out = Vec::new();
        handle_execute_workspace(&mut execute_out, &execute_req, &state, true, "ws-test")
            .expect("execute must write a response");

        let req = make_http_request(
            "GET",
            "/v1/workspaces/ws-test/executions/exec_test-req-001",
            Vec::new(),
        );
        let mut out = Vec::new();
        handle_execution_status(&mut out, &req, &state, true, "ws-test", "exec_test-req-001")
            .expect("status lookup must write a response");

        assert_eq!(response_status(&out), 200);
        let resp = parse_response_body(&out);
        assert_eq!(resp["execution_id"], "exec_test-req-001");
        assert_eq!(resp["status"], "failed");
    }

    #[test]
    fn execution_status_endpoint_returns_not_found_for_missing_execution() {
        let state = empty_state();
        let req = make_http_request(
            "GET",
            "/v1/workspaces/ws-test/executions/exec_missing",
            Vec::new(),
        );

        let mut out = Vec::new();
        handle_execution_status(&mut out, &req, &state, true, "ws-test", "exec_missing")
            .expect("status lookup must write a response");

        assert_eq!(response_status(&out), 404);
        assert_eq!(response_content_type(&out), "application/problem+json");
        let resp = parse_response_body(&out);
        assert_eq!(resp["traverse_code"], "not_found");
    }

    #[test]
    fn trace_fetch_endpoint_returns_public_trace_envelope() {
        let body = make_runtime_request_body("test.api.do-something");
        let state = test_state_with("test.api.do-something", "1.0.0");
        let execute_req = make_http_request("POST", "/v1/workspaces/ws-test/execute", body);
        let mut execute_out = Vec::new();
        handle_execute_workspace(&mut execute_out, &execute_req, &state, true, "ws-test")
            .expect("execute must write a response");

        let req = make_http_request(
            "GET",
            "/v1/workspaces/ws-test/traces/exec_test-req-001",
            Vec::new(),
        );
        let mut out = Vec::new();
        handle_trace_fetch(&mut out, &req, &state, true, "ws-test", "exec_test-req-001")
            .expect("trace lookup must write a response");

        assert_eq!(response_status(&out), 200);
        let resp = parse_response_body(&out);
        assert_eq!(resp["api_version"], "v1");
        assert_eq!(resp["execution_id"], "exec_test-req-001");
        assert_eq!(resp["trace_id"], "trace_exec_test-req-001");
        assert_eq!(resp["status"], "succeeded");
        assert!(
            resp["spans"]
                .as_array()
                .is_some_and(|spans| !spans.is_empty())
        );
        assert!(
            resp["events"]
                .as_array()
                .is_some_and(|events| !events.is_empty())
        );
        assert_eq!(
            resp["links"]["execution"],
            "/v1/workspaces/ws-test/executions/exec_test-req-001"
        );
    }

    #[test]
    fn trace_fetch_endpoint_does_not_expose_internal_runtime_trace_fields() {
        let body = make_runtime_request_body("test.api.do-something");
        let state = test_state_with("test.api.do-something", "1.0.0");
        let execute_req = make_http_request("POST", "/v1/workspaces/ws-test/execute", body);
        let mut execute_out = Vec::new();
        handle_execute_workspace(&mut execute_out, &execute_req, &state, true, "ws-test")
            .expect("execute must write a response");

        let req = make_http_request(
            "GET",
            "/v1/workspaces/ws-test/traces/exec_test-req-001",
            Vec::new(),
        );
        let mut out = Vec::new();
        handle_trace_fetch(&mut out, &req, &state, true, "ws-test", "exec_test-req-001")
            .expect("trace lookup must write a response");

        let body = std::str::from_utf8(&out).expect("response must be utf-8");
        assert!(!body.contains("\"request\""));
        assert!(!body.contains("\"input\""));
        assert!(!body.contains("\"output\""));
        assert!(!body.contains("\"candidate_collection\""));
        assert!(!body.contains("\"decision_evidence\""));
        assert!(!body.contains("\"state_machine_validation\""));
    }

    #[test]
    fn trace_fetch_endpoint_returns_not_found_for_missing_trace() {
        let state = empty_state();
        let req = make_http_request(
            "GET",
            "/v1/workspaces/ws-test/traces/exec_missing",
            Vec::new(),
        );

        let mut out = Vec::new();
        handle_trace_fetch(&mut out, &req, &state, true, "ws-test", "exec_missing")
            .expect("trace lookup must write a response");

        assert_eq!(response_status(&out), 404);
        let resp = parse_response_body(&out);
        assert_eq!(resp["traverse_code"], "not_found");
    }

    // ------------------------------------------------------------------
    // execute endpoint — unknown capability
    // ------------------------------------------------------------------

    #[test]
    fn execute_endpoint_returns_error_status_for_unknown_capability() {
        let body = make_runtime_request_body("unknown.capability.does-not-exist");
        let state = empty_state();
        let req = with_workspace_query(
            make_http_request("POST", "/v1/capabilities/execute", body),
            "ws-test",
        );

        let mut out = Vec::new();
        handle_execute(&mut out, &req, &state, true)
            .expect("handle_execute must write a response even on runtime error");

        let status = response_status(&out);
        let resp = parse_response_body(&out);

        assert_eq!(status, 200);
        assert_eq!(resp["status"], "error");
    }

    // ------------------------------------------------------------------
    // execute endpoint — invalid body
    // ------------------------------------------------------------------

    #[test]
    fn execute_endpoint_rejects_malformed_json_body() {
        let state = empty_state();
        let req = make_http_request(
            "POST",
            "/v1/capabilities/execute",
            b"{not valid json".to_vec(),
        );

        let mut out = Vec::new();
        handle_execute(&mut out, &req, &state, true).expect("handle_execute must write a response");

        let status = response_status(&out);
        let resp = parse_response_body(&out);

        assert_eq!(status, 400);
        assert_eq!(response_content_type(&out), "application/problem+json");
        assert_eq!(
            resp["type"],
            "https://traverse.dev/problems/invalid_request"
        );
        assert_eq!(resp["title"], "Bad Request");
        assert_eq!(resp["status"], 400);
        assert!(resp["traverse_code"].as_str().is_some());
        assert!(resp["detail"].as_str().is_some());
    }

    #[test]
    fn register_capability_validation_failure_returns_problem_details() {
        let state = empty_state();
        let req = make_http_request(
            "POST",
            "/v1/capabilities/register",
            json!({
                "workspace_id": "ws-test",
                "contract": {
                    "kind": "capability_contract"
                }
            })
            .to_string()
            .into_bytes(),
        );

        let mut out = Vec::new();
        handle_register_capability(&mut out, &req, &state, true)
            .expect("register must write a response");

        assert_eq!(response_status(&out), 422);
        assert_eq!(response_content_type(&out), "application/problem+json");
        let resp = parse_response_body(&out);
        assert_eq!(resp["title"], "Unprocessable Entity");
        assert_eq!(resp["status"], 422);
        assert_eq!(resp["traverse_code"], "contract_validation_failed");
    }

    #[test]
    fn workspace_capability_registration_is_discoverable_and_executable() {
        let state = empty_state();
        let artifact_path = state.registry_root.join("registered-module.wasm");
        std::fs::write(&artifact_path, b"wasm bytes").expect("artifact must be writable");
        let req = make_http_request(
            "POST",
            "/v1/workspaces/ws-test/capabilities",
            valid_registration_body("test.api.registered", "1.0.0", &artifact_path),
        );

        let mut out = Vec::new();
        handle_workspace_operation(&mut out, &req, &state, true)
            .expect("workspace registration must write a response");

        assert_eq!(response_status(&out), 201);
        let resp = parse_response_body(&out);
        assert_eq!(resp["api_version"], "v1");
        assert_eq!(resp["registered"], true);
        assert_eq!(resp["already_registered"], false);
        assert_eq!(resp["artifact_type"], "capability");
        assert_eq!(resp["artifact_id"], "test.api.registered");
        assert_eq!(resp["links"]["execute"], "/v1/workspaces/ws-test/execute");

        let list_req = with_workspace_query(
            make_http_request("GET", "/v1/capabilities", Vec::new()),
            "ws-test",
        );
        let mut list_out = Vec::new();
        handle_list_capabilities(&mut list_out, &list_req, &state, true)
            .expect("list capabilities must write a response");
        let listed = parse_response_body(&list_out);
        assert!(
            listed.as_array().is_some_and(|items| {
                items.iter().any(|item| item["id"] == "test.api.registered")
            })
        );

        let execute_req = make_http_request(
            "POST",
            "/v1/workspaces/ws-test/execute",
            make_runtime_request_body("test.api.registered"),
        );
        let mut execute_out = Vec::new();
        handle_workspace_operation(&mut execute_out, &execute_req, &state, true)
            .expect("workspace execute must write a response");
        let executed = parse_response_body(&execute_out);
        assert_eq!(executed["status"], "succeeded");
    }

    #[test]
    fn workspace_capability_registration_rejects_missing_artifact_before_storage() {
        let state = empty_state();
        let missing_artifact = state.registry_root.join("missing-module.wasm");
        let req = make_http_request(
            "POST",
            "/v1/workspaces/ws-test/capabilities",
            valid_registration_body("test.api.missing-artifact", "1.0.0", &missing_artifact),
        );

        let mut out = Vec::new();
        handle_workspace_operation(&mut out, &req, &state, true)
            .expect("workspace registration must write a response");

        assert_eq!(response_status(&out), 422);
        assert_eq!(response_content_type(&out), "application/problem+json");
        let resp = parse_response_body(&out);
        assert_eq!(resp["traverse_code"], "artifact_not_found");

        let list_req = with_workspace_query(
            make_http_request("GET", "/v1/capabilities", Vec::new()),
            "ws-test",
        );
        let mut list_out = Vec::new();
        handle_list_capabilities(&mut list_out, &list_req, &state, true)
            .expect("list capabilities must write a response");
        assert_eq!(
            parse_response_body(&list_out)
                .as_array()
                .map(Vec::len)
                .unwrap_or_default(),
            0
        );
    }

    #[test]
    fn workspace_capability_registration_handles_idempotent_duplicate_and_conflict() {
        let state = empty_state();
        let artifact_path = state.registry_root.join("duplicate-module.wasm");
        std::fs::write(&artifact_path, b"wasm bytes").expect("artifact must be writable");
        let first_body = valid_registration_body("test.api.duplicate", "1.0.0", &artifact_path);
        let first_req = make_http_request(
            "POST",
            "/v1/workspaces/ws-test/capabilities",
            first_body.clone(),
        );
        let second_req =
            make_http_request("POST", "/v1/workspaces/ws-test/capabilities", first_body);

        let mut first_out = Vec::new();
        handle_workspace_operation(&mut first_out, &first_req, &state, true)
            .expect("first registration must write a response");
        let mut second_out = Vec::new();
        handle_workspace_operation(&mut second_out, &second_req, &state, true)
            .expect("second registration must write a response");

        assert_eq!(response_status(&first_out), 201);
        assert_eq!(response_status(&second_out), 200);
        let duplicate = parse_response_body(&second_out);
        assert_eq!(duplicate["registered"], false);
        assert_eq!(duplicate["already_registered"], true);

        let mut changed_contract = test_contract("test.api.duplicate", "1.0.0");
        changed_contract.summary = "changed summary".to_string();
        changed_contract.execution.entrypoint.command = artifact_path.to_string_lossy().to_string();
        let conflict_req = make_http_request(
            "POST",
            "/v1/workspaces/ws-test/capabilities",
            json!({
                "scope": "workspace_persisted",
                "registry_scope": "private",
                "contract": changed_contract
            })
            .to_string()
            .into_bytes(),
        );

        let mut conflict_out = Vec::new();
        handle_workspace_operation(&mut conflict_out, &conflict_req, &state, true)
            .expect("conflict registration must write a response");

        assert_eq!(response_status(&conflict_out), 409);
        assert_eq!(
            response_content_type(&conflict_out),
            "application/problem+json"
        );
        assert_eq!(
            parse_response_body(&conflict_out)["traverse_code"],
            "registration_conflict"
        );
    }

    #[test]
    fn workspace_event_contract_registration_succeeds() {
        let state = empty_state();
        let req = make_http_request(
            "POST",
            "/v1/workspaces/ws-test/event-contracts",
            valid_event_registration_body("test.api.event-created", "1.0.0"),
        );

        let mut out = Vec::new();
        handle_workspace_operation(&mut out, &req, &state, true)
            .expect("event registration must write a response");

        assert_eq!(response_status(&out), 201);
        let resp = parse_response_body(&out);
        assert_eq!(resp["api_version"], "v1");
        assert_eq!(resp["registered"], true);
        assert_eq!(resp["already_registered"], false);
        assert_eq!(resp["artifact_type"], "event_contract");
        assert_eq!(resp["artifact_id"], "test.api.event-created");

        let registered = state
            .with_workspace_mut("ws-test", |ws| {
                Ok(ws.event_registry.find_exact(
                    LookupScope::PreferPrivate,
                    "test.api.event-created",
                    "1.0.0",
                ))
            })
            .expect("workspace lookup must succeed");
        assert!(registered.is_some());
    }

    #[test]
    fn workspace_event_contract_registration_rejects_invalid_contract_without_storage() {
        let state = empty_state();
        let req = make_http_request(
            "POST",
            "/v1/workspaces/ws-test/event-contracts",
            json!({
                "scope": "workspace_persisted",
                "registry_scope": "private",
                "event_contract": {
                    "kind": "event_contract"
                }
            })
            .to_string()
            .into_bytes(),
        );

        let mut out = Vec::new();
        handle_workspace_operation(&mut out, &req, &state, true)
            .expect("event registration must write a response");

        assert_eq!(response_status(&out), 422);
        assert_eq!(response_content_type(&out), "application/problem+json");
        let resp = parse_response_body(&out);
        assert_eq!(resp["traverse_code"], "event_contract_validation_failed");

        let registered = state
            .with_workspace_mut("ws-test", |ws| {
                Ok(ws.event_registry.find_exact(
                    LookupScope::PreferPrivate,
                    "test.api.event-created",
                    "1.0.0",
                ))
            })
            .expect("workspace lookup must succeed");
        assert!(registered.is_none());
    }

    #[test]
    fn workspace_event_contract_registration_handles_duplicate_and_conflict() {
        let state = empty_state();
        let first_body = valid_event_registration_body("test.api.event-duplicate", "1.0.0");
        let first_req = make_http_request(
            "POST",
            "/v1/workspaces/ws-test/event-contracts",
            first_body.clone(),
        );
        let second_req =
            make_http_request("POST", "/v1/workspaces/ws-test/event-contracts", first_body);

        let mut first_out = Vec::new();
        handle_workspace_operation(&mut first_out, &first_req, &state, true)
            .expect("first event registration must write a response");
        let mut second_out = Vec::new();
        handle_workspace_operation(&mut second_out, &second_req, &state, true)
            .expect("second event registration must write a response");

        assert_eq!(response_status(&first_out), 201);
        assert_eq!(response_status(&second_out), 200);
        let duplicate = parse_response_body(&second_out);
        assert_eq!(duplicate["registered"], false);
        assert_eq!(duplicate["already_registered"], true);

        let mut changed_contract = test_event_contract("test.api.event-duplicate", "1.0.0");
        changed_contract.summary = "changed summary".to_string();
        let conflict_req = make_http_request(
            "POST",
            "/v1/workspaces/ws-test/event-contracts",
            json!({
                "scope": "workspace_persisted",
                "registry_scope": "private",
                "event_contract": changed_contract
            })
            .to_string()
            .into_bytes(),
        );

        let mut conflict_out = Vec::new();
        handle_workspace_operation(&mut conflict_out, &conflict_req, &state, true)
            .expect("conflict event registration must write a response");

        assert_eq!(response_status(&conflict_out), 409);
        assert_eq!(
            response_content_type(&conflict_out),
            "application/problem+json"
        );
        assert_eq!(
            parse_response_body(&conflict_out)["traverse_code"],
            "registration_conflict"
        );
    }

    #[test]
    fn workspace_workflow_registration_succeeds_and_is_discoverable() {
        let state = test_state_with("test.api.workflow-capability", "1.0.0");
        let req = make_http_request(
            "POST",
            "/v1/workspaces/ws-test/workflows",
            valid_workflow_registration_body(
                "test.api.workflow-registered",
                "1.0.0",
                "test.api.workflow-capability",
            ),
        );

        let mut out = Vec::new();
        handle_workspace_operation(&mut out, &req, &state, true)
            .expect("workflow registration must write a response");

        assert_eq!(response_status(&out), 201);
        let resp = parse_response_body(&out);
        assert_eq!(resp["api_version"], "v1");
        assert_eq!(resp["registered"], true);
        assert_eq!(resp["already_registered"], false);
        assert_eq!(resp["artifact_type"], "workflow");
        assert_eq!(resp["artifact_id"], "test.api.workflow-registered");

        let list_req = with_workspace_query(
            make_http_request("GET", "/v1/workflows", Vec::new()),
            "ws-test",
        );
        let mut list_out = Vec::new();
        handle_list_workflows(&mut list_out, &list_req, &state, true)
            .expect("list workflows must write a response");
        let listed = parse_response_body(&list_out);
        assert!(listed.as_array().is_some_and(|items| {
            items
                .iter()
                .any(|item| item["id"] == "test.api.workflow-registered")
        }));
    }

    #[test]
    fn workspace_workflow_registration_rejects_invalid_workflow_without_storage() {
        let state = empty_state();
        let req = make_http_request(
            "POST",
            "/v1/workspaces/ws-test/workflows",
            json!({
                "scope": "workspace_persisted",
                "registry_scope": "private",
                "workflow": {
                    "kind": "workflow_definition"
                }
            })
            .to_string()
            .into_bytes(),
        );

        let mut out = Vec::new();
        handle_workspace_operation(&mut out, &req, &state, true)
            .expect("workflow registration must write a response");

        assert_eq!(response_status(&out), 422);
        assert_eq!(response_content_type(&out), "application/problem+json");
        let resp = parse_response_body(&out);
        assert_eq!(resp["traverse_code"], "invalid_workflow");

        let list_req = with_workspace_query(
            make_http_request("GET", "/v1/workflows", Vec::new()),
            "ws-test",
        );
        let mut list_out = Vec::new();
        handle_list_workflows(&mut list_out, &list_req, &state, true)
            .expect("list workflows must write a response");
        assert_eq!(
            parse_response_body(&list_out)
                .as_array()
                .map(Vec::len)
                .unwrap_or_default(),
            0
        );
    }

    #[test]
    fn workspace_workflow_registration_handles_duplicate_and_conflict() {
        let state = test_state_with("test.api.workflow-capability", "1.0.0");
        let first_body = valid_workflow_registration_body(
            "test.api.workflow-duplicate",
            "1.0.0",
            "test.api.workflow-capability",
        );
        let first_req = make_http_request(
            "POST",
            "/v1/workspaces/ws-test/workflows",
            first_body.clone(),
        );
        let second_req = make_http_request("POST", "/v1/workspaces/ws-test/workflows", first_body);

        let mut first_out = Vec::new();
        handle_workspace_operation(&mut first_out, &first_req, &state, true)
            .expect("first workflow registration must write a response");
        let mut second_out = Vec::new();
        handle_workspace_operation(&mut second_out, &second_req, &state, true)
            .expect("second workflow registration must write a response");

        assert_eq!(response_status(&first_out), 201);
        assert_eq!(response_status(&second_out), 200);
        let duplicate = parse_response_body(&second_out);
        assert_eq!(duplicate["registered"], false);
        assert_eq!(duplicate["already_registered"], true);

        let mut changed_definition = test_workflow_definition(
            "test.api.workflow-duplicate",
            "1.0.0",
            "test.api.workflow-capability",
        );
        changed_definition.summary = "changed summary".to_string();
        let conflict_req = make_http_request(
            "POST",
            "/v1/workspaces/ws-test/workflows",
            json!({
                "scope": "workspace_persisted",
                "registry_scope": "private",
                "workflow": changed_definition
            })
            .to_string()
            .into_bytes(),
        );

        let mut conflict_out = Vec::new();
        handle_workspace_operation(&mut conflict_out, &conflict_req, &state, true)
            .expect("conflict workflow registration must write a response");

        assert_eq!(response_status(&conflict_out), 409);
        assert_eq!(
            response_content_type(&conflict_out),
            "application/problem+json"
        );
        assert_eq!(
            parse_response_body(&conflict_out)["traverse_code"],
            "immutable_version_conflict"
        );
    }

    #[test]
    fn execute_endpoint_requires_workspace_id() {
        let body = make_runtime_request_body("test.api.do-something");
        let state = test_state_with("test.api.do-something", "1.0.0");
        let req = make_http_request("POST", "/v1/capabilities/execute", body);

        let mut out = Vec::new();
        handle_execute(&mut out, &req, &state, true).expect("handle_execute must write a response");

        assert_eq!(response_status(&out), 400);
        assert_eq!(response_content_type(&out), "application/problem+json");
        let resp = parse_response_body(&out);
        assert_eq!(resp["traverse_code"], "workspace_id_required");
    }

    #[test]
    fn execute_endpoint_rejects_expired_jwt() {
        let body = make_runtime_request_body("test.api.do-something");
        let state = test_state_with("test.api.do-something", "1.0.0");
        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time must be valid")
            .as_secs();
        let now = i64::try_from(now_secs).expect("time must fit i64");
        let token = make_jwt("alice", now - 10, false);
        let req = with_bearer(
            with_workspace_query(
                make_http_request("POST", "/v1/capabilities/execute", body),
                "ws-test",
            ),
            &token,
        );

        let mut out = Vec::new();
        handle_execute(&mut out, &req, &state, true).expect("handle_execute must write a response");

        assert_eq!(response_status(&out), 401);
        assert_eq!(response_content_type(&out), "application/problem+json");
        let resp = parse_response_body(&out);
        assert_eq!(resp["traverse_code"], "token_expired");
    }

    #[test]
    fn unauthenticated_request_returns_problem_details() {
        let body = make_runtime_request_body("test.api.do-something");
        let mut state = test_state_with("test.api.do-something", "1.0.0");
        state.allow_unauthenticated = false;
        let req = with_workspace_query(
            make_http_request("POST", "/v1/capabilities/execute", body),
            "ws-test",
        );

        let mut out = Vec::new();
        handle_execute(&mut out, &req, &state, false).expect("execute must write a response");

        assert_eq!(response_status(&out), 401);
        assert_eq!(response_content_type(&out), "application/problem+json");
        let resp = parse_response_body(&out);
        assert_eq!(resp["title"], "Unauthorized");
        assert_eq!(resp["status"], 401);
        assert_eq!(resp["traverse_code"], "unauthorized");
    }

    #[test]
    fn unsupported_media_type_returns_problem_details() {
        let mut req = make_http_request("POST", "/v1/workspaces/ws-test/execute", b"{}".to_vec());
        req.headers
            .insert("content-type".to_string(), "text/plain".to_string());
        let err = unsupported_media_type_error(&req).expect("media type must be rejected");

        let mut out = Vec::new();
        write_json(
            &mut out,
            err.status,
            err.reason,
            &error_envelope(err.code, &err.message),
        )
        .expect("problem response must serialize");

        assert_eq!(response_status(&out), 415);
        assert_eq!(response_content_type(&out), "application/problem+json");
        let resp = parse_response_body(&out);
        assert_eq!(resp["title"], "Unsupported Media Type");
        assert_eq!(resp["status"], 415);
        assert_eq!(resp["traverse_code"], "unsupported_media_type");
    }

    #[test]
    fn payload_too_large_returns_problem_details() {
        let mut out = Vec::new();
        write_json(
            &mut out,
            413,
            "Payload Too Large",
            &error_envelope("payload_too_large", "HTTP request body too large"),
        )
        .expect("problem response must serialize");

        assert_eq!(response_status(&out), 413);
        assert_eq!(response_content_type(&out), "application/problem+json");
        let resp = parse_response_body(&out);
        assert_eq!(resp["title"], "Payload Too Large");
        assert_eq!(resp["status"], 413);
        assert_eq!(resp["traverse_code"], "payload_too_large");
    }

    #[test]
    fn conflict_returns_problem_details() {
        let mut out = Vec::new();
        write_json(
            &mut out,
            409,
            "Conflict",
            &error_envelope("immutable_version_conflict", "version is immutable"),
        )
        .expect("problem response must serialize");

        assert_eq!(response_status(&out), 409);
        assert_eq!(response_content_type(&out), "application/problem+json");
        let resp = parse_response_body(&out);
        assert_eq!(resp["title"], "Conflict");
        assert_eq!(resp["status"], 409);
        assert_eq!(resp["traverse_code"], "immutable_version_conflict");
    }

    // ------------------------------------------------------------------
    // auth helpers — loopback detection via std
    // ------------------------------------------------------------------

    #[test]
    fn loopback_ipv4_is_recognized() {
        let ip: IpAddr = "127.0.0.1".parse().expect("valid IP");
        assert!(ip.is_loopback());
    }

    #[test]
    fn loopback_ipv6_is_recognized() {
        let ip: IpAddr = "::1".parse().expect("valid IP");
        assert!(ip.is_loopback());
    }

    #[test]
    fn non_loopback_ip_is_not_loopback() {
        let ip: IpAddr = "192.168.1.100".parse().expect("valid IP");
        assert!(!ip.is_loopback());
    }

    // ------------------------------------------------------------------
    // error envelope shape
    // ------------------------------------------------------------------

    #[test]
    fn error_envelope_has_correct_json_shape() {
        let env = error_envelope("unauthorized", "Bearer token required");
        assert_eq!(env["type"], "https://traverse.dev/problems/unauthorized");
        assert_eq!(env["detail"], "Bearer token required");
        assert_eq!(env["traverse_code"], "unauthorized");
    }
}
