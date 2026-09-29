#!/usr/bin/env node
// Regenerates the signed Spec 138 model-package fixtures (Decision 101):
// schema 2.0.0 manifests, detached Ed25519 `model.sig.json` over the exact
// manifest bytes, and the native/browser conformance vector. Deterministic:
// same WASM + same test key => byte-identical output (Ed25519 is RFC 8032).
//
// The key is TEST-ONLY. No embedder trusts it by default.
//
//   node scripts/fixtures/sign-model-fixtures.mjs
import { createHash, createPrivateKey, createPublicKey, sign } from "node:crypto";
import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const modelsDir = join(root, "fixtures", "models");
const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");

const seed = createHash("sha256").update("traverse-test-model-signing-key-v1").digest();
const privateKey = createPrivateKey({
  key: Buffer.concat([Buffer.from("302e020100300506032b657004220420", "hex"), seed]),
  format: "der",
  type: "pkcs8",
});
const publicKey = Buffer.from(createPublicKey(privateKey).export({ format: "jwk" }).x, "base64url");
const keyId = `ed25519:${sha256(publicKey)}`;

writeJson(join(modelsDir, "test-signing-key.json"), {
  warning: "TEST-ONLY Spec 138 fixture signing key. Never trust it in a real host.",
  alg: "ed25519",
  key_id: keyId,
  public_key_hex: publicKey.toString("hex"),
  secret_key_hex: seed.toString("hex"),
});

const common = {
  registry: "registry",
  redistribution: "test-only; not for production redistribution claims",
};
const fixtures = [
  {
    dir: "fixture-echo-1.0.0",
    model_id: "fixture.echo",
    attribution: "Traverse Spec 138 conformance fixture",
    input: ["schema:fixture-in", "schema:fixture-out"],
  },
  {
    dir: "fixture-classifier-1.0.0",
    model_id: "fixture.classifier",
    attribution: "Traverse Spec 138 real-inference conformance fixture",
    input: ["schema:fixture-classifier-in", "schema:fixture-classifier-out"],
  },
  {
    dir: "fixture-responder-1.0.0",
    model_id: "fixture.responder",
    attribution: "Traverse Spec 045/138 bridge conformance fixture",
    input: ["schema:bridged-generate-in", "schema:bridged-generate-out"],
  },
  // First trained model (Decision 102, #1461). model.wasm is built from
  // crates/traverse-digits-mlp-guest; limits come from measured usage
  // (~56k fuel per inference, one 64 KiB page, 268-byte input, 56-byte output).
  {
    dir: "digits-mlp-1.0.0",
    model_id: "traverse.digits-mlp",
    attribution:
      "Weights trained by Traverse on \"Optical Recognition of Handwritten Digits\" by E. Alpaydin and C. Kaynak, UCI Machine Learning Repository, https://doi.org/10.24432/C50P49 (CC BY 4.0)",
    input: ["schema:traverse-digits-mlp-in", "schema:traverse-digits-mlp-out"],
    rights: {
      license_id: "CC-BY-4.0",
      redistribution:
        "Redistribution permitted under CC BY 4.0 with the attribution above; signed with the test-only key (no production trust)",
      commercial_use: "allowed",
      source_url: "https://archive.ics.uci.edu/dataset/80/optical+recognition+of+handwritten+digits",
    },
    limits: {
      max_memory_bytes: 131072,
      max_fuel: 200000,
      max_input_bytes: 268,
      max_output_bytes: 64,
      max_execution_ms: 1000,
    },
  },
];

const pins = {};
for (const fixture of fixtures) {
  const dir = join(modelsDir, fixture.dir);
  const wasm = readFileSync(join(dir, "model.wasm"));
  const manifest = {
    schema_version: "2.0.0",
    model_id: fixture.model_id,
    version: "1.0.0",
    wasm_digest: sha256(wasm),
    registry_ref: `${common.registry}:${fixture.model_id}@1.0.0`,
    executable_format: "traverse-model-wasm",
    abi_version: 1,
    input_schema_ref: fixture.input[0],
    input_schema_version: "1.0.0",
    output_schema_ref: fixture.input[1],
    output_schema_version: "1.0.0",
    rights: {
      license_id: fixture.rights?.license_id ?? "Apache-2.0",
      attribution: fixture.attribution,
      redistribution: fixture.rights?.redistribution ?? common.redistribution,
      commercial_use: fixture.rights?.commercial_use ?? "allowed",
      source_url:
        fixture.rights?.source_url ??
        `https://github.com/traverse-framework/Traverse/tree/main/fixtures/models/${fixture.dir}`,
    },
    supported_profiles: ["wasm-cpu"],
    ...(fixture.limits ?? {
      max_memory_bytes: 131072,
      max_fuel: 1000000,
      max_input_bytes: 4096,
      max_output_bytes: 4096,
      max_execution_ms: 5000,
    }),
    offline_allowed: true,
  };
  const manifestBytes = Buffer.from(`${JSON.stringify(manifest, null, 2)}\n`);
  writeFileSync(join(dir, "model.manifest.json"), manifestBytes);
  writeJson(join(dir, "model.sig.json"), {
    alg: "ed25519",
    key_id: keyId,
    signature: sign(null, manifestBytes, privateKey).toString("hex"),
  });
  pins[fixture.model_id] = {
    model_id: fixture.model_id,
    version: "1.0.0",
    digest: sha256(manifestBytes),
    offline_allowed: true,
    target: "wasm-cpu",
    rights: {
      license_id: fixture.rights?.license_id ?? "Apache-2.0",
      commercial_use: fixture.rights?.commercial_use ?? "allowed",
    },
    key_id: keyId,
  };
}

// Native/browser conformance vector: same signed package, same request.
const classifierWasm = readFileSync(join(modelsDir, "fixture-classifier-1.0.0", "model.wasm"));
const input = guestFrame(2, [4], f32Bytes([1.0, 0.5, 0.25, 0.1]));
const output = runGuest(classifierWasm, input, 4096);
mkdirSync(join(modelsDir, "conformance"), { recursive: true });
writeJson(join(modelsDir, "conformance", "signed-classifier.json"), {
  governing_spec: "138-governed-exact-model-execution",
  package_dir: "fixture-classifier-1.0.0",
  trusted_public_key_hex: publicKey.toString("hex"),
  pin: pins["fixture.classifier"],
  request: {
    policy_ref: "policy-1",
    data_classification: "sensitive",
    input_schema_ref: "schema:fixture-classifier-in",
    input_schema_version: "1.0.0",
    max_output_bytes: 4096,
    input_frame_hex: Buffer.from(input).toString("hex"),
  },
  expected: {
    placement: "wasm-cpu",
    output_frame_hex: Buffer.from(output).toString("hex"),
  },
});

// Trained digits MLP vector: the first 10 held-out test rows through the
// signed package; native and browser must match byte-for-byte.
const digitsWasm = readFileSync(join(modelsDir, "digits-mlp-1.0.0", "model.wasm"));
const testRows = readFileSync(join(root, "fixtures", "datasets", "uci-optdigits", "optdigits.tes"), "utf8")
  .trim()
  .split("\n")
  .slice(0, 10)
  .map((line) => line.split(",").map(Number));
writeJson(join(modelsDir, "conformance", "signed-digits-mlp.json"), {
  governing_spec: "138-governed-exact-model-execution",
  package_dir: "digits-mlp-1.0.0",
  trusted_public_key_hex: publicKey.toString("hex"),
  pin: pins["traverse.digits-mlp"],
  request: {
    policy_ref: "policy-1",
    data_classification: "sensitive",
    input_schema_ref: "schema:traverse-digits-mlp-in",
    input_schema_version: "1.0.0",
    max_output_bytes: 64,
  },
  cases: testRows.map((row) => {
    const label = row.pop();
    const frame = guestFrame(2, [64], f32Bytes(row));
    const out = Buffer.from(runGuest(digitsWasm, frame, 64));
    return {
      label,
      predicted: out.readFloatLE(52),
      input_frame_hex: frame.toString("hex"),
      output_frame_hex: out.toString("hex"),
    };
  }),
});

function writeJson(path, value) {
  writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`);
}

function f32Bytes(values) {
  const out = Buffer.alloc(values.length * 4);
  values.forEach((value, index) => out.writeFloatLE(value, index * 4));
  return out;
}

function guestFrame(dtype, dims, payload) {
  const header = Buffer.alloc(4 + dims.length * 4 + 4);
  header.writeUInt16LE(1, 0);
  header[2] = dtype;
  header[3] = dims.length;
  dims.forEach((dim, index) => header.writeUInt32LE(dim, 4 + index * 4));
  header.writeUInt32LE(payload.length, 4 + dims.length * 4);
  return Buffer.concat([header, payload]);
}

function runGuest(wasm, inputFrame, cap) {
  const instance = new WebAssembly.Instance(new WebAssembly.Module(wasm), {});
  const memory = instance.exports.memory;
  const inPtr = 64;
  const outPtr = inPtr + inputFrame.length + 64;
  new Uint8Array(memory.buffer, inPtr, inputFrame.length).set(inputFrame);
  const len = instance.exports.model_execute(inPtr, inputFrame.length, outPtr, cap);
  return new Uint8Array(memory.buffer.slice(outPtr, outPtr + len));
}
