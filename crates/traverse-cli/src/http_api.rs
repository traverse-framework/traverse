use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::fmt::Write as FmtWrite;
use std::io::{Read, Write};
use std::net::{IpAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use traverse_contracts::{CapabilityContract, EventContract, parse_contract, parse_event_contract};
use traverse_registry::{
    ApplicationStateMachine, ApplicationStateTransition, ApplicationStateTransitionCondition,
    ApplicationStateTransitionConditionOp, ArtifactDigests, BinaryFormat, BinaryReference,
    CapabilityArtifactRecord, CapabilityRegistration, CapabilityRegistry, ComposabilityMetadata,
    CompositionKind, CompositionPattern, DiscoveryQuery, EventRegistration, EventRegistry,
    ImplementationKind, LookupScope, RegistryProvenance, RegistryScope, SourceKind,
    SourceReference, WorkflowDefinition, WorkflowRegistration, WorkflowRegistry,
    WorkspaceAppStateErrorCode, load_application_bundle_manifest,
};
use traverse_runtime::security::RuntimeSecurityConfig;
use traverse_runtime::{
    LocalExecutor, PlacementTarget, Runtime, RuntimeContext, RuntimeExecutionOutcome,
    RuntimeIntent, RuntimeLookup, RuntimeLookupScope, RuntimeRequest, RuntimeResultStatus,
    RuntimeTrace, parse_runtime_request,
};
use zeroize::Zeroizing;

/// Map an HTTP auth mode to the runtime security posture: the dev auth modes
/// run local unsigned example artifacts (development), while the network-facing
/// `bearer-required` mode enforces the production posture that rejects unsigned
/// artifacts (spec 030-security-identity-model FR-013).
fn runtime_security_for_auth_mode(auth_mode: &str) -> RuntimeSecurityConfig {
    if matches!(auth_mode, "dev-loopback" | "dev-any") {
        RuntimeSecurityConfig::development()
    } else {
        RuntimeSecurityConfig::production()
    }
}

const MAX_REQUEST_BODY: usize = 4 * 1024 * 1024; // 4 MiB
const MAX_REQUEST_HEADER_BYTES: usize = 16 * 1024; // 16 KiB
const MAX_REQUEST_HEADER_COUNT: usize = 100;
const DEFAULT_READ_TIMEOUT_SECS: u64 = 10;
const DEFAULT_WRITE_TIMEOUT_SECS: u64 = 10;
const DEFAULT_REQUEST_DEADLINE_SECS: u64 = 30;
const DEFAULT_MAX_CONCURRENT_CONNECTIONS: usize = 64;
const SYSTEM_WORKSPACE_ID: &str = "system";
const SYSTEM_ADMIN_SUBJECT: &str = "system_admin";
const PERSISTED_REGISTRY_SCHEMA_VERSION: &str = "1.0.0";
const WORKSPACE_METADATA_SCHEMA_VERSION: &str = "1.0.0";
const DEFAULT_WORKSPACE_ID: &str = "local-default";
const SERVER_DISCOVERY_SCHEMA_VERSION: &str = "1.0.0";
const DEFAULT_IDEMPOTENCY_RETENTION_SECONDS: u64 = 24 * 60 * 60;
const MIN_IDEMPOTENCY_RETENTION_SECONDS: u64 = 60;
const CORS_ALLOW_METHODS: &str = "GET, POST, OPTIONS";
const CORS_ALLOW_HEADERS: &str =
    "Authorization, Content-Type, Idempotency-Key, Last-Event-ID, Prefer";
const CORS_MAX_AGE_SECONDS: &str = "600";
const SCOPE_RUNTIME_EXECUTE: &str = "runtime:execute";
const SCOPE_RUNTIME_TRACE_READ: &str = "runtime:trace:read";
const SCOPE_RUNTIME_EVENTS_READ: &str = "runtime:events:read";
const SCOPE_REGISTRY_READ: &str = "registry:read";
const SCOPE_REGISTRY_WRITE: &str = "registry:write";
const SCOPE_GRANTS_APPROVE: &str = "grants:approve";

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
    pub requested_auth_mode: Option<String>,
    pub allow_unauthenticated: bool,
    pub allowed_origins: Vec<String>,
    pub render_mobile_qr: bool,
    pub capability_registry: CapabilityRegistry,
    pub workflow_registry: WorkflowRegistry,
    pub registry_root: PathBuf,
    pub executor: E,
    /// Optional Idempotency-Key retention in seconds. Values below 60 seconds are floored to 60.
    pub idempotency_retention_seconds: Option<u64>,
    /// Optional hex-encoded `Ed25519` public key used to verify JWT bearer-token
    /// signatures (`EdDSA`). Required for the `bearer-required` auth mode: without
    /// it, every bearer token is rejected (fail closed). Ignored for the dev
    /// auth modes when absent, where unverified tokens can never yield admin.
    pub jwt_verification_key_hex: Option<String>,
    /// Per-read/write socket timeout applied to every accepted connection before
    /// any I/O; bounds a single blocking read/write call. Defaults to
    /// [`DEFAULT_READ_TIMEOUT_SECS`] / [`DEFAULT_WRITE_TIMEOUT_SECS`].
    pub read_timeout: Option<Duration>,
    pub write_timeout: Option<Duration>,
    /// Whole-request deadline spanning the header and body read phases, so a
    /// slow trickle cannot extend a request past this bound even though each
    /// individual read stays within `read_timeout`. Defaults to
    /// [`DEFAULT_REQUEST_DEADLINE_SECS`].
    pub request_deadline: Option<Duration>,
    /// Maximum number of connections serviced concurrently by the bounded
    /// worker pool. Defaults to [`DEFAULT_MAX_CONCURRENT_CONNECTIONS`].
    pub max_concurrent_connections: Option<usize>,
}

struct ApiState<E> {
    auth_mode: String,
    allow_unauthenticated: bool,
    allowed_origins: Vec<String>,
    registry_root: PathBuf,
    executor: E,
    workspaces: Mutex<HashMap<String, WorkspaceState<E>>>,
    idempotency_records: Mutex<HashMap<String, IdempotencyRecord>>,
    idempotency_retention_seconds: u64,
    jwt_verification_key: Option<ed25519_dalek::VerifyingKey>,
}

struct WorkspaceState<E> {
    runtime: traverse_runtime::Runtime<E>,
    event_registry: EventRegistry,
    persisted: PersistedWorkspaceRegistryV1,
    loaded_from_disk: bool,
    executions: HashMap<String, ExecutionStatusRecord>,
    traces: HashMap<String, RuntimeTrace>,
    app_events: Vec<AppStateEventRecord>,
    app_list_context_fields: HashMap<String, Vec<String>>,
    app_state_machines: HashMap<String, ApplicationStateMachine>,
    runtime_grants: Vec<RuntimeGrantRecord>,
}

#[derive(Debug, Clone)]
struct ExecutionStatusRecord {
    execution_id: String,
    status: String,
    created_at: String,
    updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
struct AppStateEventRecord {
    event_id: String,
    event_type: String,
    workspace_id: String,
    app_id: String,
    session_id: String,
    execution_id: String,
    state: String,
    previous_state: Option<String>,
    timestamp: String,
    data: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RuntimeGrantLifetime {
    Execution,
    Session,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RuntimeGrantRecord {
    grant_id: String,
    capability_id: String,
    grant_scope: String,
    resource: String,
    lifetime: RuntimeGrantLifetime,
    approved_by: String,
    granted_at: String,
    expires_at: String,
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

/// A durable, append-only delta for a workspace registry snapshot. Keeping
/// mutations separate from the legacy snapshot prevents a single registration
/// from rewriting every previously registered artifact.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct PersistedRegistryMutationV1 {
    #[serde(default)]
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

#[derive(Debug, Clone)]
struct ParsedBundleRegistrationV1 {
    scope: RegistrationScope,
    capabilities: Vec<PersistedCapabilityRegistrationV1>,
    events: Vec<PersistedEventRegistrationV1>,
    workflows: Vec<PersistedWorkflowRegistrationV1>,
}

#[derive(Debug, Clone)]
struct BundleRegistrationHttpOutcome {
    already_registered: bool,
    outcomes: Vec<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ServerDiscoveryV1 {
    schema_version: String,
    base_url: String,
    bind_address: String,
    health_url: String,
    workspace_default: String,
    pid: u32,
    started_at: String,
    auth_mode: String,
    mobile_connect_url: String,
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
    scopes: Vec<String>,
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
    RegisterBundle(String),
    ApproveRuntimeGrant(String),
    ExecutionStatus(String, String),
    Trace(String, String),
    AppEvents(String, String),
    AppSessions(String, String),
    AppCommands(String, String),
}

/// Start the HTTP/JSON API server, blocking until the listener fails.
///
/// # Errors
///
/// Returns [`ServeError`] when the server cannot bind or the accept loop fails.
#[allow(clippy::too_many_lines)]
pub fn serve_http_api<E>(config: ApiServerConfig<E>) -> Result<(), ServeError>
where
    E: LocalExecutor + Clone + Send + Sync + 'static,
{
    let listener = TcpListener::bind(&config.bind_address)
        .map_err(|e| ServeError::BindFailed(format!("{}: {e}", config.bind_address)))?;

    let connection_limits = ConnectionLimits {
        read_timeout: config
            .read_timeout
            .unwrap_or(Duration::from_secs(DEFAULT_READ_TIMEOUT_SECS)),
        write_timeout: config
            .write_timeout
            .unwrap_or(Duration::from_secs(DEFAULT_WRITE_TIMEOUT_SECS)),
        request_deadline: config
            .request_deadline
            .unwrap_or(Duration::from_secs(DEFAULT_REQUEST_DEADLINE_SECS)),
    };
    let worker_count = config
        .max_concurrent_connections
        .unwrap_or(DEFAULT_MAX_CONCURRENT_CONNECTIONS)
        .max(1);

    let local_addr = listener
        .local_addr()
        .map_err(|e| ServeError::BindFailed(format!("could not read local address: {e}")))?;
    let auth_mode = if config.requested_auth_mode.as_deref() == Some("dev-any") {
        "dev-any"
    } else if local_addr.ip().is_loopback() {
        "dev-loopback"
    } else {
        "bearer-required"
    };
    let local_dev_token = if matches!(auth_mode, "dev-loopback" | "dev-any") {
        Some(mint_local_dev_token(&local_addr.to_string()))
    } else {
        None
    };

    if auth_mode == "dev-any" {
        eprintln!("WARNING: dev-any: accepting connections from LAN. Do not use in production.");
    }

    if config.allow_unauthenticated {
        eprintln!(
            "WARNING: --allow-unauthenticated is set. Any caller on any network interface may \
             invoke this API without credentials. Do not use in production."
        );
    }

    eprintln!(
        "traverse-cli serve: HTTP/JSON API listening on http://{local_addr} (spec 033-http-json-api)"
    );
    let base_url = format!("http://{local_addr}");
    let mobile_connect_url = mobile_connect_url(&base_url, DEFAULT_WORKSPACE_ID, auth_mode);
    eprintln!("traverse-cli serve: mobile connect URL {mobile_connect_url}");
    if config.render_mobile_qr {
        eprintln!(
            "{}",
            render_mobile_connect_qr(&mobile_connect_url).map_err(ServeError::BindFailed)?
        );
    }
    let jwt_verification_key = resolve_jwt_verification_key(
        config.jwt_verification_key_hex.as_deref(),
        auth_mode,
        config.allow_unauthenticated,
    )?;
    let _ = std::io::stderr().flush();

    write_server_discovery(
        Path::new("."),
        &base_url,
        auth_mode,
        &mobile_connect_url,
        local_dev_token.as_deref(),
    )
    .map_err(ServeError::BindFailed)?;

    let mut workspaces = HashMap::new();
    workspaces.insert(
        SYSTEM_WORKSPACE_ID.to_string(),
        WorkspaceState {
            runtime: Runtime::new(config.capability_registry, config.executor.clone())
                .with_workflow_registry(config.workflow_registry)
                .with_security_config(runtime_security_for_auth_mode(auth_mode)),
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
            app_events: Vec::new(),
            app_list_context_fields: HashMap::new(),
            app_state_machines: HashMap::new(),
            runtime_grants: Vec::new(),
        },
    );

    let state = Arc::new(ApiState {
        allow_unauthenticated: config.allow_unauthenticated,
        auth_mode: auth_mode.to_string(),
        allowed_origins: config.allowed_origins,
        registry_root: config.registry_root,
        executor: config.executor,
        workspaces: Mutex::new(workspaces),
        idempotency_records: Mutex::new(HashMap::new()),
        idempotency_retention_seconds: configured_idempotency_retention(
            config.idempotency_retention_seconds,
        ),
        jwt_verification_key,
    });

    run_connection_pool(&listener, &state, connection_limits, worker_count)
}

/// Bounded socket timeouts and per-request time budget applied to every
/// connection serviced by [`run_connection_pool`] (spec 033-http-json-api
/// connection-handling model). Set before any read so a slow or idle client
/// cannot hold a worker indefinitely (CWE-400 / slowloris).
#[derive(Clone, Copy)]
struct ConnectionLimits {
    read_timeout: Duration,
    write_timeout: Duration,
    request_deadline: Duration,
}

/// Services accepted connections with a fixed-size worker pool so a single
/// slow or idle client cannot block other callers. The bounded channel
/// (`worker_count * 2` capacity) caps the number of connections that can be
/// in flight or queued at once; once full, `sender.send` applies backpressure
/// to the accept loop rather than growing threads or memory without bound.
fn run_connection_pool<E>(
    listener: &TcpListener,
    state: &Arc<ApiState<E>>,
    limits: ConnectionLimits,
    worker_count: usize,
) -> Result<(), ServeError>
where
    E: LocalExecutor + Clone + Send + Sync + 'static,
{
    let (sender, receiver) = mpsc::sync_channel::<TcpStream>(worker_count * 2);
    let receiver = Arc::new(Mutex::new(receiver));

    let workers: Vec<_> = (0..worker_count)
        .map(|_| {
            let receiver = Arc::clone(&receiver);
            let state = Arc::clone(state);
            thread::spawn(move || worker_loop(&receiver, &state, limits))
        })
        .collect();

    for connection in listener.incoming() {
        match connection {
            Ok(stream) => {
                if sender.send(stream).is_err() {
                    break;
                }
            }
            Err(e) => return Err(ServeError::AcceptFailed(e.to_string())),
        }
    }

    drop(sender);
    for worker in workers {
        let _ = worker.join();
    }

    Ok(())
}

fn worker_loop<E>(
    receiver: &Arc<Mutex<mpsc::Receiver<TcpStream>>>,
    state: &Arc<ApiState<E>>,
    limits: ConnectionLimits,
) where
    E: LocalExecutor + Clone,
{
    loop {
        let next = {
            let receiver = receiver.lock().unwrap_or_else(PoisonError::into_inner);
            receiver.recv()
        };
        let Ok(stream) = next else {
            break;
        };
        if let Err(e) = stream
            .set_read_timeout(Some(limits.read_timeout))
            .and_then(|()| stream.set_write_timeout(Some(limits.write_timeout)))
        {
            eprintln!("traverse-cli serve: failed to configure connection timeouts: {e}");
            continue;
        }
        if let Err(e) = handle_connection(stream, state, limits.request_deadline) {
            eprintln!("traverse-cli serve: connection error: {e}");
        }
    }
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
        .map_or(0, |duration| duration.as_secs())
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
    mobile_connect_url: &str,
    local_dev_token: Option<&str>,
) -> Result<PathBuf, String> {
    let traverse_dir = repo_root.join(".traverse");
    std::fs::create_dir_all(&traverse_dir)
        .map_err(|e| format!("failed to create .traverse directory: {e}"))?;
    let discovery_path = traverse_dir.join("server.json");
    let discovery = ServerDiscoveryV1 {
        schema_version: SERVER_DISCOVERY_SCHEMA_VERSION.to_string(),
        base_url: base_url.to_string(),
        bind_address: base_url
            .strip_prefix("http://")
            .unwrap_or(base_url)
            .to_string(),
        health_url: format!("{base_url}/healthz"),
        workspace_default: DEFAULT_WORKSPACE_ID.to_string(),
        pid: std::process::id(),
        started_at: generated_registered_at().map_err(|e| e.message)?,
        auth_mode: auth_mode.to_string(),
        mobile_connect_url: mobile_connect_url.to_string(),
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

fn mobile_connect_url(base_url: &str, workspace_default: &str, auth_mode: &str) -> String {
    format!(
        "traverse://connect?base_url={}&workspace_default={}&auth_mode={}",
        percent_encode_query_value(base_url),
        percent_encode_query_value(workspace_default),
        percent_encode_query_value(auth_mode)
    )
}

fn percent_encode_query_value(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            let _ = write!(&mut encoded, "%{byte:02X}");
        }
    }
    encoded
}

fn render_mobile_connect_qr(url: &str) -> Result<String, String> {
    let code = qrcode::QrCode::new(url.as_bytes())
        .map_err(|e| format!("failed to generate mobile provisioning QR code: {e}"))?;
    let width = code.width();
    let quiet_zone = 2usize;
    let total_width = width + quiet_zone * 2;
    let mut rendered = String::new();

    for y in 0..total_width {
        for x in 0..total_width {
            let dark = x >= quiet_zone
                && x < width + quiet_zone
                && y >= quiet_zone
                && y < width + quiet_zone
                && code[(x - quiet_zone, y - quiet_zone)] == qrcode::types::Color::Dark;
            rendered.push_str(if dark { "██" } else { "  " });
        }
        rendered.push('\n');
    }

    Ok(rendered)
}

fn is_trusted_dev_caller(peer_ip: IpAddr, auth_mode: &str) -> bool {
    match auth_mode {
        "dev-loopback" => peer_ip.is_loopback(),
        "dev-any" => peer_ip.is_loopback() || is_rfc1918_private_ip(peer_ip),
        _ => false,
    }
}

fn is_rfc1918_private_ip(peer_ip: IpAddr) -> bool {
    let IpAddr::V4(ipv4) = peer_ip else {
        return false;
    };
    let [a, b, _, _] = ipv4.octets();
    a == 10 || (a == 172 && (16..=31).contains(&b)) || (a == 192 && b == 168)
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
                    .with_workflow_registry(config.workflow_registry)
                    .with_security_config(RuntimeSecurityConfig::development()),
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
                app_events: Vec::new(),
                app_list_context_fields: HashMap::new(),
                app_state_machines: HashMap::new(),
                runtime_grants: Vec::new(),
            },
        );

        Self {
            state: ApiState {
                auth_mode: "dev-loopback".to_string(),
                allow_unauthenticated: config.allow_unauthenticated,
                allowed_origins: config.allowed_origins,
                registry_root: config.registry_root,
                executor: config.executor,
                workspaces: Mutex::new(workspaces),
                idempotency_records: Mutex::new(HashMap::new()),
                idempotency_retention_seconds: configured_idempotency_retention(
                    config.idempotency_retention_seconds,
                ),
                jwt_verification_key: None,
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
        let mut workspaces = self
            .workspaces
            .lock()
            .map_err(|_| "workspace registry lock poisoned".to_string())?;
        let entry = workspaces
            .entry(workspace_id.to_string())
            .or_insert_with(|| WorkspaceState {
                runtime: Runtime::new(CapabilityRegistry::new(), self.executor.clone())
                    .with_workflow_registry(WorkflowRegistry::new())
                    .with_security_config(RuntimeSecurityConfig::development()),
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
                app_events: Vec::new(),
                app_list_context_fields: HashMap::new(),
                app_state_machines: HashMap::new(),
                runtime_grants: Vec::new(),
            });

        if !entry.loaded_from_disk {
            entry.persisted = load_persisted_registry(&self.registry_root, workspace_id)?;
            load_workspace_app_runtime(self, workspace_id, entry)?;
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
    let mut persisted = if path.exists() {
        let bytes =
            std::fs::read(&path).map_err(|e| format!("failed to read persisted registry: {e}"))?;
        serde_json::from_slice(&bytes).map_err(|e| {
            format!(
                "failed to parse persisted registry at {}: {e}",
                path.display()
            )
        })?
    } else {
        PersistedWorkspaceRegistryV1 {
            schema_version: PERSISTED_REGISTRY_SCHEMA_VERSION.to_string(),
            registrations: Vec::new(),
            events: Vec::new(),
            workflows: Vec::new(),
        }
    };
    load_registry_journal(registry_root, workspace_id, &mut persisted)?;
    Ok(persisted)
}

fn persisted_registry_path(registry_root: &Path, workspace_id: &str) -> PathBuf {
    registry_root
        .join("workspaces")
        .join(workspace_id)
        .join("capabilities.json")
}

fn persisted_registry_journal_path(registry_root: &Path, workspace_id: &str) -> PathBuf {
    registry_root
        .join("workspaces")
        .join(workspace_id)
        .join("capabilities.jsonl")
}

fn workspace_metadata_path(registry_root: &Path, workspace_id: &str) -> PathBuf {
    registry_root
        .join("workspaces")
        .join(workspace_id)
        .join("workspace.json")
}

fn workspace_audit_log_path(registry_root: &Path, workspace_id: &str) -> PathBuf {
    registry_root
        .join("workspaces")
        .join(workspace_id)
        .join("audit.jsonl")
}

fn append_registry_mutation(
    registry_root: &Path,
    workspace_id: &str,
    mutation: &PersistedRegistryMutationV1,
) -> Result<(), String> {
    let path = persisted_registry_journal_path(registry_root, workspace_id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create persisted registry directory: {e}"))?;
    }

    let mut bytes = serde_json::to_vec(mutation)
        .map_err(|e| format!("failed to serialize persisted registry mutation: {e}"))?;
    bytes.push(b'\n');
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| format!("failed to open persisted registry journal: {e}"))?;
    file.write_all(&bytes)
        .map_err(|e| format!("failed to append persisted registry mutation: {e}"))?;
    file.sync_data()
        .map_err(|e| format!("failed to sync persisted registry mutation: {e}"))
}

fn load_registry_journal(
    registry_root: &Path,
    workspace_id: &str,
    persisted: &mut PersistedWorkspaceRegistryV1,
) -> Result<(), String> {
    let path = persisted_registry_journal_path(registry_root, workspace_id);
    if !path.exists() {
        return Ok(());
    }

    let bytes = std::fs::read(&path)
        .map_err(|e| format!("failed to read persisted registry journal: {e}"))?;
    for (index, line) in bytes.split(|byte| *byte == b'\n').enumerate() {
        if line.is_empty() {
            continue;
        }
        let mutation: PersistedRegistryMutationV1 = match serde_json::from_slice(line) {
            Ok(mutation) => mutation,
            Err(_error) if index + 1 == bytes.split(|byte| *byte == b'\n').count() => break,
            Err(error) => {
                return Err(format!(
                    "failed to parse persisted registry journal at {}: {error}",
                    path.display()
                ));
            }
        };
        persisted.registrations.extend(mutation.registrations);
        persisted.events.extend(mutation.events);
        persisted.workflows.extend(mutation.workflows);
    }
    Ok(())
}

fn append_workspace_audit<E: LocalExecutor + Clone>(
    state: &ApiState<E>,
    workspace_id: &str,
    entry: &Value,
) -> Result<(), String> {
    let path = workspace_audit_log_path(&state.registry_root, workspace_id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create audit log directory: {e}"))?;
    }
    let line =
        serde_json::to_string(entry).map_err(|e| format!("failed to serialize audit log: {e}"))?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| format!("failed to open audit log: {e}"))?;
    writeln!(file, "{line}").map_err(|e| format!("failed to append audit log: {e}"))
}

fn audit_workspace_event<E: LocalExecutor + Clone>(
    state: &ApiState<E>,
    workspace_id: &str,
    event_type: &str,
    identity: Option<&DerivedIdentity>,
    target_resource: Option<&str>,
    outcome: &str,
    traverse_code: Option<&str>,
) -> Result<(), String> {
    let mut entry = json!({
        "timestamp": generated_registered_at().map_err(|e| e.message)?,
        "workspace_id": workspace_id,
        "event_type": event_type,
        "outcome": outcome,
    });
    if let Some(identity) = identity {
        entry["subject_id"] = Value::String(identity.subject_id.clone());
        entry["effective_scopes"] = Value::Array(
            identity
                .scopes
                .iter()
                .map(|scope| Value::String(scope.clone()))
                .collect(),
        );
    }
    if let Some(target_resource) = target_resource {
        entry["target_resource"] = Value::String(target_resource.to_string());
    }
    if let Some(traverse_code) = traverse_code {
        entry["traverse_code"] = Value::String(traverse_code.to_string());
    }
    append_workspace_audit(state, workspace_id, &entry)
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
    Ok(format!("unix:{}", current_unix_seconds()?))
}

fn current_unix_seconds() -> Result<u64, ApiError> {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| ApiError {
            status: 500,
            reason: "Internal Server Error",
            code: "internal_error",
            message: format!("failed to read system time: {e}"),
        })?
        .as_secs();
    Ok(seconds)
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

fn is_dev_auth_mode(auth_mode: &str) -> bool {
    matches!(auth_mode, "dev-loopback" | "dev-any")
}

fn resolve_jwt_verification_key(
    key_hex: Option<&str>,
    auth_mode: &str,
    allow_unauthenticated: bool,
) -> Result<Option<ed25519_dalek::VerifyingKey>, ServeError> {
    let key = if let Some(hex) = key_hex {
        Some(parse_ed25519_verifying_key(hex).map_err(ServeError::BindFailed)?)
    } else {
        None
    };
    if auth_mode == "bearer-required" && key.is_none() && !allow_unauthenticated {
        eprintln!(
            "WARNING: bearer-required auth mode without a configured JWT verification key. \
             All bearer tokens will be rejected (fail closed). Set the \
             TRAVERSE_JWT_VERIFICATION_KEY environment variable to a hex-encoded \
             Ed25519 public key."
        );
    }
    Ok(key)
}

fn subject_from_state<E>(
    headers: &HashMap<String, String>,
    state: &ApiState<E>,
    loopback: bool,
) -> Result<DerivedIdentity, ApiError> {
    subject_from_request(
        headers,
        &state.auth_mode,
        state.allow_unauthenticated,
        loopback,
        state.jwt_verification_key.as_ref(),
    )
}

fn subject_from_request(
    headers: &HashMap<String, String>,
    auth_mode: &str,
    allow_unauthenticated: bool,
    loopback: bool,
    jwt_key: Option<&ed25519_dalek::VerifyingKey>,
) -> Result<DerivedIdentity, ApiError> {
    let token = headers
        .get("authorization")
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToString::to_string);

    if let Some(token) = token {
        if is_jwt_shaped(&token) {
            return derive_identity_from_jwt(&token, auth_mode, jwt_key);
        }

        // Non-JWT bearer tokens are accepted as direct subject identifiers only
        // in the local/dev auth modes. A network-facing (`bearer-required`)
        // listener requires a signature-verified JWT and never trusts an opaque
        // token, so it can never yield an admin identity.
        if !is_dev_auth_mode(auth_mode) {
            return Err(ApiError {
                status: 401,
                reason: "Unauthorized",
                code: "unauthorized",
                message: "a signed JWT bearer token is required".to_string(),
            });
        }

        validate_subject_id(&token).map_err(|msg| ApiError {
            status: 401,
            reason: "Unauthorized",
            code: "unauthorized",
            message: msg,
        })?;

        return Ok(DerivedIdentity {
            subject_id: token.clone(),
            is_admin: token == SYSTEM_ADMIN_SUBJECT,
            scopes: Vec::new(),
        });
    }

    if loopback && allow_unauthenticated {
        return Ok(DerivedIdentity {
            subject_id: "local".to_string(),
            is_admin: false,
            scopes: Vec::new(),
        });
    }

    Err(ApiError {
        status: 401,
        reason: "Unauthorized",
        code: "unauthorized",
        message: "Bearer token required".to_string(),
    })
}

fn is_jwt_shaped(token: &str) -> bool {
    let mut parts = token.split('.');
    let (Some(header), Some(payload), Some(signature), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    !header.is_empty() && !payload.is_empty() && !signature.is_empty()
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

fn unauthorized(code: &'static str, message: impl Into<String>) -> ApiError {
    ApiError {
        status: 401,
        reason: "Unauthorized",
        code,
        message: message.into(),
    }
}

/// Derive a caller identity from a JWT bearer token.
///
/// The token signature is verified against the configured `Ed25519` key before
/// any claim is trusted. A token is only allowed to yield an admin identity
/// when its signature verified. In the dev auth modes, when no key is
/// configured, claims are accepted as unverified but `is_admin` is forced to
/// `false`. In `bearer-required` mode a missing key rejects every token
/// (fail closed).
fn derive_identity_from_jwt(
    token: &str,
    auth_mode: &str,
    jwt_key: Option<&ed25519_dalek::VerifyingKey>,
) -> Result<DerivedIdentity, ApiError> {
    let mut parts = token.split('.');
    let (Some(header_b64), Some(payload_b64), Some(signature_b64), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(unauthorized("unauthorized", "malformed JWT bearer token"));
    };

    verify_jwt_header_alg(header_b64)?;

    let signature_verified = if let Some(key) = jwt_key {
        verify_jwt_signature(header_b64, payload_b64, signature_b64, key)?;
        true
    } else if is_dev_auth_mode(auth_mode) {
        false
    } else {
        // Fail closed: a network-facing listener with no verification key
        // configured cannot trust any token.
        return Err(unauthorized(
            "jwt_verification_unavailable",
            "server has no JWT verification key configured; bearer tokens are rejected",
        ));
    };

    // Zeroize the decoded credential bytes on drop (success and error paths),
    // so bearer-token material does not linger in memory (spec 030 NFR-001).
    let payload_bytes = Zeroizing::new(base64url_decode(payload_b64).map_err(|msg| ApiError {
        status: 401,
        reason: "Unauthorized",
        code: "unauthorized",
        message: msg,
    })?);

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

    validate_jwt_time_claims(&value)?;

    // Privilege claims are only honored for a signature-verified token. An
    // unverified (dev, no-key) token can name a subject but never escalates.
    let is_admin = signature_verified && jwt_claims_admin(&value);

    Ok(DerivedIdentity {
        subject_id,
        is_admin,
        scopes: parse_jwt_scopes(&value),
    })
}

fn jwt_claims_admin(value: &Value) -> bool {
    value
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
            .is_some_and(|s| s == "traverse_admin" || s == SYSTEM_ADMIN_SUBJECT)
}

fn validate_jwt_time_claims(value: &Value) -> Result<(), ApiError> {
    let exp = value.get("exp").and_then(Value::as_i64);
    let nbf = value.get("nbf").and_then(Value::as_i64);
    if exp.is_none() && nbf.is_none() {
        return Ok(());
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

    if let Some(exp) = exp
        && (exp <= 0 || now > exp)
    {
        return Err(unauthorized("token_expired", "token is expired"));
    }
    if let Some(nbf) = nbf
        && now < nbf
    {
        return Err(unauthorized(
            "token_not_yet_valid",
            "token is not yet valid",
        ));
    }
    Ok(())
}

const JWT_ALLOWED_ALG: &str = "EdDSA";

/// Enforce the JWT `alg` allow-list. Only `EdDSA` is accepted; `none` and every
/// other algorithm are rejected so an attacker cannot strip the signature.
fn verify_jwt_header_alg(header_b64: &str) -> Result<(), ApiError> {
    let header_bytes = Zeroizing::new(
        base64url_decode(header_b64)
            .map_err(|msg| unauthorized("unauthorized", format!("invalid JWT header: {msg}")))?,
    );
    let header: Value = serde_json::from_slice(&header_bytes)
        .map_err(|e| unauthorized("unauthorized", format!("invalid JWT header: {e}")))?;
    let alg = header
        .get("alg")
        .and_then(Value::as_str)
        .ok_or_else(|| unauthorized("token_alg_not_allowed", "JWT header missing 'alg'"))?;
    if alg != JWT_ALLOWED_ALG {
        return Err(unauthorized(
            "token_alg_not_allowed",
            format!("JWT alg '{alg}' is not allowed; only {JWT_ALLOWED_ALG} is accepted"),
        ));
    }
    Ok(())
}

/// Verify the Ed25519 signature over the `header.payload` signing input.
fn verify_jwt_signature(
    header_b64: &str,
    payload_b64: &str,
    signature_b64: &str,
    key: &ed25519_dalek::VerifyingKey,
) -> Result<(), ApiError> {
    use ed25519_dalek::Verifier;

    let signature_bytes = Zeroizing::new(
        base64url_decode(signature_b64)
            .map_err(|_| unauthorized("signature_verification_failed", "invalid JWT signature"))?,
    );
    let signature_array = <[u8; 64]>::try_from(signature_bytes.as_slice())
        .map_err(|_| unauthorized("signature_verification_failed", "invalid JWT signature"))?;
    let signature = ed25519_dalek::Signature::from_bytes(&signature_array);

    let signing_input = format!("{header_b64}.{payload_b64}");
    key.verify(signing_input.as_bytes(), &signature)
        .map_err(|_| {
            unauthorized(
                "signature_verification_failed",
                "JWT signature verification failed",
            )
        })
}

fn parse_ed25519_verifying_key(hex: &str) -> Result<ed25519_dalek::VerifyingKey, String> {
    let bytes = hex_decode(hex).map_err(|msg| format!("invalid JWT verification key: {msg}"))?;
    let array = <[u8; 32]>::try_from(bytes.as_slice())
        .map_err(|_| "JWT verification key must be 32 bytes (Ed25519 public key)".to_string())?;
    ed25519_dalek::VerifyingKey::from_bytes(&array)
        .map_err(|e| format!("invalid Ed25519 verification key: {e}"))
}

fn hex_decode(input: &str) -> Result<Vec<u8>, String> {
    if !input.len().is_multiple_of(2) {
        return Err("hex input must have an even number of digits".to_string());
    }
    let mut out = Vec::with_capacity(input.len() / 2);
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let hi = hex_nibble(bytes[i])?;
        let lo = hex_nibble(bytes[i + 1])?;
        out.push((hi << 4) | lo);
        i += 2;
    }
    Ok(out)
}

fn hex_nibble(byte: u8) -> Result<u8, String> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err("hex input contains a non-hex character".to_string()),
    }
}

fn parse_jwt_scopes(value: &Value) -> Vec<String> {
    let mut scopes = Vec::new();
    if let Some(scope) = value.get("scope").and_then(Value::as_str) {
        scopes.extend(scope.split_whitespace().map(ToString::to_string));
    }
    for claim in ["scp", "scopes"] {
        if let Some(items) = value.get(claim).and_then(Value::as_array) {
            scopes.extend(items.iter().filter_map(|item| {
                item.as_str()
                    .filter(|scope| !scope.trim().is_empty())
                    .map(ToString::to_string)
            }));
        }
    }
    scopes.sort();
    scopes.dedup();
    scopes
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

fn ensure_workspace_authorized(
    registry_root: &Path,
    workspace_id: &str,
    identity: &DerivedIdentity,
    required_scope: &str,
    scopes_optional: bool,
) -> Result<WorkspaceMetadataV1, ApiError> {
    if !scopes_optional && !identity_has_scope(identity, required_scope) {
        return Err(ApiError {
            status: 403,
            reason: "Forbidden",
            code: "unauthorized",
            message: format!("missing required scope `{required_scope}`"),
        });
    }
    ensure_workspace_access(registry_root, workspace_id, identity)
}

fn identity_has_scope(identity: &DerivedIdentity, required_scope: &str) -> bool {
    identity.is_admin
        || identity.scopes.iter().any(|scope| scope == required_scope)
        || identity.scopes.iter().any(|scope| scope == "*")
}

fn scopes_optional_for_request(
    _allow_unauthenticated: bool,
    loopback: bool,
    identity: &DerivedIdentity,
) -> bool {
    loopback || identity.subject_id == "local"
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
            WorkflowErrorCode::MissingRequiredField if err.path == "$.nodes" => {
                has_empty_nodes = true;
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
                signature: None,
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
    // Derived from the contract's own provenance rather than wall-clock time so
    // that re-registering an identical contract produces an identical record for
    // the duplicate-detection equality check in `EventRegistry::register`.
    let registered_at = contract.provenance.created_at.clone();

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

fn parse_bundle_register_body_for_workspace(
    body: &[u8],
    workspace_id: &str,
) -> Result<ParsedBundleRegistrationV1, ApiError> {
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

    reject_unknown_bundle_registration_wrapper_fields(&value)?;
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
    let scope_value = registration_scope_value(scope);

    let bundle = value.get("bundle").ok_or_else(|| ApiError {
        status: 400,
        reason: "Bad Request",
        code: "missing_bundle",
        message: "bundle is required".to_string(),
    })?;
    reject_unknown_bundle_fields(bundle)?;

    let mut capabilities = Vec::new();
    for item in bundle_array(bundle, "capabilities")? {
        let (_, _, registration) = parse_register_body_with_workspace(
            &serde_json::to_vec(&bundle_item_with_scope(item, scope_value)?).map_err(|e| {
                ApiError {
                    status: 400,
                    reason: "Bad Request",
                    code: "invalid_request",
                    message: format!("failed to serialize capability registration: {e}"),
                }
            })?,
            Some(workspace_id),
            true,
        )?;
        capabilities.push(registration);
    }

    let mut events = Vec::new();
    for item in bundle_array(bundle, "event_contracts")? {
        let (_, registration) = parse_event_register_body_for_workspace(
            &serde_json::to_vec(&bundle_item_with_scope(item, scope_value)?).map_err(|e| {
                ApiError {
                    status: 400,
                    reason: "Bad Request",
                    code: "invalid_request",
                    message: format!("failed to serialize event registration: {e}"),
                }
            })?,
            workspace_id,
        )?;
        events.push(registration);
    }

    let mut workflows = Vec::new();
    for item in bundle_array(bundle, "workflows")? {
        let (_, registration) = parse_workflow_register_body_for_workspace(
            &serde_json::to_vec(&bundle_item_with_scope(item, scope_value)?).map_err(|e| {
                ApiError {
                    status: 400,
                    reason: "Bad Request",
                    code: "invalid_request",
                    message: format!("failed to serialize workflow registration: {e}"),
                }
            })?,
            workspace_id,
        )?;
        workflows.push(registration);
    }

    reject_internal_bundle_duplicates(&capabilities, &events, &workflows)?;

    Ok(ParsedBundleRegistrationV1 {
        scope,
        capabilities,
        events,
        workflows,
    })
}

fn registration_scope_value(scope: RegistrationScope) -> &'static str {
    match scope {
        RegistrationScope::WorkspacePersisted => "workspace_persisted",
        RegistrationScope::SessionEphemeral => "session_ephemeral",
    }
}

fn bundle_item_with_scope(item: &Value, scope: &str) -> Result<Value, ApiError> {
    let mut item = item.clone();
    let Some(object) = item.as_object_mut() else {
        return Err(ApiError {
            status: 400,
            reason: "Bad Request",
            code: "invalid_bundle_artifact",
            message: "bundle artifact entries must be JSON objects".to_string(),
        });
    };
    object
        .entry("scope".to_string())
        .or_insert_with(|| Value::String(scope.to_string()));
    Ok(item)
}

fn bundle_array<'a>(bundle: &'a Value, key: &str) -> Result<&'a [Value], ApiError> {
    match bundle.get(key) {
        Some(Value::Array(values)) => Ok(values),
        Some(_) => Err(ApiError {
            status: 400,
            reason: "Bad Request",
            code: "invalid_bundle",
            message: format!("bundle.{key} must be an array"),
        }),
        None => Ok(&[]),
    }
}

fn reject_unknown_bundle_registration_wrapper_fields(value: &Value) -> Result<(), ApiError> {
    let Some(object) = value.as_object() else {
        return Err(ApiError {
            status: 400,
            reason: "Bad Request",
            code: "invalid_request",
            message: "bundle registration body must be a JSON object".to_string(),
        });
    };

    for key in object.keys() {
        if !matches!(key.as_str(), "workspace_id" | "scope" | "bundle") {
            return Err(ApiError {
                status: 400,
                reason: "Bad Request",
                code: "unknown_field",
                message: format!("unknown bundle registration field `{key}`"),
            });
        }
    }
    Ok(())
}

fn reject_unknown_bundle_fields(bundle: &Value) -> Result<(), ApiError> {
    let Some(object) = bundle.as_object() else {
        return Err(ApiError {
            status: 400,
            reason: "Bad Request",
            code: "invalid_bundle",
            message: "bundle must be a JSON object".to_string(),
        });
    };

    for key in object.keys() {
        if !matches!(
            key.as_str(),
            "capabilities" | "event_contracts" | "workflows"
        ) {
            return Err(ApiError {
                status: 400,
                reason: "Bad Request",
                code: "unknown_field",
                message: format!("unknown bundle field `{key}`"),
            });
        }
    }
    Ok(())
}

fn reject_internal_bundle_duplicates(
    capabilities: &[PersistedCapabilityRegistrationV1],
    events: &[PersistedEventRegistrationV1],
    workflows: &[PersistedWorkflowRegistrationV1],
) -> Result<(), ApiError> {
    let mut seen = HashSet::new();
    for registration in capabilities {
        reject_duplicate_bundle_key(
            &mut seen,
            "capability",
            &registration.contract.id,
            &registration.contract.version,
        )?;
    }
    for registration in events {
        reject_duplicate_bundle_key(
            &mut seen,
            "event_contract",
            &registration.contract.id,
            &registration.contract.version,
        )?;
    }
    for registration in workflows {
        reject_duplicate_bundle_key(
            &mut seen,
            "workflow",
            &registration.definition.id,
            &registration.definition.version,
        )?;
    }
    Ok(())
}

fn reject_duplicate_bundle_key(
    seen: &mut HashSet<String>,
    artifact_type: &str,
    artifact_id: &str,
    version: &str,
) -> Result<(), ApiError> {
    let key = format!("{artifact_type}:{artifact_id}:{version}");
    if !seen.insert(key) {
        return Err(ApiError {
            status: 409,
            reason: "Conflict",
            code: "duplicate_bundle_artifact",
            message: format!("bundle contains duplicate {artifact_type} {artifact_id}@{version}"),
        });
    }
    Ok(())
}

fn parse_runtime_grant_request(body: &[u8]) -> Result<RuntimeGrantRecord, ApiError> {
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
    reject_unknown_runtime_grant_fields(&value)?;

    let capability_id = required_non_empty_string(&value, "capability_id")?;
    let grant_scope = required_non_empty_string(&value, "grant_scope")?;
    let resource = required_non_empty_string(&value, "resource")?;
    let lifetime = match required_non_empty_string(&value, "lifetime")?.as_str() {
        "execution" => RuntimeGrantLifetime::Execution,
        "session" => RuntimeGrantLifetime::Session,
        other => {
            return Err(ApiError {
                status: 422,
                reason: "Unprocessable Entity",
                code: "invalid_grant_lifetime",
                message: format!("lifetime must be execution or session (got {other})"),
            });
        }
    };
    let expires_in_seconds = value
        .get("expires_in_seconds")
        .and_then(Value::as_u64)
        .unwrap_or(3600);
    let now_secs = current_unix_seconds()?;
    let expires_at = now_secs.saturating_add(expires_in_seconds);

    Ok(RuntimeGrantRecord {
        grant_id: String::new(),
        capability_id,
        grant_scope,
        resource,
        lifetime,
        approved_by: String::new(),
        granted_at: format!("unix:{now_secs}"),
        expires_at: format!("unix:{expires_at}"),
    })
}

fn reject_unknown_runtime_grant_fields(value: &Value) -> Result<(), ApiError> {
    let Some(object) = value.as_object() else {
        return Err(ApiError {
            status: 400,
            reason: "Bad Request",
            code: "invalid_request",
            message: "runtime grant body must be a JSON object".to_string(),
        });
    };
    for key in object.keys() {
        if !matches!(
            key.as_str(),
            "capability_id" | "grant_scope" | "resource" | "lifetime" | "expires_in_seconds"
        ) {
            return Err(ApiError {
                status: 400,
                reason: "Bad Request",
                code: "unknown_field",
                message: format!("unknown runtime grant field `{key}`"),
            });
        }
    }
    Ok(())
}

fn required_non_empty_string(value: &Value, key: &str) -> Result<String, ApiError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|v| !v.trim().is_empty())
        .map(ToString::to_string)
        .ok_or_else(|| ApiError {
            status: 422,
            reason: "Unprocessable Entity",
            code: "invalid_runtime_grant",
            message: format!("{key} is required"),
        })
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
    load_workspace_app_runtime(state, workspace_id, ws)?;
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

fn load_workspace_app_runtime<E: LocalExecutor + Clone>(
    state: &ApiState<E>,
    workspace_id: &str,
    ws: &mut WorkspaceState<E>,
) -> Result<(), String> {
    let workspace_root = state
        .registry_root
        .parent()
        .and_then(Path::parent)
        .unwrap_or_else(|| Path::new("."));
    match Runtime::from_workspace_app_state(
        workspace_root,
        workspace_id,
        state.executor.clone(),
        env!("CARGO_PKG_VERSION"),
    ) {
        Ok(runtime) => {
            let machines = load_workspace_app_state_machines(workspace_root, &runtime);
            ws.app_list_context_fields = machines
                .iter()
                .map(|(app_id, machine)| (app_id.clone(), machine.list_context_fields.clone()))
                .collect();
            ws.app_state_machines = machines;
            ws.runtime = runtime;
            Ok(())
        }
        Err(failure)
            if failure
                .errors
                .iter()
                .all(|error| error.code == WorkspaceAppStateErrorCode::MissingWorkspaceState) =>
        {
            Ok(())
        }
        Err(failure) => Err(format!(
            "workspace app state contains invalid entries: {}",
            render_workspace_app_state_failure(&failure)
        )),
    }
}

fn load_workspace_app_state_machines<E: LocalExecutor>(
    workspace_root: &Path,
    runtime: &Runtime<E>,
) -> HashMap<String, ApplicationStateMachine> {
    let mut machines = HashMap::new();
    for app in runtime.workspace_applications() {
        let manifest_path = PathBuf::from(&app.manifest_path);
        let manifest_path = if manifest_path.is_absolute() {
            manifest_path
        } else {
            workspace_root.join(manifest_path)
        };
        let Ok(manifest) = load_application_bundle_manifest(&manifest_path) else {
            continue;
        };
        let Some(state_machine) = manifest.state_machine else {
            continue;
        };
        machines.insert(app.app_id.clone(), state_machine);
    }
    machines
}

fn render_workspace_app_state_failure(
    failure: &traverse_registry::WorkspaceAppStateFailure,
) -> String {
    failure
        .errors
        .iter()
        .map(|error| format!("{:?}: {} ({})", error.code, error.message, error.path))
        .collect::<Vec<_>>()
        .join("; ")
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
    let mut workspaces = state
        .workspaces
        .lock()
        .map_err(|_| "workspace registry lock poisoned".to_string())?;
    let ws = workspaces
        .entry(workspace_id.to_string())
        .or_insert_with(|| WorkspaceState {
            runtime: Runtime::new(CapabilityRegistry::new(), state.executor.clone())
                .with_workflow_registry(WorkflowRegistry::new())
                .with_security_config(runtime_security_for_auth_mode(&state.auth_mode)),
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
            app_events: Vec::new(),
            app_list_context_fields: HashMap::new(),
            app_state_machines: HashMap::new(),
            runtime_grants: Vec::new(),
        });

    ensure_workspace_loaded(state, workspace_id, ws)?;

    match ws.runtime.register_capability(registration) {
        Ok(outcome) => {
            if scope == RegistrationScope::WorkspacePersisted && !outcome.already_registered {
                append_registry_mutation(
                    &state.registry_root,
                    workspace_id,
                    &PersistedRegistryMutationV1 {
                        registrations: vec![persisted_registration.clone()],
                        ..PersistedRegistryMutationV1::default()
                    },
                )?;
                ws.persisted.registrations.push(persisted_registration);
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
    let mut workspaces = state
        .workspaces
        .lock()
        .map_err(|_| "workspace registry lock poisoned".to_string())?;
    let ws = workspaces
        .entry(workspace_id.to_string())
        .or_insert_with(|| WorkspaceState {
            runtime: Runtime::new(CapabilityRegistry::new(), state.executor.clone())
                .with_workflow_registry(WorkflowRegistry::new())
                .with_security_config(runtime_security_for_auth_mode(&state.auth_mode)),
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
            app_events: Vec::new(),
            app_list_context_fields: HashMap::new(),
            app_state_machines: HashMap::new(),
            runtime_grants: Vec::new(),
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
                append_registry_mutation(
                    &state.registry_root,
                    workspace_id,
                    &PersistedRegistryMutationV1 {
                        events: vec![persisted_registration.clone()],
                        ..PersistedRegistryMutationV1::default()
                    },
                )?;
                ws.persisted.events.push(persisted_registration);
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
                    append_registry_mutation(
                        &state.registry_root,
                        workspace_id,
                        &PersistedRegistryMutationV1 {
                            workflows: vec![persisted.clone()],
                            ..PersistedRegistryMutationV1::default()
                        },
                    )?;
                    ws.persisted.workflows.push(persisted);
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

fn apply_bundle_registration<E: LocalExecutor + Clone>(
    state: &ApiState<E>,
    workspace_id: &str,
    bundle: ParsedBundleRegistrationV1,
) -> Result<Result<BundleRegistrationHttpOutcome, ApiError>, String> {
    state.with_workspace_mut(workspace_id, |ws| {
        let persisted_lengths = (
            ws.persisted.registrations.len(),
            ws.persisted.events.len(),
            ws.persisted.workflows.len(),
        );
        let mut staged_runtime = ws.runtime.clone();
        let mut staged_event_registry = ws.event_registry.clone();
        let mut staged_persisted = ws.persisted.clone();
        let mut outcomes = Vec::new();
        let mut all_already_registered = true;

        for persisted in bundle.events {
            let (already_registered, outcome) = match stage_bundle_event(
                &mut staged_event_registry,
                &mut staged_persisted,
                workspace_id,
                bundle.scope,
                persisted,
            ) {
                Ok(outcome) => outcome,
                Err(err) => return Ok(Err(err)),
            };
            all_already_registered &= already_registered;
            outcomes.push(outcome);
        }

        for persisted in bundle.capabilities {
            let (already_registered, outcome) = match stage_bundle_capability(
                &mut staged_runtime,
                &mut staged_persisted,
                workspace_id,
                bundle.scope,
                persisted,
            ) {
                Ok(outcome) => outcome,
                Err(err) => return Ok(Err(err)),
            };
            all_already_registered &= already_registered;
            outcomes.push(outcome);
        }

        for persisted in bundle.workflows {
            let (already_registered, outcome) = match stage_bundle_workflow(
                &mut staged_runtime,
                &mut staged_persisted,
                workspace_id,
                bundle.scope,
                persisted,
            ) {
                Ok(outcome) => outcome,
                Err(err) => return Ok(Err(err)),
            };
            all_already_registered &= already_registered;
            outcomes.push(outcome);
        }

        if bundle.scope == RegistrationScope::WorkspacePersisted {
            append_registry_mutation(
                &state.registry_root,
                workspace_id,
                &PersistedRegistryMutationV1 {
                    registrations: staged_persisted.registrations[persisted_lengths.0..].to_vec(),
                    events: staged_persisted.events[persisted_lengths.1..].to_vec(),
                    workflows: staged_persisted.workflows[persisted_lengths.2..].to_vec(),
                },
            )?;
        }

        ws.runtime = staged_runtime;
        ws.event_registry = staged_event_registry;
        ws.persisted = staged_persisted;

        Ok(Ok(BundleRegistrationHttpOutcome {
            already_registered: all_already_registered,
            outcomes,
        }))
    })
}

fn stage_bundle_event(
    staged_event_registry: &mut EventRegistry,
    staged_persisted: &mut PersistedWorkspaceRegistryV1,
    workspace_id: &str,
    scope: RegistrationScope,
    persisted: PersistedEventRegistrationV1,
) -> Result<(bool, Value), ApiError> {
    let registration = derive_event_registration(workspace_id, &persisted)?;
    let lookup_scope = match registration.scope {
        RegistryScope::Public => LookupScope::PublicOnly,
        RegistryScope::Private => LookupScope::PreferPrivate,
    };
    let existing = staged_event_registry.find_exact(
        lookup_scope,
        &registration.contract.id,
        &registration.contract.version,
    );
    match staged_event_registry.register(registration) {
        Ok(outcome) => {
            let already_registered = existing.is_some_and(|existing| {
                existing.record.contract_digest == outcome.record.contract_digest
            });
            if scope == RegistrationScope::WorkspacePersisted && !already_registered {
                staged_persisted.events.push(persisted);
            }
            let self_link = format!(
                "/v1/workspaces/{workspace_id}/event-contracts/{}/{}",
                outcome.record.id, outcome.record.version
            );
            Ok((
                already_registered,
                registration_outcome_value(
                    already_registered,
                    "event_contract",
                    &outcome.record.id,
                    &outcome.record.version,
                    &outcome.record.contract_digest,
                    scope,
                    &self_link,
                ),
            ))
        }
        Err(failure) => {
            let (status, code, reason) = map_event_registry_failure_http(&failure);
            Err(ApiError {
                status,
                reason,
                code,
                message: render_event_registry_failure_as_string(failure),
            })
        }
    }
}

fn stage_bundle_capability<E: LocalExecutor + Clone>(
    staged_runtime: &mut Runtime<E>,
    staged_persisted: &mut PersistedWorkspaceRegistryV1,
    workspace_id: &str,
    scope: RegistrationScope,
    persisted: PersistedCapabilityRegistrationV1,
) -> Result<(bool, Value), ApiError> {
    let registration = derive_registration(workspace_id, &persisted)?;
    match staged_runtime.register_capability(registration) {
        Ok(outcome) => {
            if scope == RegistrationScope::WorkspacePersisted && !outcome.already_registered {
                staged_persisted.registrations.push(persisted);
            }
            let self_link = format!(
                "/v1/workspaces/{workspace_id}/capabilities/{}/{}",
                outcome.record.id, outcome.record.version
            );
            Ok((
                outcome.already_registered,
                registration_outcome_value(
                    outcome.already_registered,
                    "capability",
                    &outcome.record.id,
                    &outcome.record.version,
                    &outcome.record.contract_digest,
                    scope,
                    &self_link,
                ),
            ))
        }
        Err(failure) => {
            let (status, code, reason) = map_registry_failure_http(&failure);
            Err(ApiError {
                status,
                reason,
                code,
                message: render_registry_failure_as_string(failure),
            })
        }
    }
}

fn stage_bundle_workflow<E: LocalExecutor + Clone>(
    staged_runtime: &mut Runtime<E>,
    staged_persisted: &mut PersistedWorkspaceRegistryV1,
    workspace_id: &str,
    scope: RegistrationScope,
    persisted: PersistedWorkflowRegistrationV1,
) -> Result<(bool, Value), ApiError> {
    let already_registered = staged_runtime
        .workflow_registry()
        .find_exact(
            LookupScope::PreferPrivate,
            &persisted.definition.id,
            &persisted.definition.version,
        )
        .is_some();
    let registration = derive_workflow_registration(workspace_id, &persisted)?;
    match staged_runtime.register_workflow(registration) {
        Ok(outcome) => {
            if scope == RegistrationScope::WorkspacePersisted && !already_registered {
                staged_persisted.workflows.push(persisted);
            }
            let self_link = format!(
                "/v1/workspaces/{workspace_id}/workflows/{}/{}",
                outcome.record.id, outcome.record.version
            );
            Ok((
                already_registered,
                registration_outcome_value(
                    already_registered,
                    "workflow",
                    &outcome.record.id,
                    &outcome.record.version,
                    &outcome.record.workflow_digest,
                    scope,
                    &self_link,
                ),
            ))
        }
        Err(failure) => {
            let definition = persisted.definition.clone();
            let (status, code, reason, _) = map_workflow_failure_http(&failure, &definition);
            Err(ApiError {
                status,
                reason,
                code,
                message: render_workflow_failure_as_string(failure),
            })
        }
    }
}

fn registration_outcome_value(
    already_registered: bool,
    artifact_type: &str,
    artifact_id: &str,
    version: &str,
    digest: &str,
    scope: RegistrationScope,
    self_link: &str,
) -> Value {
    json!({
        "api_version": "v1",
        "registered": !already_registered,
        "already_registered": already_registered,
        "artifact_type": artifact_type,
        "artifact_id": artifact_id,
        "version": version,
        "digest": digest,
        "scope": registration_scope_value(scope),
        "links": {
            "self": self_link
        }
    })
}

fn approve_runtime_grant<E: LocalExecutor + Clone>(
    state: &ApiState<E>,
    workspace_id: &str,
    identity: &DerivedIdentity,
    mut grant: RuntimeGrantRecord,
) -> Result<RuntimeGrantRecord, String> {
    state.with_workspace_mut(workspace_id, |ws| {
        let next_id = ws.runtime_grants.len() + 1;
        grant.grant_id = format!("grant_{next_id}");
        grant.approved_by = identity.subject_id.clone();
        ws.runtime_grants.push(grant.clone());
        Ok(grant)
    })
}

fn active_runtime_grants_for_execution<E: LocalExecutor + Clone>(
    state: &ApiState<E>,
    workspace_id: &str,
    capability_id: Option<&str>,
) -> Result<Vec<RuntimeGrantRecord>, String> {
    let now = current_unix_seconds().map_err(|e| e.message)?;
    let (active, expired) = state.with_workspace_mut(workspace_id, |ws| {
        let mut expired = Vec::new();
        ws.runtime_grants.retain(|grant| {
            if grant_expiration_seconds(grant).is_some_and(|expires| expires <= now) {
                expired.push(grant.clone());
                return false;
            }
            true
        });
        let Some(capability_id) = capability_id else {
            return Ok((Vec::new(), expired));
        };
        let active = ws
            .runtime_grants
            .iter()
            .filter(|grant| grant.capability_id == capability_id)
            .cloned()
            .collect();
        Ok((active, expired))
    })?;
    for grant in expired {
        audit_workspace_event(
            state,
            workspace_id,
            "runtime_grant_expired",
            None,
            Some(&grant.grant_id),
            "expired",
            None,
        )?;
    }
    Ok(active)
}

fn consume_execution_runtime_grants<E: LocalExecutor + Clone>(
    state: &ApiState<E>,
    workspace_id: &str,
    execution_grants: &[RuntimeGrantRecord],
) -> Result<(), String> {
    let consumed: HashSet<&str> = execution_grants
        .iter()
        .filter(|grant| grant.lifetime == RuntimeGrantLifetime::Execution)
        .map(|grant| grant.grant_id.as_str())
        .collect();
    if consumed.is_empty() {
        return Ok(());
    }
    state.with_workspace_mut(workspace_id, |ws| {
        ws.runtime_grants
            .retain(|grant| !consumed.contains(grant.grant_id.as_str()));
        Ok(())
    })?;
    for grant in execution_grants {
        if grant.lifetime == RuntimeGrantLifetime::Execution {
            audit_workspace_event(
                state,
                workspace_id,
                "runtime_grant_revoked",
                None,
                Some(&grant.grant_id),
                "revoked",
                None,
            )?;
        }
    }
    Ok(())
}

fn grant_expiration_seconds(grant: &RuntimeGrantRecord) -> Option<u64> {
    grant.expires_at.strip_prefix("unix:")?.parse().ok()
}

fn runtime_grants_json(grants: &[RuntimeGrantRecord]) -> Value {
    Value::Array(
        grants
            .iter()
            .map(|grant| {
                json!({
                    "grant_id": grant.grant_id,
                    "capability_id": grant.capability_id,
                    "grant_scope": grant.grant_scope,
                    "resource": grant.resource,
                    "lifetime": grant.lifetime,
                    "approved_by": grant.approved_by,
                    "granted_at": grant.granted_at,
                    "expires_at": grant.expires_at,
                })
            })
            .collect(),
    )
}

fn static_permissions_json<E: LocalExecutor + Clone>(
    state: &ApiState<E>,
    workspace_id: &str,
    capability_id: Option<&str>,
    capability_version: Option<&str>,
) -> Result<Value, String> {
    let Some(capability_id) = capability_id else {
        return Ok(Value::Array(Vec::new()));
    };
    let capability_version = capability_version.unwrap_or("1.0.0");
    state.with_workspace_mut(workspace_id, |ws| {
        let permissions = ws
            .runtime
            .capability_registry()
            .find_exact(
                LookupScope::PreferPrivate,
                capability_id,
                capability_version,
            )
            .map(|resolved| resolved.contract.permissions)
            .unwrap_or_default();
        Ok(json!(permissions))
    })
}

// ---------------------------------------------------------------------------
// Connection handler
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_lines)]
fn handle_connection<E: LocalExecutor + Clone>(
    mut stream: TcpStream,
    state: &ApiState<E>,
    request_deadline: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + request_deadline;
    let request = match read_http_request(&mut stream, deadline) {
        Ok(request) => request,
        Err(message) => {
            let (status, reason, code) = if message.contains("timed out") {
                (408, "Request Timeout", "request_timeout")
            } else if message.contains("too many headers") {
                (431, "Request Header Fields Too Large", "too_many_headers")
            } else if message.contains("headers too large") {
                (431, "Request Header Fields Too Large", "header_too_large")
            } else if message.contains("too large") {
                (413, "Payload Too Large", "payload_too_large")
            } else {
                (400, "Bad Request", "invalid_request")
            };
            return write_json(&mut stream, status, reason, &error_envelope(code, &message));
        }
    };

    let peer_ip = stream
        .peer_addr()
        .map_or(IpAddr::from([127, 0, 0, 1]), |a| a.ip());
    let trusted_dev_caller = is_trusted_dev_caller(peer_ip, &state.auth_mode);

    if reject_dev_any_public_caller(&mut stream, &state.auth_mode, trusted_dev_caller)? {
        return Ok(());
    }

    if request.method == "OPTIONS" {
        return handle_cors_preflight(&mut stream, &request, state, trusted_dev_caller);
    }

    let cors_headers = match cors_response_headers(&request, state, trusted_dev_caller) {
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

    if request.path != "/healthz" && !state.allow_unauthenticated && !trusted_dev_caller {
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
        ("GET", "/healthz") => handle_health(&mut response, &state.auth_mode),
        ("GET", "/v1/capabilities") => {
            handle_list_capabilities(&mut response, &request, state, trusted_dev_caller)
        }
        ("POST", "/v1/capabilities/register") => {
            handle_register_capability(&mut response, &request, state, trusted_dev_caller)
        }
        ("POST", "/v1/capabilities/execute") => {
            handle_execute(&mut response, &request, state, trusted_dev_caller)
        }
        (method, path) if workspace_operation_path(method, path).is_some() => {
            handle_workspace_operation(&mut response, &request, state, trusted_dev_caller)
        }
        ("POST", "/v1/workflows/register") => {
            handle_register_workflow(&mut response, &request, state, trusted_dev_caller)
        }
        ("GET", "/v1/workflows") => {
            handle_list_workflows(&mut response, &request, state, trusted_dev_caller)
        }
        ("GET", path) if path.starts_with("/v1/workflows/") => handle_get_workflow(
            &mut response,
            &request,
            state,
            trusted_dev_caller,
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

fn reject_dev_any_public_caller<W: Write>(
    w: &mut W,
    auth_mode: &str,
    trusted_dev_caller: bool,
) -> Result<bool, String> {
    if auth_mode != "dev-any" || trusted_dev_caller {
        return Ok(false);
    }

    write_json(
        w,
        403,
        "Forbidden",
        &error_envelope(
            "dev_any_public_ip_forbidden",
            "auth_mode: dev-any does not allow public IPs",
        ),
    )?;
    Ok(true)
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
        WorkspaceOperation::RegisterBundle(workspace_id) => {
            handle_register_workspace_bundle(w, request, state, loopback, &workspace_id)
        }
        WorkspaceOperation::ApproveRuntimeGrant(workspace_id) => {
            handle_approve_runtime_grant(w, request, state, loopback, &workspace_id)
        }
        WorkspaceOperation::ExecutionStatus(workspace_id, execution_id) => {
            handle_execution_status(w, request, state, loopback, &workspace_id, &execution_id)
        }
        WorkspaceOperation::Trace(workspace_id, execution_id) => {
            handle_trace_fetch(w, request, state, loopback, &workspace_id, &execution_id)
        }
        WorkspaceOperation::AppEvents(workspace_id, app_id) => {
            handle_app_events(w, request, state, loopback, &workspace_id, &app_id)
        }
        WorkspaceOperation::AppSessions(workspace_id, app_id) => {
            handle_app_sessions(w, request, state, loopback, &workspace_id, &app_id)
        }
        WorkspaceOperation::AppCommands(workspace_id, app_id) => {
            handle_app_commands(w, request, state, loopback, &workspace_id, &app_id)
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

fn handle_health<W: Write>(w: &mut W, auth_mode: &str) -> Result<(), String> {
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

    let identity = match subject_from_state(&request.headers, state, loopback) {
        Ok(identity) => identity,
        Err(err) => {
            audit_workspace_event(
                state,
                &workspace_id,
                "auth_failure",
                None,
                Some("registry_read"),
                "failure",
                Some(err.code),
            )?;
            return write_json(
                w,
                err.status,
                err.reason,
                &error_envelope(err.code, &err.message),
            );
        }
    };

    let _ = match ensure_workspace_authorized(
        &state.registry_root,
        &workspace_id,
        &identity,
        SCOPE_REGISTRY_READ,
        scopes_optional_for_request(state.allow_unauthenticated, loopback, &identity),
    ) {
        Ok(metadata) => metadata,
        Err(err) => {
            audit_workspace_event(
                state,
                &workspace_id,
                "auth_failure",
                Some(&identity),
                Some("registry_read"),
                "failure",
                Some(err.code),
            )?;
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
    let identity = match subject_from_state(&request.headers, state, loopback) {
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

    let _ = match ensure_workspace_authorized(
        &state.registry_root,
        &workspace_id,
        &identity,
        SCOPE_REGISTRY_WRITE,
        scopes_optional_for_request(state.allow_unauthenticated, loopback, &identity),
    ) {
        Ok(metadata) => metadata,
        Err(err) => {
            audit_workspace_event(
                state,
                &workspace_id,
                "auth_failure",
                Some(&identity),
                Some("capability_registration"),
                "failure",
                Some(err.code),
            )?;
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
    let identity = match subject_from_state(&request.headers, state, loopback) {
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
    let target_resource = format!(
        "capability:{}@{}",
        persisted_registration.contract.id, persisted_registration.contract.version
    );
    audit_workspace_event(
        state,
        workspace_id,
        "registration_attempted",
        Some(&identity),
        Some(&target_resource),
        "attempted",
        None,
    )?;

    let _ = match ensure_workspace_authorized(
        &state.registry_root,
        workspace_id,
        &identity,
        SCOPE_REGISTRY_WRITE,
        scopes_optional_for_request(state.allow_unauthenticated, loopback, &identity),
    ) {
        Ok(metadata) => metadata,
        Err(err) => {
            audit_workspace_event(
                state,
                workspace_id,
                "auth_failure",
                Some(&identity),
                Some(&target_resource),
                "failure",
                Some(err.code),
            )?;
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

    let result = apply_registration(
        state,
        workspace_id,
        scope,
        persisted_registration,
        registration,
    )?;
    write_workspace_capability_registration_result(
        w,
        state,
        workspace_id,
        &identity,
        scope,
        &target_resource,
        result,
    )
}

fn write_workspace_capability_registration_result<W: Write, E: LocalExecutor + Clone>(
    w: &mut W,
    state: &ApiState<E>,
    workspace_id: &str,
    identity: &DerivedIdentity,
    scope: RegistrationScope,
    target_resource: &str,
    result: Result<traverse_registry::RegistrationOutcome, traverse_registry::RegistryFailure>,
) -> Result<(), String> {
    match result {
        Ok(outcome) => {
            let status = if outcome.already_registered { 200 } else { 201 };
            let scope_name = match scope {
                RegistrationScope::WorkspacePersisted => "workspace_persisted",
                RegistrationScope::SessionEphemeral => "session_ephemeral",
            };
            audit_workspace_event(
                state,
                workspace_id,
                "registration_outcome",
                Some(identity),
                Some(target_resource),
                "success",
                None,
            )?;
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
            audit_workspace_event(
                state,
                workspace_id,
                "registration_outcome",
                Some(identity),
                Some(target_resource),
                "failure",
                Some(code),
            )?;
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
    let identity = match subject_from_state(&request.headers, state, loopback) {
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

    let _ = match ensure_workspace_authorized(
        &state.registry_root,
        workspace_id,
        &identity,
        SCOPE_REGISTRY_WRITE,
        scopes_optional_for_request(state.allow_unauthenticated, loopback, &identity),
    ) {
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
    let identity = match subject_from_state(&request.headers, state, loopback) {
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

    let _ = match ensure_workspace_authorized(
        &state.registry_root,
        workspace_id,
        &identity,
        SCOPE_REGISTRY_WRITE,
        scopes_optional_for_request(state.allow_unauthenticated, loopback, &identity),
    ) {
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

fn handle_register_workspace_bundle<W: Write, E: LocalExecutor + Clone>(
    w: &mut W,
    request: &HttpRequest,
    state: &ApiState<E>,
    loopback: bool,
    workspace_id: &str,
) -> Result<(), String> {
    let identity = match subject_from_state(&request.headers, state, loopback) {
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

    let bundle = match parse_bundle_register_body_for_workspace(&request.body, workspace_id) {
        Ok(bundle) => bundle,
        Err(err) => {
            return write_json(
                w,
                err.status,
                err.reason,
                &error_envelope(err.code, &err.message),
            );
        }
    };

    let _ = match ensure_workspace_authorized(
        &state.registry_root,
        workspace_id,
        &identity,
        SCOPE_REGISTRY_WRITE,
        scopes_optional_for_request(state.allow_unauthenticated, loopback, &identity),
    ) {
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

    match apply_bundle_registration(state, workspace_id, bundle)? {
        Ok(outcome) => {
            let status = if outcome.already_registered { 200 } else { 201 };
            write_json(
                w,
                status,
                if status == 200 { "OK" } else { "Created" },
                &json!({
                    "api_version": "v1",
                    "registered": !outcome.already_registered,
                    "already_registered": outcome.already_registered,
                    "outcomes": outcome.outcomes
                }),
            )
        }
        Err(err) => write_json(
            w,
            err.status,
            err.reason,
            &error_envelope(err.code, &err.message),
        ),
    }
}

fn handle_approve_runtime_grant<W: Write, E: LocalExecutor + Clone>(
    w: &mut W,
    request: &HttpRequest,
    state: &ApiState<E>,
    loopback: bool,
    workspace_id: &str,
) -> Result<(), String> {
    let identity = match subject_from_state(&request.headers, state, loopback) {
        Ok(identity) => identity,
        Err(err) => {
            audit_workspace_event(
                state,
                workspace_id,
                "auth_failure",
                None,
                Some("runtime_grants"),
                "failure",
                Some(err.code),
            )?;
            return write_json(
                w,
                err.status,
                err.reason,
                &error_envelope(err.code, &err.message),
            );
        }
    };

    let _ = match ensure_workspace_authorized(
        &state.registry_root,
        workspace_id,
        &identity,
        SCOPE_GRANTS_APPROVE,
        false,
    ) {
        Ok(metadata) => metadata,
        Err(err) => {
            audit_workspace_event(
                state,
                workspace_id,
                "auth_failure",
                Some(&identity),
                Some("runtime_grants"),
                "failure",
                Some(err.code),
            )?;
            return write_json(
                w,
                err.status,
                err.reason,
                &error_envelope(err.code, &err.message),
            );
        }
    };

    let grant = match parse_runtime_grant_request(&request.body) {
        Ok(grant) => grant,
        Err(err) => {
            return write_json(
                w,
                err.status,
                err.reason,
                &error_envelope(err.code, &err.message),
            );
        }
    };
    let grant = approve_runtime_grant(state, workspace_id, &identity, grant)?;
    audit_workspace_event(
        state,
        workspace_id,
        "runtime_grant_created",
        Some(&identity),
        Some(&grant.grant_id),
        "created",
        None,
    )?;

    write_json(
        w,
        201,
        "Created",
        &json!({
            "api_version": "v1",
            "approved": true,
            "grant": grant,
            "links": {
                "workspace": format!("/v1/workspaces/{workspace_id}")
            }
        }),
    )
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

    let identity = match subject_from_state(&request.headers, state, loopback) {
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

    let _ = match ensure_workspace_authorized(
        &state.registry_root,
        &workspace_id,
        &identity,
        SCOPE_RUNTIME_EXECUTE,
        scopes_optional_for_request(state.allow_unauthenticated, loopback, &identity),
    ) {
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

    let identity = match subject_from_state(&request.headers, state, loopback) {
        Ok(identity) => identity,
        Err(err) => {
            audit_workspace_event(
                state,
                workspace_id,
                "auth_failure",
                None,
                Some("workspace_execute"),
                "failure",
                Some(err.code),
            )?;
            return write_json(
                w,
                err.status,
                err.reason,
                &error_envelope(err.code, &err.message),
            );
        }
    };

    let _ = match ensure_workspace_authorized(
        &state.registry_root,
        workspace_id,
        &identity,
        SCOPE_RUNTIME_EXECUTE,
        scopes_optional_for_request(state.allow_unauthenticated, loopback, &identity),
    ) {
        Ok(metadata) => metadata,
        Err(err) => {
            audit_workspace_event(
                state,
                workspace_id,
                "auth_failure",
                Some(&identity),
                Some("workspace_execute"),
                "failure",
                Some(err.code),
            )?;
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

    handle_sync_workspace_execution(w, request, state, workspace_id, runtime_request)
}

fn handle_sync_workspace_execution<W: Write, E: LocalExecutor + Clone>(
    w: &mut W,
    request: &HttpRequest,
    state: &ApiState<E>,
    workspace_id: &str,
    runtime_request: RuntimeRequest,
) -> Result<(), String> {
    let capability_id = runtime_request.intent.capability_id.clone();
    let capability_version = runtime_request.intent.capability_version.clone();
    let runtime_grants =
        active_runtime_grants_for_execution(state, workspace_id, capability_id.as_deref())?;
    let static_permissions = static_permissions_json(
        state,
        workspace_id,
        capability_id.as_deref(),
        capability_version.as_deref(),
    )?;
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
    record_app_execution_events(state, workspace_id, &outcome)?;
    for grant in &runtime_grants {
        audit_workspace_event(
            state,
            workspace_id,
            "runtime_grant_used",
            None,
            Some(&grant.grant_id),
            "used",
            None,
        )?;
    }

    let body = json!({
        "api_version": "v1",
        "execution_id": outcome.result.execution_id,
        "status": status,
        "output": outcome.result.output,
        "error": outcome.result.error.as_ref().map(|e| json!({
            "code": format!("{:?}", e.code).to_lowercase(),
            "message": e.message,
        })),
        "static_permissions": static_permissions,
        "runtime_grants": runtime_grants_json(&runtime_grants),
        "links": execution_links(workspace_id, &outcome.result.execution_id, false),
    });
    consume_execution_runtime_grants(state, workspace_id, &runtime_grants)?;
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
            if let Ok(value) = parse_simplified_workspace_execute_request(body_str) {
                Ok(value)
            } else {
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
}

#[derive(Debug, Deserialize)]
struct SimplifiedWorkspaceExecuteRequest {
    capability_id: String,
    #[serde(default)]
    capability_version: Option<String>,
    input: Value,
    #[serde(default)]
    request_id: Option<String>,
}

fn parse_simplified_workspace_execute_request(body: &str) -> Result<RuntimeRequest, ()> {
    let request: SimplifiedWorkspaceExecuteRequest = serde_json::from_str(body).map_err(|_| ())?;
    let capability_id = request.capability_id.trim();
    if capability_id.is_empty() {
        return Err(());
    }
    let capability_version = request
        .capability_version
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("1.0.0")
        .to_string();
    let request_id = request
        .request_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map_or_else(
            || {
                format!(
                    "http_{}",
                    capability_id
                        .chars()
                        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
                        .collect::<String>()
                )
            },
            ToString::to_string,
        );

    Ok(RuntimeRequest {
        kind: "runtime_request".to_string(),
        schema_version: "1.0.0".to_string(),
        request_id,
        intent: RuntimeIntent {
            capability_id: Some(capability_id.to_string()),
            capability_version: Some(capability_version),
            version_range: None,
            intent_key: None,
        },
        input: request.input,
        lookup: RuntimeLookup {
            scope: RuntimeLookupScope::PreferPrivate,
            allow_ambiguity: false,
        },
        context: RuntimeContext {
            requested_target: PlacementTarget::Local,
            correlation_id: None,
            caller: Some("workspace-execute".to_string()),
            traceparent: None,
            tracestate: None,
            metadata: None,
            identity: None,
        },
        governing_spec: "006-runtime-request-execution".to_string(),
    })
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

fn workspace_bundles_path(path: &str) -> Option<String> {
    let suffix = path.strip_prefix("/v1/workspaces/")?;
    let workspace_id = suffix.strip_suffix("/bundles")?;
    if workspace_id.trim().is_empty() || workspace_id.contains('/') {
        return None;
    }
    Some(workspace_id.to_string())
}

fn workspace_runtime_grants_path(path: &str) -> Option<String> {
    let suffix = path.strip_prefix("/v1/workspaces/")?;
    let workspace_id = suffix.strip_suffix("/runtime-grants")?;
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
            .or_else(|| workspace_workflows_path(path).map(WorkspaceOperation::RegisterWorkflow))
            .or_else(|| workspace_bundles_path(path).map(WorkspaceOperation::RegisterBundle))
            .or_else(|| {
                workspace_runtime_grants_path(path).map(WorkspaceOperation::ApproveRuntimeGrant)
            })
            .or_else(|| {
                workspace_app_commands_path(path).map(|(workspace_id, app_id)| {
                    WorkspaceOperation::AppCommands(workspace_id, app_id)
                })
            }),
        "GET" => workspace_execution_status_path(path)
            .map(|(workspace_id, execution_id)| {
                WorkspaceOperation::ExecutionStatus(workspace_id, execution_id)
            })
            .or_else(|| {
                workspace_trace_path(path).map(|(workspace_id, execution_id)| {
                    WorkspaceOperation::Trace(workspace_id, execution_id)
                })
            })
            .or_else(|| {
                workspace_app_events_path(path).map(|(workspace_id, app_id)| {
                    WorkspaceOperation::AppEvents(workspace_id, app_id)
                })
            })
            .or_else(|| {
                workspace_app_sessions_path(path).map(|(workspace_id, app_id)| {
                    WorkspaceOperation::AppSessions(workspace_id, app_id)
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

fn workspace_app_events_path(path: &str) -> Option<(String, String)> {
    let suffix = path.strip_prefix("/v1/workspaces/")?;
    let (workspace_id, tail) = suffix.split_once("/apps/")?;
    let app_id = tail.strip_suffix("/events")?;
    if workspace_id.trim().is_empty()
        || app_id.trim().is_empty()
        || workspace_id.contains('/')
        || app_id.contains('/')
    {
        return None;
    }
    Some((workspace_id.to_string(), app_id.to_string()))
}

fn workspace_app_commands_path(path: &str) -> Option<(String, String)> {
    let suffix = path.strip_prefix("/v1/workspaces/")?;
    let (workspace_id, tail) = suffix.split_once("/apps/")?;
    let app_id = tail.strip_suffix("/commands")?;
    if workspace_id.trim().is_empty()
        || app_id.trim().is_empty()
        || workspace_id.contains('/')
        || app_id.contains('/')
    {
        return None;
    }
    Some((workspace_id.to_string(), app_id.to_string()))
}

fn workspace_app_sessions_path(path: &str) -> Option<(String, String)> {
    let suffix = path.strip_prefix("/v1/workspaces/")?;
    let (workspace_id, tail) = suffix.split_once("/apps/")?;
    let app_id = tail.strip_suffix("/sessions")?;
    if workspace_id.trim().is_empty()
        || app_id.trim().is_empty()
        || workspace_id.contains('/')
        || app_id.contains('/')
    {
        return None;
    }
    Some((workspace_id.to_string(), app_id.to_string()))
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

fn handle_app_events<W: Write, E: LocalExecutor + Clone>(
    w: &mut W,
    request: &HttpRequest,
    state: &ApiState<E>,
    loopback: bool,
    workspace_id: &str,
    app_id: &str,
) -> Result<(), String> {
    let identity = match subject_from_state(&request.headers, state, loopback) {
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

    let _ = match ensure_workspace_authorized(
        &state.registry_root,
        workspace_id,
        &identity,
        SCOPE_RUNTIME_EVENTS_READ,
        scopes_optional_for_request(state.allow_unauthenticated, loopback, &identity),
    ) {
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

    let last_event_id = request.headers.get("last-event-id").map(String::as_str);
    let events = state.with_workspace_mut(workspace_id, |ws| {
        Ok(replay_app_events(&ws.app_events, app_id, last_event_id))
    })?;
    let body = if events.is_empty() {
        serialize_sse_event(
            None,
            "heartbeat",
            &json!({
                "workspace_id": workspace_id,
                "app_id": app_id,
                "timestamp": generated_registered_at().map_err(|e| e.message)?,
            }),
        )?
    } else {
        let mut body = String::new();
        for event in events {
            body.push_str(&serialize_sse_event(
                Some(&event.event_id),
                &event.event_type,
                &event.data,
            )?);
        }
        body
    };

    write_raw_with_headers(
        w,
        200,
        "OK",
        "text/event-stream",
        body.as_bytes(),
        &[
            HeaderLine {
                name: "Cache-Control".to_string(),
                value: "no-cache".to_string(),
            },
            HeaderLine {
                name: "X-Accel-Buffering".to_string(),
                value: "no".to_string(),
            },
        ],
    )
}

fn handle_app_sessions<W: Write, E: LocalExecutor + Clone>(
    w: &mut W,
    request: &HttpRequest,
    state: &ApiState<E>,
    loopback: bool,
    workspace_id: &str,
    app_id: &str,
) -> Result<(), String> {
    let identity = match subject_from_state(&request.headers, state, loopback) {
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

    let _ = match ensure_workspace_authorized(
        &state.registry_root,
        workspace_id,
        &identity,
        SCOPE_RUNTIME_EVENTS_READ,
        scopes_optional_for_request(state.allow_unauthenticated, loopback, &identity),
    ) {
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

    let state_filter = request.query.get("state").map(String::as_str);
    let limit = match parse_sessions_limit(request.query.get("limit").map(String::as_str)) {
        Ok(limit) => limit,
        Err(message) => {
            return write_json(
                w,
                400,
                "Bad Request",
                &error_envelope("invalid_query", &message),
            );
        }
    };
    let cursor = request.query.get("cursor").map(String::as_str);
    let order = match parse_sessions_order(request.query.get("order").map(String::as_str)) {
        Ok(order) => order,
        Err(message) => {
            return write_json(
                w,
                400,
                "Bad Request",
                &error_envelope("invalid_query", &message),
            );
        }
    };
    let response = state.with_workspace_mut(workspace_id, |ws| {
        Ok(app_sessions_response(
            &ws.app_events,
            ws.app_list_context_fields
                .get(app_id)
                .map_or(&[], Vec::as_slice),
            app_id,
            state_filter,
            limit,
            cursor,
            order,
        ))
    })?;

    write_json(w, 200, "OK", &response)
}

fn handle_app_commands<W: Write, E: LocalExecutor + Clone>(
    w: &mut W,
    request: &HttpRequest,
    state: &ApiState<E>,
    loopback: bool,
    workspace_id: &str,
    app_id: &str,
) -> Result<(), String> {
    let identity = match subject_from_state(&request.headers, state, loopback) {
        Ok(identity) => identity,
        Err(err) => {
            audit_workspace_event(
                state,
                workspace_id,
                "auth_failure",
                None,
                Some("app_command_dispatch"),
                "failure",
                Some(err.code),
            )?;
            return write_json(
                w,
                err.status,
                err.reason,
                &error_envelope(err.code, &err.message),
            );
        }
    };

    let _ = match ensure_workspace_authorized(
        &state.registry_root,
        workspace_id,
        &identity,
        SCOPE_RUNTIME_EXECUTE,
        scopes_optional_for_request(state.allow_unauthenticated, loopback, &identity),
    ) {
        Ok(metadata) => metadata,
        Err(err) => {
            audit_workspace_event(
                state,
                workspace_id,
                "auth_failure",
                Some(&identity),
                Some("app_command_dispatch"),
                "failure",
                Some(err.code),
            )?;
            return write_json(
                w,
                err.status,
                err.reason,
                &error_envelope(err.code, &err.message),
            );
        }
    };

    let parsed = match parse_app_command_request(&request.body) {
        Ok(parsed) => parsed,
        Err(message) => {
            return write_json(
                w,
                400,
                "Bad Request",
                &error_envelope("invalid_request", &message),
            );
        }
    };

    let (status, reason, body) = state.with_workspace_mut(workspace_id, |ws| {
        dispatch_app_command(ws, workspace_id, app_id, &parsed)
    })?;
    write_json(w, status, reason, &body)
}

struct AppCommandRequest {
    command: String,
    payload: Value,
    session_id: Option<String>,
}

fn parse_app_command_request(body: &[u8]) -> Result<AppCommandRequest, String> {
    let value: Value =
        serde_json::from_slice(body).map_err(|e| format!("request body is not valid JSON: {e}"))?;
    let Some(object) = value.as_object() else {
        return Err("request body must be a JSON object".to_string());
    };
    let command = match object.get("command") {
        Some(Value::String(command)) if !command.trim().is_empty() => command.clone(),
        _ => return Err("'command' must be a non-empty string".to_string()),
    };
    let session_id = match object.get("session_id") {
        None | Some(Value::Null) => None,
        Some(Value::String(session_id)) if !session_id.trim().is_empty() => {
            Some(session_id.clone())
        }
        _ => return Err("'session_id' must be a non-empty string when present".to_string()),
    };
    let payload = object.get("payload").cloned().unwrap_or_else(|| json!({}));
    Ok(AppCommandRequest {
        command,
        payload,
        session_id,
    })
}

fn dispatch_app_command<E: LocalExecutor + Clone>(
    ws: &mut WorkspaceState<E>,
    workspace_id: &str,
    app_id: &str,
    request: &AppCommandRequest,
) -> Result<(u16, &'static str, Value), String> {
    let Some(machine) = ws.app_state_machines.get(app_id).cloned() else {
        return Ok((
            404,
            "Not Found",
            error_envelope(
                "app_not_registered",
                &format!(
                    "app '{app_id}' is not registered with a state machine \
                     in workspace '{workspace_id}'"
                ),
            ),
        ));
    };

    let (session_id, current_state) = match &request.session_id {
        Some(session_id) => {
            let last_state = ws
                .app_events
                .iter()
                .rev()
                .find(|event| event.app_id == app_id && &event.session_id == session_id)
                .map(|event| event.state.clone());
            (
                session_id.clone(),
                last_state.unwrap_or_else(|| machine.initial_state.clone()),
            )
        }
        None => (
            format!("sess-{:08}", ws.app_events.len() + 1),
            machine.initial_state.clone(),
        ),
    };

    let accepted_state =
        match resolve_app_command_transition(&machine, &session_id, &current_state, request) {
            Ok(accepted_state) => accepted_state,
            Err(rejection) => return Ok(rejection),
        };
    let command_id = format!("cmd-{:08}", ws.app_events.len() + 1);
    let timestamp = generated_registered_at().map_err(|e| e.message)?;
    push_app_state_changed_event(
        ws,
        workspace_id,
        app_id,
        &session_id,
        &command_id,
        &accepted_state,
        &current_state,
        &timestamp,
    );

    let execution_id = run_app_command_invoke(
        ws,
        workspace_id,
        app_id,
        &session_id,
        &machine,
        &accepted_state,
        &command_id,
        &request.payload,
        &timestamp,
    );

    let body = json!({
        "api_version": "v1",
        "status": "accepted",
        "workspace_id": workspace_id,
        "app_id": app_id,
        "session_id": session_id,
        "command": request.command,
        "state": accepted_state,
        "execution_id": execution_id,
        "links": {
            "events": format!("/v1/workspaces/{workspace_id}/apps/{app_id}/events"),
            "sessions": format!("/v1/workspaces/{workspace_id}/apps/{app_id}/sessions"),
        },
    });
    Ok((202, "Accepted", body))
}

fn resolve_app_command_transition(
    machine: &ApplicationStateMachine,
    session_id: &str,
    current_state: &str,
    request: &AppCommandRequest,
) -> Result<String, (u16, &'static str, Value)> {
    let Some(state_definition) = machine.states.iter().find(|s| s.id == current_state) else {
        return Err((
            409,
            "Conflict",
            error_envelope(
                "invalid_transition",
                &format!(
                    "session '{session_id}' is in state '{current_state}', \
                     which is not declared by the app state machine"
                ),
            ),
        ));
    };

    let Some(transition) = state_definition
        .transitions
        .iter()
        .find(|transition| transition.on == request.command)
    else {
        let known_command = machine.states.iter().any(|state| {
            state
                .transitions
                .iter()
                .any(|transition| transition.on == request.command)
        });
        if known_command {
            return Err((
                409,
                "Conflict",
                error_envelope(
                    "invalid_transition",
                    &format!(
                        "command '{}' is not a valid transition from state '{current_state}'",
                        request.command
                    ),
                ),
            ));
        }
        return Err((
            422,
            "Unprocessable Entity",
            error_envelope(
                "unknown_command",
                &format!(
                    "command '{}' is not declared by the app state machine",
                    request.command
                ),
            ),
        ));
    };

    Ok(transition.to.clone())
}

#[allow(clippy::too_many_arguments)]
fn run_app_command_invoke<E: LocalExecutor + Clone>(
    ws: &mut WorkspaceState<E>,
    workspace_id: &str,
    app_id: &str,
    session_id: &str,
    machine: &ApplicationStateMachine,
    accepted_state: &str,
    command_id: &str,
    payload: &Value,
    timestamp: &str,
) -> Option<String> {
    let invoke = machine
        .states
        .iter()
        .find(|state| state.id == accepted_state)
        .and_then(|state| state.invoke.clone())?;
    let runtime_request = RuntimeRequest {
        kind: "runtime_request".to_string(),
        schema_version: "1.0.0".to_string(),
        request_id: command_id.to_string(),
        intent: RuntimeIntent {
            capability_id: Some(invoke.capability_id.clone()),
            capability_version: None,
            version_range: None,
            intent_key: None,
        },
        input: payload.clone(),
        lookup: RuntimeLookup {
            scope: RuntimeLookupScope::PreferPrivate,
            allow_ambiguity: false,
        },
        context: RuntimeContext {
            requested_target: PlacementTarget::Local,
            correlation_id: Some(session_id.to_string()),
            caller: None,
            traceparent: None,
            tracestate: None,
            metadata: None,
            identity: None,
        },
        governing_spec: "006-runtime-request-execution".to_string(),
    };
    let outcome = ws.runtime.execute(runtime_request);
    let execution_id = outcome.result.execution_id.clone();
    let succeeded = outcome.result.status != RuntimeResultStatus::Error;
    ws.executions.insert(
        execution_id.clone(),
        ExecutionStatusRecord {
            execution_id: execution_id.clone(),
            status: if succeeded { "succeeded" } else { "failed" }.to_string(),
            created_at: timestamp.to_string(),
            updated_at: timestamp.to_string(),
        },
    );
    ws.traces
        .insert(execution_id.clone(), outcome.trace.clone());
    push_app_command_outcome_events(
        ws,
        workspace_id,
        app_id,
        session_id,
        machine,
        accepted_state,
        &invoke.capability_id,
        &outcome,
        timestamp,
    );
    Some(execution_id)
}

#[allow(clippy::too_many_arguments)]
fn push_app_state_changed_event<E>(
    ws: &mut WorkspaceState<E>,
    workspace_id: &str,
    app_id: &str,
    session_id: &str,
    execution_id: &str,
    state: &str,
    previous_state: &str,
    timestamp: &str,
) {
    ws.app_events.push(AppStateEventRecord {
        event_id: format!("{execution_id}:state_changed:{state}"),
        event_type: "state_changed".to_string(),
        workspace_id: workspace_id.to_string(),
        app_id: app_id.to_string(),
        session_id: session_id.to_string(),
        execution_id: execution_id.to_string(),
        state: state.to_string(),
        previous_state: Some(previous_state.to_string()),
        timestamp: timestamp.to_string(),
        data: json!({
            "workspace_id": workspace_id,
            "app_id": app_id,
            "session_id": session_id,
            "execution_id": execution_id,
            "state": state,
            "previous_state": previous_state,
            "timestamp": timestamp,
        }),
    });
}

#[allow(clippy::too_many_arguments)]
fn push_app_command_outcome_events<E>(
    ws: &mut WorkspaceState<E>,
    workspace_id: &str,
    app_id: &str,
    session_id: &str,
    machine: &ApplicationStateMachine,
    invoking_state: &str,
    capability_id: &str,
    outcome: &RuntimeExecutionOutcome,
    timestamp: &str,
) {
    let execution_id = outcome.result.execution_id.clone();
    ws.app_events.push(AppStateEventRecord {
        event_id: format!("{execution_id}:capability_invoked"),
        event_type: "capability_invoked".to_string(),
        workspace_id: workspace_id.to_string(),
        app_id: app_id.to_string(),
        session_id: session_id.to_string(),
        execution_id: execution_id.clone(),
        state: invoking_state.to_string(),
        previous_state: None,
        timestamp: timestamp.to_string(),
        data: json!({
            "workspace_id": workspace_id,
            "app_id": app_id,
            "session_id": session_id,
            "execution_id": execution_id,
            "capability_id": capability_id,
            "state": invoking_state,
            "timestamp": timestamp,
        }),
    });

    let succeeded = outcome.result.status != RuntimeResultStatus::Error;
    let lifecycle_event = if succeeded {
        "capability_succeeded"
    } else {
        "capability_failed"
    };
    let empty_output = Value::Null;
    let transition_result = resolve_lifecycle_transition(
        machine,
        invoking_state,
        lifecycle_event,
        outcome.result.output.as_ref().unwrap_or(&empty_output),
    );
    let (final_state, transition_error) = match transition_result {
        Ok(Some(transition)) => (transition.to.clone(), None),
        Ok(None) if succeeded => (
            invoking_state.to_string(),
            Some((
                "no_matching_transition",
                "no capability_succeeded transition matched the capability output",
            )),
        ),
        Ok(None) => (invoking_state.to_string(), None),
        Err(ConditionEvaluationError::Type) => (
            invoking_state.to_string(),
            Some((
                "condition_type_error",
                "a transition condition could not be evaluated against the capability output",
            )),
        ),
    };
    if final_state != invoking_state {
        push_app_state_changed_event(
            ws,
            workspace_id,
            app_id,
            session_id,
            &execution_id,
            &final_state,
            invoking_state,
            timestamp,
        );
    }

    let result_event_type = if transition_error.is_some() || !succeeded {
        "error"
    } else {
        "capability_result"
    };
    ws.app_events.push(AppStateEventRecord {
        event_id: format!("{execution_id}:{result_event_type}"),
        event_type: result_event_type.to_string(),
        workspace_id: workspace_id.to_string(),
        app_id: app_id.to_string(),
        session_id: session_id.to_string(),
        execution_id: execution_id.clone(),
        state: final_state.clone(),
        previous_state: Some(invoking_state.to_string()),
        timestamp: timestamp.to_string(),
        data: json!({
            "workspace_id": workspace_id,
            "app_id": app_id,
            "session_id": session_id,
            "execution_id": execution_id,
            "state": final_state,
            "previous_state": invoking_state,
            "timestamp": timestamp,
            "output": outcome.result.output,
            "error_code": transition_error.map(|(code, _)| code),
            "error": outcome.result.error.as_ref().map(|e| json!({
                "code": format!("{:?}", e.code).to_lowercase(),
                "message": e.message,
            })).or_else(|| transition_error.map(|(code, message)| json!({
                "code": code,
                "message": message,
            }))),
        }),
    });
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConditionEvaluationError {
    Type,
}

fn resolve_lifecycle_transition<'a>(
    machine: &'a ApplicationStateMachine,
    invoking_state: &str,
    lifecycle_event: &str,
    output: &Value,
) -> Result<Option<&'a ApplicationStateTransition>, ConditionEvaluationError> {
    let Some(state) = machine
        .states
        .iter()
        .find(|state| state.id == invoking_state)
    else {
        return Ok(None);
    };
    let matching = state
        .transitions
        .iter()
        .filter(|transition| transition.on == lifecycle_event);

    if lifecycle_event != "capability_succeeded" {
        return Ok(matching.into_iter().next());
    }

    let transitions = matching.collect::<Vec<_>>();
    for transition in transitions.iter().copied() {
        if let Some(condition) = transition.condition.as_ref()
            && evaluate_transition_condition(condition, output)?
        {
            return Ok(Some(transition));
        }
    }
    Ok(transitions
        .into_iter()
        .find(|transition| transition.condition.is_none()))
}

fn evaluate_transition_condition(
    condition: &ApplicationStateTransitionCondition,
    output: &Value,
) -> Result<bool, ConditionEvaluationError> {
    let value = condition.field.strip_prefix("output.").and_then(|path| {
        path.split('.')
            .try_fold(output, |current, segment| current.get(segment))
    });
    match condition.op {
        ApplicationStateTransitionConditionOp::Exists => {
            Ok(value.is_some_and(|value| !value.is_null()))
        }
        ApplicationStateTransitionConditionOp::In => {
            let Some(actual) = value else {
                return Ok(false);
            };
            let Some(expected) = condition.value.as_ref().and_then(Value::as_array) else {
                return Err(ConditionEvaluationError::Type);
            };
            Ok(expected.iter().any(|candidate| candidate == actual))
        }
        ApplicationStateTransitionConditionOp::Eq | ApplicationStateTransitionConditionOp::Neq => {
            let Some(actual) = value else {
                return Ok(false);
            };
            let Some(expected) = condition.value.as_ref() else {
                return Err(ConditionEvaluationError::Type);
            };
            if json_value_kind(actual) != json_value_kind(expected) {
                return Err(ConditionEvaluationError::Type);
            }
            let equal = actual == expected;
            Ok(
                if condition.op == ApplicationStateTransitionConditionOp::Eq {
                    equal
                } else {
                    !equal
                },
            )
        }
        ApplicationStateTransitionConditionOp::Gt
        | ApplicationStateTransitionConditionOp::Gte
        | ApplicationStateTransitionConditionOp::Lt
        | ApplicationStateTransitionConditionOp::Lte => {
            let Some(actual) = value.and_then(Value::as_f64) else {
                return Err(ConditionEvaluationError::Type);
            };
            let Some(expected) = condition.value.as_ref().and_then(Value::as_f64) else {
                return Err(ConditionEvaluationError::Type);
            };
            Ok(match condition.op {
                ApplicationStateTransitionConditionOp::Gt => actual > expected,
                ApplicationStateTransitionConditionOp::Gte => actual >= expected,
                ApplicationStateTransitionConditionOp::Lt => actual < expected,
                ApplicationStateTransitionConditionOp::Lte => actual <= expected,
                _ => false,
            })
        }
    }
}

fn json_value_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionsOrder {
    CreatedAsc,
    CreatedDesc,
}

fn parse_sessions_order(value: Option<&str>) -> Result<SessionsOrder, String> {
    match value.unwrap_or("created_desc") {
        "created_asc" => Ok(SessionsOrder::CreatedAsc),
        "created_desc" => Ok(SessionsOrder::CreatedDesc),
        other => Err(format!(
            "unsupported sessions order '{other}' (expected created_asc or created_desc)"
        )),
    }
}

fn parse_sessions_limit(value: Option<&str>) -> Result<usize, String> {
    let Some(value) = value else {
        return Ok(50);
    };
    let limit = value
        .parse::<usize>()
        .map_err(|_| "limit must be a positive integer".to_string())?;
    if limit == 0 || limit > 200 {
        return Err("limit must be between 1 and 200".to_string());
    }
    Ok(limit)
}

fn app_sessions_response(
    events: &[AppStateEventRecord],
    list_context_fields: &[String],
    app_id: &str,
    state_filter: Option<&str>,
    limit: usize,
    cursor: Option<&str>,
    order: SessionsOrder,
) -> Value {
    let mut sessions = materialize_app_sessions(events, list_context_fields, app_id);
    if let Some(state_filter) = state_filter {
        sessions.retain(|session| session["current_state"].as_str() == Some(state_filter));
    }
    sessions.sort_by(|a, b| {
        let left = a["created_at"].as_str().unwrap_or_default();
        let right = b["created_at"].as_str().unwrap_or_default();
        match order {
            SessionsOrder::CreatedAsc => left.cmp(right),
            SessionsOrder::CreatedDesc => right.cmp(left),
        }
    });
    let total = sessions.len();
    if let Some(cursor) = cursor
        && let Some(index) = sessions
            .iter()
            .position(|session| session["session_id"].as_str() == Some(cursor))
    {
        sessions = sessions.into_iter().skip(index + 1).collect();
    }
    let next_cursor = sessions
        .get(limit)
        .and_then(|session| session["session_id"].as_str())
        .map(ToString::to_string);
    sessions.truncate(limit);
    json!({
        "api_version": "v1",
        "app_id": app_id,
        "sessions": sessions,
        "total": total,
        "next_cursor": next_cursor,
    })
}

fn materialize_app_sessions(
    events: &[AppStateEventRecord],
    list_context_fields: &[String],
    app_id: &str,
) -> Vec<Value> {
    let mut grouped: HashMap<String, Vec<&AppStateEventRecord>> = HashMap::new();
    for event in events {
        if event.app_id == app_id {
            grouped
                .entry(event.session_id.clone())
                .or_default()
                .push(event);
        }
    }

    grouped
        .into_iter()
        .filter_map(|(session_id, mut events)| {
            events.sort_by(|a, b| {
                a.timestamp
                    .cmp(&b.timestamp)
                    .then(a.event_id.cmp(&b.event_id))
            });
            let first = events.first()?;
            let last = events.last()?;
            let context = events
                .iter()
                .rev()
                .find_map(|event| event.data.get("output"))
                .map_or_else(
                    || Value::Object(serde_json::Map::new()),
                    |output| project_list_context(output, list_context_fields),
                );
            Some(json!({
                "session_id": session_id,
                "current_state": last.state,
                "created_at": first.timestamp,
                "updated_at": last.timestamp,
                "context": context,
            }))
        })
        .collect()
}

fn project_list_context(output: &Value, list_context_fields: &[String]) -> Value {
    let mut context = serde_json::Map::new();
    for field in list_context_fields {
        let Some(path) = field.strip_prefix("output.") else {
            continue;
        };
        let Some(value) = get_json_path(output, path) else {
            continue;
        };
        context.insert(path.replace('.', "_"), value.clone());
    }
    Value::Object(context)
}

fn get_json_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = value;
    for segment in path.split('.') {
        current = current.get(segment)?;
    }
    Some(current)
}

fn replay_app_events(
    events: &[AppStateEventRecord],
    app_id: &str,
    last_event_id: Option<&str>,
) -> Vec<AppStateEventRecord> {
    let mut after_last_event = last_event_id.is_none();
    events
        .iter()
        .filter_map(|event| {
            if event.app_id != app_id {
                return None;
            }
            if !after_last_event {
                after_last_event = last_event_id == Some(event.event_id.as_str());
                return None;
            }
            Some(event.clone())
        })
        .collect()
}

fn serialize_sse_event(
    event_id: Option<&str>,
    event_type: &str,
    data: &Value,
) -> Result<String, String> {
    let mut rendered = String::new();
    if let Some(event_id) = event_id {
        rendered.push_str("id: ");
        rendered.push_str(event_id);
        rendered.push('\n');
    }
    rendered.push_str("event: ");
    rendered.push_str(event_type);
    rendered.push('\n');
    let data = serde_json::to_string(data)
        .map_err(|e| format!("failed to serialize SSE event data: {e}"))?;
    rendered.push_str("data: ");
    rendered.push_str(&data);
    rendered.push_str("\n\n");
    Ok(rendered)
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

fn record_app_execution_events<E: LocalExecutor + Clone>(
    state: &ApiState<E>,
    workspace_id: &str,
    outcome: &RuntimeExecutionOutcome,
) -> Result<(), String> {
    let execution_id = outcome.result.execution_id.clone();
    let app_id = app_id_for_outcome(outcome);
    let session_id = outcome
        .trace
        .request
        .context
        .correlation_id
        .clone()
        .unwrap_or_else(|| execution_id.clone());
    let timestamp = generated_registered_at().map_err(|e| e.message)?;
    let final_state = if outcome.result.status == RuntimeResultStatus::Error {
        "error"
    } else {
        "results"
    };
    let result_event_type = if outcome.result.status == RuntimeResultStatus::Error {
        "error"
    } else {
        "capability_result"
    };

    let events = vec![
        AppStateEventRecord {
            event_id: format!("{execution_id}:state_changed:processing"),
            event_type: "state_changed".to_string(),
            workspace_id: workspace_id.to_string(),
            app_id: app_id.clone(),
            session_id: session_id.clone(),
            execution_id: execution_id.clone(),
            state: "processing".to_string(),
            previous_state: Some("idle".to_string()),
            timestamp: timestamp.clone(),
            data: json!({
                "workspace_id": workspace_id,
                "app_id": app_id,
                "session_id": session_id,
                "execution_id": execution_id,
                "state": "processing",
                "previous_state": "idle",
                "timestamp": timestamp,
            }),
        },
        AppStateEventRecord {
            event_id: format!("{execution_id}:capability_invoked"),
            event_type: "capability_invoked".to_string(),
            workspace_id: workspace_id.to_string(),
            app_id: app_id.clone(),
            session_id: session_id.clone(),
            execution_id: execution_id.clone(),
            state: "processing".to_string(),
            previous_state: None,
            timestamp: timestamp.clone(),
            data: json!({
                "workspace_id": workspace_id,
                "app_id": app_id,
                "session_id": session_id,
                "execution_id": execution_id,
                "capability_id": outcome.trace.request.intent.capability_id,
                "capability_version": outcome.trace.request.intent.capability_version,
                "state": "processing",
                "timestamp": timestamp,
            }),
        },
        AppStateEventRecord {
            event_id: format!("{execution_id}:{result_event_type}"),
            event_type: result_event_type.to_string(),
            workspace_id: workspace_id.to_string(),
            app_id: app_id.clone(),
            session_id: session_id.clone(),
            execution_id: execution_id.clone(),
            state: final_state.to_string(),
            previous_state: Some("processing".to_string()),
            timestamp: timestamp.clone(),
            data: json!({
                "workspace_id": workspace_id,
                "app_id": app_id,
                "session_id": session_id,
                "execution_id": execution_id,
                "state": final_state,
                "previous_state": "processing",
                "timestamp": timestamp,
                "output": outcome.result.output,
                "error": outcome.result.error.as_ref().map(|e| json!({
                    "code": format!("{:?}", e.code).to_lowercase(),
                    "message": e.message,
                })),
            }),
        },
    ];

    state.with_workspace_mut(workspace_id, |ws| {
        ws.app_events.extend(events);
        Ok(())
    })
}

fn app_id_for_outcome(outcome: &RuntimeExecutionOutcome) -> String {
    outcome
        .trace
        .request
        .intent
        .capability_id
        .as_deref()
        .and_then(|id| id.rsplit_once('.').map(|(prefix, _)| prefix.to_string()))
        .unwrap_or_else(|| "default".to_string())
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
    let Some(record) = state
        .idempotency_records
        .lock()
        .map_err(|_| "idempotency record lock poisoned".to_string())?
        .get(&cache_key)
        .cloned()
    else {
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
    state
        .idempotency_records
        .lock()
        .map_err(|_| "idempotency record lock poisoned".to_string())?
        .insert(
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
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
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
    let identity = match subject_from_state(&request.headers, state, loopback) {
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

    let _ = match ensure_workspace_authorized(
        &state.registry_root,
        workspace_id,
        &identity,
        SCOPE_RUNTIME_TRACE_READ,
        scopes_optional_for_request(state.allow_unauthenticated, loopback, &identity),
    ) {
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
    let identity = match subject_from_state(&request.headers, state, loopback) {
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

    let _ = match ensure_workspace_authorized(
        &state.registry_root,
        workspace_id,
        &identity,
        SCOPE_RUNTIME_TRACE_READ,
        scopes_optional_for_request(state.allow_unauthenticated, loopback, &identity),
    ) {
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
        .otel_trace
        .spans
        .iter()
        .map(|span| {
            json!({
                "trace_id": span.trace_id,
                "span_id": span.span_id,
                "parent_span_id": span.parent_span_id,
                "name": span.name,
                "kind": span.kind,
                "status": span.status,
                "started_at": span.started_at,
                "ended_at": span.ended_at,
                "attributes": span.attributes,
                "events": span.events,
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
        "otel_trace_id": trace.otel_trace.trace_id,
        "traceparent": trace.otel_trace.parent_traceparent,
        "tracestate": trace.otel_trace.tracestate,
        "otel_exporter": trace.otel_trace.exporter,
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
    let identity = match subject_from_state(&request.headers, state, loopback) {
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

    let _ = match ensure_workspace_authorized(
        &state.registry_root,
        &workspace_id,
        &identity,
        SCOPE_REGISTRY_WRITE,
        scopes_optional_for_request(state.allow_unauthenticated, loopback, &identity),
    ) {
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

    let identity = match subject_from_state(&request.headers, state, loopback) {
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

    let _ = match ensure_workspace_authorized(
        &state.registry_root,
        &workspace_id,
        &identity,
        SCOPE_REGISTRY_READ,
        scopes_optional_for_request(state.allow_unauthenticated, loopback, &identity),
    ) {
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

    let identity = match subject_from_state(&request.headers, state, loopback) {
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

    let _ = match ensure_workspace_authorized(
        &state.registry_root,
        &workspace_id,
        &identity,
        SCOPE_REGISTRY_READ,
        scopes_optional_for_request(state.allow_unauthenticated, loopback, &identity),
    ) {
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

/// Reads and parses one HTTP request from `stream`, enforcing a whole-request
/// `deadline` across both the header and body read phases (spec
/// 033-http-json-api connection-handling model). A per-call socket
/// `read_timeout`/`write_timeout` is set by the caller before this runs and
/// bounds any single blocking read; `deadline` additionally bounds the total
/// time spent here so a slow trickle of bytes cannot extend a request
/// indefinitely by staying just under the per-read timeout.
fn read_http_request(stream: &mut TcpStream, deadline: Instant) -> Result<HttpRequest, String> {
    let mut buffer = Vec::new();
    let mut header_end = None;

    loop {
        if Instant::now() >= deadline {
            return Err("HTTP request timed out reading headers".to_string());
        }
        let mut chunk = [0_u8; 1024];
        let n = stream.read(&mut chunk).map_err(|e| {
            if is_timeout_error(&e) {
                "HTTP request timed out reading headers".to_string()
            } else {
                format!("failed to read HTTP request: {e}")
            }
        })?;
        if n == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..n]);
        // Enforce the size cap before checking for the terminator: a header
        // block that happens to complete in the same read that pushes it
        // over the cap must still be rejected, not admitted because the
        // terminator arrived in the same chunk.
        if buffer.len() > MAX_REQUEST_HEADER_BYTES {
            return Err("HTTP request headers too large".to_string());
        }
        if let Some(idx) = find_header_end(&buffer) {
            header_end = Some(idx);
            break;
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
            if headers.len() > MAX_REQUEST_HEADER_COUNT {
                return Err(format!(
                    "HTTP request has too many headers (max {MAX_REQUEST_HEADER_COUNT})"
                ));
            }
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
        if Instant::now() >= deadline {
            return Err("HTTP request timed out reading body".to_string());
        }
        let mut chunk = vec![0_u8; content_length - body.len()];
        let n = stream.read(&mut chunk).map_err(|e| {
            if is_timeout_error(&e) {
                "HTTP request timed out reading body".to_string()
            } else {
                format!("failed to read HTTP request body: {e}")
            }
        })?;
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

fn is_timeout_error(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    )
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
    use std::sync::atomic::{AtomicU64, Ordering};
    use traverse_contracts::{
        BinaryFormat as ContractBinaryFormat, CapabilityContract, CapabilityReference, Entrypoint,
        EntrypointKind, EventClassification, EventPayload, EventProvenance, EventProvenanceSource,
        EventType, Execution, ExecutionConstraints, ExecutionTarget, FilesystemAccess,
        HostApiAccess, IdReference, Lifecycle, NetworkAccess, Owner, PayloadCompatibility,
        Provenance, ProvenanceSource, SchemaContainer, ServiceType, SideEffect, SideEffectKind,
    };
    use traverse_registry::ResolvedCapability;
    use traverse_registry::{
        ApplicationState, ApplicationStateInvoke, ApplicationStateTransition, ArtifactDigests,
        BinaryFormat, BinaryReference, CapabilityArtifactRecord, CapabilityRegistration,
        ComposabilityMetadata, CompositionKind, CompositionPattern, ImplementationKind,
        RegistryProvenance, RegistryScope, SourceKind, SourceReference, WorkflowEdge, WorkflowNode,
        WorkflowNodeInput, WorkflowNodeOutput,
    };
    use traverse_runtime::{LocalExecutionFailure, LocalExecutionFailureCode};

    #[test]
    fn serve_error_display_keeps_bind_and_accept_context() {
        assert_eq!(
            ServeError::BindFailed("address in use".to_string()).to_string(),
            "failed to bind HTTP/JSON API server: address in use"
        );
        assert_eq!(
            ServeError::AcceptFailed("socket closed".to_string()).to_string(),
            "HTTP/JSON API server accept loop failed: socket closed"
        );
    }

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
        static NEXT_TEST_ROOT: AtomicU64 = AtomicU64::new(1);
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time must be valid")
            .as_nanos();
        let sequence = NEXT_TEST_ROOT.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("traverse-cli-http-api-tests-{suffix}-{sequence}"))
    }

    fn persist_test_workspace(registry_root: &Path, workspace_id: &str, owner_subject: &str) {
        let metadata = WorkspaceMetadataV1 {
            schema_version: WORKSPACE_METADATA_SCHEMA_VERSION.to_string(),
            workspace_id: workspace_id.to_string(),
            owner_subject: owner_subject.to_string(),
            shared: false,
            members: Vec::new(),
        };
        persist_workspace_metadata(registry_root, workspace_id, &metadata)
            .expect("test workspace metadata must persist");
    }

    fn persist_local_test_workspace(registry_root: &Path, workspace_id: &str) {
        persist_test_workspace(registry_root, workspace_id, "local");
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
            connector_requirements: Vec::new(),
            state_schema: None,
        }
    }

    fn test_registration(id: &str, version: &str) -> CapabilityRegistration {
        let contract = test_contract(id, version);
        test_registration_from_contract(contract)
    }

    fn test_registration_from_contract(contract: CapabilityContract) -> CapabilityRegistration {
        let id = contract.id.clone();
        let version = contract.version.clone();
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
                    signature: None,
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
                provides: vec![id.clone()],
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
                    publish_to_state_as: None,
                },
            }],
            edges: Vec::<WorkflowEdge>::new(),
            start_node: "run_capability".to_string(),
            terminal_nodes: vec!["run_capability".to_string()],
            output_projection: Vec::new(),
            tags: vec!["test".to_string()],
            governing_spec: "007-workflow-registry-traversal".to_string(),
        }
    }

    fn pipeline_workflow_definition_body(steps: &[(&str, &str, &str)]) -> Vec<u8> {
        let nodes = steps
            .iter()
            .map(|(node_id, capability_id, namespace)| {
                json!({
                    "node_id": node_id,
                    "capability_id": capability_id,
                    "capability_version": "1.0.0",
                    "input": {"from_workflow_input": []},
                    "output": {
                        "to_workflow_state": [],
                        "publish_to_state_as": namespace
                    }
                })
            })
            .collect::<Vec<_>>();
        let edges = steps
            .windows(2)
            .map(|pair| {
                json!({
                    "edge_id": format!("{}_to_{}", pair[0].0, pair[1].0),
                    "from": pair[0].0,
                    "to": pair[1].0,
                    "trigger": "direct",
                    "event": null
                })
            })
            .collect::<Vec<_>>();
        let projection = steps
            .iter()
            .map(|(_, _, namespace)| (*namespace).to_string())
            .collect::<Vec<_>>();
        json!({
            "scope": "workspace_persisted",
            "registry_scope": "private",
            "workflow": {
                "kind": "workflow_definition",
                "schema_version": "1.0.0",
                "id": "test.pipeline.run",
                "name": "run",
                "version": "1.0.0",
                "lifecycle": "active",
                "owner": {"team": "test-team", "contact": "test@example.com"},
                "summary": "test pipeline workflow",
                "inputs": {"schema": {"type": "object"}},
                "outputs": {"schema": {
                    "type": "object",
                    "required": projection,
                    "additionalProperties": false
                }},
                "nodes": nodes,
                "edges": edges,
                "start_node": steps[0].0,
                "terminal_nodes": [steps[steps.len() - 1].0],
                "output_projection": projection,
                "tags": ["test"],
                "governing_spec": "007-workflow-registry-traversal"
            }
        })
        .to_string()
        .into_bytes()
    }

    #[test]
    fn execute_endpoint_runs_pipeline_workflow_capability_with_merged_output() {
        let state = empty_state();
        for capability_id in [
            "test.pipeline.validate",
            "test.pipeline.process",
            "test.pipeline.summarize",
        ] {
            state
                .with_workspace_mut("ws-test", |ws| {
                    ws.runtime
                        .register_capability(test_registration(capability_id, "1.0.0"))
                        .map_err(|failure| format!("{failure:?}"))?;
                    Ok(())
                })
                .expect("step capability must register");
        }

        let workflow_req = make_http_request(
            "POST",
            "/v1/workspaces/ws-test/workflows",
            pipeline_workflow_definition_body(&[
                ("validate_step", "test.pipeline.validate", "validate"),
                ("process_step", "test.pipeline.process", "process"),
                ("summarize_step", "test.pipeline.summarize", "summarize"),
            ]),
        );
        let mut workflow_out = Vec::new();
        handle_workspace_operation(&mut workflow_out, &workflow_req, &state, true)
            .expect("workflow registration must write a response");
        assert_eq!(response_status(&workflow_out), 201);

        let mut pipeline_registration = test_registration("test.pipeline.pipeline", "1.0.0");
        pipeline_registration.artifact.implementation_kind = ImplementationKind::Workflow;
        pipeline_registration.artifact.binary = None;
        pipeline_registration.artifact.digests.binary_digest = None;
        pipeline_registration.artifact.workflow_ref = Some(traverse_registry::WorkflowReference {
            workflow_id: "test.pipeline.run".to_string(),
            workflow_version: "1.0.0".to_string(),
        });
        pipeline_registration.composability = ComposabilityMetadata {
            kind: CompositionKind::Composite,
            patterns: vec![CompositionPattern::Sequential],
            provides: vec!["test.pipeline.pipeline".to_string()],
            requires: Vec::new(),
        };
        state
            .with_workspace_mut("ws-test", |ws| {
                ws.runtime
                    .register_capability(pipeline_registration)
                    .map_err(|failure| format!("{failure:?}"))?;
                Ok(())
            })
            .expect("pipeline capability must register");

        let execute_req = make_http_request(
            "POST",
            "/v1/workspaces/ws-test/execute",
            make_runtime_request_body("test.pipeline.pipeline"),
        );
        let mut execute_out = Vec::new();
        handle_workspace_operation(&mut execute_out, &execute_req, &state, true)
            .expect("workspace execute must write a response");

        assert_eq!(response_status(&execute_out), 200);
        let executed = parse_response_body(&execute_out);
        assert_eq!(executed["status"], "succeeded");
        assert_eq!(
            executed["output"],
            json!({
                "validate": {},
                "process": {},
                "summarize": {}
            })
        );
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

    fn valid_bundle_registration_body(
        capability_id: &str,
        event_id: &str,
        workflow_id: &str,
        artifact_path: &Path,
    ) -> Vec<u8> {
        let mut contract = test_contract(capability_id, "1.0.0");
        contract.execution.entrypoint.command = artifact_path.to_string_lossy().to_string();
        json!({
            "scope": "workspace_persisted",
            "bundle": {
                "event_contracts": [{
                    "registry_scope": "private",
                    "event_contract": test_event_contract(event_id, "1.0.0")
                }],
                "capabilities": [{
                    "registry_scope": "private",
                    "contract": contract
                }],
                "workflows": [{
                    "registry_scope": "private",
                    "workflow": test_workflow_definition(workflow_id, "1.0.0", capability_id)
                }]
            }
        })
        .to_string()
        .into_bytes()
    }

    fn runtime_grant_body(
        capability_id: &str,
        grant_scope: &str,
        resource: &str,
        lifetime: &str,
        expires_in_seconds: u64,
    ) -> Vec<u8> {
        json!({
            "capability_id": capability_id,
            "grant_scope": grant_scope,
            "resource": resource,
            "lifetime": lifetime,
            "expires_in_seconds": expires_in_seconds
        })
        .to_string()
        .into_bytes()
    }

    fn audit_log_text<E: LocalExecutor + Clone>(state: &ApiState<E>, workspace_id: &str) -> String {
        std::fs::read_to_string(workspace_audit_log_path(&state.registry_root, workspace_id))
            .expect("audit log must be readable")
    }

    fn audit_log_entries<E: LocalExecutor + Clone>(
        state: &ApiState<E>,
        workspace_id: &str,
    ) -> Vec<Value> {
        audit_log_text(state, workspace_id)
            .lines()
            .map(|line| serde_json::from_str(line).expect("audit entry must be json"))
            .collect()
    }

    fn test_state_with(id: &str, version: &str) -> ApiState<TestExecutor> {
        test_state_with_output(id, version, json!({"result": "ok"}))
    }

    fn test_state_with_output(id: &str, version: &str, output: Value) -> ApiState<TestExecutor> {
        let mut registry = CapabilityRegistry::new();
        registry
            .register(test_registration(id, version))
            .expect("test registration must succeed");

        let executor = TestExecutor::ok(output);
        let registry_root = test_registry_root();
        std::fs::create_dir_all(&registry_root).expect("registry root must be created");

        let mut workspaces = HashMap::new();
        let workspace_id = "ws-test";
        persist_local_test_workspace(&registry_root, workspace_id);
        workspaces.insert(
            workspace_id.to_string(),
            WorkspaceState {
                runtime: Runtime::new(registry, executor.clone())
                    .with_workflow_registry(WorkflowRegistry::new())
                    .with_security_config(RuntimeSecurityConfig::development()),
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
                app_events: Vec::new(),
                app_list_context_fields: HashMap::new(),
                app_state_machines: HashMap::new(),
                runtime_grants: Vec::new(),
            },
        );

        ApiState {
            auth_mode: "dev-loopback".to_string(),
            allow_unauthenticated: true,
            allowed_origins: Vec::new(),
            registry_root,
            executor,
            workspaces: Mutex::new(workspaces),
            idempotency_records: Mutex::new(HashMap::new()),
            idempotency_retention_seconds: DEFAULT_IDEMPOTENCY_RETENTION_SECONDS,
            jwt_verification_key: None,
        }
    }

    fn test_state_with_permission(
        id: &str,
        version: &str,
        permission_id: &str,
    ) -> ApiState<TestExecutor> {
        let mut contract = test_contract(id, version);
        contract.permissions = vec![IdReference {
            id: permission_id.to_string(),
        }];
        let mut registry = CapabilityRegistry::new();
        registry
            .register(test_registration_from_contract(contract))
            .expect("test registration must succeed");

        let executor = TestExecutor::ok(json!({"result": "ok"}));
        let registry_root = test_registry_root();
        std::fs::create_dir_all(&registry_root).expect("registry root must be created");

        let mut workspaces = HashMap::new();
        let workspace_id = "ws-test";
        persist_local_test_workspace(&registry_root, workspace_id);
        workspaces.insert(
            workspace_id.to_string(),
            WorkspaceState {
                runtime: Runtime::new(registry, executor.clone())
                    .with_workflow_registry(WorkflowRegistry::new())
                    .with_security_config(RuntimeSecurityConfig::development()),
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
                app_events: Vec::new(),
                app_list_context_fields: HashMap::new(),
                app_state_machines: HashMap::new(),
                runtime_grants: Vec::new(),
            },
        );

        ApiState {
            auth_mode: "dev-loopback".to_string(),
            allow_unauthenticated: true,
            allowed_origins: Vec::new(),
            registry_root,
            executor,
            workspaces: Mutex::new(workspaces),
            idempotency_records: Mutex::new(HashMap::new()),
            idempotency_retention_seconds: DEFAULT_IDEMPOTENCY_RETENTION_SECONDS,
            jwt_verification_key: None,
        }
    }

    fn empty_state() -> ApiState<TestExecutor> {
        let executor = TestExecutor::ok(json!({}));
        let registry_root = test_registry_root();
        std::fs::create_dir_all(&registry_root).expect("registry root must be created");

        let mut workspaces = HashMap::new();
        let workspace_id = "ws-test";
        persist_local_test_workspace(&registry_root, workspace_id);
        workspaces.insert(
            workspace_id.to_string(),
            WorkspaceState {
                runtime: Runtime::new(CapabilityRegistry::new(), executor.clone())
                    .with_workflow_registry(WorkflowRegistry::new())
                    .with_security_config(RuntimeSecurityConfig::development()),
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
                app_events: Vec::new(),
                app_list_context_fields: HashMap::new(),
                app_state_machines: HashMap::new(),
                runtime_grants: Vec::new(),
            },
        );

        ApiState {
            auth_mode: "dev-loopback".to_string(),
            allow_unauthenticated: true,
            allowed_origins: Vec::new(),
            registry_root,
            executor,
            workspaces: Mutex::new(workspaces),
            idempotency_records: Mutex::new(HashMap::new()),
            idempotency_retention_seconds: DEFAULT_IDEMPOTENCY_RETENTION_SECONDS,
            jwt_verification_key: None,
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

    fn test_jwt_signing_key() -> ed25519_dalek::SigningKey {
        ed25519_dalek::SigningKey::from_bytes(&[7_u8; 32])
    }

    fn test_jwt_verifying_key_hex() -> String {
        use std::fmt::Write as _;
        let mut hex = String::new();
        for byte in test_jwt_signing_key().verifying_key().to_bytes() {
            let _ = write!(hex, "{byte:02x}");
        }
        hex
    }

    fn sign_jwt_eddsa(payload: &Value, key: &ed25519_dalek::SigningKey) -> String {
        use ed25519_dalek::Signer;
        let header = base64url_encode(br#"{"alg":"EdDSA","typ":"JWT"}"#);
        let payload_b64 = base64url_encode(payload.to_string().as_bytes());
        let signing_input = format!("{header}.{payload_b64}");
        let signature = key.sign(signing_input.as_bytes());
        let signature_b64 = base64url_encode(&signature.to_bytes());
        format!("{header}.{payload_b64}.{signature_b64}")
    }

    fn make_jwt(sub: &str, exp: i64, admin: bool) -> String {
        let mut payload = json!({ "sub": sub, "exp": exp });
        if admin {
            payload["traverse_admin"] = json!(true);
        }
        sign_jwt_eddsa(&payload, &test_jwt_signing_key())
    }

    fn make_scoped_jwt(sub: &str, exp: i64, scopes: &[&str]) -> String {
        let payload = json!({
            "sub": sub,
            "exp": exp,
            "scope": scopes.join(" ")
        });
        sign_jwt_eddsa(&payload, &test_jwt_signing_key())
    }

    /// Forge a token with `alg:none` and no real signature — the shape an
    /// attacker uses to strip verification.
    fn forge_unsigned_jwt(sub: &str, exp: i64, admin: bool) -> String {
        let header = base64url_encode(br#"{"alg":"none","typ":"JWT"}"#);
        let mut payload = json!({ "sub": sub, "exp": exp });
        if admin {
            payload["traverse_admin"] = json!(true);
        }
        let payload_b64 = base64url_encode(payload.to_string().as_bytes());
        format!("{header}.{payload_b64}.sig")
    }

    fn future_exp() -> i64 {
        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time must be valid")
            .as_secs();
        i64::try_from(now_secs).expect("time must fit i64") + 3600
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

    fn seed_list_context_fields(state: &ApiState<TestExecutor>, fields: &[&str]) {
        state
            .with_workspace_mut("ws-test", |ws| {
                ws.app_list_context_fields.insert(
                    "traverse-starter".to_string(),
                    fields.iter().map(|field| (*field).to_string()).collect(),
                );
                Ok(())
            })
            .expect("list context fields must be seeded");
    }

    fn seed_app_session_event(
        state: &ApiState<TestExecutor>,
        session_id: &str,
        current_state: &str,
        timestamp: &str,
        output: &Value,
    ) {
        state
            .with_workspace_mut("ws-test", |ws| {
                ws.app_events.push(AppStateEventRecord {
                    event_id: format!("{session_id}:{current_state}"),
                    event_type: "capability_result".to_string(),
                    workspace_id: "ws-test".to_string(),
                    app_id: "traverse-starter".to_string(),
                    session_id: session_id.to_string(),
                    execution_id: format!("exec_{session_id}"),
                    state: current_state.to_string(),
                    previous_state: Some("processing".to_string()),
                    timestamp: timestamp.to_string(),
                    data: json!({
                        "workspace_id": "ws-test",
                        "app_id": "traverse-starter",
                        "session_id": session_id,
                        "execution_id": format!("exec_{session_id}"),
                        "state": current_state,
                        "previous_state": "processing",
                        "timestamp": timestamp,
                        "output": output,
                    }),
                });
                Ok(())
            })
            .expect("app session event must be seeded");
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

    fn response_body_text(response: &[u8]) -> String {
        let pos = response
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .expect("response must contain \\r\\n\\r\\n");
        std::str::from_utf8(&response[pos + 4..])
            .expect("response body must be UTF-8")
            .to_string()
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
        handle_health(&mut out, "dev-loopback").expect("health must succeed");

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
        handle_health(&mut out, "bearer-required").expect("health must succeed");

        assert_eq!(response_status(&out), 200);
        let body = parse_response_body(&out);
        assert_eq!(body["status"], "ok");
        assert_eq!(body["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(body["api_version"], "v1");
        assert_eq!(body["workspace_default"], "local-default");
        assert_eq!(body["auth_mode"], "bearer-required");
    }

    #[test]
    fn health_endpoint_reports_dev_any_auth_mode() {
        let mut out = Vec::new();
        handle_health(&mut out, "dev-any").expect("health must succeed");

        assert_eq!(response_status(&out), 200);
        let body = parse_response_body(&out);
        assert_eq!(body["auth_mode"], "dev-any");
    }

    #[test]
    fn server_discovery_file_contains_health_url_and_local_token_metadata() {
        let repo_root = test_registry_root();
        let discovery_path = write_server_discovery(
            &repo_root,
            "http://127.0.0.1:8787",
            "dev-loopback",
            "traverse://connect?base_url=http%3A%2F%2F127.0.0.1%3A8787&workspace_default=local-default&auth_mode=dev-loopback",
            Some("local-token"),
        )
        .expect("discovery file must be written");

        assert_eq!(discovery_path, repo_root.join(".traverse/server.json"));
        let body = std::fs::read_to_string(&discovery_path).expect("discovery file must be read");
        let json: Value = serde_json::from_str(&body).expect("discovery must be valid json");
        assert_eq!(json["schema_version"], SERVER_DISCOVERY_SCHEMA_VERSION);
        assert_eq!(json["base_url"], "http://127.0.0.1:8787");
        assert_eq!(json["bind_address"], "127.0.0.1:8787");
        assert_eq!(json["health_url"], "http://127.0.0.1:8787/healthz");
        assert_eq!(json["workspace_default"], DEFAULT_WORKSPACE_ID);
        assert_eq!(json["auth_mode"], "dev-loopback");
        assert_eq!(
            json["mobile_connect_url"],
            "traverse://connect?base_url=http%3A%2F%2F127.0.0.1%3A8787&workspace_default=local-default&auth_mode=dev-loopback"
        );
        assert_eq!(json["local_dev_token"], "local-token");
        assert!(json["pid"].as_u64().is_some());
        assert!(
            json["started_at"]
                .as_str()
                .is_some_and(|value| !value.is_empty())
        );
    }

    #[test]
    fn server_discovery_file_omits_local_token_when_none_was_minted() {
        let repo_root = test_registry_root();
        let discovery_path = write_server_discovery(
            &repo_root,
            "http://127.0.0.1:8787",
            "bearer-required",
            "traverse://connect?base_url=http%3A%2F%2F127.0.0.1%3A8787",
            None,
        )
        .expect("discovery file must be written");

        let body = std::fs::read_to_string(&discovery_path).expect("discovery file must be read");
        let json: Value = serde_json::from_str(&body).expect("discovery must be valid json");
        assert!(json.get("local_dev_token").is_none());
        assert_eq!(json["auth_mode"], "bearer-required");
    }

    #[test]
    fn server_discovery_reports_a_filesystem_error_when_repo_root_is_a_file() {
        let repo_root = test_registry_root();
        std::fs::create_dir_all(&repo_root).expect("fixture root must be created");
        let file_root = repo_root.join("not-a-directory");
        std::fs::write(&file_root, "fixture").expect("fixture file must be written");

        let error = write_server_discovery(
            &file_root,
            "http://127.0.0.1:8787",
            "bearer-required",
            "traverse://connect",
            None,
        )
        .expect_err("a file cannot contain the .traverse directory");

        assert!(error.contains("failed to create .traverse directory"));
    }

    #[test]
    fn server_discovery_reports_a_filesystem_error_when_server_file_is_a_directory() {
        let repo_root = test_registry_root();
        let server_path = repo_root.join(".traverse/server.json");
        std::fs::create_dir_all(&server_path).expect("server path directory must be created");

        let error = write_server_discovery(
            &repo_root,
            "http://127.0.0.1:8787",
            "bearer-required",
            "traverse://connect",
            None,
        )
        .expect_err("a directory cannot receive server discovery JSON");

        assert!(error.contains("failed to write"));
        assert!(error.contains("server.json"));
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
            "traverse://connect?base_url=http%3A%2F%2F127.0.0.1%3A8787&workspace_default=local-default&auth_mode=dev-loopback",
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

    #[test]
    fn mobile_connect_url_percent_encodes_runtime_fields() {
        let url = mobile_connect_url("http://192.168.1.42:8787", "local default", "dev-loopback");

        assert_eq!(
            url,
            "traverse://connect?base_url=http%3A%2F%2F192.168.1.42%3A8787&workspace_default=local%20default&auth_mode=dev-loopback"
        );
    }

    #[test]
    fn local_dev_token_has_a_scoped_nonempty_suffix() {
        let token = mint_local_dev_token("127.0.0.1:8787");
        let prefix = format!("trv_local_{}_", std::process::id());

        assert!(token.starts_with(&prefix));
        assert!(
            token
                .strip_prefix(&prefix)
                .is_some_and(|suffix| !suffix.is_empty())
        );
    }

    #[test]
    fn server_discovery_file_records_dev_any_bind_address() {
        let repo_root = test_registry_root();
        let discovery_path = write_server_discovery(
            &repo_root,
            "http://0.0.0.0:8787",
            "dev-any",
            "traverse://connect?base_url=http%3A%2F%2F0.0.0.0%3A8787&workspace_default=local-default&auth_mode=dev-any",
            Some("local-token"),
        )
        .expect("discovery file must be written");

        let body = std::fs::read_to_string(&discovery_path).expect("discovery file must be read");
        let json: Value = serde_json::from_str(&body).expect("discovery must be valid json");
        assert_eq!(json["auth_mode"], "dev-any");
        assert_eq!(json["bind_address"], "0.0.0.0:8787");
    }

    #[test]
    fn mobile_connect_qr_renders_ascii_blocks() {
        let rendered = render_mobile_connect_qr(
            "traverse://connect?base_url=http%3A%2F%2F127.0.0.1%3A8787&workspace_default=local-default&auth_mode=dev-loopback",
        )
        .expect("QR code must render");

        assert!(rendered.contains("██"));
        assert!(rendered.lines().count() > 10);
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
    fn non_loopback_requires_bearer_even_when_dev_unauthenticated_is_enabled() {
        let state = empty_state();
        let req = with_workspace_query(
            make_http_request("GET", "/v1/capabilities", Vec::new()),
            "ws-prod",
        );

        let mut out = Vec::new();
        handle_list_capabilities(&mut out, &req, &state, false)
            .expect("list must write a response");

        assert_eq!(response_status(&out), 401);
        assert_eq!(response_content_type(&out), "application/problem+json");
        assert_eq!(parse_response_body(&out)["traverse_code"], "unauthorized");
    }

    #[test]
    fn non_loopback_rejects_valid_bearer_without_required_scope() {
        let state = empty_state();
        let token = make_scoped_jwt("alice", future_exp(), &["runtime:execute"]);
        let req = with_bearer(
            with_workspace_query(
                make_http_request("GET", "/v1/capabilities", Vec::new()),
                "ws-prod",
            ),
            &token,
        );

        let mut out = Vec::new();
        handle_list_capabilities(&mut out, &req, &state, false)
            .expect("list must write a response");

        assert_eq!(response_status(&out), 403);
        assert_eq!(response_content_type(&out), "application/problem+json");
        assert_eq!(parse_response_body(&out)["traverse_code"], "unauthorized");
    }

    #[test]
    fn non_loopback_allows_valid_bearer_with_required_scope() {
        let state = empty_state();
        let token = make_scoped_jwt("alice", future_exp(), &["registry:read"]);
        let req = with_bearer(
            with_workspace_query(
                make_http_request("GET", "/v1/capabilities", Vec::new()),
                "ws-prod",
            ),
            &token,
        );

        let mut out = Vec::new();
        handle_list_capabilities(&mut out, &req, &state, false)
            .expect("list must write a response");

        assert_eq!(response_status(&out), 200);
        assert!(
            parse_response_body(&out)
                .as_array()
                .expect("array")
                .is_empty()
        );
    }

    #[test]
    fn runtime_grant_approval_requires_grants_approve_scope() {
        let state = empty_state();
        let token = make_scoped_jwt("alice", future_exp(), &["runtime:execute"]);
        let req = with_bearer(
            make_http_request(
                "POST",
                "/v1/workspaces/ws-test/runtime-grants",
                runtime_grant_body(
                    "test.api.do-something",
                    "external.api.read",
                    "resource:one",
                    "execution",
                    3600,
                ),
            ),
            &token,
        );

        let mut out = Vec::new();
        handle_workspace_operation(&mut out, &req, &state, false)
            .expect("grant approval must write a response");

        assert_eq!(response_status(&out), 403);
        assert_eq!(parse_response_body(&out)["traverse_code"], "unauthorized");
    }

    #[test]
    fn runtime_grant_approval_returns_created_grant() {
        let state = empty_state();
        persist_test_workspace(&state.registry_root, "ws-test", "alice");
        let token = make_scoped_jwt("alice", future_exp(), &["grants:approve"]);
        let req = with_bearer(
            make_http_request(
                "POST",
                "/v1/workspaces/ws-test/runtime-grants",
                runtime_grant_body(
                    "test.api.do-something",
                    "external.api.read",
                    "resource:one",
                    "execution",
                    3600,
                ),
            ),
            &token,
        );

        let mut out = Vec::new();
        handle_workspace_operation(&mut out, &req, &state, false)
            .expect("grant approval must write a response");

        assert_eq!(response_status(&out), 201);
        let body = parse_response_body(&out);
        assert_eq!(body["approved"], true);
        assert_eq!(body["grant"]["grant_scope"], "external.api.read");
        assert_eq!(body["grant"]["approved_by"], "alice");
    }

    #[test]
    fn execution_runtime_grant_is_available_once_then_consumed() {
        let state = test_state_with("test.api.do-something", "1.0.0");
        persist_test_workspace(&state.registry_root, "ws-test", "alice");
        let approve_token = make_scoped_jwt("alice", future_exp(), &["grants:approve"]);
        let approve_req = with_bearer(
            make_http_request(
                "POST",
                "/v1/workspaces/ws-test/runtime-grants",
                runtime_grant_body(
                    "test.api.do-something",
                    "external.api.read",
                    "resource:one",
                    "execution",
                    3600,
                ),
            ),
            &approve_token,
        );
        let mut approve_out = Vec::new();
        handle_workspace_operation(&mut approve_out, &approve_req, &state, false)
            .expect("grant approval must write a response");
        assert_eq!(response_status(&approve_out), 201);

        let execute_token = make_scoped_jwt("alice", future_exp(), &["runtime:execute"]);
        let first_req = with_bearer(
            make_http_request(
                "POST",
                "/v1/workspaces/ws-test/execute",
                make_runtime_request_body("test.api.do-something"),
            ),
            &execute_token,
        );
        let second_req = with_bearer(
            make_http_request(
                "POST",
                "/v1/workspaces/ws-test/execute",
                make_runtime_request_body("test.api.do-something"),
            ),
            &execute_token,
        );

        let mut first_out = Vec::new();
        handle_workspace_operation(&mut first_out, &first_req, &state, false)
            .expect("first execute must write a response");
        let first = parse_response_body(&first_out);
        assert_eq!(response_status(&first_out), 200);
        assert_eq!(first["runtime_grants"].as_array().map(Vec::len), Some(1));

        let mut second_out = Vec::new();
        handle_workspace_operation(&mut second_out, &second_req, &state, false)
            .expect("second execute must write a response");
        let second = parse_response_body(&second_out);
        assert_eq!(response_status(&second_out), 200);
        assert_eq!(second["runtime_grants"].as_array().map(Vec::len), Some(0));
    }

    #[test]
    fn expired_session_grant_is_pruned_and_static_permissions_remain() {
        let state =
            test_state_with_permission("test.api.permissioned", "1.0.0", "static.permission.read");
        persist_test_workspace(&state.registry_root, "ws-test", "alice");
        let approve_token = make_scoped_jwt("alice", future_exp(), &["grants:approve"]);
        let approve_req = with_bearer(
            make_http_request(
                "POST",
                "/v1/workspaces/ws-test/runtime-grants",
                runtime_grant_body(
                    "test.api.permissioned",
                    "external.api.read",
                    "resource:expired",
                    "session",
                    0,
                ),
            ),
            &approve_token,
        );
        let mut approve_out = Vec::new();
        handle_workspace_operation(&mut approve_out, &approve_req, &state, false)
            .expect("grant approval must write a response");
        assert_eq!(response_status(&approve_out), 201);

        let execute_token = make_scoped_jwt("alice", future_exp(), &["runtime:execute"]);
        let execute_req = with_bearer(
            make_http_request(
                "POST",
                "/v1/workspaces/ws-test/execute",
                make_runtime_request_body("test.api.permissioned"),
            ),
            &execute_token,
        );
        let mut execute_out = Vec::new();
        handle_workspace_operation(&mut execute_out, &execute_req, &state, false)
            .expect("execute must write a response");

        let body = parse_response_body(&execute_out);
        assert_eq!(response_status(&execute_out), 200);
        assert_eq!(body["runtime_grants"].as_array().map(Vec::len), Some(0));
        assert_eq!(
            body["static_permissions"][0]["id"],
            "static.permission.read"
        );
    }

    #[test]
    fn runtime_grant_lifecycle_writes_secret_free_audit_entries() {
        let state = test_state_with("test.api.do-something", "1.0.0");
        persist_test_workspace(&state.registry_root, "ws-test", "alice");
        let approve_token = make_scoped_jwt("alice", future_exp(), &["grants:approve"]);
        let approve_req = with_bearer(
            make_http_request(
                "POST",
                "/v1/workspaces/ws-test/runtime-grants",
                runtime_grant_body(
                    "test.api.do-something",
                    "external.api.read",
                    "resource:one",
                    "execution",
                    3600,
                ),
            ),
            &approve_token,
        );
        let mut approve_out = Vec::new();
        handle_workspace_operation(&mut approve_out, &approve_req, &state, false)
            .expect("grant approval must write a response");
        assert_eq!(response_status(&approve_out), 201);

        let execute_token = make_scoped_jwt("alice", future_exp(), &["runtime:execute"]);
        let execute_req = with_bearer(
            make_http_request(
                "POST",
                "/v1/workspaces/ws-test/execute",
                make_runtime_request_body("test.api.do-something"),
            ),
            &execute_token,
        );
        let mut execute_out = Vec::new();
        handle_workspace_operation(&mut execute_out, &execute_req, &state, false)
            .expect("execute must write a response");
        assert_eq!(response_status(&execute_out), 200);

        let text = audit_log_text(&state, "ws-test");
        assert!(!text.contains(&approve_token));
        assert!(!text.contains(&execute_token));
        let event_types: Vec<String> = audit_log_entries(&state, "ws-test")
            .iter()
            .filter_map(|entry| entry["event_type"].as_str().map(ToString::to_string))
            .collect();
        assert!(event_types.contains(&"runtime_grant_created".to_string()));
        assert!(event_types.contains(&"runtime_grant_used".to_string()));
        assert!(event_types.contains(&"runtime_grant_revoked".to_string()));
    }

    #[test]
    fn auth_failure_writes_secret_free_audit_entry() {
        let state = empty_state();
        persist_test_workspace(&state.registry_root, "ws-test", "alice");
        let token = make_scoped_jwt("alice", future_exp(), &["runtime:execute"]);
        let req = with_bearer(
            make_http_request(
                "POST",
                "/v1/workspaces/ws-test/runtime-grants",
                runtime_grant_body(
                    "test.api.do-something",
                    "external.api.read",
                    "resource:one",
                    "execution",
                    3600,
                ),
            ),
            &token,
        );
        let mut out = Vec::new();
        handle_workspace_operation(&mut out, &req, &state, false)
            .expect("grant approval must write a response");

        assert_eq!(response_status(&out), 403);
        let text = audit_log_text(&state, "ws-test");
        assert!(!text.contains(&token));
        let entries = audit_log_entries(&state, "ws-test");
        assert_eq!(entries[0]["event_type"], "auth_failure");
        assert_eq!(entries[0]["traverse_code"], "unauthorized");
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
    fn system_workspace_allows_verified_admin_jwt() {
        let mut state = empty_state();
        state.jwt_verification_key = Some(
            parse_ed25519_verifying_key(&test_jwt_verifying_key_hex())
                .expect("test verifying key must parse"),
        );
        let token = make_jwt("admin-user", future_exp(), true);
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

    #[test]
    fn forged_unsigned_admin_token_is_denied_system_workspace() {
        let mut state = empty_state();
        state.jwt_verification_key = Some(
            parse_ed25519_verifying_key(&test_jwt_verifying_key_hex())
                .expect("test verifying key must parse"),
        );
        let token = forge_unsigned_jwt("attacker", future_exp(), true);
        let req = with_bearer(
            with_workspace_query(
                make_http_request("GET", "/v1/capabilities", Vec::new()),
                SYSTEM_WORKSPACE_ID,
            ),
            &token,
        );
        let mut out = Vec::new();
        handle_list_capabilities(&mut out, &req, &state, true).expect("list must write a response");
        assert_eq!(response_status(&out), 401);
        assert_eq!(
            parse_response_body(&out)["traverse_code"],
            "token_alg_not_allowed"
        );
    }

    #[test]
    fn tampered_payload_of_signed_token_is_rejected() {
        let key = Some(
            parse_ed25519_verifying_key(&test_jwt_verifying_key_hex())
                .expect("test verifying key must parse"),
        );
        let token = make_jwt("alice", future_exp(), false);
        let mut parts = token.split('.');
        let header = parts.next().expect("header");
        let signature = parts.nth(1).expect("signature");
        let forged_payload = base64url_encode(
            json!({ "sub": "alice", "exp": future_exp(), "traverse_admin": true })
                .to_string()
                .as_bytes(),
        );
        let tampered = format!("{header}.{forged_payload}.{signature}");
        let mut headers = HashMap::new();
        headers.insert("authorization".to_string(), format!("Bearer {tampered}"));

        let err = subject_from_request(&headers, "bearer-required", false, false, key.as_ref())
            .expect_err("tampered token must be rejected");
        assert_eq!(err.status, 401);
        assert_eq!(err.code, "signature_verification_failed");
    }

    #[test]
    fn bearer_required_without_key_rejects_all_tokens() {
        let token = make_jwt("alice", future_exp(), false);
        let mut headers = HashMap::new();
        headers.insert("authorization".to_string(), format!("Bearer {token}"));

        let err = subject_from_request(&headers, "bearer-required", false, false, None)
            .expect_err("fail closed without a verification key");
        assert_eq!(err.status, 401);
        assert_eq!(err.code, "jwt_verification_unavailable");
    }

    #[test]
    fn bearer_required_rejects_opaque_non_jwt_token() {
        let mut headers = HashMap::new();
        headers.insert(
            "authorization".to_string(),
            "Bearer system_admin".to_string(),
        );

        let err = subject_from_request(&headers, "bearer-required", false, false, None)
            .expect_err("opaque tokens are not accepted on a network listener");
        assert_eq!(err.status, 401);
        assert_eq!(err.code, "unauthorized");
    }

    #[test]
    fn dev_mode_unverified_jwt_cannot_be_admin() {
        // dev-loopback, no key configured: a signed-shaped token still parses a
        // subject but can never yield admin because it was not verified.
        let token = make_jwt("admin-user", future_exp(), true);
        let mut headers = HashMap::new();
        headers.insert("authorization".to_string(), format!("Bearer {token}"));

        let identity = subject_from_request(&headers, "dev-loopback", false, true, None)
            .expect("dev token must resolve to a subject");
        assert_eq!(identity.subject_id, "admin-user");
        assert!(!identity.is_admin);
    }

    #[test]
    fn verified_jwt_yields_admin_when_signed() {
        let key = Some(
            parse_ed25519_verifying_key(&test_jwt_verifying_key_hex())
                .expect("test verifying key must parse"),
        );
        let token = make_jwt("admin-user", future_exp(), true);
        let mut headers = HashMap::new();
        headers.insert("authorization".to_string(), format!("Bearer {token}"));

        let identity =
            subject_from_request(&headers, "bearer-required", false, false, key.as_ref())
                .expect("verified token must resolve");
        assert_eq!(identity.subject_id, "admin-user");
        assert!(identity.is_admin);
    }

    #[test]
    fn nbf_in_future_rejects_token() {
        let key = Some(
            parse_ed25519_verifying_key(&test_jwt_verifying_key_hex())
                .expect("test verifying key must parse"),
        );
        let payload = json!({ "sub": "alice", "exp": future_exp(), "nbf": future_exp() });
        let token = sign_jwt_eddsa(&payload, &test_jwt_signing_key());
        let mut headers = HashMap::new();
        headers.insert("authorization".to_string(), format!("Bearer {token}"));

        let err = subject_from_request(&headers, "bearer-required", false, false, key.as_ref())
            .expect_err("not-yet-valid token must be rejected");
        assert_eq!(err.code, "token_not_yet_valid");
    }

    #[test]
    fn parse_ed25519_verifying_key_rejects_wrong_length() {
        assert!(parse_ed25519_verifying_key("00ff").is_err());
        assert!(parse_ed25519_verifying_key("zz").is_err());
        assert!(parse_ed25519_verifying_key(&test_jwt_verifying_key_hex()).is_ok());
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
    fn workspace_execute_endpoint_accepts_simplified_capability_request() {
        let body = json!({
            "capability_id": "test.api.do-something",
            "input": {
                "value": "demo"
            }
        })
        .to_string()
        .into_bytes();
        let state = test_state_with("test.api.do-something", "1.0.0");
        let req = make_http_request("POST", "/v1/workspaces/ws-test/execute", body);

        let mut out = Vec::new();
        handle_execute_workspace(&mut out, &req, &state, true, "ws-test")
            .expect("execute must write a response");

        let status = response_status(&out);
        let resp = parse_response_body(&out);

        assert_eq!(status, 200);
        assert_eq!(resp["status"], "succeeded");
        assert_eq!(resp["output"]["result"], "ok");
        assert_eq!(resp["execution_id"], "exec_http_test_api_do_something");
    }

    #[test]
    fn workspace_execute_endpoint_rejects_simplified_request_without_capability_id() {
        let body = json!({"input": {"value": "demo"}}).to_string().into_bytes();
        let state = test_state_with("test.api.do-something", "1.0.0");
        let req = make_http_request("POST", "/v1/workspaces/ws-test/execute", body);

        let mut out = Vec::new();
        handle_execute_workspace(&mut out, &req, &state, true, "ws-test")
            .expect("invalid request must write a response");

        assert_eq!(response_status(&out), 400);
        assert_eq!(
            parse_response_body(&out)["traverse_code"],
            "invalid_request"
        );
    }

    #[test]
    fn workspace_execute_endpoint_rejects_blank_simplified_capability_id() {
        let body = json!({
            "capability_id": "  ",
            "input": {"value": "demo"}
        })
        .to_string()
        .into_bytes();
        let state = test_state_with("test.api.do-something", "1.0.0");
        let req = make_http_request("POST", "/v1/workspaces/ws-test/execute", body);

        let mut out = Vec::new();
        handle_execute_workspace(&mut out, &req, &state, true, "ws-test")
            .expect("invalid request must write a response");

        assert_eq!(response_status(&out), 400);
        assert_eq!(
            parse_response_body(&out)["traverse_code"],
            "invalid_request"
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
        assert_eq!(
            state
                .idempotency_records
                .lock()
                .expect("idempotency lock must not be poisoned")
                .len(),
            1
        );
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

        state
            .idempotency_records
            .lock()
            .expect("idempotency lock must not be poisoned")
            .insert(
                "old".to_string(),
                IdempotencyRecord {
                    body_digest: "fnv1a64:old".to_string(),
                    status: 200,
                    reason: "OK".to_string(),
                    content_type: "application/json".to_string(),
                    body: b"{}".to_vec(),
                    stored_at: unix_timestamp()
                        .saturating_sub(MIN_IDEMPOTENCY_RETENTION_SECONDS + 1),
                },
            );
        let mut state = state;
        state.idempotency_retention_seconds = 1;
        prune_idempotency_records(&state);

        assert!(
            state
                .idempotency_records
                .lock()
                .expect("idempotency lock must not be poisoned")
                .is_empty()
        );
    }

    #[test]
    fn idempotency_retention_normalizes_default_floor_and_explicit_values() {
        assert_eq!(
            configured_idempotency_retention(None),
            DEFAULT_IDEMPOTENCY_RETENTION_SECONDS
        );
        assert_eq!(
            configured_idempotency_retention(Some(1)),
            MIN_IDEMPOTENCY_RETENTION_SECONDS
        );
        assert_eq!(
            configured_idempotency_retention(Some(MIN_IDEMPOTENCY_RETENTION_SECONDS + 1)),
            MIN_IDEMPOTENCY_RETENTION_SECONDS + 1
        );
    }

    #[test]
    fn in_process_api_initializes_system_workspace_and_configured_state() {
        let registry_root = test_registry_root();
        let api = InProcessApi::new(ApiServerConfig {
            bind_address: "127.0.0.1:0".to_string(),
            requested_auth_mode: None,
            allow_unauthenticated: true,
            allowed_origins: vec!["http://127.0.0.1:3000".to_string()],
            render_mobile_qr: false,
            capability_registry: CapabilityRegistry::new(),
            workflow_registry: WorkflowRegistry::new(),
            registry_root: registry_root.clone(),
            executor: TestExecutor::ok(json!({})),
            idempotency_retention_seconds: Some(1),
            jwt_verification_key_hex: None,
            read_timeout: None,
            write_timeout: None,
            request_deadline: None,
            max_concurrent_connections: None,
        });

        assert!(api.state.allow_unauthenticated);
        assert_eq!(api.state.allowed_origins, ["http://127.0.0.1:3000"]);
        assert_eq!(api.state.registry_root, registry_root);
        assert_eq!(
            api.state.idempotency_retention_seconds,
            MIN_IDEMPOTENCY_RETENTION_SECONDS
        );
        assert!(
            api.state
                .workspaces
                .lock()
                .expect("workspace lock must not be poisoned")
                .contains_key(SYSTEM_WORKSPACE_ID)
        );
    }

    #[test]
    fn in_process_api_rejects_system_workspace_listing_without_membership_metadata() {
        let api = InProcessApi::new(ApiServerConfig {
            bind_address: "127.0.0.1:0".to_string(),
            requested_auth_mode: None,
            allow_unauthenticated: true,
            allowed_origins: Vec::new(),
            render_mobile_qr: false,
            capability_registry: CapabilityRegistry::new(),
            workflow_registry: WorkflowRegistry::new(),
            registry_root: test_registry_root(),
            executor: TestExecutor::ok(json!({})),
            idempotency_retention_seconds: None,
            jwt_verification_key_hex: None,
            read_timeout: None,
            write_timeout: None,
            request_deadline: None,
            max_concurrent_connections: None,
        });

        let (status, body) = api
            .list_workflows(SYSTEM_WORKSPACE_ID, true)
            .expect("listing must render a JSON response");

        assert_eq!(status, 403);
        assert_eq!(body["status"], 403);
    }

    #[test]
    fn in_process_api_lists_an_authorized_empty_workspace() {
        let registry_root = test_registry_root();
        persist_local_test_workspace(&registry_root, "ws-authorized");
        let api = InProcessApi::new(ApiServerConfig {
            bind_address: "127.0.0.1:0".to_string(),
            requested_auth_mode: None,
            allow_unauthenticated: true,
            allowed_origins: Vec::new(),
            render_mobile_qr: false,
            capability_registry: CapabilityRegistry::new(),
            workflow_registry: WorkflowRegistry::new(),
            registry_root,
            executor: TestExecutor::ok(json!({})),
            idempotency_retention_seconds: None,
            jwt_verification_key_hex: None,
            read_timeout: None,
            write_timeout: None,
            request_deadline: None,
            max_concurrent_connections: None,
        });

        let (status, body) = api
            .list_workflows("ws-authorized", true)
            .expect("listing must render a JSON response");

        assert_eq!(status, 200);
        assert_eq!(body, json!([]));
    }

    #[test]
    fn in_process_api_returns_not_found_for_missing_authorized_workflow() {
        let registry_root = test_registry_root();
        persist_local_test_workspace(&registry_root, "ws-authorized");
        let api = InProcessApi::new(ApiServerConfig {
            bind_address: "127.0.0.1:0".to_string(),
            requested_auth_mode: None,
            allow_unauthenticated: true,
            allowed_origins: Vec::new(),
            render_mobile_qr: false,
            capability_registry: CapabilityRegistry::new(),
            workflow_registry: WorkflowRegistry::new(),
            registry_root,
            executor: TestExecutor::ok(json!({})),
            idempotency_retention_seconds: None,
            jwt_verification_key_hex: None,
            read_timeout: None,
            write_timeout: None,
            request_deadline: None,
            max_concurrent_connections: None,
        });

        let (status, body) = api
            .get_workflow("ws-authorized", "missing-workflow", None, true)
            .expect("lookup must render a JSON response");

        assert_eq!(status, 404);
        assert_eq!(body["status"], 404);
        assert_eq!(body["traverse_code"], "workflow_not_found");
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
    fn app_events_path_parses_workspace_and_app_id() {
        assert_eq!(
            workspace_app_events_path("/v1/workspaces/ws-test/apps/traverse-starter/events"),
            Some(("ws-test".to_string(), "traverse-starter".to_string()))
        );
        assert!(workspace_app_events_path("/v1/workspaces/ws-test/apps//events").is_none());
        assert!(
            workspace_app_events_path("/v1/workspaces/ws-test/apps/traverse-starter/other")
                .is_none()
        );
    }

    fn seed_app_state_machine(state: &ApiState<TestExecutor>, app_id: &str, capability_id: &str) {
        let machine = ApplicationStateMachine {
            initial_state: "idle".to_string(),
            list_context_fields: Vec::new(),
            states: vec![
                ApplicationState {
                    id: "idle".to_string(),
                    invoke: None,
                    transitions: vec![ApplicationStateTransition {
                        on: "submit".to_string(),
                        to: "processing".to_string(),
                        condition: None,
                        with_last_payload: false,
                    }],
                },
                ApplicationState {
                    id: "processing".to_string(),
                    invoke: Some(ApplicationStateInvoke {
                        capability_id: capability_id.to_string(),
                        input_from: "command.payload".to_string(),
                    }),
                    transitions: vec![
                        ApplicationStateTransition {
                            on: "capability_succeeded".to_string(),
                            to: "results".to_string(),
                            condition: None,
                            with_last_payload: false,
                        },
                        ApplicationStateTransition {
                            on: "capability_failed".to_string(),
                            to: "error".to_string(),
                            condition: None,
                            with_last_payload: false,
                        },
                    ],
                },
                ApplicationState {
                    id: "results".to_string(),
                    invoke: None,
                    transitions: Vec::new(),
                },
                ApplicationState {
                    id: "error".to_string(),
                    invoke: None,
                    transitions: Vec::new(),
                },
            ],
        };
        state
            .with_workspace_mut("ws-test", |ws| {
                ws.app_state_machines.insert(app_id.to_string(), machine);
                Ok(())
            })
            .expect("state machine must be seeded");
    }

    fn conditional_transition(
        op: ApplicationStateTransitionConditionOp,
        value: Value,
        to: &str,
    ) -> ApplicationStateTransition {
        ApplicationStateTransition {
            on: "capability_succeeded".to_string(),
            to: to.to_string(),
            condition: Some(ApplicationStateTransitionCondition {
                field: "output.confidence_score".to_string(),
                op,
                value: Some(value),
            }),
            with_last_payload: false,
        }
    }

    fn replace_succeeded_transitions(
        state: &ApiState<TestExecutor>,
        transitions: Vec<ApplicationStateTransition>,
    ) {
        state
            .with_workspace_mut("ws-test", |ws| {
                let machine = ws
                    .app_state_machines
                    .get_mut("traverse-starter")
                    .expect("state machine must be present");
                let processing = machine
                    .states
                    .iter_mut()
                    .find(|state| state.id == "processing")
                    .expect("processing state must be present");
                processing
                    .transitions
                    .retain(|transition| transition.on != "capability_succeeded");
                processing.transitions.splice(0..0, transitions);
                Ok(())
            })
            .expect("state machine transitions must be replaced");
    }

    fn make_app_command_body(command: &str, payload: &Value, session_id: Option<&str>) -> Vec<u8> {
        let mut body = json!({
            "command": command,
            "payload": payload,
        });
        if let (Some(session_id), Value::Object(root)) = (session_id, &mut body) {
            root.insert(
                "session_id".to_string(),
                Value::String(session_id.to_string()),
            );
        }
        body.to_string().into_bytes()
    }

    #[test]
    fn app_commands_path_parses_workspace_and_app_id() {
        assert_eq!(
            workspace_app_commands_path("/v1/workspaces/ws-test/apps/traverse-starter/commands"),
            Some(("ws-test".to_string(), "traverse-starter".to_string()))
        );
        assert!(workspace_app_commands_path("/v1/workspaces/ws-test/apps//commands").is_none());
        assert!(
            workspace_app_commands_path("/v1/workspaces/ws-test/apps/traverse-starter/other")
                .is_none()
        );
    }

    #[test]
    fn app_command_returns_404_for_app_without_state_machine() {
        let state = empty_state();
        let req = make_http_request(
            "POST",
            "/v1/workspaces/ws-test/apps/traverse-starter/commands",
            make_app_command_body("submit", &json!({}), None),
        );

        let mut out = Vec::new();
        handle_workspace_operation(&mut out, &req, &state, true)
            .expect("command dispatch must write a response");

        assert_eq!(response_status(&out), 404);
        assert_eq!(response_content_type(&out), "application/problem+json");
        let resp = parse_response_body(&out);
        assert_eq!(resp["traverse_code"], "app_not_registered");
    }

    #[test]
    fn app_command_returns_422_for_unknown_command() {
        let state = test_state_with("traverse-starter.process", "1.0.0");
        seed_app_state_machine(&state, "traverse-starter", "traverse-starter.process");
        let req = make_http_request(
            "POST",
            "/v1/workspaces/ws-test/apps/traverse-starter/commands",
            make_app_command_body("does-not-exist", &json!({}), None),
        );

        let mut out = Vec::new();
        handle_workspace_operation(&mut out, &req, &state, true)
            .expect("command dispatch must write a response");

        assert_eq!(response_status(&out), 422);
        assert_eq!(response_content_type(&out), "application/problem+json");
        let resp = parse_response_body(&out);
        assert_eq!(resp["traverse_code"], "unknown_command");
        assert_eq!(resp["status"], 422);
    }

    #[test]
    fn app_command_returns_409_for_invalid_transition_from_current_state() {
        let state = test_state_with("traverse-starter.process", "1.0.0");
        seed_app_state_machine(&state, "traverse-starter", "traverse-starter.process");

        let first_req = make_http_request(
            "POST",
            "/v1/workspaces/ws-test/apps/traverse-starter/commands",
            make_app_command_body("submit", &json!({"note": "first"}), Some("sess-repeat")),
        );
        let mut first_out = Vec::new();
        handle_workspace_operation(&mut first_out, &first_req, &state, true)
            .expect("command dispatch must write a response");
        assert_eq!(response_status(&first_out), 202);

        let second_req = make_http_request(
            "POST",
            "/v1/workspaces/ws-test/apps/traverse-starter/commands",
            make_app_command_body("submit", &json!({"note": "second"}), Some("sess-repeat")),
        );
        let mut second_out = Vec::new();
        handle_workspace_operation(&mut second_out, &second_req, &state, true)
            .expect("command dispatch must write a response");

        assert_eq!(response_status(&second_out), 409);
        assert_eq!(
            response_content_type(&second_out),
            "application/problem+json"
        );
        let resp = parse_response_body(&second_out);
        assert_eq!(resp["traverse_code"], "invalid_transition");
        assert_eq!(resp["status"], 409);
    }

    #[test]
    fn app_command_returns_400_for_malformed_body() {
        let state = test_state_with("traverse-starter.process", "1.0.0");
        seed_app_state_machine(&state, "traverse-starter", "traverse-starter.process");
        let req = make_http_request(
            "POST",
            "/v1/workspaces/ws-test/apps/traverse-starter/commands",
            b"{\"payload\": {}}".to_vec(),
        );

        let mut out = Vec::new();
        handle_workspace_operation(&mut out, &req, &state, true)
            .expect("command dispatch must write a response");

        assert_eq!(response_status(&out), 400);
        let resp = parse_response_body(&out);
        assert_eq!(resp["traverse_code"], "invalid_request");
    }

    #[test]
    fn app_command_dispatch_emits_state_changed_and_capability_result_on_sse() {
        let state = test_state_with("traverse-starter.process", "1.0.0");
        seed_app_state_machine(&state, "traverse-starter", "traverse-starter.process");

        let command_req = make_http_request(
            "POST",
            "/v1/workspaces/ws-test/apps/traverse-starter/commands",
            make_app_command_body("submit", &json!({"note": "Meeting with design team"}), None),
        );
        let mut command_out = Vec::new();
        handle_workspace_operation(&mut command_out, &command_req, &state, true)
            .expect("command dispatch must write a response");

        assert_eq!(response_status(&command_out), 202);
        let accepted = parse_response_body(&command_out);
        assert_eq!(accepted["api_version"], "v1");
        assert_eq!(accepted["status"], "accepted");
        assert_eq!(accepted["command"], "submit");
        assert_eq!(accepted["state"], "processing");
        let session_id = accepted["session_id"]
            .as_str()
            .expect("session_id must be a string")
            .to_string();
        assert!(!session_id.is_empty());
        assert!(
            accepted["execution_id"]
                .as_str()
                .is_some_and(|id| !id.is_empty())
        );
        assert_eq!(
            accepted["links"]["events"],
            "/v1/workspaces/ws-test/apps/traverse-starter/events"
        );

        let events_req = make_http_request(
            "GET",
            "/v1/workspaces/ws-test/apps/traverse-starter/events",
            Vec::new(),
        );
        let mut events_out = Vec::new();
        handle_app_events(
            &mut events_out,
            &events_req,
            &state,
            true,
            "ws-test",
            "traverse-starter",
        )
        .expect("events endpoint must write a response");

        assert_eq!(response_status(&events_out), 200);
        assert_eq!(response_content_type(&events_out), "text/event-stream");
        let body = response_body_text(&events_out);
        assert!(body.contains("event: state_changed"));
        assert!(body.contains("\"state\":\"processing\""));
        assert!(body.contains("\"previous_state\":\"idle\""));
        assert!(body.contains("event: capability_invoked"));
        assert!(body.contains("\"capability_id\":\"traverse-starter.process\""));
        assert!(body.contains("event: capability_result"));
        assert!(body.contains("\"state\":\"results\""));
        assert!(body.contains("\"previous_state\":\"processing\""));
        assert!(body.contains("\"result\":\"ok\""));
        assert!(body.contains(&format!("\"session_id\":\"{session_id}\"")));
    }

    #[test]
    fn app_command_routes_capability_output_with_ordered_conditions_and_fallback() {
        for (confidence, expected_state) in [(0.90, "auto_approved"), (0.70, "pending_review")] {
            let state = test_state_with_output(
                "traverse-starter.process",
                "1.0.0",
                json!({"confidence_score": confidence}),
            );
            seed_app_state_machine(&state, "traverse-starter", "traverse-starter.process");
            replace_succeeded_transitions(
                &state,
                vec![
                    conditional_transition(
                        ApplicationStateTransitionConditionOp::Gte,
                        json!(0.85),
                        "auto_approved",
                    ),
                    ApplicationStateTransition {
                        on: "capability_succeeded".to_string(),
                        to: "pending_review".to_string(),
                        condition: None,
                        with_last_payload: false,
                    },
                ],
            );

            let command_req = make_http_request(
                "POST",
                "/v1/workspaces/ws-test/apps/traverse-starter/commands",
                make_app_command_body("submit", &json!({}), None),
            );
            let mut command_out = Vec::new();
            handle_workspace_operation(&mut command_out, &command_req, &state, true)
                .expect("command dispatch must write a response");

            let events_req = make_http_request(
                "GET",
                "/v1/workspaces/ws-test/apps/traverse-starter/events",
                Vec::new(),
            );
            let mut events_out = Vec::new();
            handle_app_events(
                &mut events_out,
                &events_req,
                &state,
                true,
                "ws-test",
                "traverse-starter",
            )
            .expect("events endpoint must write a response");
            let body = response_body_text(&events_out);
            assert!(body.contains(&format!("\"state\":\"{expected_state}\"")));
            assert!(body.contains("event: capability_result"));
        }
    }

    #[test]
    fn app_command_emits_condition_errors_without_changing_the_invoking_state() {
        let state = test_state_with_output(
            "traverse-starter.process",
            "1.0.0",
            json!({"confidence_score": "unknown"}),
        );
        seed_app_state_machine(&state, "traverse-starter", "traverse-starter.process");
        replace_succeeded_transitions(
            &state,
            vec![conditional_transition(
                ApplicationStateTransitionConditionOp::Gte,
                json!(0.85),
                "auto_approved",
            )],
        );

        let command_req = make_http_request(
            "POST",
            "/v1/workspaces/ws-test/apps/traverse-starter/commands",
            make_app_command_body("submit", &json!({}), None),
        );
        let mut command_out = Vec::new();
        handle_workspace_operation(&mut command_out, &command_req, &state, true)
            .expect("command dispatch must write a response");

        let events_req = make_http_request(
            "GET",
            "/v1/workspaces/ws-test/apps/traverse-starter/events",
            Vec::new(),
        );
        let mut events_out = Vec::new();
        handle_app_events(
            &mut events_out,
            &events_req,
            &state,
            true,
            "ws-test",
            "traverse-starter",
        )
        .expect("events endpoint must write a response");
        let body = response_body_text(&events_out);
        assert!(body.contains("event: error"));
        assert!(body.contains("\"code\":\"condition_type_error\""));
        assert!(body.contains("\"state\":\"processing\""));
        assert!(!body.contains("auto_approved"));
    }

    #[test]
    fn app_command_emits_no_matching_transition_without_changing_the_invoking_state() {
        let state = test_state_with_output(
            "traverse-starter.process",
            "1.0.0",
            json!({"confidence_score": 0.70}),
        );
        seed_app_state_machine(&state, "traverse-starter", "traverse-starter.process");
        replace_succeeded_transitions(
            &state,
            vec![conditional_transition(
                ApplicationStateTransitionConditionOp::Gte,
                json!(0.85),
                "auto_approved",
            )],
        );

        let command_req = make_http_request(
            "POST",
            "/v1/workspaces/ws-test/apps/traverse-starter/commands",
            make_app_command_body("submit", &json!({}), None),
        );
        let mut command_out = Vec::new();
        handle_workspace_operation(&mut command_out, &command_req, &state, true)
            .expect("command dispatch must write a response");

        let events_req = make_http_request(
            "GET",
            "/v1/workspaces/ws-test/apps/traverse-starter/events",
            Vec::new(),
        );
        let mut events_out = Vec::new();
        handle_app_events(
            &mut events_out,
            &events_req,
            &state,
            true,
            "ws-test",
            "traverse-starter",
        )
        .expect("events endpoint must write a response");
        let body = response_body_text(&events_out);
        assert!(body.contains("event: error"));
        assert!(body.contains("\"code\":\"no_matching_transition\""));
        assert!(body.contains("\"state\":\"processing\""));
        assert!(!body.contains("auto_approved"));
    }

    #[test]
    fn transition_condition_evaluator_supports_all_approved_operators() {
        let output = json!({"score": 2, "label": "ready", "present": true, "empty": null});
        let cases = [
            (
                ApplicationStateTransitionConditionOp::Eq,
                "output.label",
                json!("ready"),
            ),
            (
                ApplicationStateTransitionConditionOp::Neq,
                "output.label",
                json!("waiting"),
            ),
            (
                ApplicationStateTransitionConditionOp::Gt,
                "output.score",
                json!(1),
            ),
            (
                ApplicationStateTransitionConditionOp::Gte,
                "output.score",
                json!(2),
            ),
            (
                ApplicationStateTransitionConditionOp::Lt,
                "output.score",
                json!(3),
            ),
            (
                ApplicationStateTransitionConditionOp::Lte,
                "output.score",
                json!(2),
            ),
            (
                ApplicationStateTransitionConditionOp::In,
                "output.label",
                json!(["ready", "done"]),
            ),
            (
                ApplicationStateTransitionConditionOp::Exists,
                "output.present",
                Value::Null,
            ),
        ];
        for (op, field, value) in cases {
            let condition = ApplicationStateTransitionCondition {
                field: field.to_string(),
                op,
                value: if op == ApplicationStateTransitionConditionOp::Exists {
                    None
                } else {
                    Some(value)
                },
            };
            assert!(
                evaluate_transition_condition(&condition, &output)
                    .expect("approved condition must evaluate")
            );
        }

        let missing = ApplicationStateTransitionCondition {
            field: "output.missing".to_string(),
            op: ApplicationStateTransitionConditionOp::Exists,
            value: None,
        };
        assert!(
            !evaluate_transition_condition(&missing, &output).expect("missing field must evaluate")
        );
    }

    #[test]
    fn app_events_endpoint_returns_event_stream_heartbeat_when_empty() {
        let state = empty_state();
        let req = make_http_request(
            "GET",
            "/v1/workspaces/ws-test/apps/traverse-starter/events",
            Vec::new(),
        );

        let mut out = Vec::new();
        handle_app_events(&mut out, &req, &state, true, "ws-test", "traverse-starter")
            .expect("events endpoint must write a response");

        assert_eq!(response_status(&out), 200);
        assert_eq!(response_content_type(&out), "text/event-stream");
        assert_eq!(
            response_header(&out, "Cache-Control").as_deref(),
            Some("no-cache")
        );
        let body = response_body_text(&out);
        assert!(body.contains("event: heartbeat"));
        assert!(body.contains("\"app_id\":\"traverse-starter\""));
    }

    #[test]
    fn app_events_endpoint_replays_execution_events() {
        let body = make_runtime_request_body("traverse-starter.process");
        let state = test_state_with("traverse-starter.process", "1.0.0");
        let execute_req = make_http_request("POST", "/v1/workspaces/ws-test/execute", body);
        let mut execute_out = Vec::new();
        handle_execute_workspace(&mut execute_out, &execute_req, &state, true, "ws-test")
            .expect("execute must write a response");

        let req = make_http_request(
            "GET",
            "/v1/workspaces/ws-test/apps/traverse-starter/events",
            Vec::new(),
        );
        let mut out = Vec::new();
        handle_app_events(&mut out, &req, &state, true, "ws-test", "traverse-starter")
            .expect("events endpoint must write a response");

        assert_eq!(response_status(&out), 200);
        assert_eq!(response_content_type(&out), "text/event-stream");
        let body = response_body_text(&out);
        assert!(body.contains("event: state_changed"));
        assert!(body.contains("event: capability_invoked"));
        assert!(body.contains("event: capability_result"));
        assert!(body.contains("\"state\":\"results\""));
        assert!(body.contains("\"result\":\"ok\""));
    }

    #[test]
    fn app_events_endpoint_honors_last_event_id_replay() {
        let body = make_runtime_request_body("traverse-starter.process");
        let state = test_state_with("traverse-starter.process", "1.0.0");
        let execute_req = make_http_request("POST", "/v1/workspaces/ws-test/execute", body);
        let mut execute_out = Vec::new();
        handle_execute_workspace(&mut execute_out, &execute_req, &state, true, "ws-test")
            .expect("execute must write a response");

        let mut req = make_http_request(
            "GET",
            "/v1/workspaces/ws-test/apps/traverse-starter/events",
            Vec::new(),
        );
        req.headers.insert(
            "last-event-id".to_string(),
            "exec_test-req-001:state_changed:processing".to_string(),
        );
        let mut out = Vec::new();
        handle_app_events(&mut out, &req, &state, true, "ws-test", "traverse-starter")
            .expect("events endpoint must write a response");

        let body = response_body_text(&out);
        assert!(!body.contains("event: state_changed"));
        assert!(body.contains("event: capability_invoked"));
        assert!(body.contains("event: capability_result"));
    }

    #[test]
    fn app_sessions_path_parses_workspace_and_app_id() {
        assert_eq!(
            workspace_app_sessions_path("/v1/workspaces/ws-test/apps/traverse-starter/sessions"),
            Some(("ws-test".to_string(), "traverse-starter".to_string()))
        );
        assert!(workspace_app_sessions_path("/v1/workspaces/ws-test/apps//sessions").is_none());
        assert!(
            workspace_app_sessions_path("/v1/workspaces/ws-test/apps/traverse-starter/other")
                .is_none()
        );
    }

    #[test]
    fn app_sessions_endpoint_filters_state_and_projects_context_fields() {
        let state = empty_state();
        seed_app_session_event(
            &state,
            "sess-a",
            "pending_review",
            "unix:1",
            &json!({
                "document_type": "invoice",
                "confidence_score": 0.72,
                "secret_notes": "must not leak",
                "extracted_fields": { "summary": "Invoice from Acme Corp" }
            }),
        );
        seed_app_session_event(
            &state,
            "sess-b",
            "auto_approved",
            "unix:2",
            &json!({
                "document_type": "policy",
                "confidence_score": 0.91,
                "extracted_fields": { "summary": "Policy update" }
            }),
        );
        seed_app_session_event(
            &state,
            "sess-c",
            "pending_review",
            "unix:3",
            &json!({
                "document_type": "contract",
                "confidence_score": 0.61,
                "extracted_fields": { "summary": "Contract draft" }
            }),
        );
        seed_list_context_fields(
            &state,
            &[
                "output.document_type",
                "output.confidence_score",
                "output.extracted_fields.summary",
            ],
        );

        let mut req = make_http_request(
            "GET",
            "/v1/workspaces/ws-test/apps/traverse-starter/sessions",
            Vec::new(),
        );
        req.query
            .insert("state".to_string(), "pending_review".to_string());
        let mut out = Vec::new();
        handle_app_sessions(&mut out, &req, &state, true, "ws-test", "traverse-starter")
            .expect("sessions endpoint must write a response");

        assert_eq!(response_status(&out), 200);
        let resp = parse_response_body(&out);
        assert_eq!(resp["api_version"], "v1");
        assert_eq!(resp["total"], 2);
        let sessions = resp["sessions"].as_array().expect("sessions must be array");
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0]["session_id"], "sess-c");
        assert_eq!(sessions[0]["current_state"], "pending_review");
        assert_eq!(sessions[0]["context"]["document_type"], "contract");
        assert_eq!(sessions[0]["context"]["confidence_score"], 0.61);
        assert_eq!(
            sessions[0]["context"]["extracted_fields_summary"],
            "Contract draft"
        );
        assert!(sessions[0]["context"].get("secret_notes").is_none());
    }

    #[test]
    fn app_sessions_endpoint_paginates_with_cursor() {
        let state = empty_state();
        seed_app_session_event(
            &state,
            "sess-a",
            "results",
            "unix:1",
            &json!({ "document_type": "invoice" }),
        );
        seed_app_session_event(
            &state,
            "sess-b",
            "results",
            "unix:2",
            &json!({ "document_type": "contract" }),
        );
        seed_app_session_event(
            &state,
            "sess-c",
            "results",
            "unix:3",
            &json!({ "document_type": "policy" }),
        );
        seed_list_context_fields(&state, &["output.document_type"]);

        let mut first_req = make_http_request(
            "GET",
            "/v1/workspaces/ws-test/apps/traverse-starter/sessions",
            Vec::new(),
        );
        first_req.query.insert("limit".to_string(), "2".to_string());
        let mut first = Vec::new();
        handle_app_sessions(
            &mut first,
            &first_req,
            &state,
            true,
            "ws-test",
            "traverse-starter",
        )
        .expect("first page must write a response");
        let first_resp = parse_response_body(&first);
        assert_eq!(first_resp["sessions"][0]["session_id"], "sess-c");
        assert_eq!(first_resp["sessions"][1]["session_id"], "sess-b");
        assert_eq!(first_resp["next_cursor"], "sess-a");

        let mut second_req = make_http_request(
            "GET",
            "/v1/workspaces/ws-test/apps/traverse-starter/sessions",
            Vec::new(),
        );
        second_req
            .query
            .insert("cursor".to_string(), "sess-c".to_string());
        second_req
            .query
            .insert("order".to_string(), "created_desc".to_string());
        let mut second = Vec::new();
        handle_app_sessions(
            &mut second,
            &second_req,
            &state,
            true,
            "ws-test",
            "traverse-starter",
        )
        .expect("second page must write a response");
        let second_resp = parse_response_body(&second);
        assert_eq!(second_resp["sessions"][0]["session_id"], "sess-b");
        assert_eq!(second_resp["sessions"][1]["session_id"], "sess-a");
    }

    #[test]
    fn app_sessions_endpoint_rejects_invalid_limit() {
        let state = empty_state();
        let mut req = make_http_request(
            "GET",
            "/v1/workspaces/ws-test/apps/traverse-starter/sessions",
            Vec::new(),
        );
        req.query.insert("limit".to_string(), "0".to_string());

        let mut out = Vec::new();
        handle_app_sessions(&mut out, &req, &state, true, "ws-test", "traverse-starter")
            .expect("sessions endpoint must write a response");

        assert_eq!(response_status(&out), 400);
        assert_eq!(response_content_type(&out), "application/problem+json");
        assert_eq!(parse_response_body(&out)["traverse_code"], "invalid_query");
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

        assert!(!persisted_registry_path(&state.registry_root, "ws-test").exists());
        let reloaded = load_persisted_registry(&state.registry_root, "ws-test")
            .expect("journaled capability registration must reload");
        assert_eq!(reloaded.registrations.len(), 1);
    }

    #[test]
    fn workspace_capability_registration_rejects_mismatched_body_workspace() {
        let state = empty_state();
        let artifact_path = state.registry_root.join("mismatch.wasm");
        std::fs::write(&artifact_path, b"wasm bytes").expect("artifact must be writable");
        let mut body: Value = serde_json::from_slice(&valid_registration_body(
            "test.api.mismatch",
            "1.0.0",
            &artifact_path,
        ))
        .expect("fixture must be JSON");
        body["workspace_id"] = json!("other-workspace");
        let req = make_http_request(
            "POST",
            "/v1/workspaces/ws-test/capabilities",
            body.to_string().into_bytes(),
        );
        let mut out = Vec::new();
        handle_workspace_operation(&mut out, &req, &state, true).expect("handler must respond");
        assert_eq!(response_status(&out), 400);
        assert_eq!(
            parse_response_body(&out)["traverse_code"],
            "invalid_workspace_id"
        );
    }

    #[test]
    fn workspace_capability_registration_writes_audit_jsonl() {
        let state = empty_state();
        let artifact_path = state.registry_root.join("audit-module.wasm");
        std::fs::write(&artifact_path, b"wasm bytes").expect("artifact must be writable");
        let req = make_http_request(
            "POST",
            "/v1/workspaces/ws-test/capabilities",
            valid_registration_body("test.api.audited", "1.0.0", &artifact_path),
        );

        let mut out = Vec::new();
        handle_workspace_operation(&mut out, &req, &state, true)
            .expect("workspace registration must write a response");

        assert_eq!(response_status(&out), 201);
        let entries = audit_log_entries(&state, "ws-test");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0]["event_type"], "registration_attempted");
        assert_eq!(entries[0]["workspace_id"], "ws-test");
        assert_eq!(entries[0]["subject_id"], "local");
        assert_eq!(entries[1]["event_type"], "registration_outcome");
        assert_eq!(entries[1]["outcome"], "success");
        assert_eq!(
            entries[1]["target_resource"],
            "capability:test.api.audited@1.0.0"
        );
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
    fn workspace_event_contract_registration_duplicate_is_stable_across_a_clock_tick() {
        // Regression test for a CI flake (issue link in PR description): the
        // duplicate-vs-conflict decision must not depend on wall-clock time, or
        // two back-to-back identical registrations straddling a second boundary
        // spuriously return 409 instead of 200. Sleeping past a full second here
        // reproduces that race deterministically.
        let state = empty_state();
        let body = valid_event_registration_body("test.api.event-clock-tick", "1.0.0");
        let first_req = make_http_request(
            "POST",
            "/v1/workspaces/ws-test/event-contracts",
            body.clone(),
        );
        let mut first_out = Vec::new();
        handle_workspace_operation(&mut first_out, &first_req, &state, true)
            .expect("first event registration must write a response");
        assert_eq!(response_status(&first_out), 201);

        thread::sleep(Duration::from_millis(1100));

        let second_req = make_http_request("POST", "/v1/workspaces/ws-test/event-contracts", body);
        let mut second_out = Vec::new();
        handle_workspace_operation(&mut second_out, &second_req, &state, true)
            .expect("second event registration must write a response");

        assert_eq!(response_status(&second_out), 200);
        let duplicate = parse_response_body(&second_out);
        assert_eq!(duplicate["registered"], false);
        assert_eq!(duplicate["already_registered"], true);
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
    fn workspace_bundle_registration_registers_all_artifacts_atomically() {
        let state = empty_state();
        let artifact_path = state.registry_root.join("bundle-module.wasm");
        std::fs::write(&artifact_path, b"wasm bytes").expect("artifact must be writable");
        let req = make_http_request(
            "POST",
            "/v1/workspaces/ws-test/bundles",
            valid_bundle_registration_body(
                "test.api.bundle-capability",
                "test.api.bundle-event",
                "test.api.bundle-workflow",
                &artifact_path,
            ),
        );

        let mut out = Vec::new();
        handle_workspace_operation(&mut out, &req, &state, true)
            .expect("bundle registration must write a response");

        assert_eq!(response_status(&out), 201);
        let resp = parse_response_body(&out);
        assert_eq!(resp["api_version"], "v1");
        assert_eq!(resp["registered"], true);
        assert_eq!(resp["already_registered"], false);
        assert_eq!(resp["outcomes"].as_array().map(Vec::len), Some(3));

        let registered = state
            .with_workspace_mut("ws-test", |ws| {
                Ok((
                    ws.runtime
                        .capability_registry()
                        .find_exact(
                            LookupScope::PreferPrivate,
                            "test.api.bundle-capability",
                            "1.0.0",
                        )
                        .is_some(),
                    ws.event_registry
                        .find_exact(LookupScope::PreferPrivate, "test.api.bundle-event", "1.0.0")
                        .is_some(),
                    ws.runtime
                        .workflow_registry()
                        .find_exact(
                            LookupScope::PreferPrivate,
                            "test.api.bundle-workflow",
                            "1.0.0",
                        )
                        .is_some(),
                ))
            })
            .expect("workspace lookup must succeed");
        assert_eq!(registered, (true, true, true));

        let journal = std::fs::read_to_string(persisted_registry_journal_path(
            &state.registry_root,
            "ws-test",
        ))
        .expect("bundle registration must append a journal entry");
        assert_eq!(journal.lines().count(), 1);

        let reloaded = load_persisted_registry(&state.registry_root, "ws-test")
            .expect("persisted bundle must reload");
        assert_eq!(reloaded.registrations.len(), 1);
        assert_eq!(reloaded.events.len(), 1);
        assert_eq!(reloaded.workflows.len(), 1);
    }

    #[test]
    fn workspace_bundle_registration_rejects_invalid_artifact_without_storage() {
        let state = empty_state();
        let missing_artifact = state.registry_root.join("missing-bundle-module.wasm");
        let req = make_http_request(
            "POST",
            "/v1/workspaces/ws-test/bundles",
            valid_bundle_registration_body(
                "test.api.bundle-invalid-capability",
                "test.api.bundle-invalid-event",
                "test.api.bundle-invalid-workflow",
                &missing_artifact,
            ),
        );

        let mut out = Vec::new();
        handle_workspace_operation(&mut out, &req, &state, true)
            .expect("bundle registration must write a response");

        assert_eq!(response_status(&out), 422);
        assert_eq!(response_content_type(&out), "application/problem+json");
        assert_eq!(
            parse_response_body(&out)["traverse_code"],
            "artifact_not_found"
        );

        let registered = state
            .with_workspace_mut("ws-test", |ws| {
                Ok((
                    ws.runtime
                        .capability_registry()
                        .find_exact(
                            LookupScope::PreferPrivate,
                            "test.api.bundle-invalid-capability",
                            "1.0.0",
                        )
                        .is_some(),
                    ws.event_registry
                        .find_exact(
                            LookupScope::PreferPrivate,
                            "test.api.bundle-invalid-event",
                            "1.0.0",
                        )
                        .is_some(),
                    ws.runtime
                        .workflow_registry()
                        .find_exact(
                            LookupScope::PreferPrivate,
                            "test.api.bundle-invalid-workflow",
                            "1.0.0",
                        )
                        .is_some(),
                ))
            })
            .expect("workspace lookup must succeed");
        assert_eq!(registered, (false, false, false));
    }

    #[test]
    fn workspace_bundle_registration_rejects_internal_duplicate_before_storage() {
        let state = empty_state();
        let artifact_path = state.registry_root.join("duplicate-bundle-module.wasm");
        std::fs::write(&artifact_path, b"wasm bytes").expect("artifact must be writable");
        let mut contract = test_contract("test.api.bundle-duplicate", "1.0.0");
        contract.execution.entrypoint.command = artifact_path.to_string_lossy().to_string();
        let req = make_http_request(
            "POST",
            "/v1/workspaces/ws-test/bundles",
            json!({
                "scope": "workspace_persisted",
                "bundle": {
                    "capabilities": [
                        {"registry_scope": "private", "contract": contract.clone()},
                        {"registry_scope": "private", "contract": contract}
                    ]
                }
            })
            .to_string()
            .into_bytes(),
        );

        let mut out = Vec::new();
        handle_workspace_operation(&mut out, &req, &state, true)
            .expect("bundle registration must write a response");

        assert_eq!(response_status(&out), 409);
        assert_eq!(
            parse_response_body(&out)["traverse_code"],
            "duplicate_bundle_artifact"
        );
        let registered = state
            .with_workspace_mut("ws-test", |ws| {
                Ok(ws
                    .runtime
                    .capability_registry()
                    .find_exact(
                        LookupScope::PreferPrivate,
                        "test.api.bundle-duplicate",
                        "1.0.0",
                    )
                    .is_some())
            })
            .expect("workspace lookup must succeed");
        assert!(!registered);
    }

    #[test]
    fn workspace_bundle_registration_conflict_rolls_back_valid_artifacts() {
        let state = empty_state();
        let artifact_path = state.registry_root.join("conflict-bundle-module.wasm");
        std::fs::write(&artifact_path, b"wasm bytes").expect("artifact must be writable");

        let first_req = make_http_request(
            "POST",
            "/v1/workspaces/ws-test/capabilities",
            valid_registration_body("test.api.bundle-conflict", "1.0.0", &artifact_path),
        );
        let mut first_out = Vec::new();
        handle_workspace_operation(&mut first_out, &first_req, &state, true)
            .expect("initial capability registration must write a response");
        assert_eq!(response_status(&first_out), 201);

        let mut changed_contract = test_contract("test.api.bundle-conflict", "1.0.0");
        changed_contract.summary = "changed by bundle".to_string();
        changed_contract.execution.entrypoint.command = artifact_path.to_string_lossy().to_string();
        let req = make_http_request(
            "POST",
            "/v1/workspaces/ws-test/bundles",
            json!({
                "scope": "workspace_persisted",
                "bundle": {
                    "event_contracts": [{
                        "registry_scope": "private",
                        "event_contract": test_event_contract("test.api.bundle-conflict-event", "1.0.0")
                    }],
                    "capabilities": [{
                        "registry_scope": "private",
                        "contract": changed_contract
                    }]
                }
            })
            .to_string()
            .into_bytes(),
        );

        let mut out = Vec::new();
        handle_workspace_operation(&mut out, &req, &state, true)
            .expect("conflicting bundle registration must write a response");

        assert_eq!(response_status(&out), 409);
        assert_eq!(
            parse_response_body(&out)["traverse_code"],
            "registration_conflict"
        );
        let event_registered = state
            .with_workspace_mut("ws-test", |ws| {
                Ok(ws
                    .event_registry
                    .find_exact(
                        LookupScope::PreferPrivate,
                        "test.api.bundle-conflict-event",
                        "1.0.0",
                    )
                    .is_some())
            })
            .expect("workspace lookup must succeed");
        assert!(!event_registered);
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

    #[test]
    fn dev_any_trusts_loopback_and_rfc1918_callers() {
        assert!(is_trusted_dev_caller(
            IpAddr::from([127, 0, 0, 1]),
            "dev-any"
        ));
        assert!(is_trusted_dev_caller(
            IpAddr::from([10, 0, 0, 42]),
            "dev-any"
        ));
        assert!(is_trusted_dev_caller(
            IpAddr::from([172, 16, 0, 42]),
            "dev-any"
        ));
        assert!(is_trusted_dev_caller(
            IpAddr::from([192, 168, 1, 42]),
            "dev-any"
        ));
    }

    #[test]
    fn dev_any_rejects_public_and_ipv6_non_loopback_callers() {
        assert!(!is_trusted_dev_caller(
            IpAddr::from([8, 8, 8, 8]),
            "dev-any"
        ));
        assert!(!is_trusted_dev_caller(
            "2001:4860:4860::8888".parse().expect("valid IP"),
            "dev-any"
        ));
    }

    #[test]
    fn dev_any_public_rejection_returns_problem_details() {
        let mut out = Vec::new();
        let rejected = reject_dev_any_public_caller(&mut out, "dev-any", false)
            .expect("rejection must serialize");

        assert!(rejected);
        assert_eq!(response_status(&out), 403);
        let body = parse_response_body(&out);
        assert_eq!(body["traverse_code"], "dev_any_public_ip_forbidden");
        assert_eq!(
            body["detail"],
            "auth_mode: dev-any does not allow public IPs"
        );
    }

    #[test]
    fn dev_loopback_does_not_trust_lan_callers() {
        assert!(!is_trusted_dev_caller(
            IpAddr::from([192, 168, 1, 42]),
            "dev-loopback"
        ));
    }

    #[test]
    fn unknown_auth_mode_is_never_a_trusted_development_caller() {
        assert!(!is_trusted_dev_caller(
            IpAddr::from([127, 0, 0, 1]),
            "production"
        ));
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

    #[test]
    fn runtime_security_follows_auth_mode() {
        assert_eq!(
            runtime_security_for_auth_mode("dev-loopback"),
            RuntimeSecurityConfig::development()
        );
        assert_eq!(
            runtime_security_for_auth_mode("dev-any"),
            RuntimeSecurityConfig::development()
        );
        assert_eq!(
            runtime_security_for_auth_mode("bearer-required"),
            RuntimeSecurityConfig::production()
        );
    }

    // ------------------------------------------------------------------
    // Connection handling / DoS hardening (spec 033-http-json-api,
    // issue #581 - bounded timeouts + bounded worker pool)
    // ------------------------------------------------------------------

    fn spawn_test_pool(limits: ConnectionLimits, worker_count: usize) -> std::net::SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind must succeed");
        let addr = listener.local_addr().expect("local addr must resolve");
        let state = Arc::new(test_state_with("test.api.do-something", "1.0.0"));
        thread::spawn(move || {
            let _ = run_connection_pool(&listener, &state, limits, worker_count);
        });
        addr
    }

    fn read_all_with_timeout(stream: &mut TcpStream, timeout: Duration) -> Vec<u8> {
        stream
            .set_read_timeout(Some(timeout))
            .expect("client read timeout must be set");
        let mut out = Vec::new();
        let mut chunk = [0_u8; 4096];
        loop {
            match stream.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => out.extend_from_slice(&chunk[..n]),
            }
        }
        out
    }

    #[test]
    fn idle_connection_times_out_without_blocking_other_callers() {
        let limits = ConnectionLimits {
            read_timeout: Duration::from_millis(200),
            write_timeout: Duration::from_millis(200),
            request_deadline: Duration::from_millis(400),
        };
        let addr = spawn_test_pool(limits, 1);

        let mut idle = TcpStream::connect(addr).expect("idle connection must connect");
        idle.write_all(b"GET /healthz HTTP/1.1\r\nHost: x\r\n")
            .expect("partial idle request must write");

        // Give the lone worker a moment to pick up the idle connection first,
        // so the healthy request below is proven to be queued behind it.
        thread::sleep(Duration::from_millis(50));

        let mut healthy = TcpStream::connect(addr).expect("healthy connection must connect");
        healthy
            .write_all(b"GET /healthz HTTP/1.1\r\nHost: x\r\n\r\n")
            .expect("healthy request must write");

        let healthy_response = read_all_with_timeout(&mut healthy, Duration::from_secs(5));
        assert_eq!(
            response_status(&healthy_response),
            200,
            "a concurrent healthy request must not be blocked by an idle connection"
        );

        let idle_response = read_all_with_timeout(&mut idle, Duration::from_secs(5));
        assert_eq!(response_status(&idle_response), 408);
    }

    #[test]
    fn slow_trickle_connection_is_bounded_by_request_deadline() {
        let limits = ConnectionLimits {
            read_timeout: Duration::from_secs(5),
            write_timeout: Duration::from_secs(5),
            request_deadline: Duration::from_millis(300),
        };
        let addr = spawn_test_pool(limits, 1);

        let mut trickle = TcpStream::connect(addr).expect("trickle connection must connect");
        trickle
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("client read timeout must be set");
        let mut writer = trickle
            .try_clone()
            .expect("trickle connection must be cloneable");

        let started = Instant::now();
        // Write on a separate handle to the same socket so a read of the
        // server's response can happen concurrently: once the server closes
        // the connection after the deadline, continuing to write on this
        // thread can trigger a reset that would otherwise race with (and
        // potentially discard) the still-unread response.
        let writer_handle = thread::spawn(move || {
            for byte in b"GET /healthz HTTP/1.1\r\nHost: x\r\n" {
                if writer.write_all(&[*byte]).is_err() {
                    break;
                }
                thread::sleep(Duration::from_millis(60));
            }
        });

        let response = read_all_with_timeout(&mut trickle, Duration::from_secs(5));
        let elapsed = started.elapsed();
        let _ = writer_handle.join();

        assert_eq!(response_status(&response), 408);
        assert!(
            elapsed < Duration::from_secs(2),
            "the request deadline (300ms) must cut off a slow trickle long before \
             the 5s per-read timeout would, but it took {elapsed:?}"
        );
    }

    #[test]
    fn concurrent_healthy_clients_are_all_served() {
        let limits = ConnectionLimits {
            read_timeout: Duration::from_secs(5),
            write_timeout: Duration::from_secs(5),
            request_deadline: Duration::from_secs(5),
        };
        let addr = spawn_test_pool(limits, 4);

        let handles: Vec<_> = (0..8)
            .map(|_| {
                thread::spawn(move || {
                    let mut stream =
                        TcpStream::connect(addr).expect("client connection must connect");
                    stream
                        .write_all(b"GET /healthz HTTP/1.1\r\nHost: x\r\n\r\n")
                        .expect("client request must write");
                    let response = read_all_with_timeout(&mut stream, Duration::from_secs(5));
                    response_status(&response)
                })
            })
            .collect();

        for handle in handles {
            assert_eq!(handle.join().expect("client thread must not panic"), 200);
        }
    }

    #[test]
    fn oversized_body_is_rejected_over_socket() {
        let limits = ConnectionLimits {
            read_timeout: Duration::from_secs(5),
            write_timeout: Duration::from_secs(5),
            request_deadline: Duration::from_secs(5),
        };
        let addr = spawn_test_pool(limits, 1);

        let mut stream = TcpStream::connect(addr).expect("connection must connect");
        let oversized_length = MAX_REQUEST_BODY + 1;
        let request = format!(
            "POST /v1/workspaces/ws-test/execute HTTP/1.1\r\nHost: x\r\nContent-Length: {oversized_length}\r\n\r\n"
        );
        stream
            .write_all(request.as_bytes())
            .expect("request headers must write");

        let response = read_all_with_timeout(&mut stream, Duration::from_secs(5));
        assert_eq!(response_status(&response), 413);
    }

    #[test]
    fn oversized_headers_are_rejected() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind must succeed");
        let addr = listener.local_addr().expect("local addr must resolve");
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept must succeed");
            let deadline = Instant::now() + Duration::from_secs(5);
            read_http_request(&mut stream, deadline).map(|_| ())
        });

        let mut client = TcpStream::connect(addr).expect("client connection must connect");
        let oversized_header_value = "x".repeat(MAX_REQUEST_HEADER_BYTES + 1);
        let request = format!(
            "GET /healthz HTTP/1.1\r\nHost: x\r\nX-Filler: {oversized_header_value}\r\n\r\n"
        );
        client
            .write_all(request.as_bytes())
            .expect("oversized header request must write");

        let result = server.join().expect("server thread must not panic");
        assert_eq!(result, Err("HTTP request headers too large".to_string()));
    }

    #[test]
    fn too_many_headers_are_rejected() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind must succeed");
        let addr = listener.local_addr().expect("local addr must resolve");
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept must succeed");
            let deadline = Instant::now() + Duration::from_secs(5);
            read_http_request(&mut stream, deadline).map(|_| ())
        });

        let mut client = TcpStream::connect(addr).expect("client connection must connect");
        let mut request = String::from("GET /healthz HTTP/1.1\r\nHost: x\r\n");
        for i in 0..(MAX_REQUEST_HEADER_COUNT + 5) {
            let _ = write!(request, "X-Filler-{i}: 1\r\n");
        }
        request.push_str("\r\n");
        client
            .write_all(request.as_bytes())
            .expect("many-header request must write");

        let result = server.join().expect("server thread must not panic");
        assert!(
            matches!(&result, Err(message) if message.contains("too many headers")),
            "expected too-many-headers rejection, got {result:?}"
        );
    }

    // ------------------------------------------------------------------
    // Auth / JWT / workspace-access helper coverage (#637, pass 1)
    // ------------------------------------------------------------------

    #[test]
    fn validate_workspace_id_rejects_invalid_inputs() {
        validate_workspace_id("ok-id_1.2").expect("conservative id must be accepted");
        let empty = validate_workspace_id(" ").expect_err("blank id must be rejected");
        assert!(empty.contains("non-empty"));
        let long = validate_workspace_id(&"a".repeat(129)).expect_err("long id must be rejected");
        assert!(long.contains("128"));
        let nul = validate_workspace_id("a\0b").expect_err("null byte must be rejected");
        assert!(nul.contains("null"));
        let bad = validate_workspace_id("bad/ws").expect_err("path separator must be rejected");
        assert!(bad.contains("ASCII"));
    }

    #[test]
    fn validate_subject_id_rejects_invalid_inputs() {
        validate_subject_id("subject").expect("plain subject must be accepted");
        let empty = validate_subject_id("  ").expect_err("blank subject must be rejected");
        assert!(empty.contains("non-empty"));
        let long =
            validate_subject_id(&"s".repeat(257)).expect_err("long subject must be rejected");
        assert!(long.contains("256"));
        let nul = validate_subject_id("a\0b").expect_err("null byte must be rejected");
        assert!(nul.contains("null"));
    }

    #[test]
    fn resolve_jwt_verification_key_covers_all_modes() {
        let configured = resolve_jwt_verification_key(
            Some(&test_jwt_verifying_key_hex()),
            "bearer-required",
            false,
        )
        .expect("valid key hex must resolve");
        assert!(configured.is_some());

        let invalid = resolve_jwt_verification_key(Some("zz"), "bearer-required", false);
        assert!(matches!(invalid, Err(ServeError::BindFailed(_))));

        let fail_closed = resolve_jwt_verification_key(None, "bearer-required", false)
            .expect("missing key must resolve to None (fail closed at request time)");
        assert!(fail_closed.is_none());

        let dev = resolve_jwt_verification_key(None, "dev-loopback", true)
            .expect("dev mode without key must resolve to None");
        assert!(dev.is_none());
    }

    #[test]
    fn subject_from_request_rejects_oversized_opaque_subject() {
        let mut headers = HashMap::new();
        headers.insert(
            "authorization".to_string(),
            format!("Bearer {}", "s".repeat(300)),
        );
        let err = subject_from_request(&headers, "dev-loopback", false, true, None)
            .expect_err("oversized opaque subject must be rejected");
        assert_eq!(err.status, 401);
        assert!(err.message.contains("256"));
    }

    #[test]
    fn derive_identity_from_jwt_rejects_malformed_tokens() {
        let malformed = derive_identity_from_jwt("a.b", "dev-loopback", None)
            .expect_err("two-part token must be rejected");
        assert!(malformed.message.contains("malformed"));

        let header = base64url_encode(br#"{"alg":"EdDSA","typ":"JWT"}"#);

        let bad_b64 = derive_identity_from_jwt(&format!("{header}.!!!.sig"), "dev-loopback", None)
            .expect_err("non-base64url payload must be rejected");
        assert!(bad_b64.message.contains("invalid characters"));

        let not_json = base64url_encode(b"not-json");
        let bad_json =
            derive_identity_from_jwt(&format!("{header}.{not_json}.sig"), "dev-loopback", None)
                .expect_err("non-JSON payload must be rejected");
        assert!(bad_json.message.contains("invalid JWT payload"));

        let no_sub = base64url_encode(b"{}");
        let missing_sub =
            derive_identity_from_jwt(&format!("{header}.{no_sub}.sig"), "dev-loopback", None)
                .expect_err("payload without sub must be rejected");
        assert!(missing_sub.message.contains("'sub'"));

        let oversized = base64url_encode(json!({ "sub": "s".repeat(257) }).to_string().as_bytes());
        let bad_sub =
            derive_identity_from_jwt(&format!("{header}.{oversized}.sig"), "dev-loopback", None)
                .expect_err("oversized sub claim must be rejected");
        assert!(bad_sub.message.contains("256"));
    }

    #[test]
    fn jwt_claims_admin_recognizes_admin_claims() {
        assert!(jwt_claims_admin(&json!({ "traverse_admin": true })));
        assert!(jwt_claims_admin(
            &json!({ "roles": ["viewer", "traverse_admin"] })
        ));
        assert!(jwt_claims_admin(
            &json!({ "roles": [SYSTEM_ADMIN_SUBJECT] })
        ));
        assert!(jwt_claims_admin(&json!({ "role": "traverse_admin" })));
        assert!(jwt_claims_admin(&json!({ "role": SYSTEM_ADMIN_SUBJECT })));
        assert!(!jwt_claims_admin(
            &json!({ "roles": ["viewer"], "role": "viewer" })
        ));
        assert!(!jwt_claims_admin(&json!({})));
    }

    #[test]
    fn validate_jwt_time_claims_allows_absent_claims() {
        validate_jwt_time_claims(&json!({})).expect("token without time claims must be valid");
    }

    #[test]
    fn parse_jwt_scopes_merges_all_scope_claims() {
        let scopes = parse_jwt_scopes(&json!({
            "scope": "read write",
            "scp": ["admin", "  ", "read"],
            "scopes": ["write", "extra"]
        }));
        assert_eq!(scopes, vec!["admin", "extra", "read", "write"]);
    }

    #[test]
    fn hex_decode_rejects_odd_length_input() {
        let err = hex_decode("abc").expect_err("odd-length hex must be rejected");
        assert!(err.contains("even number"));
    }

    #[test]
    fn base64url_decode_handles_edge_cases() {
        assert!(
            base64url_decode("")
                .expect("empty input must decode to empty output")
                .is_empty()
        );
        let padded = base64url_decode("aa==").expect_err("padding must be rejected");
        assert!(padded.contains("padding"));
        let invalid = base64url_decode("a!").expect_err("invalid character must be rejected");
        assert!(invalid.contains("invalid characters"));
        let length = base64url_decode("aaaaa").expect_err("length % 4 == 1 must be rejected");
        assert!(length.contains("invalid length"));
    }

    #[test]
    fn workspace_metadata_io_failures_surface_errors() {
        let metadata = WorkspaceMetadataV1 {
            schema_version: WORKSPACE_METADATA_SCHEMA_VERSION.to_string(),
            workspace_id: "ws".to_string(),
            owner_subject: "owner".to_string(),
            shared: false,
            members: Vec::new(),
        };

        let read_root = test_registry_root();
        std::fs::create_dir_all(workspace_metadata_path(&read_root, "ws"))
            .expect("directory squatting on the metadata path must be creatable");
        let read_err = load_workspace_metadata(&read_root, "ws")
            .expect_err("reading a directory as metadata must fail");
        assert_eq!(read_err.code, "workspace_metadata_read_failed");

        let parse_root = test_registry_root();
        let parse_path = workspace_metadata_path(&parse_root, "ws");
        std::fs::create_dir_all(
            parse_path
                .parent()
                .expect("metadata path must have a parent"),
        )
        .expect("workspace directory must be creatable");
        std::fs::write(&parse_path, b"not-json").expect("corrupt metadata must be writable");
        let parse_err = load_workspace_metadata(&parse_root, "ws")
            .expect_err("corrupt metadata must fail to parse");
        assert_eq!(parse_err.code, "workspace_metadata_parse_failed");

        let blocked_root = test_registry_root();
        std::fs::create_dir_all(&blocked_root).expect("registry root must be creatable");
        std::fs::write(blocked_root.join("workspaces"), b"file")
            .expect("file squatting on the workspaces directory must be writable");
        let dir_err = persist_workspace_metadata(&blocked_root, "ws", &metadata)
            .expect_err("directory creation over a file must fail");
        assert_eq!(dir_err.code, "workspace_metadata_write_failed");
        assert!(dir_err.message.contains("create workspace directory"));

        let tmp_root = test_registry_root();
        std::fs::create_dir_all(
            workspace_metadata_path(&tmp_root, "ws").with_extension("json.tmp"),
        )
        .expect("directory squatting on the temp path must be creatable");
        let tmp_err = persist_workspace_metadata(&tmp_root, "ws", &metadata)
            .expect_err("writing the temp file over a directory must fail");
        assert_eq!(tmp_err.code, "workspace_metadata_write_failed");
        assert!(tmp_err.message.contains("temp file"));

        let rename_root = test_registry_root();
        std::fs::create_dir_all(workspace_metadata_path(&rename_root, "ws"))
            .expect("directory squatting on the final path must be creatable");
        let rename_err = persist_workspace_metadata(&rename_root, "ws", &metadata)
            .expect_err("renaming over a directory must fail");
        assert_eq!(rename_err.code, "workspace_metadata_write_failed");
        assert!(rename_err.message.contains("atomically replace"));
    }

    #[test]
    fn ensure_workspace_access_enforces_membership_rules() {
        let root = test_registry_root();
        let owner = DerivedIdentity {
            subject_id: "owner".to_string(),
            is_admin: false,
            scopes: Vec::new(),
        };

        let invalid = ensure_workspace_access(&root, "bad/ws", &owner)
            .expect_err("invalid workspace id must be rejected");
        assert_eq!(invalid.code, "workspace_id_invalid");

        let shared = WorkspaceMetadataV1 {
            schema_version: WORKSPACE_METADATA_SCHEMA_VERSION.to_string(),
            workspace_id: "shared-ws".to_string(),
            owner_subject: "owner".to_string(),
            shared: true,
            members: vec!["member".to_string()],
        };
        persist_workspace_metadata(&root, "shared-ws", &shared)
            .expect("shared workspace metadata must persist");

        let member = DerivedIdentity {
            subject_id: "member".to_string(),
            is_admin: false,
            scopes: Vec::new(),
        };
        ensure_workspace_access(&root, "shared-ws", &member)
            .expect("shared workspace member must be authorized");

        let stranger = DerivedIdentity {
            subject_id: "stranger".to_string(),
            is_admin: false,
            scopes: Vec::new(),
        };
        let denied = ensure_workspace_access(&root, "shared-ws", &stranger)
            .expect_err("non-member must be rejected from shared workspace");
        assert_eq!(denied.code, "unauthorized_workspace");
    }

    #[test]
    fn parse_registration_scope_parses_all_variants() {
        assert!(matches!(
            parse_registration_scope(None),
            Ok(RegistrationScope::WorkspacePersisted)
        ));
        assert!(matches!(
            parse_registration_scope(Some(&json!("workspace_persisted"))),
            Ok(RegistrationScope::WorkspacePersisted)
        ));
        assert!(matches!(
            parse_registration_scope(Some(&json!("session_ephemeral"))),
            Ok(RegistrationScope::SessionEphemeral)
        ));
        let non_string = parse_registration_scope(Some(&json!(42)))
            .expect_err("non-string scope must be rejected");
        assert!(non_string.contains("must be a string"));
        let unknown = parse_registration_scope(Some(&json!("other")))
            .expect_err("unknown scope must be rejected");
        assert!(unknown.contains("workspace_persisted or session_ephemeral"));
    }

    // ------------------------------------------------------------------
    // Persisted-registry / journal / audit helper coverage (#637, pass 2)
    // ------------------------------------------------------------------

    #[test]
    fn parse_http_json_response_covers_malformed_responses() {
        let (status, body) = parse_http_json_response(
            b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n\r\n{\"ok\":true}",
        )
        .expect("well-formed response must parse");
        assert_eq!(status, 200);
        assert_eq!(body["ok"], json!(true));

        let not_utf8 = parse_http_json_response(&[0xff, 0xfe])
            .expect_err("non-UTF-8 response must be rejected");
        assert!(not_utf8.contains("not UTF-8"));

        let empty = parse_http_json_response(b"").expect_err("empty response must be rejected");
        assert!(empty.contains("missing status line"));

        let no_proto = parse_http_json_response(b"\r\n\r\n{}")
            .expect_err("blank status line must be rejected");
        assert!(no_proto.contains("missing protocol"));

        let no_status = parse_http_json_response(b"HTTP/1.1\r\n\r\n{}")
            .expect_err("status line without code must be rejected");
        assert!(no_status.contains("missing status code"));

        let bad_status = parse_http_json_response(b"HTTP/1.1 abc\r\n\r\n{}")
            .expect_err("non-numeric status must be rejected");
        assert!(bad_status.contains("not a u16"));

        let no_terminator = parse_http_json_response(b"HTTP/1.1 200 OK")
            .expect_err("response without header terminator must be rejected");
        assert!(no_terminator.contains("missing header terminator"));

        let bad_body = parse_http_json_response(b"HTTP/1.1 200 OK\r\n\r\nnot-json")
            .expect_err("non-JSON body must be rejected");
        assert!(bad_body.contains("invalid JSON response body"));
    }

    #[test]
    fn load_persisted_registry_surfaces_read_and_parse_failures() {
        let dir_root = test_registry_root();
        std::fs::create_dir_all(persisted_registry_path(&dir_root, "ws"))
            .expect("directory squatting on the registry path must be creatable");
        let read_err = load_persisted_registry(&dir_root, "ws")
            .expect_err("reading a directory as a persisted registry must fail");
        assert!(read_err.contains("failed to read persisted registry"));

        let parse_root = test_registry_root();
        let parse_path = persisted_registry_path(&parse_root, "ws");
        std::fs::create_dir_all(
            parse_path
                .parent()
                .expect("registry path must have a parent"),
        )
        .expect("workspace directory must be creatable");
        std::fs::write(&parse_path, b"not-json").expect("corrupt registry must be writable");
        let parse_err = load_persisted_registry(&parse_root, "ws")
            .expect_err("corrupt persisted registry must fail to parse");
        assert!(parse_err.contains("failed to parse persisted registry"));
    }

    #[test]
    fn registry_journal_replay_handles_partial_and_corrupt_lines() {
        let root = test_registry_root();
        let journal = persisted_registry_journal_path(&root, "ws");
        std::fs::create_dir_all(journal.parent().expect("journal path must have a parent"))
            .expect("workspace directory must be creatable");

        std::fs::write(&journal, b"{}\nnot-json")
            .expect("journal with truncated final line must be writable");
        let tolerated = load_persisted_registry(&root, "ws")
            .expect("a corrupt final journal line must be tolerated as a torn write");
        assert!(tolerated.registrations.is_empty());

        std::fs::write(&journal, b"not-json\n{}\n")
            .expect("journal with corrupt interior line must be writable");
        let corrupt = load_persisted_registry(&root, "ws")
            .expect_err("a corrupt interior journal line must fail loudly");
        assert!(corrupt.contains("failed to parse persisted registry journal"));

        let dir_journal_root = test_registry_root();
        std::fs::create_dir_all(persisted_registry_journal_path(&dir_journal_root, "ws"))
            .expect("directory squatting on the journal path must be creatable");
        let read_err = load_persisted_registry(&dir_journal_root, "ws")
            .expect_err("reading a directory as the journal must fail");
        assert!(read_err.contains("failed to read persisted registry journal"));
    }

    #[test]
    fn append_registry_mutation_and_audit_surface_directory_failures() {
        let mutation = PersistedRegistryMutationV1 {
            registrations: Vec::new(),
            events: Vec::new(),
            workflows: Vec::new(),
        };

        let blocked_root = test_registry_root();
        std::fs::create_dir_all(&blocked_root).expect("registry root must be creatable");
        std::fs::write(blocked_root.join("workspaces"), b"file")
            .expect("file squatting on the workspaces directory must be writable");
        let mutation_err = append_registry_mutation(&blocked_root, "ws", &mutation)
            .expect_err("journal directory creation over a file must fail");
        assert!(mutation_err.contains("failed to create persisted registry directory"));

        let mut state = test_state_with("content.comments.create-comment-draft", "1.0.0");
        state.registry_root = blocked_root;
        let audit_err = audit_workspace_event(&state, "ws", "test.event", None, None, "ok", None)
            .expect_err("audit directory creation over a file must fail");
        assert!(audit_err.contains("failed to create audit log directory"));
    }
}
