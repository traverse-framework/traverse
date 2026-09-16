import test from "node:test";
import assert from "node:assert/strict";
import {
  ExactModelBrowserHost,
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
