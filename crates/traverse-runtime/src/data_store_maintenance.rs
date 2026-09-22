//! Host-explicit `DataStore` retention prune and verified zip backup/restore.
//!
//! Governed by spec `083-datastore-retention-backup` / ADR-0021.

use super::{
    DataStoreError, LOCAL_DATA_STORE_FORMAT, LOCAL_DATA_STORE_LOCK_FILE,
    LOCAL_DATA_STORE_V2_FORMAT, LocalDataClassification, LocalDataStoreEnvelope,
    decode_v2_envelope, digest_for_private_envelope, digest_for_record, hex_decode, lock_error,
    v2_envelope, validate_key,
};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use zip::CompressionMethod;
use zip::ZipWriter;
use zip::read::ZipArchive;
use zip::write::SimpleFileOptions;

const MAINTENANCE_SPEC: &str = "083-datastore-retention-backup";
const MIGRATION_SPEC: &str = "092-datastore-v2-migration-ownership";
const BACKUP_MANIFEST_VERSION: &str = "1";
const BACKUP_MANIFEST_MEMBER: &str = "manifest.json";
const BACKUP_RECORDS_PREFIX: &str = "records/";
const MAX_BACKUP_MANIFEST_BYTES: u64 = 4 * 1024 * 1024;
const MAX_BACKUP_MEMBER_BYTES: u64 = 16 * 1024 * 1024;
const HEXADECIMAL_DIGITS: &[u8; 16] = b"0123456789abcdef";

/// Host retention knobs for prune (FR-002).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetentionPolicy {
    pub max_count: Option<usize>,
    pub max_age_secs: Option<u64>,
}

/// Stable maintenance failure codes (FR-010).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaintenanceErrorCode {
    StoreLocked,
    InvalidRetentionPolicy,
    BackupVerifyFailed,
    RestoreVerifyFailed,
    UnsupportedStoreFormat,
    MaintenanceIoFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaintenanceError {
    pub code: MaintenanceErrorCode,
    pub message: String,
    pub details: Value,
}

/// Stable, secret-free failure codes for an explicit host-owned v1-to-v2 migration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MigrationErrorCode {
    TransitionUnsupported,
    SourceInvalid,
    BackupFailed,
    WriteFailed,
    VerificationFailed,
    CommitFailed,
    RestoreFailed,
    OwnerLocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationError {
    pub code: MigrationErrorCode,
    pub message: String,
    pub details: Value,
}

/// Safe evidence returned to the owning host; it never contains paths, keys, or values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationReport {
    pub governing_spec: String,
    pub source_format: String,
    pub target_format: String,
    pub record_count: u64,
    pub backup_content_digest: String,
    pub outcome: String,
}

/// Host-explicit format-migration port. Generic runtime and CLI paths never call it.
pub trait DataStoreMigration {
    /// Migrates a verified v1 root to v2 only after a host-directed backup and
    /// complete candidate verification have succeeded.
    ///
    /// # Errors
    ///
    /// Returns a stable, secret-free [`MigrationError`] when the source,
    /// backup, candidate, ownership lock, or atomic commit cannot be verified.
    fn migrate_v1_to_v2(
        &mut self,
        backup_destination: &Path,
        as_of: &str,
    ) -> Result<MigrationReport, MigrationError>;
}

/// Secret-free maintenance evidence (FR-009).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaintenanceEvidence {
    pub governing_spec: String,
    pub outcome: String,
    pub as_of: String,
    pub attempted_count: u64,
    pub removed_count: u64,
    pub retained_count: u64,
    pub record_count: u64,
    pub archive_content_digest: Option<String>,
    pub failure_code: Option<MaintenanceErrorCode>,
    pub failure_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupRecordIndexEntry {
    pub key: String,
    pub classification: LocalDataClassification,
    pub envelope_digest: String,
    pub member_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupManifest {
    pub manifest_format_version: String,
    pub created_as_of: String,
    pub record_count: u64,
    pub archive_content_digest: String,
    pub records: Vec<BackupRecordIndexEntry>,
    pub store_format: String,
    pub writer_tool: String,
    pub writer_semver: String,
}

/// Separate maintenance port for the same root and exclusive lock (FR-001).
pub trait DataStoreMaintenance {
    /// Prunes records under host policy with required `as_of` (FR-002/FR-003).
    ///
    /// # Errors
    ///
    /// Returns [`MaintenanceError`] when the policy is invalid or prune fails.
    fn prune(
        &mut self,
        policy: &RetentionPolicy,
        as_of: &str,
    ) -> Result<MaintenanceEvidence, MaintenanceError>;

    /// Writes a verified zip backup to `destination` (FR-004/FR-006/FR-007).
    ///
    /// # Errors
    ///
    /// Returns [`MaintenanceError`] when backup or verification fails.
    fn backup(
        &mut self,
        destination: &Path,
        as_of: &str,
    ) -> Result<MaintenanceEvidence, MaintenanceError>;

    /// Verifies an archive and atomically replaces the store root (FR-005/FR-008).
    ///
    /// # Errors
    ///
    /// Returns [`MaintenanceError`] when verification or replace fails.
    fn restore(
        &mut self,
        archive: &Path,
        as_of: &str,
    ) -> Result<MaintenanceEvidence, MaintenanceError>;
}

#[derive(Debug)]
pub struct LocalFileDataStoreMaintenance {
    root: PathBuf,
    lock_file: File,
}

impl Drop for LocalFileDataStoreMaintenance {
    fn drop(&mut self) {
        let _ = self.lock_file.unlock();
    }
}

impl LocalFileDataStoreMaintenance {
    /// Opens maintenance for an existing host-owned store root.
    ///
    /// # Errors
    ///
    /// Returns [`MaintenanceError`] when the root cannot be created or locked.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, MaintenanceError> {
        let root = root.into();
        fs::create_dir_all(&root)
            .map_err(|error| maintenance_io("create data store root for maintenance", &error))?;
        let lock_path = root.join(LOCAL_DATA_STORE_LOCK_FILE);
        let lock_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|error| maintenance_io("open data store lock for maintenance", &error))?;
        lock_file.try_lock().map_err(|error| {
            let mapped = lock_error(error);
            map_lock_to_maintenance(mapped)
        })?;
        Ok(Self { root, lock_file })
    }

    fn list_record_keys(&self) -> Result<Vec<String>, MaintenanceError> {
        let mut keys = Vec::new();
        for entry in
            fs::read_dir(&self.root).map_err(|error| maintenance_io("list keys", &error))?
        {
            let entry = entry.map_err(|error| maintenance_io("read key entry", &error))?;
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

    fn path_for_key(&self, key: &str) -> Result<PathBuf, MaintenanceError> {
        validate_key(key).map_err(map_data_store_error)?;
        Ok(self.root.join(format!("{key}.json")))
    }

    fn read_envelope_bytes(&self, key: &str) -> Result<Vec<u8>, MaintenanceError> {
        let path = self.path_for_key(key)?;
        fs::read(&path).map_err(|error| maintenance_io("read envelope bytes", &error))
    }

    fn read_envelope(&self, key: &str) -> Result<LocalDataStoreEnvelope, MaintenanceError> {
        let bytes = self.read_envelope_bytes(key)?;
        parse_envelope_bytes(&bytes)
    }

    fn delete_key(&self, key: &str) -> Result<(), MaintenanceError> {
        let path = self.path_for_key(key)?;
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(maintenance_io("delete pruned record", &error)),
        }
    }

    fn reacquire_lock(&mut self) -> Result<(), MaintenanceError> {
        let lock_path = self.root.join(LOCAL_DATA_STORE_LOCK_FILE);
        let lock_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|error| maintenance_io("reopen data store lock", &error))?;
        lock_file
            .try_lock()
            .map_err(|error| map_lock_to_maintenance(lock_error(error)))?;
        let _ = self.lock_file.unlock();
        self.lock_file = lock_file;
        Ok(())
    }
}

impl DataStoreMaintenance for LocalFileDataStoreMaintenance {
    fn prune(
        &mut self,
        policy: &RetentionPolicy,
        as_of: &str,
    ) -> Result<MaintenanceEvidence, MaintenanceError> {
        validate_policy(policy)?;
        let as_of_instant = parse_as_of(as_of)?;
        let keys = self.list_record_keys()?;
        let total = keys.len() as u64;
        let victims = select_prune_victims(self, &keys, policy, as_of_instant)?;
        let attempted = victims.len() as u64;
        let mut removed = 0_u64;
        for key in &victims {
            if let Err(error) = self.delete_key(key) {
                return Err(MaintenanceError {
                    code: error.code,
                    message: error.message.clone(),
                    details: json!({
                        "attempted_count": attempted,
                        "removed_count": removed,
                        "retained_count": total.saturating_sub(removed),
                        "failure_reason": error.message,
                    }),
                });
            }
            removed += 1;
        }
        Ok(prune_evidence(
            "prune_completed",
            as_of,
            attempted,
            removed,
            total.saturating_sub(removed),
            None,
            None,
        ))
    }

    fn backup(
        &mut self,
        destination: &Path,
        as_of: &str,
    ) -> Result<MaintenanceEvidence, MaintenanceError> {
        let _ = parse_as_of(as_of)?;
        let keys = self.list_record_keys()?;
        let mut members: Vec<(String, Vec<u8>, BackupRecordIndexEntry)> = Vec::new();
        for key in &keys {
            let bytes = self.read_envelope_bytes(key)?;
            let envelope = parse_envelope_bytes(&bytes)?;
            let member_path = format!("{BACKUP_RECORDS_PREFIX}{key}.json");
            let envelope_digest = sha256_hex(&bytes);
            members.push((
                member_path.clone(),
                bytes,
                BackupRecordIndexEntry {
                    key: key.clone(),
                    classification: envelope.classification,
                    envelope_digest,
                    member_path,
                },
            ));
        }
        members.sort_by(|left, right| left.0.cmp(&right.0));
        let index: Vec<BackupRecordIndexEntry> =
            members.iter().map(|(_, _, entry)| entry.clone()).collect();
        let content_digest = archive_content_digest_from_members(
            &members
                .iter()
                .map(|(path, bytes, _)| (path.as_str(), bytes.as_slice()))
                .collect::<Vec<_>>(),
        );
        let manifest = BackupManifest {
            manifest_format_version: BACKUP_MANIFEST_VERSION.to_string(),
            created_as_of: as_of.to_string(),
            record_count: index.len() as u64,
            archive_content_digest: content_digest.clone(),
            records: index,
            store_format: LOCAL_DATA_STORE_FORMAT.to_string(),
            writer_tool: env!("CARGO_PKG_NAME").to_string(),
            writer_semver: env!("CARGO_PKG_VERSION").to_string(),
        };
        let manifest_bytes = serialize_backup_manifest(&manifest)?;

        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| maintenance_io("create backup destination directory", &error))?;
        }
        let temporary = destination.with_extension("zip.tmp");
        let _ = fs::remove_file(&temporary);
        {
            let file = File::create(&temporary)
                .map_err(|error| maintenance_io("create backup zip", &error))?;
            let mut zip = ZipWriter::new(file);
            let options =
                SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
            zip.start_file(BACKUP_MANIFEST_MEMBER, options)
                .map_err(|error| maintenance_zip("start manifest zip member", error))?;
            zip.write_all(&manifest_bytes)
                .map_err(|error| maintenance_io("write manifest zip member", &error))?;
            for (path, bytes, _) in &members {
                zip.start_file(path, options)
                    .map_err(|error| maintenance_zip("start record zip member", error))?;
                zip.write_all(bytes)
                    .map_err(|error| maintenance_io("write record zip member", &error))?;
            }
            zip.finish()
                .map_err(|error| maintenance_zip("finish backup zip", error))?;
        }

        verify_backup_archive(&temporary)
            .inspect_err(|_| cleanup_failed_backup_temp(&temporary))?;
        commit_backup_zip(&temporary, destination)?;

        Ok(MaintenanceEvidence {
            governing_spec: MAINTENANCE_SPEC.to_string(),
            outcome: "backup_created".to_string(),
            as_of: as_of.to_string(),
            attempted_count: members.len() as u64,
            removed_count: 0,
            retained_count: members.len() as u64,
            record_count: members.len() as u64,
            archive_content_digest: Some(content_digest),
            failure_code: None,
            failure_reason: None,
        })
    }

    fn restore(
        &mut self,
        archive: &Path,
        as_of: &str,
    ) -> Result<MaintenanceEvidence, MaintenanceError> {
        let _ = parse_as_of(as_of)?;
        let verified = verify_backup_archive(archive)?;
        let parent = restore_parent(&self.root)?;
        let work_suffix = restore_work_suffix(&self.root, as_of);
        let temp_root = parent.join(format!(".traverse-datastore-restore-{work_suffix}"));
        let backup_root = parent.join(format!(".traverse-datastore-replaced-{work_suffix}"));
        let _ = fs::remove_dir_all(&temp_root);
        let _ = fs::remove_dir_all(&backup_root);
        fs::create_dir_all(&temp_root)
            .map_err(|error| maintenance_io("create restore temp root", &error))?;

        materialize_archive_to_root(archive, &temp_root)?;
        verify_store_root_envelopes(&temp_root)?;

        // Release lock on the live root before swapping directories.
        let _ = self.lock_file.unlock();
        move_live_store_aside(self, &backup_root)?;
        replace_store_root(self, &temp_root, &backup_root)?;
        self.reacquire_lock()?;
        let _ = fs::remove_dir_all(&backup_root);

        Ok(MaintenanceEvidence {
            governing_spec: MAINTENANCE_SPEC.to_string(),
            outcome: "restore_committed".to_string(),
            as_of: as_of.to_string(),
            attempted_count: verified.record_count,
            removed_count: 0,
            retained_count: verified.record_count,
            record_count: verified.record_count,
            archive_content_digest: Some(verified.archive_content_digest),
            failure_code: None,
            failure_reason: None,
        })
    }
}

impl DataStoreMigration for LocalFileDataStoreMaintenance {
    fn migrate_v1_to_v2(
        &mut self,
        backup_destination: &Path,
        as_of: &str,
    ) -> Result<MigrationReport, MigrationError> {
        parse_as_of(as_of).map_err(map_migration_source_error)?;
        let keys = self
            .list_record_keys()
            .map_err(map_migration_source_error)?;

        // Validate every committed source envelope before any backup artifact or
        // candidate representation is written.
        for key in &keys {
            self.read_envelope(key)
                .map_err(map_migration_source_error)?;
        }

        let backup = self
            .backup(backup_destination, as_of)
            .map_err(map_migration_backup_error)?;
        let backup_digest = verified_backup_digest(backup.archive_content_digest)?;

        let parent = restore_parent(&self.root).map_err(map_migration_commit_error)?;
        let suffix = restore_work_suffix(&self.root, as_of);
        let candidate_root = parent.join(format!(".traverse-datastore-migration-{suffix}"));
        let previous_root = parent.join(format!(".traverse-datastore-pre-v2-{suffix}"));
        let _ = fs::remove_dir_all(&candidate_root);
        let _ = fs::remove_dir_all(&previous_root);
        fs::create_dir_all(&candidate_root).map_err(map_migration_write_io)?;
        File::create(candidate_root.join(LOCAL_DATA_STORE_LOCK_FILE))
            .map_err(map_migration_write_io)?;

        (|| -> Result<(), MigrationError> {
            for key in &keys {
                let source = self
                    .read_envelope(key)
                    .map_err(map_migration_source_error)?;
                let v2 = v2_envelope(source).map_err(map_migration_write_data_store)?;
                let bytes = serialize_v2_candidate(&v2)?;
                fs::write(candidate_root.join(format!("{key}.json")), bytes)
                    .map_err(map_migration_write_io)?;
            }
            verify_v2_store_root(&candidate_root).map_err(map_migration_verification_error)
        })()
        .map_err(|error| abandon_failed_candidate(&candidate_root, error))?;

        commit_migrated_store_root(self, &candidate_root, &previous_root)?;

        Ok(MigrationReport {
            governing_spec: MIGRATION_SPEC.to_string(),
            source_format: LOCAL_DATA_STORE_FORMAT.to_string(),
            target_format: LOCAL_DATA_STORE_V2_FORMAT.to_string(),
            record_count: keys.len() as u64,
            backup_content_digest: backup_digest,
            outcome: "datastore_migration_committed".to_string(),
        })
    }
}

fn verify_v2_store_root(root: &Path) -> Result<(), MaintenanceError> {
    for entry in
        fs::read_dir(root).map_err(|error| maintenance_io("list migration candidate", &error))?
    {
        let entry =
            entry.map_err(|error| maintenance_io("read migration candidate entry", &error))?;
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        let bytes =
            fs::read(path).map_err(|error| maintenance_io("read migration candidate", &error))?;
        let value: Value =
            serde_json::from_slice(&bytes).map_err(|_| malformed_envelope_error())?;
        let envelope = decode_v2_envelope(value).map_err(map_data_store_error)?;
        parse_envelope_bytes(&serialize_envelope_for_verify(&envelope)?)?;
    }
    Ok(())
}

fn serialize_envelope_for_verify(
    envelope: &LocalDataStoreEnvelope,
) -> Result<Vec<u8>, MaintenanceError> {
    json_to_vec(envelope).map_err(map_envelope_serialize_error)
}

fn map_envelope_serialize_error(_: serde_json::Error) -> MaintenanceError {
    malformed_envelope_error()
}

fn json_to_vec<T: Serialize>(value: &T) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(value)
}

fn verified_backup_digest(digest: Option<String>) -> Result<String, MigrationError> {
    digest.ok_or_else(|| {
        migration_error(
            MigrationErrorCode::BackupFailed,
            "datastore_backup_failed",
            "missing_verified_backup_digest",
        )
    })
}

fn serialize_v2_candidate(
    envelope: &super::LocalDataStoreV2Envelope,
) -> Result<Vec<u8>, MigrationError> {
    json_to_vec(envelope).map_err(map_serialize_candidate_error)
}

fn map_serialize_candidate_error(_: serde_json::Error) -> MigrationError {
    migration_error(
        MigrationErrorCode::WriteFailed,
        "datastore_write_failed",
        "serialize_candidate",
    )
}

fn abandon_failed_candidate(candidate_root: &Path, error: MigrationError) -> MigrationError {
    let _ = fs::remove_dir_all(candidate_root);
    error
}

fn commit_migrated_store_root(
    maintenance: &mut LocalFileDataStoreMaintenance,
    candidate_root: &Path,
    previous_root: &Path,
) -> Result<(), MigrationError> {
    let _ = maintenance.lock_file.unlock();
    move_live_aside_for_migration(maintenance, previous_root, candidate_root)?;
    install_migrated_candidate(maintenance, candidate_root, previous_root)?;
    reacquire_migrated_lock(maintenance, candidate_root, previous_root)?;
    let _ = fs::remove_dir_all(previous_root);
    Ok(())
}

fn move_live_aside_for_migration(
    maintenance: &mut LocalFileDataStoreMaintenance,
    previous_root: &Path,
    candidate_root: &Path,
) -> Result<(), MigrationError> {
    fs::rename(&maintenance.root, previous_root)
        .map_err(|error| abort_migration_before_aside(maintenance, candidate_root, error))
}

fn install_migrated_candidate(
    maintenance: &mut LocalFileDataStoreMaintenance,
    candidate_root: &Path,
    previous_root: &Path,
) -> Result<(), MigrationError> {
    fs::rename(candidate_root, &maintenance.root)
        .map_err(|error| abort_migration_after_aside(maintenance, previous_root, error))
}

fn abort_migration_before_aside(
    maintenance: &mut LocalFileDataStoreMaintenance,
    candidate_root: &Path,
    error: std::io::Error,
) -> MigrationError {
    let _ = maintenance.reacquire_lock();
    let _ = fs::remove_dir_all(candidate_root);
    map_migration_commit_io(error)
}

fn abort_migration_after_aside(
    maintenance: &mut LocalFileDataStoreMaintenance,
    previous_root: &Path,
    error: std::io::Error,
) -> MigrationError {
    let _ = fs::rename(previous_root, &maintenance.root);
    let _ = maintenance.reacquire_lock();
    map_migration_commit_io(error)
}

fn reacquire_migrated_lock(
    maintenance: &mut LocalFileDataStoreMaintenance,
    candidate_root: &Path,
    previous_root: &Path,
) -> Result<(), MigrationError> {
    if let Err(error) = maintenance.reacquire_lock() {
        let _ = fs::rename(&maintenance.root, candidate_root);
        let _ = fs::rename(previous_root, &maintenance.root);
        let _ = maintenance.reacquire_lock();
        return Err(map_migration_commit_error(error));
    }
    Ok(())
}

fn migration_error(code: MigrationErrorCode, message: &str, reason: &str) -> MigrationError {
    MigrationError {
        code,
        message: message.to_string(),
        details: json!({ "reason": reason }),
    }
}

fn map_migration_source_error(_: MaintenanceError) -> MigrationError {
    migration_error(
        MigrationErrorCode::SourceInvalid,
        "datastore_source_invalid",
        "source_validation_failed",
    )
}

fn map_migration_backup_error(_: MaintenanceError) -> MigrationError {
    migration_error(
        MigrationErrorCode::BackupFailed,
        "datastore_backup_failed",
        "backup_verification_failed",
    )
}

fn map_migration_write_io(_: std::io::Error) -> MigrationError {
    migration_error(
        MigrationErrorCode::WriteFailed,
        "datastore_write_failed",
        "candidate_write_failed",
    )
}

fn map_migration_write_data_store(_: DataStoreError) -> MigrationError {
    migration_error(
        MigrationErrorCode::WriteFailed,
        "datastore_write_failed",
        "candidate_serialization_failed",
    )
}

fn map_migration_verification_error(_: MaintenanceError) -> MigrationError {
    migration_error(
        MigrationErrorCode::VerificationFailed,
        "datastore_verification_failed",
        "candidate_verification_failed",
    )
}

fn map_migration_commit_io(_: std::io::Error) -> MigrationError {
    migration_error(
        MigrationErrorCode::CommitFailed,
        "datastore_commit_failed",
        "atomic_commit_failed",
    )
}

fn map_migration_commit_error(_: MaintenanceError) -> MigrationError {
    migration_error(
        MigrationErrorCode::CommitFailed,
        "datastore_commit_failed",
        "atomic_commit_failed",
    )
}

fn select_prune_victims(
    store: &LocalFileDataStoreMaintenance,
    keys: &[String],
    policy: &RetentionPolicy,
    as_of: DateTime<Utc>,
) -> Result<Vec<String>, MaintenanceError> {
    // Start retained; clear flags for records that fail any active bound.
    // Count: deterministic sorted-key order; newest = suffix of length max_count.
    // Age: remove when retained_at <= as_of - max_age (unstamped envelopes stay).
    let mut retain = vec![true; keys.len()];

    if let Some(max_count) = policy.max_count.filter(|count| keys.len() > *count) {
        for flag in retain.iter_mut().take(keys.len() - max_count) {
            *flag = false;
        }
    }

    if let Some(max_age_secs) = policy.max_age_secs {
        let cutoff = age_cutoff(as_of, max_age_secs)?;
        for (index, key) in keys.iter().enumerate() {
            let envelope = store.read_envelope(key)?;
            if let Some(stamp) = envelope.retained_at.as_deref() {
                let retained = parse_as_of(stamp)?;
                if retained <= cutoff {
                    retain[index] = false;
                }
            }
        }
    }

    Ok(keys
        .iter()
        .zip(retain)
        .filter_map(|(key, keep)| if keep { None } else { Some(key.clone()) })
        .collect())
}

fn validate_policy(policy: &RetentionPolicy) -> Result<(), MaintenanceError> {
    if policy.max_count.is_none() && policy.max_age_secs.is_none() {
        return Err(MaintenanceError {
            code: MaintenanceErrorCode::InvalidRetentionPolicy,
            message: "invalid_retention_policy".to_string(),
            details: json!({ "reason": "at_least_one_bound_required" }),
        });
    }
    Ok(())
}

fn parse_as_of(as_of: &str) -> Result<DateTime<Utc>, MaintenanceError> {
    DateTime::parse_from_rfc3339(as_of)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|error| MaintenanceError {
            code: MaintenanceErrorCode::InvalidRetentionPolicy,
            message: "invalid_retention_policy".to_string(),
            details: json!({ "reason": "invalid_as_of", "cause": error.to_string() }),
        })
}

fn parse_envelope_bytes(bytes: &[u8]) -> Result<LocalDataStoreEnvelope, MaintenanceError> {
    let value: Value = serde_json::from_slice(bytes).map_err(|_| MaintenanceError {
        code: MaintenanceErrorCode::RestoreVerifyFailed,
        message: "restore_verify_failed".to_string(),
        details: json!({ "reason": "malformed_envelope" }),
    })?;
    if value.get("format").is_none() {
        return Err(MaintenanceError {
            code: MaintenanceErrorCode::UnsupportedStoreFormat,
            message: "unsupported_store_format".to_string(),
            details: json!({ "reason": "legacy_unverified" }),
        });
    }
    let envelope: LocalDataStoreEnvelope =
        serde_json::from_value(value).map_err(|_| MaintenanceError {
            code: MaintenanceErrorCode::RestoreVerifyFailed,
            message: "restore_verify_failed".to_string(),
            details: json!({ "reason": "malformed_envelope" }),
        })?;
    if envelope.format != LOCAL_DATA_STORE_FORMAT {
        return Err(MaintenanceError {
            code: MaintenanceErrorCode::UnsupportedStoreFormat,
            message: "unsupported_store_format".to_string(),
            details: json!({ "reason": "unknown_format_version", "format": envelope.format }),
        });
    }
    let key = envelope_record_key(&envelope)?;
    let expected = match envelope.classification {
        LocalDataClassification::Public => {
            if envelope.key_id.is_some()
                || envelope.record_key.is_some()
                || envelope.nonce.is_some()
                || envelope.ciphertext.is_some()
            {
                return Err(malformed_envelope_error());
            }
            digest_for_record(
                envelope
                    .record
                    .as_ref()
                    .ok_or_else(malformed_envelope_error)?,
            )
            .map_err(map_data_store_error)?
        }
        LocalDataClassification::Private => {
            if envelope.record.is_some() {
                return Err(malformed_envelope_error());
            }
            let key_id = envelope
                .key_id
                .as_deref()
                .ok_or_else(malformed_envelope_error)?;
            let nonce = envelope
                .nonce
                .as_deref()
                .ok_or_else(malformed_envelope_error)?;
            let ciphertext = envelope
                .ciphertext
                .as_deref()
                .ok_or_else(malformed_envelope_error)?;
            let nonce_bytes = hex_decode(nonce).map_err(|_| malformed_envelope_error())?;
            let ciphertext_bytes =
                hex_decode(ciphertext).map_err(|_| malformed_envelope_error())?;
            if nonce_bytes.len() != 12 || ciphertext_bytes.len() < 16 {
                return Err(malformed_envelope_error());
            }
            digest_for_private_envelope(key_id, &key, nonce, ciphertext)
        }
    };
    if envelope.digest != expected {
        return Err(MaintenanceError {
            code: MaintenanceErrorCode::RestoreVerifyFailed,
            message: "restore_verify_failed".to_string(),
            details: json!({ "reason": "digest_mismatch", "key": key }),
        });
    }
    Ok(envelope)
}

fn envelope_record_key(envelope: &LocalDataStoreEnvelope) -> Result<String, MaintenanceError> {
    match envelope.classification {
        LocalDataClassification::Public => envelope
            .record
            .as_ref()
            .map(|record| record.key.clone())
            .ok_or_else(malformed_envelope_error),
        LocalDataClassification::Private => envelope
            .record_key
            .clone()
            .ok_or_else(malformed_envelope_error),
    }
}

fn malformed_envelope_error() -> MaintenanceError {
    MaintenanceError {
        code: MaintenanceErrorCode::RestoreVerifyFailed,
        message: "restore_verify_failed".to_string(),
        details: json!({ "reason": "malformed_envelope" }),
    }
}

fn archive_content_digest_from_members(members: &[(&str, &[u8])]) -> String {
    let mut hasher = Sha256::new();
    for (path, bytes) in members {
        hasher.update(path.as_bytes());
        hasher.update([0]);
        hasher.update(bytes);
        hasher.update([0]);
    }
    format!("sha256:{}", hex_encode(&hasher.finalize()))
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("sha256:{}", hex_encode(&Sha256::digest(bytes)))
}

fn restore_work_suffix(root: &Path, as_of: &str) -> String {
    let root_digest = hex_encode(&Sha256::digest(root.as_os_str().as_encoded_bytes()));
    format!(
        "{}-{}-{}",
        std::process::id(),
        &root_digest[..16],
        as_of.replace(':', "")
    )
}

fn hex_encode(digest: &[u8]) -> String {
    let mut hexadecimal = String::with_capacity(digest.len() * 2);
    for byte in digest {
        hexadecimal.push(char::from(HEXADECIMAL_DIGITS[usize::from(byte >> 4)]));
        hexadecimal.push(char::from(HEXADECIMAL_DIGITS[usize::from(byte & 0x0f)]));
    }
    hexadecimal
}

fn verify_backup_archive(path: &Path) -> Result<BackupManifest, MaintenanceError> {
    let mut archive = open_backup_zip(path)?;
    let manifest = read_and_validate_manifest_header(&mut archive)?;
    verify_manifest_member_payloads(&mut archive, &manifest)?;
    Ok(manifest)
}

fn open_backup_zip(path: &Path) -> Result<ZipArchive<File>, MaintenanceError> {
    let file = File::open(path).map_err(|error| MaintenanceError {
        code: MaintenanceErrorCode::BackupVerifyFailed,
        message: "backup_verify_failed".to_string(),
        details: json!({ "reason": "open_archive", "cause": error.to_string() }),
    })?;
    ZipArchive::new(file).map_err(|error| MaintenanceError {
        code: MaintenanceErrorCode::BackupVerifyFailed,
        message: "backup_verify_failed".to_string(),
        details: json!({ "reason": "invalid_zip", "cause": error.to_string() }),
    })
}

fn read_and_validate_manifest_header(
    archive: &mut ZipArchive<File>,
) -> Result<BackupManifest, MaintenanceError> {
    let mut manifest_bytes = Vec::new();
    {
        let mut manifest_file =
            archive
                .by_name(BACKUP_MANIFEST_MEMBER)
                .map_err(|_| MaintenanceError {
                    code: MaintenanceErrorCode::BackupVerifyFailed,
                    message: "backup_verify_failed".to_string(),
                    details: json!({ "reason": "missing_manifest" }),
                })?;
        read_zip_member_bounded(
            &mut manifest_file,
            MAX_BACKUP_MANIFEST_BYTES,
            &mut manifest_bytes,
        )
        .map_err(|error| read_manifest_bytes_error(&error))?;
    }
    let manifest: BackupManifest =
        serde_json::from_slice(&manifest_bytes).map_err(|_| MaintenanceError {
            code: MaintenanceErrorCode::BackupVerifyFailed,
            message: "backup_verify_failed".to_string(),
            details: json!({ "reason": "malformed_manifest" }),
        })?;
    if manifest.manifest_format_version != BACKUP_MANIFEST_VERSION {
        return Err(MaintenanceError {
            code: MaintenanceErrorCode::UnsupportedStoreFormat,
            message: "unsupported_store_format".to_string(),
            details: json!({
                "reason": "unsupported_manifest_version",
                "version": manifest.manifest_format_version
            }),
        });
    }
    if manifest.store_format != LOCAL_DATA_STORE_FORMAT {
        return Err(MaintenanceError {
            code: MaintenanceErrorCode::UnsupportedStoreFormat,
            message: "unsupported_store_format".to_string(),
            details: json!({ "reason": "unsupported_store_format", "format": manifest.store_format }),
        });
    }
    Ok(manifest)
}

fn verify_manifest_member_payloads(
    archive: &mut ZipArchive<File>,
    manifest: &BackupManifest,
) -> Result<(), MaintenanceError> {
    let mut members = Vec::new();
    for entry in &manifest.records {
        if !entry.member_path.starts_with(BACKUP_RECORDS_PREFIX) || entry.member_path.contains("..")
        {
            return Err(MaintenanceError {
                code: MaintenanceErrorCode::BackupVerifyFailed,
                message: "backup_verify_failed".to_string(),
                details: json!({ "reason": "invalid_member_path", "path": entry.member_path }),
            });
        }
        let mut bytes = Vec::new();
        {
            let mut file = archive
                .by_name(&entry.member_path)
                .map_err(|_| MaintenanceError {
                    code: MaintenanceErrorCode::BackupVerifyFailed,
                    message: "backup_verify_failed".to_string(),
                    details: json!({ "reason": "missing_member", "path": entry.member_path }),
                })?;
            read_zip_member_bounded(&mut file, MAX_BACKUP_MEMBER_BYTES, &mut bytes)
                .map_err(|error| read_member_bytes_error(&error))?;
        }
        if sha256_hex(&bytes) != entry.envelope_digest {
            return Err(MaintenanceError {
                code: MaintenanceErrorCode::BackupVerifyFailed,
                message: "backup_verify_failed".to_string(),
                details: json!({ "reason": "member_digest_mismatch", "key": entry.key }),
            });
        }
        let envelope = parse_envelope_bytes(&bytes)?;
        if envelope_record_key(&envelope)? != entry.key {
            return Err(MaintenanceError {
                code: MaintenanceErrorCode::BackupVerifyFailed,
                message: "backup_verify_failed".to_string(),
                details: json!({ "reason": "key_mismatch", "key": entry.key }),
            });
        }
        members.push((entry.member_path.clone(), bytes));
    }
    members.sort_by(|left, right| left.0.cmp(&right.0));
    let computed = archive_content_digest_from_members(
        &members
            .iter()
            .map(|(path, bytes)| (path.as_str(), bytes.as_slice()))
            .collect::<Vec<_>>(),
    );
    if computed != manifest.archive_content_digest {
        return Err(MaintenanceError {
            code: MaintenanceErrorCode::BackupVerifyFailed,
            message: "backup_verify_failed".to_string(),
            details: json!({ "reason": "archive_content_digest_mismatch" }),
        });
    }
    if manifest.record_count != manifest.records.len() as u64 {
        return Err(MaintenanceError {
            code: MaintenanceErrorCode::BackupVerifyFailed,
            message: "backup_verify_failed".to_string(),
            details: json!({ "reason": "record_count_mismatch" }),
        });
    }
    Ok(())
}

fn materialize_archive_to_root(archive: &Path, root: &Path) -> Result<(), MaintenanceError> {
    let manifest = verify_backup_archive(archive)?;
    let mut zip = open_restore_zip(archive)?;
    // Ensure lock file exists in the new root so reopen can acquire it.
    let lock_path = root.join(LOCAL_DATA_STORE_LOCK_FILE);
    File::create(&lock_path)
        .map_err(|error| maintenance_io("create restored lock file", &error))?;

    for entry in &manifest.records {
        let key = &entry.key;
        validate_key(key).map_err(map_data_store_error)?;
        let mut bytes = Vec::new();
        {
            let mut member = zip
                .by_name(&entry.member_path)
                .map_err(|_| restore_missing_member_error(&entry.member_path))?;
            read_zip_member_bounded(&mut member, MAX_BACKUP_MEMBER_BYTES, &mut bytes)
                .map_err(|error| maintenance_io("extract member", &error))?;
        }
        let dest = root.join(format!("{key}.json"));
        fs::write(&dest, bytes)
            .map_err(|error| maintenance_io("write restored envelope", &error))?;
    }
    Ok(())
}

fn read_zip_member_bounded(
    member: &mut zip::read::ZipFile<'_, File>,
    maximum_bytes: u64,
    output: &mut Vec<u8>,
) -> Result<(), std::io::Error> {
    if member.size() > maximum_bytes {
        return Err(std::io::Error::other("member_too_large"));
    }
    let mut limited = member.take(maximum_bytes.saturating_add(1));
    limited.read_to_end(output)?;
    if output.len() as u64 > maximum_bytes {
        return Err(std::io::Error::other("member_too_large"));
    }
    Ok(())
}

fn verify_store_root_envelopes(root: &Path) -> Result<(), MaintenanceError> {
    for entry in fs::read_dir(root).map_err(|error| maintenance_io("list restored root", &error))? {
        let entry = entry.map_err(|error| maintenance_io("read restored entry", &error))?;
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        let bytes =
            fs::read(&path).map_err(|error| maintenance_io("read restored envelope", &error))?;
        parse_envelope_bytes(&bytes)?;
    }
    Ok(())
}

fn prune_evidence(
    outcome: &str,
    as_of: &str,
    attempted: u64,
    removed: u64,
    retained: u64,
    failure_code: Option<MaintenanceErrorCode>,
    failure_reason: Option<String>,
) -> MaintenanceEvidence {
    MaintenanceEvidence {
        governing_spec: MAINTENANCE_SPEC.to_string(),
        outcome: outcome.to_string(),
        as_of: as_of.to_string(),
        attempted_count: attempted,
        removed_count: removed,
        retained_count: retained,
        record_count: retained,
        archive_content_digest: None,
        failure_code,
        failure_reason,
    }
}

fn age_cutoff(as_of: DateTime<Utc>, max_age_secs: u64) -> Result<DateTime<Utc>, MaintenanceError> {
    let age = Duration::seconds(
        i64::try_from(max_age_secs).map_err(|_| max_age_secs_out_of_range_error())?,
    );
    as_of
        .checked_sub_signed(age)
        .ok_or_else(as_of_minus_max_age_underflow_error)
}

fn max_age_secs_out_of_range_error() -> MaintenanceError {
    MaintenanceError {
        code: MaintenanceErrorCode::InvalidRetentionPolicy,
        message: "invalid_retention_policy".to_string(),
        details: json!({ "reason": "max_age_secs_out_of_range" }),
    }
}

fn as_of_minus_max_age_underflow_error() -> MaintenanceError {
    MaintenanceError {
        code: MaintenanceErrorCode::InvalidRetentionPolicy,
        message: "invalid_retention_policy".to_string(),
        details: json!({ "reason": "as_of_minus_max_age_underflow" }),
    }
}

fn move_live_store_aside(
    maintenance: &mut LocalFileDataStoreMaintenance,
    backup_root: &Path,
) -> Result<(), MaintenanceError> {
    fs::rename(&maintenance.root, backup_root)
        .map_err(|error| handle_move_live_store_aside_failure(maintenance, &error))
}

fn replace_store_root(
    maintenance: &mut LocalFileDataStoreMaintenance,
    temp_root: &Path,
    backup_root: &Path,
) -> Result<(), MaintenanceError> {
    if let Err(error) = fs::rename(temp_root, &maintenance.root) {
        return Err(handle_replace_store_root_failure(
            maintenance,
            backup_root,
            &error,
        ));
    }
    Ok(())
}

fn read_manifest_bytes_error(error: &std::io::Error) -> MaintenanceError {
    MaintenanceError {
        code: MaintenanceErrorCode::BackupVerifyFailed,
        message: "backup_verify_failed".to_string(),
        details: json!({
            "reason": if error.to_string() == "member_too_large" { "member_too_large" } else { "read_manifest" },
            "cause": error.to_string()
        }),
    }
}

fn read_member_bytes_error(error: &std::io::Error) -> MaintenanceError {
    MaintenanceError {
        code: MaintenanceErrorCode::BackupVerifyFailed,
        message: "backup_verify_failed".to_string(),
        details: json!({
            "reason": if error.to_string() == "member_too_large" { "member_too_large" } else { "read_member" },
            "cause": error.to_string()
        }),
    }
}

fn open_restore_zip(archive: &Path) -> Result<ZipArchive<File>, MaintenanceError> {
    let file = File::open(archive).map_err(|error| maintenance_io("reopen archive", &error))?;
    ZipArchive::new(file).map_err(|error| restore_invalid_zip_error(&error))
}

fn restore_invalid_zip_error(error: &zip::result::ZipError) -> MaintenanceError {
    MaintenanceError {
        code: MaintenanceErrorCode::RestoreVerifyFailed,
        message: "restore_verify_failed".to_string(),
        details: json!({ "reason": "invalid_zip", "cause": error.to_string() }),
    }
}

fn restore_missing_member_error(path: &str) -> MaintenanceError {
    MaintenanceError {
        code: MaintenanceErrorCode::RestoreVerifyFailed,
        message: "restore_verify_failed".to_string(),
        details: json!({ "reason": "missing_member", "path": path }),
    }
}

fn serialize_backup_manifest(manifest: &BackupManifest) -> Result<Vec<u8>, MaintenanceError> {
    serde_json::to_vec_pretty(manifest).map_err(|error| serialize_manifest_error(&error))
}

fn serialize_manifest_error(error: &serde_json::Error) -> MaintenanceError {
    MaintenanceError {
        code: MaintenanceErrorCode::MaintenanceIoFailed,
        message: "maintenance_io_failed".to_string(),
        details: json!({ "operation": "serialize_manifest", "reason": error.to_string() }),
    }
}

fn cleanup_failed_backup_temp(temporary: &Path) {
    let _ = fs::remove_file(temporary);
}

fn commit_backup_zip(temporary: &Path, destination: &Path) -> Result<(), MaintenanceError> {
    fs::rename(temporary, destination).map_err(|error| {
        cleanup_failed_backup_temp(temporary);
        maintenance_io("commit backup zip", &error)
    })
}

fn restore_parent(root: &Path) -> Result<&Path, MaintenanceError> {
    root.parent().ok_or_else(store_root_has_no_parent_error)
}

fn store_root_has_no_parent_error() -> MaintenanceError {
    MaintenanceError {
        code: MaintenanceErrorCode::MaintenanceIoFailed,
        message: "maintenance_io_failed".to_string(),
        details: json!({ "operation": "restore", "reason": "store_root_has_no_parent" }),
    }
}

fn handle_move_live_store_aside_failure(
    maintenance: &mut LocalFileDataStoreMaintenance,
    error: &std::io::Error,
) -> MaintenanceError {
    let _ = maintenance.reacquire_lock();
    maintenance_io("move live store aside for restore", error)
}

fn handle_replace_store_root_failure(
    maintenance: &mut LocalFileDataStoreMaintenance,
    backup_root: &Path,
    error: &std::io::Error,
) -> MaintenanceError {
    let _ = fs::rename(backup_root, &maintenance.root);
    let _ = maintenance.reacquire_lock();
    maintenance_io("atomically replace store root", error)
}

fn maintenance_io(operation: &str, error: &std::io::Error) -> MaintenanceError {
    MaintenanceError {
        code: MaintenanceErrorCode::MaintenanceIoFailed,
        message: "maintenance_io_failed".to_string(),
        details: json!({ "operation": operation, "reason": error.to_string() }),
    }
}

fn maintenance_zip(operation: &str, error: impl std::fmt::Display) -> MaintenanceError {
    MaintenanceError {
        code: MaintenanceErrorCode::MaintenanceIoFailed,
        message: "maintenance_io_failed".to_string(),
        details: json!({ "operation": operation, "reason": error.to_string() }),
    }
}

fn map_lock_to_maintenance(error: DataStoreError) -> MaintenanceError {
    match error.code {
        super::DataStoreErrorCode::StoreLocked => MaintenanceError {
            code: MaintenanceErrorCode::StoreLocked,
            message: "store_locked".to_string(),
            details: error.details,
        },
        _ => MaintenanceError {
            code: MaintenanceErrorCode::MaintenanceIoFailed,
            message: "maintenance_io_failed".to_string(),
            details: error.details,
        },
    }
}

#[allow(clippy::needless_pass_by_value)]
fn map_data_store_error(error: DataStoreError) -> MaintenanceError {
    MaintenanceError {
        code: MaintenanceErrorCode::MaintenanceIoFailed,
        message: "maintenance_io_failed".to_string(),
        details: json!({ "cause": error.message, "details": error.details }),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]

    use super::*;
    use crate::data_store::{
        DataStore, DataStoreError, DataStoreErrorCode, InMemoryKeyProvider, KeyProvider,
        LocalFileDataStore, StateRecord,
    };
    use serde_json::json;
    use std::fs::{self, File, OpenOptions, TryLockError};
    use std::path::Path;
    use std::sync::Arc;
    use uuid::Uuid;

    fn temp_root(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "traverse-maintenance-{label}-{}-{}",
            std::process::id(),
            Uuid::new_v4()
        ));
        fs::create_dir_all(&root).expect("temp root");
        root
    }

    fn seed_store(root: &Path, count: usize, stamp: Option<&str>) -> LocalFileDataStore {
        let mut store =
            LocalFileDataStore::with_classification(root, LocalDataClassification::Public)
                .expect("open");
        if let Some(stamp) = stamp {
            store.set_write_retained_at(Some(stamp.to_string()));
        }
        for index in 0..count {
            store
                .write(StateRecord {
                    key: format!("k{index:02}"),
                    value: json!({ "n": index }),
                    lamport_clock: (index + 1) as u64,
                    writer_id: "host".to_string(),
                })
                .expect("write");
        }
        store
    }

    #[test]
    fn prune_max_count_removes_oldest_prefix() {
        let root = temp_root("count");
        let store = seed_store(&root, 10, None);
        drop(store);
        let mut maintenance = LocalFileDataStoreMaintenance::open(&root).expect("maintenance");
        let evidence = maintenance
            .prune(
                &RetentionPolicy {
                    max_count: Some(3),
                    max_age_secs: None,
                },
                "2026-07-29T00:00:00Z",
            )
            .expect("prune");
        assert_eq!(evidence.outcome, "prune_completed");
        assert_eq!(evidence.removed_count, 7);
        assert_eq!(evidence.retained_count, 3);
        drop(maintenance);
        let store = LocalFileDataStore::new(&root).expect("reopen");
        let keys = store.list_keys().expect("keys");
        assert_eq!(
            keys,
            vec!["k07".to_string(), "k08".to_string(), "k09".to_string()]
        );
    }

    #[test]
    fn prune_rejects_empty_policy_and_reports_age_bounds() {
        let root = temp_root("policy");
        drop(seed_store(&root, 3, Some("2026-07-01T00:00:00Z")));
        let mut maintenance = LocalFileDataStoreMaintenance::open(&root).expect("maintenance");
        let invalid = maintenance
            .prune(
                &RetentionPolicy {
                    max_count: None,
                    max_age_secs: None,
                },
                "2026-07-29T00:00:00Z",
            )
            .expect_err("policy");
        assert_eq!(invalid.code, MaintenanceErrorCode::InvalidRetentionPolicy);

        let evidence = maintenance
            .prune(
                &RetentionPolicy {
                    max_count: None,
                    max_age_secs: Some(7 * 24 * 60 * 60),
                },
                "2026-07-29T00:00:00Z",
            )
            .expect("age prune");
        assert_eq!(evidence.removed_count, 3);
    }

    #[test]
    fn prune_partial_failure_preserves_remaining_victims() {
        let root = temp_root("partial");
        drop(seed_store(&root, 5, None));
        let mut maintenance = LocalFileDataStoreMaintenance::open(&root).expect("maintenance");
        // Turn the second victim path into a directory so delete fails mid-prune.
        let victim = root.join("k01.json");
        fs::remove_file(&victim).expect("remove");
        fs::create_dir_all(&victim).expect("dir");
        fs::write(victim.join("nested"), b"x").expect("nested");
        let failure = maintenance
            .prune(
                &RetentionPolicy {
                    max_count: Some(2),
                    max_age_secs: None,
                },
                "2026-07-29T00:00:00Z",
            )
            .expect_err("partial");
        assert_eq!(failure.code, MaintenanceErrorCode::MaintenanceIoFailed);
        assert_eq!(failure.details["removed_count"], 1);
        // First victim k00 removed; k01 blocked; k02 still present among remaining.
        assert!(!root.join("k00.json").exists());
        assert!(root.join("k02.json").exists());
    }

    #[test]
    fn backup_restore_round_trip_including_empty_store() {
        let root = temp_root("backup");
        drop(seed_store(&root, 2, Some("2026-07-28T00:00:00Z")));
        let archive = root
            .parent()
            .expect("parent")
            .join(format!("backup-{}.zip", Uuid::new_v4()));
        {
            let mut maintenance = LocalFileDataStoreMaintenance::open(&root).expect("maintenance");
            let evidence = maintenance
                .backup(&archive, "2026-07-29T00:00:00Z")
                .expect("backup");
            assert_eq!(evidence.outcome, "backup_created");
            assert!(evidence.archive_content_digest.is_some());
        }
        // Mutate store then restore.
        {
            let mut store =
                LocalFileDataStore::with_classification(&root, LocalDataClassification::Public)
                    .expect("open");
            store
                .write(StateRecord {
                    key: "extra".to_string(),
                    value: json!({ "x": 1 }),
                    lamport_clock: 99,
                    writer_id: "host".to_string(),
                })
                .expect("extra");
        }
        {
            let mut maintenance = LocalFileDataStoreMaintenance::open(&root).expect("maintenance");
            let evidence = maintenance
                .restore(&archive, "2026-07-29T01:00:00Z")
                .expect("restore");
            assert_eq!(evidence.outcome, "restore_committed");
        }
        let store = LocalFileDataStore::new(&root).expect("reopen");
        let keys = store.list_keys().expect("keys");
        assert_eq!(keys, vec!["k00".to_string(), "k01".to_string()]);

        // Empty store backup/restore.
        let empty_root = temp_root("empty");
        drop(LocalFileDataStore::new(&empty_root).expect("empty"));
        let empty_archive = empty_root
            .parent()
            .expect("parent")
            .join(format!("empty-{}.zip", Uuid::new_v4()));
        {
            let mut maintenance =
                LocalFileDataStoreMaintenance::open(&empty_root).expect("maintenance");
            maintenance
                .backup(&empty_archive, "2026-07-29T00:00:00Z")
                .expect("empty backup");
            maintenance
                .restore(&empty_archive, "2026-07-29T00:00:00Z")
                .expect("empty restore");
        }
    }

    #[test]
    fn backup_restore_verifies_private_envelopes_without_plaintext() {
        let root = temp_root("private-backup");
        let provider: Arc<dyn KeyProvider> =
            Arc::new(InMemoryKeyProvider::new("backup-key", [11; 32]));
        let expected = StateRecord {
            key: "secret".to_string(),
            value: json!({ "private": true }),
            lamport_clock: 1,
            writer_id: "host".to_string(),
        };
        let mut store = LocalFileDataStore::new(&root)
            .expect("open")
            .with_key_provider(Arc::clone(&provider));
        store.write(expected.clone()).expect("private write");
        drop(store);

        let archive = root
            .parent()
            .expect("parent")
            .join(format!("private-{}.zip", Uuid::new_v4()));
        let mut maintenance = LocalFileDataStoreMaintenance::open(&root).expect("maintenance");
        maintenance
            .backup(&archive, "2026-07-29T00:00:00Z")
            .expect("private backup");
        maintenance
            .restore(&archive, "2026-07-29T05:00:00Z")
            .expect("private restore");
        drop(maintenance);

        let reopened = LocalFileDataStore::new(&root)
            .expect("reopen")
            .with_key_provider(provider);
        assert_eq!(reopened.read("secret").expect("decrypt"), Some(expected));
    }

    #[test]
    fn second_owner_receives_store_locked() {
        let root = temp_root("locked");
        let _owner = LocalFileDataStore::new(&root).expect("owner");
        let locked = LocalFileDataStoreMaintenance::open(&root).expect_err("locked");
        assert_eq!(locked.code, MaintenanceErrorCode::StoreLocked);
    }

    #[test]
    fn backup_verify_rejects_tampered_archive() {
        let root = temp_root("tamper");
        drop(seed_store(&root, 1, None));
        let archive = root
            .parent()
            .expect("parent")
            .join(format!("tamper-{}.zip", Uuid::new_v4()));
        {
            let mut maintenance = LocalFileDataStoreMaintenance::open(&root).expect("maintenance");
            maintenance
                .backup(&archive, "2026-07-29T00:00:00Z")
                .expect("backup");
        }
        let mut bytes = fs::read(&archive).expect("read");
        if let Some(byte) = bytes.last_mut() {
            *byte ^= 0xff;
        }
        fs::write(&archive, bytes).expect("write");
        let failure = verify_backup_archive(&archive).expect_err("tampered");
        assert_eq!(failure.code, MaintenanceErrorCode::BackupVerifyFailed);
    }

    #[test]
    fn local_file_store_root_accessor_returns_open_path() {
        let root = temp_root("root-accessor");
        let store = LocalFileDataStore::new(&root).expect("open");
        assert_eq!(store.root(), &root);
    }

    #[test]
    fn maintenance_error_helpers_report_stable_codes() {
        let io = std::io::Error::other("disk full");
        let maintenance = maintenance_io("unit op", &io);
        assert_eq!(maintenance.code, MaintenanceErrorCode::MaintenanceIoFailed);
        assert_eq!(maintenance.details["operation"], "unit op");

        let zip = maintenance_zip("zip op", "zip failed");
        assert_eq!(zip.code, MaintenanceErrorCode::MaintenanceIoFailed);
        assert_eq!(zip.details["operation"], "zip op");

        let locked = map_lock_to_maintenance(lock_error(TryLockError::WouldBlock));
        assert_eq!(locked.code, MaintenanceErrorCode::StoreLocked);

        let lock_io = map_lock_to_maintenance(lock_error(TryLockError::Error(
            std::io::Error::other("lock device"),
        )));
        assert_eq!(lock_io.code, MaintenanceErrorCode::MaintenanceIoFailed);

        let mapped = map_data_store_error(DataStoreError {
            code: DataStoreErrorCode::InvalidKey,
            message: "invalid".to_string(),
            details: json!({ "key": "bad.key" }),
        });
        assert_eq!(mapped.code, MaintenanceErrorCode::MaintenanceIoFailed);

        let parent = restore_parent(Path::new("/")).expect_err("root has no parent");
        assert_eq!(parent.code, MaintenanceErrorCode::MaintenanceIoFailed);
        assert_eq!(parent.details["reason"], "store_root_has_no_parent");

        let parse_error = serde_json::from_str::<Value>("{").expect_err("invalid json");
        let manifest = serialize_manifest_error(&parse_error);
        assert_eq!(manifest.details["operation"], "serialize_manifest");

        let io = std::io::Error::other("read failed");
        assert_eq!(
            read_manifest_bytes_error(&io).details["reason"],
            "read_manifest"
        );
        assert_eq!(
            read_member_bytes_error(&io).details["reason"],
            "read_member"
        );

        let bad = temp_root("restore-zip").join("bad.zip");
        fs::write(&bad, b"not a zip").expect("write");
        let invalid = open_restore_zip(&bad).expect_err("invalid");
        assert_eq!(invalid.details["reason"], "invalid_zip");

        let missing = restore_missing_member_error("records/missing.json");
        assert_eq!(missing.details["reason"], "missing_member");
    }

    #[test]
    fn age_prune_retains_unstamped_envelopes() {
        let root = temp_root("unstamped");
        drop(seed_store(&root, 2, None));
        let mut maintenance = LocalFileDataStoreMaintenance::open(&root).expect("maintenance");
        let evidence = maintenance
            .prune(
                &RetentionPolicy {
                    max_count: None,
                    max_age_secs: Some(86_400),
                },
                "2026-07-29T00:00:00Z",
            )
            .expect("prune");
        assert_eq!(evidence.removed_count, 0);
        assert_eq!(evidence.retained_count, 2);
    }

    #[test]
    fn backup_without_destination_parent_skips_parent_creation() {
        let root = temp_root("skip-parent");
        drop(seed_store(&root, 1, None));
        let mut maintenance = LocalFileDataStoreMaintenance::open(&root).expect("maintenance");
        let destination = PathBuf::new();
        assert!(destination.parent().is_none());
        let _ = maintenance.backup(&destination, "2026-07-29T00:00:00Z");
    }

    #[test]
    fn restore_replace_failure_rolls_back_live_store() {
        let root = temp_root("replace-live");
        drop(seed_store(&root, 1, None));
        let parent = root.parent().expect("parent");
        let temp_root_path = parent.join(format!(
            ".traverse-datastore-restore-{}-manual",
            std::process::id()
        ));
        let backup_root = parent.join(format!(
            ".traverse-datastore-replaced-{}-manual",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&temp_root_path);
        let _ = fs::remove_dir_all(&backup_root);
        fs::create_dir_all(&temp_root_path).expect("temp");
        fs::write(temp_root_path.join(LOCAL_DATA_STORE_LOCK_FILE), b"").expect("lock");
        let mut maintenance = LocalFileDataStoreMaintenance::open(&root).expect("maintenance");
        move_live_store_aside(&mut maintenance, &backup_root).expect("move aside");
        fs::write(&root, "block replace").expect("occupy root path");
        let failure = replace_store_root(&mut maintenance, &temp_root_path, &backup_root)
            .expect_err("replace");
        assert_eq!(
            failure.details["operation"],
            "atomically replace store root"
        );
        let _ = fs::remove_file(&root);
        let _ = fs::rename(&backup_root, &root);
        let _ = fs::remove_dir_all(&temp_root_path);
    }

    #[test]
    fn verify_io_error_helpers_and_truncated_manifest_reads_fail() {
        let root = temp_root("truncated");
        drop(seed_store(&root, 1, None));
        let archive = root
            .parent()
            .expect("parent")
            .join(format!("trunc-{}.zip", Uuid::new_v4()));
        {
            let mut maintenance = LocalFileDataStoreMaintenance::open(&root).expect("maintenance");
            maintenance
                .backup(&archive, "2026-07-29T00:00:00Z")
                .expect("backup");
        }
        let file = OpenOptions::new().write(true).open(&archive).expect("open");
        file.set_len(32).expect("truncate");
        drop(file);
        let failure = verify_backup_archive(&archive).expect_err("truncated");
        assert_eq!(failure.code, MaintenanceErrorCode::BackupVerifyFailed);
    }

    #[test]
    fn parse_as_of_and_policy_bounds_reject_invalid_inputs() {
        let invalid = parse_as_of("not-a-timestamp").expect_err("invalid as_of");
        assert_eq!(invalid.code, MaintenanceErrorCode::InvalidRetentionPolicy);
        assert_eq!(invalid.details["reason"], "invalid_as_of");

        let root = temp_root("age-bounds");
        drop(seed_store(&root, 1, Some("2026-07-01T00:00:00Z")));
        let _maintenance = LocalFileDataStoreMaintenance::open(&root).expect("maintenance");
        let overflow = age_cutoff(
            parse_as_of("2026-07-29T00:00:00Z").expect("as_of"),
            u64::MAX,
        )
        .expect_err("max age overflow");
        assert_eq!(overflow.details["reason"], "max_age_secs_out_of_range");

        let underflow = age_cutoff(
            parse_as_of("2026-07-29T00:00:00Z").expect("as_of"),
            9_000_000_000_000,
        )
        .expect_err("as_of underflow");
        assert_eq!(underflow.details["reason"], "as_of_minus_max_age_underflow");
        assert_eq!(
            max_age_secs_out_of_range_error().details["reason"],
            "max_age_secs_out_of_range"
        );
        assert_eq!(
            as_of_minus_max_age_underflow_error().details["reason"],
            "as_of_minus_max_age_underflow"
        );
    }

    #[test]
    fn parse_envelope_bytes_rejects_malformed_legacy_and_tampered_records() {
        let malformed = parse_envelope_bytes(b"{").expect_err("malformed");
        assert_eq!(malformed.code, MaintenanceErrorCode::RestoreVerifyFailed);

        let legacy = json!({
            "record": {
                "key": "k",
                "value": {},
                "lamport_clock": 1,
                "writer_id": "host"
            },
            "digest": "sha256:00"
        });
        let legacy_err = parse_envelope_bytes(&serde_json::to_vec(&legacy).expect("serialize"))
            .expect_err("legacy");
        assert_eq!(
            legacy_err.code,
            MaintenanceErrorCode::UnsupportedStoreFormat
        );
        assert_eq!(legacy_err.details["reason"], "legacy_unverified");

        let record = StateRecord {
            key: "k".to_string(),
            value: json!({ "n": 1 }),
            lamport_clock: 1,
            writer_id: "host".to_string(),
        };
        let digest = digest_for_record(&record).expect("digest");
        let wrong_shape = json!({
            "format": LOCAL_DATA_STORE_FORMAT,
            "classification": "public",
            "record": { "key": "k" },
            "digest": digest
        });
        let shape_err = parse_envelope_bytes(&serde_json::to_vec(&wrong_shape).expect("serialize"))
            .expect_err("shape");
        assert_eq!(shape_err.code, MaintenanceErrorCode::RestoreVerifyFailed);

        let unknown = json!({
            "format": "local-datastore/99",
            "classification": "public",
            "record": record,
            "digest": digest
        });
        let version_err = parse_envelope_bytes(&serde_json::to_vec(&unknown).expect("serialize"))
            .expect_err("version");
        assert_eq!(
            version_err.code,
            MaintenanceErrorCode::UnsupportedStoreFormat
        );

        let tampered = json!({
            "format": LOCAL_DATA_STORE_FORMAT,
            "classification": "public",
            "record": record,
            "digest": "sha256:deadbeef"
        });
        let digest_err = parse_envelope_bytes(&serde_json::to_vec(&tampered).expect("serialize"))
            .expect_err("digest");
        assert_eq!(digest_err.code, MaintenanceErrorCode::RestoreVerifyFailed);
        assert_eq!(digest_err.details["reason"], "digest_mismatch");

        let public_with_private_metadata = json!({
            "format": LOCAL_DATA_STORE_FORMAT,
            "classification": "public",
            "record": record,
            "digest": digest,
            "key_id": "unexpected"
        });
        let public_shape = parse_envelope_bytes(
            &serde_json::to_vec(&public_with_private_metadata).expect("serialize"),
        )
        .expect_err("public private metadata");
        assert_eq!(public_shape.details["reason"], "malformed_envelope");

        let private_with_plaintext = json!({
            "format": LOCAL_DATA_STORE_FORMAT,
            "classification": "private",
            "record": record,
            "record_key": "k",
            "key_id": "key-1",
            "nonce": "000000000000000000000000",
            "ciphertext": "00000000000000000000000000000000",
            "digest": "sha256:00"
        });
        let private_shape =
            parse_envelope_bytes(&serde_json::to_vec(&private_with_plaintext).expect("serialize"))
                .expect_err("private plaintext");
        assert_eq!(private_shape.details["reason"], "malformed_envelope");

        let private_short_nonce = json!({
            "format": LOCAL_DATA_STORE_FORMAT,
            "classification": "private",
            "record_key": "k",
            "key_id": "key-1",
            "nonce": "00",
            "ciphertext": "00000000000000000000000000000000",
            "digest": "sha256:00"
        });
        let private_lengths =
            parse_envelope_bytes(&serde_json::to_vec(&private_short_nonce).expect("serialize"))
                .expect_err("private lengths");
        assert_eq!(private_lengths.details["reason"], "malformed_envelope");
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn verify_backup_archive_rejects_invalid_archives() {
        let missing = temp_root("missing-archive").join("missing.zip");
        let open = open_backup_zip(&missing).expect_err("missing");
        assert_eq!(open.code, MaintenanceErrorCode::BackupVerifyFailed);
        assert_eq!(open.details["reason"], "open_archive");

        let bad_zip = temp_root("bad-zip").join("bad.zip");
        fs::write(&bad_zip, b"not a zip").expect("write");
        let invalid = open_backup_zip(&bad_zip).expect_err("invalid zip");
        assert_eq!(invalid.details["reason"], "invalid_zip");

        let no_manifest = write_zip(bad_zip.parent().expect("parent"), &[]);
        let missing_manifest = verify_backup_archive(&no_manifest).expect_err("manifest");
        assert_eq!(missing_manifest.details["reason"], "missing_manifest");

        let malformed_manifest = write_zip(
            bad_zip.parent().expect("parent"),
            &[(BACKUP_MANIFEST_MEMBER, b"{".as_slice())],
        );
        let malformed = verify_backup_archive(&malformed_manifest).expect_err("malformed");
        assert_eq!(malformed.details["reason"], "malformed_manifest");

        let unsupported_manifest = backup_zip_with_manifest(&json!({
            "manifest_format_version": "99",
            "created_as_of": "2026-07-29T00:00:00Z",
            "record_count": 0,
            "archive_content_digest": "sha256:00",
            "records": [],
            "store_format": LOCAL_DATA_STORE_FORMAT,
            "writer_tool": "test",
            "writer_semver": "0.0.0"
        }));
        let version = verify_backup_archive(&unsupported_manifest).expect_err("version");
        assert_eq!(version.code, MaintenanceErrorCode::UnsupportedStoreFormat);

        let unsupported_store = backup_zip_with_manifest(&json!({
            "manifest_format_version": BACKUP_MANIFEST_VERSION,
            "created_as_of": "2026-07-29T00:00:00Z",
            "record_count": 0,
            "archive_content_digest": "sha256:00",
            "records": [],
            "store_format": "legacy/0",
            "writer_tool": "test",
            "writer_semver": "0.0.0"
        }));
        let format = verify_backup_archive(&unsupported_store).expect_err("format");
        assert_eq!(format.details["reason"], "unsupported_store_format");

        let invalid_member = backup_zip_with_manifest(&json!({
            "manifest_format_version": BACKUP_MANIFEST_VERSION,
            "created_as_of": "2026-07-29T00:00:00Z",
            "record_count": 1,
            "archive_content_digest": "sha256:00",
            "records": [{
                "key": "k00",
                "classification": "public",
                "envelope_digest": "sha256:00",
                "member_path": "../escape.json"
            }],
            "store_format": LOCAL_DATA_STORE_FORMAT,
            "writer_tool": "test",
            "writer_semver": "0.0.0"
        }));
        let path = verify_backup_archive(&invalid_member).expect_err("path");
        assert_eq!(path.details["reason"], "invalid_member_path");

        let root = temp_root("member-errors");
        drop(seed_store(&root, 1, None));
        let good = root
            .parent()
            .expect("parent")
            .join(format!("good-{}.zip", Uuid::new_v4()));
        {
            let mut maintenance = LocalFileDataStoreMaintenance::open(&root).expect("maintenance");
            maintenance
                .backup(&good, "2026-07-29T00:00:00Z")
                .expect("backup");
        }
        let envelope_bytes = fs::read(root.join("k00.json")).expect("envelope");
        let missing_member = write_zip_with_manifest(
            good.parent().expect("parent"),
            &json!({
                "manifest_format_version": BACKUP_MANIFEST_VERSION,
                "created_as_of": "2026-07-29T00:00:00Z",
                "record_count": 1,
                "archive_content_digest": "sha256:00",
                "records": [{
                    "key": "k00",
                    "classification": "public",
                    "envelope_digest": "sha256:00",
                    "member_path": "records/missing.json"
                }],
                "store_format": LOCAL_DATA_STORE_FORMAT,
                "writer_tool": "test",
                "writer_semver": "0.0.0"
            }),
            &[],
        );
        let missing = verify_backup_archive(&missing_member).expect_err("missing member");
        assert_eq!(missing.details["reason"], "missing_member");

        let digest_mismatch = write_zip_with_manifest(
            good.parent().expect("parent"),
            &json!({
                "manifest_format_version": BACKUP_MANIFEST_VERSION,
                "created_as_of": "2026-07-29T00:00:00Z",
                "record_count": 1,
                "archive_content_digest": "sha256:00",
                "records": [{
                    "key": "k00",
                    "classification": "public",
                    "envelope_digest": "sha256:deadbeef",
                    "member_path": "records/k00.json"
                }],
                "store_format": LOCAL_DATA_STORE_FORMAT,
                "writer_tool": "test",
                "writer_semver": "0.0.0"
            }),
            &[("records/k00.json", envelope_bytes.as_slice())],
        );
        let digest = verify_backup_archive(&digest_mismatch).expect_err("digest");
        assert_eq!(digest.details["reason"], "member_digest_mismatch");

        let key_mismatch = write_zip_with_manifest(
            good.parent().expect("parent"),
            &json!({
                "manifest_format_version": BACKUP_MANIFEST_VERSION,
                "created_as_of": "2026-07-29T00:00:00Z",
                "record_count": 1,
                "archive_content_digest": archive_content_digest_from_members(&[(
                    "records/k00.json",
                    envelope_bytes.as_slice(),
                )]),
                "records": [{
                    "key": "other",
                    "classification": "public",
                    "envelope_digest": sha256_hex(&envelope_bytes),
                    "member_path": "records/k00.json"
                }],
                "store_format": LOCAL_DATA_STORE_FORMAT,
                "writer_tool": "test",
                "writer_semver": "0.0.0"
            }),
            &[("records/k00.json", envelope_bytes.as_slice())],
        );
        let key = verify_backup_archive(&key_mismatch).expect_err("key");
        assert_eq!(key.details["reason"], "key_mismatch");

        let count_mismatch = write_zip_with_manifest(
            good.parent().expect("parent"),
            &json!({
                "manifest_format_version": BACKUP_MANIFEST_VERSION,
                "created_as_of": "2026-07-29T00:00:00Z",
                "record_count": 2,
                "archive_content_digest": archive_content_digest_from_members(&[(
                    "records/k00.json",
                    envelope_bytes.as_slice(),
                )]),
                "records": [{
                    "key": "k00",
                    "classification": "public",
                    "envelope_digest": sha256_hex(&envelope_bytes),
                    "member_path": "records/k00.json"
                }],
                "store_format": LOCAL_DATA_STORE_FORMAT,
                "writer_tool": "test",
                "writer_semver": "0.0.0"
            }),
            &[("records/k00.json", envelope_bytes.as_slice())],
        );
        let count = verify_backup_archive(&count_mismatch).expect_err("count");
        assert_eq!(count.details["reason"], "record_count_mismatch");

        let digest_mismatch_archive = write_zip_with_manifest(
            good.parent().expect("parent"),
            &json!({
                "manifest_format_version": BACKUP_MANIFEST_VERSION,
                "created_as_of": "2026-07-29T00:00:00Z",
                "record_count": 1,
                "archive_content_digest": "sha256:deadbeef",
                "records": [{
                    "key": "k00",
                    "classification": "public",
                    "envelope_digest": sha256_hex(&envelope_bytes),
                    "member_path": "records/k00.json"
                }],
                "store_format": LOCAL_DATA_STORE_FORMAT,
                "writer_tool": "test",
                "writer_semver": "0.0.0"
            }),
            &[("records/k00.json", envelope_bytes.as_slice())],
        );
        let archive_digest =
            verify_backup_archive(&digest_mismatch_archive).expect_err("archive digest");
        assert_eq!(
            archive_digest.details["reason"],
            "archive_content_digest_mismatch"
        );
    }

    #[test]
    fn verify_backup_archive_rejects_oversized_member_before_digest_read() {
        let oversized_size = usize::try_from(MAX_BACKUP_MEMBER_BYTES + 1)
            .expect("governed member ceiling fits usize");
        let oversized = vec![0_u8; oversized_size];
        let archive = write_zip_with_manifest(
            &temp_root("oversized-backup-member"),
            &json!({
                "manifest_format_version": BACKUP_MANIFEST_VERSION,
                "created_as_of": "2026-07-29T00:00:00Z",
                "record_count": 1,
                "archive_content_digest": "sha256:00",
                "records": [{
                    "key": "k00",
                    "classification": "public",
                    "envelope_digest": "sha256:00",
                    "member_path": "records/k00.json"
                }],
                "store_format": LOCAL_DATA_STORE_FORMAT,
                "writer_tool": "test",
                "writer_semver": "0.0.0"
            }),
            &[("records/k00.json", oversized.as_slice())],
        );
        let failure = verify_backup_archive(&archive).expect_err("oversized member");
        assert_eq!(failure.code, MaintenanceErrorCode::BackupVerifyFailed);
        assert_eq!(failure.details["reason"], "member_too_large");
    }

    #[test]
    fn open_and_backup_surface_io_failures_without_weakening_fail_closed() {
        let file_root = temp_root("file-root");
        fs::remove_dir(&file_root).expect("remove dir");
        fs::write(&file_root, "not a directory").expect("write");
        let open = LocalFileDataStoreMaintenance::open(&file_root).expect_err("open");
        assert_eq!(open.code, MaintenanceErrorCode::MaintenanceIoFailed);

        let root = temp_root("backup-io");
        drop(seed_store(&root, 1, None));
        let parent = root.parent().expect("parent");
        let file_parent = parent.join(format!("file-parent-{}", Uuid::new_v4()));
        fs::write(&file_parent, "blocker").expect("write");
        let blocked = file_parent.join("backup.zip");
        {
            let mut maintenance = LocalFileDataStoreMaintenance::open(&root).expect("maintenance");
            let failure = maintenance
                .backup(&blocked, "2026-07-29T00:00:00Z")
                .expect_err("destination parent blocked");
            assert_eq!(failure.code, MaintenanceErrorCode::MaintenanceIoFailed);
        }

        let commit_blocked = parent.join(format!("commit-dir-{}", Uuid::new_v4()));
        fs::create_dir_all(&commit_blocked).expect("dir");
        let destination = commit_blocked.join("backup.zip");
        fs::create_dir_all(&destination).expect("block destination");
        let temporary = parent.join(format!("backup-{}.zip.tmp", Uuid::new_v4()));
        fs::write(&temporary, b"partial").expect("temp");
        let commit = commit_backup_zip(&temporary, &destination).expect_err("commit");
        assert_eq!(commit.details["operation"], "commit backup zip");
        assert!(!temporary.exists());

        cleanup_failed_backup_temp(&parent.join("missing-temp.zip"));
    }

    #[test]
    fn backup_rejects_invalid_as_of_and_verify_failure_cleans_temp() {
        let root = temp_root("backup-as-of");
        drop(seed_store(&root, 1, None));
        let archive = root
            .parent()
            .expect("parent")
            .join(format!("asof-{}.zip", Uuid::new_v4()));
        let mut maintenance = LocalFileDataStoreMaintenance::open(&root).expect("maintenance");
        let invalid = maintenance
            .backup(&archive, "not-a-timestamp")
            .expect_err("as_of");
        assert_eq!(invalid.code, MaintenanceErrorCode::InvalidRetentionPolicy);

        let temporary = archive.with_extension("zip.tmp");
        fs::write(&temporary, b"not a zip").expect("temp");
        let failure = verify_backup_archive(&temporary)
            .inspect_err(|_| cleanup_failed_backup_temp(&temporary));
        assert!(failure.is_err());
        assert!(!temporary.exists());
    }

    #[test]
    fn restore_reports_invalid_as_of_and_rename_failures() {
        let root = temp_root("restore-as-of");
        drop(seed_store(&root, 1, None));
        let archive = root
            .parent()
            .expect("parent")
            .join(format!("restore-{}.zip", Uuid::new_v4()));
        {
            let mut maintenance = LocalFileDataStoreMaintenance::open(&root).expect("maintenance");
            maintenance
                .backup(&archive, "2026-07-29T00:00:00Z")
                .expect("backup");
        }
        let mut maintenance = LocalFileDataStoreMaintenance::open(&root).expect("maintenance");
        let invalid = maintenance
            .restore(&archive, "not-a-timestamp")
            .expect_err("as_of");
        assert_eq!(invalid.code, MaintenanceErrorCode::InvalidRetentionPolicy);
        drop(maintenance);

        let mut maintenance = LocalFileDataStoreMaintenance::open(&root).expect("maintenance");
        let parent = root.parent().expect("parent");
        let backup_root = parent.join(format!(
            ".traverse-datastore-replaced-{}",
            restore_work_suffix(&root, "2026-07-29T01:00:00Z")
        ));
        let _ = fs::remove_dir_all(&backup_root);
        let _ = fs::remove_file(&backup_root);
        fs::write(&backup_root, "blocker").expect("blocker");
        let blocked = maintenance
            .restore(&archive, "2026-07-29T01:00:00Z")
            .expect_err("move aside");
        assert_eq!(blocked.code, MaintenanceErrorCode::MaintenanceIoFailed);
        let _ = fs::remove_file(&backup_root);
        drop(maintenance);

        let mut rollback_maintenance =
            LocalFileDataStoreMaintenance::open(&root).expect("maintenance");
        let simulated = std::io::Error::other("simulated");
        let rollback_error =
            handle_move_live_store_aside_failure(&mut rollback_maintenance, &simulated);
        assert_eq!(
            rollback_error.details["operation"],
            "move live store aside for restore"
        );

        let replace_root = temp_root("replace-failure");
        drop(seed_store(&replace_root, 1, None));
        let replace_archive = replace_root
            .parent()
            .expect("parent")
            .join(format!("replace-{}.zip", Uuid::new_v4()));
        {
            let mut writer =
                LocalFileDataStoreMaintenance::open(&replace_root).expect("maintenance");
            writer
                .backup(&replace_archive, "2026-07-29T00:00:00Z")
                .expect("backup");
        }
        let mut replacer = LocalFileDataStoreMaintenance::open(&replace_root).expect("maintenance");
        let simulated = std::io::Error::other("simulated");
        let replace = handle_replace_store_root_failure(
            &mut replacer,
            &replace_root.join("backup-aside"),
            &simulated,
        );
        assert_eq!(
            replace.details["operation"],
            "atomically replace store root"
        );
    }

    #[test]
    fn materialize_archive_reports_missing_members_and_invalid_zip() {
        let root = temp_root("materialize");
        fs::create_dir_all(&root).expect("dir");
        let bad_zip = temp_root("materialize-bad").join("bad.zip");
        fs::write(&bad_zip, b"not a zip").expect("write");
        let invalid = materialize_archive_to_root(&bad_zip, &root).expect_err("zip");
        assert_eq!(invalid.code, MaintenanceErrorCode::BackupVerifyFailed);

        let missing_member = write_zip_with_manifest(
            bad_zip.parent().expect("parent"),
            &json!({
                "manifest_format_version": BACKUP_MANIFEST_VERSION,
                "created_as_of": "2026-07-29T00:00:00Z",
                "record_count": 1,
                "archive_content_digest": "sha256:00",
                "records": [{
                    "key": "k00",
                    "classification": "public",
                    "envelope_digest": "sha256:00",
                    "member_path": "records/k00.json"
                }],
                "store_format": LOCAL_DATA_STORE_FORMAT,
                "writer_tool": "test",
                "writer_semver": "0.0.0"
            }),
            &[],
        );
        let missing = materialize_archive_to_root(&missing_member, &root).expect_err("missing");
        assert_eq!(missing.code, MaintenanceErrorCode::BackupVerifyFailed);
        assert_eq!(missing.details["reason"], "missing_member");
    }

    #[test]
    fn explicit_v1_to_v2_migration_is_verified_and_restore_remains_explicit() {
        let root = temp_root("v2-migration-success");
        drop(seed_store(&root, 2, None));
        let archive = root
            .parent()
            .expect("parent")
            .join(format!("migration-{}.zip", Uuid::new_v4()));

        let mut maintenance = LocalFileDataStoreMaintenance::open(&root).expect("maintenance");
        let report = maintenance
            .migrate_v1_to_v2(&archive, "2026-08-05T00:00:00Z")
            .expect("migration");
        assert_eq!(report.governing_spec, MIGRATION_SPEC);
        assert_eq!(report.source_format, LOCAL_DATA_STORE_FORMAT);
        assert_eq!(report.target_format, LOCAL_DATA_STORE_V2_FORMAT);
        assert_eq!(report.record_count, 2);
        assert!(report.backup_content_digest.starts_with("sha256:"));
        assert_eq!(
            serde_json::from_slice::<Value>(&fs::read(root.join("k00.json")).expect("v2 file"))
                .expect("v2 json")["format"],
            LOCAL_DATA_STORE_V2_FORMAT
        );
        drop(maintenance);

        let mut reader =
            LocalFileDataStore::with_classification(&root, LocalDataClassification::Public)
                .expect("v2 reader");
        assert_eq!(
            reader.read("k00").expect("read").expect("record").key,
            "k00"
        );
        reader
            .write(StateRecord {
                key: "k00".to_string(),
                value: json!({ "n": 99 }),
                lamport_clock: 99,
                writer_id: "host".to_string(),
            })
            .expect("v2-preserving write");
        assert_eq!(
            serde_json::from_slice::<Value>(&fs::read(root.join("k00.json")).expect("v2 rewrite"))
                .expect("v2 json")["format"],
            LOCAL_DATA_STORE_V2_FORMAT
        );
        drop(reader);

        let mut restore = LocalFileDataStoreMaintenance::open(&root).expect("restore maintenance");
        restore
            .restore(&archive, "2026-08-05T00:01:00Z")
            .expect("explicit restore");
        drop(restore);
        let restored =
            serde_json::from_slice::<Value>(&fs::read(root.join("k00.json")).expect("v1 file"))
                .expect("v1 json");
        assert_eq!(restored["format"], LOCAL_DATA_STORE_FORMAT);
    }

    #[test]
    fn migration_rejects_unknown_source_without_backup_or_rewrite() {
        let root = temp_root("v2-migration-unknown");
        fs::write(root.join("k00.json"), br#"{"format":"unknown/9"}"#).expect("source");
        let original = fs::read(root.join("k00.json")).expect("original");
        let archive = root
            .parent()
            .expect("parent")
            .join(format!("migration-unknown-{}.zip", Uuid::new_v4()));
        let mut maintenance = LocalFileDataStoreMaintenance::open(&root).expect("maintenance");
        let error = maintenance
            .migrate_v1_to_v2(&archive, "2026-08-05T00:00:00Z")
            .expect_err("unknown source must fail closed");
        assert_eq!(error.code, MigrationErrorCode::SourceInvalid);
        assert_eq!(error.message, "datastore_source_invalid");
        assert_eq!(
            fs::read(root.join("k00.json")).expect("source preserved"),
            original
        );
        assert!(!archive.exists());
    }

    #[test]
    fn corrupt_restore_backup_preserves_the_committed_store() {
        let root = temp_root("v2-migration-corrupt-backup");
        drop(seed_store(&root, 1, None));
        let before = fs::read(root.join("k00.json")).expect("source");
        let corrupt = root
            .parent()
            .expect("parent")
            .join(format!("migration-corrupt-{}.zip", Uuid::new_v4()));
        fs::write(&corrupt, b"not a verified backup").expect("corrupt archive");
        let mut maintenance = LocalFileDataStoreMaintenance::open(&root).expect("maintenance");
        let error = maintenance
            .restore(&corrupt, "2026-08-05T00:00:00Z")
            .expect_err("corrupt archive");
        assert_eq!(error.code, MaintenanceErrorCode::BackupVerifyFailed);
        assert_eq!(
            fs::read(root.join("k00.json")).expect("source preserved"),
            before
        );
    }

    #[test]
    fn migration_error_mapping_is_stable_and_secret_free() {
        let maintenance = MaintenanceError {
            code: MaintenanceErrorCode::MaintenanceIoFailed,
            message: "internal path must not escape".to_string(),
            details: json!({ "path": "/secret" }),
        };
        let io = std::io::Error::other("internal path must not escape");
        let data_store = DataStoreError {
            code: DataStoreErrorCode::IoFailure,
            message: "internal path must not escape".to_string(),
            details: json!({ "path": "/secret" }),
        };
        let mapped = [
            map_migration_source_error(maintenance.clone()),
            map_migration_backup_error(maintenance.clone()),
            map_migration_write_io(io),
            map_migration_write_data_store(data_store),
            map_migration_verification_error(maintenance.clone()),
            map_migration_commit_io(std::io::Error::other("internal")),
            map_migration_commit_error(maintenance),
        ];
        let expected = [
            MigrationErrorCode::SourceInvalid,
            MigrationErrorCode::BackupFailed,
            MigrationErrorCode::WriteFailed,
            MigrationErrorCode::WriteFailed,
            MigrationErrorCode::VerificationFailed,
            MigrationErrorCode::CommitFailed,
            MigrationErrorCode::CommitFailed,
        ];
        for (error, code) in mapped.into_iter().zip(expected) {
            assert_eq!(error.code, code);
            assert!(!error.details.to_string().contains("secret"));
        }
    }

    #[test]
    fn migration_requires_verified_backup_digest() {
        let error = verified_backup_digest(None).expect_err("digest is required");
        assert_eq!(error.code, MigrationErrorCode::BackupFailed);
        assert_eq!(error.message, "datastore_backup_failed");
        assert_eq!(error.details["reason"], "missing_verified_backup_digest");
    }

    #[derive(Debug)]
    struct Boom;
    impl Serialize for Boom {
        fn serialize<S: serde::Serializer>(&self, _serializer: S) -> Result<S::Ok, S::Error> {
            Err(serde::ser::Error::custom("boom"))
        }
    }

    #[test]
    fn migration_serialize_and_abandon_helpers_are_covered() {
        let parse_error = serde_json::from_str::<Value>("{").expect_err("invalid json");
        let write_failed = map_serialize_candidate_error(parse_error);
        assert_eq!(write_failed.code, MigrationErrorCode::WriteFailed);
        assert_eq!(write_failed.details["reason"], "serialize_candidate");

        let boom = json_to_vec(&Boom).expect_err("custom serialize failure");
        assert_eq!(
            map_envelope_serialize_error(boom).code,
            MaintenanceErrorCode::RestoreVerifyFailed
        );
        assert!(
            json_to_vec(&Boom)
                .map_err(map_serialize_candidate_error)
                .is_err()
        );

        let candidate = temp_root("abandon-candidate").join("work");
        fs::create_dir_all(&candidate).expect("candidate");
        fs::write(candidate.join("marker.json"), b"{}").expect("marker");
        let abandoned = abandon_failed_candidate(
            &candidate,
            migration_error(
                MigrationErrorCode::VerificationFailed,
                "datastore_verification_failed",
                "candidate_verification_failed",
            ),
        );
        assert_eq!(abandoned.code, MigrationErrorCode::VerificationFailed);
        assert!(!candidate.exists());
    }

    #[test]
    fn migration_commit_rename_failures_preserve_live_store() {
        let as_of = "2026-08-05T00:00:00Z";
        let root = temp_root("mig-commit-aside");
        drop(seed_store(&root, 1, None));
        let before = fs::read(root.join("k00.json")).expect("source");
        let parent = root.parent().expect("parent");
        let suffix = restore_work_suffix(&root, as_of);
        let candidate_root = parent.join(format!(".traverse-datastore-migration-{suffix}"));
        let previous_root = parent.join(format!(".traverse-datastore-pre-v2-{suffix}"));
        let _ = fs::remove_dir_all(&candidate_root);
        let _ = fs::remove_file(&previous_root);
        let _ = fs::remove_dir_all(&previous_root);
        fs::create_dir_all(&candidate_root).expect("candidate");
        fs::write(candidate_root.join(LOCAL_DATA_STORE_LOCK_FILE), b"").expect("lock");
        fs::write(&previous_root, b"block aside").expect("occupy previous");

        let mut maintenance = LocalFileDataStoreMaintenance::open(&root).expect("maintenance");
        let failed = commit_migrated_store_root(&mut maintenance, &candidate_root, &previous_root)
            .expect_err("aside must fail");
        assert_eq!(failed.code, MigrationErrorCode::CommitFailed);
        assert_eq!(
            fs::read(root.join("k00.json")).expect("live preserved"),
            before
        );
        assert!(!candidate_root.exists());
        let _ = fs::remove_file(&previous_root);
    }

    #[test]
    fn migration_commit_install_failure_restores_previous_root() {
        let as_of = "2026-08-05T00:00:00Z";
        let root = temp_root("mig-commit-install");
        drop(seed_store(&root, 1, None));
        let before = fs::read(root.join("k00.json")).expect("source");
        let parent = root.parent().expect("parent");
        let suffix = restore_work_suffix(&root, as_of);
        let candidate_root = parent.join(format!(".traverse-datastore-migration-{suffix}"));
        let previous_root = parent.join(format!(".traverse-datastore-pre-v2-{suffix}"));
        let _ = fs::remove_dir_all(&candidate_root);
        let _ = fs::remove_dir_all(&previous_root);
        fs::create_dir_all(&candidate_root).expect("candidate");
        fs::write(candidate_root.join(LOCAL_DATA_STORE_LOCK_FILE), b"").expect("lock");

        let mut maintenance = LocalFileDataStoreMaintenance::open(&root).expect("maintenance");
        let _ = maintenance.lock_file.unlock();
        fs::rename(&root, &previous_root).expect("aside");
        fs::write(&root, b"block install").expect("occupy root");

        let failed = install_migrated_candidate(&mut maintenance, &candidate_root, &previous_root)
            .expect_err("install must fail");
        assert_eq!(failed.code, MigrationErrorCode::CommitFailed);
        assert_eq!(
            fs::read(previous_root.join("k00.json")).expect("aside preserved"),
            before
        );
        assert!(candidate_root.exists());
        let _ = fs::remove_file(&root);
        let _ = fs::rename(&previous_root, &root);
        let _ = fs::remove_dir_all(&candidate_root);
    }

    #[test]
    fn migration_reacquire_failure_rolls_back_committed_swap() {
        let as_of = "2026-08-05T00:00:00Z";
        let root = temp_root("mig-reacquire");
        drop(seed_store(&root, 1, None));
        let before = fs::read(root.join("k00.json")).expect("source");
        let parent = root.parent().expect("parent");
        let suffix = restore_work_suffix(&root, as_of);
        let candidate_root = parent.join(format!(".traverse-datastore-migration-{suffix}"));
        let previous_root = parent.join(format!(".traverse-datastore-pre-v2-{suffix}"));
        let _ = fs::remove_dir_all(&candidate_root);
        let _ = fs::remove_dir_all(&previous_root);
        fs::create_dir_all(&candidate_root).expect("candidate");
        fs::write(candidate_root.join(LOCAL_DATA_STORE_LOCK_FILE), b"").expect("candidate lock");
        fs::write(
            candidate_root.join("k00.json"),
            br#"{"format":"candidate"}"#,
        )
        .expect("cand");

        let mut maintenance = LocalFileDataStoreMaintenance::open(&root).expect("maintenance");
        let _ = maintenance.lock_file.unlock();
        fs::rename(&root, &previous_root).expect("aside");
        fs::rename(&candidate_root, &root).expect("install");

        let blocker = OpenOptions::new()
            .read(true)
            .write(true)
            .open(root.join(LOCAL_DATA_STORE_LOCK_FILE))
            .expect("blocker");
        blocker.try_lock().expect("hold lock");

        let failed = reacquire_migrated_lock(&mut maintenance, &candidate_root, &previous_root)
            .expect_err("reacquire must fail while blocked");
        assert_eq!(failed.code, MigrationErrorCode::CommitFailed);
        drop(blocker);
        assert_eq!(
            fs::read(root.join("k00.json")).expect("rolled back"),
            before
        );
        let _ = fs::remove_dir_all(&candidate_root);
        let _ = fs::remove_dir_all(&previous_root);
    }

    fn write_zip(parent: &Path, members: &[(&str, &[u8])]) -> PathBuf {
        let path = parent.join(format!("archive-{}.zip", Uuid::new_v4()));
        let file = File::create(&path).expect("create");
        let mut zip = ZipWriter::new(file);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        for (name, bytes) in members {
            zip.start_file(*name, options).expect("start");
            zip.write_all(bytes).expect("write");
        }
        zip.finish().expect("finish");
        path
    }

    fn backup_zip_with_manifest(manifest: &Value) -> PathBuf {
        write_zip_with_manifest(&temp_root("manifest-only"), manifest, &[])
    }

    fn write_zip_with_manifest(
        parent: &Path,
        manifest: &Value,
        members: &[(&str, &[u8])],
    ) -> PathBuf {
        let manifest_bytes = serde_json::to_vec_pretty(&manifest).expect("manifest");
        let path = parent.join(format!("archive-{}.zip", Uuid::new_v4()));
        let file = File::create(&path).expect("create");
        let mut zip = ZipWriter::new(file);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        zip.start_file(BACKUP_MANIFEST_MEMBER, options)
            .expect("manifest");
        zip.write_all(&manifest_bytes).expect("write manifest");
        for (name, bytes) in members {
            zip.start_file(*name, options).expect("start");
            zip.write_all(bytes).expect("write");
        }
        zip.finish().expect("finish");
        path
    }
}
