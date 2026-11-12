// Spec 138 browser host ceilings and Worker-based mid-run interruption
// (#1582, Decision 104). Node has no `Worker` global, so these tests pass a
// `createWorker` factory that runs the shipped dist/modelWorker.js in a
// `worker_threads` Worker.
import test from "node:test";
import assert from "node:assert/strict";
import { createHash, createPrivateKey, sign } from "node:crypto";
import { readFileSync } from "node:fs";
import { Worker } from "node:worker_threads";
import { ExactModelBrowserHost, ExactModelError, PLACEMENT_WASM_CPU } from "../dist/index.js";

const MODELS = new URL("../../../../fixtures/models/", import.meta.url);
const readModelFile = (path) => new Uint8Array(readFileSync(new URL(path, MODELS)));
const TEST_KEY = JSON.parse(readFileSync(new URL("test-signing-key.json", MODELS), "utf8"));
const V3_VECTOR = JSON.parse(readFileSync(new URL("conformance/signed-prepared-v3.json", MODELS), "utf8"));
const TRUST = { trustedPublicKeysHex: [TEST_KEY.public_key_hex], modelUsage: "commercial" };
const POLICY = { policy_ref: "policy-1" };

const fixture = (dir) => ({
  manifest: readModelFile(`${dir}/model.manifest.json`),
  wasm: readModelFile(`${dir}/model.wasm`),
  sig: readModelFile(`${dir}/model.sig.json`),
});
const sha256Hex = (bytes) => createHash("sha256").update(bytes).digest("hex");
const pinFor = (modelId, manifestBytes) => ({
  model_id: modelId,
  version: "1.0.0",
  digest: sha256Hex(manifestBytes),
  offline_allowed: true,
  target: PLACEMENT_WASM_CPU,
  rights: { license_id: "Apache-2.0", commercial_use: "allowed" },
});
const executeArgs = (modelId, digest, input_ref, extra = {}) => ({
  model_ref: { model_id: modelId, version: "1.0.0", digest },
  input_ref,
  policy_ref: "policy-1",
  data_classification: "sensitive",
  input_schema_ref: "schema:fixture-in",
  input_schema_version: "1.0.0",
  max_output_bytes: 4096,
  allowed_classifications: ["sensitive"],
  ...extra,
});

/** Re-sign an edited manifest with the test key. */
function resigned(manifestBytes, edit) {
  const value = JSON.parse(new TextDecoder().decode(manifestBytes));
  edit(value);
  const bytes = new TextEncoder().encode(JSON.stringify(value));
  const key = createPrivateKey({
    key: Buffer.concat([Buffer.from("302e020100300506032b657004220420", "hex"), Buffer.from(TEST_KEY.secret_key_hex, "hex")]),
    format: "der",
    type: "pkcs8",
  });
  const signature = sign(null, bytes, key).toString("hex");
  const sig = new TextEncoder().encode(JSON.stringify({ alg: "ed25519", key_id: TEST_KEY.key_id, signature }));
  return { bytes, sig };
}

/** Counts live Workers so each test can prove it stopped all of them. */
let live = 0;
function nodeWorker() {
  const worker = new Worker(new URL("./support/modelWorkerNode.mjs", import.meta.url));
  live += 1;
  let stopped = false;
  const adapter = {
    onmessage: null,
    onerror: null,
    postMessage: (message) => worker.postMessage(message),
    terminate: () => {
      if (!stopped) {
        stopped = true;
        live -= 1;
        void worker.terminate();
      }
    },
  };
  worker.on("message", (data) => adapter.onmessage?.({ data }));
  worker.on("error", (error) => adapter.onerror?.(error));
  return adapter;
}

const WORKERS = { createWorker: nodeWorker };

async function hostWith(dir, modelId, options = {}, pkg = fixture(dir)) {
  const host = new ExactModelBrowserHost([pinFor(modelId, pkg.manifest)], { ...TRUST, ...options });
  const digest = await host.registerPackage(pkg.manifest, pkg.wasm, pkg.sig, POLICY);
  return { host, digest };
}

const run = (host, modelId, digest, bytes, extra) =>
  host.execute(executeArgs(modelId, digest, host.io.stageModelInput(bytes, 4096), extra));

test("registration enforces host ceilings with host_limit_exceeded (FR-028)", async () => {
  const looper = fixture("fixture-looper-1.0.0");
  const v3 = fixture("fixture-prepared-v3-1.0.0");
  const exceeded = (e) => e instanceof ExactModelError && e.code === "model_incompatible" && e.reason === "host_limit_exceeded";
  const cases = [
    [looper, "fixture.looper", { maxPackageBytes: looper.manifest.length + looper.wasm.length - 1 }],
    [looper, "fixture.looper", { maxMemoryBytes: 131_071 }],
    [looper, "fixture.looper", { maxFuel: 8_999_999_999 }],
    // The prepare budget is held to the same fuel ceiling.
    [v3, "fixture.prepared-v3", { maxFuel: 999_999, createWorker: null }],
  ];
  for (const [pkg, modelId, limits] of cases) {
    await assert.rejects(() => hostWith("", modelId, limits, pkg), exceeded, JSON.stringify(limits));
  }
  // At the ceilings exactly, registration succeeds.
  const { host } = await hostWith("", "fixture.looper", {
    maxPackageBytes: looper.manifest.length + looper.wasm.length,
    maxMemoryBytes: 131_072,
    maxFuel: 9_000_000_000,
    createWorker: null,
  }, looper);
  host.shutdown();
});

test("a Worker interrupts a running inference on deadline and cancellation (FR-029)", async () => {
  const { host, digest } = await hostWith("fixture-looper-1.0.0", "fixture.looper", WORKERS);
  const looper = (extra) => run(host, "fixture.looper", digest, new Uint8Array([1]), extra);
  let started = performance.now();
  await assert.rejects(() => looper({ timeout_ms: 200 }), (e) => e.code === "timeout");
  assert.ok(performance.now() - started < 10_000, "stopped mid-run, not at max_execution_ms");
  // The next call respawns the Worker; a cancel that lands mid-run stops it.
  const midway = new AbortController();
  setTimeout(() => midway.abort(), 200);
  started = performance.now();
  await assert.rejects(() => looper({ signal: midway.signal }), (e) => e.code === "cancelled");
  assert.ok(performance.now() - started < 10_000);
  // An already-aborted call never reaches the guest.
  const aborted = new AbortController();
  aborted.abort();
  await assert.rejects(() => looper({ signal: aborted.signal }), (e) => e.code === "cancelled");
  host.shutdown();
  assert.equal(live, 0);
});

test("a reused Worker keeps the v3 snapshot and the signed vector matches (FR-055)", async () => {
  const { host, digest } = await hostWith("fixture-prepared-v3-1.0.0", "fixture.prepared-v3", WORKERS);
  assert.ok(host.snapshotBytes() > 0, "model_prepare ran at registration, in the Worker");
  for (let round = 0; round < 2; round++) {
    for (const testCase of V3_VECTOR.cases) {
      const result = await run(host, "fixture.prepared-v3", digest, Buffer.from(testCase.input_frame_hex, "hex"));
      assert.equal(Buffer.from(host.io.readModelOutput(result.output_ref, 4096)).toString("hex"), testCase.output_frame_hex);
    }
  }
  // A status change drops the snapshot in the Worker too; the next call
  // takes the fresh path and stores a new one.
  host.setPackageStatus({ [digest]: { status: "deprecated", reason: "superseded" } });
  assert.equal(host.snapshotBytes(), 0);
  await run(host, "fixture.prepared-v3", digest, Buffer.from("abc"));
  assert.ok(host.snapshotBytes() > 0);
  host.shutdown();
  assert.equal(host.snapshotBytes(), 0);
  assert.equal(live, 0);
});

test("a Worker interrupts a running model_prepare; nothing is kept (FR-056)", async () => {
  const { default: wabtInit } = await import("wabt");
  const wabt = await wabtInit();
  const wasm = new Uint8Array(
    wabt
      .parseWat(
        "spin.wat",
        `(module (memory (export "memory") 1)
           (func (export "model_prepare") (result i32) (loop $spin (br $spin)) i32.const 0)
           (func (export "model_alloc") (param i32) (result i32) i32.const 64)
           (func (export "model_execute") (param i32 i32 i32 i32) (result i32) i32.const 0))`,
      )
      .toBinary({}).buffer,
  );
  const { bytes, sig } = resigned(fixture("fixture-prepared-v3-1.0.0").manifest, (m) => {
    m.wasm_digest = sha256Hex(wasm);
  });
  const register = (options) => {
    const host = new ExactModelBrowserHost([pinFor("fixture.prepared-v3", bytes)], { ...TRUST, ...WORKERS });
    return { host, pending: host.registerPackage(bytes, wasm, sig, POLICY, options) };
  };
  let started = performance.now();
  const overBudget = register({ preparationBudgetMs: 200 });
  await assert.rejects(
    () => overBudget.pending,
    (e) => e.reason === "placement_unavailable" && e.message.includes("wasm-cpu over_budget"),
  );
  assert.ok(performance.now() - started < 10_000, "stopped mid-prepare");
  assert.equal(overBudget.host.snapshotBytes(), 0);
  const midway = new AbortController();
  setTimeout(() => midway.abort(), 200);
  started = performance.now();
  const cancelled = register({ signal: midway.signal });
  await assert.rejects(() => cancelled.pending, (e) => e.code === "cancelled");
  assert.ok(performance.now() - started < 10_000);
  assert.equal(cancelled.host.snapshotBytes(), 0);
  // A package that never registered keeps no Worker.
  assert.equal(live, 0);
});

test("a Worker that fails, or a guest that traps, fails closed with execution_failed", async () => {
  const broken = () => {
    const adapter = {
      onmessage: null,
      onerror: null,
      postMessage: () => queueMicrotask(() => adapter.onerror?.(new Error("script failed to load"))),
      terminate: () => {},
    };
    return adapter;
  };
  const echo = fixture("fixture-echo-1.0.0");
  const { host, digest } = await hostWith("", "fixture.echo", { createWorker: broken }, echo);
  await assert.rejects(
    () => run(host, "fixture.echo", digest, new Uint8Array([1, 2, 3])),
    (e) => e.code === "execution_failed" && e.message === "model guest worker failed",
  );
  host.shutdown();
  // A trap crosses the Worker boundary as a stable failure, inline and in a Worker.
  const { default: wabtInit } = await import("wabt");
  const wabt = await wabtInit();
  const wasm = new Uint8Array(
    wabt
      .parseWat(
        "trap.wat",
        `(module (memory (export "memory") 1)
           (func (export "model_execute") (param i32 i32 i32 i32) (result i32) unreachable))`,
      )
      .toBinary({}).buffer,
  );
  const { bytes, sig } = resigned(echo.manifest, (m) => {
    m.wasm_digest = sha256Hex(wasm);
  });
  for (const createWorker of [null, nodeWorker]) {
    const trapping = await hostWith("", "fixture.echo", { createWorker }, { manifest: bytes, wasm, sig });
    await assert.rejects(
      () => run(trapping.host, "fixture.echo", trapping.digest, new Uint8Array([1])),
      (e) => e instanceof ExactModelError && e.code === "execution_failed" && e.message === "model guest trapped",
    );
    trapping.host.shutdown();
  }
  assert.equal(live, 0);
});
