//! `traverse-cli model package-onnx` (Spec 138, Decision 105, #1591).
//!
//! Turns one ONNX file into one Spec 138 guest-ABI-v2 `model.wasm` by patching
//! a copy of the audited, prebuilt ONNX runner guest. Only three sections are
//! rewritten: the memory minimum grows to cover the model blob, the data
//! section gains one active segment holding the blob at the old memory end,
//! and the runner's `TRAVERSE_MODEL_BLOB` static (`[ptr, len]`, sentinel
//! `u32::MAX`) is patched to point at it. Every other section — including the
//! code — is copied byte for byte, so each package runs the audited runner.
//!
//! The unsigned `model.manifest.json` is written next to it; signing is a
//! separate step (`model sign`, or the fixture signing script).

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use traverse_runtime::exact_model::{
    CommercialUse, DerivationKind, MODEL_PACKAGE_SCHEMA_VERSION_PREPARED, ModelDerivation,
    ModelPackageManifest, ModelRights,
};
use wasm_encoder::{
    ConstExpr, DataCountSection, DataSection, MemorySection, MemoryType, Module, RawSection,
};
use wasmparser::{DataKind, ExternalKind, Operator, Parser, Payload, Validator};

/// Model blob magic; must match the runner guest's `BLOB_MAGIC`.
pub const BLOB_MAGIC: &[u8; 8] = b"TVONNX01";
/// Exported static the runner reads the blob location from.
pub const BLOB_EXPORT: &str = "TRAVERSE_MODEL_BLOB";
/// Executable format of runner-built packages.
pub const EXECUTABLE_FORMAT: &str = "traverse-model-wasm";
/// Runner guests speak guest ABI v3 (`model_prepare` then `model_alloc`).
pub const RUNNER_ABI_VERSION: u16 = 3;
const PAGE: u64 = 65_536;
const REQUIRED_EXPORTS: [&str; 4] = ["model_alloc", "model_prepare", "model_execute", "memory"];

/// Tensor config fixed at package time; mirrors the runner's `TensorConfig`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TensorConfig {
    pub input_name: String,
    pub output_name: String,
    pub input_shape: Vec<usize>,
    pub output_shape: Vec<usize>,
    pub input_dtype: u8,
    pub output_dtype: u8,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub symbols: BTreeMap<String, i64>,
}

/// `package.json`: the manifest fields the packager owns, plus the tensor
/// config. `wasm_digest`, `executable_format`, `abi_version`, and the
/// schema version are computed.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OnnxPackageSpec {
    pub model_id: String,
    pub version: String,
    pub registry_ref: String,
    pub input_schema_ref: String,
    pub input_schema_version: String,
    pub output_schema_ref: String,
    pub output_schema_version: String,
    pub rights: ModelRights,
    pub supported_profiles: Vec<String>,
    pub max_memory_bytes: u64,
    pub max_fuel: u64,
    pub max_input_bytes: u64,
    pub max_output_bytes: u64,
    pub max_execution_ms: u64,
    pub offline_allowed: bool,
    /// Fuel ceiling for ABI v3 `model_prepare`.
    pub max_prepare_fuel: u64,
    pub tensor: TensorConfig,
    /// Rights of the source ONNX model; recorded with its SHA-256 as
    /// `rights.derivation` (manifest schema 2.1.0, Decision 107).
    pub source: OnnxSource,
}

/// Rights of the ONNX model a runner package is converted from.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OnnxSource {
    pub license_id: String,
    pub commercial_use: CommercialUse,
    pub url: String,
}

/// JSON report printed by `model package-onnx`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PackageOnnxReport {
    pub model_id: String,
    pub version: String,
    pub runner_sha256: String,
    pub source_onnx_sha256: String,
    pub wasm_digest: String,
    pub manifest_digest: String,
    pub blob_offset: u64,
    pub blob_bytes: u64,
    pub memory_pages: u64,
    pub wasm_path: PathBuf,
    pub manifest_path: PathBuf,
}

/// Package `onnx` into `out_dir/model.wasm` + `out_dir/model.manifest.json`.
///
/// # Errors
///
/// Fails closed on unreadable inputs, an invalid spec or tensor config,
/// limits the package could never meet, or a runner that is not an
/// unpatched, import-free, ABI-v2 runner guest.
pub fn package_onnx(
    runner_path: &Path,
    onnx_path: &Path,
    spec_path: &Path,
    out_dir: &Path,
) -> Result<PackageOnnxReport, String> {
    let runner = read(runner_path)?;
    let onnx = read(onnx_path)?;
    let spec: OnnxPackageSpec = serde_json::from_slice(&read(spec_path)?)
        .map_err(|e| format!("invalid package spec {}: {e}", spec_path.display()))?;
    let package = build_package(&runner, &onnx, &spec)?;
    fs::create_dir_all(out_dir)
        .map_err(|e| format!("failed to create {}: {e}", out_dir.display()))?;
    let wasm_path = out_dir.join("model.wasm");
    let manifest_path = out_dir.join("model.manifest.json");
    write(&wasm_path, &package.wasm)?;
    write(&manifest_path, &package.manifest)?;
    Ok(PackageOnnxReport {
        model_id: spec.model_id,
        version: spec.version,
        runner_sha256: sha256_hex(&runner),
        source_onnx_sha256: sha256_hex(&onnx),
        wasm_digest: sha256_hex(&package.wasm),
        manifest_digest: sha256_hex(&package.manifest),
        blob_offset: package.layout.blob_offset,
        blob_bytes: package.layout.blob_bytes,
        memory_pages: package.layout.memory_pages,
        wasm_path,
        manifest_path,
    })
}

/// Packaged bytes plus where the blob landed.
#[derive(Debug)]
pub struct Package {
    pub wasm: Vec<u8>,
    pub manifest: Vec<u8>,
    pub layout: Layout,
}

/// Where the blob was placed in the patched module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    pub blob_offset: u64,
    pub blob_bytes: u64,
    pub memory_pages: u64,
}

/// Pure packaging: runner + ONNX + spec → (`model.wasm`, manifest bytes).
///
/// # Errors
///
/// See [`package_onnx`].
pub fn build_package(
    runner: &[u8],
    onnx: &[u8],
    spec: &OnnxPackageSpec,
) -> Result<Package, String> {
    check_tensor_config(&spec.tensor)?;
    check_frame_limits(spec)?;
    let blob = model_blob(&spec.tensor, onnx)?;
    let (wasm, layout) = patch_runner(runner, &blob)?;
    if spec.max_memory_bytes < layout.memory_pages * PAGE {
        return Err(format!(
            "max_memory_bytes {} is below the packaged initial memory {} bytes",
            spec.max_memory_bytes,
            layout.memory_pages * PAGE
        ));
    }
    let manifest = manifest_bytes(spec, &sha256_hex(&wasm), &sha256_hex(onnx))?;
    Ok(Package {
        wasm,
        manifest,
        layout,
    })
}

fn check_tensor_config(tensor: &TensorConfig) -> Result<(), String> {
    if tensor.input_name.trim().is_empty() || tensor.output_name.trim().is_empty() {
        return Err("tensor input_name and output_name are required".to_string());
    }
    for (name, shape) in [
        ("input_shape", &tensor.input_shape),
        ("output_shape", &tensor.output_shape),
    ] {
        if shape.is_empty() || shape.len() > 8 || shape.contains(&0) {
            return Err(format!("tensor {name} must have 1-8 non-zero dimensions"));
        }
    }
    Ok(())
}

/// Spec 138 frame size: `u16 version, u8 dtype, u8 rank, u32 dims[rank],
/// u32 payload_len, f32 payload[count]`.
fn frame_bytes(shape: &[usize]) -> Option<u64> {
    let count = shape
        .iter()
        .try_fold(1_u64, |acc, dim| acc.checked_mul(u64::try_from(*dim).ok()?))?;
    let header = 8 + 4 * u64::try_from(shape.len()).ok()?;
    header.checked_add(count.checked_mul(4)?)
}

fn check_frame_limits(spec: &OnnxPackageSpec) -> Result<(), String> {
    let pairs = [
        (
            "max_input_bytes",
            spec.max_input_bytes,
            &spec.tensor.input_shape,
        ),
        (
            "max_output_bytes",
            spec.max_output_bytes,
            &spec.tensor.output_shape,
        ),
    ];
    for (name, limit, shape) in pairs {
        let needed =
            frame_bytes(shape).ok_or_else(|| format!("tensor shape for {name} overflows"))?;
        if limit < needed {
            return Err(format!(
                "{name} {limit} is below the {needed}-byte tensor frame"
            ));
        }
    }
    Ok(())
}

fn model_blob(tensor: &TensorConfig, onnx: &[u8]) -> Result<Vec<u8>, String> {
    let config = serde_json::to_vec(tensor).map_err(|e| format!("tensor config: {e}"))?;
    let len = |bytes: &[u8]| {
        u32::try_from(bytes.len()).map_err(|_| "model blob exceeds 4 GiB".to_string())
    };
    let mut blob = Vec::with_capacity(16 + config.len() + onnx.len());
    blob.extend_from_slice(BLOB_MAGIC);
    blob.extend_from_slice(&len(&config)?.to_le_bytes());
    blob.extend_from_slice(&len(onnx)?.to_le_bytes());
    blob.extend_from_slice(&config);
    blob.extend_from_slice(onnx);
    Ok(blob)
}

/// What the first pass learns about the runner.
struct RunnerFacts {
    memory: wasmparser::MemoryType,
    blob_static: u64,
    exports: Vec<String>,
}

fn runner_facts(runner: &[u8]) -> Result<RunnerFacts, String> {
    let mut memories = Vec::new();
    let mut global_inits = Vec::new();
    let mut blob_global = None;
    let mut exports = Vec::new();
    for payload in Parser::new(0).parse_all(runner) {
        match payload.map_err(|e| format!("runner is not valid wasm: {e}"))? {
            Payload::ImportSection(reader) if reader.count() > 0 => {
                return Err("runner must be import-free".to_string());
            }
            Payload::MemorySection(reader) => {
                for memory in reader {
                    memories.push(memory.map_err(|e| e.to_string())?);
                }
            }
            Payload::GlobalSection(reader) => {
                for global in reader {
                    let global = global.map_err(|e| e.to_string())?;
                    let init = global
                        .init_expr
                        .get_operators_reader()
                        .read()
                        .map_err(|e| e.to_string())?;
                    global_inits.push(match init {
                        Operator::I32Const { value } => u64::try_from(value).ok(),
                        _ => None,
                    });
                }
            }
            Payload::ExportSection(reader) => {
                for export in reader {
                    let export = export.map_err(|e| e.to_string())?;
                    if export.name == BLOB_EXPORT && export.kind == ExternalKind::Global {
                        blob_global = Some(export.index);
                    }
                    exports.push(export.name.to_string());
                }
            }
            _ => {}
        }
    }
    let [memory] = memories.as_slice() else {
        return Err("runner must declare exactly one memory".to_string());
    };
    if memory.memory64 || memory.shared || memory.page_size_log2.is_some() {
        return Err("runner memory must be a plain 32-bit memory".to_string());
    }
    let blob_static = blob_global
        .and_then(|index| {
            global_inits
                .get(usize::try_from(index).ok()?)
                .copied()
                .flatten()
        })
        .ok_or_else(|| format!("runner does not export the {BLOB_EXPORT} static"))?;
    Ok(RunnerFacts {
        memory: *memory,
        blob_static,
        exports,
    })
}

fn patch_runner(runner: &[u8], blob: &[u8]) -> Result<(Vec<u8>, Layout), String> {
    let facts = runner_facts(runner)?;
    if let Some(missing) = REQUIRED_EXPORTS
        .iter()
        .find(|name| !facts.exports.iter().any(|e| e == *name))
    {
        return Err(format!(
            "runner is missing the guest ABI v3 export {missing}"
        ));
    }
    let blob_bytes = u64::try_from(blob.len()).map_err(|e| e.to_string())?;
    let blob_offset = facts.memory.initial * PAGE;
    let memory_pages = facts.memory.initial + blob_bytes.div_ceil(PAGE);
    if memory_pages > PAGE || facts.memory.maximum.is_some_and(|max| max < memory_pages) {
        return Err("model blob does not fit in the runner's 32-bit memory".to_string());
    }
    let pointer = [
        u32::try_from(blob_offset).map_err(|e| e.to_string())?,
        u32::try_from(blob_bytes).map_err(|e| e.to_string())?,
    ];
    let offset_expr = ConstExpr::i32_const(i32::try_from(blob_offset).map_err(|e| e.to_string())?);
    let mut module = Module::new();
    let mut patched_static = false;
    for payload in Parser::new(0).parse_all(runner) {
        let payload = payload.map_err(|e| e.to_string())?;
        match &payload {
            Payload::MemorySection(_) => {
                let mut section = MemorySection::new();
                section.memory(MemoryType {
                    minimum: memory_pages,
                    maximum: facts.memory.maximum,
                    memory64: false,
                    shared: false,
                    page_size_log2: None,
                });
                module.section(&section);
            }
            Payload::DataCountSection { count, .. } => {
                module.section(&DataCountSection { count: count + 1 });
            }
            Payload::DataSection(reader) => {
                let (mut section, patched) =
                    rewrite_data(reader.clone(), facts.blob_static, pointer)?;
                patched_static |= patched;
                section.active(0, &offset_expr, blob.iter().copied());
                module.section(&section);
            }
            other => {
                if let Some((id, range)) = other.as_section() {
                    // wasmparser 0.259 reports section ranges as `u64`.
                    let data = usize::try_from(range.start)
                        .ok()
                        .zip(usize::try_from(range.end).ok())
                        .and_then(|(start, end)| runner.get(start..end))
                        .ok_or_else(|| "runner section range is out of bounds".to_string())?;
                    module.section(&RawSection { id, data });
                }
            }
        }
    }
    if !patched_static {
        return Err(format!(
            "runner data does not hold the {BLOB_EXPORT} static"
        ));
    }
    let wasm = module.finish();
    Validator::new()
        .validate_all(&wasm)
        .map_err(|e| format!("patched module failed validation: {e}"))?;
    Ok((
        wasm,
        Layout {
            blob_offset,
            blob_bytes,
            memory_pages,
        },
    ))
}

/// Copy the runner's data segments, patching the blob static where found.
fn rewrite_data(
    reader: wasmparser::DataSectionReader<'_>,
    blob_static: u64,
    pointer: [u32; 2],
) -> Result<(DataSection, bool), String> {
    let mut section = DataSection::new();
    let mut patched = false;
    for data in reader {
        let data = data.map_err(|e| e.to_string())?;
        let DataKind::Active {
            memory_index: 0,
            offset_expr,
        } = data.kind
        else {
            if matches!(data.kind, DataKind::Passive) {
                section.passive(data.data.iter().copied());
                continue;
            }
            return Err("runner data segment targets an unknown memory".to_string());
        };
        let Operator::I32Const { value: start } = offset_expr
            .get_operators_reader()
            .read()
            .map_err(|e| e.to_string())?
        else {
            return Err("runner data segment offset is not i32.const".to_string());
        };
        let mut bytes = data.data.to_vec();
        patched |= patch_static(&mut bytes, start, blob_static, pointer)?;
        section.active(0, &ConstExpr::i32_const(start), bytes);
    }
    Ok((section, patched))
}

/// Patch `[ptr, len]` into `bytes` if the segment starting at `start` covers
/// the static; the static must still hold the unpatched sentinel.
fn patch_static(bytes: &mut [u8], start: i32, at: u64, pointer: [u32; 2]) -> Result<bool, String> {
    let Some(relative) = u64::try_from(start)
        .ok()
        .and_then(|start| at.checked_sub(start))
    else {
        return Ok(false);
    };
    let Some(slot) = usize::try_from(relative)
        .ok()
        .and_then(|relative| bytes.get_mut(relative..relative.checked_add(8)?))
    else {
        return Ok(false);
    };
    if slot != [0xff; 8] {
        return Err(format!(
            "runner {BLOB_EXPORT} is already patched; package from the pristine runner"
        ));
    }
    slot[..4].copy_from_slice(&pointer[0].to_le_bytes());
    slot[4..].copy_from_slice(&pointer[1].to_le_bytes());
    Ok(true)
}

fn manifest_bytes(
    spec: &OnnxPackageSpec,
    wasm_digest: &str,
    onnx_sha256: &str,
) -> Result<Vec<u8>, String> {
    let mut rights = spec.rights.clone();
    rights.derivation = Some(ModelDerivation {
        kind: DerivationKind::Converted,
        source_digest: onnx_sha256.to_string(),
        source_license_id: spec.source.license_id.clone(),
        source_commercial_use: spec.source.commercial_use,
        source_url: spec.source.url.clone(),
    });
    let manifest = ModelPackageManifest {
        schema_version: MODEL_PACKAGE_SCHEMA_VERSION_PREPARED.to_string(),
        model_id: spec.model_id.clone(),
        version: spec.version.clone(),
        wasm_digest: wasm_digest.to_string(),
        registry_ref: spec.registry_ref.clone(),
        executable_format: EXECUTABLE_FORMAT.to_string(),
        abi_version: RUNNER_ABI_VERSION,
        input_schema_ref: spec.input_schema_ref.clone(),
        input_schema_version: spec.input_schema_version.clone(),
        output_schema_ref: spec.output_schema_ref.clone(),
        output_schema_version: spec.output_schema_version.clone(),
        rights,
        supported_profiles: spec.supported_profiles.clone(),
        max_memory_bytes: spec.max_memory_bytes,
        max_fuel: spec.max_fuel,
        max_input_bytes: spec.max_input_bytes,
        max_output_bytes: spec.max_output_bytes,
        max_execution_ms: spec.max_execution_ms,
        offline_allowed: spec.offline_allowed,
        max_prepare_fuel: Some(spec.max_prepare_fuel),
    };
    manifest
        .validate()
        .map_err(|e| format!("manifest invalid: {}", e.message))?;
    let mut bytes = serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn sha256_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    Sha256::digest(bytes)
        .iter()
        .fold(String::with_capacity(64), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        })
}

fn read(path: &Path) -> Result<Vec<u8>, String> {
    fs::read(path).map_err(|e| format!("failed to read {}: {e}", path.display()))
}

fn write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    fs::write(path, bytes).map_err(|e| format!("failed to write {}: {e}", path.display()))
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

    fn root(relative: &str) -> PathBuf {
        Path::new(ROOT).join(relative)
    }

    /// Smallest module with the runner's shape: one memory, the blob static
    /// in a data segment, and the guest ABI v2 exports.
    fn fake_runner(memory: &str, data: &str, extra: &str) -> Vec<u8> {
        wat::parse_str(format!(
            r#"(module
                {extra}
                (memory (export "memory") {memory})
                (global (export "TRAVERSE_MODEL_BLOB") i32 (i32.const 1024))
                {data}
                (func (export "model_alloc") (param i32) (result i32) i32.const 0)
                (func (export "model_prepare") (result i32) i32.const 0)
                (func (export "model_execute") (param i32 i32 i32 i32) (result i32) i32.const -1))"#
        ))
        .expect("wat")
    }

    fn pristine() -> Vec<u8> {
        fake_runner(
            "1",
            r#"(data (i32.const 1020) "abcd\ff\ff\ff\ff\ff\ff\ff\ff")"#,
            "",
        )
    }

    fn spec() -> OnnxPackageSpec {
        serde_json::from_slice(
            &fs::read(root("fixtures/onnx/digits-onnx.package.json")).expect("spec"),
        )
        .expect("spec json")
    }

    fn segments(wasm: &[u8]) -> Vec<(u32, Vec<u8>)> {
        let mut out = Vec::new();
        for payload in Parser::new(0).parse_all(wasm) {
            if let Payload::DataSection(reader) = payload.expect("payload") {
                for data in reader {
                    let data = data.expect("data");
                    if let DataKind::Active { offset_expr, .. } = data.kind {
                        let Operator::I32Const { value } =
                            offset_expr.get_operators_reader().read().expect("op")
                        else {
                            panic!("offset");
                        };
                        out.push((u32::try_from(value).expect("offset"), data.data.to_vec()));
                    }
                }
            }
        }
        out
    }

    #[test]
    fn patches_the_blob_static_and_appends_the_blob_at_the_old_memory_end() {
        let package = build_package(&pristine(), b"onnx-bytes", &spec()).expect("package");
        assert_eq!(
            package.layout,
            Layout {
                blob_offset: PAGE,
                blob_bytes: package.layout.blob_bytes,
                memory_pages: 2,
            }
        );
        let segments = segments(&package.wasm);
        let mut expected_static = b"abcd".to_vec();
        expected_static.extend_from_slice(&65_536_u32.to_le_bytes());
        expected_static.extend_from_slice(
            &u32::try_from(package.layout.blob_bytes)
                .unwrap()
                .to_le_bytes(),
        );
        assert_eq!(segments[0], (1020, expected_static));
        let (offset, blob) = &segments[1];
        assert_eq!(u64::from(*offset), PAGE);
        assert_eq!(&blob[..8], BLOB_MAGIC);
        assert!(blob.ends_with(b"onnx-bytes"));
        let config_len =
            usize::try_from(u32::from_le_bytes(blob[8..12].try_into().unwrap())).unwrap();
        let config: TensorConfig =
            serde_json::from_slice(&blob[16..16 + config_len]).expect("config");
        assert_eq!(config, spec().tensor);
        // Deterministic: same inputs, same bytes.
        assert_eq!(
            package.wasm,
            build_package(&pristine(), b"onnx-bytes", &spec())
                .unwrap()
                .wasm
        );
    }

    #[test]
    fn manifest_carries_computed_fields_and_the_source_onnx_digest() {
        let package = build_package(&pristine(), b"onnx-bytes", &spec()).expect("package");
        assert_eq!(package.manifest.last(), Some(&b'\n'));
        let manifest: ModelPackageManifest =
            serde_json::from_slice(&package.manifest).expect("manifest");
        assert_eq!(manifest.wasm_digest, sha256_hex(&package.wasm));
        assert_eq!(manifest.abi_version, RUNNER_ABI_VERSION);
        assert_eq!(manifest.executable_format, EXECUTABLE_FORMAT);
        assert_eq!(
            manifest.schema_version,
            MODEL_PACKAGE_SCHEMA_VERSION_PREPARED
        );
        let derivation = manifest.rights.derivation.expect("derivation");
        assert_eq!(derivation.kind, DerivationKind::Converted);
        assert_eq!(derivation.source_digest, sha256_hex(b"onnx-bytes"));
        assert_eq!(derivation.source_license_id, "CC-BY-4.0");
        assert_eq!(derivation.source_commercial_use, CommercialUse::Allowed);
        assert!(!manifest.rights.attribution.contains("sha256:"));
    }

    #[test]
    fn a_package_more_permissive_than_its_onnx_source_is_rejected() {
        let mut spec = spec();
        spec.source.commercial_use = CommercialUse::Prohibited;
        let error = build_package(&pristine(), b"onnx-bytes", &spec).expect_err("inconsistent");
        assert!(error.contains("more permissive"), "{error}");
    }

    #[test]
    fn keeps_passive_segments_and_bumps_the_data_count() {
        let runner = fake_runner(
            "1",
            r#"(data (i32.const 1024) "\ff\ff\ff\ff\ff\ff\ff\ff") (data $p "passive")"#,
            "(func data.drop $p)",
        );
        let package = build_package(&runner, b"x", &spec()).expect("package");
        let mut count = None;
        for payload in Parser::new(0).parse_all(&package.wasm) {
            if let Payload::DataCountSection { count: value, .. } = payload.expect("payload") {
                count = Some(value);
            }
        }
        assert_eq!(count, Some(3));
    }

    #[test]
    fn rejects_runners_that_are_not_pristine_abi_v2_runner_guests() {
        let ff = r#"(data (i32.const 1024) "\ff\ff\ff\ff\ff\ff\ff\ff")"#;
        let cases = [
            (b"not wasm".to_vec(), "not valid wasm"),
            (fake_runner("1", ff, r#"(import "env" "f" (func))"#), "import-free"),
            (fake_runner("i64 1", ff, ""), "plain 32-bit memory"),
            (fake_runner("1 1", ff, ""), "does not fit"),
            (fake_runner("1", r#"(data (i32.const 1024) "\01\00\00\00\02\00\00\00")"#, ""), "already patched"),
            (fake_runner("1", r#"(data (i32.const 0) "\ff")"#, ""), "does not hold"),
            (fake_runner("1", r#"(data (i32.const 2048) "\ff\ff\ff\ff\ff\ff\ff\ff")"#, ""), "does not hold"),
            (
                wat::parse_str(r#"(module (memory (export "memory") 1) (global (export "TRAVERSE_MODEL_BLOB") i32 (i32.const 8)))"#)
                    .unwrap(),
                "missing the guest ABI v3 export",
            ),
            (
                wat::parse_str(
                    r#"(module (memory (export "memory") 1) (func (export "model_alloc") (param i32) (result i32) i32.const 0)
                        (func (export "model_execute") (param i32 i32 i32 i32) (result i32) i32.const 0))"#,
                )
                .unwrap(),
                "does not export the TRAVERSE_MODEL_BLOB static",
            ),
            (
                wat::parse_str(
                    r#"(module (global (export "TRAVERSE_MODEL_BLOB") i32 (i32.const 8))
                        (func (export "model_alloc") (param i32) (result i32) i32.const 0))"#,
                )
                .unwrap(),
                "exactly one memory",
            ),
        ];
        for (runner, expected) in cases {
            let error = build_package(&runner, b"x", &spec()).expect_err(expected);
            assert!(error.contains(expected), "{expected}: {error}");
        }
    }

    #[test]
    fn rejects_tensor_configs_and_limits_the_package_could_never_meet() {
        let mut cases: Vec<(OnnxPackageSpec, &str)> = Vec::new();
        let mut spec_with = |edit: fn(&mut OnnxPackageSpec), expected| {
            let mut value = spec();
            edit(&mut value);
            cases.push((value, expected));
        };
        spec_with(
            |s| s.tensor.input_name = " ".to_string(),
            "input_name and output_name",
        );
        spec_with(
            |s| s.tensor.output_name = String::new(),
            "input_name and output_name",
        );
        spec_with(|s| s.tensor.input_shape = vec![], "input_shape must have");
        spec_with(
            |s| s.tensor.output_shape = vec![1, 0],
            "output_shape must have",
        );
        spec_with(
            |s| s.tensor.input_shape = vec![1; 9],
            "input_shape must have",
        );
        spec_with(
            |s| s.max_input_bytes = 271,
            "max_input_bytes 271 is below the 272-byte",
        );
        spec_with(
            |s| s.max_output_bytes = 55,
            "max_output_bytes 55 is below the 56-byte",
        );
        spec_with(|s| s.tensor.input_shape = vec![usize::MAX, 2], "overflows");
        spec_with(
            |s| s.max_memory_bytes = 65_536,
            "below the packaged initial memory",
        );
        spec_with(|s| s.rights.license_id = String::new(), "manifest invalid");
        spec_with(
            |s| s.supported_profiles = vec!["gpu".to_string()],
            "manifest invalid",
        );
        for (spec, expected) in cases {
            let error = build_package(&pristine(), b"x", &spec).expect_err(expected);
            assert!(error.contains(expected), "{expected}: {error}");
        }
    }

    #[test]
    fn committed_runner_package_is_reproducible_and_keeps_the_runner_code() {
        let runner = fs::read(root("fixtures/onnx/runner.wasm")).expect("runner");
        let onnx = fs::read(root("fixtures/onnx/digits-mlp-1.0.0.onnx")).expect("onnx");
        let package = build_package(&runner, &onnx, &spec()).expect("package");
        let dir = "fixtures/models/digits-onnx-1.0.0";
        assert!(
            package.wasm == fs::read(root(&format!("{dir}/model.wasm"))).expect("wasm")
                && package.manifest
                    == fs::read(root(&format!("{dir}/model.manifest.json"))).expect("manifest"),
            "{dir} is stale: re-run `traverse-cli model package-onnx` and scripts/fixtures/sign-model-fixtures.mjs"
        );
        let sections = |wasm: &[u8]| -> Vec<(u8, Vec<u8>)> {
            Parser::new(0)
                .parse_all(wasm)
                .filter_map(|payload| {
                    let (id, range) = payload.expect("payload").as_section()?;
                    Some((
                        id,
                        wasm[usize::try_from(range.start).expect("start")
                            ..usize::try_from(range.end).expect("end")]
                            .to_vec(),
                    ))
                })
                .collect()
        };
        let (before, after) = (sections(&runner), sections(&package.wasm));
        assert_eq!(before.len(), after.len());
        for ((id, a), (_, b)) in before.iter().zip(&after) {
            assert_eq!(a == b, *id != 5 && *id != 11, "section {id}");
        }
    }

    #[test]
    fn package_onnx_writes_the_package_and_reports_digests() {
        let out =
            std::env::temp_dir().join(format!("traverse-package-onnx-{}", std::process::id()));
        let runner = out.with_extension("runner.wasm");
        let spec_path = root("fixtures/onnx/digits-onnx.package.json");
        let onnx = root("fixtures/onnx/digits-mlp-1.0.0.onnx");
        fs::write(&runner, pristine()).expect("runner");
        let report = package_onnx(&runner, &onnx, &spec_path, &out).expect("package");
        let wasm = fs::read(out.join("model.wasm")).expect("wasm");
        let manifest = fs::read(out.join("model.manifest.json")).expect("manifest");
        assert_eq!(report.wasm_digest, sha256_hex(&wasm));
        assert_eq!(report.manifest_digest, sha256_hex(&manifest));
        assert_eq!(report.runner_sha256, sha256_hex(&pristine()));
        assert_eq!(
            report.source_onnx_sha256,
            sha256_hex(&fs::read(&onnx).unwrap())
        );
        assert_eq!(
            (report.model_id.as_str(), report.version.as_str()),
            ("traverse.digits-onnx", "1.0.0")
        );

        let missing = out.join("missing");
        assert!(
            package_onnx(&missing, &onnx, &spec_path, &out)
                .unwrap_err()
                .contains("failed to read")
        );
        assert!(
            package_onnx(&runner, &onnx, &runner, &out)
                .unwrap_err()
                .contains("invalid package spec")
        );
        let bad = build_package(b"x", b"x", &spec()).unwrap_err();
        assert!(
            package_onnx(&onnx, &onnx, &spec_path, &out)
                .unwrap_err()
                .contains(&bad[..10])
        );
        // out-dir below a file cannot be created; an out-dir holding a
        // directory named model.wasm cannot be written.
        assert!(
            package_onnx(&runner, &onnx, &spec_path, &runner.join("x"))
                .unwrap_err()
                .contains("failed to create")
        );
        let blocked = out.join("blocked");
        fs::create_dir_all(blocked.join("model.wasm")).expect("dir");
        assert!(
            package_onnx(&runner, &onnx, &spec_path, &blocked)
                .unwrap_err()
                .contains("failed to write")
        );
        let _ = fs::remove_dir_all(&out);
        let _ = fs::remove_file(&runner);
    }
}
