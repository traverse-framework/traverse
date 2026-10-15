//! `traverse-cli model` packaging commands (Spec 138, Decision 105, #1589):
//! `digest`, `sign`, `verify`, `pin`, and `conformance`.
//!
//! A package is a directory holding `model.manifest.json`, `model.wasm`, and
//! `model.sig.json`. `verify` runs the runtime's own registration
//! (`ExactModelHostConnector::register_package`) so the CLI and every host
//! enforce the same rules (signature, trusted key, manifest, rights including
//! `rights.derivation`, target, host ceilings, WASM digest), plus a
//! zero-import check. `conformance` runs each vector case on both `wasmtime`
//! and `wasmi` and requires byte-identical output.

use crate::CliError;
use crate::model_packaging::COREML_CONVERT_TOOL;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use traverse_runtime::exact_model::{
    ADAPTER_COREML, AcceleratorVariant, COREML_VARIANT_FORMAT, ExactModelHostConnector,
    ExactModelPin, ExecutionPolicy, HostModelLimits, MODEL_PACKAGE_SCHEMA_VERSION_PREPARED,
    ModelEngine, ModelPackageManifest, ModelPackageSignature, ModelUsage, PLACEMENT_WASM_CPU,
    PinRights, TrustedModelKeys, VariantConversion, VariantTolerance, decode_guest_frame,
    encode_guest_frame, sign_model_manifest,
};
use traverse_runtime::host_connector_dispatch::{
    HostConnectorError, HostConnectorHostRequest, HostConnectorPort, MODEL_EXECUTE_OPERATION,
    MODEL_RUNTIME_CONNECTOR, ModelFailureReason,
};

const MANIFEST_FILE: &str = "model.manifest.json";
const WASM_FILE: &str = "model.wasm";
const SIGNATURE_FILE: &str = "model.sig.json";
const CONFORMANCE_POLICY: &str = "model-conformance";
const CONFORMANCE_CLASSIFICATION: &str = "conformance";

/// Parsed `traverse-cli model` subcommand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ModelCommand {
    Digest {
        manifest_path: PathBuf,
        json: bool,
    },
    Sign {
        manifest_path: PathBuf,
        key_path: PathBuf,
        json: bool,
    },
    Verify {
        package_dir: PathBuf,
        trusted_keys_hex: Vec<String>,
        limits: HostModelLimits,
        /// App `model_usage` to judge rights against (`None` = undeclared).
        model_usage: Option<ModelUsage>,
        /// When set with no `--model-usage`, registration fails `usage_undeclared`.
        require_model_usage: bool,
        json: bool,
    },
    Pin {
        package_dir: PathBuf,
        json: bool,
    },
    ConformanceGenerate {
        package_dir: PathBuf,
        trusted_keys_hex: Vec<String>,
        model_usage: Option<ModelUsage>,
        require_model_usage: bool,
        inputs: Vec<PathBuf>,
        out_path: PathBuf,
        json: bool,
    },
    ConformanceCheck {
        package_dir: PathBuf,
        trusted_keys_hex: Vec<String>,
        model_usage: Option<ModelUsage>,
        require_model_usage: bool,
        vector_path: PathBuf,
        json: bool,
    },
    /// Attach one `coreml` variant and drop a stale `model.sig.json`.
    AddVariant {
        package_dir: PathBuf,
        file: PathBuf,
        conformance: PathBuf,
        peak_memory_bytes: u64,
        max_abs_diff: String,
        top_k: u32,
        tool: String,
        tool_version: String,
        f32_dtypes: Vec<u8>,
        json: bool,
    },
    /// Convert an ONNX file into a canonical `mlpackage-zip` Core ML variant
    /// with the pinned converter (Decisions 112 and 113).
    ConvertCoreml {
        onnx_path: PathBuf,
        out_path: PathBuf,
        python: String,
        converter: PathBuf,
        json: bool,
    },
}

/// Parse `traverse-cli model <subcommand> ...`.
pub(crate) fn parse(args: &[String]) -> Result<ModelCommand, String> {
    let rest: Vec<&str> = args.iter().skip(2).map(String::as_str).collect();
    let json = rest.contains(&"--json");
    let rest: Vec<&str> = rest.into_iter().filter(|arg| *arg != "--json").collect();
    match rest.as_slice() {
        ["digest", manifest] => Ok(ModelCommand::Digest {
            manifest_path: PathBuf::from(manifest),
            json,
        }),
        ["sign", manifest, "--key", key] => Ok(ModelCommand::Sign {
            manifest_path: PathBuf::from(manifest),
            key_path: PathBuf::from(key),
            json,
        }),
        ["verify", package_dir, flags @ ..] => parse_verify(package_dir, flags, json),
        ["pin", package_dir] => Ok(ModelCommand::Pin {
            package_dir: PathBuf::from(package_dir),
            json,
        }),
        ["conformance", "generate", package_dir, flags @ ..] => {
            parse_conformance_generate(package_dir, flags, json)
        }
        ["conformance", "check", package_dir, vector, flags @ ..] => {
            let registration = parse_registration_flags(flags, "conformance")?;
            Ok(ModelCommand::ConformanceCheck {
                package_dir: PathBuf::from(package_dir),
                trusted_keys_hex: registration.trusted_keys_hex,
                model_usage: registration.model_usage,
                require_model_usage: registration.require_model_usage,
                vector_path: PathBuf::from(vector),
                json,
            })
        }
        ["add-variant", package_dir, flags @ ..] => parse_add_variant(package_dir, flags, json),
        ["convert-coreml", onnx, out, flags @ ..] => parse_convert_coreml(onnx, out, flags, json),
        _ => Err(help(None)),
    }
}

fn parse_u64(flag: &str, value: &str) -> Result<u64, String> {
    value
        .parse::<u64>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| format!("{flag} must be a positive integer, got '{value}'"))
}

fn parse_model_usage_value(value: &str) -> Result<ModelUsage, String> {
    match value {
        "commercial" => Ok(ModelUsage::Commercial),
        "non_commercial" => Ok(ModelUsage::NonCommercial),
        other => Err(format!(
            "--model-usage must be commercial or non_commercial (got '{other}')"
        )),
    }
}

fn model_usage_wire(usage: ModelUsage) -> &'static str {
    match usage {
        ModelUsage::Commercial => "commercial",
        ModelUsage::NonCommercial => "non_commercial",
    }
}

/// Drop a boolean flag from `flags`; returns whether it was present.
fn take_bool_flag(flags: &mut Vec<&str>, name: &str) -> bool {
    let before = flags.len();
    flags.retain(|flag| *flag != name);
    before != flags.len()
}

/// Shared `--trusted-key` / `--model-usage` / `--require-model-usage` parsing.
struct RegistrationFlags {
    trusted_keys_hex: Vec<String>,
    model_usage: Option<ModelUsage>,
    require_model_usage: bool,
}

fn parse_registration_flags(
    flags: &[&str],
    subcommand: &str,
) -> Result<RegistrationFlags, String> {
    let mut flags = flags.to_vec();
    let require_model_usage = take_bool_flag(&mut flags, "--require-model-usage");
    let mut trusted_keys_hex = Vec::new();
    let mut model_usage = None;
    for pair in flags.chunks(2) {
        match pair {
            ["--trusted-key", key] => trusted_keys_hex.push((*key).to_string()),
            ["--model-usage", value] => model_usage = Some(parse_model_usage_value(value)?),
            _ => return Err(help(Some(subcommand))),
        }
    }
    if trusted_keys_hex.is_empty() {
        return Err(format!(
            "model {subcommand} requires at least one --trusted-key <hex>"
        ));
    }
    Ok(RegistrationFlags {
        trusted_keys_hex,
        model_usage,
        require_model_usage,
    })
}

fn parse_verify(package_dir: &str, flags: &[&str], json: bool) -> Result<ModelCommand, String> {
    let mut flags = flags.to_vec();
    let require_model_usage = take_bool_flag(&mut flags, "--require-model-usage");
    let mut trusted_keys_hex = Vec::new();
    let mut limits = HostModelLimits::default();
    let mut model_usage = None;
    for pair in flags.chunks(2) {
        match pair {
            ["--trusted-key", key] => trusted_keys_hex.push((*key).to_string()),
            ["--model-usage", value] => model_usage = Some(parse_model_usage_value(value)?),
            ["--max-package-bytes", value] => {
                limits.max_package_bytes = parse_u64("--max-package-bytes", value)?;
            }
            ["--max-memory-bytes", value] => {
                limits.max_memory_bytes = parse_u64("--max-memory-bytes", value)?;
            }
            ["--max-fuel", value] => limits.max_fuel = parse_u64("--max-fuel", value)?,
            _ => return Err(help(Some("verify"))),
        }
    }
    if trusted_keys_hex.is_empty() {
        return Err("model verify requires at least one --trusted-key <hex>".to_string());
    }
    Ok(ModelCommand::Verify {
        package_dir: PathBuf::from(package_dir),
        trusted_keys_hex,
        limits,
        model_usage,
        require_model_usage,
        json,
    })
}

/// Default converter entry point, relative to the repository root.
pub(crate) const COREML_CONVERTER: &str = "tools/coreml/convert_variant.py";
/// Environment variable naming the pinned converter Python.
pub(crate) const COREML_PYTHON_ENV: &str = "TRAVERSE_COREML_PYTHON";

fn parse_convert_coreml(
    onnx: &str,
    out: &str,
    flags: &[&str],
    json: bool,
) -> Result<ModelCommand, String> {
    let mut python = std::env::var(COREML_PYTHON_ENV).unwrap_or_else(|_| "python3".to_string());
    let mut converter = PathBuf::from(COREML_CONVERTER);
    for pair in flags.chunks(2) {
        match pair {
            ["--python", value] => python = (*value).to_string(),
            ["--converter", value] => converter = PathBuf::from(value),
            _ => return Err(help(Some("convert-coreml"))),
        }
    }
    Ok(ModelCommand::ConvertCoreml {
        onnx_path: PathBuf::from(onnx),
        out_path: PathBuf::from(out),
        python,
        converter,
        json,
    })
}

fn parse_add_variant(
    package_dir: &str,
    flags: &[&str],
    json: bool,
) -> Result<ModelCommand, String> {
    let mut file = None;
    let mut conformance = None;
    let mut peak_memory_bytes = None;
    let mut max_abs_diff = None;
    let mut top_k = None;
    let mut tool = None;
    let mut tool_version = None;
    let mut f32_dtypes = Vec::new();
    for pair in flags.chunks(2) {
        match pair {
            ["--file", value] => file = Some(PathBuf::from(value)),
            ["--conformance", value] => conformance = Some(PathBuf::from(value)),
            ["--peak-memory-bytes", value] => {
                peak_memory_bytes = Some(parse_u64("--peak-memory-bytes", value)?);
            }
            ["--max-abs-diff", value] => max_abs_diff = Some((*value).to_string()),
            ["--top-k", value] => {
                top_k = Some(
                    value
                        .parse::<u32>()
                        .map_err(|_| format!("--top-k must be an integer ≥ 0, got '{value}'"))?,
                );
            }
            ["--tool", value] => tool = Some((*value).to_string()),
            ["--tool-version", value] => tool_version = Some((*value).to_string()),
            ["--f32-dtype", value] => f32_dtypes.push(value.parse::<u8>().map_err(|_| {
                format!("--f32-dtype must be an integer in 0..=255, got '{value}'")
            })?),
            _ => return Err(help(Some("add-variant"))),
        }
    }
    match (
        file,
        conformance,
        peak_memory_bytes,
        max_abs_diff,
        top_k,
        tool,
        tool_version,
    ) {
        (
            Some(file),
            Some(conformance),
            Some(peak_memory_bytes),
            Some(max_abs_diff),
            Some(top_k),
            Some(tool),
            Some(tool_version),
        ) => Ok(ModelCommand::AddVariant {
            package_dir: PathBuf::from(package_dir),
            file,
            conformance,
            peak_memory_bytes,
            max_abs_diff,
            top_k,
            tool,
            tool_version,
            f32_dtypes,
            json,
        }),
        _ => Err(help(Some("add-variant"))),
    }
}

fn parse_conformance_generate(
    package_dir: &str,
    flags: &[&str],
    json: bool,
) -> Result<ModelCommand, String> {
    let mut flags = flags.to_vec();
    let require_model_usage = take_bool_flag(&mut flags, "--require-model-usage");
    let mut inputs = Vec::new();
    let mut out_path = None;
    let mut registration_flags = Vec::new();
    for pair in flags.chunks(2) {
        match pair {
            ["--input", path] => inputs.push(PathBuf::from(path)),
            ["--out", path] => out_path = Some(PathBuf::from(path)),
            ["--trusted-key", key] => registration_flags.extend(["--trusted-key", *key]),
            ["--model-usage", value] => registration_flags.extend(["--model-usage", *value]),
            _ => return Err(help(Some("conformance"))),
        }
    }
    let registration = parse_registration_flags(&registration_flags, "conformance")?;
    match out_path {
        Some(out_path) if !inputs.is_empty() => Ok(ModelCommand::ConformanceGenerate {
            package_dir: PathBuf::from(package_dir),
            trusted_keys_hex: registration.trusted_keys_hex,
            model_usage: registration.model_usage,
            require_model_usage: require_model_usage || registration.require_model_usage,
            inputs,
            out_path,
            json,
        }),
        _ => Err(help(Some("conformance"))),
    }
}

/// Help text for `traverse-cli model [subcommand]`.
pub(crate) fn help(subcommand: Option<&str>) -> String {
    match subcommand {
        Some("digest") => "traverse-cli model digest <model.manifest.json> [--json]

  Purpose:
    Print the pin digest: the SHA-256 of the exact manifest bytes (Spec 138).

  Example:
    traverse-cli model digest out/my-model/model.manifest.json --json"
            .to_string(),
        Some("sign") => "traverse-cli model sign <model.manifest.json> --key <secret-key-file> [--json]

  Purpose:
    Write a detached Ed25519 model.sig.json next to the manifest, signing its
    exact bytes. The key file holds the 32-byte secret as 64 hex characters;
    keep it outside the repository. The CLI never creates or writes keys.

  Example:
    traverse-cli model sign out/my-model/model.manifest.json --key ~/keys/model-signing.hex"
            .to_string(),
        Some("verify") => "traverse-cli model verify <package-dir> --trusted-key <hex>... [--model-usage commercial|non_commercial] [--require-model-usage] [--max-package-bytes N] [--max-memory-bytes N] [--max-fuel N] [--json]

  Purpose:
    Verify a package exactly as a host registers it: manifest schema and
    rights (including rights.derivation), signature by a trusted key, WASM
    digest, wasm-cpu target, declared limits against the host ceilings, and
    zero WASM imports. Exits non-zero with the stable code/reason on failure.

  Usage policy:
    --model-usage sets the app model_usage rights are checked against
    (commercial|non_commercial). Without it, verification still uses a
    non_commercial default so prohibited packages verify, but the JSON
    report sets rights_record.effective_usage to null and keeps
    verification_usage explicit. --require-model-usage with no
    --model-usage reproduces usage_undeclared like other hosts. Invalid
    --model-usage values fail closed. With --json, argument/usage errors
    are a JSON error envelope on stdout (code usage_error).

  Example:
    traverse-cli model verify out/my-model --trusted-key <public-key-hex> --model-usage commercial --json"
            .to_string(),
        Some("pin") => "traverse-cli model pin <package-dir> [--json]

  Purpose:
    Print the app manifest exact_model_dependencies entry for a signed package
    (digest, rights, target, signer key_id).

  Example:
    traverse-cli model pin out/my-model --json"
            .to_string(),
        Some("conformance") => "traverse-cli model conformance generate <package-dir> --trusted-key <hex> [--model-usage commercial|non_commercial] [--require-model-usage] --input <frame.bin>... --out <vector.json> [--json]
traverse-cli model conformance check <package-dir> <vector.json> --trusted-key <hex> [--model-usage commercial|non_commercial] [--require-model-usage] [--json]

  Purpose:
    generate: run each input frame on wasmtime and wasmi, require
    byte-identical output, and write a conformance vector.
    check: re-run a vector on both engines and compare with its expected
    outputs. Third-party packages ship a vector that passes here.
    --model-usage / --require-model-usage match model verify (registration
    uses the same rights policy as other hosts).

  Example:
    traverse-cli model conformance generate out/my-model --trusted-key <public-key-hex> --input in.bin --out out/my-model/conformance.json"
            .to_string(),
        Some("add-variant") => "traverse-cli model add-variant <package-dir> --file <model.mlpackage.zip> --conformance <vector.json> --peak-memory-bytes N --max-abs-diff N --top-k N --tool <name> --tool-version <version> [--f32-dtype N]... [--json]

  Purpose:
    Copy a Core ML variant (mlpackage-zip) and its conformance vector into
    variants/coreml/, record their digests, and set
    conversion.source_digest from rights.derivation.source_digest.
    --f32-dtype names each output dtype code holding f32 values (compared
    under the tolerance); other outputs must match byte-for-byte. Removes
    model.sig.json because the manifest bytes changed; sign again before
    verify.

  Example:
    traverse-cli model add-variant out/birdnet --file birdnet.mlpackage.zip --conformance vector.json --peak-memory-bytes 180000000 --max-abs-diff 0.005 --top-k 5 --tool traverse-coreml --tool-version lock-sha256:<digest> --f32-dtype 3"
            .to_string(),
        Some("convert-coreml") => "traverse-cli model convert-coreml <model.onnx> <out.mlpackage.zip> [--python <python>] [--converter <convert_variant.py>] [--json]

  Purpose:
    Convert ONNX into a real Core ML ML Program and write its canonical,
    byte-reproducible mlpackage-zip (Decisions 112 and 113). Runs
    tools/coreml/convert_variant.py with a Python that has the pinned
    environment from tools/coreml/requirements.lock installed (--python, or
    $TRAVERSE_COREML_PYTHON, default python3). Reports the tool,
    tool_version (lock-sha256:<lockfile digest>), source_digest and digest
    to record on the variant.

  Example:
    traverse-cli model convert-coreml model.onnx out/model.mlpackage.zip --python .venv/bin/python --json"
            .to_string(),
        _ => "traverse-cli model <subcommand> [options]

  Subcommands:
    digest <model.manifest.json>                    Print the pin digest.
    sign <model.manifest.json> --key <file>         Write a detached model.sig.json.
    verify <package-dir> --trusted-key <hex>...     Verify a package as a host registers it.
    pin <package-dir>                               Print the exact_model_dependencies entry.
    conformance generate|check <package-dir> ...    Cross-engine (wasmtime + wasmi) conformance vectors.
    convert-coreml <model.onnx> <out.mlpackage.zip> Pinned ONNX → Core ML ML Program (mlpackage-zip).
    add-variant <package-dir> --file <model.mlpackage.zip> ...
                                                    Add a signed-ready coreml variant. Re-sign after.
    package-onnx <runner.wasm> <model.onnx> <package.json> <out-dir> [--json]
                                                    Package an ONNX model on the audited runner guest
                                                    (ABI v3 / schema 2.2.0).

  All subcommands accept --json. Run `traverse-cli model <subcommand> --help` for details."
            .to_string(),
    }
}

/// Run a parsed `traverse-cli model` subcommand.
pub(crate) fn run(command: &ModelCommand) -> Result<String, CliError> {
    match command {
        ModelCommand::Digest {
            manifest_path,
            json,
        } => {
            let digest = sha256_hex(&read(manifest_path)?);
            Ok(render(*json, &json!({ "digest": digest }), || {
                digest.clone()
            }))
        }
        ModelCommand::Sign {
            manifest_path,
            key_path,
            json,
        } => sign(manifest_path, key_path, *json),
        ModelCommand::Verify {
            package_dir,
            trusted_keys_hex,
            limits,
            model_usage,
            require_model_usage,
            json,
        } => verify(
            package_dir,
            trusted_keys_hex,
            *limits,
            *model_usage,
            *require_model_usage,
            *json,
        ),
        ModelCommand::Pin { package_dir, json } => {
            let pin = pin_for(package_dir)?;
            let value = to_json(&pin)?;
            Ok(render(*json, &value, || value.to_string()))
        }
        ModelCommand::ConformanceGenerate {
            package_dir,
            trusted_keys_hex,
            model_usage,
            require_model_usage,
            inputs,
            out_path,
            json,
        } => conformance_generate(
            package_dir,
            trusted_keys_hex,
            *model_usage,
            *require_model_usage,
            inputs,
            out_path,
            *json,
        ),
        ModelCommand::ConformanceCheck {
            package_dir,
            trusted_keys_hex,
            model_usage,
            require_model_usage,
            vector_path,
            json,
        } => conformance_check(
            package_dir,
            trusted_keys_hex,
            *model_usage,
            *require_model_usage,
            vector_path,
            *json,
        ),
        ModelCommand::AddVariant {
            package_dir,
            file,
            conformance,
            peak_memory_bytes,
            max_abs_diff,
            top_k,
            tool,
            tool_version,
            f32_dtypes,
            json,
        } => add_variant(&AddVariantRequest {
            package_dir,
            file,
            conformance,
            peak_memory_bytes: *peak_memory_bytes,
            max_abs_diff,
            top_k: *top_k,
            tool,
            tool_version,
            f32_dtypes,
            json: *json,
        }),
        ModelCommand::ConvertCoreml {
            onnx_path,
            out_path,
            python,
            converter,
            json,
        } => convert_coreml(onnx_path, out_path, python, converter, *json),
    }
}

fn render(json: bool, value: &Value, human: impl FnOnce() -> String) -> String {
    if json {
        serde_json::to_string_pretty(value).unwrap_or_default()
    } else {
        human()
    }
}

fn to_json<T: Serialize>(value: &T) -> Result<Value, CliError> {
    serde_json::to_value(value).map_err(|error| CliError::IoError(error.to_string()))
}

fn read(path: &Path) -> Result<Vec<u8>, CliError> {
    fs::read(path).map_err(|error| CliError::IoError(format!("{}: {error}", path.display())))
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex_encode(&Sha256::digest(bytes))
}

fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

fn hex_decode(value: &str) -> Option<Vec<u8>> {
    let value = value.trim();
    if !value.len().is_multiple_of(2) {
        return None;
    }
    (0..value.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(value.get(index..index + 2)?, 16).ok())
        .collect()
}

fn key_bytes(hex: &str, what: &str) -> Result<[u8; 32], CliError> {
    hex_decode(hex)
        .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
        .ok_or_else(|| CliError::UsageError(format!("{what} must be 64 hex characters")))
}

fn sign(manifest_path: &Path, key_path: &Path, json: bool) -> Result<String, CliError> {
    let manifest = read(manifest_path)?;
    let key_text = String::from_utf8(read(key_path)?)
        .map_err(|_| CliError::UsageError("signing key file is not UTF-8 hex".to_string()))?;
    let secret = key_bytes(&key_text, "signing key file")?;
    let signature = sign_model_manifest(&secret, &manifest);
    let out_path = manifest_path.with_file_name(SIGNATURE_FILE);
    let document = to_json(&signature)?;
    let bytes = format!(
        "{}\n",
        serde_json::to_string_pretty(&document).unwrap_or_default()
    );
    fs::write(&out_path, bytes)
        .map_err(|error| CliError::IoError(format!("{}: {error}", out_path.display())))?;
    let report = json!({
        "signature_path": out_path.display().to_string(),
        "key_id": signature.key_id,
        "digest": sha256_hex(&manifest),
    });
    Ok(render(json, &report, || {
        format!(
            "signed {} with {} -> {}",
            manifest_path.display(),
            signature.key_id,
            out_path.display()
        )
    }))
}

struct PackageFiles {
    manifest: Vec<u8>,
    wasm: Vec<u8>,
    signature: Vec<u8>,
}

fn package_files(package_dir: &Path) -> Result<PackageFiles, CliError> {
    Ok(PackageFiles {
        manifest: read(&package_dir.join(MANIFEST_FILE))?,
        wasm: read(&package_dir.join(WASM_FILE))?,
        signature: read(&package_dir.join(SIGNATURE_FILE))?,
    })
}

fn parse_manifest(bytes: &[u8]) -> Result<ModelPackageManifest, CliError> {
    serde_json::from_slice(bytes).map_err(|error| {
        CliError::ValidationFailed(format!("model manifest is malformed: {error}"))
    })
}

fn pin_for(package_dir: &Path) -> Result<ExactModelPin, CliError> {
    let files = package_files(package_dir)?;
    let manifest = parse_manifest(&files.manifest)?;
    let signature: ModelPackageSignature =
        serde_json::from_slice(&files.signature).map_err(|error| {
            CliError::ValidationFailed(format!("model.sig.json is malformed: {error}"))
        })?;
    Ok(pin_from(&files.manifest, manifest, signature.key_id))
}

/// The exact pin for these manifest bytes, signed by `key_id`.
fn pin_from(
    manifest_bytes: &[u8],
    manifest: ModelPackageManifest,
    key_id: String,
) -> ExactModelPin {
    ExactModelPin {
        model_id: manifest.model_id,
        version: manifest.version,
        digest: sha256_hex(manifest_bytes),
        offline_allowed: manifest.offline_allowed,
        target: PLACEMENT_WASM_CPU.to_string(),
        rights: PinRights {
            license_id: manifest.rights.license_id,
            commercial_use: manifest.rights.commercial_use,
        },
        key_id: Some(key_id),
    }
}

/// A host for one package.
///
/// When `model_usage` is `None` and `require_model_usage` is false, the host
/// uses a non-commercial *verification* default so `prohibited` packages still
/// verify; callers must not report that default as an app `effective_usage`.
/// When `require_model_usage` is true and usage is undeclared, registration
/// fails with `usage_undeclared` like other hosts.
fn host_for(
    pin: &ExactModelPin,
    trusted_keys_hex: &[String],
    limits: HostModelLimits,
    model_usage: Option<ModelUsage>,
    require_model_usage: bool,
) -> Result<ExactModelHostConnector, CliError> {
    let mut keys = TrustedModelKeys::new();
    for hex in trusted_keys_hex {
        keys.trust(&key_bytes(hex, "--trusted-key")?)
            .map_err(|error| CliError::UsageError(error.message))?;
    }
    Ok(host_with_keys(
        pin,
        keys,
        limits,
        model_usage,
        require_model_usage,
    ))
}

fn host_with_keys(
    pin: &ExactModelPin,
    keys: TrustedModelKeys,
    limits: HostModelLimits,
    model_usage: Option<ModelUsage>,
    require_model_usage: bool,
) -> ExactModelHostConnector {
    let mut host = ExactModelHostConnector::new(vec![pin.clone()], keys);
    host.model_usage = if require_model_usage && model_usage.is_none() {
        None
    } else {
        Some(model_usage.unwrap_or(ModelUsage::NonCommercial))
    };
    host.host_limits = limits;
    host.policies.insert(
        CONFORMANCE_POLICY.to_string(),
        ExecutionPolicy {
            policy_ref: CONFORMANCE_POLICY.to_string(),
            allowed_classifications: vec![CONFORMANCE_CLASSIFICATION.to_string()],
            max_output_bytes: u64::MAX,
            ..ExecutionPolicy::default()
        },
    );
    host
}

fn error_json(error: &HostConnectorError) -> Value {
    let mut out = json!({
        "code": error.code.as_str(),
        "reason": error.reason.map(ModelFailureReason::as_str),
        "message": error.message,
    });
    if let Some(detail) = &error.detail {
        out["detail"] = json!(detail);
    }
    out
}

/// Count WASM imports (0 for a valid import-free module). `None` when the
/// bytes are not a well-formed module header/section stream.
fn wasm_import_count(wasm: &[u8]) -> Option<u64> {
    if wasm.get(..8)? != b"\0asm\x01\0\0\0" {
        return None;
    }
    let mut at = 8;
    while at < wasm.len() {
        let id = wasm[at];
        let (size, next) = leb128_u32(wasm, at + 1)?;
        let end = next.checked_add(usize::try_from(size).ok()?)?;
        if end > wasm.len() {
            return None;
        }
        if id == 2 {
            return leb128_u32(wasm, next).map(|(count, _)| u64::from(count));
        }
        at = end;
    }
    Some(0)
}

fn leb128_u32(bytes: &[u8], mut at: usize) -> Option<(u32, usize)> {
    let mut value: u32 = 0;
    for shift in (0..35).step_by(7) {
        let byte = *bytes.get(at)?;
        at += 1;
        value |= u32::from(byte & 0x7f).checked_shl(shift)?;
        if byte & 0x80 == 0 {
            return Some((value, at));
        }
    }
    None
}

fn verify(
    package_dir: &Path,
    trusted_keys_hex: &[String],
    limits: HostModelLimits,
    model_usage: Option<ModelUsage>,
    require_model_usage: bool,
    json: bool,
) -> Result<String, CliError> {
    let files = package_files(package_dir)?;
    let pin = pin_for(package_dir)?;
    let mut host = host_for(
        &pin,
        trusted_keys_hex,
        limits,
        model_usage,
        require_model_usage,
    )?;
    let variant_files = variant_files(package_dir, &files.manifest)?;
    let variant_refs: Vec<(&str, &[u8])> = variant_files
        .iter()
        .map(|(path, bytes)| (path.as_str(), bytes.as_slice()))
        .collect();
    if let Ok(manifest) = parse_manifest(&files.manifest) {
        host.supported_adapters = manifest
            .accelerator_variants
            .unwrap_or_default()
            .into_iter()
            .map(|variant| variant.adapter)
            .collect();
    }
    let registered = host.register_package_with_variants(
        &files.manifest,
        files.wasm.clone(),
        &files.signature,
        &variant_refs,
    );
    let imports = wasm_import_count(&files.wasm);
    // Usage actually applied to the host for this run (#1660 / #1686).
    let verification_usage = match (model_usage, require_model_usage) {
        (Some(usage), _) => json!(model_usage_wire(usage)),
        (None, true) => Value::Null,
        (None, false) => json!("non_commercial"),
    };
    let mut report = json!({
        "ok": registered.is_ok() && imports == Some(0),
        "package_dir": package_dir.display().to_string(),
        "model_id": pin.model_id,
        "version": pin.version,
        "digest": pin.digest,
        "key_id": pin.key_id,
        "imports": imports,
        "verification_usage": verification_usage,
    });
    match &registered {
        Ok(_) => {
            let mut rights = to_json(&host.model_rights_record(&pin.digest))?;
            // Without --model-usage, do not claim an app effective_usage even
            // though verification used the non_commercial default (#1686).
            if model_usage.is_none() {
                if let Some(record) = rights.as_object_mut() {
                    record.insert("effective_usage".to_string(), Value::Null);
                }
            }
            report["rights_record"] = rights;
            if imports != Some(0) {
                report["error"] = json!({
                    "code": "model_incompatible",
                    "reason": "manifest_invalid",
                    "message": "model.wasm must be a well-formed module with zero imports",
                });
            }
        }
        Err(error) => report["error"] = error_json(error),
    }
    let ok = report["ok"] == json!(true);
    let text = render(json, &report, || {
        if ok {
            format!("verified {}@{} ({})", pin.model_id, pin.version, pin.digest)
        } else {
            format!(
                "verification failed for {}: {} / {}",
                package_dir.display(),
                report["error"]["code"],
                report["error"]["reason"]
            )
        }
    });
    if ok {
        Ok(text)
    } else {
        Err(CliError::ValidationFailed(text))
    }
}

/// A cross-engine conformance vector (`model conformance`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConformanceVector {
    governing_spec: String,
    pin: ExactModelPin,
    engines: Vec<String>,
    input_schema_ref: String,
    input_schema_version: String,
    max_output_bytes: u64,
    cases: Vec<ConformanceCase>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConformanceCase {
    input_frame_hex: String,
    output_frame_hex: String,
}

const ENGINES: [(&str, ModelEngine); 2] = [
    ("wasmtime", ModelEngine::Wasmtime),
    ("wasmi", ModelEngine::Wasmi),
];

/// Execute one frame on one engine through the governed signed
/// register → execute path, exactly as a host runs it.
fn execute_on(
    files: &PackageFiles,
    pin: &ExactModelPin,
    trusted_keys_hex: &[String],
    model_usage: Option<ModelUsage>,
    require_model_usage: bool,
    engine: ModelEngine,
    input: &[u8],
) -> Result<Vec<u8>, CliError> {
    let manifest = parse_manifest(&files.manifest)?;
    let host = host_for(
        pin,
        trusted_keys_hex,
        HostModelLimits::default(),
        model_usage,
        require_model_usage,
    )?;
    execute_governed(host, files, pin, &manifest, engine, input)
        .map_err(|error| CliError::ValidationFailed(error_json(&error).to_string()))
}

fn execute_governed(
    mut host: ExactModelHostConnector,
    files: &PackageFiles,
    pin: &ExactModelPin,
    manifest: &ModelPackageManifest,
    engine: ModelEngine,
    input: &[u8],
) -> Result<Vec<u8>, HostConnectorError> {
    host.engine = engine;
    host.register_package(&files.manifest, files.wasm.clone(), &files.signature)?;
    let input_ref = host.io.stage_model_input(
        input,
        usize::try_from(manifest.max_input_bytes).unwrap_or(usize::MAX),
    )?;
    let result = host.invoke(&HostConnectorHostRequest {
        connector_id: MODEL_RUNTIME_CONNECTOR.to_string(),
        operation: MODEL_EXECUTE_OPERATION.to_string(),
        binding_id: "model-conformance".to_string(),
        target_family: "native".to_string(),
        correlation_id: "model-conformance".to_string(),
        payload: json!({
            "model_ref": { "model_id": pin.model_id, "version": pin.version, "digest": pin.digest },
            "input_ref": input_ref,
            "policy_ref": CONFORMANCE_POLICY,
            "data_classification": CONFORMANCE_CLASSIFICATION,
            "input_schema_ref": manifest.input_schema_ref,
            "input_schema_version": manifest.input_schema_version,
            "max_output_bytes": manifest.max_output_bytes,
        }),
        cancel_requested: false,
    })?;
    host.io.read_model_output(
        result.artifact_ref.as_deref().unwrap_or_default(),
        usize::try_from(manifest.max_output_bytes).unwrap_or(usize::MAX),
    )
}

/// Fixed preflight-only Ed25519 secret (Decision 115). It signs an in-memory
/// copy of a freshly packaged manifest so the preflight can run it through
/// the governed register → execute path; it is public, never written to
/// disk, and trusted only by the preflight's own host.
const PREFLIGHT_SECRET: [u8; 32] = *b"traverse-package-onnx-preflight!";

/// `model package-onnx` preflight (Spec 138 0.14.0, Decision 115): run
/// `model_prepare` and one zero-valued input frame built from the tensor
/// config on every engine, under the manifest's limits and deadline, through
/// the same audited runner every host executes. The outputs must agree byte
/// for byte and carry the declared output dtype and shape.
///
/// # Errors
///
/// A JSON object naming the failing engine and the stable `code` / `reason`.
pub(crate) fn preflight_package(
    package: &crate::model_packaging::Package,
    tensor: &crate::model_packaging::TensorConfig,
) -> Result<crate::model_packaging::PreflightReport, String> {
    let fail = |engine: &str, error: Value| {
        json!({ "stage": "preflight", "engine": engine, "error": error }).to_string()
    };
    let manifest = parse_manifest(&package.manifest)
        .map_err(|error| fail("none", json!({ "message": error.to_string() })))?;
    let signature = sign_model_manifest(&PREFLIGHT_SECRET, &package.manifest);
    let pin = pin_from(
        &package.manifest,
        manifest.clone(),
        signature.key_id.clone(),
    );
    let files = PackageFiles {
        manifest: package.manifest.clone(),
        wasm: package.wasm.clone(),
        signature: serde_json::to_vec(&signature).unwrap_or_default(),
    };
    let input = zero_frame(tensor);
    let public = ed25519_dalek::SigningKey::from_bytes(&PREFLIGHT_SECRET)
        .verifying_key()
        .to_bytes();
    let mut outputs = Vec::new();
    for (name, engine) in ENGINES {
        let mut keys = TrustedModelKeys::new();
        keys.trust(&public)
            .map_err(|error| fail(name, error_json(&error)))?;
        let host = host_with_keys(
            &pin,
            keys,
            HostModelLimits::default(),
            None,
            false,
        );
        let output = execute_governed(host, &files, &pin, &manifest, engine, &input)
            .map_err(|error| fail(name, error_json(&error)))?;
        outputs.push((name, output));
    }
    let output = agreed_output(outputs).map_err(|error| {
        fail(
            "all",
            json!({ "code": "execution_failed", "reason": null, "message": error.to_string() }),
        )
    })?;
    if !output_matches(&output, tensor) {
        return Err(fail(
            "all",
            json!({
                "code": "model_incompatible",
                "reason": ModelFailureReason::OutputMismatch.as_str(),
                "message": "preflight output frame does not carry the declared output dtype and shape",
            }),
        ));
    }
    Ok(crate::model_packaging::PreflightReport {
        engines: ENGINES
            .iter()
            .map(|(name, _)| (*name).to_string())
            .collect(),
        input_frame_bytes: u64::try_from(input.len()).unwrap_or(u64::MAX),
        output_frame_sha256: sha256_hex(&output),
    })
}

/// Whether an output frame carries the tensor config's output dtype and shape.
fn output_matches(output: &[u8], tensor: &crate::model_packaging::TensorConfig) -> bool {
    let declared: Vec<u32> = tensor
        .output_shape
        .iter()
        .map(|dim| u32::try_from(*dim).unwrap_or(u32::MAX))
        .collect();
    decode_guest_frame(output)
        .is_ok_and(|(dtype, dims, _)| dtype == tensor.output_dtype && dims == declared)
}

/// A zero-valued `f32` input frame matching the tensor config.
fn zero_frame(tensor: &crate::model_packaging::TensorConfig) -> Vec<u8> {
    let dims: Vec<u32> = tensor
        .input_shape
        .iter()
        .map(|dim| u32::try_from(*dim).unwrap_or(u32::MAX))
        .collect();
    let count: usize = tensor.input_shape.iter().product();
    encode_guest_frame(tensor.input_dtype, &dims, &vec![0; count.saturating_mul(4)])
}

/// Run a frame on every engine and require byte-identical output.
fn cross_engine(
    files: &PackageFiles,
    pin: &ExactModelPin,
    trusted_keys_hex: &[String],
    model_usage: Option<ModelUsage>,
    require_model_usage: bool,
    input: &[u8],
) -> Result<Vec<u8>, CliError> {
    let mut outputs = Vec::new();
    for (name, engine) in ENGINES {
        outputs.push((
            name,
            execute_on(
                files,
                pin,
                trusted_keys_hex,
                model_usage,
                require_model_usage,
                engine,
                input,
            )?,
        ));
    }
    agreed_output(outputs)
}

/// The shared output when every engine agrees byte for byte.
fn agreed_output(outputs: Vec<(&str, Vec<u8>)>) -> Result<Vec<u8>, CliError> {
    let mut outputs = outputs.into_iter();
    let (first_name, first) = outputs
        .next()
        .ok_or_else(|| CliError::ValidationFailed("no engine ran".to_string()))?;
    match outputs.find(|(_, output)| *output != first) {
        Some((name, _)) => Err(CliError::ValidationFailed(format!(
            "engine {name} output differs from {first_name}"
        ))),
        None => Ok(first),
    }
}

fn conformance_generate(
    package_dir: &Path,
    trusted_keys_hex: &[String],
    model_usage: Option<ModelUsage>,
    require_model_usage: bool,
    inputs: &[PathBuf],
    out_path: &Path,
    json: bool,
) -> Result<String, CliError> {
    let files = package_files(package_dir)?;
    let pin = pin_for(package_dir)?;
    let manifest = parse_manifest(&files.manifest)?;
    let mut cases = Vec::new();
    for input_path in inputs {
        let input = read(input_path)?;
        let output = cross_engine(
            &files,
            &pin,
            trusted_keys_hex,
            model_usage,
            require_model_usage,
            &input,
        )?;
        cases.push(ConformanceCase {
            input_frame_hex: hex_encode(&input),
            output_frame_hex: hex_encode(&output),
        });
    }
    let vector = ConformanceVector {
        governing_spec: "138-governed-exact-model-execution".to_string(),
        pin,
        engines: ENGINES
            .iter()
            .map(|(name, _)| (*name).to_string())
            .collect(),
        input_schema_ref: manifest.input_schema_ref,
        input_schema_version: manifest.input_schema_version,
        max_output_bytes: manifest.max_output_bytes,
        cases,
    };
    let document = to_json(&vector)?;
    fs::write(
        out_path,
        format!(
            "{}\n",
            serde_json::to_string_pretty(&document).unwrap_or_default()
        ),
    )
    .map_err(|error| CliError::IoError(format!("{}: {error}", out_path.display())))?;
    let report = json!({
        "vector_path": out_path.display().to_string(),
        "engines": vector.engines,
        "cases": vector.cases.len(),
    });
    Ok(render(json, &report, || {
        format!(
            "wrote {} ({} cases, identical on wasmtime and wasmi)",
            out_path.display(),
            vector.cases.len()
        )
    }))
}

fn conformance_check(
    package_dir: &Path,
    trusted_keys_hex: &[String],
    model_usage: Option<ModelUsage>,
    require_model_usage: bool,
    vector_path: &Path,
    json: bool,
) -> Result<String, CliError> {
    let files = package_files(package_dir)?;
    let pin = pin_for(package_dir)?;
    let vector: ConformanceVector =
        serde_json::from_slice(&read(vector_path)?).map_err(|error| {
            CliError::ValidationFailed(format!("conformance vector is malformed: {error}"))
        })?;
    if vector.pin.digest != pin.digest {
        return Err(CliError::ValidationFailed(
            "conformance vector pins a different package digest".to_string(),
        ));
    }
    let mut failures = Vec::new();
    for (index, case) in vector.cases.iter().enumerate() {
        let input = hex_decode(&case.input_frame_hex).ok_or_else(|| {
            CliError::ValidationFailed(format!("case {index}: input_frame_hex is not hex"))
        })?;
        let output = cross_engine(
            &files,
            &pin,
            trusted_keys_hex,
            model_usage,
            require_model_usage,
            &input,
        )?;
        if hex_encode(&output) != case.output_frame_hex {
            failures.push(index);
        }
    }
    let report = json!({
        "ok": failures.is_empty(),
        "vector_path": vector_path.display().to_string(),
        "engines": ENGINES.iter().map(|(name, _)| *name).collect::<Vec<_>>(),
        "cases": vector.cases.len(),
        "failed_cases": failures,
    });
    let text = render(json, &report, || {
        format!(
            "{} of {} cases match on wasmtime and wasmi",
            vector.cases.len() - failures.len(),
            vector.cases.len()
        )
    });
    if failures.is_empty() {
        Ok(text)
    } else {
        Err(CliError::ValidationFailed(text))
    }
}

fn variant_files(
    package_dir: &Path,
    manifest_bytes: &[u8],
) -> Result<Vec<(String, Vec<u8>)>, CliError> {
    let Ok(manifest) = parse_manifest(manifest_bytes) else {
        return Ok(Vec::new());
    };
    let Some(variants) = manifest.accelerator_variants else {
        return Ok(Vec::new());
    };
    let mut files = Vec::new();
    for variant in variants {
        for rel in [variant.path, variant.conformance_vector_path] {
            let Some(path) = package_child(package_dir, &rel) else {
                continue;
            };
            match fs::read(&path) {
                Ok(bytes) => files.push((rel, bytes)),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(CliError::IoError(format!("{}: {error}", path.display())));
                }
            }
        }
    }
    Ok(files)
}

fn package_child(root: &Path, rel: &str) -> Option<PathBuf> {
    if Path::new(rel).is_absolute()
        || rel
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return None;
    }
    Some(root.join(rel))
}

struct AddVariantRequest<'a> {
    package_dir: &'a Path,
    file: &'a Path,
    conformance: &'a Path,
    peak_memory_bytes: u64,
    max_abs_diff: &'a str,
    top_k: u32,
    tool: &'a str,
    tool_version: &'a str,
    f32_dtypes: &'a [u8],
    json: bool,
}

fn add_variant(request: &AddVariantRequest<'_>) -> Result<String, CliError> {
    let &AddVariantRequest {
        package_dir,
        file,
        conformance,
        peak_memory_bytes,
        max_abs_diff,
        top_k,
        tool,
        tool_version,
        f32_dtypes,
        json,
    } = request;
    let manifest_path = package_dir.join(MANIFEST_FILE);
    let mut manifest = parse_manifest(&read(&manifest_path)?)?;
    let derivation = manifest.rights.derivation.clone().ok_or_else(|| {
        CliError::ValidationFailed("accelerator variants require rights.derivation".to_string())
    })?;
    let max_abs_diff = max_abs_diff
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
        .ok_or_else(|| {
            CliError::ValidationFailed(format!(
                "--max-abs-diff must be a number in [0, 0.005], got '{max_abs_diff}'"
            ))
        })?;
    let model_bytes = read(file)?;
    let vector_bytes = read(conformance)?;
    let model_rel = format!("variants/{ADAPTER_COREML}/model.mlpackage.zip");
    let vector_rel = format!("variants/{ADAPTER_COREML}/conformance.json");
    let model_path = package_dir.join(&model_rel);
    let vector_path = package_dir.join(&vector_rel);
    if let Some(parent) = model_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| CliError::IoError(format!("{}: {error}", parent.display())))?;
    }
    fs::write(&model_path, &model_bytes)
        .map_err(|error| CliError::IoError(format!("{}: {error}", model_path.display())))?;
    fs::write(&vector_path, &vector_bytes)
        .map_err(|error| CliError::IoError(format!("{}: {error}", vector_path.display())))?;
    let mut variants = manifest.accelerator_variants.take().unwrap_or_default();
    variants.retain(|existing| existing.adapter != ADAPTER_COREML);
    variants.push(AcceleratorVariant {
        adapter: ADAPTER_COREML.to_string(),
        format: COREML_VARIANT_FORMAT.to_string(),
        path: model_rel,
        digest: sha256_hex(&model_bytes),
        conversion: VariantConversion {
            tool: tool.to_string(),
            tool_version: tool_version.to_string(),
            source_digest: derivation.source_digest,
        },
        conformance_vector_path: vector_rel,
        conformance_vector_digest: sha256_hex(&vector_bytes),
        tolerance: VariantTolerance {
            max_abs_diff,
            top_k,
            f32_dtypes: f32_dtypes.to_vec(),
        },
        peak_memory_bytes,
    });
    manifest.accelerator_variants = Some(variants);
    manifest.schema_version = MODEL_PACKAGE_SCHEMA_VERSION_PREPARED.to_string();
    manifest
        .validate()
        .map_err(|error| CliError::ValidationFailed(error.message))?;
    let mut bytes = serde_json::to_vec_pretty(&manifest)
        .map_err(|error| CliError::IoError(error.to_string()))?;
    bytes.push(b'\n');
    fs::write(&manifest_path, &bytes)
        .map_err(|error| CliError::IoError(format!("{}: {error}", manifest_path.display())))?;
    let signature = package_dir.join(SIGNATURE_FILE);
    let signature_removed = signature.exists();
    if signature_removed {
        fs::remove_file(&signature)
            .map_err(|error| CliError::IoError(format!("{}: {error}", signature.display())))?;
    }
    let report = json!({
        "model_id": manifest.model_id,
        "adapter": ADAPTER_COREML,
        "schema_version": manifest.schema_version,
        "signature_removed": signature_removed,
    });
    Ok(render(json, &report, || {
        format!(
            "added {ADAPTER_COREML} variant to {}; sign the manifest before verify",
            package_dir.display()
        )
    }))
}

fn convert_coreml(
    onnx_path: &Path,
    out_path: &Path,
    python: &str,
    converter: &Path,
    json: bool,
) -> Result<String, CliError> {
    let onnx = read(onnx_path)?;
    if let Some(parent) = out_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .map_err(|error| CliError::IoError(format!("{}: {error}", parent.display())))?;
    }
    let output = std::process::Command::new(python)
        .arg(converter)
        .arg(onnx_path)
        .arg(out_path)
        .output()
        .map_err(|error| CliError::IoError(format!("{python}: {error}")))?;
    if !output.status.success() {
        return Err(CliError::ValidationFailed(format!(
            "Core ML converter failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let report: Value = stdout
        .lines()
        .last()
        .and_then(|line| serde_json::from_str(line).ok())
        .ok_or_else(|| {
            CliError::ValidationFailed("Core ML converter printed no JSON report".to_string())
        })?;
    let digest = sha256_hex(&read(out_path)?);
    let field = |name: &str| report[name].as_str().unwrap_or_default().to_string();
    if field("source_digest") != sha256_hex(&onnx) || field("digest") != digest {
        return Err(CliError::ValidationFailed(
            "Core ML converter report does not match the ONNX input or the written variant"
                .to_string(),
        ));
    }
    let lock_digest = field("lock_digest");
    if lock_digest.len() != 64 || !lock_digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(CliError::ValidationFailed(
            "Core ML converter report has no requirements.lock digest".to_string(),
        ));
    }
    let summary = json!({
        "tool": COREML_CONVERT_TOOL,
        "tool_version": format!("lock-sha256:{lock_digest}"),
        "source_digest": sha256_hex(&onnx),
        "digest": digest,
        "path": out_path.display().to_string(),
        "rewrites": report["rewrites"],
    });
    Ok(render(json, &summary, || {
        format!("wrote {}", out_path.display())
    }))
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_used,
    clippy::too_many_lines
)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

    fn repo(path: &str) -> PathBuf {
        PathBuf::from(format!("{ROOT}/{path}"))
    }

    fn temp_dir() -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "traverse-cli-model-test-{nanos}-{}",
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).expect("temp dir");
        path
    }

    fn test_key(field: &str) -> String {
        let key: Value = serde_json::from_slice(
            &fs::read(repo("fixtures/models/test-signing-key.json")).expect("key"),
        )
        .expect("key json");
        key[field].as_str().expect("hex").to_string()
    }

    fn cli(args: &[&str]) -> Result<String, CliError> {
        let args: Vec<String> = ["traverse-cli", "model"]
            .iter()
            .chain(args)
            .map(ToString::to_string)
            .collect();
        run(&parse(&args).expect("parse"))
    }

    fn json_out(result: Result<String, CliError>) -> Value {
        let text = match result {
            Ok(text) | Err(CliError::ValidationFailed(text)) => text,
            Err(other) => panic!("unexpected error {other}"),
        };
        serde_json::from_str(&text).expect("json output")
    }

    fn path(value: &Path) -> &str {
        value.to_str().expect("utf-8 path")
    }

    /// An unsigned copy of a fixture package (manifest + wasm only).
    fn unsigned_copy(manifest: &Path, wasm: &Path) -> PathBuf {
        let dir = temp_dir();
        fs::copy(manifest, dir.join(MANIFEST_FILE)).expect("manifest");
        fs::copy(wasm, dir.join(WASM_FILE)).expect("wasm");
        dir
    }

    fn signed_copy(manifest: &Path, wasm: &Path) -> PathBuf {
        let dir = unsigned_copy(manifest, wasm);
        let key = dir.join("key.hex");
        fs::write(&key, format!("{}\n", test_key("secret_key_hex"))).expect("key");
        cli(&["sign", path(&dir.join(MANIFEST_FILE)), "--key", path(&key)]).expect("sign");
        dir
    }

    fn rights_package(name: &str) -> PathBuf {
        let dir = format!("fixtures/models/rights-conformance/packages/{name}");
        signed_copy(
            &repo(&format!("{dir}/{MANIFEST_FILE}")),
            &repo("fixtures/models/fixture-echo-1.0.0/model.wasm"),
        )
    }

    /// End to end with only the CLI (DoD): package the trained digits model
    /// from its unsigned manifest + wasm, then sign, pin, verify, and run the
    /// cross-engine conformance vector. The CLI-built package is byte-identical
    /// to the checked-in fixture that the native and web suites register.
    #[test]
    fn digits_packages_end_to_end_with_only_the_cli() {
        let fixture = repo("fixtures/models/digits-mlp-1.0.0");
        let dir = unsigned_copy(&fixture.join(MANIFEST_FILE), &fixture.join(WASM_FILE));
        let key = dir.join("key.hex");
        fs::write(&key, test_key("secret_key_hex")).expect("key");
        let manifest = dir.join(MANIFEST_FILE);
        let public = test_key("public_key_hex");
        let vector: Value = serde_json::from_slice(
            &fs::read(repo("fixtures/models/conformance/signed-digits-mlp.json")).expect("vector"),
        )
        .expect("vector json");

        let digest = json_out(cli(&["digest", path(&manifest), "--json"]));
        assert_eq!(digest, json!({ "digest": vector["pin"]["digest"] }));
        assert_eq!(
            cli(&["digest", path(&manifest)]).expect("digest"),
            vector["pin"]["digest"].as_str().expect("digest")
        );

        let signed = json_out(cli(&[
            "sign",
            path(&manifest),
            "--key",
            path(&key),
            "--json",
        ]));
        assert_eq!(signed["key_id"], json!(test_key("key_id")));
        assert_eq!(signed["digest"], vector["pin"]["digest"]);
        assert_eq!(
            fs::read(dir.join(SIGNATURE_FILE)).expect("sig"),
            fs::read(fixture.join(SIGNATURE_FILE)).expect("fixture sig"),
            "CLI signature is byte-identical to the checked-in fixture"
        );
        assert!(
            cli(&["sign", path(&manifest), "--key", path(&key)])
                .expect("sign")
                .starts_with("signed ")
        );

        let pin = json_out(cli(&["pin", path(&dir), "--json"]));
        assert_eq!(pin, vector["pin"]);
        assert!(
            cli(&["pin", path(&dir)])
                .expect("pin")
                .contains("\"traverse.digits-mlp\"")
        );

        let verified = json_out(cli(&[
            "verify",
            path(&dir),
            "--trusted-key",
            &public,
            "--json",
        ]));
        assert_eq!(verified["ok"], json!(true));
        assert_eq!(verified["imports"], json!(0));
        assert_eq!(verified["rights_record"]["status"], json!("active"));
        assert_eq!(
            verified["rights_record"]["rights"]["license_id"],
            json!("CC-BY-4.0")
        );
        assert!(
            cli(&["verify", path(&dir), "--trusted-key", &public])
                .expect("verify")
                .starts_with("verified traverse.digits-mlp@1.0.0")
        );

        let mut args = vec![
            "conformance".to_string(),
            "generate".to_string(),
            path(&dir).to_string(),
            "--trusted-key".to_string(),
            public.clone(),
        ];
        let cases = vector["cases"].as_array().expect("cases");
        for (index, case) in cases.iter().enumerate() {
            let input = dir.join(format!("in-{index}.bin"));
            fs::write(
                &input,
                hex_decode(case["input_frame_hex"].as_str().expect("in")).expect("hex"),
            )
            .expect("input");
            args.extend(["--input".to_string(), path(&input).to_string()]);
        }
        let out = dir.join("conformance.json");
        args.extend([
            "--out".to_string(),
            path(&out).to_string(),
            "--json".to_string(),
        ]);
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let generated = json_out(cli(&refs));
        assert_eq!(generated["engines"], json!(["wasmtime", "wasmi"]));
        assert_eq!(generated["cases"], json!(cases.len()));
        let written: Value =
            serde_json::from_slice(&fs::read(&out).expect("vector")).expect("json");
        for (case, expected) in written["cases"]
            .as_array()
            .expect("cases")
            .iter()
            .zip(cases)
        {
            assert_eq!(case["output_frame_hex"], expected["output_frame_hex"]);
        }
        assert_eq!(written["pin"], vector["pin"]);

        let checked = json_out(cli(&[
            "conformance",
            "check",
            path(&dir),
            path(&out),
            "--trusted-key",
            &public,
            "--json",
        ]));
        assert_eq!(
            checked,
            json!({
                "ok": true,
                "vector_path": path(&out),
                "engines": ["wasmtime", "wasmi"],
                "cases": cases.len(),
                "failed_cases": [],
            })
        );
        assert_eq!(
            cli(&[
                "conformance",
                "check",
                path(&dir),
                path(&out),
                "--trusted-key",
                &public
            ])
            .expect("check"),
            format!(
                "{} of {} cases match on wasmtime and wasmi",
                cases.len(),
                cases.len()
            )
        );
        assert!(
            cli(&refs[..refs.len() - 1])
                .expect("generate")
                .contains("identical on wasmtime and wasmi")
        );
    }

    /// Decision 115: the package-time preflight runs the built package on both
    /// engines and fails with the stable code and reason of whatever stage
    /// breaks, before anything is written.
    #[test]
    fn package_onnx_preflight_reports_the_failing_stage_and_reason() {
        use crate::model_packaging::{OnnxPackageSpec, build_package};
        let runner = fs::read(repo("fixtures/onnx/runner.wasm")).expect("runner");
        let onnx = fs::read(repo("fixtures/onnx/digits-mlp-1.0.0.onnx")).expect("onnx");
        let base: Value = serde_json::from_slice(
            &fs::read(repo("fixtures/onnx/digits-onnx.package.json")).expect("spec"),
        )
        .expect("spec json");
        let preflight = |tweak: &dyn Fn(&mut Value)| {
            let mut value = base.clone();
            tweak(&mut value);
            let spec: OnnxPackageSpec = serde_json::from_value(value).expect("spec");
            let package = build_package(&runner, &onnx, &spec).expect("package");
            preflight_package(&package, &spec.tensor)
        };

        let passed = preflight(&|_| {}).expect("digits passes");
        assert_eq!(passed.engines, ["wasmtime", "wasmi"]);
        assert_eq!(passed.input_frame_bytes, 272);

        let failure = |tweak: &dyn Fn(&mut Value)| -> Value {
            serde_json::from_str(&preflight(tweak).expect_err("preflight fails"))
                .expect("failure json")
        };
        // An input shape the graph cannot run: tract fails (-3).
        let inference = failure(&|v| v["tensor"]["input_shape"] = json!([1, 32]));
        assert_eq!(
            (&inference["stage"], &inference["engine"]),
            (&json!("preflight"), &json!("wasmtime"))
        );
        assert_eq!(
            (&inference["error"]["code"], &inference["error"]["reason"]),
            (&json!("execution_failed"), &json!("inference_failed"))
        );
        // A declared output shape the graph does not produce (-4).
        let output = failure(&|v| {
            v["tensor"]["output_shape"] = json!([1, 11]);
            v["max_output_bytes"] = json!(60);
        });
        assert_eq!(
            (&output["error"]["code"], &output["error"]["reason"]),
            (&json!("model_incompatible"), &json!("output_mismatch"))
        );
        // A tensor name the graph does not have: model_prepare fails.
        let prepare = failure(&|v| v["tensor"]["input_name"] = json!("nope"));
        assert_eq!(prepare["error"]["code"], json!("execution_failed"));
        // Declared fuel below the measured need, for execute and for prepare.
        for field in ["max_fuel", "max_prepare_fuel"] {
            let fuel = failure(&|v| v[field] = json!(1000));
            assert_eq!(fuel["error"]["code"], json!("execution_failed"), "{field}");
        }
    }

    #[test]
    fn preflight_output_check_requires_the_declared_dtype_and_shape() {
        let tensor: crate::model_packaging::TensorConfig = serde_json::from_value(json!({
            "input_name": "x", "output_name": "y", "input_shape": [1, 64],
            "output_shape": [1, 10], "input_dtype": 2, "output_dtype": 3,
        }))
        .expect("tensor");
        let frame = |dtype, dims: &[u32]| encode_guest_frame(dtype, dims, &[0; 40]);
        assert!(output_matches(&frame(3, &[1, 10]), &tensor));
        assert!(!output_matches(&frame(2, &[1, 10]), &tensor));
        assert!(!output_matches(&frame(3, &[10, 1]), &tensor));
        assert!(!output_matches(b"not a frame", &tensor));
        assert_eq!(zero_frame(&tensor).len(), 4 + 8 + 4 + 256);
    }

    /// #1591 DoD: `model package-onnx` → `model sign` → `model verify` →
    /// `model conformance` yields a registrable package. The result is
    /// byte-identical to the checked-in `digits-onnx-1.0.0` fixture that the
    /// native and web suites register, and its outputs match that vector.
    #[test]
    fn onnx_runner_package_signs_verifies_and_conforms_end_to_end() {
        let dir = temp_dir();
        crate::model_packaging::package_onnx(
            &repo("fixtures/onnx/runner.wasm"),
            &repo("fixtures/onnx/digits-mlp-1.0.0.onnx"),
            &repo("fixtures/onnx/digits-onnx.package.json"),
            &dir,
            preflight_package,
        )
        .expect("package-onnx");
        let key = dir.join("key.hex");
        fs::write(&key, test_key("secret_key_hex")).expect("key");
        cli(&["sign", path(&dir.join(MANIFEST_FILE)), "--key", path(&key)]).expect("sign");
        let fixture = repo("fixtures/models/digits-onnx-1.0.0");
        for file in [MANIFEST_FILE, WASM_FILE, SIGNATURE_FILE] {
            assert_eq!(
                fs::read(dir.join(file)).expect("built"),
                fs::read(fixture.join(file)).expect("fixture"),
                "{file} is byte-identical to the checked-in fixture"
            );
        }

        let public = test_key("public_key_hex");
        let verified = json_out(cli(&[
            "verify",
            path(&dir),
            "--trusted-key",
            &public,
            "--json",
        ]));
        assert_eq!(verified["ok"], json!(true), "{verified}");
        assert_eq!(
            verified["rights_record"]["rights"]["derivation"]["kind"],
            json!("converted")
        );

        let vector: Value = serde_json::from_slice(
            &fs::read(repo("fixtures/models/conformance/signed-digits-onnx.json")).expect("vector"),
        )
        .expect("vector json");
        let case = &vector["cases"][0];
        let input = dir.join("in.bin");
        fs::write(
            &input,
            hex_decode(case["input_frame_hex"].as_str().expect("in")).expect("hex"),
        )
        .expect("input");
        let out = dir.join("conformance.json");
        cli(&[
            "conformance",
            "generate",
            path(&dir),
            "--trusted-key",
            &public,
            "--input",
            path(&input),
            "--out",
            path(&out),
        ])
        .expect("generate");
        let written: Value =
            serde_json::from_slice(&fs::read(&out).expect("vector")).expect("json");
        assert_eq!(
            written["cases"][0]["output_frame_hex"],
            case["output_frame_hex"]
        );
        assert_eq!(written["pin"], vector["pin"]);
        cli(&[
            "conformance",
            "check",
            path(&dir),
            path(&out),
            "--trusted-key",
            &public,
        ])
        .expect("check");
    }

    fn verify_error(dir: &Path, extra: &[&str]) -> Value {
        let public = test_key("public_key_hex");
        let mut args = vec!["verify", path(dir), "--trusted-key", &public, "--json"];
        args.extend(extra);
        let report = json_out(cli(&args));
        assert_eq!(report["ok"], json!(false), "{report}");
        report["error"].clone()
    }

    #[test]
    fn verify_model_usage_flag_and_json_usage_errors() {
        let public = test_key("public_key_hex");
        let permissive = rights_package("permissive");
        let prohibited = rights_package("non-commercial");

        let undeclared = json_out(cli(&[
            "verify",
            path(&permissive),
            "--trusted-key",
            &public,
            "--json",
        ]));
        assert_eq!(undeclared["ok"], json!(true), "{undeclared}");
        assert_eq!(undeclared["verification_usage"], json!("non_commercial"));
        assert_eq!(undeclared["rights_record"]["effective_usage"], Value::Null);

        let commercial = json_out(cli(&[
            "verify",
            path(&permissive),
            "--trusted-key",
            &public,
            "--model-usage",
            "commercial",
            "--json",
        ]));
        assert_eq!(commercial["ok"], json!(true), "{commercial}");
        assert_eq!(commercial["verification_usage"], json!("commercial"));
        assert_eq!(
            commercial["rights_record"]["effective_usage"],
            json!("commercial")
        );

        let denied = json_out(cli(&[
            "verify",
            path(&prohibited),
            "--trusted-key",
            &public,
            "--model-usage",
            "commercial",
            "--json",
        ]));
        assert_eq!(denied["ok"], json!(false), "{denied}");
        assert_eq!(denied["error"]["reason"], json!("rights_policy_denied"));

        let require = json_out(cli(&[
            "verify",
            path(&permissive),
            "--trusted-key",
            &public,
            "--require-model-usage",
            "--json",
        ]));
        assert_eq!(require["ok"], json!(false), "{require}");
        assert_eq!(require["error"]["reason"], json!("usage_undeclared"));
        assert_eq!(require["verification_usage"], Value::Null);

        let args = |rest: &[&str]| -> Vec<String> {
            ["traverse-cli", "model"]
                .iter()
                .chain(rest)
                .map(ToString::to_string)
                .collect()
        };
        assert!(
            parse(&args(&[
                "verify",
                "dir",
                "--trusted-key",
                "k",
                "--model-usage",
                "research",
            ]))
            .unwrap_err()
            .contains("commercial or non_commercial")
        );
        assert_eq!(
            parse(&args(&[
                "verify",
                "dir",
                "--trusted-key",
                "k",
                "--model-usage",
                "commercial",
                "--require-model-usage",
            ])),
            Ok(ModelCommand::Verify {
                package_dir: PathBuf::from("dir"),
                trusted_keys_hex: vec!["k".to_string()],
                limits: HostModelLimits::default(),
                model_usage: Some(ModelUsage::Commercial),
                require_model_usage: true,
                json: false,
            })
        );

        // UsageError path with --json must be a JSON envelope (exercised via
        // run(), matching main's emit_usage_error for the binary).
        let short_key = cli(&[
            "verify",
            path(&permissive),
            "--trusted-key",
            "00",
            "--json",
        ]);
        match short_key {
            Err(CliError::UsageError(message)) => {
                assert!(message.contains("64 hex"), "{message}");
            }
            other => panic!("expected usage error, got {other:?}"),
        }
    }

    #[test]
    fn verify_reports_runtime_rights_rules_with_stable_reasons() {
        let public = test_key("public_key_hex");
        for ok in ["non-commercial", "restricted", "derivative"] {
            let report = json_out(cli(&[
                "verify",
                path(&rights_package(ok)),
                "--trusted-key",
                &public,
                "--json",
            ]));
            assert_eq!(report["ok"], json!(true), "{ok}: {report}");
        }
        let inconsistent = verify_error(&rights_package("derivative-inconsistent"), &[]);
        assert_eq!(inconsistent["reason"], json!("rights_inconsistent"));
        assert_eq!(
            inconsistent["detail"]["field"],
            json!("rights.commercial_use")
        );
        assert_eq!(
            verify_error(&rights_package("missing-attribution"), &[])["detail"]["field"],
            json!("rights.attribution")
        );
        assert_eq!(
            verify_error(&rights_package("derivation-in-2-0-0"), &[])["reason"],
            json!("manifest_invalid")
        );
        assert_eq!(
            verify_error(&rights_package("permissive"), &["--max-fuel", "1"])["reason"],
            json!("host_limit_exceeded")
        );

        let tampered = rights_package("permissive");
        let mut wasm = fs::read(tampered.join(WASM_FILE)).expect("wasm");
        wasm.push(0);
        fs::write(tampered.join(WASM_FILE), wasm).expect("tamper");
        assert_eq!(
            verify_error(&tampered, &[])["reason"],
            json!("digest_mismatch")
        );

        let other_key = "11".repeat(32);
        let untrusted = json_out(cli(&[
            "verify",
            path(&rights_package("permissive")),
            "--trusted-key",
            &other_key,
            "--json",
        ]));
        assert_eq!(untrusted["error"]["reason"], json!("key_untrusted"));
        match cli(&[
            "verify",
            path(&rights_package("permissive")),
            "--trusted-key",
            &other_key,
        ]) {
            Err(CliError::ValidationFailed(text)) => assert!(text.contains("key_untrusted")),
            other => panic!("expected validation failure, got {other:?}"),
        }
    }

    /// `(module (import "m" "f" (func)))`, a well-formed module with one import.
    const IMPORTING_WASM: &[u8] = &[
        0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x01, 0x04, 0x01, 0x60, 0x00, 0x00, 0x02,
        0x07, 0x01, 0x01, 0x6d, 0x01, 0x66, 0x00, 0x00,
    ];

    #[test]
    fn verify_rejects_wasm_imports_and_counts_sections() {
        let dir = temp_dir();
        fs::write(dir.join(WASM_FILE), IMPORTING_WASM).expect("wasm");
        let mut manifest: Value = serde_json::from_slice(
            &fs::read(repo(
                "fixtures/models/rights-conformance/packages/permissive/model.manifest.json",
            ))
            .expect("manifest"),
        )
        .expect("json");
        manifest["wasm_digest"] = json!(sha256_hex(IMPORTING_WASM));
        fs::write(
            dir.join(MANIFEST_FILE),
            serde_json::to_vec(&manifest).expect("bytes"),
        )
        .expect("manifest");
        let key = dir.join("key.hex");
        fs::write(&key, test_key("secret_key_hex")).expect("key");
        cli(&["sign", path(&dir.join(MANIFEST_FILE)), "--key", path(&key)]).expect("sign");
        let error = verify_error(&dir, &[]);
        assert_eq!(error["reason"], json!("manifest_invalid"));

        assert_eq!(wasm_import_count(IMPORTING_WASM), Some(1));
        assert_eq!(wasm_import_count(b"\0asm\x01\0\0\0"), Some(0));
        assert_eq!(
            wasm_import_count(b"\0asm\x01\0\0\0\x01\x05\x00"),
            None,
            "truncated section"
        );
        assert_eq!(
            wasm_import_count(b"\0asm\x01\0\0\0\x01\xff"),
            None,
            "unterminated leb128"
        );
        assert_eq!(wasm_import_count(b"\0asm\x02\0\0\0"), None, "wrong version");
        assert_eq!(wasm_import_count(b"\0as"), None, "short header");
        assert_eq!(leb128_u32(&[0xff, 0xff, 0xff, 0xff, 0xff, 0x01], 0), None);
    }

    #[test]
    fn conformance_check_reports_mismatches_and_bad_vectors() {
        let public = test_key("public_key_hex");
        let dir = rights_package("permissive");
        let input = dir.join("in.bin");
        fs::write(&input, b"rights").expect("input");
        let out = dir.join("vector.json");
        cli(&[
            "conformance",
            "generate",
            path(&dir),
            "--trusted-key",
            &public,
            "--input",
            path(&input),
            "--out",
            path(&out),
        ])
        .expect("generate");
        let mut vector: Value =
            serde_json::from_slice(&fs::read(&out).expect("vector")).expect("json");
        assert_eq!(
            vector["cases"][0]["output_frame_hex"],
            json!(hex_encode(b"rights"))
        );

        vector["cases"][0]["output_frame_hex"] = json!("00");
        fs::write(&out, serde_json::to_vec(&vector).expect("bytes")).expect("write");
        let checked = json_out(cli(&[
            "conformance",
            "check",
            path(&dir),
            path(&out),
            "--trusted-key",
            &public,
            "--json",
        ]));
        assert_eq!(checked["failed_cases"], json!([0]));

        vector["cases"][0]["input_frame_hex"] = json!("zz");
        fs::write(&out, serde_json::to_vec(&vector).expect("bytes")).expect("write");
        assert!(matches!(
            cli(&["conformance", "check", path(&dir), path(&out), "--trusted-key", &public]),
            Err(CliError::ValidationFailed(message)) if message.contains("not hex")
        ));

        vector["pin"]["digest"] = json!("00");
        fs::write(&out, serde_json::to_vec(&vector).expect("bytes")).expect("write");
        assert!(matches!(
            cli(&["conformance", "check", path(&dir), path(&out), "--trusted-key", &public]),
            Err(CliError::ValidationFailed(message)) if message.contains("different package digest")
        ));

        fs::write(&out, b"{}").expect("write");
        assert!(matches!(
            cli(&["conformance", "check", path(&dir), path(&out), "--trusted-key", &public]),
            Err(CliError::ValidationFailed(message)) if message.contains("malformed")
        ));

        // A package the trusted key did not sign cannot run conformance.
        assert!(matches!(
            cli(&[
                "conformance", "generate", path(&dir), "--trusted-key", &"11".repeat(32),
                "--input", path(&input), "--out", path(&out),
            ]),
            Err(CliError::ValidationFailed(message)) if message.contains("key_untrusted")
        ));
        // An input over the manifest ceiling fails closed at staging.
        let big = dir.join("big.bin");
        fs::write(&big, vec![0_u8; 5000]).expect("big");
        assert!(matches!(
            cli(&[
                "conformance",
                "generate",
                path(&dir),
                "--trusted-key",
                &public,
                "--input",
                path(&big),
                "--out",
                path(&out),
            ]),
            Err(CliError::ValidationFailed(_))
        ));
    }

    #[test]
    fn engines_must_agree_byte_for_byte() {
        assert_eq!(
            agreed_output(vec![("wasmtime", vec![1]), ("wasmi", vec![1])]).expect("agree"),
            vec![1]
        );
        assert!(matches!(
            agreed_output(vec![("wasmtime", vec![1]), ("wasmi", vec![2])]),
            Err(CliError::ValidationFailed(message)) if message == "engine wasmi output differs from wasmtime"
        ));
        assert!(agreed_output(Vec::new()).is_err());
    }

    #[test]
    fn parse_rejects_malformed_invocations_and_help_covers_every_subcommand() {
        let args = |rest: &[&str]| -> Vec<String> {
            ["traverse-cli", "model"]
                .iter()
                .chain(rest)
                .map(ToString::to_string)
                .collect()
        };
        for bad in [
            vec![],
            vec!["digest"],
            vec!["sign", "m.json"],
            vec!["verify", "dir"],
            vec!["verify", "dir", "--bogus", "x"],
            vec!["verify", "dir", "--trusted-key", "k", "--model-usage", "research"],
            vec!["verify", "dir", "--trusted-key", "k", "--max-fuel", "0"],
            vec![
                "verify",
                "dir",
                "--trusted-key",
                "k",
                "--max-package-bytes",
                "x",
            ],
            vec!["conformance", "generate", "dir", "--trusted-key", "k"],
            vec![
                "conformance",
                "generate",
                "dir",
                "--input",
                "i",
                "--out",
                "o",
            ],
            vec!["conformance", "generate", "dir", "--nope", "x"],
            vec!["conformance", "check", "dir", "v.json"],
            vec!["conformance", "check", "dir", "v.json", "--bogus", "x"],
            vec!["convert-coreml", "a.onnx", "b.zip", "--bogus", "x"],
            vec!["add-variant", "pkg", "--f32-dtype", "256"],
        ] {
            assert!(parse(&args(&bad)).is_err(), "{bad:?}");
        }
        assert_eq!(
            parse(&args(&[
                "verify",
                "dir",
                "--trusted-key",
                "k",
                "--max-package-bytes",
                "1",
                "--max-memory-bytes",
                "2",
                "--max-fuel",
                "3",
            ])),
            Ok(ModelCommand::Verify {
                package_dir: PathBuf::from("dir"),
                trusted_keys_hex: vec!["k".to_string()],
                limits: HostModelLimits {
                    max_package_bytes: 1,
                    max_memory_bytes: 2,
                    max_fuel: 3,
                    ..HostModelLimits::default()
                },
                model_usage: None,
                require_model_usage: false,
                json: false,
            })
        );
        for subcommand in [
            None,
            Some("digest"),
            Some("sign"),
            Some("verify"),
            Some("pin"),
            Some("conformance"),
        ] {
            assert!(
                help(subcommand).starts_with("traverse-cli model"),
                "{subcommand:?}"
            );
        }
    }

    #[test]
    fn io_and_key_errors_fail_closed() {
        let dir = temp_dir();
        let missing = dir.join("missing.json");
        assert!(matches!(
            cli(&["digest", path(&missing)]),
            Err(CliError::IoError(_))
        ));
        assert!(matches!(
            cli(&["pin", path(&dir)]),
            Err(CliError::IoError(_))
        ));

        let manifest = dir.join(MANIFEST_FILE);
        fs::write(&manifest, b"{}").expect("manifest");
        let key = dir.join("key.hex");
        fs::write(&key, "abcd").expect("key");
        assert!(matches!(
            cli(&["sign", path(&manifest), "--key", path(&key)]),
            Err(CliError::UsageError(message)) if message.contains("64 hex")
        ));
        fs::write(&key, [0xff, 0xfe]).expect("key");
        assert!(matches!(
            cli(&["sign", path(&manifest), "--key", path(&key)]),
            Err(CliError::UsageError(message)) if message.contains("UTF-8")
        ));
        fs::write(&key, "abc").expect("key");
        assert!(matches!(
            cli(&["sign", path(&manifest), "--key", path(&key)]),
            Err(CliError::UsageError(_))
        ));

        fs::write(dir.join(WASM_FILE), b"").expect("wasm");
        fs::write(dir.join(SIGNATURE_FILE), b"{}").expect("sig");
        assert!(matches!(
            cli(&["pin", path(&dir)]),
            Err(CliError::ValidationFailed(message)) if message.contains("manifest is malformed")
        ));
        let package = rights_package("permissive");
        fs::write(package.join(SIGNATURE_FILE), b"{}").expect("sig");
        assert!(matches!(
            cli(&["pin", path(&package)]),
            Err(CliError::ValidationFailed(message)) if message.contains("model.sig.json is malformed")
        ));
        assert!(matches!(
            cli(&[
                "verify",
                path(&rights_package("permissive")),
                "--trusted-key",
                "zz"
            ]),
            Err(CliError::UsageError(_))
        ));
        let off_curve = "ff".repeat(32);
        assert!(matches!(
            cli(&[
                "verify",
                path(&rights_package("permissive")),
                "--trusted-key",
                &off_curve
            ]),
            Err(CliError::UsageError(_) | CliError::ValidationFailed(_))
        ));
        let unwritable = dir.join("no-such-dir").join(MANIFEST_FILE);
        assert!(matches!(
            cli(&[
                "conformance",
                "generate",
                path(&rights_package("permissive")),
                "--trusted-key",
                &test_key("public_key_hex"),
                "--input",
                path(&key),
                "--out",
                path(&unwritable),
            ]),
            Err(CliError::IoError(_))
        ));
    }

    /// BirdNET-shaped package: signed `coreml` variant, reproducible conversion,
    /// and verify failures for a tampered digest and a missing variant file.
    /// The ONNX bytes are a stand-in; BirdNET weights are not in the repo.
    const FAKE_LOCK: &str = "e6a67b7c977655163c28f91e548a2b9745eb42f3ca338aae0bad854ab08ae5bc";

    /// A `sh` script standing in for `tools/coreml/convert_variant.py`: it
    /// writes `variant` to its second argument and prints the report the real
    /// converter prints, with `source` as the ONNX bytes it claims to read.
    fn fake_converter(dir: &Path, source: &[u8], variant: &[u8], lock: &str) -> PathBuf {
        let lock = if lock.is_empty() { FAKE_LOCK } else { lock };
        let script = dir.join("fake-convert.sh");
        let report = json!({
            "source_digest": sha256_hex(source),
            "digest": sha256_hex(variant),
            "lock_digest": lock,
            "rewrites": {"explicit_pads": 2, "reverse_slices": 0, "dynamic_int8_matmuls": 0},
        });
        fs::write(
            &script,
            format!(
                "printf '%s' '{}' > \"$2\"\necho converting\necho '{report}'\n",
                String::from_utf8_lossy(variant)
            ),
        )
        .expect("script");
        script
    }

    fn convert_error(dir: &Path, script: &Path, onnx: &Path) -> String {
        let out = dir.join("out/model.mlpackage.zip");
        match cli(&[
            "convert-coreml",
            path(onnx),
            path(&out),
            "--python",
            "sh",
            "--converter",
            path(script),
        ]) {
            Err(error) => error.to_string(),
            Ok(output) => panic!("expected failure, got {output}"),
        }
    }

    #[test]
    fn convert_coreml_rejects_a_failing_or_inconsistent_converter() {
        let dir = temp_dir();
        let onnx = dir.join("source.onnx");
        fs::write(&onnx, b"onnx").expect("onnx");

        let failing = dir.join("fail.sh");
        fs::write(&failing, "echo 'unsupported op' >&2\nexit 3\n").expect("script");
        assert!(convert_error(&dir, &failing, &onnx).contains("unsupported op"));

        let silent = dir.join("silent.sh");
        fs::write(&silent, "printf x > \"$2\"\necho done\n").expect("script");
        assert!(convert_error(&dir, &silent, &onnx).contains("no JSON report"));

        let wrong_source = fake_converter(&dir, b"other onnx", b"zip", "");
        assert!(convert_error(&dir, &wrong_source, &onnx).contains("does not match"));

        let no_lock = fake_converter(&dir, b"onnx", b"zip", "not-a-digest");
        assert!(convert_error(&dir, &no_lock, &onnx).contains("requirements.lock"));

        let wrong_digest = dir.join("wrong-digest.sh");
        let report = json!({
            "source_digest": sha256_hex(b"onnx"),
            "digest": sha256_hex(b"claimed"),
            "lock_digest": FAKE_LOCK,
        });
        fs::write(
            &wrong_digest,
            format!("printf actual > \"$2\"\necho '{report}'\n"),
        )
        .expect("script");
        assert!(convert_error(&dir, &wrong_digest, &onnx).contains("does not match"));

        let missing = cli(&[
            "convert-coreml",
            path(&onnx),
            path(&dir.join("x.mlpackage.zip")),
            "--python",
            "/nonexistent/python3",
        ])
        .expect_err("missing interpreter");
        assert!(missing.to_string().contains("/nonexistent/python3"));
    }

    #[test]
    fn birdnet_package_signed_coreml_variant_verifies() {
        let package = unsigned_copy(
            &repo("fixtures/models/rights-conformance/packages/derivative/model.manifest.json"),
            &repo("fixtures/models/fixture-echo-1.0.0/model.wasm"),
        );
        let manifest_path = package.join(MANIFEST_FILE);
        let mut manifest: Value =
            serde_json::from_slice(&fs::read(&manifest_path).expect("manifest")).expect("json");
        manifest["model_id"] = json!("birdnet-v2.4-int8");
        fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&manifest).expect("json"),
        )
        .expect("write");

        let onnx_path = package.join("source.onnx");
        fs::write(&onnx_path, b"birdnet-onnx-stand-in").expect("onnx");
        let script = fake_converter(&package, b"birdnet-onnx-stand-in", b"canonical-zip", "");
        let mlpackage = package.join("converted.mlpackage.zip");
        let converted = json_out(cli(&[
            "convert-coreml",
            path(&onnx_path),
            path(&mlpackage),
            "--python",
            "sh",
            "--converter",
            path(&script),
            "--json",
        ]));
        assert_eq!(converted["tool"], json!(COREML_CONVERT_TOOL));
        let tool_version = format!("lock-sha256:{FAKE_LOCK}");
        assert_eq!(converted["tool_version"], json!(tool_version));
        assert_eq!(converted["digest"], json!(sha256_hex(b"canonical-zip")));
        assert_eq!(converted["rewrites"]["explicit_pads"], json!(2));

        manifest["rights"]["derivation"]["source_digest"] = converted["source_digest"].clone();
        fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&manifest).expect("json"),
        )
        .expect("write");
        let vector = package.join("vector.json");
        fs::write(&vector, b"{\"cases\":[]}").expect("vector");
        let added = json_out(cli(&[
            "add-variant",
            path(&package),
            "--file",
            path(&mlpackage),
            "--conformance",
            path(&vector),
            "--peak-memory-bytes",
            "1048576",
            "--max-abs-diff",
            "0.005",
            "--top-k",
            "5",
            "--tool",
            COREML_CONVERT_TOOL,
            "--tool-version",
            &tool_version,
            "--f32-dtype",
            "3",
            "--json",
        ]));
        assert_eq!(added["model_id"], json!("birdnet-v2.4-int8"));
        let signed: Value =
            serde_json::from_slice(&fs::read(&manifest_path).expect("manifest")).expect("json");
        assert_eq!(
            signed["accelerator_variants"][0]["tolerance"]["f32_dtypes"],
            json!([3])
        );
        assert_eq!(
            signed["accelerator_variants"][0]["format"],
            json!("mlpackage-zip")
        );
        assert_eq!(added["adapter"], json!("coreml"));

        let key = package.join("key.hex");
        fs::write(&key, format!("{}\n", test_key("secret_key_hex"))).expect("key");
        cli(&["sign", path(&manifest_path), "--key", path(&key)]).expect("sign");
        let verified = json_out(cli(&[
            "verify",
            path(&package),
            "--trusted-key",
            &test_key("public_key_hex"),
            "--json",
        ]));
        assert_eq!(verified["ok"], json!(true), "{verified}");

        let variant_path = package.join("variants/coreml/model.mlpackage.zip");
        let mut tampered = fs::read(&variant_path).expect("variant");
        tampered[0] = tampered[0].wrapping_add(1);
        fs::write(&variant_path, &tampered).expect("tamper");
        let failed = json_out(cli(&[
            "verify",
            path(&package),
            "--trusted-key",
            &test_key("public_key_hex"),
            "--json",
        ]));
        assert_eq!(failed["ok"], json!(false));
        assert_eq!(failed["error"]["reason"], json!("digest_mismatch"));

        fs::remove_file(&variant_path).expect("remove");
        let missing = json_out(cli(&[
            "verify",
            path(&package),
            "--trusted-key",
            &test_key("public_key_hex"),
            "--json",
        ]));
        assert_eq!(missing["ok"], json!(false));
        assert_eq!(missing["error"]["reason"], json!("digest_mismatch"));
        assert!(
            missing["error"]["message"]
                .as_str()
                .unwrap_or("")
                .contains("missing")
        );
    }
}
