import test from "node:test";
import assert from "node:assert/strict";
import {
  ExactModelBrowserHost,
  ExactModelError,
  ModelIoStore,
  encodeGuestFrame,
  normalizeModelExecuteEvidence,
  PLACEMENT_WASM_CPU,
} from "../dist/index.js";

// fixtures/models/fixture-echo-1.0.0/model.wasm
const FIXTURE_WASM_B64 =
  "AGFzbQEAAAABCQFgBH9/f38BfwMCAQAFAwEAAgcaAgZtZW1vcnkCAA1tb2RlbF9leGVjdXRlAAAKPgE8AQJ/IAEhBSAFIANLBEAgAyEFCwJAA0AgBCAFTw0BIAIgBGogACAEai0AADoAACAEQQFqIQQMAAsLIAUL";

// fixtures/models/fixture-classifier-1.0.0/model.wasm — a fixed-weight linear
// classifier over 4 f32 features; proves real computed inference, not a
// pass-through, through the same browser wasm-cpu guest path as the echo fixture.
const FIXTURE_CLASSIFIER_WASM_B64 =
  "AGFzbQEAAAABCQFgBH9/f38BfwMCAQAFAwEAAgcaAgZtZW1vcnkCAA1tb2RlbF9leGVjdXRlAAAKrAEBqQEBBn0gAUEcSQRAQX8PCyADQRRJBEBBfw8LIAAqAgwhBCAAKgIQIQUgACoCFCEGIAAqAhghByAEQwAAAD+UIAVDAACAvpSSIAZDAACAP5QgB0MAAEA/lJKSQwAAAD+TIQhDAACAP0MAAAAAIAhDAAAAAGAbIQkgAkEBOwEAIAJBAzoAAiACQQE6AAMgAkECNgIEIAJBCDYCCCACIAg4AgwgAiAJOAIQQRQL";

// fixtures/models/fixture-responder-1.0.0/model.wasm (Spec 045/138 bridge,
// #1455) — scans the prompt payload for the keyword "hi" and returns one of
// two fixed text responses; proves the real, checked-in bridge fixture
// resolves end-to-end through the same browser wasm-cpu guest path.
const FIXTURE_RESPONDER_WASM_B64 =
  "AGFzbQEAAAABCQFgBH9/f38BfwMCAQAFAwEAAgcaAgZtZW1vcnkCAA1tb2RlbF9leGVjdXRlAAAK9AEB8QEBB38gAUEMSQRAQX8PCyAAKAIIIQQgAEEMaiEFQQwgBGogAUsEQEF/DwtBACEIQQAhBgJAA0AgBkECaiAESw0BIAUgBmotAABB6ABGIAUgBkEBamotAABB6QBGcQRAQQEhCAwCCyAGQQFqIQYMAAsLIAhBAUYEQEHKuAIhCUEIIQoFQd64AiEJQQMhCgtBDCAKaiADSwRAQX8PCyACQQE7AQAgAkEEOgACIAJBAToAAyACIAo2AgQgAiAKNgIIQQAhBwJAA0AgByAKTw0BIAJBDGogB2ogCSAHai0AADoAACAHQQFqIQcMAAsLQQwgCmoLCyMDAEHAuAILAmhpAEHKuAILCGhpIHRoZXJlAEHeuAILA2htbQ==";

function b64ToBytes(value) {
  return Uint8Array.from(Buffer.from(value, "base64"));
}

async function sha256Hex(bytes) {
  const hash = await crypto.subtle.digest("SHA-256", bytes);
  return [...new Uint8Array(hash)].map((b) => b.toString(16).padStart(2, "0")).join("");
}

test("browser wasm-cpu echo fixture matches native envelope normalization", async () => {
  const wasm = b64ToBytes(FIXTURE_WASM_B64);
  const digest = await sha256Hex(wasm);
  const host = new ExactModelBrowserHost([
    {
      model_id: "fixture.echo",
      version: "1.0.0",
      digest,
      offline_allowed: true,
    },
  ]);
  await host.insertVerified(
    {
      model_id: "fixture.echo",
      version: "1.0.0",
      wasm_digest: digest,
      package_digest: digest,
      input_schema_ref: "schema:fixture-in",
      input_schema_version: "1.0.0",
      max_memory_bytes: 131072,
      max_fuel: 1_000_000,
      max_input_bytes: 4096,
      max_output_bytes: 4096,
      offline_allowed: true,
      supported_profiles: [PLACEMENT_WASM_CPU],
      license_id: "Apache-2.0",
      abi_version: 1,
    },
    wasm,
  );

  const frame = encodeGuestFrame(1, [4], new TextEncoder().encode("test"));
  const input_ref = host.io.stageModelInput(frame, 4096);
  const result = await host.execute({
    model_ref: { model_id: "fixture.echo", version: "1.0.0", digest },
    input_ref,
    policy_ref: "policy-1",
    data_classification: "sensitive",
    input_schema_ref: "schema:fixture-in",
    input_schema_version: "1.0.0",
    max_output_bytes: 4096,
    allowed_classifications: ["sensitive"],
  });
  assert.equal(result.placement, PLACEMENT_WASM_CPU);
  const output = host.io.readModelOutput(result.output_ref, 4096);
  assert.deepEqual([...output], [...frame]);

  const browserEvidence = normalizeModelExecuteEvidence({
    status: "ok",
    model_ref: { model_id: "fixture.echo", version: "1.0.0", digest },
    placement: result.placement,
    output_ref: result.output_ref,
  });
  const nativeEvidence = normalizeModelExecuteEvidence({
    status: "ok",
    model_ref: { model_id: "fixture.echo", version: "1.0.0", digest },
    placement: PLACEMENT_WASM_CPU,
    artifact_ref: "output-1",
  });
  assert.equal(browserEvidence.governing_spec, nativeEvidence.governing_spec);
  assert.equal(browserEvidence.status, nativeEvidence.status);
  assert.equal(browserEvidence.placement, nativeEvidence.placement);
  assert.equal(browserEvidence.has_output_ref, true);
});

test("browser wasm-cpu classifier fixture computes real inference, not a pass-through", async () => {
  const wasm = b64ToBytes(FIXTURE_CLASSIFIER_WASM_B64);
  const digest = await sha256Hex(wasm);
  const host = new ExactModelBrowserHost([
    {
      model_id: "fixture.classifier",
      version: "1.0.0",
      digest,
      offline_allowed: true,
    },
  ]);
  await host.insertVerified(
    {
      model_id: "fixture.classifier",
      version: "1.0.0",
      wasm_digest: digest,
      package_digest: digest,
      input_schema_ref: "schema:fixture-classifier-in",
      input_schema_version: "1.0.0",
      max_memory_bytes: 131072,
      max_fuel: 1_000_000,
      max_input_bytes: 4096,
      max_output_bytes: 4096,
      offline_allowed: true,
      supported_profiles: [PLACEMENT_WASM_CPU],
      license_id: "Apache-2.0",
      abi_version: 1,
    },
    wasm,
  );

  const features = Float32Array.from([1.0, 2.0, -1.0, 4.0]);
  const payload = new Uint8Array(features.buffer);
  const frame = encodeGuestFrame(2, [4], payload);
  const input_ref = host.io.stageModelInput(frame, 4096);
  const result = await host.execute({
    model_ref: { model_id: "fixture.classifier", version: "1.0.0", digest },
    input_ref,
    policy_ref: "policy-1",
    data_classification: "sensitive",
    input_schema_ref: "schema:fixture-classifier-in",
    input_schema_version: "1.0.0",
    max_output_bytes: 4096,
    allowed_classifications: ["sensitive"],
  });
  assert.equal(result.placement, PLACEMENT_WASM_CPU);
  const output = host.io.readModelOutput(result.output_ref, 4096);
  assert.notDeepEqual([...output], [...frame], "classifier output must not echo the input");

  const view = new DataView(output.buffer, output.byteOffset, output.byteLength);
  assert.equal(view.getUint16(0, true), 1); // abi_version
  assert.equal(output[2], 3); // dtype
  assert.equal(output[3], 1); // rank
  assert.equal(view.getUint32(4, true), 2); // dims = [2]
  assert.equal(view.getUint32(8, true), 8); // payload_len
  // 0.5*1.0 - 0.25*2.0 + 1.0*-1.0 + 0.75*4.0 - 0.5 == 1.5
  assert.ok(Math.abs(view.getFloat32(12, true) - 1.5) < 1e-6);
  assert.ok(Math.abs(view.getFloat32(16, true) - 1.0) < 1e-6);
});

async function runResponderPrompt(promptText) {
  const wasm = b64ToBytes(FIXTURE_RESPONDER_WASM_B64);
  const digest = await sha256Hex(wasm);
  const host = new ExactModelBrowserHost([
    {
      model_id: "fixture.responder",
      version: "1.0.0",
      digest,
      offline_allowed: true,
    },
  ]);
  await host.insertVerified(
    {
      model_id: "fixture.responder",
      version: "1.0.0",
      wasm_digest: digest,
      package_digest: digest,
      input_schema_ref: "schema:bridged-generate-in",
      input_schema_version: "1.0.0",
      max_memory_bytes: 131072,
      max_fuel: 1_000_000,
      max_input_bytes: 4096,
      max_output_bytes: 4096,
      offline_allowed: true,
      supported_profiles: [PLACEMENT_WASM_CPU],
      license_id: "Apache-2.0",
      abi_version: 1,
    },
    wasm,
  );

  const promptBytes = new TextEncoder().encode(promptText);
  const frame = encodeGuestFrame(4, [promptBytes.length], promptBytes);
  const input_ref = host.io.stageModelInput(frame, 4096);
  const result = await host.execute({
    model_ref: { model_id: "fixture.responder", version: "1.0.0", digest },
    input_ref,
    policy_ref: "policy-1",
    data_classification: "sensitive",
    input_schema_ref: "schema:bridged-generate-in",
    input_schema_version: "1.0.0",
    max_output_bytes: 4096,
    allowed_classifications: ["sensitive"],
  });
  assert.equal(result.placement, PLACEMENT_WASM_CPU);
  const output = host.io.readModelOutput(result.output_ref, 4096);
  const view = new DataView(output.buffer, output.byteOffset, output.byteLength);
  const respLen = view.getUint32(8, true);
  const response = new TextDecoder().decode(output.slice(12, 12 + respLen));
  return response;
}

test("browser wasm-cpu bridge fixture (#1455) returns the keyword-matched response", async () => {
  const response = await runResponderPrompt("hi there, how are you?");
  assert.equal(response, "hi there");
});

test("browser wasm-cpu bridge fixture (#1455) returns the fallback response", async () => {
  const response = await runResponderPrompt("what is the weather today");
  assert.equal(response, "hmm");
});

test("browser offline cache miss is model_unavailable", async () => {
  const host = new ExactModelBrowserHost([
    {
      model_id: "fixture.echo",
      version: "1.0.0",
      digest: "deadbeef",
      offline_allowed: true,
    },
  ]);
  const input_ref = host.io.stageModelInput(new Uint8Array([1, 2, 3]), 64);
  await assert.rejects(
    () =>
      host.execute({
        model_ref: { model_id: "fixture.echo", version: "1.0.0", digest: "deadbeef" },
        input_ref,
        policy_ref: "policy-1",
        data_classification: "sensitive",
        input_schema_ref: "schema:fixture-in",
        input_schema_version: "1.0.0",
        max_output_bytes: 64,
        allowed_classifications: ["sensitive"],
      }),
    (error) => error.code === "model_unavailable",
  );
});

test("artifact refs are multi-read, bounded, opaque, and separate from model refs", () => {
  const io = new ModelIoStore();
  assert.throws(() => io.stageArtifact(new Uint8Array(0), 8), (e) => e instanceof ExactModelError && e.code === "input_limit_exceeded");
  assert.throws(() => io.stageArtifact(new Uint8Array(6), 4), (e) => e.code === "input_limit_exceeded");
  const bytes = Uint8Array.from([1, 2, 3]);
  const ref = io.stageArtifact(bytes, 8);
  assert.equal(ref, "artifact-1");
  assert.ok(!ref.includes("/") && !ref.includes(":"));
  // Bytes are copied in and out: caller mutation never reaches the store.
  bytes[0] = 9;
  const first = io.readArtifact(ref, 8);
  first[1] = 9;
  assert.deepEqual([...io.readArtifact(ref, 8)], [1, 2, 3]);
  assert.throws(() => io.readArtifact(ref, 2), (e) => e.code === "input_limit_exceeded");
  assert.throws(() => io.readArtifact("artifact-9", 8), (e) => e.code === "unavailable");
  const inputRef = io.stageModelInput(Uint8Array.from([1]), 8);
  assert.throws(() => io.readArtifact(inputRef, 8), (e) => e.code === "unavailable");
  io.takeInput(inputRef);
  assert.throws(() => io.takeInput(inputRef), (e) => e.code === "invalid_input");
  assert.deepEqual([...io.readArtifact(ref, 8)], [1, 2, 3]);
  io.dropRef(ref);
  assert.throws(() => io.readArtifact(ref, 8), (e) => e.code === "unavailable");
});

test("shutdown invalidates every staged ref", () => {
  const io = new ModelIoStore();
  const artifactRef = io.stageArtifact(Uint8Array.from([1]), 8);
  const inputRef = io.stageModelInput(Uint8Array.from([1]), 8);
  const outputRef = io.putOutput(Uint8Array.from([1]));
  io.shutdown();
  assert.throws(() => io.readArtifact(artifactRef, 8), (e) => e.code === "unavailable");
  assert.throws(() => io.takeInput(inputRef), (e) => e.code === "invalid_input");
  assert.throws(() => io.readModelOutput(outputRef, 8), (e) => e.code === "unavailable");
});
