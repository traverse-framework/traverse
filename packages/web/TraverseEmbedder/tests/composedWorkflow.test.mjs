import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { ComposedWorkflowError, MemoryRegistryCacheStore, executeBrowserComposedWorkflow, prepareRegistryDependency } from "../dist/index.js";
import { ECHO_WAT, compileWat, emitEventWat } from "./fixtures.mjs";

const digest = (bytes) => `sha256:${createHash("sha256").update(bytes).digest("hex")}`;
const stable = (value) => Array.isArray(value) ? `[${value.map(stable).join(",")}]` : value && typeof value === "object" ? `{${Object.keys(value).sort().map(key => `${JSON.stringify(key)}:${stable(value[key])}`).join(",")}}` : JSON.stringify(value);

const RUNTIME_WASM_BYTES = await readFile(
  join(dirname(fileURLToPath(import.meta.url)), "fixtures/runtime.wasm"),
);

async function fixture(overrides = {}) {
  const wasm = overrides.wasm ?? await compileWat(ECHO_WAT);
  const contractBody = {
    id: "echo",
    version: "1.0.0",
    risk: { effect_class: "pure_read", determinism_class: "deterministic" },
    ...overrides.contract,
  };
  const contract = Buffer.from(JSON.stringify(contractBody));
  const record = { namespace: "demo", id: "echo", version: "1.0.0", digest: digest(wasm), artifactUrl: "wasm", contractDigest: digest(contract), contractUrl: "contract", deprecated: false };
  const snapshot = { releaseTag: "registry-v1", capabilities: [record] };
  const store = new MemoryRegistryCacheStore();
  await prepareRegistryDependency(store, snapshot, { namespace: "demo", id: "echo", versionRange: "1.0.0" }, { fetch(url) { return new Uint8Array(url === "wasm" ? wasm : contract); } });
  const proposal = { kind: "browser_workflow_proposal", schema_version: "1.0.0", snapshot_digest: digest(Buffer.from(stable(snapshot))), source_release: "registry-v1", mapping_unconfirmed: false, proposal: { kind: "workflow_proposal", schema_version: "1.0.0", proposal_id: "p1", workspace_id: "local", app_manifest: {}, nodes: [{ node_id: "one", capability_id: "echo", capability_version: "1.0.0", artifact_digest: record.digest }], edges: [], mappings: [], initial_input: { hello: "world" } } };
  return { store, snapshot, proposal };
}

function withRuntime(options = {}) {
  return { runtimeWasmBytes: new Uint8Array(RUNTIME_WASM_BYTES), ...options };
}

test("reviewed composed proposal executes exact prepared WASM offline with a redacted trace", async () => {
  const { store, snapshot, proposal } = await fixture();
  const trace = await executeBrowserComposedWorkflow(proposal, store, snapshot, withRuntime());
  assert.equal(trace.terminal_state, "succeeded");
  assert.deepEqual(trace.node_outcomes.map(outcome => outcome.status), ["succeeded"]);
  assert.equal(JSON.stringify(trace).includes("world"), false);
});

test("composed execution fails closed on artifact drift and unreviewed mappings", async () => {
  const { store, snapshot, proposal } = await fixture();
  await assert.rejects(() => executeBrowserComposedWorkflow({ ...proposal, mapping_unconfirmed: true }, store, snapshot, withRuntime()), error => error instanceof ComposedWorkflowError && error.code === "composed_workflow_proposal_invalid");
  await assert.rejects(() => executeBrowserComposedWorkflow({ ...proposal, proposal: { ...proposal.proposal, nodes: [{ ...proposal.proposal.nodes[0], artifact_digest: digest(Buffer.from("wrong")) }] } }, store, snapshot, withRuntime()), error => error instanceof ComposedWorkflowError && error.code === "composed_workflow_artifact_digest_drift");
});

test("composed emit_event: declared Subscribable event reaches onCapabilityEvent", async () => {
  const wasm = await compileWat(emitEventWat());
  const { store, snapshot, proposal } = await fixture({
    wasm,
    contract: {
      service_type: "subscribable",
      emits: [{ event_id: "dev.traverse.test.emitted", version: "1.0.0" }],
    },
  });
  const accepted = [];
  const trace = await executeBrowserComposedWorkflow(proposal, store, snapshot, withRuntime({
    onCapabilityEvent: (event) => accepted.push(event),
  }));
  assert.equal(trace.terminal_state, "succeeded");
  assert.equal(accepted.length, 1);
  assert.equal(accepted[0].event_id, "dev.traverse.test.emitted");
  assert.deepEqual(accepted[0].payload, { n: 1 });
  assert.equal(accepted[0].node_id, "one");
});

test("composed emit_event: undeclared and non-Subscribable events do not invoke callback", async () => {
  const wasm = await compileWat(emitEventWat());
  for (const contract of [
    { service_type: "subscribable", emits: [{ event_id: "other.event", version: "1.0.0" }] },
    { service_type: "stateless", emits: [{ event_id: "dev.traverse.test.emitted", version: "1.0.0" }] },
  ]) {
    const { store, snapshot, proposal } = await fixture({ wasm, contract });
    const accepted = [];
    const trace = await executeBrowserComposedWorkflow(proposal, store, snapshot, withRuntime({
      onCapabilityEvent: (event) => accepted.push(event),
    }));
    assert.equal(trace.terminal_state, "succeeded");
    assert.equal(accepted.length, 0);
  }
});
