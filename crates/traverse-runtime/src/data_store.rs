//! Portable state access for Traverse capabilities.
//!
//! Governed by spec `032-universal-data-access`.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;
use traverse_contracts::CapabilityContract;

const DATA_STORE_SPEC: &str = "032-universal-data-access";

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

#[derive(Debug, Clone)]
pub struct LocalFileDataStore {
    root: PathBuf,
}

impl LocalFileDataStore {
    /// Creates a local filesystem-backed data store rooted at `root`.
    ///
    /// # Errors
    ///
    /// Returns [`DataStoreError`] when the root directory cannot be created.
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, DataStoreError> {
        let root = root.into();
        fs::create_dir_all(&root).map_err(|error| io_error("create data store root", &error))?;
        Ok(Self { root })
    }

    fn path_for_key(&self, key: &str) -> Result<PathBuf, DataStoreError> {
        validate_key(key)?;
        Ok(self.root.join(format!("{key}.json")))
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
        serde_json::from_str(&text)
            .map(Some)
            .map_err(|error| serialization_error("deserialize state record", &error))
    }

    fn write(&mut self, record: StateRecord) -> Result<(), DataStoreError> {
        let path = self.path_for_key(&record.key)?;
        let text = serde_json::to_string_pretty(&record)
            .map_err(|error| serialization_error("serialize state record", &error))?;
        fs::write(path, text).map_err(|error| io_error("write state record", &error))
    }

    fn delete(&mut self, key: &str) -> Result<(), DataStoreError> {
        let path = self.path_for_key(key)?;
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
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
        action,
        json!({ "error": error.to_string() }),
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
        let adapter = LocalFileDataStore::new(&root).expect("local adapter should initialize");
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

        assert_eq!(report.governing_spec, "032-universal-data-access");
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
        assert_eq!(invalid_json.code, DataStoreErrorCode::SerializationFailure);
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
        }
    }
}
