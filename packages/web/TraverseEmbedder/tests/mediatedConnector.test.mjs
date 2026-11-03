// Spec 1719 (Decision 122): a guest suspends on a deferred connector and the
// host's asynchronous answer resumes it, through the production runtime.wasm.
import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import {
  BundleEmbedder,
  ExactModelBrowserHost,
  ExactModelError,
  MODEL_RUNTIME_CONNECTOR,
  MODEL_RUNTIME_CONNECTOR_VERSION,
  NodeFsBundleLoader,
  exactModelMediatedConnector,
} from "../dist/index.js";
import { compileWat, writeBundleFixture } from "./fixtures.mjs";

const CONNECTOR = "traverse.test";
const REQUIREMENTS = [{ connector_id: CONNECTOR, version: "^1.0.0" }];
const REQUEST = JSON.stringify({
  abi_version: "1.0.0",
  connector_id: CONNECTOR,
  operation: "op",
  payload: { question: 1 },
});

/** Calls the connector `calls` times, writing each answer to stdout. */
function guestWat(calls) {
  const call = `
    i32.const 32 i32.const ${REQUEST.length} i32.const 1024 i32.const 4096 call $invoke local.set $len
    i32.const 4 i32.const 1024 i32.store
    i32.const 8 local.get $len i32.store
    i32.const 1 i32.const 4 i32.const 1 i32.const 12 call $write drop`;
  return `(module
    (import "wasi_snapshot_preview1" "fd_write" (func $write (param i32 i32 i32 i32) (result i32)))
    (import "traverse_host" "connector_invoke" (func $invoke (param i32 i32 i32 i32) (result i32)))
    (memory (export "memory") 1)
    (data (i32.const 32) "${REQUEST.replaceAll('"', '\\"')}")
    (func (export "_start") (local $len i32) ${call.repeat(calls)}))`;
}

async function load({ calls = 1, connectorRequirements = REQUIREMENTS } = {}) {
  const manifestPath = await writeBundleFixture({
    appId: "fixture.deferred",
    components: [
      {
        capabilityId: "fixture.deferred",
        wasmBytes: await compileWat(guestWat(calls)),
        connectorRequirements,
      },
    ],
  });
  const embedder = await BundleEmbedder.init({
    manifestPath,
    loader: new NodeFsBundleLoader(),
    platform: "web",
  });
  const events = [];
  embedder.subscribe((event) => events.push(event));
  return { embedder, events };
}

const answer = (marker) => ({
  abi_version: "1.0.0",
  result_class: "success",
  payload: { marker },
});

const terminal = (events) => events.find((e) => e.event_type === "capability_result" || e.event_type === "error");

test("submitAsync resumes a suspended guest with the connector's answer", async () => {
  const { embedder, events } = await load();
  const requests = [];
  embedder.activateMediatedConnector(CONNECTOR, "1.0.0", {
    async invoke(request) {
      requests.push(request);
      await new Promise((resolve) => setTimeout(resolve, 1));
      return answer("first");
    },
  });
  const outcome = await embedder.submitAsync("fixture.deferred", {});
  assert.equal(outcome.status, "accepted");
  assert.deepEqual(requests.map((r) => r.payload), [{ question: 1 }]);
  const result = terminal(events);
  assert.equal(result.event_type, "capability_result");
  assert.equal(result.data.output.payload.marker, "first");
});

test("a guest can suspend more than once", async () => {
  const { embedder, events } = await load({ calls: 2 });
  let call = 0;
  embedder.activateMediatedConnector(CONNECTOR, "1.0.0", {
    invoke: async () => answer(`answer-${(call += 1)}`),
  });
  await embedder.submitAsync("fixture.deferred", {});
  const output = JSON.stringify(terminal(events).data.output);
  assert.equal(call, 2);
  assert.ok(output.includes("answer-1") && output.includes("answer-2"), output);
});

test("sync submit refuses a target with an active deferred connector", async () => {
  const { embedder } = await load();
  embedder.activateMediatedConnector(CONNECTOR, "1.0.0", { invoke: async () => answer("x") });
  const outcome = embedder.submit("fixture.deferred", {});
  assert.equal(outcome.status, "rejected");
  assert.equal(outcome.error.code, "async_submit_required");
});

test("deactivating a connector makes it unavailable again", async () => {
  const { embedder, events } = await load();
  const deactivate = embedder.activateMediatedConnector(CONNECTOR, "1.0.0", {
    invoke: async () => assert.fail("must not be called"),
  });
  deactivate();
  assert.equal(embedder.submit("fixture.deferred", {}).status, "accepted");
  assert.equal(terminal(events).event_type, "capability_result");
});

test("a connector that throws reaches the guest as a failed call", async () => {
  const { embedder, events } = await load();
  embedder.activateMediatedConnector(CONNECTOR, "1.0.0", {
    invoke: async () => {
      throw new Error("host could not answer");
    },
  });
  await embedder.submitAsync("fixture.deferred", {});
  assert.equal(terminal(events).event_type, "capability_result");
});

test("an answer holding host-private data never reaches the guest", async () => {
  const { embedder, events } = await load();
  embedder.activateMediatedConnector(CONNECTOR, "1.0.0", {
    invoke: async () => ({ ...answer("x"), payload: { secret: "do-not-leak" } }),
  });
  await embedder.submitAsync("fixture.deferred", {});
  assert.ok(!JSON.stringify(terminal(events)).includes("do-not-leak"));
});

test("cancelling while suspended drops the guest and reports cancelled", async () => {
  const { embedder } = await load();
  const controller = new AbortController();
  embedder.activateMediatedConnector(CONNECTOR, "1.0.0", {
    async invoke() {
      controller.abort();
      return answer("late");
    },
  });
  const outcome = await embedder.submitAsync("fixture.deferred", {}, { signal: controller.signal });
  assert.equal(outcome.status, "cancelled");
});

test("a connector the contract doesn't require stays out of the guest's reach", async () => {
  const { embedder } = await load({ connectorRequirements: [] });
  embedder.activateMediatedConnector(CONNECTOR, "1.0.0", {
    invoke: async () => assert.fail("must not be called"),
  });
  assert.equal(embedder.submit("fixture.deferred", {}).status, "accepted");
});

const modelRequest = {
  abi_version: "1.0.0",
  connector_id: MODEL_RUNTIME_CONNECTOR,
  operation: "model.execute",
  payload: { model_ref: { model_id: "m", version: "1.0.0", digest: "sha256:ab" } },
};

test("the model binding answers with the native binding's success shape", async () => {
  const seen = [];
  const execution = {
    output_ref: "output-1",
    output_schema_ref: "schema:out",
    model_ref: modelRequest.payload.model_ref,
    placement: "wasm-cpu",
    model_evidence: { model_id: "m" },
  };
  const connector = exactModelMediatedConnector(
    { execute: async (args) => (seen.push(args), execution) },
    { allowedClassifications: ["public"] },
  );
  const response = await connector.invoke(modelRequest, {});
  assert.deepEqual(seen[0].allowed_classifications, ["public"]);
  assert.equal(response.result_class, "success");
  assert.deepEqual(response.payload, {
    status: "ok",
    output_ref: "output-1",
    output_schema_ref: "schema:out",
    model_ref: modelRequest.payload.model_ref,
    placement: "wasm-cpu",
    result_class: "success",
    model_evidence: { model_id: "m" },
  });
  assert.equal(MODEL_RUNTIME_CONNECTOR_VERSION, "2.3.0");
});

test("the model binding reports a model failure the way the native binding does", async () => {
  const connector = exactModelMediatedConnector(
    {
      execute: async () => {
        throw new ExactModelError("model_incompatible", "usage is undeclared", "usage_undeclared");
      },
    },
    { allowedClassifications: [] },
  );
  const response = await connector.invoke(modelRequest, {});
  assert.equal(response.result_class, "usage_undeclared");
  assert.deepEqual(response.payload, {
    status: "error",
    error: { code: "model_incompatible", message: "usage is undeclared", reason: "usage_undeclared" },
  });
});

test("the model binding refuses anything but model.execute and rethrows host bugs", async () => {
  const connector = exactModelMediatedConnector(
    {
      execute: async () => {
        throw new TypeError("bug");
      },
    },
    { allowedClassifications: [] },
  );
  const refused = await connector.invoke({ ...modelRequest, operation: "other" }, {});
  assert.equal(refused.result_class, "incompatible");
  await assert.rejects(connector.invoke(modelRequest, {}), TypeError);
});

// #1719: the published catalog capability, unchanged, over a signed package.
const MODELS = new URL("../../../../fixtures/models/", import.meta.url);
const CROSS_HOST = new URL("../../../../fixtures/cross-host/model-execute-v1/", import.meta.url);
const crossHostJson = (name) => JSON.parse(readFileSync(new URL(name, CROSS_HOST), "utf8"));

async function runCatalogModelExecute({ modelUsage, bound }) {
  const vector = JSON.parse(readFileSync(new URL("conformance/signed-digits-onnx.json", MODELS), "utf8"));
  const file = (name) => new Uint8Array(readFileSync(new URL(`${vector.package_dir}/${name}`, MODELS)));
  const host = new ExactModelBrowserHost([vector.pin], {
    trustedPublicKeysHex: [vector.trusted_public_key_hex],
    modelUsage,
  });
  // Without a declared usage, registration is what fails closed.
  await host
    .registerPackage(file("model.manifest.json"), file("model.wasm"), file("model.sig.json"), {
      policy_ref: "policy:default-wasm-cpu",
    })
    .then(
      () => assert.notEqual(modelUsage, undefined),
      (error) => {
        assert.equal(modelUsage, undefined, String(error));
        assert.equal(error.reason, "usage_undeclared");
      },
    );
  const manifestPath = await writeBundleFixture({
    appId: "fixture.catalog",
    components: [
      {
        capabilityId: "model.execute",
        wasmBytes: new Uint8Array(readFileSync(new URL("model-execute-agent.wasm", CROSS_HOST))),
        connectorRequirements: [{ connector_id: MODEL_RUNTIME_CONNECTOR, version: "2.3.0" }],
      },
    ],
  });
  const embedder = await BundleEmbedder.init({ manifestPath, loader: new NodeFsBundleLoader(), platform: "web" });
  const events = [];
  embedder.subscribe((event) => events.push(event));
  if (bound) {
    embedder.activateMediatedConnector(
      MODEL_RUNTIME_CONNECTOR,
      MODEL_RUNTIME_CONNECTOR_VERSION,
      exactModelMediatedConnector(host, { allowedClassifications: ["public"] }),
    );
  }
  const [testCase] = vector.cases;
  const input = crossHostJson("input.json");
  assert.equal(host.io.stageModelInput(Buffer.from(testCase.input_frame_hex, "hex"), 4096), input.input_ref);
  await embedder.submitAsync("model.execute", input);
  const result = terminal(events);
  assert.equal(result.event_type, "capability_result");
  return { output: result.data.output, host, testCase };
}

test("catalog model.execute matches the cross-host vector with no app-side shim", async () => {
  const { output, host, testCase } = await runCatalogModelExecute({
    modelUsage: "non_commercial",
    bound: true,
  });
  assert.deepEqual(output, crossHostJson("expected-output.json"));
  const frame = host.io.readModelOutput(output.output_ref, 56);
  assert.equal(Buffer.from(frame).toString("hex"), testCase.output_frame_hex);
});

test("catalog model.execute fails closed with the cross-host reasons", async () => {
  const unbound = await runCatalogModelExecute({ modelUsage: "non_commercial", bound: false });
  assert.equal(unbound.output.status, "error");
  assert.equal(unbound.output.reason, "connector_unavailable");
  const undeclared = await runCatalogModelExecute({ modelUsage: undefined, bound: true });
  assert.equal(undeclared.output.status, "error");
  assert.equal(undeclared.output.error.code, "model_unavailable");
});

/** One `fail-closed.json` case: the registration's reason and the guest's output. */
async function runFailClosedCase(testCase) {
  const vector = JSON.parse(readFileSync(new URL("conformance/signed-digits-onnx.json", MODELS), "utf8"));
  const file = (name) => new Uint8Array(readFileSync(new URL(`${vector.package_dir}/${name}`, MODELS)));
  const pin = testCase.pin_digest ? { ...vector.pin, digest: testCase.pin_digest } : vector.pin;
  const host = new ExactModelBrowserHost([pin], {
    trustedPublicKeysHex: testCase.trusted === false ? [] : [vector.trusted_public_key_hex],
    modelUsage: "non_commercial",
  });
  const wasm = file("model.wasm");
  if (testCase.tamper_wasm) {
    wasm[wasm.length - 1] ^= 1;
  }
  const reason = await host
    .registerPackage(file("model.manifest.json"), wasm, file("model.sig.json"), {
      policy_ref: "policy:default-wasm-cpu",
    })
    .then(
      () => null,
      (error) => error.reason,
    );
  if (testCase.revoke) {
    host.setPackageStatus({ [vector.pin.digest]: { status: "revoked", reason: "fixture" } });
  }
  const manifestPath = await writeBundleFixture({
    appId: "fixture.catalog",
    components: [
      {
        capabilityId: "model.execute",
        wasmBytes: new Uint8Array(readFileSync(new URL("model-execute-agent.wasm", CROSS_HOST))),
        connectorRequirements: [{ connector_id: MODEL_RUNTIME_CONNECTOR, version: "2.3.0" }],
      },
    ],
  });
  const embedder = await BundleEmbedder.init({ manifestPath, loader: new NodeFsBundleLoader(), platform: "web" });
  const events = [];
  embedder.subscribe((event) => events.push(event));
  embedder.activateMediatedConnector(
    MODEL_RUNTIME_CONNECTOR,
    MODEL_RUNTIME_CONNECTOR_VERSION,
    exactModelMediatedConnector(host, { allowedClassifications: ["public"] }),
  );
  const input = crossHostJson("input.json");
  assert.equal(host.io.stageModelInput(Buffer.from(vector.cases[0].input_frame_hex, "hex"), 4096), input.input_ref);
  await embedder.submitAsync("model.execute", input);
  const result = terminal(events);
  assert.equal(result.event_type, "capability_result");
  return { reason, output: result.data.output };
}

for (const testCase of crossHostJson("fail-closed.json").cases) {
  test(`catalog model.execute fails closed like native: ${testCase.name}`, async () => {
    const { reason, output } = await runFailClosedCase(testCase);
    assert.equal(reason, testCase.registration_reason);
    assert.deepEqual(output, testCase.expected_output);
  });
}
