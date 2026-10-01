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
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use traverse_runtime::exact_model::{
    ExactModelHostConnector, ExactModelPin, ExecutionPolicy, HostModelLimits, ModelEngine,
    ModelPackageManifest, ModelPackageSignature, ModelUsage, PLACEMENT_WASM_CPU, PinRights,
    TrustedModelKeys, sign_model_manifest,
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
        json: bool,
    },
    Pin {
        package_dir: PathBuf,
        json: bool,
    },
    ConformanceGenerate {
        package_dir: PathBuf,
        trusted_keys_hex: Vec<String>,
        inputs: Vec<PathBuf>,
        out_path: PathBuf,
        json: bool,
    },
    ConformanceCheck {
        package_dir: PathBuf,
        trusted_keys_hex: Vec<String>,
        vector_path: PathBuf,
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
            let trusted_keys_hex = trusted_keys(flags, "conformance")?;
            Ok(ModelCommand::ConformanceCheck {
                package_dir: PathBuf::from(package_dir),
                trusted_keys_hex,
                vector_path: PathBuf::from(vector),
                json,
            })
        }
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

fn parse_verify(package_dir: &str, flags: &[&str], json: bool) -> Result<ModelCommand, String> {
    let mut trusted_keys_hex = Vec::new();
    let mut limits = HostModelLimits::default();
    for pair in flags.chunks(2) {
        match pair {
            ["--trusted-key", key] => trusted_keys_hex.push((*key).to_string()),
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
        json,
    })
}

/// `--trusted-key <hex>` pairs only; at least one is required.
fn trusted_keys(flags: &[&str], subcommand: &str) -> Result<Vec<String>, String> {
    let mut keys = Vec::new();
    for pair in flags.chunks(2) {
        match pair {
            ["--trusted-key", key] => keys.push((*key).to_string()),
            _ => return Err(help(Some(subcommand))),
        }
    }
    if keys.is_empty() {
        return Err(format!(
            "model {subcommand} requires at least one --trusted-key <hex>"
        ));
    }
    Ok(keys)
}

fn parse_conformance_generate(
    package_dir: &str,
    flags: &[&str],
    json: bool,
) -> Result<ModelCommand, String> {
    let mut inputs = Vec::new();
    let mut out_path = None;
    let mut key_flags = Vec::new();
    for pair in flags.chunks(2) {
        match pair {
            ["--input", path] => inputs.push(PathBuf::from(path)),
            ["--out", path] => out_path = Some(PathBuf::from(path)),
            ["--trusted-key", key] => key_flags.extend(["--trusted-key", *key]),
            _ => return Err(help(Some("conformance"))),
        }
    }
    let trusted_keys_hex = trusted_keys(&key_flags, "conformance")?;
    match out_path {
        Some(out_path) if !inputs.is_empty() => Ok(ModelCommand::ConformanceGenerate {
            package_dir: PathBuf::from(package_dir),
            trusted_keys_hex,
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
        Some("verify") => "traverse-cli model verify <package-dir> --trusted-key <hex>... [--max-package-bytes N] [--max-memory-bytes N] [--max-fuel N] [--json]

  Purpose:
    Verify a package exactly as a host registers it: manifest schema and
    rights (including rights.derivation), signature by a trusted key, WASM
    digest, wasm-cpu target, declared limits against the host ceilings, and
    zero WASM imports. Exits non-zero with the stable code/reason on failure.

  Example:
    traverse-cli model verify out/my-model --trusted-key <public-key-hex> --json"
            .to_string(),
        Some("pin") => "traverse-cli model pin <package-dir> [--json]

  Purpose:
    Print the app manifest exact_model_dependencies entry for a signed package
    (digest, rights, target, signer key_id).

  Example:
    traverse-cli model pin out/my-model --json"
            .to_string(),
        Some("conformance") => "traverse-cli model conformance generate <package-dir> --trusted-key <hex> --input <frame.bin>... --out <vector.json> [--json]
traverse-cli model conformance check <package-dir> <vector.json> --trusted-key <hex> [--json]

  Purpose:
    generate: run each input frame on wasmtime and wasmi, require
    byte-identical output, and write a conformance vector.
    check: re-run a vector on both engines and compare with its expected
    outputs. Third-party packages ship a vector that passes here.

  Example:
    traverse-cli model conformance generate out/my-model --trusted-key <public-key-hex> --input in.bin --out out/my-model/conformance.json"
            .to_string(),
        _ => "traverse-cli model <subcommand> [options]

  Subcommands:
    digest <model.manifest.json>                    Print the pin digest.
    sign <model.manifest.json> --key <file>         Write a detached model.sig.json.
    verify <package-dir> --trusted-key <hex>...     Verify a package as a host registers it.
    pin <package-dir>                               Print the exact_model_dependencies entry.
    conformance generate|check <package-dir> ...    Cross-engine (wasmtime + wasmi) conformance vectors.

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
            json,
        } => verify(package_dir, trusted_keys_hex, *limits, *json),
        ModelCommand::Pin { package_dir, json } => {
            let pin = pin_for(package_dir)?;
            let value = to_json(&pin)?;
            Ok(render(*json, &value, || value.to_string()))
        }
        ModelCommand::ConformanceGenerate {
            package_dir,
            trusted_keys_hex,
            inputs,
            out_path,
            json,
        } => conformance_generate(package_dir, trusted_keys_hex, inputs, out_path, *json),
        ModelCommand::ConformanceCheck {
            package_dir,
            trusted_keys_hex,
            vector_path,
            json,
        } => conformance_check(package_dir, trusted_keys_hex, vector_path, *json),
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
    Ok(ExactModelPin {
        model_id: manifest.model_id,
        version: manifest.version,
        digest: sha256_hex(&files.manifest),
        offline_allowed: manifest.offline_allowed,
        target: PLACEMENT_WASM_CPU.to_string(),
        rights: PinRights {
            license_id: manifest.rights.license_id,
            commercial_use: manifest.rights.commercial_use,
        },
        key_id: Some(signature.key_id),
    })
}

/// A host for one package. `non_commercial` usage judges the package itself,
/// not an app's commercial policy, so `prohibited` packages still verify.
fn host_for(
    pin: &ExactModelPin,
    trusted_keys_hex: &[String],
    limits: HostModelLimits,
) -> Result<ExactModelHostConnector, CliError> {
    let mut keys = TrustedModelKeys::new();
    for hex in trusted_keys_hex {
        keys.trust(&key_bytes(hex, "--trusted-key")?)
            .map_err(|error| CliError::UsageError(error.message))?;
    }
    let mut host = ExactModelHostConnector::new(vec![pin.clone()], keys);
    host.model_usage = Some(ModelUsage::NonCommercial);
    host.host_limits = limits;
    host.policies.insert(
        CONFORMANCE_POLICY.to_string(),
        ExecutionPolicy {
            policy_ref: CONFORMANCE_POLICY.to_string(),
            allowed_classifications: vec![CONFORMANCE_CLASSIFICATION.to_string()],
            max_output_bytes: u64::MAX,
        },
    );
    Ok(host)
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
    json: bool,
) -> Result<String, CliError> {
    let files = package_files(package_dir)?;
    let pin = pin_for(package_dir)?;
    let mut host = host_for(&pin, trusted_keys_hex, limits)?;
    let registered = host.register_package(&files.manifest, files.wasm.clone(), &files.signature);
    let imports = wasm_import_count(&files.wasm);
    let mut report = json!({
        "ok": registered.is_ok() && imports == Some(0),
        "package_dir": package_dir.display().to_string(),
        "model_id": pin.model_id,
        "version": pin.version,
        "digest": pin.digest,
        "key_id": pin.key_id,
        "imports": imports,
    });
    match &registered {
        Ok(_) => {
            report["rights_record"] = to_json(&host.model_rights_record(&pin.digest))?;
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
    engine: ModelEngine,
    input: &[u8],
) -> Result<Vec<u8>, CliError> {
    let manifest = parse_manifest(&files.manifest)?;
    let mut host = host_for(pin, trusted_keys_hex, HostModelLimits::default())?;
    host.engine = engine;
    host.register_package(&files.manifest, files.wasm.clone(), &files.signature)
        .map_err(|error| CliError::ValidationFailed(error_json(&error).to_string()))?;
    let input_ref = host
        .io
        .stage_model_input(
            input,
            usize::try_from(manifest.max_input_bytes).unwrap_or(usize::MAX),
        )
        .map_err(|error| CliError::ValidationFailed(error_json(&error).to_string()))?;
    let result = host
        .invoke(&HostConnectorHostRequest {
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
        })
        .map_err(|error| CliError::ValidationFailed(error_json(&error).to_string()))?;
    host.io
        .read_model_output(
            result.artifact_ref.as_deref().unwrap_or_default(),
            usize::try_from(manifest.max_output_bytes).unwrap_or(usize::MAX),
        )
        .map_err(|error| CliError::ValidationFailed(error_json(&error).to_string()))
}

/// Run a frame on every engine and require byte-identical output.
fn cross_engine(
    files: &PackageFiles,
    pin: &ExactModelPin,
    trusted_keys_hex: &[String],
    input: &[u8],
) -> Result<Vec<u8>, CliError> {
    let mut outputs = Vec::new();
    for (name, engine) in ENGINES {
        outputs.push((
            name,
            execute_on(files, pin, trusted_keys_hex, engine, input)?,
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
        let output = cross_engine(&files, &pin, trusted_keys_hex, &input)?;
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
        let output = cross_engine(&files, &pin, trusted_keys_hex, &input)?;
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

    fn verify_error(dir: &Path, extra: &[&str]) -> Value {
        let public = test_key("public_key_hex");
        let mut args = vec!["verify", path(dir), "--trusted-key", &public, "--json"];
        args.extend(extra);
        let report = json_out(cli(&args));
        assert_eq!(report["ok"], json!(false), "{report}");
        report["error"].clone()
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
                },
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
}
