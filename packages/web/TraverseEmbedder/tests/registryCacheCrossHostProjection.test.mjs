// Issue #1300: the Web registry-cache path must expose error/evidence
// projections equivalent to the Rust path for every spec
// `1258-offline-cache-activation` FR-004 failure category, and native adapter
// coverage is recorded against the spec 107 FR-009 equivalence set.
//
// Both hosts assert the same repository-controlled fixture
// (`fixtures/cross-host/registry-cache-projections/projection-matrix.json`); the
// Rust side lives in
// `crates/traverse-cli/src/registry_resolution_diagnostics.rs`. An inequality on
// either side is a cross-host conformance failure.
import test from "node:test";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import {
  MemoryRegistryCacheStore,
  RegistryCacheError,
  prepareRegistryDependency,
  resolveRegistryDependencyOffline,
} from "../dist/registryCache.js";

const fixtureUrl = new URL(
  "../../../../fixtures/cross-host/registry-cache-projections/projection-matrix.json",
  import.meta.url,
);

// Mirrors the `RegistryCacheErrorCode` union in `src/registryCache.ts`. The Web
// package emits stable codes only for these boundaries today.
const WEB_REGISTRY_CACHE_CODES = new Set([
  "registry_sync_missing",
  "registry_version_not_found",
  "registry_dependency_yanked",
  "registry_prepare_failed",
  "registry_artifact_digest_mismatch",
  "registry_cache_entry_missing",
]);

const EXPECTED_CATEGORIES = [
  "missing",
  "altered",
  "lifecycle-rejected",
  "signature-invalid",
  "abi-incompatible",
  "target-incompatible",
];

const REDACTION_PROBES = ["/", "\\", "://", "authorization", "bearer ", "secret", "token="];

async function loadMatrix() {
  return JSON.parse(await readFile(fixtureUrl, "utf8"));
}

function digestFor(bytes) {
  return `sha256:${createHash("sha256").update(bytes).digest("hex")}`;
}

test("cross-host projection matrix declares the spec 1258 / spec 107 evidence contract", async () => {
  const matrix = await loadMatrix();
  assert.equal(matrix.governing_spec, "1258-offline-cache-activation");
  assert.equal(matrix.conformance_matrix_spec, "107-cross-host-embedded-registry-cache");
  assert.equal(matrix.implementation_issue, 1272);
  assert.equal(matrix.evidence_issue, 1300);
  assert.deepEqual(
    matrix.categories.map((entry) => entry.category),
    EXPECTED_CATEGORIES,
  );
});

test("every canonical projection is redacted and key-sorted", async () => {
  const matrix = await loadMatrix();
  const allowed = new Set(matrix.allowed_evidence_fields);

  for (const entry of matrix.categories) {
    const evidence = entry.redacted_evidence;
    const keys = Object.keys(evidence);

    assert.equal(evidence.code, entry.code, `${entry.category}: code parity`);
    assert.equal(evidence.stage, entry.stage, `${entry.category}: stage parity`);

    for (const key of keys) {
      assert.ok(allowed.has(key), `${entry.category}: field ${key} is not permitted`);
    }
    assert.deepEqual(keys, [...keys].sort(), `${entry.category}: keys must be sorted`);

    const serialized = JSON.stringify(evidence).toLowerCase();
    for (const probe of REDACTION_PROBES) {
      assert.ok(
        !serialized.includes(probe),
        `${entry.category}: canonical projection leaked ${JSON.stringify(probe)}`,
      );
    }
  }
});

test("Web registry-cache codes align with the matrix for the boundaries it emits", async () => {
  const matrix = await loadMatrix();
  const byCategory = new Map(matrix.categories.map((entry) => [entry.category, entry]));

  // The Web package emits these two codes directly; they must match the matrix.
  assert.equal(byCategory.get("missing").code, "registry_cache_entry_missing");
  assert.equal(byCategory.get("altered").code, "registry_artifact_digest_mismatch");
  assert.ok(WEB_REGISTRY_CACHE_CODES.has(byCategory.get("missing").code));
  assert.ok(WEB_REGISTRY_CACHE_CODES.has(byCategory.get("altered").code));

  // The extended resolution-path boundaries are Rust/Web projections that the
  // Web package does not surface as distinct codes yet; the matrix records that
  // honestly rather than the test asserting a code the package cannot produce.
  for (const category of ["lifecycle-rejected", "signature-invalid", "abi-incompatible", "target-incompatible"]) {
    assert.ok(
      !WEB_REGISTRY_CACHE_CODES.has(byCategory.get(category).code),
      `${category}: not a Web six-code boundary`,
    );
  }
});

test("missing entry projects registry_cache_entry_missing from real Web resolution", async () => {
  const store = new MemoryRegistryCacheStore();
  const error = await resolveRegistryDependencyOffline(store, {
    namespace: "inference",
    id: "inference.evidence-normalize",
    versionRange: "==1.0.1",
  }).then(
    () => null,
    (caught) => caught,
  );
  assert.ok(error instanceof RegistryCacheError);
  assert.equal(error.code, "registry_cache_entry_missing");
});

test("altered cache entry projects registry_artifact_digest_mismatch from real Web preparation", async () => {
  const store = new MemoryRegistryCacheStore();
  const artifact = Buffer.from("verified-wasm-bytes");
  const contract = Buffer.from('{"kind":"capability_contract"}');
  const record = {
    namespace: "inference",
    id: "inference.evidence-normalize",
    version: "1.0.1",
    digest: digestFor(artifact),
    artifactUrl: "https://example.test/evidence-normalize.wasm",
    contractDigest: digestFor(contract),
    contractUrl: "https://example.test/evidence-normalize.json",
    deprecated: false,
  };
  const assets = new Map([
    [record.artifactUrl, new Uint8Array(artifact)],
    [record.contractUrl, new Uint8Array(contract)],
  ]);
  // The Web range matcher accepts bare/caret ranges; `==1.0.1` in the fixture is
  // the canonical redacted `requested_range` string, not a matcher input.
  const reference = { namespace: record.namespace, id: record.id, versionRange: "1.0.1" };
  const snapshot = { releaseTag: "index-v243", capabilities: [record] };
  const fetcher = { fetch: (url) => assets.get(url) };
  await prepareRegistryDependency(store, snapshot, reference, fetcher);

  // A different byte stream now sits under the immutable digest key; the
  // content-addressed cache must fail closed rather than accept it.
  const artifactHex = record.digest.slice("sha256:".length);
  store.set(`sha256/${artifactHex}`, new Uint8Array(Buffer.from("tampered-wasm-bytes")));

  const error = await prepareRegistryDependency(store, snapshot, reference, fetcher).then(
    () => null,
    (caught) => caught,
  );
  assert.ok(error instanceof RegistryCacheError);
  assert.equal(error.code, "registry_artifact_digest_mismatch");
});

test("native matrix records the spec 107 FR-009 equivalence set without widening it", async () => {
  const matrix = await loadMatrix();
  const native = matrix.native_matrix;
  assert.deepEqual([...native.spec_107_fr_009_equivalence_set].sort(), [
    "artifact_digest_mismatch",
    "missing_cache",
    "preparation_success",
    "yanked_dependency",
  ]);
  assert.deepEqual([...native.adapters].sort(), ["dotnet", "kotlin", "swift"]);

  for (const category of ["missing", "altered", "lifecycle-rejected"]) {
    assert.equal(
      typeof native.coverage[category].native_code,
      "string",
      `${category}: observable native code recorded`,
    );
  }
  for (const category of ["signature-invalid", "abi-incompatible", "target-incompatible"]) {
    assert.equal(native.coverage[category].status, "rust_web_projection_only");
    assert.equal(native.coverage[category].native_code, null);
  }
});
