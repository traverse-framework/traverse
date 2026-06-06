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
    if manifest.model_dependencies.is_empty() {
        return Err("agent package must declare at least one model dependency".to_string());
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
