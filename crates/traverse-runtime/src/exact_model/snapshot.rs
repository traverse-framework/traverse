//! Guest ABI v3 pristine-snapshot reuse (Spec 138 0.12, Decision 110).
//!
//! A v3 guest exports `model_prepare`. The host runs it once, records the
//! guest's linear memory and exported mutable globals, and restores that
//! snapshot into a fresh instance on later calls. Registration first proves
//! that memory and those globals are the guest's only mutable state
//! (FR-053), so a restored instance is indistinguishable from a freshly
//! prepared one.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use wasmparser::{ExternalKind, Operator, Parser, Payload, ValType};

use super::ModelEngine;

/// Guest ABI v3 prepare export: `model_prepare() -> i32`, `0` = prepared.
pub const MODEL_PREPARE_EXPORT: &str = "model_prepare";

/// Value of an exported mutable numeric global. Floats keep their raw bits
/// so a restore is bit-exact (NaN payloads included).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GlobalValue {
    I32(i32),
    I64(i64),
    F32(u32),
    F64(u64),
}

/// Post-`model_prepare` guest state: full linear memory and every exported
/// mutable global, sorted by export name.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Snapshot {
    pub(super) memory: Vec<u8>,
    pub(super) globals: Vec<(String, GlobalValue)>,
}

impl Snapshot {
    /// Bytes charged against `max_snapshot_bytes`.
    fn bytes(&self) -> u64 {
        let globals: usize = self.globals.iter().map(|(name, _)| name.len() + 8).sum();
        u64::try_from(self.memory.len() + globals).unwrap_or(u64::MAX)
    }
}

/// Host-owned, in-memory snapshot cache keyed by package digest and engine,
/// bounded by `max_snapshot_bytes` with least-recently-used eviction
/// (FR-055). Never persisted and never exposed to applications.
#[derive(Debug, Default)]
pub(super) struct SnapshotCache {
    entries: HashMap<(String, ModelEngine), (Arc<Snapshot>, u64)>,
    tick: u64,
    total: u64,
}

impl SnapshotCache {
    /// The snapshot for `(digest, engine)`, marking it most recently used.
    pub(super) fn get(&mut self, digest: &str, engine: ModelEngine) -> Option<Arc<Snapshot>> {
        self.tick += 1;
        let tick = self.tick;
        self.entries
            .get_mut(&(digest.to_string(), engine))
            .map(|(snapshot, used)| {
                *used = tick;
                Arc::clone(snapshot)
            })
    }

    /// Store a snapshot, evicting least-recently-used entries until it fits
    /// `budget`. A snapshot larger than the whole budget is not stored; the
    /// next call then takes the fresh path again, which only costs time.
    pub(super) fn insert(
        &mut self,
        digest: &str,
        engine: ModelEngine,
        snapshot: Snapshot,
        budget: u64,
    ) {
        let bytes = snapshot.bytes();
        if bytes > budget {
            return;
        }
        self.remove(&(digest.to_string(), engine));
        let mut by_age: Vec<_> = self
            .entries
            .iter()
            .map(|(key, (_, used))| (*used, key.clone()))
            .collect();
        by_age.sort_by_key(|(used, _)| *used);
        for (_, key) in by_age {
            if self.total + bytes <= budget {
                break;
            }
            self.remove(&key);
        }
        self.tick += 1;
        self.total += bytes;
        self.entries.insert(
            (digest.to_string(), engine),
            (Arc::new(snapshot), self.tick),
        );
    }

    /// Drop every snapshot of a package (status change, failed digest
    /// re-check, unregister).
    pub(super) fn drop_package(&mut self, digest: &str) {
        let keys: Vec<_> = self
            .entries
            .keys()
            .filter(|(key, _)| key == digest)
            .cloned()
            .collect();
        for key in keys {
            self.remove(&key);
        }
    }

    /// Drop every snapshot (shutdown).
    pub(super) fn clear(&mut self) {
        self.entries.clear();
        self.total = 0;
    }

    /// Bytes currently held.
    #[cfg(test)]
    pub(super) fn total_bytes(&self) -> u64 {
        self.total
    }

    fn remove(&mut self, key: &(String, ModelEngine)) {
        if let Some((snapshot, _)) = self.entries.remove(key) {
            self.total -= snapshot.bytes();
        }
    }
}

/// FR-053: a v3 module's only mutable state must be its single exported
/// memory and its exported numeric mutable globals, it must export
/// `model_prepare`, and it must import nothing. Returns the failure message
/// (the caller maps it to `model_incompatible`).
pub(super) fn check_v3_module(wasm: &[u8]) -> Result<(), &'static str> {
    const MALFORMED: &str = "abi_version 3 model wasm failed validation";
    let mut mutable_globals: Vec<(u32, bool)> = Vec::new();
    let mut global_index = 0_u32;
    let mut memories = 0_u32;
    let mut exported_memories = 0_u32;
    let mut exported_globals = HashSet::new();
    let mut prepare = false;
    for payload in Parser::new(0).parse_all(wasm) {
        match payload.map_err(|_| MALFORMED)? {
            Payload::ImportSection(imports) if imports.count() > 0 => {
                return Err("abi_version 3 model wasm must not import anything");
            }
            Payload::MemorySection(section) => memories += section.count(),
            Payload::GlobalSection(section) => {
                for global in section {
                    let ty = global.map_err(|_| MALFORMED)?.ty;
                    if ty.mutable {
                        let numeric = matches!(
                            ty.content_type,
                            ValType::I32 | ValType::I64 | ValType::F32 | ValType::F64
                        );
                        mutable_globals.push((global_index, numeric));
                    }
                    global_index += 1;
                }
            }
            Payload::ExportSection(section) => {
                for export in section {
                    let export = export.map_err(|_| MALFORMED)?;
                    match export.kind {
                        ExternalKind::Memory => exported_memories += 1,
                        ExternalKind::Global => {
                            exported_globals.insert(export.index);
                        }
                        ExternalKind::Func => prepare |= export.name == MODEL_PREPARE_EXPORT,
                        _ => {}
                    }
                }
            }
            Payload::CodeSectionEntry(body) => {
                let mut operators = body.get_operators_reader().map_err(|_| MALFORMED)?;
                while !operators.eof() {
                    if let Operator::TableSet { .. }
                    | Operator::TableGrow { .. }
                    | Operator::TableFill { .. }
                    | Operator::TableCopy { .. }
                    | Operator::TableInit { .. }
                    | Operator::ElemDrop { .. }
                    | Operator::DataDrop { .. } = operators.read().map_err(|_| MALFORMED)?
                    {
                        return Err(
                            "abi_version 3 model wasm uses a table- or segment-mutating instruction",
                        );
                    }
                }
            }
            _ => {}
        }
    }
    if memories != 1 || exported_memories != 1 {
        return Err("abi_version 3 model wasm must define and export exactly one memory");
    }
    if mutable_globals
        .iter()
        .any(|(index, numeric)| !numeric || !exported_globals.contains(index))
    {
        return Err("abi_version 3 model wasm has a non-exported or non-numeric mutable global");
    }
    if !prepare {
        return Err("abi_version 3 model wasm missing model_prepare export");
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    fn snapshot(bytes: usize) -> Snapshot {
        Snapshot {
            memory: vec![7; bytes],
            globals: vec![("g".to_string(), GlobalValue::I32(1))],
        }
    }

    #[test]
    fn cache_evicts_least_recently_used_within_budget() {
        let mut cache = SnapshotCache::default();
        let budget = 2 * (100 + 9);
        cache.insert("a", ModelEngine::Wasmi, snapshot(100), budget);
        cache.insert("b", ModelEngine::Wasmi, snapshot(100), budget);
        assert_eq!(cache.total_bytes(), budget);
        // Touch `a`, so inserting `c` evicts `b`.
        assert!(cache.get("a", ModelEngine::Wasmi).is_some());
        cache.insert("c", ModelEngine::Wasmi, snapshot(100), budget);
        assert!(cache.get("b", ModelEngine::Wasmi).is_none());
        assert!(cache.get("a", ModelEngine::Wasmi).is_some());
        assert!(cache.get("c", ModelEngine::Wasmi).is_some());
        // Engines are separate keys; replacing a key does not double-count.
        assert!(cache.get("a", ModelEngine::Wasmtime).is_none());
        cache.insert("a", ModelEngine::Wasmi, snapshot(100), budget);
        assert_eq!(cache.total_bytes(), budget);
        // Larger than the whole budget: not stored, nothing evicted.
        cache.insert("d", ModelEngine::Wasmi, snapshot(1000), budget);
        assert!(cache.get("d", ModelEngine::Wasmi).is_none());
        assert_eq!(cache.total_bytes(), budget);
        cache.insert("a", ModelEngine::Wasmtime, snapshot(10), budget * 2);
        cache.drop_package("a");
        assert!(cache.get("a", ModelEngine::Wasmi).is_none());
        assert!(cache.get("a", ModelEngine::Wasmtime).is_none());
        assert_eq!(cache.total_bytes(), 109);
        cache.clear();
        assert_eq!(cache.total_bytes(), 0);
        assert!(cache.get("c", ModelEngine::Wasmi).is_none());
    }

    fn module(body: &str) -> Vec<u8> {
        wat::parse_str(format!(
            r#"(module (memory (export "memory") 1) {body}
               (func (export "model_prepare") (result i32) i32.const 0))"#
        ))
        .expect("wat")
    }

    #[test]
    fn v3_module_check_accepts_only_snapshot_safe_guests() {
        assert_eq!(
            check_v3_module(&module(
                r#"(global (export "g") (mut i32) (i32.const 0)) (global i32 (i32.const 7))
                   (table (export "t") 1 funcref)"#
            )),
            Ok(())
        );
        let rejected = [
            module("(global (mut i32) (i32.const 0))"),
            module(r#"(global (export "g") (mut v128) (v128.const i64x2 0 0))"#),
            module("(table 1 funcref) (func (table.set 0 (i32.const 0) (ref.null func)))"),
            module(r#"(data "x") (func (data.drop 0))"#),
            wat::parse_str(
                r#"(module (memory 1) (func (export "model_prepare") (result i32) i32.const 0))"#,
            )
            .expect("wat"),
            wat::parse_str(r#"(module (memory (export "memory") 1))"#).expect("wat"),
            wat::parse_str(
                r#"(module (import "env" "f" (func)) (memory (export "memory") 1)
                   (func (export "model_prepare") (result i32) i32.const 0))"#,
            )
            .expect("wat"),
            b"\0asm\x01\0\0\0\x01".to_vec(),
        ];
        for (index, wasm) in rejected.iter().enumerate() {
            assert!(check_v3_module(wasm).is_err(), "case {index}");
        }
        // A function body that is not valid operator bytes is malformed.
        let mut broken = module("(func nop)");
        let nop = broken.iter().rposition(|byte| *byte == 0x01).expect("nop");
        broken[nop] = 0xff;
        assert!(check_v3_module(&broken).is_err());
    }
}
