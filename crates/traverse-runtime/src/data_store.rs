//! Portable state access for Traverse capabilities.
//!
//! General operations are governed by spec `032-universal-data-access`; the
//! local-file adapter durability boundary is governed by spec
//! `518-durable-local-datastore`. Retention prune and verified backup/restore
//! are governed by spec `083-datastore-retention-backup`.

#[path = "data_store_coordinator.rs"]
mod coordinator;
#[path = "data_store_hosted_sync.rs"]
mod hosted_sync;
#[path = "data_store_maintenance.rs"]
mod maintenance;
#[path = "data_store_remote.rs"]
mod remote;
pub use coordinator::{DataStoreCoordinator, DataStoreCoordinatorError};
pub use hosted_sync::{
    AblyEdgeError, AblyHistoryBatch, AblyHostedSyncTransport, AblyRealtimeEdge,
    EncryptedSyncOperation, HostedSyncConnectionState, HostedSyncCredential,
    HostedSyncDegradedReason, HostedSyncError, HostedSyncErrorCode, HostedSyncLineageEvidence,
    HostedSyncObservation, HostedSyncPublishReceipt, HostedSyncReplayResult, HostedSyncTransport,
    InMemoryAblyEdge, InMemoryHostedSyncTransport, SyncScopeId, run_hosted_sync_conformance,
};
pub use maintenance::{
    BackupManifest, BackupRecordIndexEntry, DataStoreMaintenance, DataStoreMigration,
    LocalFileDataStoreMaintenance, MaintenanceError, MaintenanceErrorCode, MaintenanceEvidence,
    MigrationError, MigrationErrorCode, MigrationReport, RetentionPolicy,
};
pub use remote::{
    RemoteBackendFailure, RemoteDataStoreBackend, RemoteKeyValueDataStore, RemoteObject,
    RemoteOperationEvidence, RemoteVersionToken, RemoteWriteOutcome,
};

#[cfg(feature = "datastore-encryption")]
use aes_gcm::aead::consts::U12;
#[cfg(feature = "datastore-encryption")]
use aes_gcm::aead::{Aead, KeyInit, Payload};
#[cfg(feature = "datastore-encryption")]
use aes_gcm::{Aes256Gcm, Nonce};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use traverse_contracts::CapabilityContract;
use zeroize::Zeroizing;

/// The synchronization report is governed by the approved protocol, rather
/// than the earlier generic `DataStore` surface that supplied its merge helper.
const DATA_STORE_SPEC: &str = "089-datastore-synchronization";
const LOCAL_DATA_STORE_FORMAT: &str = "local-datastore/1";
const LOCAL_DATA_STORE_V2_FORMAT: &str = "local-datastore/2";
const LOCAL_DATA_STORE_V2_FORMAT_VERSION: u32 = 2;
const LOCAL_DATA_STORE_V2_SCHEMA_VERSION: u32 = 1;
const LOCAL_DATA_STORE_LOCK_FILE: &str = ".traverse-datastore.lock";
const HEXADECIMAL_DIGITS: &[u8; 16] = b"0123456789abcdef";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateRecord {
    pub key: String,
    pub value: Value,
    pub lamport_clock: u64,
    pub writer_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergeDecision {
    pub key: String,
    pub winning_writer_id: String,
    pub winning_lamport_clock: u64,
    pub resolution_rule: ConflictResolutionRule,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictResolutionRule {
    OnlyLocal,
    OnlyRemote,
    HigherLamportClock,
    WriterIdentityTieBreak,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncReport {
    pub governing_spec: String,
    pub decisions: Vec<MergeDecision>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataStoreError {
    pub code: DataStoreErrorCode,
    pub message: String,
    pub details: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataStoreErrorCode {
    SchemaValidationError,
    NoStateSchemaDeclared,
    LamportClockOverflow,
    InvalidKey,
    IoFailure,
    SerializationFailure,
    SyncFailure,
    IntegrityCheckFailed,
    StoreLocked,
    DurabilityCommitFailed,
    KeyProviderRequired,
    KeyNotFound,
    KeyExpired,
    KeyProviderFailure,
    CryptoFailure,
    ClassificationChangeNotAllowed,
    RemoteConflict,
    RemoteUnavailable,
    RemoteTimeout,
    RemoteOutcomeUnknown,
    RemoteUnauthorized,
    RemoteScopeDenied,
    RemoteIntegrityFailed,
    RemoteBackendFailed,
}

/// Classification recorded with each locally durable record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalDataClassification {
    Public,
    Private,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct LocalDataStoreEnvelope {
    format: String,
    classification: LocalDataClassification,
    digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    record: Option<StateRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    record_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    key_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    nonce: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ciphertext: Option<String>,
    /// Host-supplied RFC3339 instant stamped at write time for age-based prune.
    /// Absent on legacy envelopes; age prune treats missing stamps as retained.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    retained_at: Option<String>,
}

/// Version-two wrapper keeps the durable payload self-identifying and
/// independently integrity-protected. The payload remains opaque to callers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct LocalDataStoreV2Envelope {
    format: String,
    format_version: u32,
    schema_version: u32,
    payload_integrity: String,
    integrity: LocalDataStoreIntegrity,
    encryption_disclosure: String,
    payload: LocalDataStoreEnvelope,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct LocalDataStoreIntegrity {
    algorithm: String,
    content_digest: String,
}

fn v2_envelope(
    payload: LocalDataStoreEnvelope,
) -> Result<LocalDataStoreV2Envelope, DataStoreError> {
    let payload_bytes = serde_json::to_vec(&payload)
        .map_err(|error| serialization_error("serialize v2 data store payload", &error))?;
    let payload_integrity = digest_bytes(&payload_bytes);
    Ok(LocalDataStoreV2Envelope {
        format: LOCAL_DATA_STORE_V2_FORMAT.to_string(),
        format_version: LOCAL_DATA_STORE_V2_FORMAT_VERSION,
        schema_version: LOCAL_DATA_STORE_V2_SCHEMA_VERSION,
        integrity: LocalDataStoreIntegrity {
            algorithm: "sha256".to_string(),
            content_digest: payload_integrity.clone(),
        },
        payload_integrity,
        encryption_disclosure: match payload.classification {
            LocalDataClassification::Public => "not_enabled".to_string(),
            LocalDataClassification::Private => "host_managed_opaque".to_string(),
        },
        payload,
    })
}

fn decode_v2_envelope(value: Value) -> Result<LocalDataStoreEnvelope, DataStoreError> {
    let envelope: LocalDataStoreV2Envelope =
        serde_json::from_value(value).map_err(|_| integrity_error("malformed_envelope"))?;
    if envelope.format != LOCAL_DATA_STORE_V2_FORMAT
        || envelope.format_version != LOCAL_DATA_STORE_V2_FORMAT_VERSION
        || envelope.schema_version != LOCAL_DATA_STORE_V2_SCHEMA_VERSION
        || envelope.integrity.algorithm != "sha256"
        || envelope.integrity.content_digest != envelope.payload_integrity
        || !matches!(
            envelope.encryption_disclosure.as_str(),
            "not_enabled" | "host_managed_opaque"
        )
    {
        return Err(integrity_error("unknown_format_version"));
    }
    let payload_bytes = serde_json::to_vec(&envelope.payload)
        .map_err(|error| serialization_error("serialize v2 payload for verification", &error))?;
    if digest_bytes(&payload_bytes) != envelope.payload_integrity {
        return Err(integrity_error("digest_mismatch"));
    }
    if envelope.payload.format != LOCAL_DATA_STORE_FORMAT {
        return Err(integrity_error("unknown_format_version"));
    }
    Ok(envelope.payload)
}

/// Stable, secret-free key-provider failure codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyProviderErrorCode {
    MissingKey,
    ExpiredKeyId,
    ProviderFailure,
}

/// A key-provider failure that never includes key material or provider internals.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyProviderError {
    pub code: KeyProviderErrorCode,
    pub message: String,
}

impl KeyProviderError {
    #[must_use]
    pub fn missing_key() -> Self {
        Self {
            code: KeyProviderErrorCode::MissingKey,
            message: "key_not_found".to_string(),
        }
    }

    #[must_use]
    pub fn expired_key_id() -> Self {
        Self {
            code: KeyProviderErrorCode::ExpiredKeyId,
            message: "key_expired".to_string(),
        }
    }

    #[must_use]
    pub fn provider_failure() -> Self {
        Self {
            code: KeyProviderErrorCode::ProviderFailure,
            message: "key_provider_failed".to_string(),
        }
    }
}

/// Host-owned source of AES-256 keys.
pub trait KeyProvider: Send + Sync {
    /// Returns the key id used for new private writes.
    ///
    /// # Errors
    ///
    /// Returns a secret-free [`KeyProviderError`] when no write key is available.
    fn active_key_id(&self) -> Result<String, KeyProviderError>;

    /// Returns key material for an envelope key id.
    ///
    /// # Errors
    ///
    /// Returns a secret-free [`KeyProviderError`] for missing, expired, or
    /// unavailable keys.
    fn key_for(&self, key_id: &str) -> Result<Zeroizing<[u8; 32]>, KeyProviderError>;
}

/// In-memory provider for tests and host wiring.
#[derive(Clone)]
pub struct InMemoryKeyProvider {
    active_key_id: String,
    keys: BTreeMap<String, Zeroizing<[u8; 32]>>,
    expired_key_ids: BTreeSet<String>,
}

impl InMemoryKeyProvider {
    #[must_use]
    pub fn new(active_key_id: impl Into<String>, key: [u8; 32]) -> Self {
        let active_key_id = active_key_id.into();
        let mut keys = BTreeMap::new();
        keys.insert(active_key_id.clone(), Zeroizing::new(key));
        Self {
            active_key_id,
            keys,
            expired_key_ids: BTreeSet::new(),
        }
    }

    #[must_use]
    pub fn with_read_key(mut self, key_id: impl Into<String>, key: [u8; 32]) -> Self {
        self.keys.insert(key_id.into(), Zeroizing::new(key));
        self
    }

    #[must_use]
    pub fn with_expired_key_id(mut self, key_id: impl Into<String>) -> Self {
        self.expired_key_ids.insert(key_id.into());
        self
    }
}

impl KeyProvider for InMemoryKeyProvider {
    fn active_key_id(&self) -> Result<String, KeyProviderError> {
        if self.expired_key_ids.contains(&self.active_key_id) {
            return Err(KeyProviderError::expired_key_id());
        }
        if self.keys.contains_key(&self.active_key_id) {
            Ok(self.active_key_id.clone())
        } else {
            Err(KeyProviderError::missing_key())
        }
    }

    fn key_for(&self, key_id: &str) -> Result<Zeroizing<[u8; 32]>, KeyProviderError> {
        if self.expired_key_ids.contains(key_id) {
            return Err(KeyProviderError::expired_key_id());
        }
        self.keys
            .get(key_id)
            .cloned()
            .ok_or_else(KeyProviderError::missing_key)
    }
}

pub trait DataStore {
    /// Reads a stored state record.
    ///
    /// # Errors
    ///
    /// Returns [`DataStoreError`] when the adapter cannot read the key.
    fn read(&self, key: &str) -> Result<Option<StateRecord>, DataStoreError>;

    /// Writes a stamped state record.
    ///
    /// # Errors
    ///
    /// Returns [`DataStoreError`] when the adapter cannot persist the record.
    fn write(&mut self, record: StateRecord) -> Result<(), DataStoreError>;

    /// Deletes a stored state record.
    ///
    /// # Errors
    ///
    /// Returns [`DataStoreError`] when the adapter cannot delete the key.
    fn delete(&mut self, key: &str) -> Result<(), DataStoreError>;

    /// Lists stored state keys.
    ///
    /// # Errors
    ///
    /// Returns [`DataStoreError`] when the adapter cannot enumerate keys.
    fn list_keys(&self) -> Result<Vec<String>, DataStoreError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LamportClock {
    writer_id: String,
    value: u64,
}

impl LamportClock {
    #[must_use]
    pub fn new(writer_id: impl Into<String>) -> Self {
        Self {
            writer_id: writer_id.into(),
            value: 0,
        }
    }

    #[must_use]
    pub fn with_value(writer_id: impl Into<String>, value: u64) -> Self {
        Self {
            writer_id: writer_id.into(),
            value,
        }
    }

    fn next(&mut self) -> Result<u64, DataStoreError> {
        let next = self.value.checked_add(1).ok_or_else(|| {
            data_store_error(
                DataStoreErrorCode::LamportClockOverflow,
                "lamport clock overflow",
                json!({ "writer_id": self.writer_id }),
            )
        })?;
        self.value = next;
        Ok(next)
    }
}

pub struct RuntimeDataStore<A> {
    adapter: A,
    clock: LamportClock,
}

impl<A: DataStore> RuntimeDataStore<A> {
    #[must_use]
    pub fn new(adapter: A, writer_id: impl Into<String>) -> Self {
        Self {
            adapter,
            clock: LamportClock::new(writer_id),
        }
    }

    #[must_use]
    pub fn with_clock(adapter: A, clock: LamportClock) -> Self {
        Self { adapter, clock }
    }

    /// Reads and validates a state value by key.
    ///
    /// # Errors
    ///
    /// Returns [`DataStoreError`] when the key is invalid, the adapter cannot
    /// read the key, or the stored value violates the contract state schema.
    pub fn read(
        &self,
        contract: &CapabilityContract,
        key: &str,
    ) -> Result<Option<Value>, DataStoreError> {
        validate_key(key)?;
        if contract.state_schema.is_none() {
            return Ok(None);
        }
        self.adapter.read(key).and_then(|record| {
            record
                .map(|record| {
                    validate_state_write(contract, key, &record.value)?;
                    Ok(record.value)
                })
                .transpose()
        })
    }

    /// Validates, stamps, and writes a state value for a capability contract.
    ///
    /// # Errors
    ///
    /// Returns [`DataStoreError`] when the key is invalid, no state schema is
    /// declared, schema validation fails, the Lamport clock overflows, or the
    /// adapter cannot persist the stamped record.
    pub fn write(
        &mut self,
        contract: &CapabilityContract,
        key: &str,
        value: Value,
    ) -> Result<StateRecord, DataStoreError> {
        validate_state_write(contract, key, &value)?;
        let record = StateRecord {
            key: key.to_string(),
            value,
            lamport_clock: self.clock.next()?,
            writer_id: self.clock.writer_id.clone(),
        };
        self.adapter.write(record.clone())?;
        Ok(record)
    }

    /// Deletes a state value by key.
    ///
    /// # Errors
    ///
    /// Returns [`DataStoreError`] when the adapter cannot delete the key.
    pub fn delete(&mut self, key: &str) -> Result<(), DataStoreError> {
        self.adapter.delete(key)
    }

    /// Lists state keys.
    ///
    /// # Errors
    ///
    /// Returns [`DataStoreError`] when the adapter cannot enumerate keys.
    pub fn list_keys(&self) -> Result<Vec<String>, DataStoreError> {
        self.adapter.list_keys()
    }

    /// Triggers explicit sync after a reconnect event.
    ///
    /// # Errors
    ///
    /// Returns [`DataStoreError`] when either adapter cannot read, write, list,
    /// or restore state during sync.
    pub fn sync_on_reconnect(
        &mut self,
        remote: &mut dyn DataStore,
    ) -> Result<SyncReport, DataStoreError> {
        sync_adapters(&mut self.adapter, remote)
    }

    pub fn into_inner(self) -> A {
        self.adapter
    }
}

pub struct LocalFileDataStore {
    root: PathBuf,
    classification: LocalDataClassification,
    lock_file: File,
    key_provider: Option<Arc<dyn KeyProvider>>,
    /// Host-supplied retained-at stamp applied to subsequent writes (no OS clock).
    write_retained_at: Option<String>,
}

impl std::fmt::Debug for LocalFileDataStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LocalFileDataStore")
            .field("root", &self.root)
            .field("classification", &self.classification)
            .field("key_provider_configured", &self.key_provider.is_some())
            .field("write_retained_at", &self.write_retained_at)
            .finish_non_exhaustive()
    }
}

impl Drop for LocalFileDataStore {
    fn drop(&mut self) {
        let _ = self.lock_file.unlock();
    }
}

impl LocalFileDataStore {
    /// Creates a local filesystem-backed data store rooted at `root`.
    ///
    /// # Errors
    ///
    /// Returns [`DataStoreError`] when the root directory cannot be created or
    /// another process owns the store.
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, DataStoreError> {
        Self::with_classification(root, LocalDataClassification::Private)
    }

    /// Creates a local filesystem-backed data store with explicit persisted
    /// record classification.
    ///
    /// # Errors
    ///
    /// Returns [`DataStoreError`] when the root directory cannot be created or
    /// another process owns the store.
    pub fn with_classification(
        root: impl Into<PathBuf>,
        classification: LocalDataClassification,
    ) -> Result<Self, DataStoreError> {
        let root = root.into();
        fs::create_dir_all(&root).map_err(|error| io_error("create data store root", &error))?;
        let lock_path = root.join(LOCAL_DATA_STORE_LOCK_FILE);
        let lock_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|error| io_error("open data store lock", &error))?;
        lock_file.try_lock().map_err(lock_error)?;
        Ok(Self {
            root,
            classification,
            lock_file,
            key_provider: None,
            write_retained_at: None,
        })
    }

    /// Configures the host-owned key provider used for private records.
    #[must_use]
    pub fn with_key_provider(mut self, key_provider: Arc<dyn KeyProvider>) -> Self {
        self.key_provider = Some(key_provider);
        self
    }

    /// Sets the host-supplied retained-at stamp used by subsequent writes.
    ///
    /// Traverse does not read the OS clock; hosts must supply RFC3339 instants
    /// when age-based retention is desired.
    pub fn set_write_retained_at(&mut self, retained_at: Option<String>) {
        self.write_retained_at = retained_at;
    }

    /// Returns the store root path for host-owned maintenance construction.
    #[must_use]
    pub fn root(&self) -> &PathBuf {
        &self.root
    }

    fn path_for_key(&self, key: &str) -> Result<PathBuf, DataStoreError> {
        validate_key(key)?;
        Ok(self.root.join(format!("{key}.json")))
    }

    fn temporary_path_for_key(&self, key: &str) -> PathBuf {
        self.root.join(format!(".{key}.{}.tmp", std::process::id()))
    }

    fn sync_root(&self) -> Result<(), DataStoreError> {
        File::open(&self.root)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| durability_error(&self.root, "parent_directory", &error))
    }
}

impl DataStore for LocalFileDataStore {
    fn read(&self, key: &str) -> Result<Option<StateRecord>, DataStoreError> {
        let path = self.path_for_key(key)?;
        if !path.exists() {
            return Ok(None);
        }
        let text =
            fs::read_to_string(&path).map_err(|error| io_error("read state record", &error))?;
        let value: Value =
            serde_json::from_str(&text).map_err(|_| integrity_error("malformed_envelope"))?;
        if value.get("format").is_none() {
            return Err(integrity_error("legacy_unverified"));
        }
        let envelope: LocalDataStoreEnvelope = match value.get("format").and_then(Value::as_str) {
            Some(LOCAL_DATA_STORE_FORMAT) => {
                serde_json::from_value(value).map_err(|_| integrity_error("malformed_envelope"))?
            }
            Some(LOCAL_DATA_STORE_V2_FORMAT) => decode_v2_envelope(value)?,
            _ => return Err(integrity_error("unknown_format_version")),
        };
        match envelope.classification {
            LocalDataClassification::Public => {
                let record = envelope
                    .record
                    .ok_or_else(|| integrity_error("malformed_envelope"))?;
                if envelope.key_id.is_some()
                    || envelope.nonce.is_some()
                    || envelope.ciphertext.is_some()
                    || envelope.record_key.is_some()
                {
                    return Err(integrity_error("malformed_envelope"));
                }
                let expected_digest = digest_for_record(&record)?;
                if envelope.digest != expected_digest {
                    return Err(integrity_error("digest_mismatch"));
                }
                Ok(Some(record))
            }
            LocalDataClassification::Private => self.decrypt_private_envelope(key, envelope),
        }
    }

    fn write(&mut self, record: StateRecord) -> Result<(), DataStoreError> {
        let path = self.path_for_key(&record.key)?;
        let write_v2 = self.ensure_classification_unchanged(&path)?;
        let record_key = record.key.clone();
        let envelope = match self.classification {
            LocalDataClassification::Public => LocalDataStoreEnvelope {
                format: LOCAL_DATA_STORE_FORMAT.to_string(),
                classification: self.classification,
                digest: digest_for_record(&record)?,
                record: Some(record),
                record_key: None,
                key_id: None,
                nonce: None,
                ciphertext: None,
                retained_at: self.write_retained_at.clone(),
            },
            LocalDataClassification::Private => self.encrypt_private_record(record)?,
        };
        let text = if write_v2 {
            serde_json::to_vec(&v2_envelope(envelope)?)
        } else {
            serde_json::to_vec(&envelope)
        }
        .map_err(|error| serialization_error("serialize state record envelope", &error))?;
        let temporary_path = self.temporary_path_for_key(&record_key);
        let write_result = (|| {
            let mut temporary_file = File::create(&temporary_path)
                .map_err(|error| io_error("create temporary state record", &error))?;
            temporary_file
                .write_all(&text)
                .map_err(|error| io_error("write temporary state record", &error))?;
            temporary_file
                .sync_all()
                .map_err(|error| durability_error(&self.root, "temporary_file", &error))?;
            fs::rename(&temporary_path, &path)
                .map_err(|error| io_error("atomically commit state record", &error))?;
            self.sync_root()
        })();
        discard_temporary_record(&temporary_path);
        write_result
    }

    fn delete(&mut self, key: &str) -> Result<(), DataStoreError> {
        let path = self.path_for_key(key)?;
        match fs::remove_file(path) {
            Ok(()) => self.sync_root(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(io_error("delete state record", &error)),
        }
    }

    fn list_keys(&self) -> Result<Vec<String>, DataStoreError> {
        let mut keys = Vec::new();
        for entry in
            fs::read_dir(&self.root).map_err(|error| io_error("list state keys", &error))?
        {
            let entry = entry.map_err(|error| io_error("read state key entry", &error))?;
            let path = entry.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
                continue;
            }
            if let Some(key) = path.file_stem().and_then(|stem| stem.to_str()) {
                keys.push(key.to_string());
            }
        }
        keys.sort();
        Ok(keys)
    }
}

impl LocalFileDataStore {
    #[cfg(feature = "datastore-encryption")]
    fn encrypt_private_record(
        &self,
        record: StateRecord,
    ) -> Result<LocalDataStoreEnvelope, DataStoreError> {
        let provider = self.required_key_provider()?;
        let key_id = provider.active_key_id().map_err(map_key_provider_error)?;
        let key = provider.key_for(&key_id).map_err(map_key_provider_error)?;
        let plaintext = Zeroizing::new(
            serde_json::to_vec(&record)
                .map_err(|error| serialization_error("serialize private state record", &error))?,
        );
        let cipher = Aes256Gcm::new_from_slice(key.as_ref()).map_err(|_| crypto_error())?;
        let nonce = fresh_aes_nonce();
        let aad = private_record_aad(&key_id, &record.key);
        let ciphertext = cipher
            .encrypt(
                &nonce,
                Payload {
                    msg: plaintext.as_ref(),
                    aad: &aad,
                },
            )
            .map_err(|_| crypto_error())?;
        let nonce = hex_encode(&nonce);
        let ciphertext = hex_encode(&ciphertext);
        let digest = digest_for_private_envelope(&key_id, &record.key, &nonce, &ciphertext);
        Ok(LocalDataStoreEnvelope {
            format: LOCAL_DATA_STORE_FORMAT.to_string(),
            classification: LocalDataClassification::Private,
            digest,
            record: None,
            record_key: Some(record.key),
            key_id: Some(key_id),
            nonce: Some(nonce),
            ciphertext: Some(ciphertext),
            retained_at: self.write_retained_at.clone(),
        })
    }

    #[cfg(not(feature = "datastore-encryption"))]
    fn encrypt_private_record(
        &self,
        _record: StateRecord,
    ) -> Result<LocalDataStoreEnvelope, DataStoreError> {
        Err(encryption_feature_disabled())
    }

    #[cfg(feature = "datastore-encryption")]
    fn decrypt_private_envelope(
        &self,
        requested_key: &str,
        envelope: LocalDataStoreEnvelope,
    ) -> Result<Option<StateRecord>, DataStoreError> {
        if envelope.record.is_some() {
            return Err(integrity_error("malformed_envelope"));
        }
        let record_key = envelope
            .record_key
            .ok_or_else(|| integrity_error("malformed_envelope"))?;
        let key_id = envelope
            .key_id
            .ok_or_else(|| integrity_error("malformed_envelope"))?;
        let nonce_hex = envelope
            .nonce
            .ok_or_else(|| integrity_error("malformed_envelope"))?;
        let ciphertext_hex = envelope
            .ciphertext
            .ok_or_else(|| integrity_error("malformed_envelope"))?;
        if record_key != requested_key {
            return Err(integrity_error("record_key_mismatch"));
        }
        let expected_digest =
            digest_for_private_envelope(&key_id, &record_key, &nonce_hex, &ciphertext_hex);
        if envelope.digest != expected_digest {
            return Err(integrity_error("digest_mismatch"));
        }
        let provider = self.required_key_provider()?;
        let key = provider.key_for(&key_id).map_err(map_key_provider_error)?;
        let nonce_bytes = hex_decode(&nonce_hex)?;
        let nonce = Nonce::try_from(nonce_bytes.as_slice())
            .map_err(|_| integrity_error("invalid_nonce"))?;
        let ciphertext = hex_decode(&ciphertext_hex)?;
        let cipher = Aes256Gcm::new_from_slice(key.as_ref()).map_err(|_| crypto_error())?;
        let aad = private_record_aad(&key_id, &record_key);
        let plaintext = Zeroizing::new(
            cipher
                .decrypt(
                    &nonce,
                    Payload {
                        msg: &ciphertext,
                        aad: &aad,
                    },
                )
                .map_err(|_| integrity_error("authentication_failed"))?,
        );
        let record: StateRecord = serde_json::from_slice(&plaintext)
            .map_err(|_| integrity_error("malformed_plaintext"))?;
        if record.key != record_key {
            return Err(integrity_error("record_key_mismatch"));
        }
        Ok(Some(record))
    }

    #[cfg(not(feature = "datastore-encryption"))]
    fn decrypt_private_envelope(
        &self,
        _requested_key: &str,
        _envelope: LocalDataStoreEnvelope,
    ) -> Result<Option<StateRecord>, DataStoreError> {
        Err(encryption_feature_disabled())
    }

    #[cfg(feature = "datastore-encryption")]
    fn required_key_provider(&self) -> Result<&dyn KeyProvider, DataStoreError> {
        self.key_provider.as_deref().ok_or_else(|| {
            data_store_error(
                DataStoreErrorCode::KeyProviderRequired,
                "key_provider_required",
                json!({ "reason": "private_record" }),
            )
        })
    }

    fn ensure_classification_unchanged(&self, path: &PathBuf) -> Result<bool, DataStoreError> {
        if !path.exists() {
            return Ok(false);
        }
        let bytes = fs::read(path).map_err(|error| io_error("read existing envelope", &error))?;
        let value: Value =
            serde_json::from_slice(&bytes).map_err(|_| integrity_error("malformed_envelope"))?;
        let write_v2 =
            value.get("format").and_then(Value::as_str) == Some(LOCAL_DATA_STORE_V2_FORMAT);
        let envelope = if write_v2 {
            decode_v2_envelope(value)?
        } else {
            serde_json::from_value(value).map_err(|_| integrity_error("malformed_envelope"))?
        };
        if envelope.format != LOCAL_DATA_STORE_FORMAT {
            return Err(integrity_error("unknown_format_version"));
        }
        if envelope.classification != self.classification {
            return Err(data_store_error(
                DataStoreErrorCode::ClassificationChangeNotAllowed,
                "classification_change_not_allowed",
                json!({ "reason": "delete_before_reclassifying" }),
            ));
        }
        Ok(write_v2)
    }
}

fn digest_bytes(bytes: &[u8]) -> String {
    format!("sha256:{}", hex_encode(&Sha256::digest(bytes)))
}

fn digest_for_record(record: &StateRecord) -> Result<String, DataStoreError> {
    let canonical = serde_json::to_vec(record)
        .map_err(|error| serialization_error("serialize canonical state record", &error))?;
    let digest = Sha256::digest(canonical);
    let mut hexadecimal = String::with_capacity(digest.len() * 2);
    for byte in digest {
        hexadecimal.push(char::from(HEXADECIMAL_DIGITS[usize::from(byte >> 4)]));
        hexadecimal.push(char::from(HEXADECIMAL_DIGITS[usize::from(byte & 0x0f)]));
    }
    Ok(format!("sha256:{hexadecimal}"))
}

fn digest_for_private_envelope(
    key_id: &str,
    record_key: &str,
    nonce: &str,
    ciphertext: &str,
) -> String {
    let mut hasher = Sha256::new();
    update_length_prefixed(&mut hasher, key_id.as_bytes());
    update_length_prefixed(&mut hasher, record_key.as_bytes());
    update_length_prefixed(&mut hasher, b"private");
    update_length_prefixed(&mut hasher, nonce.as_bytes());
    update_length_prefixed(&mut hasher, ciphertext.as_bytes());
    format!("sha256:{}", hex_encode(&hasher.finalize()))
}

#[cfg(feature = "datastore-encryption")]
fn private_record_aad(key_id: &str, record_key: &str) -> Vec<u8> {
    let mut aad = Vec::new();
    append_length_prefixed(&mut aad, key_id.as_bytes());
    append_length_prefixed(&mut aad, record_key.as_bytes());
    append_length_prefixed(&mut aad, b"private");
    aad
}

fn update_length_prefixed(hasher: &mut Sha256, value: &[u8]) {
    hasher.update(value.len().to_le_bytes());
    hasher.update(value);
}

#[cfg(feature = "datastore-encryption")]
fn append_length_prefixed(output: &mut Vec<u8>, value: &[u8]) {
    output.extend_from_slice(&value.len().to_le_bytes());
    output.extend_from_slice(value);
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut hexadecimal = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        hexadecimal.push(char::from(HEXADECIMAL_DIGITS[usize::from(byte >> 4)]));
        hexadecimal.push(char::from(HEXADECIMAL_DIGITS[usize::from(byte & 0x0f)]));
    }
    hexadecimal
}

fn hex_decode(value: &str) -> Result<Vec<u8>, DataStoreError> {
    if !value.len().is_multiple_of(2) {
        return Err(integrity_error("invalid_hex"));
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let high = decode_hex_digit(pair[0]).ok_or_else(|| integrity_error("invalid_hex"))?;
            let low = decode_hex_digit(pair[1]).ok_or_else(|| integrity_error("invalid_hex"))?;
            Ok((high << 4) | low)
        })
        .collect()
}

fn decode_hex_digit(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        _ => None,
    }
}

#[cfg(feature = "datastore-encryption")]
fn map_key_provider_error(error: KeyProviderError) -> DataStoreError {
    let code = match error.code {
        KeyProviderErrorCode::MissingKey => DataStoreErrorCode::KeyNotFound,
        KeyProviderErrorCode::ExpiredKeyId => DataStoreErrorCode::KeyExpired,
        KeyProviderErrorCode::ProviderFailure => DataStoreErrorCode::KeyProviderFailure,
    };
    let message = error.message;
    data_store_error(
        code,
        &message,
        json!({ "provider_error_code": key_provider_error_code(error.code) }),
    )
}

#[cfg(feature = "datastore-encryption")]
fn key_provider_error_code(code: KeyProviderErrorCode) -> &'static str {
    match code {
        KeyProviderErrorCode::MissingKey => "missing_key",
        KeyProviderErrorCode::ExpiredKeyId => "expired_key_id",
        KeyProviderErrorCode::ProviderFailure => "provider_failure",
    }
}

#[cfg(feature = "datastore-encryption")]
fn crypto_error() -> DataStoreError {
    data_store_error(
        DataStoreErrorCode::CryptoFailure,
        "crypto_failed",
        json!({ "reason": "encryption_failed" }),
    )
}

#[cfg(not(feature = "datastore-encryption"))]
fn encryption_feature_disabled() -> DataStoreError {
    data_store_error(
        DataStoreErrorCode::KeyProviderRequired,
        "key_provider_required",
        json!({ "reason": "datastore_encryption_feature_disabled" }),
    )
}

#[cfg(feature = "datastore-encryption")]
fn fresh_aes_nonce() -> Nonce<U12> {
    // Host-local UUID entropy avoids aes-gcm's getrandom Generate path so
    // wasm32 `--no-default-features` checks do not require a getrandom backend.
    let entropy = *uuid::Uuid::new_v4().as_bytes();
    let mut nonce_bytes = [0_u8; 12];
    nonce_bytes.copy_from_slice(&entropy[..12]);
    Nonce::<U12>::from(nonce_bytes)
}

fn lock_error(error: TryLockError) -> DataStoreError {
    match error {
        TryLockError::WouldBlock => data_store_error(
            DataStoreErrorCode::StoreLocked,
            "store_locked",
            json!({ "reason": "exclusive_owner_active" }),
        ),
        TryLockError::Error(error) if error.kind() == std::io::ErrorKind::Unsupported => {
            data_store_error(
                DataStoreErrorCode::IoFailure,
                "storage_io_failed",
                json!({ "operation": "acquire_lock", "reason": "locking_unsupported" }),
            )
        }
        TryLockError::Error(_) => data_store_error(
            DataStoreErrorCode::IoFailure,
            "storage_io_failed",
            json!({ "operation": "acquire_lock", "reason": "lock_acquisition_failed" }),
        ),
    }
}

fn durability_error(root: &PathBuf, stage: &str, error: &std::io::Error) -> DataStoreError {
    data_store_error(
        DataStoreErrorCode::DurabilityCommitFailed,
        "durability_commit_failed",
        json!({ "root": root, "stage": stage, "reason": error.to_string() }),
    )
}

fn discard_temporary_record(path: &PathBuf) {
    if path.exists() {
        let _ignored = fs::remove_file(path).is_ok();
    }
}

fn integrity_error(reason: &str) -> DataStoreError {
    data_store_error(
        DataStoreErrorCode::IntegrityCheckFailed,
        "integrity_check_failed",
        json!({ "reason": reason }),
    )
}

/// Validates a capability state write against the contract-declared state schema.
///
/// # Errors
///
/// Returns [`DataStoreError`] when the key is invalid, the contract does not
/// declare a state schema, the key is not declared by the schema, or the value
/// does not match the declared key schema.
pub fn validate_state_write(
    contract: &CapabilityContract,
    key: &str,
    value: &Value,
) -> Result<(), DataStoreError> {
    validate_key(key)?;
    let schema = contract.state_schema.as_ref().ok_or_else(|| {
        data_store_error(
            DataStoreErrorCode::NoStateSchemaDeclared,
            "no_state_schema_declared",
            json!({ "capability_id": contract.id, "key": key }),
        )
    })?;
    let property_schema = schema
        .get("properties")
        .and_then(Value::as_object)
        .and_then(|properties| properties.get(key))
        .ok_or_else(|| {
            data_store_error(
                DataStoreErrorCode::SchemaValidationError,
                "schema_validation_error",
                json!({ "key": key, "reason": "state key is not declared in schema" }),
            )
        })?;
    let mut violations = Vec::new();
    crate::validate_value_against_schema(value, property_schema, "$", &mut violations);
    if violations.is_empty() {
        Ok(())
    } else {
        Err(data_store_error(
            DataStoreErrorCode::SchemaValidationError,
            "schema_validation_error",
            json!({ "key": key, "violations": violations }),
        ))
    }
}

fn sync_adapters(
    local: &mut dyn DataStore,
    remote: &mut dyn DataStore,
) -> Result<SyncReport, DataStoreError> {
    let keys = merged_keys(local.list_keys()?, remote.list_keys()?);
    let mut decisions = Vec::new();
    let mut snapshots = BTreeMap::new();

    for key in keys {
        let local_record = local.read(&key)?;
        let remote_record = remote.read(&key)?;
        snapshots.insert(key.clone(), local_record.clone());
        let Some((winner, rule)) = merge_records(local_record.as_ref(), remote_record.as_ref())
        else {
            continue;
        };
        apply_winner(local, remote, &key, &winner).map_err(|error| {
            rollback_local(local, &snapshots);
            data_store_error(
                DataStoreErrorCode::SyncFailure,
                "sync failed; local state restored",
                json!({ "key": key, "cause": error.message }),
            )
        })?;
        decisions.push(MergeDecision {
            key,
            winning_writer_id: winner.writer_id,
            winning_lamport_clock: winner.lamport_clock,
            resolution_rule: rule,
        });
    }

    Ok(SyncReport {
        governing_spec: DATA_STORE_SPEC.to_string(),
        decisions,
    })
}

fn merged_keys(local_keys: Vec<String>, remote_keys: Vec<String>) -> Vec<String> {
    local_keys
        .into_iter()
        .chain(remote_keys)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn merge_records(
    local: Option<&StateRecord>,
    remote: Option<&StateRecord>,
) -> Option<(StateRecord, ConflictResolutionRule)> {
    match (local, remote) {
        (Some(record), None) => Some((record.clone(), ConflictResolutionRule::OnlyLocal)),
        (None, Some(record)) => Some((record.clone(), ConflictResolutionRule::OnlyRemote)),
        (Some(local), Some(remote)) => Some(select_conflict_winner(local, remote)),
        (None, None) => None,
    }
}

fn select_conflict_winner(
    local: &StateRecord,
    remote: &StateRecord,
) -> (StateRecord, ConflictResolutionRule) {
    if local.lamport_clock > remote.lamport_clock {
        return (local.clone(), ConflictResolutionRule::HigherLamportClock);
    }
    if remote.lamport_clock > local.lamport_clock {
        return (remote.clone(), ConflictResolutionRule::HigherLamportClock);
    }
    if local.writer_id >= remote.writer_id {
        (
            local.clone(),
            ConflictResolutionRule::WriterIdentityTieBreak,
        )
    } else {
        (
            remote.clone(),
            ConflictResolutionRule::WriterIdentityTieBreak,
        )
    }
}

fn apply_winner(
    local: &mut dyn DataStore,
    remote: &mut dyn DataStore,
    key: &str,
    winner: &StateRecord,
) -> Result<(), DataStoreError> {
    if local.read(key)?.as_ref() != Some(winner) {
        local.write(winner.clone())?;
    }
    if remote.read(key)?.as_ref() != Some(winner) {
        remote.write(winner.clone())?;
    }
    Ok(())
}

fn rollback_local(local: &mut dyn DataStore, snapshots: &BTreeMap<String, Option<StateRecord>>) {
    for (key, snapshot) in snapshots {
        let result = match snapshot {
            Some(record) => local.write(record.clone()),
            None => local.delete(key),
        };
        let _ignored = result.is_ok();
    }
}

fn validate_key(key: &str) -> Result<(), DataStoreError> {
    let valid = !key.is_empty()
        && key
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'));
    if valid {
        Ok(())
    } else {
        Err(data_store_error(
            DataStoreErrorCode::InvalidKey,
            "state key must be non-empty and contain only ASCII letters, numbers, '_' or '-'",
            json!({ "key": key }),
        ))
    }
}

fn data_store_error(code: DataStoreErrorCode, message: &str, details: Value) -> DataStoreError {
    DataStoreError {
        code,
        message: message.to_string(),
        details,
    }
}

fn io_error(action: &str, error: &std::io::Error) -> DataStoreError {
    data_store_error(
        DataStoreErrorCode::IoFailure,
        "storage_io_failed",
        json!({ "action": action, "reason": error.to_string() }),
    )
}

fn serialization_error(action: &str, error: &serde_json::Error) -> DataStoreError {
    data_store_error(
        DataStoreErrorCode::SerializationFailure,
        action,
        json!({ "error": error.to_string() }),
    )
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::cell::Cell;
    use std::path::Path;
    use std::process::{Child, Command, Stdio};
    use std::thread;
    use std::time::Duration;
    use traverse_contracts::{
        BinaryFormat, CapabilityContract, Condition, DependencyReference, Entrypoint,
        EntrypointKind, EventReference, Execution, ExecutionConstraints, ExecutionTarget,
        FilesystemAccess, HostApiAccess, IdReference, Lifecycle, NetworkAccess, Owner, Provenance,
        ProvenanceSource, SchemaContainer, ServiceType, SideEffect, SideEffectKind,
        ValidationEvidence,
    };
    use uuid::Uuid;

    #[derive(Debug, Clone, Default)]
    struct MemoryDataStore {
        records: BTreeMap<String, StateRecord>,
        fail_writes: Cell<bool>,
    }

    #[derive(Debug, Clone, Default)]
    struct PhantomKeyStore;

    struct FailingKeyProvider;

    impl KeyProvider for FailingKeyProvider {
        fn active_key_id(&self) -> Result<String, KeyProviderError> {
            Err(KeyProviderError::provider_failure())
        }

        fn key_for(&self, _key_id: &str) -> Result<Zeroizing<[u8; 32]>, KeyProviderError> {
            Err(KeyProviderError::provider_failure())
        }
    }

    impl DataStore for MemoryDataStore {
        fn read(&self, key: &str) -> Result<Option<StateRecord>, DataStoreError> {
            Ok(self.records.get(key).cloned())
        }

        fn write(&mut self, record: StateRecord) -> Result<(), DataStoreError> {
            if self.fail_writes.get() {
                return Err(data_store_error(
                    DataStoreErrorCode::IoFailure,
                    "forced write failure",
                    json!({ "key": record.key }),
                ));
            }
            self.records.insert(record.key.clone(), record);
            Ok(())
        }

        fn delete(&mut self, key: &str) -> Result<(), DataStoreError> {
            self.records.remove(key);
            Ok(())
        }

        fn list_keys(&self) -> Result<Vec<String>, DataStoreError> {
            Ok(self.records.keys().cloned().collect())
        }
    }

    impl DataStore for PhantomKeyStore {
        fn read(&self, _key: &str) -> Result<Option<StateRecord>, DataStoreError> {
            Ok(None)
        }

        fn write(&mut self, _record: StateRecord) -> Result<(), DataStoreError> {
            Ok(())
        }

        fn delete(&mut self, _key: &str) -> Result<(), DataStoreError> {
            Ok(())
        }

        fn list_keys(&self) -> Result<Vec<String>, DataStoreError> {
            Ok(vec!["phantom".to_string()])
        }
    }

    #[test]
    fn runtime_data_store_validates_writes_and_reads_from_local_file_adapter() {
        let root = temp_root("valid");
        let adapter = public_store(&root);
        let mut store = RuntimeDataStore::new(adapter, "writer-a");
        let contract = stateful_contract(Some(json!({
            "type": "object",
            "properties": {
                "draft": {"type": "string"}
            }
        })));

        let record = store
            .write(&contract, "draft", json!("ready"))
            .expect("valid state write should succeed");

        assert_eq!(record.lamport_clock, 1);
        assert_eq!(
            store.read(&contract, "draft").expect("read should succeed"),
            Some(json!("ready"))
        );
        assert_eq!(
            store.list_keys().expect("list should succeed"),
            vec!["draft".to_string()]
        );
        store.delete("draft").expect("delete should succeed");
        assert_eq!(
            store.read(&contract, "draft").expect("read should succeed"),
            None
        );
    }

    #[test]
    fn runtime_data_store_rejects_missing_schema_bad_keys_and_schema_violations() {
        let adapter = MemoryDataStore::default();
        let mut store = RuntimeDataStore::new(adapter, "writer-a");
        let no_schema = stateful_contract(None);
        let schema = stateful_contract(Some(json!({
            "type": "object",
            "properties": {
                "count": {"type": "integer"}
            }
        })));

        let missing = store
            .write(&no_schema, "count", json!(1))
            .expect_err("missing state schema should fail");
        assert_eq!(missing.code, DataStoreErrorCode::NoStateSchemaDeclared);

        let invalid_key = store
            .write(&schema, "bad.key", json!(1))
            .expect_err("invalid key should fail");
        assert_eq!(invalid_key.code, DataStoreErrorCode::InvalidKey);

        let undeclared = store
            .write(&schema, "other", json!(1))
            .expect_err("undeclared state key should fail");
        assert_eq!(undeclared.code, DataStoreErrorCode::SchemaValidationError);

        let wrong_type = store
            .write(&schema, "count", json!("one"))
            .expect_err("wrong state type should fail");
        assert_eq!(wrong_type.code, DataStoreErrorCode::SchemaValidationError);

        let no_schema_read = store
            .read(&no_schema, "count")
            .expect("no-schema read should succeed");
        assert_eq!(no_schema_read, None);

        let bad_read_key = store
            .read(&schema, "bad.key")
            .expect_err("invalid read key should fail");
        assert_eq!(bad_read_key.code, DataStoreErrorCode::InvalidKey);
    }

    #[test]
    fn lamport_clock_overflow_is_rejected_before_adapter_write() {
        let adapter = MemoryDataStore::default();
        let clock = LamportClock::with_value("writer-a", u64::MAX);
        let mut store = RuntimeDataStore::with_clock(adapter, clock);
        let contract = stateful_contract(Some(json!({
            "type": "object",
            "properties": {
                "draft": {"type": "string"}
            }
        })));

        let error = store
            .write(&contract, "draft", json!("ready"))
            .expect_err("overflow should fail");

        assert_eq!(error.code, DataStoreErrorCode::LamportClockOverflow);
        assert!(store.into_inner().records.is_empty());
    }

    #[test]
    fn runtime_data_store_validates_reads_before_returning_stored_values() {
        let mut adapter = MemoryDataStore::default();
        adapter
            .write(record("count", "writer-a", 1, json!("not an integer")))
            .expect("seed should succeed");
        let store = RuntimeDataStore::new(adapter, "writer-a");
        let contract = stateful_contract(Some(json!({
            "type": "object",
            "properties": {
                "count": {"type": "integer"}
            }
        })));

        let error = store
            .read(&contract, "count")
            .expect_err("invalid stored value should fail");

        assert_eq!(error.code, DataStoreErrorCode::SchemaValidationError);
    }

    #[test]
    fn reconnect_sync_merges_only_local_only_remote_clock_winner_and_writer_tie_breaks() {
        let mut local = MemoryDataStore::default();
        let mut remote = MemoryDataStore::default();
        local
            .write(record("local_only", "local-a", 1, json!("local")))
            .expect("local write should succeed");
        remote
            .write(record("remote_only", "remote-a", 1, json!("remote")))
            .expect("remote write should succeed");
        local
            .write(record("clock", "local-a", 2, json!("old")))
            .expect("local write should succeed");
        remote
            .write(record("clock", "remote-a", 3, json!("new")))
            .expect("remote write should succeed");
        local
            .write(record("tie", "writer-z", 4, json!("winner")))
            .expect("local write should succeed");
        remote
            .write(record("tie", "writer-a", 4, json!("loser")))
            .expect("remote write should succeed");

        let report = sync_adapters(&mut local, &mut remote).expect("sync should succeed");

        assert_eq!(report.governing_spec, "089-datastore-synchronization");
        assert_eq!(report.decisions.len(), 4);
        assert_eq!(
            local.read("remote_only").expect("read should succeed"),
            remote.read("remote_only").expect("read should succeed")
        );
        assert_eq!(
            local.read("clock").expect("read should succeed"),
            Some(record("clock", "remote-a", 3, json!("new")))
        );
        assert_eq!(
            remote.read("tie").expect("read should succeed"),
            Some(record("tie", "writer-z", 4, json!("winner")))
        );
        assert!(
            report
                .decisions
                .iter()
                .any(|decision| decision.resolution_rule
                    == ConflictResolutionRule::WriterIdentityTieBreak)
        );
    }

    #[test]
    fn sync_failure_restores_local_snapshot() {
        let mut local = MemoryDataStore::default();
        let mut remote = MemoryDataStore::default();
        local
            .write(record("shared", "local-a", 2, json!("local")))
            .expect("local write should succeed");
        remote
            .write(record("shared", "remote-a", 1, json!("remote")))
            .expect("remote write should succeed");
        remote.fail_writes.set(true);

        let error = sync_adapters(&mut local, &mut remote).expect_err("sync should fail");

        assert_eq!(error.code, DataStoreErrorCode::SyncFailure);
        assert_eq!(
            local.read("shared").expect("read should succeed"),
            Some(record("shared", "local-a", 2, json!("local")))
        );
    }

    #[test]
    fn local_file_adapter_reports_bad_keys_and_bad_json() {
        let root = temp_root("bad-json");
        let adapter = LocalFileDataStore::new(&root).expect("local adapter should initialize");
        let invalid = adapter
            .read("bad.key")
            .expect_err("invalid key should fail");
        assert_eq!(invalid.code, DataStoreErrorCode::InvalidKey);

        fs::write(root.join("broken.json"), "{").expect("bad json fixture should write");
        let invalid_json = adapter
            .read("broken")
            .expect_err("invalid json should fail");
        assert_eq!(invalid_json.code, DataStoreErrorCode::IntegrityCheckFailed);
        assert_eq!(invalid_json.message, "integrity_check_failed");
    }

    #[test]
    fn helper_paths_cover_remaining_datastore_branches() {
        let mut local = RuntimeDataStore::new(MemoryDataStore::default(), "local-a");
        let mut remote = MemoryDataStore::default();
        remote
            .write(record("remote_only", "remote-a", 1, json!("remote")))
            .expect("remote seed should succeed");

        let report = local
            .sync_on_reconnect(&mut remote)
            .expect("public reconnect sync should succeed");
        assert_eq!(report.decisions.len(), 1);

        assert!(merge_records(None, None).is_none());
        let (_winner, rule) = select_conflict_winner(
            &record("tie", "writer-a", 1, json!("local")),
            &record("tie", "writer-z", 1, json!("remote")),
        );
        assert_eq!(rule, ConflictResolutionRule::WriterIdentityTieBreak);

        let mut failing_local = MemoryDataStore::default();
        failing_local.fail_writes.set(true);
        let mut seeded_remote = MemoryDataStore::default();
        seeded_remote
            .write(record("missing_local", "remote-a", 1, json!("remote")))
            .expect("remote seed should succeed");
        let error =
            sync_adapters(&mut failing_local, &mut seeded_remote).expect_err("sync should fail");
        assert_eq!(error.code, DataStoreErrorCode::SyncFailure);
        assert_eq!(
            failing_local
                .delete("missing_local")
                .expect("delete should succeed"),
            ()
        );

        let mut phantom_local = PhantomKeyStore;
        let mut phantom_remote = PhantomKeyStore;
        assert!(
            sync_adapters(&mut phantom_local, &mut phantom_remote)
                .expect("phantom sync should succeed")
                .decisions
                .is_empty()
        );
        phantom_local
            .write(record("phantom", "writer-a", 1, json!("value")))
            .expect("phantom write should succeed");
        phantom_local
            .delete("phantom")
            .expect("phantom delete should succeed");

        let root = temp_root("listing");
        fs::create_dir_all(&root).expect("root should be created");
        fs::write(root.join("skip.txt"), "not state").expect("non-json fixture should write");
        let adapter = LocalFileDataStore::new(&root).expect("local adapter should initialize");
        assert!(adapter.list_keys().expect("list should succeed").is_empty());
        drop(adapter);
        let mut delete_missing =
            LocalFileDataStore::new(&root).expect("local adapter should initialize");
        delete_missing
            .delete("missing")
            .expect("missing delete should succeed");
        fs::create_dir(root.join("cant_delete.json")).expect("directory fixture should write");
        let delete_failure = delete_missing
            .delete("cant_delete")
            .expect_err("directory delete should fail");
        assert_eq!(delete_failure.code, DataStoreErrorCode::IoFailure);

        let file_root = temp_root("file-root");
        fs::write(&file_root, "not a directory").expect("file root fixture should write");
        let io_failure = LocalFileDataStore::new(&file_root).expect_err("file root should fail");
        assert_eq!(io_failure.code, DataStoreErrorCode::IoFailure);
    }

    #[test]
    fn local_file_adapter_writes_integrity_envelope_and_reopens() {
        let root = temp_root("integrity-envelope");
        let record = record("draft", "writer-a", 1, json!("ready"));
        let mut adapter =
            LocalFileDataStore::with_classification(&root, LocalDataClassification::Public)
                .expect("local adapter should initialize");
        adapter.write(record.clone()).expect("write should succeed");

        let envelope: LocalDataStoreEnvelope = serde_json::from_slice(
            &fs::read(root.join("draft.json")).expect("envelope should be present"),
        )
        .expect("envelope should deserialize");
        assert_eq!(envelope.format, LOCAL_DATA_STORE_FORMAT);
        assert_eq!(envelope.classification, LocalDataClassification::Public);
        assert_eq!(
            envelope.digest,
            digest_for_record(&record).expect("digest should compute")
        );

        drop(adapter);
        let reopened = LocalFileDataStore::new(&root).expect("reopen should acquire lock");
        assert_eq!(
            reopened.read("draft").expect("read should succeed"),
            Some(record)
        );
    }

    #[test]
    fn private_records_encrypt_reopen_and_require_provider() {
        let root = temp_root("private-encryption");
        let provider: Arc<dyn KeyProvider> = Arc::new(InMemoryKeyProvider::new("key-1", [7; 32]));
        let original = record("secret", "writer-a", 1, json!("classified value"));
        let mut store = LocalFileDataStore::new(&root)
            .expect("private store should open")
            .with_key_provider(Arc::clone(&provider));
        store.write(original.clone()).expect("private write");

        let bytes = fs::read(root.join("secret.json")).expect("private envelope");
        let text = String::from_utf8(bytes.clone()).expect("json utf8");
        let envelope: LocalDataStoreEnvelope =
            serde_json::from_slice(&bytes).expect("private envelope shape");
        assert_eq!(envelope.classification, LocalDataClassification::Private);
        assert!(envelope.record.is_none());
        assert_eq!(envelope.record_key.as_deref(), Some("secret"));
        assert_eq!(envelope.key_id.as_deref(), Some("key-1"));
        assert!(envelope.nonce.is_some());
        assert!(envelope.ciphertext.is_some());
        assert!(!text.contains("classified value"));
        assert!(!text.contains(&hex_encode(&[7; 32])));

        let first_nonce = envelope.nonce;
        store.write(original.clone()).expect("second private write");
        let second: LocalDataStoreEnvelope =
            serde_json::from_slice(&fs::read(root.join("secret.json")).expect("second envelope"))
                .expect("second envelope shape");
        assert_ne!(first_nonce, second.nonce);
        drop(store);

        let reopened = LocalFileDataStore::new(&root)
            .expect("private store should reopen")
            .with_key_provider(Arc::clone(&provider));
        assert_eq!(
            reopened.read("secret").expect("private read"),
            Some(original)
        );
        drop(reopened);

        let no_provider = LocalFileDataStore::new(&root).expect("store without provider opens");
        let read_error = no_provider
            .read("secret")
            .expect_err("private read must require provider");
        assert_eq!(read_error.code, DataStoreErrorCode::KeyProviderRequired);
    }

    #[test]
    fn private_write_without_provider_fails_before_commit_and_public_crud_works() {
        let private_root = temp_root("private-provider-required");
        let mut private =
            LocalFileDataStore::new(&private_root).expect("private store should open");
        let error = private
            .write(record("secret", "writer-a", 1, json!("value")))
            .expect_err("private write must fail");
        assert_eq!(error.code, DataStoreErrorCode::KeyProviderRequired);
        assert!(!private_root.join("secret.json").exists());

        let public_root = temp_root("public-without-provider");
        let mut public = public_store(&public_root);
        let public_record = record("cache", "writer-a", 1, json!("visible"));
        public.write(public_record.clone()).expect("public write");
        assert_eq!(
            public.read("cache").expect("public read"),
            Some(public_record)
        );
        public.delete("cache").expect("public delete");
        assert!(public.list_keys().expect("public list").is_empty());
    }

    #[test]
    fn private_authentication_and_key_provider_failures_are_stable() {
        let root = temp_root("private-authentication");
        let key = [9; 32];
        let provider: Arc<dyn KeyProvider> =
            Arc::new(InMemoryKeyProvider::new("key-a", key).with_read_key("key-b", key));
        let mut store = LocalFileDataStore::new(&root)
            .expect("private store should open")
            .with_key_provider(Arc::clone(&provider));
        store
            .write(record("secret", "writer-a", 1, json!("value")))
            .expect("private write");

        let path = root.join("secret.json");
        let mut envelope: LocalDataStoreEnvelope =
            serde_json::from_slice(&fs::read(&path).expect("envelope")).expect("shape");
        let original_envelope = envelope.clone();
        let mut ciphertext =
            hex_decode(envelope.ciphertext.as_deref().expect("ciphertext")).expect("valid hex");
        ciphertext[0] ^= 1;
        envelope.ciphertext = Some(hex_encode(&ciphertext));
        envelope.digest = digest_for_private_envelope(
            envelope.key_id.as_deref().expect("key id"),
            envelope.record_key.as_deref().expect("record key"),
            envelope.nonce.as_deref().expect("nonce"),
            envelope.ciphertext.as_deref().expect("ciphertext"),
        );
        fs::write(&path, serde_json::to_vec(&envelope).expect("serialize")).expect("tamper");
        let ciphertext_authentication = store
            .read("secret")
            .expect_err("ciphertext tampering must fail");
        assert_eq!(
            ciphertext_authentication.code,
            DataStoreErrorCode::IntegrityCheckFailed
        );
        assert_eq!(
            ciphertext_authentication.details["reason"],
            "authentication_failed"
        );

        envelope = original_envelope;
        envelope.key_id = Some("key-b".to_string());
        envelope.digest = digest_for_private_envelope(
            "key-b",
            envelope.record_key.as_deref().expect("record key"),
            envelope.nonce.as_deref().expect("nonce"),
            envelope.ciphertext.as_deref().expect("ciphertext"),
        );
        fs::write(&path, serde_json::to_vec(&envelope).expect("serialize")).expect("tamper");
        let authentication = store.read("secret").expect_err("AAD tampering must fail");
        assert_eq!(
            authentication.code,
            DataStoreErrorCode::IntegrityCheckFailed
        );
        assert_eq!(authentication.details["reason"], "authentication_failed");
        drop(store);

        let missing_provider: Arc<dyn KeyProvider> =
            Arc::new(InMemoryKeyProvider::new("other", [3; 32]));
        let missing = LocalFileDataStore::new(&root)
            .expect("reopen")
            .with_key_provider(missing_provider)
            .read("secret")
            .expect_err("missing key must fail");
        assert_eq!(missing.code, DataStoreErrorCode::KeyNotFound);
        assert_eq!(missing.message, "key_not_found");

        let expired_provider: Arc<dyn KeyProvider> =
            Arc::new(InMemoryKeyProvider::new("key-b", key).with_expired_key_id("key-b"));
        let expired = LocalFileDataStore::new(&root)
            .expect("reopen")
            .with_key_provider(expired_provider)
            .read("secret")
            .expect_err("expired key must fail");
        assert_eq!(expired.code, DataStoreErrorCode::KeyExpired);

        let failing_provider: Arc<dyn KeyProvider> = Arc::new(FailingKeyProvider);
        let failure_root = temp_root("provider-failure");
        let provider_failure = LocalFileDataStore::new(&failure_root)
            .expect("open")
            .with_key_provider(failing_provider)
            .write(record("secret", "writer-a", 1, json!("value")))
            .expect_err("provider failure must fail");
        assert_eq!(
            provider_failure.code,
            DataStoreErrorCode::KeyProviderFailure
        );
        assert_eq!(provider_failure.message, "key_provider_failed");
    }

    #[test]
    fn classification_change_requires_delete_before_write() {
        let root = temp_root("classification-immutable");
        let mut public = public_store(&root);
        public
            .write(record("shared", "writer-a", 1, json!("public")))
            .expect("public write");
        drop(public);

        let provider: Arc<dyn KeyProvider> = Arc::new(InMemoryKeyProvider::new("key-1", [4; 32]));
        let mut private = LocalFileDataStore::new(&root)
            .expect("private reopen")
            .with_key_provider(provider);
        let rejected = private
            .write(record("shared", "writer-a", 2, json!("private")))
            .expect_err("in-place reclassification must fail");
        assert_eq!(
            rejected.code,
            DataStoreErrorCode::ClassificationChangeNotAllowed
        );
        private.delete("shared").expect("delete old classification");
        private
            .write(record("shared", "writer-a", 2, json!("private")))
            .expect("write after delete");
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn private_envelope_corruption_and_error_helpers_fail_closed() {
        let root = temp_root("private-corruption-branches");
        let key = [23; 32];
        let provider: Arc<dyn KeyProvider> = Arc::new(InMemoryKeyProvider::new("key-1", key));
        let mut store = LocalFileDataStore::new(&root)
            .expect("open")
            .with_key_provider(Arc::clone(&provider));
        store
            .write(record("secret", "writer-a", 1, json!("value")))
            .expect("write");
        let path = root.join("secret.json");
        let original: LocalDataStoreEnvelope =
            serde_json::from_slice(&fs::read(&path).expect("read")).expect("shape");

        let mut plaintext_leak = original.clone();
        plaintext_leak.record = Some(record("secret", "writer-a", 1, json!("value")));
        write_envelope_fixture(&path, &plaintext_leak);
        assert_integrity_reason(&store, "malformed_envelope");

        let mut wrong_key = original.clone();
        wrong_key.record_key = Some("other".to_string());
        wrong_key.digest = digest_for_private_envelope(
            wrong_key.key_id.as_deref().expect("key id"),
            "other",
            wrong_key.nonce.as_deref().expect("nonce"),
            wrong_key.ciphertext.as_deref().expect("ciphertext"),
        );
        write_envelope_fixture(&path, &wrong_key);
        assert_integrity_reason(&store, "record_key_mismatch");

        let mut bad_digest = original.clone();
        bad_digest.digest = "sha256:deadbeef".to_string();
        write_envelope_fixture(&path, &bad_digest);
        assert_integrity_reason(&store, "digest_mismatch");

        let mut odd_nonce = original.clone();
        odd_nonce.nonce = Some("0".to_string());
        odd_nonce.digest = digest_for_private_envelope(
            odd_nonce.key_id.as_deref().expect("key id"),
            odd_nonce.record_key.as_deref().expect("record key"),
            "0",
            odd_nonce.ciphertext.as_deref().expect("ciphertext"),
        );
        write_envelope_fixture(&path, &odd_nonce);
        assert_integrity_reason(&store, "invalid_hex");

        let mut invalid_nonce = original.clone();
        invalid_nonce.nonce = Some("00".to_string());
        invalid_nonce.digest = digest_for_private_envelope(
            invalid_nonce.key_id.as_deref().expect("key id"),
            invalid_nonce.record_key.as_deref().expect("record key"),
            "00",
            invalid_nonce.ciphertext.as_deref().expect("ciphertext"),
        );
        write_envelope_fixture(&path, &invalid_nonce);
        assert_integrity_reason(&store, "invalid_nonce");

        let mut invalid_ciphertext = original.clone();
        invalid_ciphertext.ciphertext = Some("gg".to_string());
        invalid_ciphertext.digest = digest_for_private_envelope(
            invalid_ciphertext.key_id.as_deref().expect("key id"),
            invalid_ciphertext
                .record_key
                .as_deref()
                .expect("record key"),
            invalid_ciphertext.nonce.as_deref().expect("nonce"),
            "gg",
        );
        write_envelope_fixture(&path, &invalid_ciphertext);
        assert_integrity_reason(&store, "invalid_hex");

        let malformed_plaintext = encrypted_fixture("secret", "key-1", key, b"not-json".as_slice());
        write_envelope_fixture(&path, &malformed_plaintext);
        assert_integrity_reason(&store, "malformed_plaintext");

        let mismatched_record =
            serde_json::to_vec(&record("other", "writer-a", 1, json!("value"))).expect("serialize");
        let mismatched_plaintext = encrypted_fixture("secret", "key-1", key, &mismatched_record);
        write_envelope_fixture(&path, &mismatched_plaintext);
        assert_integrity_reason(&store, "record_key_mismatch");

        let mut unknown_format = original;
        unknown_format.format = "local-datastore/unknown".to_string();
        write_envelope_fixture(&path, &unknown_format);
        let unknown = store
            .write(record("secret", "writer-a", 2, json!("new")))
            .expect_err("unknown existing format");
        assert_eq!(unknown.details["reason"], "unknown_format_version");

        assert_eq!(crypto_error().code, DataStoreErrorCode::CryptoFailure);
        assert_eq!(
            FailingKeyProvider
                .key_for("key-1")
                .expect_err("provider failure")
                .code,
            KeyProviderErrorCode::ProviderFailure
        );
        assert_eq!(KeyProviderError::missing_key().message, "key_not_found");
        for code in [
            KeyProviderErrorCode::MissingKey,
            KeyProviderErrorCode::ExpiredKeyId,
            KeyProviderErrorCode::ProviderFailure,
        ] {
            let encoded = serde_json::to_vec(&code).expect("serialize provider code");
            let decoded: KeyProviderErrorCode =
                serde_json::from_slice(&encoded).expect("deserialize provider code");
            assert_eq!(decoded, code);
        }

        let mut missing_active = InMemoryKeyProvider::new("missing", [1; 32]);
        missing_active.keys.clear();
        assert_eq!(
            missing_active
                .active_key_id()
                .expect_err("missing active key")
                .code,
            KeyProviderErrorCode::MissingKey
        );
        let expired_active =
            InMemoryKeyProvider::new("expired", [1; 32]).with_expired_key_id("expired");
        assert_eq!(
            expired_active
                .active_key_id()
                .expect_err("expired active key")
                .code,
            KeyProviderErrorCode::ExpiredKeyId
        );
        assert!(format!("{store:?}").contains("key_provider_configured"));

        let public_root = temp_root("public-extra-encryption-fields");
        let mut public = public_store(&public_root);
        public
            .write(record("cache", "writer-a", 1, json!("value")))
            .expect("public write");
        let public_path = public_root.join("cache.json");
        let mut public_envelope: LocalDataStoreEnvelope =
            serde_json::from_slice(&fs::read(&public_path).expect("read")).expect("shape");
        public_envelope.key_id = Some("unexpected".to_string());
        write_envelope_fixture(&public_path, &public_envelope);
        let malformed_public = public.read("cache").expect_err("extra private metadata");
        assert_eq!(malformed_public.details["reason"], "malformed_envelope");
    }

    #[test]
    fn local_file_adapter_rejects_tampered_and_legacy_records() {
        let root = temp_root("tampered");
        let mut adapter = public_store(&root);
        adapter
            .write(record("draft", "writer-a", 1, json!("ready")))
            .expect("write should succeed");

        let mut envelope: Value = serde_json::from_slice(
            &fs::read(root.join("draft.json")).expect("envelope should be present"),
        )
        .expect("fixture should deserialize");
        envelope["record"]["value"] = json!("tampered");
        fs::write(
            root.join("draft.json"),
            serde_json::to_vec(&envelope).expect("fixture should serialize"),
        )
        .expect("tampered fixture should write");
        let tampered = adapter.read("draft").expect_err("tampering must fail");
        assert_eq!(tampered.code, DataStoreErrorCode::IntegrityCheckFailed);
        assert_eq!(tampered.details["reason"], "digest_mismatch");

        fs::write(
            root.join("legacy.json"),
            serde_json::to_vec(&record("legacy", "writer-a", 1, json!("old")))
                .expect("legacy fixture should serialize"),
        )
        .expect("legacy fixture should write");
        let legacy = adapter.read("legacy").expect_err("legacy must fail closed");
        assert_eq!(legacy.code, DataStoreErrorCode::IntegrityCheckFailed);
        assert_eq!(legacy.details["reason"], "legacy_unverified");
    }

    #[test]
    fn local_file_adapter_ignores_temporary_records_and_rejects_second_owner() {
        let root = temp_root("temporary-and-lock");
        let mut adapter = public_store(&root);
        adapter
            .write(record("draft", "writer-a", 1, json!("committed")))
            .expect("write should succeed");
        fs::write(root.join(".draft.temporary.tmp"), "incomplete")
            .expect("temporary fixture should write");

        assert_eq!(
            adapter.list_keys().expect("listing should succeed"),
            vec!["draft".to_string()]
        );
        assert_eq!(
            adapter
                .read("draft")
                .expect("committed read should succeed"),
            Some(record("draft", "writer-a", 1, json!("committed")))
        );
        let second_owner = LocalFileDataStore::new(&root).expect_err("second owner must fail");
        assert_eq!(second_owner.code, DataStoreErrorCode::StoreLocked);
        assert_eq!(second_owner.message, "store_locked");
        assert_eq!(
            second_owner.details,
            json!({ "reason": "exclusive_owner_active" })
        );
    }

    #[test]
    fn local_file_adapter_lock_child() -> Result<(), String> {
        let Ok(root) = std::env::var("TRAVERSE_DATA_STORE_LOCK_CHILD_ROOT") else {
            return Ok(());
        };
        let root = PathBuf::from(root);
        let _adapter = LocalFileDataStore::new(&root).expect("child should acquire lock");
        fs::write(lock_child_ready_path(&root), "ready").expect("child should signal readiness");
        wait_for_lock_child_release(&root, 500)
    }

    #[test]
    fn local_file_adapter_rejects_cross_process_owner_and_recovers_after_exit() {
        let root = temp_root("cross-process-lock");
        let mut initial_owner = public_store(&root);
        let committed = record("draft", "writer-a", 1, json!("committed"));
        initial_owner
            .write(committed.clone())
            .expect("initial write should succeed");
        drop(initial_owner);

        let mut child = start_lock_child(&root);
        wait_for_lock_child(&root, 500).expect("lock child should become ready");
        let blocked = LocalFileDataStore::new(&root).expect_err("second process must be blocked");
        assert_eq!(blocked.code, DataStoreErrorCode::StoreLocked);
        assert_eq!(
            blocked.details,
            json!({ "reason": "exclusive_owner_active" })
        );

        fs::write(lock_child_release_path(&root), "release").expect("parent should release child");
        assert!(child.wait().expect("child should exit").success());
        let reopened = LocalFileDataStore::new(&root).expect("released lock should reopen");
        assert_eq!(
            reopened
                .read("draft")
                .expect("committed record should remain readable"),
            Some(committed)
        );
    }

    #[test]
    fn local_file_adapter_recovers_after_lock_owner_crash() {
        let root = temp_root("owner-crash-lock");
        let mut initial_owner = public_store(&root);
        initial_owner
            .write(record("draft", "writer-a", 1, json!("committed")))
            .expect("initial write should succeed");
        drop(initial_owner);

        let mut child = start_lock_child(&root);
        wait_for_lock_child(&root, 500).expect("lock child should become ready");
        child.kill().expect("parent should terminate child");
        child.wait().expect("terminated child should exit");

        let reopened = LocalFileDataStore::new(&root).expect("crashed owner lock should release");
        assert_eq!(
            reopened
                .read("draft")
                .expect("committed record should remain readable"),
            Some(record("draft", "writer-a", 1, json!("committed")))
        );
    }

    #[test]
    fn local_file_adapter_reports_unknown_version_and_helper_failures_stably() {
        let root = temp_root("helper-failures");
        let mut adapter = public_store(&root);
        adapter
            .write(record("draft", "writer-a", 1, json!("ready")))
            .expect("write should succeed");

        let mut envelope: Value = serde_json::from_slice(
            &fs::read(root.join("draft.json")).expect("envelope should be present"),
        )
        .expect("fixture should deserialize");
        envelope["format"] = json!("local-datastore/unsupported");
        fs::write(
            root.join("draft.json"),
            serde_json::to_vec(&envelope).expect("fixture should serialize"),
        )
        .expect("unknown-version fixture should write");
        let unknown_version = adapter
            .read("draft")
            .expect_err("unknown format version must fail");
        assert_eq!(
            unknown_version.code,
            DataStoreErrorCode::IntegrityCheckFailed
        );
        assert_eq!(unknown_version.details["reason"], "unknown_format_version");

        let lock_io = lock_error(TryLockError::Error(std::io::Error::other(
            "lock device failure",
        )));
        assert_eq!(lock_io.code, DataStoreErrorCode::IoFailure);
        assert_eq!(lock_io.message, "storage_io_failed");
        assert_eq!(
            lock_io.details,
            json!({ "operation": "acquire_lock", "reason": "lock_acquisition_failed" })
        );

        let unsupported_lock = lock_error(TryLockError::Error(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "locking unavailable",
        )));
        assert_eq!(unsupported_lock.code, DataStoreErrorCode::IoFailure);
        assert_eq!(
            unsupported_lock.details,
            json!({ "operation": "acquire_lock", "reason": "locking_unsupported" })
        );

        let durability = durability_error(
            &root,
            "temporary_file",
            &std::io::Error::other("sync failure"),
        );
        assert_eq!(durability.code, DataStoreErrorCode::DurabilityCommitFailed);
        assert_eq!(durability.message, "durability_commit_failed");

        let temporary = root.join(".orphan.tmp");
        fs::write(&temporary, "incomplete").expect("temporary fixture should write");
        discard_temporary_record(&temporary);
        assert!(!temporary.exists());
        discard_temporary_record(&temporary);

        let parse_error = serde_json::from_str::<Value>("{")
            .expect_err("invalid fixture must produce a serialization error");
        let serialization = serialization_error("deserialize fixture", &parse_error);
        assert_eq!(serialization.code, DataStoreErrorCode::SerializationFailure);
    }

    fn record(key: &str, writer_id: &str, lamport_clock: u64, value: Value) -> StateRecord {
        StateRecord {
            key: key.to_string(),
            value,
            lamport_clock,
            writer_id: writer_id.to_string(),
        }
    }

    fn temp_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("traverse-data-store-{name}-{}", Uuid::new_v4()))
    }

    fn public_store(root: &Path) -> LocalFileDataStore {
        LocalFileDataStore::with_classification(root, LocalDataClassification::Public)
            .expect("public local adapter should initialize")
    }

    fn write_envelope_fixture(path: &Path, envelope: &LocalDataStoreEnvelope) {
        fs::write(
            path,
            serde_json::to_vec(envelope).expect("serialize envelope"),
        )
        .expect("write envelope");
    }

    fn assert_integrity_reason(store: &LocalFileDataStore, reason: &str) {
        let error = store.read("secret").expect_err("corruption must fail");
        assert_eq!(error.code, DataStoreErrorCode::IntegrityCheckFailed);
        assert_eq!(error.details["reason"], reason);
    }

    #[cfg(feature = "datastore-encryption")]
    fn encrypted_fixture(
        record_key: &str,
        key_id: &str,
        key: [u8; 32],
        plaintext: &[u8],
    ) -> LocalDataStoreEnvelope {
        let cipher = Aes256Gcm::new_from_slice(&key).expect("valid AES-256 key");
        let nonce = fresh_aes_nonce();
        let ciphertext = cipher
            .encrypt(
                &nonce,
                Payload {
                    msg: plaintext,
                    aad: &private_record_aad(key_id, record_key),
                },
            )
            .expect("fixture encryption");
        let nonce = hex_encode(&nonce);
        let ciphertext = hex_encode(&ciphertext);
        LocalDataStoreEnvelope {
            format: LOCAL_DATA_STORE_FORMAT.to_string(),
            classification: LocalDataClassification::Private,
            digest: digest_for_private_envelope(key_id, record_key, &nonce, &ciphertext),
            record: None,
            record_key: Some(record_key.to_string()),
            key_id: Some(key_id.to_string()),
            nonce: Some(nonce),
            ciphertext: Some(ciphertext),
            retained_at: None,
        }
    }

    fn lock_child_ready_path(root: &Path) -> PathBuf {
        root.join(".lock-child-ready")
    }

    fn lock_child_release_path(root: &Path) -> PathBuf {
        root.join(".lock-child-release")
    }

    fn start_lock_child(root: &Path) -> Child {
        Command::new(std::env::current_exe().expect("test binary path should resolve"))
            .args([
                "--exact",
                "data_store::tests::local_file_adapter_lock_child",
                "--nocapture",
            ])
            .env("TRAVERSE_DATA_STORE_LOCK_CHILD_ROOT", root)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("lock child should start")
    }

    #[test]
    fn lock_child_waits_report_bounded_timeouts() {
        let root = temp_root("lock-child-timeout");
        assert_eq!(
            wait_for_lock_child(&root, 0),
            Err("lock child did not become ready")
        );
        assert_eq!(
            wait_for_lock_child_release(&root, 0),
            Err("parent did not release the lock child".to_string())
        );
    }

    fn wait_for_lock_child(root: &Path, attempts: usize) -> Result<(), &'static str> {
        if wait_for_path(&lock_child_ready_path(root), attempts) {
            Ok(())
        } else {
            Err("lock child did not become ready")
        }
    }

    fn wait_for_lock_child_release(root: &Path, attempts: usize) -> Result<(), String> {
        if wait_for_path(&lock_child_release_path(root), attempts) {
            Ok(())
        } else {
            Err("parent did not release the lock child".to_string())
        }
    }

    fn wait_for_path(path: &Path, attempts: usize) -> bool {
        for _ in 0..attempts {
            if path.exists() {
                return true;
            }
            thread::sleep(Duration::from_millis(10));
        }
        false
    }

    #[test]
    fn v2_envelope_rejects_tampering_unknown_fields_and_invalid_inner_format() {
        let private = LocalDataStoreEnvelope {
            format: LOCAL_DATA_STORE_FORMAT.to_string(),
            classification: LocalDataClassification::Private,
            digest: "sha256:opaque".to_string(),
            record: None,
            record_key: Some("record".to_string()),
            key_id: Some("host-key".to_string()),
            nonce: Some("000000000000000000000000".to_string()),
            ciphertext: Some("00000000000000000000000000000000".to_string()),
            retained_at: None,
        };
        let wrapped = v2_envelope(private).expect("wrap private payload");
        assert_eq!(wrapped.encryption_disclosure, "host_managed_opaque");

        let mut tampered = serde_json::to_value(&wrapped).expect("json");
        tampered["payload_integrity"] = Value::String("sha256:bad".to_string());
        assert!(decode_v2_envelope(tampered).is_err());

        let mut payload_tampered = serde_json::to_value(&wrapped).expect("json");
        payload_tampered["payload_integrity"] = Value::String("sha256:bad".to_string());
        payload_tampered["integrity"]["content_digest"] = Value::String("sha256:bad".to_string());
        assert!(decode_v2_envelope(payload_tampered).is_err());

        let mut unknown = serde_json::to_value(&wrapped).expect("json");
        unknown["format_version"] = json!(99);
        assert!(decode_v2_envelope(unknown).is_err());

        let mut invalid_inner = wrapped;
        invalid_inner.payload.format = "unknown/9".to_string();
        let bytes = serde_json::to_vec(&invalid_inner.payload).expect("payload");
        invalid_inner.payload_integrity = digest_bytes(&bytes);
        invalid_inner.integrity.content_digest = invalid_inner.payload_integrity.clone();
        assert!(decode_v2_envelope(serde_json::to_value(invalid_inner).expect("json")).is_err());
    }

    fn stateful_contract(state_schema: Option<Value>) -> CapabilityContract {
        CapabilityContract {
            kind: "capability_contract".to_string(),
            schema_version: "1.0.0".to_string(),
            id: "stateful.example".to_string(),
            namespace: "stateful".to_string(),
            name: "example".to_string(),
            version: "1.0.0".to_string(),
            lifecycle: Lifecycle::Active,
            owner: Owner {
                team: "runtime".to_string(),
                contact: "runtime@example.com".to_string(),
            },
            summary: "Stateful test capability".to_string(),
            description: "Stateful test capability".to_string(),
            inputs: SchemaContainer {
                schema: json!({"type": "object"}),
            },
            outputs: SchemaContainer {
                schema: json!({"type": "object"}),
            },
            preconditions: Vec::<Condition>::new(),
            postconditions: Vec::<Condition>::new(),
            side_effects: vec![SideEffect {
                kind: SideEffectKind::StateChange,
                description: "writes capability state".to_string(),
            }],
            emits: Vec::<EventReference>::new(),
            consumes: Vec::<EventReference>::new(),
            permissions: Vec::<IdReference>::new(),
            execution: Execution {
                binary_format: BinaryFormat::Wasm,
                constraints: ExecutionConstraints {
                    network_access: NetworkAccess::Forbidden,
                    filesystem_access: FilesystemAccess::SandboxOnly,
                    host_api_access: HostApiAccess::None,
                },
                entrypoint: Entrypoint {
                    kind: EntrypointKind::WasiCommand,
                    command: "run".to_string(),
                },
                preferred_targets: vec![ExecutionTarget::Local],
            },
            policies: Vec::<IdReference>::new(),
            dependencies: Vec::<DependencyReference>::new(),
            provenance: Provenance {
                source: ProvenanceSource::Greenfield,
                author: "Codex".to_string(),
                created_at: "2026-04-19T00:00:00Z".to_string(),
                spec_ref: Some("032-universal-data-access".to_string()),
                adr_refs: Vec::new(),
                exception_refs: Vec::new(),
            },
            evidence: Vec::<ValidationEvidence>::new(),
            service_type: ServiceType::Stateful,
            permitted_targets: vec![ExecutionTarget::Local],
            event_trigger: None,
            connector_requirements: Vec::new(),
            state_schema,
            use_cases: Vec::new(),
            risk: traverse_contracts::default_risk_metadata(),
        }
    }
}
