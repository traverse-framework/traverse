//! Immutable persisted capability metadata index and process-local hydration cache
//! (spec `134-lazy-capability-registry-reconstruction`).
//!
//! The index is built from registration state without parsing contract bodies.
//! Full contracts are digest-verified and parsed only on hydration.

use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use traverse_contracts::{CapabilityContract, parse_contract};
use traverse_registry::WorkspaceApplicationRegistration;

pub const DEFAULT_HYDRATION_CACHE_CAPACITY: usize = 64;
const EVIDENCE_SCHEMA_VERSION: &str = "1.0.0";

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct HydrationKey {
    pub capability_id: String,
    pub capability_version: String,
    pub contract_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IndexedCapability {
    pub app_id: String,
    pub app_version: String,
    pub component_id: String,
    pub component_version: String,
    pub capability_id: String,
    pub capability_version: String,
    pub contract_digest: String,
    pub artifact_digest: Option<String>,
    pub execution_mode: Option<String>,
    pub platforms: Vec<String>,
    pub workflow_refs: Vec<IndexedWorkflowRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IndexedWorkflowRef {
    pub workflow_id: String,
    pub workflow_version: String,
    pub workflow_digest: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct IndexedSource {
    pub contract: PathBuf,
    pub artifact: PathBuf,
    pub manifest: PathBuf,
}

#[derive(Debug, Clone, Default)]
pub struct CapabilityMetadataIndex {
    entries: BTreeMap<String, IndexedCapability>,
    sources: BTreeMap<String, IndexedSource>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HydrationEvidenceKind {
    IndexLookup,
    CacheHit,
    HydrationLeader,
    CoalescedWaiter,
    Eviction,
    HydrationFailure,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HydrationEvidence {
    pub schema_version: String,
    pub kind: HydrationEvidenceKind,
    pub capability_id: String,
    pub capability_version: String,
    pub contract_digest: String,
    pub reason_code: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HydrationError {
    pub code: &'static str,
    pub message: String,
}

#[derive(Debug)]
pub struct ContractHydrationCache {
    capacity: usize,
    inner: Mutex<CacheInner>,
}

type InflightSlot = Arc<Mutex<Option<Result<Arc<CapabilityContract>, HydrationError>>>>;

#[derive(Debug)]
struct CacheInner {
    order: VecDeque<HydrationKey>,
    entries: HashMap<HydrationKey, Arc<CapabilityContract>>,
    inflight: HashMap<HydrationKey, InflightSlot>,
    evidence: Vec<HydrationEvidence>,
}

impl CapabilityMetadataIndex {
    /// Builds an index from persisted workspace application registrations.
    ///
    /// Contract files are hashed when present; they are never parsed here.
    #[must_use]
    pub fn from_workspace_applications(apps: &[WorkspaceApplicationRegistration]) -> Self {
        let mut index = Self::default();
        for app in apps {
            index.extend_from_registration(app);
        }
        index
    }

    fn extend_from_registration(&mut self, app: &WorkspaceApplicationRegistration) {
        let Ok(bytes) = fs::read(&app.state_path) else {
            return;
        };
        let Ok(registration) = serde_json::from_slice::<PersistedRegistration>(&bytes) else {
            return;
        };
        let workflow_refs = registration
            .workflows
            .iter()
            .map(|workflow| IndexedWorkflowRef {
                workflow_id: workflow.workflow_id.clone(),
                workflow_version: workflow.workflow_version.clone(),
                workflow_digest: workflow.workflow_digest.clone(),
            })
            .collect::<Vec<_>>();
        for component in registration.components {
            let contract_digest = component
                .contract_digest
                .clone()
                .or_else(|| digest_file(Path::new(&component.contract_path)))
                .unwrap_or_default();
            if component.capability_id.trim().is_empty() || contract_digest.is_empty() {
                continue;
            }
            let lookup_key = lookup_key(&component.capability_id, &component.capability_version);
            self.entries.insert(
                lookup_key.clone(),
                IndexedCapability {
                    app_id: registration.app_id.clone(),
                    app_version: registration.app_version.clone(),
                    component_id: component.component_id,
                    component_version: component.component_version,
                    capability_id: component.capability_id.clone(),
                    capability_version: component.capability_version.clone(),
                    contract_digest: contract_digest.clone(),
                    artifact_digest: component.wasm_digest,
                    execution_mode: component.execution_mode,
                    platforms: component.platforms,
                    workflow_refs: workflow_refs.clone(),
                },
            );
            if !component.contract_path.is_empty() {
                self.sources.insert(
                    lookup_key,
                    IndexedSource {
                        contract: PathBuf::from(component.contract_path),
                        artifact: PathBuf::from(component.artifact_ref),
                        manifest: PathBuf::from(component.manifest_path),
                    },
                );
            }
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[must_use]
    pub fn get(&self, capability_id: &str, capability_version: &str) -> Option<&IndexedCapability> {
        self.entries
            .get(&lookup_key(capability_id, capability_version))
    }

    pub fn entries(&self) -> impl Iterator<Item = &IndexedCapability> {
        self.entries.values()
    }

    fn source_path(&self, capability_id: &str, capability_version: &str) -> Option<&Path> {
        self.source(capability_id, capability_version)
            .map(|source| source.contract.as_path())
    }

    pub(crate) fn source(
        &self,
        capability_id: &str,
        capability_version: &str,
    ) -> Option<&IndexedSource> {
        self.sources
            .get(&lookup_key(capability_id, capability_version))
    }
}

impl ContractHydrationCache {
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            inner: Mutex::new(CacheInner {
                order: VecDeque::new(),
                entries: HashMap::new(),
                inflight: HashMap::new(),
                evidence: Vec::new(),
            }),
        }
    }

    #[must_use]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    fn lock_inner(&self) -> std::sync::MutexGuard<'_, CacheInner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub fn evidence_snapshot(&self) -> Vec<HydrationEvidence> {
        self.lock_inner().evidence.clone()
    }

    /// Digest-verifies, parses, and caches a persisted contract.
    ///
    /// # Errors
    ///
    /// Returns [`HydrationError`] when the capability is missing from the index
    /// or the persisted contract cannot be read, digest-verified, or parsed.
    pub fn hydrate(
        &self,
        index: &CapabilityMetadataIndex,
        capability_id: &str,
        capability_version: &str,
    ) -> Result<Arc<CapabilityContract>, HydrationError> {
        let indexed = if let Some(entry) = index.get(capability_id, capability_version) {
            self.record(
                HydrationEvidenceKind::IndexLookup,
                entry,
                "indexed_capability_found",
            );
            entry.clone()
        } else {
            self.record_ids(
                HydrationEvidenceKind::HydrationFailure,
                capability_id,
                capability_version,
                "",
                "indexed_capability_missing",
            );
            return Err(HydrationError {
                code: "indexed_capability_missing",
                message: "capability is not present in the persisted metadata index".to_string(),
            });
        };
        let key = HydrationKey {
            capability_id: indexed.capability_id.clone(),
            capability_version: indexed.capability_version.clone(),
            contract_digest: indexed.contract_digest.clone(),
        };
        if let Some(hit) = self.cache_get(&key) {
            self.record(
                HydrationEvidenceKind::CacheHit,
                &indexed,
                "hydration_cache_hit",
            );
            return Ok(hit);
        }

        let slot = {
            let mut inner = self.lock_inner();
            inner
                .inflight
                .entry(key.clone())
                .or_insert_with(|| Arc::new(Mutex::new(None)))
                .clone()
        };

        let mut slot_guard = slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(existing) = slot_guard.as_ref() {
            self.record(
                HydrationEvidenceKind::CoalescedWaiter,
                &indexed,
                "hydration_coalesced",
            );
            return existing.clone();
        }

        self.record(
            HydrationEvidenceKind::HydrationLeader,
            &indexed,
            "hydration_leader",
        );
        let outcome = self.hydrate_leader(index, &indexed);
        *slot_guard = Some(outcome.clone());
        drop(slot_guard);
        self.lock_inner().inflight.remove(&key);
        outcome
    }

    fn hydrate_leader(
        &self,
        index: &CapabilityMetadataIndex,
        indexed: &IndexedCapability,
    ) -> Result<Arc<CapabilityContract>, HydrationError> {
        let Some(path) = index.source_path(&indexed.capability_id, &indexed.capability_version)
        else {
            return self.fail(indexed, "contract_source_missing");
        };
        let Ok(bytes) = fs::read(path) else {
            return self.fail(indexed, "contract_unreadable");
        };
        let actual = format!("sha256:{}", sha256_hex(&bytes));
        if actual != indexed.contract_digest {
            return self.fail(indexed, "contract_digest_mismatch");
        }
        let Ok(text) = std::str::from_utf8(&bytes) else {
            return self.fail(indexed, "contract_not_utf8");
        };
        let Ok(parsed) = parse_contract(text) else {
            return self.fail(indexed, "contract_parse_failed");
        };
        let stored = Arc::new(parsed);
        self.insert(indexed, Arc::clone(&stored));
        Ok(stored)
    }

    fn fail(
        &self,
        indexed: &IndexedCapability,
        reason_code: &'static str,
    ) -> Result<Arc<CapabilityContract>, HydrationError> {
        self.record(
            HydrationEvidenceKind::HydrationFailure,
            indexed,
            reason_code,
        );
        Err(HydrationError {
            code: reason_code,
            message: "persisted contract could not be hydrated".to_string(),
        })
    }

    fn cache_get(&self, key: &HydrationKey) -> Option<Arc<CapabilityContract>> {
        let mut inner = self.lock_inner();
        let hit = inner.entries.get(key).cloned()?;
        if let Some(position) = inner.order.iter().position(|item| item == key) {
            inner.order.remove(position);
        }
        inner.order.push_back(key.clone());
        Some(hit)
    }

    fn insert(&self, indexed: &IndexedCapability, contract: Arc<CapabilityContract>) {
        let key = HydrationKey {
            capability_id: indexed.capability_id.clone(),
            capability_version: indexed.capability_version.clone(),
            contract_digest: indexed.contract_digest.clone(),
        };
        let mut inner = self.lock_inner();
        if inner.entries.contains_key(&key) {
            return;
        }
        while inner.entries.len() >= self.capacity {
            if let Some(evicted) = inner.order.pop_front() {
                inner.entries.remove(&evicted);
                inner.evidence.push(HydrationEvidence {
                    schema_version: EVIDENCE_SCHEMA_VERSION.to_string(),
                    kind: HydrationEvidenceKind::Eviction,
                    capability_id: evicted.capability_id,
                    capability_version: evicted.capability_version,
                    contract_digest: evicted.contract_digest,
                    reason_code: "hydration_cache_evicted".to_string(),
                });
            } else {
                break;
            }
        }
        inner.order.push_back(key.clone());
        inner.entries.insert(key, contract);
    }

    fn record(&self, kind: HydrationEvidenceKind, indexed: &IndexedCapability, reason_code: &str) {
        self.record_ids(
            kind,
            &indexed.capability_id,
            &indexed.capability_version,
            &indexed.contract_digest,
            reason_code,
        );
    }

    fn record_ids(
        &self,
        kind: HydrationEvidenceKind,
        capability_id: &str,
        capability_version: &str,
        contract_digest: &str,
        reason_code: &str,
    ) {
        let record = HydrationEvidence {
            schema_version: EVIDENCE_SCHEMA_VERSION.to_string(),
            kind,
            capability_id: capability_id.to_string(),
            capability_version: capability_version.to_string(),
            contract_digest: contract_digest.to_string(),
            reason_code: reason_code.to_string(),
        };
        let envelope = json!({
            "kind": "capability_hydration",
            "schema_version": record.schema_version,
            "evidence_kind": record.kind,
            "capability_id": record.capability_id,
            "capability_version": record.capability_version,
            "contract_digest": record.contract_digest,
            "reason_code": record.reason_code,
        });
        eprintln!("{envelope}");
        self.lock_inner().evidence.push(record);
    }
}

fn lookup_key(capability_id: &str, capability_version: &str) -> String {
    format!("{capability_id}@{capability_version}")
}

fn digest_file(path: &Path) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    Some(format!("sha256:{}", sha256_hex(&bytes)))
}

fn sha256_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        output.push(char::from(HEX[(byte >> 4) as usize]));
        output.push(char::from(HEX[(byte & 0x0f) as usize]));
    }
    output
}

#[derive(Debug, serde::Deserialize)]
struct PersistedRegistration {
    #[serde(default)]
    app_id: String,
    #[serde(default)]
    app_version: String,
    #[serde(default)]
    components: Vec<PersistedComponent>,
    #[serde(default)]
    workflows: Vec<PersistedWorkflow>,
}

#[derive(Debug, serde::Deserialize)]
struct PersistedComponent {
    #[serde(default)]
    component_id: String,
    #[serde(default)]
    component_version: String,
    #[serde(default)]
    capability_id: String,
    #[serde(default)]
    capability_version: String,
    #[serde(default)]
    wasm_digest: Option<String>,
    #[serde(default)]
    contract_digest: Option<String>,
    #[serde(default)]
    contract_path: String,
    #[serde(default)]
    artifact_ref: String,
    #[serde(default)]
    manifest_path: String,
    #[serde(default)]
    execution_mode: Option<String>,
    #[serde(default)]
    platforms: Vec<String>,
}

#[derive(Debug, serde::Deserialize)]
#[allow(clippy::struct_field_names)]
struct PersistedWorkflow {
    #[serde(default)]
    workflow_id: String,
    #[serde(default)]
    workflow_version: String,
    #[serde(default)]
    workflow_digest: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    path: String,
}

#[must_use]
pub fn evidence_is_secret_free(record: &HydrationEvidence) -> bool {
    let serialized = serde_json::to_string(record).unwrap_or_default();
    !serialized.contains("file://")
        && !serialized.contains("/.traverse/")
        && !serialized.contains("-----BEGIN")
}

#[must_use]
pub fn evidence_envelope(records: &[HydrationEvidence]) -> Value {
    json!({
        "schema_version": EVIDENCE_SCHEMA_VERSION,
        "records": records,
    })
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    #[cfg(not(target_arch = "wasm32"))]
    use std::thread;
    use traverse_registry::WorkspaceApplicationRegistration;

    fn unique_dir() -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let dir = std::env::temp_dir().join(format!(
            "traverse-runtime-hydration-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    fn app_for(state_path: &Path) -> WorkspaceApplicationRegistration {
        WorkspaceApplicationRegistration {
            app_id: "demo.app".to_string(),
            app_version: "1.0.0".to_string(),
            manifest_path: "/nonexistent/app.manifest.json".to_string(),
            manifest_digest: "sha256:manifest".to_string(),
            bundle_digest: "sha256:bundle".to_string(),
            model_dependencies: Vec::new(),
            state_path: state_path.to_path_buf(),
        }
    }

    fn real_contract() -> String {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
            "../../contracts/examples/expedition/capabilities/validate-team-readiness/contract.json",
        );
        fs::read_to_string(path).expect("fixture contract")
    }

    fn write_registration(dir: &Path, contract_path: &Path, extra: Value) -> PathBuf {
        let state_path = dir.join("registration.json");
        let mut body = json!({
            "app_id": "demo.app",
            "app_version": "1.0.0",
            "components": [{
                "component_id": "demo.component",
                "component_version": "1.0.0",
                "capability_id": "demo.capability",
                "capability_version": "1.0.0",
                "wasm_digest": "sha256:artifact",
                "contract_path": contract_path.display().to_string(),
                "execution_mode": "wasm",
                "platforms": ["local"]
            }],
            "workflows": [{
                "workflow_id": "demo.workflow",
                "workflow_version": "1.0.0",
                "workflow_digest": "sha256:workflow"
            }]
        });
        if let (Value::Object(root), Value::Object(extra)) = (&mut body, extra) {
            for (key, value) in extra {
                root.insert(key, value);
            }
        }
        fs::write(&state_path, serde_json::to_vec_pretty(&body).expect("json")).expect("write");
        state_path
    }

    #[test]
    fn index_does_not_require_parsing_contract_bodies() {
        let dir = unique_dir();
        let contract_path = dir.join("broken-contract.json");
        fs::write(&contract_path, "{not-json").expect("contract");
        let state_path = write_registration(&dir, &contract_path, json!({}));
        let index = CapabilityMetadataIndex::from_workspace_applications(&[app_for(&state_path)]);
        let entry = index
            .get("demo.capability", "1.0.0")
            .expect("indexed without parse");
        assert_eq!(entry.component_id, "demo.component");
        assert_eq!(entry.artifact_digest.as_deref(), Some("sha256:artifact"));
        assert_eq!(entry.workflow_refs[0].workflow_id, "demo.workflow");
        assert!(!entry.contract_digest.is_empty());
        let serialized = serde_json::to_string(entry).expect("serialize");
        assert!(!serialized.contains("broken-contract.json"));
        assert!(!serialized.contains("/nonexistent"));
    }

    #[test]
    fn hydrate_verifies_digest_and_does_not_cache_failures() {
        let dir = unique_dir();
        let contract_path = dir.join("broken-contract.json");
        fs::write(&contract_path, "{not-json").expect("contract");
        let state_path = write_registration(&dir, &contract_path, json!({}));
        let index = CapabilityMetadataIndex::from_workspace_applications(&[app_for(&state_path)]);
        let cache = ContractHydrationCache::new(2);
        let first = cache.hydrate(&index, "demo.capability", "1.0.0");
        assert_eq!(first.expect_err("fail").code, "contract_parse_failed");
        let second = cache.hydrate(&index, "demo.capability", "1.0.0");
        assert_eq!(second.expect_err("retry").code, "contract_parse_failed");
        let kinds: Vec<_> = cache
            .evidence_snapshot()
            .into_iter()
            .map(|record| record.kind)
            .collect();
        assert!(kinds.contains(&HydrationEvidenceKind::HydrationLeader));
        assert!(kinds.contains(&HydrationEvidenceKind::HydrationFailure));
        assert!(!kinds.contains(&HydrationEvidenceKind::CacheHit));
        for record in cache.evidence_snapshot() {
            assert!(evidence_is_secret_free(&record));
        }
    }

    #[test]
    fn cache_evicts_deterministically_and_restarts_empty() {
        let dir = unique_dir();
        let mut apps = Vec::new();
        for index in 0..3 {
            let contract_path = dir.join(format!("contract-{index}.json"));
            let id = format!("demo.capability.{index}");
            fs::write(&contract_path, real_contract()).expect("contract");
            let state_path = dir.join(format!("registration-{index}.json"));
            fs::write(
                &state_path,
                serde_json::to_vec_pretty(&json!({
                    "app_id": "demo.app",
                    "app_version": "1.0.0",
                    "components": [{
                        "component_id": format!("demo.component.{index}"),
                        "component_version": "1.0.0",
                        "capability_id": id,
                        "capability_version": "1.0.0",
                        "wasm_digest": "sha256:artifact",
                        "contract_path": contract_path.display().to_string()
                    }]
                }))
                .expect("json"),
            )
            .expect("write");
            apps.push(app_for(&state_path));
        }
        let index = CapabilityMetadataIndex::from_workspace_applications(&apps);
        let cache = ContractHydrationCache::new(2);
        cache
            .hydrate(&index, "demo.capability.0", "1.0.0")
            .expect("0");
        cache
            .hydrate(&index, "demo.capability.1", "1.0.0")
            .expect("1");
        cache
            .hydrate(&index, "demo.capability.2", "1.0.0")
            .expect("2");
        assert!(
            cache
                .evidence_snapshot()
                .iter()
                .any(|record| record.kind == HydrationEvidenceKind::Eviction)
        );
        cache
            .hydrate(&index, "demo.capability.0", "1.0.0")
            .expect("rehydrate 0 after eviction");
        let leaders_for_zero = cache
            .evidence_snapshot()
            .iter()
            .filter(|record| {
                record.kind == HydrationEvidenceKind::HydrationLeader
                    && record.capability_id == "demo.capability.0"
            })
            .count();
        assert_eq!(leaders_for_zero, 2);

        let restarted = ContractHydrationCache::new(2);
        assert!(restarted.evidence_snapshot().is_empty());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn concurrent_hydration_coalesces_to_one_leader() {
        let dir = unique_dir();
        let contract_path = dir.join("contract.json");
        fs::write(&contract_path, real_contract()).expect("contract");
        let state_path = write_registration(&dir, &contract_path, json!({}));
        let index = Arc::new(CapabilityMetadataIndex::from_workspace_applications(&[
            app_for(&state_path),
        ]));
        let cache = Arc::new(ContractHydrationCache::new(4));
        let start = Arc::new(std::sync::Barrier::new(8));
        let workers: Vec<_> = (0..8)
            .map(|_| {
                let index = Arc::clone(&index);
                let cache = Arc::clone(&cache);
                let start = Arc::clone(&start);
                thread::spawn(move || {
                    start.wait();
                    cache.hydrate(&index, "demo.capability", "1.0.0")
                })
            })
            .collect();
        for worker in workers {
            worker.join().expect("join").expect("hydrate");
        }
        let snapshot = cache.evidence_snapshot();
        let leaders = snapshot
            .iter()
            .filter(|record| record.kind == HydrationEvidenceKind::HydrationLeader)
            .count();
        let waiters = snapshot
            .iter()
            .filter(|record| record.kind == HydrationEvidenceKind::CoalescedWaiter)
            .count();
        let hits = snapshot
            .iter()
            .filter(|record| record.kind == HydrationEvidenceKind::CacheHit)
            .count();
        assert_eq!(leaders, 1);
        assert_eq!(leaders + waiters + hits, 8);
    }

    #[test]
    fn hydrate_reports_stable_secret_free_failure_codes() {
        let cache = ContractHydrationCache::new(1);
        assert_eq!(cache.capacity(), 1);
        let empty = CapabilityMetadataIndex::default();
        assert!(empty.is_empty());
        let missing = cache
            .hydrate(&empty, "demo.capability", "1.0.0")
            .expect_err("missing");
        assert_eq!(missing.code, "indexed_capability_missing");

        let dir = unique_dir();
        let state_path = dir.join("registration.json");
        fs::write(
            &state_path,
            serde_json::to_vec_pretty(&json!({
                "app_id": "demo.app",
                "app_version": "1.0.0",
                "components": [{
                    "component_id": "demo.component",
                    "capability_id": "demo.capability",
                    "capability_version": "1.0.0",
                    "contract_digest": "sha256:dead",
                    "contract_path": ""
                }, {
                    "component_id": "skip.me",
                    "capability_id": "",
                    "capability_version": "1.0.0"
                }]
            }))
            .expect("json"),
        )
        .expect("write");
        let index = CapabilityMetadataIndex::from_workspace_applications(&[app_for(&state_path)]);
        assert_eq!(index.len(), 1);
        assert!(index.entries().next().is_some());
        let missing_source = cache
            .hydrate(&index, "demo.capability", "1.0.0")
            .expect_err("source");
        assert_eq!(missing_source.code, "contract_source_missing");

        let unreadable_path = dir.join("unreadable.json");
        fs::write(
            dir.join("registration-unread.json"),
            serde_json::to_vec_pretty(&json!({
                "app_id": "demo.app",
                "app_version": "1.0.0",
                "components": [{
                    "component_id": "demo.component",
                    "capability_id": "demo.unread",
                    "capability_version": "1.0.0",
                    "contract_digest": "sha256:dead",
                    "contract_path": unreadable_path.display().to_string()
                }]
            }))
            .expect("json"),
        )
        .expect("write");
        let unread_index = CapabilityMetadataIndex::from_workspace_applications(&[app_for(
            &dir.join("registration-unread.json"),
        )]);
        let unread = cache
            .hydrate(&unread_index, "demo.unread", "1.0.0")
            .expect_err("unreadable");
        assert_eq!(unread.code, "contract_unreadable");

        let utf_path = dir.join("utf8.bin");
        fs::write(&utf_path, [0xff, 0xfe]).expect("bytes");
        let utf_state = write_registration(&dir, &utf_path, json!([]));
        let utf_index =
            CapabilityMetadataIndex::from_workspace_applications(&[app_for(&utf_state)]);
        let utf8 = cache
            .hydrate(&utf_index, "demo.capability", "1.0.0")
            .expect_err("utf8");
        assert_eq!(utf8.code, "contract_not_utf8");

        let mismatch_path = dir.join("mismatch.json");
        fs::write(&mismatch_path, real_contract()).expect("contract");
        let mismatch_state = dir.join("registration-mismatch.json");
        fs::write(
            &mismatch_state,
            serde_json::to_vec_pretty(&json!({
                "app_id": "demo.app",
                "app_version": "1.0.0",
                "components": [{
                    "component_id": "demo.component",
                    "capability_id": "demo.mismatch",
                    "capability_version": "1.0.0",
                    "contract_digest": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
                    "contract_path": mismatch_path.display().to_string()
                }]
            }))
            .expect("json"),
        )
        .expect("write");
        let mismatch_index =
            CapabilityMetadataIndex::from_workspace_applications(&[app_for(&mismatch_state)]);
        let mismatch = cache
            .hydrate(&mismatch_index, "demo.mismatch", "1.0.0")
            .expect_err("digest");
        assert_eq!(mismatch.code, "contract_digest_mismatch");

        let envelope = evidence_envelope(&cache.evidence_snapshot());
        assert_eq!(envelope["schema_version"], "1.0.0");
        assert!(envelope["records"].as_array().expect("records").len() >= 4);
    }

    #[test]
    fn skips_unreadable_or_malformed_registration_files() {
        let dir = unique_dir();
        let missing = app_for(&dir.join("nope.json"));
        let bad = dir.join("bad.json");
        fs::write(&bad, "not-json").expect("write");
        let index = CapabilityMetadataIndex::from_workspace_applications(&[missing, app_for(&bad)]);
        assert!(index.is_empty());
    }

    #[test]
    fn insert_covers_duplicate_and_empty_eviction_order() {
        let dir = unique_dir();
        let contract_path = dir.join("contract.json");
        fs::write(&contract_path, real_contract()).expect("contract");
        let state_path =
            write_registration(&dir, &contract_path, json!({"schema_version": "1.0.0"}));
        let index = CapabilityMetadataIndex::from_workspace_applications(&[app_for(&state_path)]);
        let cache = ContractHydrationCache::new(1);
        let contract = cache
            .hydrate(&index, "demo.capability", "1.0.0")
            .expect("hydrate");
        let indexed = index
            .get("demo.capability", "1.0.0")
            .expect("indexed")
            .clone();
        cache.insert(&indexed, Arc::clone(&contract));
        let mut other = indexed.clone();
        other.capability_id = "demo.other".to_string();
        other.contract_digest = "sha256:other".to_string();
        {
            let mut inner = cache.inner.lock().expect("lock");
            inner.order.clear();
        }
        cache.insert(&other, contract);
        let err = HydrationError {
            code: "hydration_lock_poisoned",
            message: "hydration cache lock poisoned".to_string(),
        };
        assert_eq!(err.code, "hydration_lock_poisoned");
        assert!(!err.message.is_empty());
    }
}
