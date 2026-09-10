import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import test from "node:test";
import { browserLocalPlan, BrowserPlanError } from "../dist/index.js";

const stable = (value) => Array.isArray(value) ? `[${value.map(stable).join(",")}]` : value && typeof value === "object" ? `{${Object.keys(value).sort().map((key) => `${JSON.stringify(key)}:${stable(value[key])}`).join(",")}}` : JSON.stringify(value);
const sha = (value) => `sha256:${createHash("sha256").update(stable(value)).digest("hex")}`;
const digest = (marker) => `sha256:${marker.repeat(64).slice(0, 64)}`;
const contract = (id, inputs, outputs) => ({ schema_version: "1.0.0", id, inputs: { schema: { required: inputs, properties: Object.fromEntries(inputs.map(key => [key, { type: "string" }])) } }, outputs: { schema: { required: outputs, properties: Object.fromEntries(outputs.map(key => [key, { type: "string" }])) } }, emits: [] });

function inputs() {
  const snapshot = { releaseTag: "registry-v1", capabilities: [{ namespace: "demo", id: "source", version: "1.0.0", digest: digest("a"), artifactUrl: "", contractDigest: "", contractUrl: "", deprecated: false }, { namespace: "demo", id: "sink", version: "1.0.0", digest: digest("b"), artifactUrl: "", contractDigest: "", contractUrl: "", deprecated: false }] };
  const identity = { registry_snapshot_digest: sha(snapshot), source_release: snapshot.releaseTag, contract_schema_version: "1.0.0" };
  const dependency = (id, marker, value) => ({ wasmBytes: new Uint8Array(), contractBytes: new TextEncoder().encode(JSON.stringify(value)), wasmDigest: digest(marker), evidence: { namespace: "demo", id, selectedVersion: "1.0.0", versionRange: "1.0.0", sourceRelease: "registry-v1", indexDigest: identity.registry_snapshot_digest, artifactDigest: digest(marker), verifiedAt: 1, outcome: "prepared" } });
  return { snapshot, identity, dependencies: [dependency("source", "a", contract("source", ["seed"], ["middle"])), dependency("sink", "b", contract("sink", ["middle"], ["result"]))] };
}

test("browser planner is deterministic, structural, and leaves mappings unconfirmed", async () => {
  const { snapshot, identity, dependencies } = inputs();
  const args = [identity, snapshot, dependencies, { capability_id: "sink", capability_version: "1.0.0" }, { seed: "x" }, "local", { app_id: "demo" }];
  const first = await browserLocalPlan(...args);
  const second = await browserLocalPlan(...args);
  assert.deepEqual(first, second);
  assert.equal(first.proposals.length, 1);
  assert.equal(first.proposals[0].mapping_unconfirmed, true);
  assert.deepEqual(first.proposals[0].proposal.nodes.map(node => node.capability_id), ["source", "sink"]);
});

test("browser planner keeps forwarding intermediate nodes in end-to-end chains", async () => {
  // Regression for #1338 / registry#441: an intermediate node that both
  // consumes and forwards a field (inputs ∩ outputs ≠ ∅) must remain on the
  // planned path. The old TS visit() folded predecessor outputs into
  // `available`, so the intermediate looked like a valid chain head and the
  // upstream edge was dropped.
  const nodes = [
    { id: "collect", inputs: ["fragments"], outputs: ["fragments", "structured_facts"] },
    { id: "enrich", inputs: ["fragments", "structured_facts"], outputs: ["fragments", "structured_facts", "insights"] },
    { id: "summarize", inputs: ["structured_facts", "insights"], outputs: ["structured_facts", "summary"] },
    { id: "format", inputs: ["summary"], outputs: ["report"] },
  ];
  const snapshot = {
    releaseTag: "registry-v1",
    capabilities: nodes.map((node, index) => ({
      namespace: "report",
      id: node.id,
      version: "1.0.0",
      digest: digest(String.fromCharCode(97 + index)),
      artifactUrl: "",
      contractDigest: "",
      contractUrl: "",
      deprecated: false,
    })),
  };
  const identity = {
    registry_snapshot_digest: sha(snapshot),
    source_release: snapshot.releaseTag,
    contract_schema_version: "1.0.0",
  };
  const dependencies = nodes.map((node, index) => ({
    wasmBytes: new Uint8Array(),
    contractBytes: new TextEncoder().encode(JSON.stringify(contract(node.id, node.inputs, node.outputs))),
    wasmDigest: digest(String.fromCharCode(97 + index)),
    evidence: {
      namespace: "report",
      id: node.id,
      selectedVersion: "1.0.0",
      versionRange: "1.0.0",
      sourceRelease: "registry-v1",
      indexDigest: identity.registry_snapshot_digest,
      artifactDigest: digest(String.fromCharCode(97 + index)),
      verifiedAt: 1,
      outcome: "prepared",
    },
  }));
  const result = await browserLocalPlan(
    identity,
    snapshot,
    dependencies,
    { capability_id: "format", capability_version: "1.0.0" },
    { fragments: "[]" },
    "local",
    { app_id: "report" },
  );
  const paths = result.proposals.map((proposal) =>
    proposal.proposal.nodes.map((node) => node.capability_id),
  );
  assert.ok(
    paths.some((path) =>
      path.length === 4
      && path[0] === "collect"
      && path[1] === "enrich"
      && path[2] === "summarize"
      && path[3] === "format",
    ),
    `expected collect→enrich→summarize→format among proposals, got ${JSON.stringify(paths)}`,
  );
});

test("browser planner fails closed before planning on altered snapshot evidence", async () => {
  const { snapshot, identity, dependencies } = inputs();
  await assert.rejects(() => browserLocalPlan({ ...identity, registry_snapshot_digest: digest("z") }, snapshot, dependencies, { capability_id: "sink", capability_version: "1.0.0" }, {}, "local", {}), (error) => error instanceof BrowserPlanError && error.code === "browser_plan_snapshot_digest_mismatch");
});
