//! Wasmtime-backed WASM executor.
//!
//! Executes `wasm32-wasi` capability binaries inside a sandboxed Wasmtime engine.
//! Input is fed via WASI stdin; output is captured from WASI stdout.
//! No ambient WASI authority is granted — all capabilities are deny-by-default.

use chrono::Utc;
use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};
use std::fmt::Write as _;
use std::fs;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::SystemTime;
use uuid::Uuid;
use wasmtime::{
    Caller, Config, Engine, Extern, Linker, Module, Store, StoreLimits, StoreLimitsBuilder,
};
use wasmtime_wasi::WasiCtxBuilder;
use wasmtime_wasi::p1::WasiP1Ctx;
use wasmtime_wasi::p2::pipe::{MemoryInputPipe, MemoryOutputPipe};

use super::{
    ArtifactType, CapabilityExecutor, ConnectorInvocationEvidence, ExecutorCapability,
    ExecutorError, ExecutorOutput,
};
use crate::events::types::{LifecycleStatus, TraverseEvent};
use traverse_contracts::{ConnectorRequirement, EventReference, ServiceType};

/// Traverse Host ABI v1 is independently versioned from the runtime crate.
pub const SUPPORTED_HOST_ABI_VERSION: &str = "1.0.0";

const HOST_ABI_V1_WHITELIST: &str = include_str!("host_abi_v1.json");
const DEFAULT_FUEL_BUDGET: u64 = 5_000_000;
const DEFAULT_MEMORY_LIMIT_BYTES: usize = 8 * 1024 * 1024;
const DEFAULT_TABLE_ELEMENT_LIMIT: usize = 1_024;
const DEFAULT_INSTANCE_LIMIT: usize = 1;
const DEFAULT_TABLE_LIMIT: usize = 8;
const DEFAULT_LINEAR_MEMORY_LIMIT: usize = 1;
const DEFAULT_MODULE_CACHE_MAX_ENTRIES: usize = 64;

/// Maximum bytes accepted for one `traverse_host::emit_event` payload
/// (spec 098-capability-event-host-abi FR-008). Enforced before the guest
/// memory read, and before deserialization.
const MAX_EVENT_EMIT_PAYLOAD_BYTES: usize = 64 * 1024;

/// `traverse_host::emit_event` accepted the event; it will be published to
/// `EventBroker` once execution completes (spec 098 acceptance scenario 1).
const EMIT_EVENT_OK: i32 = 0;
/// The guest-supplied pointer/length was out of the guest's linear memory
/// bounds, or the payload exceeded [`MAX_EVENT_EMIT_PAYLOAD_BYTES`], or the
/// bytes were not a valid JSON object with `event_id`/`version` string
/// fields (spec 098 FR-008, acceptance scenario 5).
const EMIT_EVENT_ERR_INVALID_PAYLOAD: i32 = -1;
/// The event type/version is not declared in the calling capability's
/// contract `emits` list (spec 098 FR-002, acceptance scenario 2).
const EMIT_EVENT_ERR_UNDECLARED_EVENT: i32 = -2;
/// The calling capability's `service_type` is not `Subscribable` (spec 098
/// FR-003, acceptance scenario 3).
const EMIT_EVENT_ERR_NOT_SUBSCRIBABLE: i32 = -3;

/// `traverse_host::connector_invoke` is unavailable unless the embedding host
/// supplies an activated, capability-authorized connector binding. The default
/// WASM executor deliberately returns this stable failure rather than granting
/// any ambient authority (Spec 104 FR-002/FR-007).
const CONNECTOR_INVOKE_ERR_UNBOUND: i32 = -2;
const CONNECTOR_INVOKE_ERR_INVALID_REQUEST: i32 = -1;
const CONNECTOR_INVOKE_ERR_UNDECLARED: i32 = -3;
const CONNECTOR_INVOKE_ERR_UNAUTHORIZED: i32 = -4;
const CONNECTOR_INVOKE_ERR_PAYLOAD_TOO_LARGE: i32 = -5;
const CONNECTOR_INVOKE_ERR_EXECUTION_FAILED: i32 = -6;
const MAX_CONNECTOR_INVOKE_REQUEST_BYTES: usize = 64 * 1024;
const MAX_CONNECTOR_INVOKE_RESPONSE_BYTES: usize = 64 * 1024;

/// Versioned guest request accepted by `traverse_host::connector_invoke`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConnectorInvokeRequest {
    pub abi_version: String,
    pub connector_id: String,
    pub operation: String,
    pub payload: Value,
}

/// Non-secret response returned to the guest by a mediated connector.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConnectorInvokeResponse {
    pub abi_version: String,
    pub result_class: String,
    pub payload: Value,
}

/// A host-owned activated connector. Its implementation is never visible to a guest.
pub trait MediatedConnector: Send + Sync {
    /// # Errors
    ///
    /// Returns a stable, non-secret failure description when the host-owned
    /// connector cannot complete the requested operation.
    fn invoke(&self, request: &ConnectorInvokeRequest) -> Result<ConnectorInvokeResponse, String>;
}

/// The host-owned authorization context for one WASM execution.
#[derive(Clone)]
pub struct MediatedConnectorContext {
    pub declared_requirements: Vec<ConnectorRequirement>,
    pub activated_connectors: Vec<ActivatedConnector>,
}

#[derive(Clone)]
pub struct ActivatedConnector {
    pub connector_id: String,
    pub version: String,
    pub implementation: Arc<dyn MediatedConnector>,
}

static HOST_ABI_V1_WHITELIST_CACHE: LazyLock<Result<HostAbiWhitelist, String>> =
    LazyLock::new(|| {
        serde_json::from_str::<HostAbiWhitelist>(HOST_ABI_V1_WHITELIST).map_err(|e| e.to_string())
    });

/// A host import observed in a WASM module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostAbiImport {
    /// Imported module namespace.
    pub module: String,
    /// Imported function or item name.
    pub name: String,
}

/// Successful load-time ABI validation evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostAbiValidation {
    /// ABI version used for whitelist validation.
    pub abi_version: String,
    /// All imports observed in deterministic module/name order.
    pub imports: Vec<HostAbiImport>,
}

#[derive(Debug, Clone, Deserialize)]
struct HostAbiWhitelist {
    abi_version: String,
    imports: Vec<HostAbiWhitelistImport>,
}

#[derive(Debug, Clone, Deserialize)]
struct HostAbiWhitelistImport {
    module: String,
    name: String,
}

/// Return the Traverse Host ABI versions supported by this runtime.
#[must_use]
pub fn supported_host_abi_versions() -> &'static [&'static str] {
    &[SUPPORTED_HOST_ABI_VERSION]
}

/// Validate a WASM binary against the declared Traverse Host ABI import whitelist.
///
/// # Errors
///
/// Returns [`ExecutorError`] when the binary is malformed, the ABI version is unsupported,
/// or a module imports a host function outside the whitelist.
pub fn verify_wasm_host_abi_bytes(
    wasm_bytes: &[u8],
    abi_version: &str,
) -> Result<HostAbiValidation, ExecutorError> {
    let engine = Engine::default();
    let module = Module::from_binary(&engine, wasm_bytes).map_err(|e| {
        ExecutorError::MalformedWasmArtifact {
            error_code: "malformed_wasm_artifact".to_string(),
            detail: format!("module compile: {e}"),
        }
    })?;
    validate_module_imports(&module, abi_version)
}

/// Executes `.wasm32-wasi` capability binaries via Wasmtime.
///
/// Every invocation creates a fresh Wasmtime `Store` — no state leaks between calls.
#[derive(Debug)]
pub struct WasmExecutor {
    engine: Engine,
    limits: WasmExecutionLimits,
    module_cache: Mutex<CompiledModuleCache>,
    binary_cache: Mutex<LoadedBinaryCache>,
}

impl WasmExecutor {
    /// Create a new [`WasmExecutor`] with a default Wasmtime engine.
    ///
    /// # Errors
    ///
    /// Returns [`ExecutorError::RuntimeSetupFailed`] if Wasmtime cannot initialise.
    pub fn new() -> Result<Self, ExecutorError> {
        Self::with_limits(WasmExecutionLimits::default())
    }

    /// Create a [`WasmExecutor`] with explicit per-invocation resource limits.
    ///
    /// # Errors
    ///
    /// Returns [`ExecutorError::RuntimeSetupFailed`] if Wasmtime cannot initialise.
    pub fn with_limits(limits: WasmExecutionLimits) -> Result<Self, ExecutorError> {
        Self::with_limits_and_cache_config(limits, WasmModuleCacheConfig::default())
    }

    /// Create a [`WasmExecutor`] with explicit resource limits and module cache bounds.
    ///
    /// # Errors
    ///
    /// Returns [`ExecutorError::RuntimeSetupFailed`] if Wasmtime cannot initialise.
    pub fn with_limits_and_cache_config(
        limits: WasmExecutionLimits,
        cache_config: WasmModuleCacheConfig,
    ) -> Result<Self, ExecutorError> {
        let mut config = Config::new();
        config.consume_fuel(true);
        let engine = Engine::new(&config)
            .map_err(|e| ExecutorError::RuntimeSetupFailed(format!("engine config: {e}")))?;
        Ok(Self {
            engine,
            limits,
            module_cache: Mutex::new(CompiledModuleCache::new(cache_config.max_entries)),
            binary_cache: Mutex::new(LoadedBinaryCache::new(cache_config.max_entries)),
        })
    }

    /// Return current compiled-module cache counters.
    #[must_use]
    pub fn module_cache_stats(&self) -> WasmModuleCacheStats {
        let cache = self
            .module_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        cache.stats()
    }

    /// Return current on-disk binary cache counters.
    #[must_use]
    pub fn binary_cache_stats(&self) -> WasmBinaryCacheStats {
        let cache = self
            .binary_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        cache.stats()
    }
}

/// Per-invocation resource limits for [`WasmExecutor`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WasmExecutionLimits {
    /// Fuel units available for guest code before it traps as a timeout.
    pub fuel_budget: u64,
    /// Maximum bytes for each guest linear memory.
    pub memory_bytes: usize,
    /// Maximum elements for each guest table.
    pub table_elements: usize,
    /// Maximum instances in the store.
    pub instances: usize,
    /// Maximum tables in the store.
    pub tables: usize,
    /// Maximum linear memories in the store.
    pub memories: usize,
}

impl Default for WasmExecutionLimits {
    fn default() -> Self {
        Self {
            fuel_budget: DEFAULT_FUEL_BUDGET,
            memory_bytes: DEFAULT_MEMORY_LIMIT_BYTES,
            table_elements: DEFAULT_TABLE_ELEMENT_LIMIT,
            instances: DEFAULT_INSTANCE_LIMIT,
            tables: DEFAULT_TABLE_LIMIT,
            memories: DEFAULT_LINEAR_MEMORY_LIMIT,
        }
    }
}

/// Bounded compiled-module cache configuration for [`WasmExecutor`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WasmModuleCacheConfig {
    /// Maximum number of compiled modules retained by checksum.
    pub max_entries: usize,
}

impl Default for WasmModuleCacheConfig {
    fn default() -> Self {
        Self {
            max_entries: DEFAULT_MODULE_CACHE_MAX_ENTRIES,
        }
    }
}

/// Snapshot of compiled-module cache counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WasmModuleCacheStats {
    /// Current retained compiled modules.
    pub entries: usize,
    /// Number of executions served from cache.
    pub hits: u64,
    /// Number of executions that compiled a module before insertion.
    pub misses: u64,
    /// Number of deterministic oldest-entry evictions.
    pub evictions: u64,
}

/// Snapshot of on-disk WASM binary cache counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WasmBinaryCacheStats {
    /// Current retained binary entries.
    pub entries: usize,
    /// Number of executions served without a binary read or hash.
    pub hits: u64,
    /// Number of executions that required a binary read.
    pub loads: u64,
    /// Number of SHA-256 computations required to load binaries.
    pub hashes: u64,
    /// Number of deterministic oldest-entry evictions.
    pub evictions: u64,
}

#[derive(Debug, Clone)]
struct CachedModule {
    module: Module,
    validation: HostAbiValidation,
}

#[derive(Debug)]
struct CompiledModuleCache {
    max_entries: usize,
    entries: HashMap<String, CachedModule>,
    insertion_order: VecDeque<String>,
    hits: u64,
    misses: u64,
    evictions: u64,
}

impl CompiledModuleCache {
    fn new(max_entries: usize) -> Self {
        Self {
            max_entries: max_entries.max(1),
            entries: HashMap::new(),
            insertion_order: VecDeque::new(),
            hits: 0,
            misses: 0,
            evictions: 0,
        }
    }

    fn get(&mut self, checksum: &str, abi_version: &str) -> Option<CachedModule> {
        let cached = self.entries.get(checksum)?;
        if cached.validation.abi_version != abi_version {
            self.misses += 1;
            return None;
        }
        self.hits += 1;
        Some(cached.clone())
    }

    fn insert(&mut self, checksum: String, cached: CachedModule) {
        self.misses += 1;
        while self.entries.len() >= self.max_entries {
            if let Some(oldest) = self.insertion_order.pop_front()
                && self.entries.remove(&oldest).is_some()
            {
                self.evictions += 1;
            }
        }
        self.insertion_order.push_back(checksum.clone());
        self.entries.insert(checksum, cached);
    }

    fn stats(&self) -> WasmModuleCacheStats {
        WasmModuleCacheStats {
            entries: self.entries.len(),
            hits: self.hits,
            misses: self.misses,
            evictions: self.evictions,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BinaryFileIdentity {
    len: u64,
    modified: SystemTime,
}

#[derive(Debug, Clone)]
struct CachedBinary {
    identity: BinaryFileIdentity,
    bytes: Arc<[u8]>,
    checksum: String,
}

#[derive(Debug)]
struct LoadedBinaryCache {
    max_entries: usize,
    entries: HashMap<String, CachedBinary>,
    insertion_order: VecDeque<String>,
    hits: u64,
    loads: u64,
    hashes: u64,
    evictions: u64,
}

impl LoadedBinaryCache {
    fn new(max_entries: usize) -> Self {
        Self {
            max_entries: max_entries.max(1),
            entries: HashMap::new(),
            insertion_order: VecDeque::new(),
            hits: 0,
            loads: 0,
            hashes: 0,
            evictions: 0,
        }
    }

    fn get(&mut self, path: &str, identity: &BinaryFileIdentity) -> Option<CachedBinary> {
        let cached = self.entries.get(path)?;
        if cached.identity != *identity {
            return None;
        }
        self.hits += 1;
        Some(cached.clone())
    }

    fn insert(&mut self, path: String, cached: CachedBinary) {
        if let std::collections::hash_map::Entry::Occupied(mut entry) =
            self.entries.entry(path.clone())
        {
            entry.insert(cached);
            return;
        }
        while self.entries.len() >= self.max_entries {
            if let Some(oldest) = self.insertion_order.pop_front()
                && self.entries.remove(&oldest).is_some()
            {
                self.evictions += 1;
            }
        }
        self.insertion_order.push_back(path.clone());
        self.entries.insert(path, cached);
    }

    fn record_load(&mut self) {
        self.loads += 1;
        self.hashes += 1;
    }

    fn stats(&self) -> WasmBinaryCacheStats {
        WasmBinaryCacheStats {
            entries: self.entries.len(),
            hits: self.hits,
            loads: self.loads,
            hashes: self.hashes,
            evictions: self.evictions,
        }
    }
}

struct WasmStoreState {
    wasi: WasiP1Ctx,
    limits: StoreLimits,
    /// Calling capability's id, `emits`, and `service_type` — used by the
    /// `traverse_host::emit_event` host function to validate emissions
    /// synchronously, at call time (spec 098-capability-event-host-abi
    /// FR-002/FR-003).
    capability_id: String,
    emits: Vec<EventReference>,
    service_type: ServiceType,
    /// Events accepted via `traverse_host::emit_event` during this call.
    emitted_events: Vec<TraverseEvent>,
    connector_context: Option<MediatedConnectorContext>,
    connector_invocation_evidence: Vec<ConnectorInvocationEvidence>,
}

impl CapabilityExecutor for WasmExecutor {
    fn execute(
        &self,
        capability: &ExecutorCapability,
        input: &Value,
    ) -> Result<ExecutorOutput, ExecutorError> {
        if capability.artifact_type != ArtifactType::Wasm {
            return Err(ExecutorError::UnsupportedArtifactType);
        }

        // --- Load binary ---
        let wasm_path = capability.wasm_binary_path.as_deref().ok_or_else(|| {
            ExecutorError::BinaryLoadFailed("no wasm_binary_path set".to_string())
        })?;

        let binary = self.load_binary(wasm_path)?;

        // --- Checksum validation ---
        if let Some(expected) = capability.wasm_checksum.as_deref()
            && binary.checksum != expected
        {
            return Err(ExecutorError::ChecksumMismatch {
                expected: expected.to_string(),
                actual: binary.checksum.clone(),
            });
        }

        let abi_version = capability
            .host_abi_version
            .as_deref()
            .unwrap_or(SUPPORTED_HOST_ABI_VERSION);

        self.run_wasm_with_connectors(
            &binary.bytes,
            input,
            abi_version,
            &capability.capability_id,
            &capability.emits,
            capability.service_type.clone(),
            None,
            Some(&binary.checksum),
        )
    }
}

impl WasmExecutor {
    /// Execute pre-loaded WASM bytes with the given input.
    ///
    /// Exposed separately so tests can pass raw bytes without needing a file on disk.
    /// The capability is treated as `Stateless` with no declared `emits` — it
    /// cannot call `traverse_host::emit_event`. Use
    /// [`run_bytes_with_capability`](Self::run_bytes_with_capability) to
    /// exercise the event-emit host function.
    ///
    /// # Errors
    ///
    /// Returns [`ExecutorError`] if input serialization fails, the WASM module cannot be
    /// compiled or linked, execution fails, or stdout is not valid JSON.
    pub fn run_bytes(&self, wasm_bytes: &[u8], input: &Value) -> Result<Value, ExecutorError> {
        self.run_bytes_with_host_abi(wasm_bytes, input, SUPPORTED_HOST_ABI_VERSION)
    }

    /// Execute pre-loaded WASM bytes with an explicit Traverse Host ABI version.
    ///
    /// # Errors
    ///
    /// Returns [`ExecutorError`] if ABI validation fails or execution cannot complete.
    pub fn run_bytes_with_host_abi(
        &self,
        wasm_bytes: &[u8],
        input: &Value,
        abi_version: &str,
    ) -> Result<Value, ExecutorError> {
        self.run_wasm(
            wasm_bytes,
            input,
            abi_version,
            "test-capability",
            &[],
            ServiceType::Stateless,
        )
        .map(|output| output.value)
    }

    /// Execute pre-loaded WASM bytes as a specific capability, exercising
    /// `traverse_host::emit_event` validation against `emits`/`service_type`
    /// exactly as [`CapabilityExecutor::execute`] does.
    ///
    /// # Errors
    ///
    /// Returns [`ExecutorError`] if ABI validation fails or execution cannot complete.
    pub fn run_bytes_with_capability(
        &self,
        wasm_bytes: &[u8],
        input: &Value,
        capability_id: &str,
        emits: &[EventReference],
        service_type: ServiceType,
    ) -> Result<ExecutorOutput, ExecutorError> {
        self.run_wasm(
            wasm_bytes,
            input,
            SUPPORTED_HOST_ABI_VERSION,
            capability_id,
            emits,
            service_type,
        )
    }

    /// Execute bytes with host-owned, activated connector bindings. This is the
    /// only API that can enable `connector_invoke`; callers that do not supply
    /// this context retain the deny-by-default handler.
    ///
    /// # Errors
    ///
    /// Returns [`ExecutorError`] when the module, guest memory, or execution
    /// cannot be completed safely.
    pub fn run_bytes_with_mediated_connectors(
        &self,
        wasm_bytes: &[u8],
        input: &Value,
        capability_id: &str,
        connector_context: MediatedConnectorContext,
    ) -> Result<ExecutorOutput, ExecutorError> {
        self.run_wasm_with_connectors(
            wasm_bytes,
            input,
            SUPPORTED_HOST_ABI_VERSION,
            capability_id,
            &[],
            ServiceType::Stateless,
            Some(connector_context),
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn run_wasm(
        &self,
        wasm_bytes: &[u8],
        input: &Value,
        abi_version: &str,
        capability_id: &str,
        emits: &[EventReference],
        service_type: ServiceType,
    ) -> Result<ExecutorOutput, ExecutorError> {
        self.run_wasm_with_connectors(
            wasm_bytes,
            input,
            abi_version,
            capability_id,
            emits,
            service_type,
            None,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn run_wasm_with_connectors(
        &self,
        wasm_bytes: &[u8],
        input: &Value,
        abi_version: &str,
        capability_id: &str,
        emits: &[EventReference],
        service_type: ServiceType,
        connector_context: Option<MediatedConnectorContext>,
        checksum: Option<&str>,
    ) -> Result<ExecutorOutput, ExecutorError> {
        let input_json = serde_json::to_string(input)
            .map_err(|e| ExecutorError::ExecutionFailed(format!("input serialization: {e}")))?;

        let checksum = checksum.map_or_else(|| sha256_hex(wasm_bytes), str::to_string);
        let cached_module = self.compiled_module(wasm_bytes, &checksum, abi_version)?;

        // Clone pipe reference before passing to builder — needed to read output after execution
        let stdout_pipe = MemoryOutputPipe::new(65536);
        let stdout_ref = stdout_pipe.clone();

        // Build a WASI context: stdin = input JSON, stdout = captured buffer
        // No filesystem, no network, no env vars — deny-by-default
        let wasi_ctx: WasiP1Ctx = WasiCtxBuilder::new()
            .stdin(MemoryInputPipe::new(input_json.into_bytes()))
            .stdout(stdout_pipe)
            .build_p1();

        let mut linker: Linker<WasmStoreState> = Linker::new(&self.engine);
        wasmtime_wasi::p1::add_to_linker_sync(&mut linker, |s| &mut s.wasi)
            .map_err(|e| ExecutorError::RuntimeSetupFailed(e.to_string()))?;
        linker
            .func_wrap("traverse_host", "emit_event", handle_emit_event)
            .map_err(|e| ExecutorError::RuntimeSetupFailed(format!("func_wrap emit_event: {e}")))?;
        #[allow(clippy::expect_used)]
        linker
            .func_wrap("traverse_host", "connector_invoke", handle_connector_invoke)
            .expect("connector_invoke host function registration should not conflict");

        let mut store = Store::new(
            &self.engine,
            WasmStoreState {
                wasi: wasi_ctx,
                limits: self.store_limits(),
                capability_id: capability_id.to_string(),
                emits: emits.to_vec(),
                service_type,
                emitted_events: Vec::new(),
                connector_context,
                connector_invocation_evidence: Vec::new(),
            },
        );
        store.limiter(|state| &mut state.limits);
        store
            .set_fuel(self.limits.fuel_budget)
            .map_err(|e| ExecutorError::RuntimeSetupFailed(format!("set fuel: {e}")))?;

        linker
            .module(&mut store, "", &cached_module.module)
            .map_err(|e| ExecutorError::RuntimeSetupFailed(format!("module link: {e}")))?;

        let invocation = linker
            .get_default(&mut store, "")
            .map_err(|e| ExecutorError::RuntimeSetupFailed(format!("get_default: {e}")))?
            .typed::<(), ()>(&store)
            .map_err(|e| ExecutorError::RuntimeSetupFailed(format!("typed: {e}")))?
            .call(&mut store, ());

        if let Err(error) = invocation {
            // WASI preview1 implements `proc_exit` by returning `I32Exit` from
            // the guest invocation. A zero status is the command's successful
            // completion signal, so stdout remains the capability result.
            if !matches!(error.downcast_ref::<wasmtime_wasi::I32Exit>(), Some(exit) if exit.0 == 0)
            {
                return Err(classify_wasm_execution_error(&error));
            }
        }

        // Extract captured stdout — contents() reads the buffer without consuming it
        let raw_output = stdout_ref.contents();

        let value = serde_json::from_slice::<Value>(&raw_output).map_err(|e| {
            ExecutorError::OutputDeserializationFailed(format!(
                "stdout is not valid JSON: {e} — raw: {}",
                String::from_utf8_lossy(&raw_output)
            ))
        })?;

        let data = store.into_data();
        Ok(ExecutorOutput {
            value,
            emitted_events: data.emitted_events,
            connector_invocation_evidence: data.connector_invocation_evidence,
        })
    }

    fn store_limits(&self) -> StoreLimits {
        StoreLimitsBuilder::new()
            .memory_size(self.limits.memory_bytes)
            .table_elements(self.limits.table_elements)
            .instances(self.limits.instances)
            .tables(self.limits.tables)
            .memories(self.limits.memories)
            .trap_on_grow_failure(true)
            .build()
    }

    fn compiled_module(
        &self,
        wasm_bytes: &[u8],
        checksum: &str,
        abi_version: &str,
    ) -> Result<CachedModule, ExecutorError> {
        {
            let mut cache = self
                .module_cache
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(cached) = cache.get(checksum, abi_version) {
                return Ok(cached);
            }
        }

        let module = Module::from_binary(&self.engine, wasm_bytes).map_err(|e| {
            ExecutorError::MalformedWasmArtifact {
                error_code: "malformed_wasm_artifact".to_string(),
                detail: format!("module compile: {e}"),
            }
        })?;
        let validation = validate_module_imports(&module, abi_version)?;
        let cached = CachedModule { module, validation };

        let mut cache = self
            .module_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        cache.insert(checksum.to_string(), cached.clone());
        Ok(cached)
    }

    fn load_binary(&self, wasm_path: &str) -> Result<CachedBinary, ExecutorError> {
        let metadata = fs::metadata(wasm_path).map_err(|e| {
            ExecutorError::BinaryLoadFailed(format!("cannot read {wasm_path}: {e}"))
        })?;
        let identity = binary_file_identity(wasm_path, metadata.len(), metadata.modified())?;

        let mut cache = self
            .binary_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(cached) = cache.get(wasm_path, &identity) {
            return Ok(cached);
        }

        let bytes: Arc<[u8]> = fs::read(wasm_path)
            .map_err(|e| ExecutorError::BinaryLoadFailed(format!("cannot read {wasm_path}: {e}")))?
            .into();
        let cached = CachedBinary {
            identity,
            checksum: sha256_hex(&bytes),
            bytes,
        };
        cache.record_load();
        cache.insert(wasm_path.to_string(), cached.clone());
        Ok(cached)
    }
}

fn binary_file_identity(
    wasm_path: &str,
    len: u64,
    modified: Result<SystemTime, std::io::Error>,
) -> Result<BinaryFileIdentity, ExecutorError> {
    let modified = modified
        .map_err(|e| ExecutorError::BinaryLoadFailed(format!("cannot read {wasm_path}: {e}")))?;
    Ok(BinaryFileIdentity { len, modified })
}

/// Host implementation of `traverse_host::emit_event` (spec
/// 098-capability-event-host-abi FR-001). The guest passes a pointer/length
/// into its own linear memory holding a JSON payload shaped
/// `{"event_id": "...", "version": "...", "payload": {...}}`; this function
/// validates it synchronously, at call time, and never panics or traps on a
/// malformed or out-of-bounds guest pointer (FR-008) — every failure path
/// returns a negative status code to the guest instead.
fn handle_emit_event(mut caller: Caller<'_, WasmStoreState>, ptr: i32, len: i32) -> i32 {
    // FR-003: checked before any guest memory is touched — rejected
    // regardless of payload.
    if caller.data().service_type != ServiceType::Subscribable {
        return EMIT_EVENT_ERR_NOT_SUBSCRIBABLE;
    }

    // FR-008: bounds/size checked before any read or deserialization.
    if ptr < 0 || len < 0 {
        return EMIT_EVENT_ERR_INVALID_PAYLOAD;
    }
    #[allow(clippy::cast_sign_loss)]
    let (ptr, len) = (ptr as usize, len as usize);
    if len > MAX_EVENT_EMIT_PAYLOAD_BYTES {
        return EMIT_EVENT_ERR_INVALID_PAYLOAD;
    }

    let Some(Extern::Memory(memory)) = caller.get_export("memory") else {
        return EMIT_EVENT_ERR_INVALID_PAYLOAD;
    };

    let mut buffer = vec![0u8; len];
    // `Memory::read` bounds-checks `ptr + len` against actual guest memory
    // size and returns `Err` rather than panicking or reading out of bounds.
    if memory.read(&caller, ptr, &mut buffer).is_err() {
        return EMIT_EVENT_ERR_INVALID_PAYLOAD;
    }

    let Ok(payload) = serde_json::from_slice::<Value>(&buffer) else {
        return EMIT_EVENT_ERR_INVALID_PAYLOAD;
    };
    let Some(event_type) = payload
        .get("event_id")
        .and_then(Value::as_str)
        .map(str::to_string)
    else {
        return EMIT_EVENT_ERR_INVALID_PAYLOAD;
    };
    let Some(version) = payload
        .get("version")
        .and_then(Value::as_str)
        .map(str::to_string)
    else {
        return EMIT_EVENT_ERR_INVALID_PAYLOAD;
    };
    let data = payload
        .get("payload")
        .cloned()
        .unwrap_or_else(|| Value::Object(serde_json::Map::new()));

    // FR-002: declared-emission check, synchronous, at call time.
    let declared = caller
        .data()
        .emits
        .iter()
        .any(|decl| decl.event_id == event_type && decl.version == version);
    if !declared {
        return EMIT_EVENT_ERR_UNDECLARED_EVENT;
    }

    let capability_id = caller.data().capability_id.clone();
    let event = TraverseEvent {
        id: Uuid::new_v4().to_string(),
        source: format!("traverse-runtime/{capability_id}"),
        event_type: event_type.clone(),
        datacontenttype: "application/json".to_string(),
        time: Utc::now().to_rfc3339(),
        data,
        owner: capability_id.clone(),
        version: version.clone(),
        lifecycle_status: LifecycleStatus::Active,
        deduplication_id: Some(format!("{capability_id}:{event_type}:{version}")),
        ordering_scope: Some(capability_id),
        correlation_id: None,
        causation_id: None,
        subject_id: None,
        actor_id: None,
    };
    caller.data_mut().emitted_events.push(event);
    EMIT_EVENT_OK
}

/// Fail closed until an embedding host supplies an active application binding.
///
/// The four integers are the versioned ABI's request pointer/length and
/// response pointer/capacity. This default handler intentionally neither reads
/// nor writes guest memory: no request data, host configuration, credentials,
/// paths, or endpoint can cross the boundary before authorization exists.
fn handle_connector_invoke(
    mut caller: Caller<'_, WasmStoreState>,
    request_ptr: i32,
    request_len: i32,
    response_ptr: i32,
    response_capacity: i32,
) -> i32 {
    let Ok((memory, request, request_ptr, response_ptr, response_capacity)) =
        parse_connector_request(
            &mut caller,
            request_ptr,
            request_len,
            response_ptr,
            response_capacity,
        )
    else {
        return CONNECTOR_INVOKE_ERR_INVALID_REQUEST;
    };
    let Some(context) = caller.data().connector_context.clone() else {
        return connector_failure(
            &mut caller,
            &request.connector_id,
            None,
            "unbound",
            CONNECTOR_INVOKE_ERR_UNBOUND,
        );
    };
    let connector = match resolve_activated_connector(&context, &request) {
        Ok(connector) => connector,
        Err(failure) => {
            return connector_failure(
                &mut caller,
                &request.connector_id,
                failure.resolved_version.as_deref(),
                failure.failure_class,
                failure.code,
            );
        }
    };
    invoke_activated_connector(
        &mut caller,
        memory,
        request,
        request_ptr,
        response_ptr,
        response_capacity,
        &connector,
    )
}

fn parse_connector_request(
    caller: &mut Caller<'_, WasmStoreState>,
    request_ptr: i32,
    request_len: i32,
    response_ptr: i32,
    response_capacity: i32,
) -> Result<
    (
        wasmtime::Memory,
        ConnectorInvokeRequest,
        usize,
        usize,
        usize,
    ),
    (),
> {
    if request_ptr < 0 || request_len < 0 || response_ptr < 0 || response_capacity < 0 {
        return Err(());
    }
    #[allow(clippy::cast_sign_loss)]
    let (request_ptr, request_len, response_ptr, response_capacity) = (
        request_ptr as usize,
        request_len as usize,
        response_ptr as usize,
        response_capacity as usize,
    );
    if request_len > MAX_CONNECTOR_INVOKE_REQUEST_BYTES
        || response_capacity > MAX_CONNECTOR_INVOKE_RESPONSE_BYTES
    {
        return Err(());
    }
    let Some(Extern::Memory(memory)) = caller.get_export("memory") else {
        return Err(());
    };
    let mut bytes = vec![0_u8; request_len];
    if memory.read(&caller, request_ptr, &mut bytes).is_err() {
        return Err(());
    }
    let Ok(request) = serde_json::from_slice::<ConnectorInvokeRequest>(&bytes) else {
        return Err(());
    };
    if request.abi_version != SUPPORTED_HOST_ABI_VERSION
        || request.connector_id.is_empty()
        || request.operation.is_empty()
        || contains_host_private_data(&request.payload)
    {
        return Err(());
    }
    Ok((
        memory,
        request,
        request_ptr,
        response_ptr,
        response_capacity,
    ))
}

struct ConnectorInvokeFailure {
    failure_class: &'static str,
    resolved_version: Option<String>,
    code: i32,
}

fn resolve_activated_connector(
    context: &MediatedConnectorContext,
    request: &ConnectorInvokeRequest,
) -> Result<ActivatedConnector, ConnectorInvokeFailure> {
    if !context
        .declared_requirements
        .iter()
        .any(|requirement| requirement.connector_id == request.connector_id)
    {
        return Err(ConnectorInvokeFailure {
            failure_class: "undeclared",
            resolved_version: None,
            code: CONNECTOR_INVOKE_ERR_UNDECLARED,
        });
    }
    let Some(connector) = context
        .activated_connectors
        .iter()
        .find(|connector| connector.connector_id == request.connector_id)
    else {
        return Err(ConnectorInvokeFailure {
            failure_class: "unbound",
            resolved_version: None,
            code: CONNECTOR_INVOKE_ERR_UNBOUND,
        });
    };
    let compatible = context
        .declared_requirements
        .iter()
        .filter(|requirement| requirement.connector_id == request.connector_id)
        .any(|requirement| {
            VersionReq::parse(&requirement.version)
                .ok()
                .zip(Version::parse(&connector.version).ok())
                .is_some_and(|(range, version)| range.matches(&version))
        });
    if !compatible {
        return Err(ConnectorInvokeFailure {
            failure_class: "incompatible",
            resolved_version: Some(connector.version.clone()),
            code: CONNECTOR_INVOKE_ERR_UNAUTHORIZED,
        });
    }
    Ok(connector.clone())
}

fn invoke_activated_connector(
    caller: &mut Caller<'_, WasmStoreState>,
    memory: wasmtime::Memory,
    request: ConnectorInvokeRequest,
    _request_ptr: usize,
    response_ptr: usize,
    response_capacity: usize,
    connector: &ActivatedConnector,
) -> i32 {
    let Ok(response) = connector.implementation.invoke(&request) else {
        return connector_failure(
            caller,
            &request.connector_id,
            Some(&connector.version),
            "execution_failed",
            CONNECTOR_INVOKE_ERR_EXECUTION_FAILED,
        );
    };
    if response.abi_version != SUPPORTED_HOST_ABI_VERSION
        || response.result_class.is_empty()
        || contains_host_private_data(&response.payload)
    {
        return connector_failure(
            caller,
            &request.connector_id,
            Some(&connector.version),
            "unauthorized_output",
            CONNECTOR_INVOKE_ERR_UNAUTHORIZED,
        );
    }
    #[allow(clippy::expect_used)]
    let response_bytes =
        serde_json::to_vec(&response).expect("connector response uses serializable JSON value");
    if response_bytes.len() > response_capacity
        || response_bytes.len() > MAX_CONNECTOR_INVOKE_RESPONSE_BYTES
    {
        return connector_failure(
            caller,
            &request.connector_id,
            Some(&connector.version),
            "bounded_io",
            CONNECTOR_INVOKE_ERR_PAYLOAD_TOO_LARGE,
        );
    }
    if memory
        .write(&mut *caller, response_ptr, &response_bytes)
        .is_err()
    {
        return connector_failure(
            caller,
            &request.connector_id,
            Some(&connector.version),
            "invalid_response_memory",
            CONNECTOR_INVOKE_ERR_INVALID_REQUEST,
        );
    }
    caller
        .data_mut()
        .connector_invocation_evidence
        .push(ConnectorInvocationEvidence {
            connector_id: request.connector_id,
            resolved_version: Some(connector.version.clone()),
            result_class: response.result_class,
            failure_class: None,
        });
    #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
    {
        response_bytes.len() as i32
    }
}

fn connector_failure(
    caller: &mut Caller<'_, WasmStoreState>,
    connector_id: &str,
    resolved_version: Option<&str>,
    failure_class: &str,
    code: i32,
) -> i32 {
    caller
        .data_mut()
        .connector_invocation_evidence
        .push(ConnectorInvocationEvidence {
            connector_id: connector_id.to_string(),
            resolved_version: resolved_version.map(str::to_string),
            result_class: "failure".to_string(),
            failure_class: Some(failure_class.to_string()),
        });
    code
}

fn contains_host_private_data(value: &Value) -> bool {
    match value {
        Value::Object(map) => map.iter().any(|(key, value)| {
            let key = key.to_ascii_lowercase();
            [
                "config",
                "credential",
                "secret",
                "password",
                "path",
                "device",
                "endpoint",
                "bucket",
            ]
            .iter()
            .any(|needle| key.contains(needle))
                || contains_host_private_data(value)
        }),
        Value::Array(values) => values.iter().any(contains_host_private_data),
        Value::String(value) => value.starts_with('/') || value.contains("://"),
        _ => false,
    }
}

fn classify_wasm_execution_error(error: &wasmtime::Error) -> ExecutorError {
    let display = error.to_string();
    let debug = format!("{error:?}");
    if display.contains("all fuel consumed by WebAssembly")
        || debug.contains("all fuel consumed by WebAssembly")
    {
        return ExecutorError::Timeout(debug);
    }
    if display.contains("forcing trap when growing") || debug.contains("forcing trap when growing")
    {
        return ExecutorError::ResourceExhausted(debug);
    }
    ExecutorError::ExecutionFailed(display)
}

fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher
        .finalize()
        .iter()
        .fold(String::new(), |mut acc, byte| {
            let _ = write!(acc, "{byte:02x}");
            acc
        })
}

fn validate_module_imports(
    module: &Module,
    abi_version: &str,
) -> Result<HostAbiValidation, ExecutorError> {
    let whitelist = host_abi_whitelist(abi_version)?;
    let mut imports = module
        .imports()
        .map(|import| HostAbiImport {
            module: import.module().to_string(),
            name: import.name().to_string(),
        })
        .collect::<Vec<_>>();
    imports.sort_by(|a, b| a.module.cmp(&b.module).then_with(|| a.name.cmp(&b.name)));

    for import in &imports {
        if !whitelist
            .imports
            .iter()
            .any(|allowed| allowed.module == import.module && allowed.name == import.name)
        {
            return Err(ExecutorError::UnauthorizedHostImport {
                error_code: "unauthorized_host_import".to_string(),
                abi_version: abi_version.to_string(),
                module: import.module.clone(),
                name: import.name.clone(),
            });
        }
    }

    Ok(HostAbiValidation {
        abi_version: whitelist.abi_version,
        imports,
    })
}

fn host_abi_whitelist(abi_version: &str) -> Result<HostAbiWhitelist, ExecutorError> {
    if abi_version != SUPPORTED_HOST_ABI_VERSION {
        return Err(ExecutorError::UnsupportedAbiVersion {
            error_code: "unsupported_abi_version".to_string(),
            requested: abi_version.to_string(),
            supported: supported_host_abi_versions().join(", "),
        });
    }

    HOST_ABI_V1_WHITELIST_CACHE
        .as_ref()
        .cloned()
        .map_err(|e| ExecutorError::RuntimeSetupFailed(format!("invalid ABI whitelist: {e}")))
}

#[cfg(test)]
mod binary_cache_tests {
    use super::*;

    #[test]
    fn binary_file_identity_preserves_modified_time_failures() {
        let result = binary_file_identity(
            "unreadable-metadata.wasm",
            0,
            Err(std::io::Error::other("modified time unavailable")),
        );

        assert!(matches!(result, Err(ExecutorError::BinaryLoadFailed(_))));
    }
}
