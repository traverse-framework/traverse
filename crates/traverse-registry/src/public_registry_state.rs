use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

const PUBLIC_REGISTRY_STATE_SCHEMA_VERSION: &str = "1.0.0";
const PUBLIC_REGISTRY_STATE_SCOPE: &str = "public_registry_synced";
const PUBLIC_REGISTRY_GOVERNING_SPEC: &str = "055-registry-sync";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct PublicRegistryIndex {
    pub index_version: u64,
    pub generated_at: String,
    #[serde(default)]
    pub source_commit: Option<String>,
    pub capabilities: Vec<PublicRegistryCapabilityRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct PublicRegistryCapabilityRecord {
    pub namespace: String,
    pub id: String,
    pub version: String,
    pub digest: String,
    pub artifact_url: String,
    pub contract_digest: String,
    pub contract_url: String,
    pub deprecated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct SyncedPublicRegistryState {
    pub schema_version: String,
    pub workspace_id: String,
    pub state_scope: String,
    pub source_repo: String,
    pub release_tag: String,
    pub index_version: u64,
    pub generated_at: String,
    pub source_commit: Option<String>,
    pub synced_at: String,
    pub record_count: usize,
    pub validation_status: String,
    pub governing_spec: String,
    pub capabilities: Vec<PublicRegistryCapabilityRecord>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicRegistryStateErrorCode {
    EmptyField,
    DuplicateRecord,
    MissingSyncedState,
    StateReadFailed,
    StateParseFailed,
    IncompatibleSchemaVersion,
    IncompatibleWorkspaceState,
    StateWriteFailed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicRegistryStateError {
    pub code: PublicRegistryStateErrorCode,
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicRegistryStateFailure {
    pub errors: Vec<PublicRegistryStateError>,
}

/// Validates a public registry `index.json` document before it is persisted.
///
/// # Errors
///
/// Returns [`PublicRegistryStateFailure`] when required index metadata or
/// capability record fields are empty, or when duplicate
/// namespace/id/version records are present.
pub fn validate_public_registry_index(
    index: &PublicRegistryIndex,
) -> Result<(), PublicRegistryStateFailure> {
    let mut errors = Vec::new();
    if index.generated_at.trim().is_empty() {
        errors.push(error(
            PublicRegistryStateErrorCode::EmptyField,
            "$.generated_at",
            "registry index generated_at must be non-empty",
        ));
    }

    let mut seen = BTreeSet::new();
    for (position, record) in index.capabilities.iter().enumerate() {
        validate_record_field(
            &mut errors,
            position,
            "namespace",
            &record.namespace,
            "namespace",
        );
        validate_record_field(&mut errors, position, "id", &record.id, "id");
        validate_record_field(&mut errors, position, "version", &record.version, "version");
        validate_record_field(&mut errors, position, "digest", &record.digest, "digest");
        validate_record_field(
            &mut errors,
            position,
            "artifact_url",
            &record.artifact_url,
            "artifact_url",
        );
        validate_record_field(
            &mut errors,
            position,
            "contract_digest",
            &record.contract_digest,
            "contract_digest",
        );
        validate_record_field(
            &mut errors,
            position,
            "contract_url",
            &record.contract_url,
            "contract_url",
        );
        if !seen.insert((&record.namespace, &record.id, &record.version)) {
            errors.push(error(
                PublicRegistryStateErrorCode::DuplicateRecord,
                format!("$.capabilities[{position}]"),
                format!(
                    "duplicate public registry record {}:{}@{}",
                    record.namespace, record.id, record.version
                ),
            ));
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(PublicRegistryStateFailure { errors })
    }
}

#[must_use]
pub fn synced_public_registry_state_path(workspace_root: &Path, workspace_id: &str) -> PathBuf {
    workspace_root
        .join(".traverse")
        .join("workspaces")
        .join(workspace_id)
        .join("registry")
        .join("public")
        .join("index.json")
}

/// Atomically writes validated public registry sync state for one workspace.
///
/// # Errors
///
/// Returns [`PublicRegistryStateFailure`] when the fetched index is invalid,
/// the state cannot be serialized, the target directory cannot be created, or
/// the temporary file cannot be atomically moved into place.
pub fn write_synced_public_registry_state(
    workspace_root: &Path,
    workspace_id: &str,
    source_repo: &str,
    release_tag: &str,
    synced_at: &str,
    index: PublicRegistryIndex,
) -> Result<SyncedPublicRegistryState, PublicRegistryStateFailure> {
    validate_public_registry_index(&index)?;
    let state = SyncedPublicRegistryState {
        schema_version: PUBLIC_REGISTRY_STATE_SCHEMA_VERSION.to_string(),
        workspace_id: workspace_id.to_string(),
        state_scope: PUBLIC_REGISTRY_STATE_SCOPE.to_string(),
        source_repo: source_repo.to_string(),
        release_tag: release_tag.to_string(),
        index_version: index.index_version,
        generated_at: index.generated_at,
        source_commit: index.source_commit,
        synced_at: synced_at.to_string(),
        record_count: index.capabilities.len(),
        validation_status: "passed".to_string(),
        governing_spec: PUBLIC_REGISTRY_GOVERNING_SPEC.to_string(),
        capabilities: index.capabilities,
    };
    write_state_atomically(
        &synced_public_registry_state_path(workspace_root, workspace_id),
        &state,
    )?;
    Ok(state)
}

/// Loads previously synced public registry state from local durable storage.
///
/// # Errors
///
/// Returns [`PublicRegistryStateFailure`] when no sync has run for the
/// workspace, the state cannot be read or parsed, the schema version is
/// unsupported, or the state belongs to another workspace.
pub fn load_synced_public_registry_state(
    workspace_root: &Path,
    workspace_id: &str,
) -> Result<SyncedPublicRegistryState, PublicRegistryStateFailure> {
    let path = synced_public_registry_state_path(workspace_root, workspace_id);
    if !path.exists() {
        return Err(single_error(
            PublicRegistryStateErrorCode::MissingSyncedState,
            &path,
            format!(
                "workspace {workspace_id} has no synced public registry state; run traverse-cli registry sync"
            ),
        ));
    }
    let bytes = fs::read(&path).map_err(|error| {
        single_error(
            PublicRegistryStateErrorCode::StateReadFailed,
            &path,
            format!("failed to read synced public registry state: {error}"),
        )
    })?;
    let state: SyncedPublicRegistryState = serde_json::from_slice(&bytes).map_err(|error| {
        single_error(
            PublicRegistryStateErrorCode::StateParseFailed,
            &path,
            format!("failed to parse synced public registry state: {error}"),
        )
    })?;
    validate_synced_public_registry_state(&path, workspace_id, &state)?;
    Ok(state)
}

/// Resolves an exact non-deprecated public capability record from local state.
///
/// # Errors
///
/// Returns [`PublicRegistryStateFailure`] when the workspace has no readable
/// synced public registry state. This function never fetches from the network.
pub fn resolve_synced_public_registry_record(
    workspace_root: &Path,
    workspace_id: &str,
    namespace: &str,
    id: &str,
    version: &str,
) -> Result<Option<PublicRegistryCapabilityRecord>, PublicRegistryStateFailure> {
    let state = load_synced_public_registry_state(workspace_root, workspace_id)?;
    Ok(state.capabilities.into_iter().find(|record| {
        record.namespace == namespace
            && record.id == id
            && record.version == version
            && !record.deprecated
    }))
}

fn validate_synced_public_registry_state(
    path: &Path,
    workspace_id: &str,
    state: &SyncedPublicRegistryState,
) -> Result<(), PublicRegistryStateFailure> {
    if state.schema_version != PUBLIC_REGISTRY_STATE_SCHEMA_VERSION {
        return Err(single_error(
            PublicRegistryStateErrorCode::IncompatibleSchemaVersion,
            path,
            format!(
                "synced public registry state schema {} is not supported",
                state.schema_version
            ),
        ));
    }
    if state.workspace_id != workspace_id || state.state_scope != PUBLIC_REGISTRY_STATE_SCOPE {
        return Err(single_error(
            PublicRegistryStateErrorCode::IncompatibleWorkspaceState,
            path,
            "synced public registry state does not belong to the requested workspace".to_string(),
        ));
    }
    validate_public_registry_index(&PublicRegistryIndex {
        index_version: state.index_version,
        generated_at: state.generated_at.clone(),
        source_commit: state.source_commit.clone(),
        capabilities: state.capabilities.clone(),
    })
}

fn validate_record_field(
    errors: &mut Vec<PublicRegistryStateError>,
    position: usize,
    field: &str,
    value: &str,
    label: &str,
) {
    if value.trim().is_empty() {
        errors.push(error(
            PublicRegistryStateErrorCode::EmptyField,
            format!("$.capabilities[{position}].{field}"),
            format!("registry index capability {label} must be non-empty"),
        ));
    }
}

fn write_state_atomically(
    path: &Path,
    state: &SyncedPublicRegistryState,
) -> Result<(), PublicRegistryStateFailure> {
    let parent = path.parent().ok_or_else(|| {
        single_error(
            PublicRegistryStateErrorCode::StateWriteFailed,
            path,
            "synced public registry state path must have a parent directory".to_string(),
        )
    })?;
    fs::create_dir_all(parent).map_err(|error| {
        single_error(
            PublicRegistryStateErrorCode::StateWriteFailed,
            parent,
            format!("failed to create synced public registry state directory: {error}"),
        )
    })?;

    let tmp_path = path.with_extension("json.tmp");
    let serialized = state_json_value(state).to_string();
    fs::write(&tmp_path, format!("{serialized}\n")).map_err(|error| {
        single_error(
            PublicRegistryStateErrorCode::StateWriteFailed,
            &tmp_path,
            format!("failed to write temporary synced public registry state: {error}"),
        )
    })?;
    fs::rename(&tmp_path, path).map_err(|error| {
        let _ = fs::remove_file(&tmp_path);
        single_error(
            PublicRegistryStateErrorCode::StateWriteFailed,
            path,
            format!("failed to commit synced public registry state atomically: {error}"),
        )
    })
}

fn state_json_value(state: &SyncedPublicRegistryState) -> Value {
    serde_json::json!({
        "schema_version": state.schema_version,
        "workspace_id": state.workspace_id,
        "state_scope": state.state_scope,
        "source_repo": state.source_repo,
        "release_tag": state.release_tag,
        "index_version": state.index_version,
        "generated_at": state.generated_at,
        "source_commit": state.source_commit,
        "synced_at": state.synced_at,
        "record_count": state.record_count,
        "validation_status": state.validation_status,
        "governing_spec": state.governing_spec,
        "capabilities": state
            .capabilities
            .iter()
            .map(capability_json_value)
            .collect::<Vec<_>>()
    })
}

fn capability_json_value(record: &PublicRegistryCapabilityRecord) -> Value {
    serde_json::json!({
        "namespace": record.namespace,
        "id": record.id,
        "version": record.version,
        "digest": record.digest,
        "artifact_url": record.artifact_url,
        "contract_digest": record.contract_digest,
        "contract_url": record.contract_url,
        "deprecated": record.deprecated
    })
}

fn single_error(
    code: PublicRegistryStateErrorCode,
    path: &Path,
    message: String,
) -> PublicRegistryStateFailure {
    PublicRegistryStateFailure {
        errors: vec![PublicRegistryStateError {
            code,
            path: path.display().to_string(),
            message,
        }],
    }
}

fn error(
    code: PublicRegistryStateErrorCode,
    path: impl Into<String>,
    message: impl Into<String>,
) -> PublicRegistryStateError {
    PublicRegistryStateError {
        code,
        path: path.into(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;
    use serde_json::Value;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn writes_and_loads_synced_public_registry_state() {
        let workspace_root = unique_temp_dir();

        let state = write_synced_public_registry_state(
            &workspace_root,
            "local",
            "traverse-framework/registry",
            "index-v7",
            "2026-07-06T00:00:00Z",
            valid_index(),
        )
        .expect("public registry state should write");

        let loaded = load_synced_public_registry_state(&workspace_root, "local")
            .expect("public registry state should load");
        let record = resolve_synced_public_registry_record(
            &workspace_root,
            "local",
            "traverse-starter",
            "traverse-starter.process",
            "1.0.0",
        )
        .expect("public record lookup should read local state")
        .expect("public record should resolve");

        assert_eq!(state, loaded);
        assert_eq!(loaded.state_scope, "public_registry_synced");
        assert_eq!(loaded.record_count, 1);
        assert_eq!(record.digest, "sha256:5647");
    }

    #[test]
    fn missing_synced_public_registry_state_is_actionable() {
        let workspace_root = unique_temp_dir();

        let failure = load_synced_public_registry_state(&workspace_root, "local")
            .expect_err("missing sync state should fail");

        assert_eq!(
            failure.errors[0].code,
            PublicRegistryStateErrorCode::MissingSyncedState
        );
        assert!(failure.errors[0].message.contains("registry sync"));
    }

    #[test]
    fn malformed_index_does_not_replace_existing_state() {
        let workspace_root = unique_temp_dir();
        let state_path = synced_public_registry_state_path(&workspace_root, "local");
        write_synced_public_registry_state(
            &workspace_root,
            "local",
            "traverse-framework/registry",
            "index-v7",
            "2026-07-06T00:00:00Z",
            valid_index(),
        )
        .expect("initial public registry state should write");
        let before = fs::read_to_string(&state_path).expect("state should be readable");

        let mut malformed = valid_index();
        malformed.capabilities[0].digest = " ".to_string();
        let failure = write_synced_public_registry_state(
            &workspace_root,
            "local",
            "traverse-framework/registry",
            "index-v8",
            "2026-07-06T00:01:00Z",
            malformed,
        )
        .expect_err("malformed index should fail");
        let after = fs::read_to_string(&state_path).expect("state should remain readable");

        assert_eq!(
            failure.errors[0].code,
            PublicRegistryStateErrorCode::EmptyField
        );
        assert_eq!(before, after);
    }

    #[test]
    fn incompatible_synced_public_registry_state_fails() {
        let workspace_root = unique_temp_dir();
        let state_path = synced_public_registry_state_path(&workspace_root, "local");
        fs::create_dir_all(state_path.parent().expect("state path must have parent"))
            .expect("state parent should create");
        let mut state = serde_json::to_value(
            write_synced_public_registry_state(
                &workspace_root,
                "local",
                "traverse-framework/registry",
                "index-v7",
                "2026-07-06T00:00:00Z",
                valid_index(),
            )
            .expect("state should write"),
        )
        .expect("state should serialize");
        state["schema_version"] = Value::String("9.9.9".to_string());
        fs::write(
            &state_path,
            serde_json::to_string_pretty(&state).expect("state should serialize"),
        )
        .expect("state should overwrite");

        let failure = load_synced_public_registry_state(&workspace_root, "local")
            .expect_err("wrong schema should fail");

        assert_eq!(
            failure.errors[0].code,
            PublicRegistryStateErrorCode::IncompatibleSchemaVersion
        );
    }

    #[test]
    fn invalid_index_reports_empty_metadata_and_duplicates() {
        let mut index = valid_index();
        index.generated_at = " ".to_string();
        index.capabilities.push(index.capabilities[0].clone());

        let failure =
            validate_public_registry_index(&index).expect_err("invalid index should fail");
        let codes = failure
            .errors
            .iter()
            .map(|error| error.code)
            .collect::<Vec<_>>();

        assert!(codes.contains(&PublicRegistryStateErrorCode::EmptyField));
        assert!(codes.contains(&PublicRegistryStateErrorCode::DuplicateRecord));
    }

    #[test]
    fn corrupt_synced_public_registry_state_reports_parse_failure() {
        let workspace_root = unique_temp_dir();
        let state_path = synced_public_registry_state_path(&workspace_root, "local");
        fs::create_dir_all(state_path.parent().expect("state path must have parent"))
            .expect("state parent should create");
        fs::write(&state_path, "{not json").expect("corrupt state should write");

        let failure = load_synced_public_registry_state(&workspace_root, "local")
            .expect_err("corrupt state should fail");

        assert_eq!(
            failure.errors[0].code,
            PublicRegistryStateErrorCode::StateParseFailed
        );
    }

    #[test]
    fn directory_backed_synced_public_registry_state_reports_read_failure() {
        let workspace_root = unique_temp_dir();
        let state_path = synced_public_registry_state_path(&workspace_root, "local");
        fs::create_dir_all(&state_path).expect("directory-backed state path should create");

        let failure = load_synced_public_registry_state(&workspace_root, "local")
            .expect_err("directory-backed state should fail");

        assert_eq!(
            failure.errors[0].code,
            PublicRegistryStateErrorCode::StateReadFailed
        );
    }

    #[test]
    fn workspace_mismatch_reports_incompatible_state() {
        let workspace_root = unique_temp_dir();
        write_synced_public_registry_state(
            &workspace_root,
            "local",
            "traverse-framework/registry",
            "index-v7",
            "2026-07-06T00:00:00Z",
            valid_index(),
        )
        .expect("state should write");

        let failure = load_synced_public_registry_state(&workspace_root, "other")
            .expect_err("wrong workspace should fail");

        assert_eq!(
            failure.errors[0].code,
            PublicRegistryStateErrorCode::MissingSyncedState
        );

        let state_path = synced_public_registry_state_path(&workspace_root, "local");
        let mut state: Value = serde_json::from_str(
            &fs::read_to_string(&state_path).expect("state should be readable"),
        )
        .expect("state should parse");
        state["workspace_id"] = Value::String("other".to_string());
        fs::write(
            &state_path,
            serde_json::to_string_pretty(&state).expect("state should serialize"),
        )
        .expect("state should overwrite");

        let failure = load_synced_public_registry_state(&workspace_root, "local")
            .expect_err("workspace mismatch should fail");

        assert_eq!(
            failure.errors[0].code,
            PublicRegistryStateErrorCode::IncompatibleWorkspaceState
        );
    }

    #[test]
    fn resolve_skips_deprecated_and_missing_public_records() {
        let workspace_root = unique_temp_dir();
        let mut index = valid_index();
        index.capabilities[0].deprecated = true;
        write_synced_public_registry_state(
            &workspace_root,
            "local",
            "traverse-framework/registry",
            "index-v7",
            "2026-07-06T00:00:00Z",
            index,
        )
        .expect("state should write");

        let deprecated = resolve_synced_public_registry_record(
            &workspace_root,
            "local",
            "traverse-starter",
            "traverse-starter.process",
            "1.0.0",
        )
        .expect("lookup should read local state");
        let missing = resolve_synced_public_registry_record(
            &workspace_root,
            "local",
            "traverse-starter",
            "missing",
            "1.0.0",
        )
        .expect("lookup should read local state");

        assert!(deprecated.is_none());
        assert!(missing.is_none());
    }

    #[test]
    fn write_failure_when_parent_component_is_file_leaves_no_state() {
        let workspace_root = unique_temp_dir();
        let registry_path = workspace_root.join(".traverse/workspaces/local/registry");
        fs::create_dir_all(
            registry_path
                .parent()
                .expect("registry path must have parent"),
        )
        .expect("workspace parent should create");
        fs::write(&registry_path, "not a directory")
            .expect("conflicting registry file should write");

        let failure = write_synced_public_registry_state(
            &workspace_root,
            "local",
            "traverse-framework/registry",
            "index-v7",
            "2026-07-06T00:00:00Z",
            valid_index(),
        )
        .expect_err("parent create should fail");

        assert_eq!(
            failure.errors[0].code,
            PublicRegistryStateErrorCode::StateWriteFailed
        );
    }

    #[test]
    fn write_failure_when_temp_path_is_directory_leaves_no_state() {
        let workspace_root = unique_temp_dir();
        let state_path = synced_public_registry_state_path(&workspace_root, "local");
        let tmp_path = state_path.with_extension("json.tmp");
        fs::create_dir_all(&tmp_path).expect("conflicting temp directory should create");

        let failure = write_synced_public_registry_state(
            &workspace_root,
            "local",
            "traverse-framework/registry",
            "index-v7",
            "2026-07-06T00:00:00Z",
            valid_index(),
        )
        .expect_err("temp write should fail");

        assert_eq!(
            failure.errors[0].code,
            PublicRegistryStateErrorCode::StateWriteFailed
        );
        assert!(!state_path.exists());
    }

    #[test]
    fn rename_failure_removes_temporary_state_file() {
        let workspace_root = unique_temp_dir();
        let state_path = synced_public_registry_state_path(&workspace_root, "local");
        let tmp_path = state_path.with_extension("json.tmp");
        fs::create_dir_all(&state_path).expect("conflicting final directory should create");

        let failure = write_synced_public_registry_state(
            &workspace_root,
            "local",
            "traverse-framework/registry",
            "index-v7",
            "2026-07-06T00:00:00Z",
            valid_index(),
        )
        .expect_err("rename over directory should fail");

        assert_eq!(
            failure.errors[0].code,
            PublicRegistryStateErrorCode::StateWriteFailed
        );
        assert!(!tmp_path.exists());
    }

    #[test]
    fn write_state_rejects_path_without_parent() {
        let failure = write_state_atomically(Path::new(""), &synced_state_fixture())
            .expect_err("empty state path should fail");

        assert_eq!(
            failure.errors[0].code,
            PublicRegistryStateErrorCode::StateWriteFailed
        );
    }

    fn synced_state_fixture() -> SyncedPublicRegistryState {
        SyncedPublicRegistryState {
            schema_version: PUBLIC_REGISTRY_STATE_SCHEMA_VERSION.to_string(),
            workspace_id: "local".to_string(),
            state_scope: PUBLIC_REGISTRY_STATE_SCOPE.to_string(),
            source_repo: "traverse-framework/registry".to_string(),
            release_tag: "index-v7".to_string(),
            index_version: 7,
            generated_at: "2026-07-06T00:00:00Z".to_string(),
            source_commit: Some("abc123".to_string()),
            synced_at: "2026-07-06T00:00:00Z".to_string(),
            record_count: 1,
            validation_status: "passed".to_string(),
            governing_spec: PUBLIC_REGISTRY_GOVERNING_SPEC.to_string(),
            capabilities: valid_index().capabilities,
        }
    }

    fn valid_index() -> PublicRegistryIndex {
        PublicRegistryIndex {
            index_version: 7,
            generated_at: "2026-07-06T00:00:00Z".to_string(),
            source_commit: Some("abc123".to_string()),
            capabilities: vec![PublicRegistryCapabilityRecord {
                namespace: "traverse-starter".to_string(),
                id: "traverse-starter.process".to_string(),
                version: "1.0.0".to_string(),
                digest: "sha256:5647".to_string(),
                artifact_url: "https://github.com/traverse-framework/registry/releases/download/artifacts/traverse-starter.process-1.0.0/traverse-starter.wasm".to_string(),
                contract_digest: "sha256:5647".to_string(),
                contract_url: "https://github.com/traverse-framework/registry/releases/download/artifacts/traverse-starter.process-1.0.0/contract.json".to_string(),
                deprecated: false,
            }],
        }
    }

    fn unique_temp_dir() -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time should be after epoch")
            .as_nanos();
        let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "traverse-public-registry-state-test-{nanos}-{counter}"
        ));
        fs::create_dir_all(&path).expect("temp dir should be created");
        path
    }
}
