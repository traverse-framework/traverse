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

// --- typed-contract fixtures (issue #1476) -------------------------------
// `props` maps a property name to its declared JSON type, or to `null` for a
// declared property carrying no `type` keyword at all. `required` defaults to
// every declared property.
const schemaOf = (props, required) => ({
  required: required ?? Object.keys(props),
  properties: Object.fromEntries(Object.entries(props).map(([name, type]) => [name, type === null ? { description: "no declared type" } : { type }])),
});
const typedContract = (id, inputs, outputs, options = {}) => ({
  schema_version: "1.0.0",
  id,
  inputs: { schema: schemaOf(inputs, options.inputRequired) },
  outputs: { schema: schemaOf(outputs, options.outputRequired) },
  emits: [],
});

function typedWorld(contracts) {
  const marker = (index) => digest(String.fromCharCode(97 + index));
  const snapshot = { releaseTag: "registry-v1", capabilities: contracts.map((value, index) => ({ namespace: "demo", id: value.id, version: "1.0.0", digest: marker(index), artifactUrl: "", contractDigest: "", contractUrl: "", deprecated: false })) };
  const identity = { registry_snapshot_digest: sha(snapshot), source_release: snapshot.releaseTag, contract_schema_version: "1.0.0" };
  const dependencies = contracts.map((value, index) => ({ wasmBytes: new Uint8Array(), contractBytes: new TextEncoder().encode(JSON.stringify(value)), wasmDigest: marker(index), evidence: { namespace: "demo", id: value.id, selectedVersion: "1.0.0", versionRange: "1.0.0", sourceRelease: "registry-v1", indexDigest: identity.registry_snapshot_digest, artifactDigest: marker(index), verifiedAt: 1, outcome: "prepared" } }));
  return { snapshot, identity, dependencies };
}

async function planFor(contracts, target, facts) {
  const { snapshot, identity, dependencies } = typedWorld(contracts);
  return browserLocalPlan(identity, snapshot, dependencies, target, facts, "local", { app_id: "demo" });
}

const paths = (response) => response.proposals.map((proposal) => proposal.proposal.nodes.map((node) => node.capability_id));
const SINK = { capability_id: "sink", capability_version: "1.0.0" };

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

// Regression for #1477: `plan_search_truncated` must mean "candidates were
// excluded", matching the native `build_chains` bounds in
// `crates/traverse-embedder/src/browser_local_plan.rs` (more than five chains,
// or an edge skipped because it would need a ninth node).
const marker = (index) => String.fromCharCode(97 + (index % 26)).repeat(2) + String(index);
function graph(nodes) {
  const snapshot = {
    releaseTag: "registry-v1",
    capabilities: nodes.map((node, index) => ({ namespace: "bounds", id: node.id, version: "1.0.0", digest: digest(marker(index)), artifactUrl: "", contractDigest: "", contractUrl: "", deprecated: false })),
  };
  const identity = { registry_snapshot_digest: sha(snapshot), source_release: snapshot.releaseTag, contract_schema_version: "1.0.0" };
  const dependencies = nodes.map((node, index) => ({
    wasmBytes: new Uint8Array(),
    contractBytes: new TextEncoder().encode(JSON.stringify(contract(node.id, node.inputs, node.outputs))),
    wasmDigest: digest(marker(index)),
    evidence: { namespace: "bounds", id: node.id, selectedVersion: "1.0.0", versionRange: "1.0.0", sourceRelease: "registry-v1", indexDigest: identity.registry_snapshot_digest, artifactDigest: digest(marker(index)), verifiedAt: 1, outcome: "prepared" },
  }));
  return { snapshot, identity, dependencies };
}
const plan = (nodes, targetId, facts) => {
  const { snapshot, identity, dependencies } = graph(nodes);
  return browserLocalPlan(identity, snapshot, dependencies, { capability_id: targetId, capability_version: "1.0.0" }, facts, "local", { app_id: "bounds" });
};
// `producers` distinct capabilities each turn the starting fact into the sink's
// single required input, so the candidate count equals the producer count.
const producerGraph = (producers) => [
  { id: "sink", inputs: ["middle"], outputs: ["result"] },
  ...Array.from({ length: producers }, (_value, index) => ({ id: `producer-${index + 1}`, inputs: ["seed"], outputs: ["middle"] })),
];

for (const producers of [0, 1, 4, 5]) {
  test(`browser planner reports no truncation when ${producers} producers all fit the five-plan bound`, async () => {
    const result = await plan(producerGraph(producers), "sink", { seed: "x" });
    assert.equal(result.proposals.length, producers);
    assert.equal(result.plan_search_truncated, false);
  });
}

test("browser planner reports truncation and keeps five plans when a sixth producer exists", async () => {
  const result = await plan(producerGraph(6), "sink", { seed: "x" });
  assert.equal(result.proposals.length, 5);
  assert.equal(result.plan_search_truncated, true);
});

test("browser planner returns a deterministic five-plan prefix when bounded", async () => {
  const nodes = producerGraph(6);
  const first = await plan(nodes, "sink", { seed: "x" });
  const second = await plan(nodes, "sink", { seed: "x" });
  assert.deepEqual(first, second);
  assert.deepEqual(
    first.proposals.map((proposal) => proposal.proposal.nodes[0].capability_id),
    ["producer-1", "producer-2", "producer-3", "producer-4", "producer-5"],
  );
});

// Chain of `length` capabilities: capability n consumes `f{n-1}` and emits
// `f{n}`, so only the full chain reaches the target from `{ f0 }`.
const chainGraph = (length) => Array.from({ length }, (_value, index) => ({ id: `step-${index + 1}`, inputs: [`f${index}`], outputs: [`f${index + 1}`] }));

test("browser planner returns an exactly eight-node chain without reporting truncation", async () => {
  const result = await plan(chainGraph(8), "step-8", { f0: "x" });
  assert.equal(result.proposals.length, 1);
  assert.equal(result.proposals[0].proposal.nodes.length, 8);
  assert.equal(result.plan_search_truncated, false);
});

test("browser planner reports truncation when a chain needs a ninth node", async () => {
  const result = await plan(chainGraph(9), "step-9", { f0: "x" });
  assert.equal(result.proposals.length, 0);
  assert.equal(result.plan_search_truncated, true);
});

test("browser planner terminates on a cyclic producer pair and reports it truthfully", async () => {
  const nodes = [
    { id: "sink", inputs: ["kb"], outputs: ["result"] },
    { id: "cycle-a", inputs: ["ka"], outputs: ["kb"] },
    { id: "cycle-b", inputs: ["kb"], outputs: ["ka"] },
  ];
  const unreachable = await plan(nodes, "sink", {});
  assert.equal(unreachable.proposals.length, 0);
  assert.equal(unreachable.plan_search_truncated, false);
  const reachable = await plan(nodes, "sink", { ka: "x" });
  assert.deepEqual(reachable.proposals.map((proposal) => proposal.proposal.nodes.map((node) => node.capability_id)), [["cycle-a", "sink"]]);
  assert.equal(reachable.plan_search_truncated, false);
});

test("browser planner fails closed before planning on altered snapshot evidence", async () => {
  const { snapshot, identity, dependencies } = inputs();
  await assert.rejects(() => browserLocalPlan({ ...identity, registry_snapshot_digest: digest("z") }, snapshot, dependencies, { capability_id: "sink", capability_version: "1.0.0" }, {}, "local", {}), (error) => error instanceof BrowserPlanError && error.code === "browser_plan_snapshot_digest_mismatch");
});


test("browser planner counts each prepared capability identity only once", async () => {
  const { snapshot, identity, dependencies } = graph(producerGraph(3));
  const result = await browserLocalPlan(identity, snapshot, [...dependencies, ...dependencies],
    { capability_id: "sink", capability_version: "1.0.0" }, { seed: "x" }, "local", {});
  assert.equal(result.proposals.length, 3);
  assert.equal(result.plan_search_truncated, false);
});

test("browser planner reports its search-call bound in a large dead search", async () => {
  const nodes = [{ id: "sink", inputs: ["f6"], outputs: ["result"] }];
  for (let level = 1; level <= 6; level += 1) {
    for (let branch = 0; branch < 4; branch += 1) {
      nodes.push({ id: `level-${level}-${branch}`, inputs: [`f${level - 1}`], outputs: [`f${level}`] });
    }
  }
  // 4^6 possible dead paths, all shorter than the depth cap: only the
  // native-equivalent 4000-call work budget should mark this truncated.
  const result = await plan(nodes, "sink", {});
  assert.equal(result.proposals.length, 0);
  assert.equal(result.plan_search_truncated, true);
});

test("browser planner rejects a producer/consumer pair whose shared property types differ", async () => {
  // Regression for #1476: `source` emits `middle` as an integer, `sink`
  // requires a string `middle`. Name-only coverage chained them anyway.
  const response = await planFor(
    [typedContract("source", { seed: "string" }, { middle: "integer" }), typedContract("sink", { middle: "string" }, { result: "string" })],
    SINK,
    { seed: "x" },
  );
  assert.deepEqual(paths(response), []);
  assert.equal(response.plan_search_truncated, false);
});

test("browser planner still chains a producer/consumer pair whose shared property types match", async () => {
  const response = await planFor(
    [typedContract("source", { seed: "string" }, { middle: "string" }), typedContract("sink", { middle: "string" }, { result: "string" })],
    SINK,
    { seed: "x" },
  );
  assert.deepEqual(paths(response), [["source", "sink"]]);
  assert.equal(response.proposals[0].mapping_unconfirmed, true);
  assert.deepEqual(response.proposals[0].proposal.mappings, [
    { from_node_id: null, from_field: "seed", to_node_id: "node-1", to_field: "seed", source: "starting_facts" },
    { from_node_id: "node-1", from_field: "middle", to_node_id: "node-2", to_field: "middle", source: "capability_output" },
  ]);
});

test("browser planner matches the actual JSON type of each starting fact", async () => {
  const cases = [
    { declared: "integer", value: 3, planned: true },
    { declared: "integer", value: "3", planned: false },
    { declared: "integer", value: 1.5, planned: false },
    { declared: "number", value: 1.5, planned: true },
    { declared: "number", value: 2, planned: false },
    { declared: "boolean", value: true, planned: true },
    { declared: "boolean", value: "true", planned: false },
    { declared: "null", value: null, planned: true },
    { declared: "string", value: null, planned: false },
    { declared: "array", value: [1, 2], planned: true },
    { declared: "array", value: { a: 1 }, planned: false },
    { declared: "object", value: { a: 1 }, planned: true },
    { declared: "object", value: [1, 2], planned: false },
  ];
  for (const { declared, value, planned } of cases) {
    const response = await planFor([typedContract("sink", { fact: declared }, { result: "string" })], SINK, { fact: value });
    assert.equal(response.proposals.length, planned ? 1 : 0, `declared ${declared} against ${JSON.stringify(value)}`);
  }
});

test("browser planner treats an absent declared property type as uncovered, not as absent from the required set", async () => {
  // A required property with no `type` keyword can never be shown to match, so
  // coverage fails — the name must not silently drop out of `required`.
  const untypedOnConsumer = await planFor(
    [typedContract("sink", { middle: "string", extra: null }, { result: "string" })],
    SINK,
    { middle: "m", extra: "e" },
  );
  assert.deepEqual(paths(untypedOnConsumer), []);

  const untypedOnProducer = await planFor(
    [typedContract("source", { seed: "string" }, { middle: null }), typedContract("sink", { middle: "string" }, { result: "string" })],
    SINK,
    { seed: "x" },
  );
  assert.deepEqual(paths(untypedOnProducer), []);
});

test("browser planner ignores type disagreement on properties the consumer does not require", async () => {
  const response = await planFor(
    [
      typedContract("source", { seed: "string" }, { middle: "string", hint: "string" }),
      typedContract("sink", { middle: "string", hint: "integer" }, { result: "string" }, { inputRequired: ["middle"] }),
    ],
    SINK,
    { seed: "x" },
  );
  assert.deepEqual(paths(response), [["source", "sink"]]);
  assert.deepEqual(response.proposals[0].proposal.mappings.map((mapping) => mapping.to_field), ["seed", "middle"]);
});

test("browser planner keeps typed multihop forwarding and maps each field to a type-compatible predecessor", async () => {
  // `collect` also emits `insights`, but as a string, so it is neither a valid
  // direct predecessor of `sink` nor a valid mapping source for it.
  const response = await planFor(
    [
      typedContract("collect", { fragments: "string" }, { facts: "array", insights: "string" }),
      typedContract("enrich", { facts: "array" }, { insights: "object" }),
      typedContract("sink", { insights: "object" }, { result: "string" }),
    ],
    SINK,
    { fragments: "[]" },
  );
  assert.deepEqual(paths(response), [["collect", "enrich", "sink"]]);
  const mappings = response.proposals[0].proposal.mappings;
  assert.deepEqual(mappings.find((mapping) => mapping.to_field === "insights"), { from_node_id: "node-2", from_field: "insights", to_node_id: "node-3", to_field: "insights", source: "capability_output" });
  assert.deepEqual(mappings.filter((mapping) => mapping.source === "starting_facts").map((mapping) => mapping.to_field), ["fragments"]);
});

test("browser planner returns identical typed proposals for repeated identical calls", async () => {
  const contracts = [typedContract("source", { seed: "string" }, { middle: "integer" }), typedContract("sink", { middle: "integer" }, { result: "string" })];
  const first = await planFor(contracts, SINK, { seed: "x" });
  const second = await planFor(contracts, SINK, { seed: "x" });
  assert.deepEqual(first, second);
  assert.deepEqual(paths(first), [["source", "sink"]]);
  assert.ok(first.proposals.every((proposal) => proposal.mapping_unconfirmed === true));
});

// Non-string schema keywords must stay uncovered on either side of a match.
// A JSON Schema union is not a supported scalar type declaration here; this
// mirrors the native planner rather than silently choosing a union member.
for (const type of [["string", "null"], 42]) {
  const label = JSON.stringify(type);

  test(`browser planner rejects required type ${label} against starting facts`, async () => {
    const response = await planFor(
      [typedContract("sink", { middle: "string", extra: type }, { result: "string" })],
      SINK,
      { middle: "m", extra: "e" },
    );
    assert.deepEqual(paths(response), []);
    assert.equal(response.plan_search_truncated, false);
  });

  test(`browser planner rejects required type ${label} against predecessor outputs`, async () => {
    const response = await planFor(
      [
        typedContract("source", { seed: "string" }, { middle: "string", extra: "string" }),
        typedContract("sink", { middle: "string", extra: type }, { result: "string" }),
      ],
      SINK,
      { seed: "x" },
    );
    assert.deepEqual(paths(response), []);
    assert.equal(response.plan_search_truncated, false);
  });

  test(`browser planner rejects output type ${label} as a required mapping source`, async () => {
    const response = await planFor(
      [typedContract("source", { seed: "string" }, { middle: type }), typedContract("sink", { middle: "string" }, { result: "string" })],
      SINK,
      { seed: "x" },
    );
    assert.deepEqual(paths(response), []);
    assert.equal(response.plan_search_truncated, false);
  });
}
