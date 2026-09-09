import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import test from "node:test";
import { ComposedWorkflowError, MemoryRegistryCacheStore, executeBrowserComposedWorkflow, prepareRegistryDependency } from "../dist/index.js";
import { ECHO_WAT, compileWat } from "./fixtures.mjs";

const digest = (bytes) => `sha256:${createHash("sha256").update(bytes).digest("hex")}`;
const stable = (value) => Array.isArray(value) ? `[${value.map(stable).join(",")}]` : value && typeof value === "object" ? `{${Object.keys(value).sort().map(key => `${JSON.stringify(key)}:${stable(value[key])}`).join(",")}}` : JSON.stringify(value);

async function fixture() {
  const wasm = await compileWat(ECHO_WAT);
  const contract = Buffer.from(JSON.stringify({ id: "echo", version: "1.0.0", risk: { effect_class: "pure_read", determinism_class: "deterministic" } }));
  const record = { namespace: "demo", id: "echo", version: "1.0.0", digest: digest(wasm), artifactUrl: "wasm", contractDigest: digest(contract), contractUrl: "contract", deprecated: false };
  const snapshot = { releaseTag: "registry-v1", capabilities: [record] };
  const store = new MemoryRegistryCacheStore();
  await prepareRegistryDependency(store, snapshot, { namespace: "demo", id: "echo", versionRange: "1.0.0" }, { fetch(url) { return new Uint8Array(url === "wasm" ? wasm : contract); } });
  const proposal = { kind: "browser_workflow_proposal", schema_version: "1.0.0", snapshot_digest: digest(Buffer.from(stable(snapshot))), source_release: "registry-v1", mapping_unconfirmed: false, proposal: { kind: "workflow_proposal", schema_version: "1.0.0", proposal_id: "p1", workspace_id: "local", app_manifest: {}, nodes: [{ node_id: "one", capability_id: "echo", capability_version: "1.0.0", artifact_digest: record.digest }], edges: [], mappings: [], initial_input: { hello: "world" } } };
  return { store, snapshot, proposal };
}

test("reviewed composed proposal executes exact prepared WASM offline with a redacted trace", async () => {
  const { store, snapshot, proposal } = await fixture();
  const trace = await executeBrowserComposedWorkflow(proposal, store, snapshot);
  assert.equal(trace.terminal_state, "succeeded");
  assert.deepEqual(trace.node_outcomes.map(outcome => outcome.status), ["succeeded"]);
  assert.equal(JSON.stringify(trace).includes("world"), false);
});

test("composed execution fails closed on artifact drift and unreviewed mappings", async () => {
  const { store, snapshot, proposal } = await fixture();
  await assert.rejects(() => executeBrowserComposedWorkflow({ ...proposal, mapping_unconfirmed: true }, store, snapshot), error => error instanceof ComposedWorkflowError && error.code === "composed_workflow_proposal_invalid");
  await assert.rejects(() => executeBrowserComposedWorkflow({ ...proposal, proposal: { ...proposal.proposal, nodes: [{ ...proposal.proposal.nodes[0], artifact_digest: digest(Buffer.from("wrong")) }] } }, store, snapshot), error => error instanceof ComposedWorkflowError && error.code === "composed_workflow_artifact_digest_drift");
});
