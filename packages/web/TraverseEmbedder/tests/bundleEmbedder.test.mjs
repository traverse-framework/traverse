// BundleEmbedder tests: the production runtime-WASM execution path (spec
// 068 FR-002, NFR-001). Covers the shared spec-057 conformance corpus
// scenarios plus package edge cases, using real WAT-compiled WASI modules
// (via wabt) so execution is genuine, not mocked.
import test from "node:test";
import assert from "node:assert/strict";
import { readFile, writeFile } from "node:fs/promises";
import {
  BundleEmbedder,
  BundleRejectedError,
  NodeFsBundleLoader,
} from "../dist/index.js";
import {
  ECHO_WAT,
  INVALID_OUTPUT_WAT,
  NONZERO_EXIT_WAT,
  UNAUTHORIZED_IMPORT_WAT,
  compileWat,
  sha256Digest,
  writeBundleFixture,
} from "./fixtures.mjs";

const loader = new NodeFsBundleLoader();

function collectEvents(embedder) {
  const events = [];
  embedder.subscribe((event) => events.push(event));
  return events;
}

async function initEmbedder(manifestPath, overrides = {}) {
  return BundleEmbedder.init({ manifestPath, loader, platform: "web", ...overrides });
}

// --- init-shutdown scenario ---

test("init-shutdown: shutdown is idempotent and stops later submits", async () => {
  const echo = await compileWat(ECHO_WAT);
  const manifestPath = await writeBundleFixture({
    appId: "fixture-app",
    components: [{ capabilityId: "fixture.echo", wasmBytes: echo }],
  });
  const embedder = await initEmbedder(manifestPath);

  assert.equal(embedder.shutdown().killedInstances, 0);
  assert.equal(embedder.shutdown().killedInstances, 0);

  const rejected = embedder.submit("fixture.echo", { note: "late" });
  assert.equal(rejected.status, "rejected");
  assert.equal(rejected.error.code, "runtime_stopped");
});

// --- wasm-capability-submit scenario ---

test("wasm-capability-submit: real WASI execution echoes input as output", async () => {
  const echo = await compileWat(ECHO_WAT);
  const manifestPath = await writeBundleFixture({
    appId: "fixture-app",
    components: [{ capabilityId: "fixture.echo", wasmBytes: echo }],
  });
  const embedder = await initEmbedder(manifestPath);
  const events = collectEvents(embedder);

  const outcome = embedder.submit("fixture.echo", { note: "hello", n: 3 });
  assert.equal(outcome.status, "accepted");
  assert.equal(outcome.sessionId, "sess-00000001");

  assert.equal(events.length, 2);
  assert.equal(events[0].event_type, "capability_invoked");
  assert.equal(events[0].data.capability_id, "fixture.echo");
  assert.equal(events[1].event_type, "capability_result");
  assert.equal(events[1].data.status, "completed");
  assert.deepEqual(events[1].data.output, { note: "hello", n: 3 });
});

test("capability submit surfaces invalid stdout as output_deserialization_failed", async () => {
  const invalid = await compileWat(INVALID_OUTPUT_WAT);
  const manifestPath = await writeBundleFixture({
    appId: "fixture-app",
    components: [{ capabilityId: "fixture.bad-output", wasmBytes: invalid }],
  });
  const embedder = await initEmbedder(manifestPath);
  const events = collectEvents(embedder);

  const outcome = embedder.submit("fixture.bad-output", {});
  assert.equal(outcome.status, "accepted");
  assert.equal(events[1].event_type, "error");
  assert.equal(events[1].data.error.code, "output_deserialization_failed");
});

test("capability submit surfaces a non-zero WASI exit as execution_failed", async () => {
  const nonzero = await compileWat(NONZERO_EXIT_WAT);
  const manifestPath = await writeBundleFixture({
    appId: "fixture-app",
    components: [{ capabilityId: "fixture.exits", wasmBytes: nonzero }],
  });
  const embedder = await initEmbedder(manifestPath);
  const events = collectEvents(embedder);

  embedder.submit("fixture.exits", {});
  assert.equal(events[1].event_type, "error");
  assert.equal(events[1].data.error.code, "execution_failed");
  assert.match(events[1].data.error.message, /exited with code 1/);
});

// --- workflow (multi-node linear pipeline) ---

test("workflow pipeline executes nodes in order with runtime-owned merged output", async () => {
  const echo = await compileWat(ECHO_WAT);
  const manifestPath = await writeBundleFixture({
    appId: "fixture-app",
    components: [
      { capabilityId: "fixture.echoA", wasmBytes: echo },
      { capabilityId: "fixture.echoB", wasmBytes: echo },
    ],
    workflow: {
      nodes: [
        {
          node_id: "nodeA",
          capability_id: "fixture.echoA",
          capability_version: "1.0.0",
          input: { from_workflow_input: ["document"] },
          output: { to_workflow_state: ["document"], publish_to_state_as: "stepA" },
        },
        {
          node_id: "nodeB",
          capability_id: "fixture.echoB",
          capability_version: "1.0.0",
          input: { from_workflow_input: ["document", "stepA"] },
          output: { to_workflow_state: [], publish_to_state_as: "stepB" },
        },
      ],
      edges: [{ edge_id: "a_to_b", from: "nodeA", to: "nodeB", trigger: "direct" }],
      startNode: "nodeA",
      outputProjection: ["stepA", "stepB"],
    },
  });
  const embedder = await initEmbedder(manifestPath);
  const events = collectEvents(embedder);

  const outcome = embedder.submit("fixture.pipeline", { document: "hello" });
  assert.equal(outcome.status, "accepted");

  const invoked = events.filter((event) => event.event_type === "capability_invoked");
  assert.equal(invoked.length, 2);
  assert.deepEqual(
    invoked.map((event) => [event.data.node_id, event.data.status]),
    [
      ["nodeA", "completed"],
      ["nodeB", "completed"],
    ],
  );

  const terminal = events[events.length - 1];
  assert.equal(terminal.event_type, "capability_result");
  assert.deepEqual(terminal.data.output, {
    stepA: { document: "hello" },
    stepB: { document: "hello", stepA: { document: "hello" } },
  });
});

test("determinism: identical workflow input produces identical event JSON", async () => {
  const echo = await compileWat(ECHO_WAT);
  const wasmBytes = echo;
  const buildFixture = () =>
    writeBundleFixture({
      appId: "fixture-app",
      components: [{ capabilityId: "fixture.echo", wasmBytes }],
      workflow: {
        nodes: [
          {
            node_id: "nodeA",
            capability_id: "fixture.echo",
            capability_version: "1.0.0",
            input: { from_workflow_input: ["document"] },
            output: { to_workflow_state: [], publish_to_state_as: "result" },
          },
        ],
        edges: [],
        startNode: "nodeA",
        outputProjection: ["result"],
      },
    });

  const run = async () => {
    const embedder = await initEmbedder(await buildFixture());
    const events = collectEvents(embedder);
    embedder.submit("fixture.pipeline", { document: "same input" });
    embedder.shutdown();
    return events;
  };

  const first = await run();
  const second = await run();
  assert.equal(JSON.stringify(first), JSON.stringify(second));
});

// --- compatible lifecycle + platform-guard scenario ---

test("compatible lifecycle: start, stop, platform-guard, and kill-on-shutdown", async () => {
  const manifestPath = await writeBundleFixture({
    appId: "fixture-app",
    components: [],
    compatible: { capabilityId: "fixture.render", platforms: ["web"] },
  });
  const embedder = await initEmbedder(manifestPath, { platform: "web" });
  const events = collectEvents(embedder);

  const started = embedder.startCompatible("fixture.render", {});
  assert.equal(started.status, "started");
  const stopped = embedder.stopCompatible("fixture.render", started.instanceId);
  assert.equal(stopped.status, "stopped");
  embedder.startCompatible("fixture.render", {});
  assert.equal(embedder.shutdown().killedInstances, 1);

  const states = events
    .filter((event) => event.event_type === "state_changed")
    .map((event) => event.data.state);
  assert.deepEqual(states, ["started", "stopped", "started", "killed"]);
});

test("platform-guard: wrong platform rejects with a deterministic error event", async () => {
  const manifestPath = await writeBundleFixture({
    appId: "fixture-app",
    components: [],
    compatible: { capabilityId: "fixture.render", platforms: ["web"] },
  });
  const embedder = await initEmbedder(manifestPath, { platform: "ios" });
  const events = collectEvents(embedder);

  const outcome = embedder.startCompatible("fixture.render", {});
  assert.equal(outcome.status, "error");
  assert.equal(outcome.error.code, "platform_not_supported");
  assert.equal(events[0].data.error.code, "platform_not_supported");
});

// --- init rejection paths (spec 068 NFR-001) ---

test("init rejects a module that imports an unauthorized host function", async () => {
  const bad = await compileWat(UNAUTHORIZED_IMPORT_WAT);
  const manifestPath = await writeBundleFixture({
    appId: "fixture-app",
    components: [{ capabilityId: "fixture.evil", wasmBytes: bad }],
  });

  await assert.rejects(
    initEmbedder(manifestPath),
    (error) => {
      assert.ok(error instanceof BundleRejectedError);
      assert.equal(error.embedderError.code, "bundle_load_failed");
      assert.match(error.embedderError.message, /unauthorized host function/);
      assert.match(error.embedderError.message, /environ_get/);
      return true;
    },
  );
});

test("init rejects a component whose bytes do not match the declared digest", async () => {
  const echo = await compileWat(ECHO_WAT);
  const manifestPath = await writeBundleFixture({
    appId: "fixture-app",
    components: [{ capabilityId: "fixture.echo", wasmBytes: echo }],
  });
  // Corrupt the bytes on disk after the manifest already declared the
  // original digest, simulating tampering or transport corruption.
  const wasmPath = new URL("components/0/component.wasm", `file://${manifestPath}`);
  const tampered = await compileWat(INVALID_OUTPUT_WAT);
  await writeFile(wasmPath, tampered);

  await assert.rejects(
    initEmbedder(manifestPath),
    (error) => {
      assert.ok(error instanceof BundleRejectedError);
      assert.equal(error.embedderError.code, "bundle_load_failed");
      assert.match(error.embedderError.message, /digest mismatch/);
      return true;
    },
  );
});

test("init rejects a component whose manifest digest disagrees with the app manifest", async () => {
  const echo = await compileWat(ECHO_WAT);
  const manifestPath = await writeBundleFixture({
    appId: "fixture-app",
    components: [{ capabilityId: "fixture.echo", wasmBytes: echo }],
  });
  const componentManifestPath = new URL(
    "components/0/component.manifest.json",
    `file://${manifestPath}`,
  );
  const componentManifest = JSON.parse(await readFile(componentManifestPath, "utf8"));
  componentManifest.wasm_digest = sha256Digest(new TextEncoder().encode("wrong bytes"));
  await writeFile(componentManifestPath, JSON.stringify(componentManifest));

  await assert.rejects(
    initEmbedder(manifestPath),
    (error) => {
      assert.ok(error instanceof BundleRejectedError);
      assert.match(error.embedderError.message, /does not match the app manifest/);
      return true;
    },
  );
});

test("init rejects workflows with non-direct edges and branching edges", async () => {
  const echo = await compileWat(ECHO_WAT);
  const eventDriven = await writeBundleFixture({
    appId: "fixture-app",
    components: [{ capabilityId: "fixture.echo", wasmBytes: echo }],
    workflow: {
      nodes: [
        {
          node_id: "nodeA",
          capability_id: "fixture.echo",
          capability_version: "1.0.0",
          input: { from_workflow_input: [] },
          output: { to_workflow_state: [] },
        },
      ],
      edges: [{ edge_id: "e", from: "nodeA", to: "nodeA", trigger: "event" }],
      startNode: "nodeA",
      outputProjection: [],
    },
  });
  await assert.rejects(initEmbedder(eventDriven), (error) => {
    assert.ok(error instanceof BundleRejectedError);
    assert.match(error.embedderError.message, /supports only 'direct'-triggered/);
    return true;
  });

  const branching = await writeBundleFixture({
    appId: "fixture-app",
    components: [
      { capabilityId: "fixture.echoA", wasmBytes: echo },
      { capabilityId: "fixture.echoB", wasmBytes: echo },
    ],
    workflow: {
      nodes: [
        {
          node_id: "nodeA",
          capability_id: "fixture.echoA",
          capability_version: "1.0.0",
          input: { from_workflow_input: [] },
          output: { to_workflow_state: [] },
        },
        {
          node_id: "nodeB",
          capability_id: "fixture.echoB",
          capability_version: "1.0.0",
          input: { from_workflow_input: [] },
          output: { to_workflow_state: [] },
        },
      ],
      edges: [
        { edge_id: "a_to_b", from: "nodeA", to: "nodeB", trigger: "direct" },
        { edge_id: "a_to_a", from: "nodeA", to: "nodeA", trigger: "direct" },
      ],
      startNode: "nodeA",
      outputProjection: [],
    },
  });
  await assert.rejects(initEmbedder(branching), (error) => {
    assert.ok(error instanceof BundleRejectedError);
    assert.match(error.embedderError.message, /branching pipelines are not supported/);
    return true;
  });
});

// --- release evidence ---

test("release evidence records the linked runtime and bundled wasm digests", async () => {
  const echo = await compileWat(ECHO_WAT);
  const digest = sha256Digest(echo);
  const manifestPath = await writeBundleFixture({
    appId: "fixture-app",
    components: [{ capabilityId: "fixture.echo", wasmBytes: echo }],
  });
  const embedder = await initEmbedder(manifestPath);

  const evidence = embedder.releaseEvidence();
  assert.equal(evidence.runtime.implementation, "browser-webassembly");
  assert.equal(evidence.bundle.app_id, "fixture-app");
  assert.equal(evidence.bundle.wasm_components.length, 1);
  assert.equal(evidence.bundle.wasm_components[0].capability_id, "fixture.echo");
  assert.equal(evidence.bundle.wasm_components[0].wasm_digest, digest);
});

// --- real checked-in bundle (no-sidecar proof against actual repository files) ---

test("real checked-in traverse-starter bundle loads and executes without a sidecar", async () => {
  const manifestPath = new URL(
    "../../../../examples/applications/traverse-starter/app.manifest.json",
    import.meta.url,
  ).pathname;
  const embedder = await initEmbedder(manifestPath);
  const events = collectEvents(embedder);

  // This proves the full no-sidecar pipeline genuinely runs against the
  // real repository bundle: manifest/workflow resolution, sha-256 digest
  // verification, Traverse Host ABI import validation, and real
  // WebAssembly.instantiate + invocation. Public Trace API records are
  // independently safe projections, never a serialization of this payload.
  const outcome = embedder.submit("traverse-starter.process", { note: "hello" });
  assert.equal(outcome.status, "accepted");
  const terminal = events[events.length - 1];
  assert.equal(terminal.event_type, "capability_result");
  const page = embedder.traceList("1.0.0", 10);
  assert.equal(page.summaries.length, 1);
  const detail = embedder.traceGet("1.0.0", page.summaries[0].traceId);
  assert.equal(detail.summary.targetId, "traverse-starter.process");
  assert.equal(JSON.stringify(detail).includes("hello"), false);
});
