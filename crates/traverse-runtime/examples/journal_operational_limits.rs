//! Reproducible local workload for issue #629 journal operational limits.

use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use traverse_runtime::events::{
    DurableEventJournal, JournalConfig, LifecycleStatus, SystemClock, TraverseEvent,
};

const DEFAULT_EVENTS: usize = 1_000;
const DEFAULT_PAYLOAD_BYTES: usize = 512;
const BENCH_SEGMENT_BYTES: u64 = 16 * 1024;
const BENCH_RETENTION_BYTES: u64 = 64 * 1024;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let event_count = env_usize("TRAVERSE_JOURNAL_BENCH_EVENTS", DEFAULT_EVENTS)?;
    let payload_bytes = env_usize(
        "TRAVERSE_JOURNAL_BENCH_PAYLOAD_BYTES",
        DEFAULT_PAYLOAD_BYTES,
    )?;
    let storage_profile = std::env::var("TRAVERSE_JOURNAL_BENCH_PROFILE")
        .unwrap_or_else(|_| "host-local".to_string());
    let root = benchmark_root()?;
    let config = JournalConfig {
        max_segment_bytes: BENCH_SEGMENT_BYTES,
        max_segment_age_secs: 600,
        retention_max_age_secs: None,
        retention_max_total_bytes: Some(BENCH_RETENTION_BYTES),
    };

    let mut journal = DurableEventJournal::open(&root, config.clone(), Arc::new(SystemClock))?;
    let mut append_micros = Vec::with_capacity(event_count);
    for index in 0..event_count {
        let event = fixture_event(index, payload_bytes);
        let started = Instant::now();
        let _cursor = journal.append(&event)?;
        append_micros.push(started.elapsed().as_micros());
    }
    drop(journal);

    let bytes_before_prune = directory_bytes(&root)?;
    let segments_before_prune = segment_count(&root)?;
    let recovery_started = Instant::now();
    let mut recovered = DurableEventJournal::open(&root, config, Arc::new(SystemClock))?;
    let recovery_micros = recovery_started.elapsed().as_micros();

    let replay_started = Instant::now();
    let replayed = recovered.replay_from("0", event_count)?;
    let replay_micros = replay_started.elapsed().as_micros();

    let prune_started = Instant::now();
    let deleted = recovered.prune()?;
    let prune_micros = prune_started.elapsed().as_micros();
    drop(recovered);

    let bytes_after_prune = directory_bytes(&root)?;
    let segments_after_prune = segment_count(&root)?;
    append_micros.sort_unstable();
    let result = json!({
        "schema_version": "1.0.0",
        "environment": {
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "storage_profile": storage_profile
        },
        "workload": {
            "event_count": event_count,
            "payload_bytes": payload_bytes,
            "segment_max_bytes": BENCH_SEGMENT_BYTES,
            "retention_max_total_bytes": BENCH_RETENTION_BYTES
        },
        "append_latency_micros": {
            "min": percentile(&append_micros, 0),
            "p50": percentile(&append_micros, 50),
            "p95": percentile(&append_micros, 95),
            "p99": percentile(&append_micros, 99),
            "max": percentile(&append_micros, 100)
        },
        "restart_recovery_micros": recovery_micros,
        "replay": {
            "event_count": replayed.len(),
            "elapsed_micros": replay_micros,
            "events_per_second": throughput(replayed.len(), replay_micros)
        },
        "retention_compaction": {
            "elapsed_micros": prune_micros,
            "deleted_segments": deleted.len(),
            "segments_before": segments_before_prune,
            "segments_after": segments_after_prune,
            "bytes_before": bytes_before_prune,
            "bytes_after": bytes_after_prune
        },
        "disk": {
            "bytes_per_event_before_prune": ratio(bytes_before_prune, event_count),
            "bytes_per_event_after_prune": ratio(bytes_after_prune, event_count)
        }
    });
    println!("{}", serde_json::to_string_pretty(&result)?);
    fs::remove_dir_all(&root)?;
    Ok(())
}

fn fixture_event(index: usize, payload_bytes: usize) -> TraverseEvent {
    TraverseEvent {
        id: format!("journal-benchmark-{index:08}"),
        source: "traverse-runtime/journal-benchmark".to_string(),
        event_type: "dev.traverse.journal.benchmark".to_string(),
        datacontenttype: "application/json".to_string(),
        time: "2026-07-15T00:00:00Z".to_string(),
        data: json!({"sequence": index, "payload": "x".repeat(payload_bytes)}),
        owner: "traverse-runtime".to_string(),
        version: "1.0.0".to_string(),
        lifecycle_status: LifecycleStatus::Active,
        subject_id: Some(format!("benchmark-subject-{}", index % 32)),
        actor_id: None,
    }
}

fn env_usize(name: &str, default: usize) -> Result<usize, Box<dyn std::error::Error>> {
    match std::env::var(name) {
        Ok(value) => Ok(value.parse::<usize>()?),
        Err(std::env::VarError::NotPresent) => Ok(default),
        Err(error) => Err(Box::new(error)),
    }
}

fn benchmark_root() -> Result<PathBuf, std::time::SystemTimeError> {
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let base = std::env::var_os("TRAVERSE_JOURNAL_BENCH_ROOT")
        .map_or_else(std::env::temp_dir, PathBuf::from);
    Ok(base.join(format!("traverse-journal-benchmark-{nonce}")))
}

fn directory_bytes(root: &Path) -> Result<u64, std::io::Error> {
    fs::read_dir(root)?.try_fold(0_u64, |total, entry| {
        let entry = entry?;
        Ok(total + entry.metadata()?.len())
    })
}

fn segment_count(root: &Path) -> Result<usize, std::io::Error> {
    Ok(fs::read_dir(root)?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".jsonl"))
        .count())
}

fn percentile(sorted: &[u128], percentile: usize) -> u128 {
    if sorted.is_empty() {
        return 0;
    }
    let index = (sorted.len() - 1) * percentile / 100;
    sorted[index]
}

fn throughput(events: usize, elapsed_micros: u128) -> u128 {
    if elapsed_micros == 0 {
        return events as u128;
    }
    events as u128 * 1_000_000 / elapsed_micros
}

fn ratio(bytes: u64, events: usize) -> u64 {
    bytes / u64::try_from(events.max(1)).unwrap_or(u64::MAX)
}
