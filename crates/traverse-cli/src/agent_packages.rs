use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};
use traverse_contracts::{
    BinaryFormat as ContractBinaryFormat, CapabilityContract, EntrypointKind, FilesystemAccess,
    HostApiAccess, NetworkAccess, ValidationContext, parse_contract, validate_contract,
};
use traverse_registry::{
    ArtifactDigests, BinaryFormat, BinaryReference, CapabilityArtifactRecord,
    CapabilityRegistration, ComposabilityMetadata, CompositionKind, CompositionPattern,
    ImplementationKind, RegistryProvenance, RegistryScope, SourceKind, SourceReference,
};
use traverse_runtime::executor::SUPPORTED_HOST_ABI_VERSION;

const AGENT_PACKAGE_KIND: &str = "agent_package";
const AGENT_PACKAGE_SCHEMA_VERSION: &str = "1.0.0";
const AGENT_GOVERNING_SPEC: &str = "017-ai-agent-packaging";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedAgentPackage {
    manifest_path: PathBuf,
    pub manifest: AgentPackageManifest,
    pub contract: CapabilityContract,
    pub source_path: PathBuf,
    pub binary_path: PathBuf,
    pub binary_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AgentPackageManifest {
    pub kind: String,
    pub schema_version: String,
    pub package_id: String,
    pub version: String,
    pub summary: String,
    pub capability_ref: AgentCapabilityReference,
    #[serde(default)]
    pub workflow_refs: Vec<AgentWorkflowReference>,
    pub source: AgentSourceReference,
    pub binary: AgentBinaryReference,
    pub constraints: AgentConstraintDeclaration,
    #[serde(default)]
    pub model_dependencies: Vec<AgentModelDependency>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AgentCapabilityReference {
    pub id: String,
    pub version: String,
    pub contract_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AgentWorkflowReference {
    pub workflow_id: String,
    pub workflow_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AgentSourceReference {
    pub path: String,
    pub language: String,
    pub entry: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AgentBinaryReference {
    pub path: String,
    pub format: String,
    pub expected_digest: String,
    pub abi_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[allow(clippy::struct_field_names)]
pub struct AgentConstraintDeclaration {
    pub host_api_access: String,
    pub network_access: String,
    pub filesystem_access: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AgentModelDependency {
    pub interface: String,
    pub purpose: String,
}

impl LoadedAgentPackage {
    #[must_use]
    pub fn render_summary(&self) -> String {
        let mut lines = vec![
            format!("path: {}", self.manifest_path.display()),
            format!("package_id: {}", self.manifest.package_id),
            format!("package_version: {}", self.manifest.version),
            format!("capability_id: {}", self.manifest.capability_ref.id),
            format!(
                "capability_version: {}",
                self.manifest.capability_ref.version
            ),
            format!("source_path: {}", self.source_path.display()),
            format!("binary_path: {}", self.binary_path.display()),
            format!("binary_digest: {}", self.binary_digest),
            format!(
                "model_interfaces: {}",
                self.manifest
                    .model_dependencies
                    .iter()
                    .map(|dependency| dependency.interface.clone())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ];

        if !self.manifest.workflow_refs.is_empty() {
            lines.push(format!(
                "workflow_refs: {}",
                self.manifest
                    .workflow_refs
                    .iter()
                    .map(|workflow| {
                        format!("{}@{}", workflow.workflow_id, workflow.workflow_version)
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }

        lines.join("\n")
    }

    #[must_use]
    pub fn capability_registration(&self) -> CapabilityRegistration {
        CapabilityRegistration {
            scope: RegistryScope::Public,
            contract: self.contract.clone(),
            contract_path: self
                .manifest_path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join(&self.manifest.capability_ref.contract_path)
                .display()
                .to_string(),
            artifact: CapabilityArtifactRecord {
                artifact_ref: format!(
                    "agent-package:{}:{}",
                    self.manifest.package_id, self.manifest.version
                ),
                implementation_kind: ImplementationKind::Executable,
                source: SourceReference {
                    kind: SourceKind::Local,
                    location: self.source_path.display().to_string(),
                },
                binary: Some(BinaryReference {
                    format: BinaryFormat::Wasm,
                    location: self.binary_path.display().to_string(),
                    signature: None,
                }),
                workflow_ref: None,
                digests: ArtifactDigests {
                    source_digest: fnv1a64(
                        &fs::read(&self.source_path).unwrap_or_else(|_| Vec::new()),
                    ),
                    binary_digest: Some(self.binary_digest.clone()),
                },
                provenance: RegistryProvenance {
                    source: provenance_source_label(&self.contract.provenance.source),
                    author: self.contract.provenance.author.clone(),
                    created_at: self.contract.provenance.created_at.clone(),
                },
            },
            registered_at: format!(
                "agent-package:{}@{}",
                self.manifest.package_id, self.manifest.version
            ),
            tags: vec![
                "ai-agent".to_string(),
                "wasm".to_string(),
                "expedition".to_string(),
            ],
            composability: ComposabilityMetadata {
                kind: CompositionKind::Atomic,
                patterns: vec![CompositionPattern::Sequential],
                provides: vec![self.manifest.capability_ref.id.clone()],
                requires: self
                    .manifest
                    .workflow_refs
                    .iter()
                    .map(|workflow| workflow.workflow_id.clone())
                    .collect(),
            },
            governing_spec: AGENT_GOVERNING_SPEC.to_string(),
            validator_version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }
}

pub fn load_agent_package(manifest_path: &Path) -> Result<LoadedAgentPackage, String> {
    let manifest_contents = fs::read_to_string(manifest_path).map_err(|error| {
        format!(
            "failed to read agent package manifest {}: {error}",
            manifest_path.display()
        )
    })?;
    let manifest =
        serde_json::from_str::<AgentPackageManifest>(&manifest_contents).map_err(|error| {
            format!(
                "failed to parse agent package manifest {}: {error}",
                manifest_path.display()
            )
        })?;

    validate_manifest_shape(&manifest, manifest_path)?;

    let manifest_dir = manifest_path.parent().ok_or_else(|| {
        format!(
            "agent package manifest {} has no parent directory",
            manifest_path.display()
        )
    })?;
    let contract_path = manifest_dir.join(&manifest.capability_ref.contract_path);
    let source_path = manifest_dir.join(&manifest.source.path);
    let binary_path = manifest_dir.join(&manifest.binary.path);

    ensure_file_exists(&contract_path, "capability contract")?;
    ensure_file_exists(&source_path, "agent source")?;
    ensure_file_exists(&binary_path, "agent binary")?;

    let contract_contents = fs::read_to_string(&contract_path).map_err(|error| {
        format!(
            "failed to read capability contract {}: {error}",
            contract_path.display()
        )
    })?;
    let parsed_contract = parse_contract(&contract_contents)
        .map_err(|failure| render_contract_failure(&contract_path, failure))?;
    let validated_contract = validate_contract(
        parsed_contract,
        &ValidationContext {
            governing_spec: AGENT_GOVERNING_SPEC,
            validator_version: env!("CARGO_PKG_VERSION"),
            existing_published: None,
        },
    )
    .map_err(|failure| render_contract_failure(&contract_path, failure))?
    .normalized;

    validate_manifest_against_contract(&manifest, &validated_contract)?;

    let binary_bytes = fs::read(&binary_path).map_err(|error| {
        format!(
            "failed to read agent binary {}: {error}",
            binary_path.display()
        )
    })?;
    let binary_digest = fnv1a64(&binary_bytes);
    if binary_digest != manifest.binary.expected_digest {
        return Err(format!(
            "agent binary digest mismatch for {}: expected {}, got {}",
            binary_path.display(),
            manifest.binary.expected_digest,
            binary_digest
        ));
    }

    Ok(LoadedAgentPackage {
        manifest_path: manifest_path.to_path_buf(),
        manifest,
        contract: validated_contract,
        source_path,
        binary_path,
        binary_digest,
    })
}

fn validate_manifest_shape(
    manifest: &AgentPackageManifest,
    manifest_path: &Path,
) -> Result<(), String> {
    if manifest.kind != AGENT_PACKAGE_KIND {
        return Err(format!(
            "agent package manifest {} must declare kind={AGENT_PACKAGE_KIND}",
            manifest_path.display()
        ));
    }
    if manifest.schema_version != AGENT_PACKAGE_SCHEMA_VERSION {
        return Err(format!(
            "agent package manifest {} must declare schema_version={AGENT_PACKAGE_SCHEMA_VERSION}",
            manifest_path.display()
        ));
    }
    if manifest.package_id.trim().is_empty() {
        return Err("agent package package_id must be non-empty".to_string());
    }
    if manifest.version.trim().is_empty() {
        return Err("agent package version must be non-empty".to_string());
    }
    if manifest.capability_ref.id.trim().is_empty()
        || manifest.capability_ref.version.trim().is_empty()
    {
        return Err(
            "agent package capability_ref must declare non-empty id and version".to_string(),
        );
    }
    if manifest.capability_ref.contract_path.trim().is_empty() {
        return Err("agent package capability_ref.contract_path must be non-empty".to_string());
    }
    if manifest.source.path.trim().is_empty() || manifest.source.entry.trim().is_empty() {
        return Err("agent package source.path and source.entry must be non-empty".to_string());
    }
    if manifest.binary.path.trim().is_empty() || manifest.binary.expected_digest.trim().is_empty() {
        return Err(
            "agent package binary.path and binary.expected_digest must be non-empty".to_string(),
        );
    }
    if manifest.binary.abi_version != SUPPORTED_HOST_ABI_VERSION {
        return Err(format!(
            "agent package binary.abi_version must equal {SUPPORTED_HOST_ABI_VERSION}"
        ));
    }
    if manifest.binary.format != "wasm" {
        return Err("agent package binary.format must equal wasm".to_string());
    }
    if manifest.workflow_refs.is_empty() {
        return Err(
            "agent package must declare at least one approved workflow reference".to_string(),
        );
    }
    Ok(())
}

fn provenance_source_label(source: &traverse_contracts::ProvenanceSource) -> String {
    match source {
        traverse_contracts::ProvenanceSource::Greenfield => "greenfield".to_string(),
        traverse_contracts::ProvenanceSource::BrownfieldExtracted => {
            "brownfield-extracted".to_string()
        }
        traverse_contracts::ProvenanceSource::AiGenerated => "ai-generated".to_string(),
        traverse_contracts::ProvenanceSource::AiAssisted => "ai-assisted".to_string(),
    }
}

fn validate_manifest_against_contract(
    manifest: &AgentPackageManifest,
    contract: &CapabilityContract,
) -> Result<(), String> {
    if contract.id != manifest.capability_ref.id
        || contract.version != manifest.capability_ref.version
    {
        return Err(format!(
            "agent package capability_ref {}@{} does not match contract {}@{}",
            manifest.capability_ref.id,
            manifest.capability_ref.version,
            contract.id,
            contract.version
        ));
    }
    if contract.execution.binary_format != ContractBinaryFormat::Wasm {
        return Err(format!(
            "agent package capability {} must declare wasm execution",
            contract.id
        ));
    }
    if contract.execution.entrypoint.kind != EntrypointKind::WasiCommand
        || contract.execution.entrypoint.command != "run"
    {
        return Err(format!(
            "agent package capability {} must declare a wasi-command entrypoint named run",
            contract.id
        ));
    }
    if contract.execution.constraints.host_api_access != HostApiAccess::None
        || manifest.constraints.host_api_access != "none"
    {
        return Err(format!(
            "agent package capability {} must keep host_api_access=none",
            contract.id
        ));
    }
    if contract.execution.constraints.network_access != NetworkAccess::Forbidden
        || manifest.constraints.network_access != "forbidden"
    {
        return Err(format!(
            "agent package capability {} must keep network_access=forbidden",
            contract.id
        ));
    }
    if contract.execution.constraints.filesystem_access != FilesystemAccess::None
        || manifest.constraints.filesystem_access != "none"
    {
        return Err(format!(
            "agent package capability {} must keep filesystem_access=none",
            contract.id
        ));
    }
    Ok(())
}

fn ensure_file_exists(path: &Path, description: &str) -> Result<(), String> {
    if path.is_file() {
        Ok(())
    } else {
        Err(format!("missing {description} file {}", path.display()))
    }
}

fn render_contract_failure(
    contract_path: &Path,
    failure: traverse_contracts::ValidationFailure,
) -> String {
    let details = failure
        .errors
        .into_iter()
        .map(|error| format!("{} at {}", error.message, error.path))
        .collect::<Vec<_>>()
        .join("; ");
    format!(
        "capability contract {} is invalid: {details}",
        contract_path.display()
    )
}

pub fn fnv1a64(bytes: &[u8]) -> String {
    let mut digest = 0xcbf2_9ce4_8422_2325_u64;
    for byte in bytes {
        digest ^= u64::from(*byte);
        digest = digest.wrapping_mul(0x0100_0000_01b3);
    }
    format!("fnv1a64:{digest:016x}")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::{
        LoadedAgentPackage, fnv1a64, load_agent_package, provenance_source_label,
        render_contract_failure,
    };
    use serde_json::{Value, json};
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    use traverse_contracts::{ErrorSeverity, ProvenanceSource, ValidationError, ValidationFailure};
    use traverse_contracts::{ValidationErrorCode, parse_contract};
    use traverse_runtime::executor::SUPPORTED_HOST_ABI_VERSION;

    const SOURCE_BYTES: &[u8] = b"fn run() {}";
    const BINARY_BYTES: &[u8] = b"agent-package-fixture-wasm-bytes";

    static TEMP_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn unique_temp_dir() -> PathBuf {
        let mut path = std::env::temp_dir();
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic")
            .as_nanos();
        let sequence = TEMP_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
        path.push(format!("traverse-agent-packages-{nonce}-{sequence}"));
        fs::create_dir_all(&path).expect("temp dir should create");
        path
    }

    fn base_contract_value() -> Value {
        json!({
            "kind": "capability_contract",
            "schema_version": "1.0.0",
            "id": "test.agent-pkg.capability",
            "namespace": "test.agent-pkg",
            "name": "capability",
            "version": "1.0.0",
            "lifecycle": "active",
            "owner": {"team": "traverse-core", "contact": "test@example.com"},
            "summary": "Deterministic test capability for agent package coverage.",
            "description": "Minimal governed capability contract used only to exercise agent package load and validation coverage paths.",
            "inputs": {"schema": {"type": "object", "properties": {}, "additionalProperties": false}},
            "outputs": {"schema": {"type": "object", "properties": {}, "additionalProperties": false}},
            "preconditions": [],
            "postconditions": [],
            "side_effects": [{"kind": "memory_only", "description": "Produces one deterministic result with no observable side effects."}],
            "emits": [],
            "consumes": [],
            "permissions": [{"id": "test.agent-pkg.capability"}],
            "execution": {
                "binary_format": "wasm",
                "entrypoint": {"kind": "wasi-command", "command": "run"},
                "preferred_targets": ["local"],
                "constraints": {"host_api_access": "none", "network_access": "forbidden", "filesystem_access": "none"}
            },
            "policies": [],
            "dependencies": [],
            "provenance": {
                "source": "greenfield",
                "author": "test-author",
                "created_at": "2026-01-01T00:00:00Z",
                "spec_ref": "017-ai-agent-packaging@1.0.0",
                "adr_refs": [],
                "exception_refs": []
            },
            "evidence": []
        })
    }

    fn base_manifest_value(expected_digest: &str) -> Value {
        json!({
            "kind": "agent_package",
            "schema_version": "1.0.0",
            "package_id": "test.agent-pkg",
            "version": "1.0.0",
            "summary": "Deterministic test agent package for coverage.",
            "capability_ref": {
                "id": "test.agent-pkg.capability",
                "version": "1.0.0",
                "contract_path": "./contract.json"
            },
            "workflow_refs": [
                {"workflow_id": "test.agent-pkg.workflow", "workflow_version": "1.0.0"}
            ],
            "source": {"path": "./src/agent.rs", "language": "rust", "entry": "run"},
            "binary": {
                "path": "./artifacts/agent.wasm",
                "format": "wasm",
                "expected_digest": expected_digest,
                "abi_version": SUPPORTED_HOST_ABI_VERSION
            },
            "constraints": {"host_api_access": "none", "network_access": "forbidden", "filesystem_access": "none"},
            "model_dependencies": [
                {"interface": "test-model-v1", "purpose": "Exercise agent package coverage paths."}
            ]
        })
    }

    /// Writes a fully valid fixture (manifest + contract + source + binary) into `dir` and
    /// returns the manifest path. Callers mutate the returned `Value`s before calling this to
    /// engineer specific failure branches.
    fn write_fixture(dir: &std::path::Path, manifest: &Value, contract: &Value) -> PathBuf {
        fs::create_dir_all(dir.join("src")).expect("src dir should create");
        fs::create_dir_all(dir.join("artifacts")).expect("artifacts dir should create");
        fs::write(dir.join("src/agent.rs"), SOURCE_BYTES).expect("source should write");
        fs::write(dir.join("artifacts/agent.wasm"), BINARY_BYTES).expect("binary should write");
        fs::write(
            dir.join("contract.json"),
            serde_json::to_string_pretty(contract).expect("contract should serialize"),
        )
        .expect("contract should write");
        let manifest_path = dir.join("manifest.json");
        fs::write(
            &manifest_path,
            serde_json::to_string_pretty(manifest).expect("manifest should serialize"),
        )
        .expect("manifest should write");
        manifest_path
    }

    fn valid_fixture(dir: &std::path::Path) -> PathBuf {
        let digest = fnv1a64(BINARY_BYTES);
        write_fixture(dir, &base_manifest_value(&digest), &base_contract_value())
    }

    #[test]
    fn fnv1a64_is_deterministic_and_distinguishes_input() {
        assert_eq!(fnv1a64(b"abc"), fnv1a64(b"abc"));
        assert_ne!(fnv1a64(b"abc"), fnv1a64(b"abd"));
        assert!(fnv1a64(b"abc").starts_with("fnv1a64:"));
    }

    #[test]
    fn provenance_source_label_maps_all_variants() {
        assert_eq!(
            provenance_source_label(&ProvenanceSource::Greenfield),
            "greenfield"
        );
        assert_eq!(
            provenance_source_label(&ProvenanceSource::BrownfieldExtracted),
            "brownfield-extracted"
        );
        assert_eq!(
            provenance_source_label(&ProvenanceSource::AiGenerated),
            "ai-generated"
        );
        assert_eq!(
            provenance_source_label(&ProvenanceSource::AiAssisted),
            "ai-assisted"
        );
    }

    #[test]
    fn render_contract_failure_joins_multiple_errors_with_path() {
        let failure = ValidationFailure {
            errors: vec![
                ValidationError {
                    code: ValidationErrorCode::MissingRequiredField,
                    message: "first problem".to_string(),
                    path: "$.a".to_string(),
                    severity: ErrorSeverity::Error,
                },
                ValidationError {
                    code: ValidationErrorCode::InvalidFormat,
                    message: "second problem".to_string(),
                    path: "$.b".to_string(),
                    severity: ErrorSeverity::Error,
                },
            ],
        };
        let rendered = render_contract_failure(std::path::Path::new("contract.json"), failure);
        assert!(rendered.contains("contract.json"));
        assert!(rendered.contains("first problem at $.a"));
        assert!(rendered.contains("second problem at $.b"));
    }

    #[test]
    fn render_summary_includes_workflow_refs_when_present() {
        let dir = unique_temp_dir();
        let manifest_path = valid_fixture(&dir);
        let loaded = load_agent_package(&manifest_path).expect("package should load");
        let summary = loaded.render_summary();
        assert!(summary.contains("package_id: test.agent-pkg"));
        assert!(summary.contains("model_interfaces: test-model-v1"));
        assert!(summary.contains("workflow_refs: test.agent-pkg.workflow@1.0.0"));
    }

    #[test]
    fn render_summary_omits_workflow_refs_section_when_empty() {
        let dir = unique_temp_dir();
        let mut manifest = base_manifest_value(&fnv1a64(BINARY_BYTES));
        manifest["workflow_refs"] = json!([]);
        // An empty workflow_refs fails validate_manifest_shape, so this test constructs the
        // loaded package directly rather than through load_agent_package.
        write_fixture(&dir, &manifest, &base_contract_value());
        let contract = parse_contract(&base_contract_value().to_string())
            .expect("contract should parse")
            .clone();
        let loaded = LoadedAgentPackage {
            manifest_path: dir.join("manifest.json"),
            manifest: serde_json::from_value(manifest).expect("manifest should deserialize"),
            contract,
            source_path: dir.join("src/agent.rs"),
            binary_path: dir.join("artifacts/agent.wasm"),
            binary_digest: fnv1a64(BINARY_BYTES),
        };
        let summary = loaded.render_summary();
        assert!(!summary.contains("workflow_refs"));
    }

    #[test]
    fn capability_registration_falls_back_to_empty_digest_when_source_unreadable() {
        let dir = unique_temp_dir();
        let contract = parse_contract(&base_contract_value().to_string())
            .expect("contract should parse")
            .clone();
        let loaded = LoadedAgentPackage {
            manifest_path: dir.join("manifest.json"),
            manifest: serde_json::from_value(base_manifest_value(&fnv1a64(BINARY_BYTES)))
                .expect("manifest should deserialize"),
            contract,
            source_path: dir.join("does-not-exist.rs"),
            binary_path: dir.join("artifacts/agent.wasm"),
            binary_digest: fnv1a64(BINARY_BYTES),
        };
        let registration = loaded.capability_registration();
        assert_eq!(registration.artifact.digests.source_digest, fnv1a64(&[]));
        assert_eq!(
            registration.artifact.digests.binary_digest,
            Some(fnv1a64(BINARY_BYTES))
        );
        assert_eq!(registration.governing_spec, "017-ai-agent-packaging");
    }

    #[test]
    fn loads_valid_agent_package_successfully() {
        let dir = unique_temp_dir();
        let manifest_path = valid_fixture(&dir);
        let loaded = load_agent_package(&manifest_path).expect("package should load");
        assert_eq!(loaded.manifest.package_id, "test.agent-pkg");
        assert_eq!(loaded.binary_digest, fnv1a64(BINARY_BYTES));
    }

    #[test]
    fn missing_manifest_file_is_reported() {
        let dir = unique_temp_dir();
        let error = load_agent_package(&dir.join("manifest.json"))
            .expect_err("load_agent_package should fail");
        assert!(error.contains("failed to read agent package manifest"));
    }

    #[test]
    fn malformed_manifest_json_is_reported() {
        let dir = unique_temp_dir();
        let manifest_path = dir.join("manifest.json");
        fs::write(&manifest_path, "not json").expect("manifest should write");
        let error = load_agent_package(&manifest_path).expect_err("load_agent_package should fail");
        assert!(error.contains("failed to parse agent package manifest"));
    }

    fn shape_rejection(mutate: impl FnOnce(&mut Value)) -> String {
        let dir = unique_temp_dir();
        let mut manifest = base_manifest_value(&fnv1a64(BINARY_BYTES));
        mutate(&mut manifest);
        let manifest_path = dir.join("manifest.json");
        fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&manifest).expect("manifest should serialize"),
        )
        .expect("manifest should write");
        load_agent_package(&manifest_path).expect_err("load_agent_package should fail")
    }

    #[test]
    fn rejects_wrong_kind() {
        let error = shape_rejection(|manifest| manifest["kind"] = json!("wrong_kind"));
        assert!(error.contains("must declare kind=agent_package"));
    }

    #[test]
    fn rejects_wrong_schema_version() {
        let error = shape_rejection(|manifest| manifest["schema_version"] = json!("9.9.9"));
        assert!(error.contains("must declare schema_version=1.0.0"));
    }

    #[test]
    fn rejects_empty_package_id() {
        let error = shape_rejection(|manifest| manifest["package_id"] = json!("  "));
        assert!(error.contains("package_id must be non-empty"));
    }

    #[test]
    fn rejects_empty_version() {
        let error = shape_rejection(|manifest| manifest["version"] = json!(""));
        assert!(error.contains("agent package version must be non-empty"));
    }

    #[test]
    fn rejects_empty_capability_ref_id() {
        let error = shape_rejection(|manifest| manifest["capability_ref"]["id"] = json!(""));
        assert!(error.contains("capability_ref must declare non-empty id and version"));
    }

    #[test]
    fn rejects_empty_capability_ref_version() {
        let error = shape_rejection(|manifest| manifest["capability_ref"]["version"] = json!(""));
        assert!(error.contains("capability_ref must declare non-empty id and version"));
    }

    #[test]
    fn rejects_empty_contract_path() {
        let error =
            shape_rejection(|manifest| manifest["capability_ref"]["contract_path"] = json!(""));
        assert!(error.contains("capability_ref.contract_path must be non-empty"));
    }

    #[test]
    fn rejects_empty_source_path() {
        let error = shape_rejection(|manifest| manifest["source"]["path"] = json!(""));
        assert!(error.contains("source.path and source.entry must be non-empty"));
    }

    #[test]
    fn rejects_empty_source_entry() {
        let error = shape_rejection(|manifest| manifest["source"]["entry"] = json!(""));
        assert!(error.contains("source.path and source.entry must be non-empty"));
    }

    #[test]
    fn rejects_empty_binary_path() {
        let error = shape_rejection(|manifest| manifest["binary"]["path"] = json!(""));
        assert!(error.contains("binary.path and binary.expected_digest must be non-empty"));
    }

    #[test]
    fn rejects_empty_binary_expected_digest() {
        let error = shape_rejection(|manifest| manifest["binary"]["expected_digest"] = json!(""));
        assert!(error.contains("binary.path and binary.expected_digest must be non-empty"));
    }

    #[test]
    fn rejects_wrong_abi_version() {
        let error = shape_rejection(|manifest| manifest["binary"]["abi_version"] = json!("0.0.1"));
        assert!(error.contains("binary.abi_version must equal"));
    }

    #[test]
    fn rejects_wrong_binary_format() {
        let error = shape_rejection(|manifest| manifest["binary"]["format"] = json!("elf"));
        assert!(error.contains("binary.format must equal wasm"));
    }

    #[test]
    fn rejects_empty_workflow_refs() {
        let error = shape_rejection(|manifest| manifest["workflow_refs"] = json!([]));
        assert!(error.contains("must declare at least one approved workflow reference"));
    }

    #[test]
    fn rejects_missing_contract_file() {
        let dir = unique_temp_dir();
        let manifest = base_manifest_value(&fnv1a64(BINARY_BYTES));
        fs::create_dir_all(dir.join("src")).expect("src dir should create");
        fs::create_dir_all(dir.join("artifacts")).expect("artifacts dir should create");
        fs::write(dir.join("src/agent.rs"), SOURCE_BYTES).expect("source should write");
        fs::write(dir.join("artifacts/agent.wasm"), BINARY_BYTES).expect("binary should write");
        let manifest_path = dir.join("manifest.json");
        fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&manifest).expect("manifest should serialize"),
        )
        .expect("manifest should write");
        let error = load_agent_package(&manifest_path).expect_err("load_agent_package should fail");
        assert!(error.contains("missing capability contract file"));
    }

    #[test]
    fn rejects_missing_source_file() {
        let dir = unique_temp_dir();
        let manifest = base_manifest_value(&fnv1a64(BINARY_BYTES));
        fs::create_dir_all(dir.join("artifacts")).expect("artifacts dir should create");
        fs::write(dir.join("artifacts/agent.wasm"), BINARY_BYTES).expect("binary should write");
        fs::write(
            dir.join("contract.json"),
            serde_json::to_string_pretty(&base_contract_value())
                .expect("contract should serialize"),
        )
        .expect("contract should write");
        let manifest_path = dir.join("manifest.json");
        fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&manifest).expect("manifest should serialize"),
        )
        .expect("manifest should write");
        let error = load_agent_package(&manifest_path).expect_err("load_agent_package should fail");
        assert!(error.contains("missing agent source file"));
    }

    #[test]
    fn rejects_missing_binary_file() {
        let dir = unique_temp_dir();
        let manifest = base_manifest_value(&fnv1a64(BINARY_BYTES));
        fs::create_dir_all(dir.join("src")).expect("src dir should create");
        fs::write(dir.join("src/agent.rs"), SOURCE_BYTES).expect("source should write");
        fs::write(
            dir.join("contract.json"),
            serde_json::to_string_pretty(&base_contract_value())
                .expect("contract should serialize"),
        )
        .expect("contract should write");
        let manifest_path = dir.join("manifest.json");
        fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&manifest).expect("manifest should serialize"),
        )
        .expect("manifest should write");
        let error = load_agent_package(&manifest_path).expect_err("load_agent_package should fail");
        assert!(error.contains("missing agent binary file"));
    }

    #[test]
    fn rejects_malformed_contract_json() {
        let dir = unique_temp_dir();
        fs::create_dir_all(dir.join("src")).expect("src dir should create");
        fs::create_dir_all(dir.join("artifacts")).expect("artifacts dir should create");
        fs::write(dir.join("src/agent.rs"), SOURCE_BYTES).expect("source should write");
        fs::write(dir.join("artifacts/agent.wasm"), BINARY_BYTES).expect("binary should write");
        fs::write(dir.join("contract.json"), "not json").expect("contract should write");
        let manifest_path = dir.join("manifest.json");
        fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&base_manifest_value(&fnv1a64(BINARY_BYTES)))
                .expect("manifest should serialize"),
        )
        .expect("manifest should write");
        let error = load_agent_package(&manifest_path).expect_err("load_agent_package should fail");
        assert!(error.contains("is invalid") || error.contains("failed to read"));
    }

    fn contract_rejection(mutate: impl FnOnce(&mut Value)) -> String {
        let dir = unique_temp_dir();
        let mut contract = base_contract_value();
        mutate(&mut contract);
        let manifest_path = write_fixture(
            &dir,
            &base_manifest_value(&fnv1a64(BINARY_BYTES)),
            &contract,
        );
        load_agent_package(&manifest_path).expect_err("load_agent_package should fail")
    }

    #[test]
    fn rejects_contract_failing_semantic_validation() {
        let error = contract_rejection(|contract| contract["version"] = json!("not-a-semver"));
        assert!(error.contains("capability contract"));
        assert!(error.contains("is invalid"));
    }

    #[test]
    fn rejects_capability_ref_mismatch_with_contract() {
        let dir = unique_temp_dir();
        let mut manifest = base_manifest_value(&fnv1a64(BINARY_BYTES));
        manifest["capability_ref"]["id"] = json!("different.capability");
        let manifest_path = write_fixture(&dir, &manifest, &base_contract_value());
        let error = load_agent_package(&manifest_path).expect_err("load_agent_package should fail");
        assert!(error.contains("does not match contract"));
    }

    #[test]
    fn rejects_entrypoint_command_mismatch() {
        let error = contract_rejection(|contract| {
            contract["execution"]["entrypoint"]["command"] = json!("start");
        });
        assert!(error.contains("must declare a wasi-command entrypoint named run"));
    }

    #[test]
    fn rejects_host_api_access_mismatch_on_contract_side() {
        let error = contract_rejection(|contract| {
            contract["execution"]["constraints"]["host_api_access"] = json!("exception_required");
            contract["provenance"]["exception_refs"] = json!(["approved-exception"]);
        });
        assert!(error.contains("must keep host_api_access=none"));
    }

    #[test]
    fn rejects_host_api_access_mismatch_on_manifest_side() {
        let dir = unique_temp_dir();
        let mut manifest = base_manifest_value(&fnv1a64(BINARY_BYTES));
        manifest["constraints"]["host_api_access"] = json!("exception_required");
        let manifest_path = write_fixture(&dir, &manifest, &base_contract_value());
        let error = load_agent_package(&manifest_path).expect_err("load_agent_package should fail");
        assert!(error.contains("must keep host_api_access=none"));
    }

    #[test]
    fn rejects_network_access_mismatch() {
        let error = contract_rejection(|contract| {
            contract["execution"]["constraints"]["network_access"] = json!("required");
        });
        assert!(error.contains("must keep network_access=forbidden"));
    }

    #[test]
    fn rejects_filesystem_access_mismatch() {
        let error = contract_rejection(|contract| {
            contract["execution"]["constraints"]["filesystem_access"] = json!("sandbox_only");
        });
        assert!(error.contains("must keep filesystem_access=none"));
    }

    #[test]
    fn rejects_binary_digest_mismatch() {
        let dir = unique_temp_dir();
        let manifest = base_manifest_value("fnv1a64:0000000000000000");
        let manifest_path = write_fixture(&dir, &manifest, &base_contract_value());
        let error = load_agent_package(&manifest_path).expect_err("load_agent_package should fail");
        assert!(error.contains("agent binary digest mismatch"));
    }

    fn make_unreadable(path: &std::path::Path) {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path)
            .expect("fixture metadata should be available")
            .permissions();
        permissions.set_mode(0o000);
        fs::set_permissions(path, permissions).expect("fixture should become unreadable");
    }

    #[test]
    fn rejects_unreadable_contract_file() {
        let dir = unique_temp_dir();
        let manifest_path = valid_fixture(&dir);
        make_unreadable(&dir.join("contract.json"));
        let error = load_agent_package(&manifest_path).expect_err("load_agent_package should fail");
        assert!(error.contains("failed to read capability contract"));
    }

    #[test]
    fn rejects_unreadable_binary_file() {
        let dir = unique_temp_dir();
        let manifest_path = valid_fixture(&dir);
        make_unreadable(&dir.join("artifacts/agent.wasm"));
        let error = load_agent_package(&manifest_path).expect_err("load_agent_package should fail");
        assert!(error.contains("failed to read agent binary"));
    }
}
