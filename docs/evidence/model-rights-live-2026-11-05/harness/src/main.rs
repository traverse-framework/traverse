//! #1786: runs published traverse-runtime 0.16.2's registry rights policy and
//! published traverse-contracts 0.16.2's parser on live registry records.
use std::{env, fs, path::Path};
use traverse_registry::PublicRegistryCapabilityRecord;
use traverse_runtime::registry_rights_policy::{ModelRightsPolicy, check_registry_record_rights};

fn decision(record: &PublicRegistryCapabilityRecord, allow_conditional: bool) -> String {
    match check_registry_record_rights(record, ModelRightsPolicy { allow_conditional }) {
        Ok(()) => "permitted".to_string(),
        Err(denial) => format!("denied {}", denial.code.as_str()),
    }
}

fn main() -> Result<(), String> {
    let args: Vec<String> = env::args().collect();
    let index: serde_json::Value = serde_json::from_slice(&fs::read(&args[1]).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let cache = Path::new(&args[2]);
    let wanted = ["text.detect-entities", "text.redact-entities", "audio.detect-speech-segments",
        "audio.transcribe-speech", "report.translate-fr-semantic"];
    let mut rows = Vec::new();
    for raw in index["capabilities"].as_array().ok_or("no capabilities")? {
        let id = raw["id"].as_str().unwrap_or_default();
        if !wanted.contains(&id) { continue; }
        let record: PublicRegistryCapabilityRecord =
            serde_json::from_value(raw.clone()).map_err(|e| format!("{id}: {e}"))?;
        let mut row = serde_json::json!({
            "id": id, "version": record.version,
            "status": format!("{:?}", record.lifecycle_status()),
            "default_policy": decision(&record, false),
            "allow_conditional_policy": decision(&record, true),
        });
        let digest = record.contract_digest.trim_start_matches("sha256:");
        if let Ok(text) = fs::read_to_string(cache.join(digest)) {
            let parsed = traverse_contracts::parse_contract(&text);
            row["contract_parse_as_published"] = parsed.as_ref().map_or_else(
                |f| serde_json::json!(f.errors.iter().map(|e| e.message.clone()).collect::<Vec<_>>()),
                |_| serde_json::json!("ok"));
            let mut value: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
            value["provenance"]["source"] = serde_json::json!("greenfield");
            let patched = traverse_contracts::parse_contract(&value.to_string());
            row["contract_parse_with_spec_002_source"] = patched.as_ref().map_or_else(
                |f| serde_json::json!(f.errors.iter().map(|e| e.message.clone()).collect::<Vec<_>>()),
                |_| serde_json::json!("ok"));
        }
        rows.push(row);
    }
    println!("{}", serde_json::to_string_pretty(&rows).map_err(|e| e.to_string())?);
    Ok(())
}
