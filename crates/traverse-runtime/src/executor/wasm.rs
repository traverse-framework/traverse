//! Wasmtime-backed WASM executor.
//!
//! Executes `wasm32-wasi` capability binaries inside a sandboxed Wasmtime engine.
//! Input is fed via WASI stdin; output is captured from WASI stdout.
//! No ambient WASI authority is granted — all capabilities are deny-by-default.

use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};
use std::fs;
use std::sync::{LazyLock, Mutex};
use wasmtime::{Config, Engine, Linker, Module, Store, StoreLimits, StoreLimitsBuilder};
use wasmtime_wasi::WasiCtxBuilder;
use wasmtime_wasi::p1::WasiP1Ctx;
use wasmtime_wasi::p2::pipe::{MemoryInputPipe, MemoryOutputPipe};

use super::{ArtifactType, CapabilityExecutor, ExecutorCapability, ExecutorError};

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

struct WasmStoreState {
    wasi: WasiP1Ctx,
    limits: StoreLimits,
}

impl CapabilityExecutor for WasmExecutor {
    fn execute(
        &self,
        capability: &ExecutorCapability,
        input: &Value,
    ) -> Result<Value, ExecutorError> {
        if capability.artifact_type != ArtifactType::Wasm {
            return Err(ExecutorError::UnsupportedArtifactType);
        }

        // --- Load binary ---
        let wasm_path = capability.wasm_binary_path.as_deref().ok_or_else(|| {
            ExecutorError::BinaryLoadFailed("no wasm_binary_path set".to_string())
        })?;

        let binary = fs::read(wasm_path).map_err(|e| {
            ExecutorError::BinaryLoadFailed(format!("cannot read {wasm_path}: {e}"))
        })?;

        // --- Checksum validation ---
        if let Some(expected) = capability.wasm_checksum.as_deref() {
            let actual = sha256_hex(&binary);
            if actual != expected {
                return Err(ExecutorError::ChecksumMismatch {
                    expected: expected.to_string(),
                    actual,
                });
            }
        }

        let abi_version = capability
            .host_abi_version
            .as_deref()
            .unwrap_or(SUPPORTED_HOST_ABI_VERSION);

        self.run_wasm(&binary, input, abi_version)
    }
}

impl WasmExecutor {
    /// Execute pre-loaded WASM bytes with the given input.
    ///
    /// Exposed separately so tests can pass raw bytes without needing a file on disk.
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
        self.run_wasm(wasm_bytes, input, abi_version)
    }

    fn run_wasm(
        &self,
        wasm_bytes: &[u8],
        input: &Value,
        abi_version: &str,
    ) -> Result<Value, ExecutorError> {
        let input_json = serde_json::to_string(input)
            .map_err(|e| ExecutorError::ExecutionFailed(format!("input serialization: {e}")))?;

        let cached_module = self.compiled_module(wasm_bytes, abi_version)?;

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

        let mut store = Store::new(
            &self.engine,
            WasmStoreState {
                wasi: wasi_ctx,
                limits: self.store_limits(),
            },
        );
        store.limiter(|state| &mut state.limits);
        store
            .set_fuel(self.limits.fuel_budget)
            .map_err(|e| ExecutorError::RuntimeSetupFailed(format!("set fuel: {e}")))?;

        linker
            .module(&mut store, "", &cached_module.module)
            .map_err(|e| ExecutorError::RuntimeSetupFailed(format!("module link: {e}")))?;

        linker
            .get_default(&mut store, "")
            .map_err(|e| ExecutorError::RuntimeSetupFailed(format!("get_default: {e}")))?
            .typed::<(), ()>(&store)
            .map_err(|e| ExecutorError::RuntimeSetupFailed(format!("typed: {e}")))?
            .call(&mut store, ())
            .map_err(|error| classify_wasm_execution_error(&error))?;

        // Extract captured stdout — contents() reads the buffer without consuming it
        let raw_output = stdout_ref.contents();

        serde_json::from_slice::<Value>(&raw_output).map_err(|e| {
            ExecutorError::OutputDeserializationFailed(format!(
                "stdout is not valid JSON: {e} — raw: {}",
                String::from_utf8_lossy(&raw_output)
            ))
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
        abi_version: &str,
    ) -> Result<CachedModule, ExecutorError> {
        let checksum = sha256_hex(wasm_bytes);
        {
            let mut cache = self
                .module_cache
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(cached) = cache.get(&checksum, abi_version) {
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
        cache.insert(checksum, cached.clone());
        Ok(cached)
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
    format!("{:x}", hasher.finalize())
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
