// Shared Spec 138 rights conformance suite (0.8.0, Decision 107, FR-041):
// the browser host runs every data-only case in
// fixtures/models/rights-conformance/suite.json on a fresh host per case and
// must match the native results (code, reason, detail, rights records,
// model_evidence) exactly.
import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { ExactModelBrowserHost, ExactModelError } from "../dist/index.js";

const ROOT = new URL("../../../../", import.meta.url);
const read = (path) => new Uint8Array(readFileSync(new URL(path, ROOT)));
const SUITE = JSON.parse(readFileSync(new URL("fixtures/models/rights-conformance/suite.json", ROOT), "utf8"));
const toHex = (bytes) => [...bytes].map((b) => b.toString(16).padStart(2, "0")).join("");
const fromHex = (hex) => Uint8Array.from(hex.match(/../g).map((pair) => parseInt(pair, 16)));

/** The public error shape every embedder compares: code, reason, detail. */
function errorJson(error) {
  assert.ok(error instanceof ExactModelError, `unexpected error: ${error}`);
  return {
    ok: false,
    code: error.code,
    reason: error.reason ?? null,
    ...(error.detail ? { detail: error.detail } : {}),
  };
}

/** Compare as JSON so key order and undefined-vs-absent match native serde output. */
const asJson = (value) => JSON.parse(JSON.stringify(value));

function hostFor(testCase) {
  const host = new ExactModelBrowserHost(testCase.pins, {
    trustedPublicKeysHex: [SUITE.trusted_public_key_hex],
    ...(testCase.model_usage ? { modelUsage: testCase.model_usage } : {}),
    hostRequiresCommercial: testCase.host_requires_commercial ?? false,
  });
  host.setPackageStatus(testCase.package_status ?? {});
  return host;
}

const pinFor = (testCase, name) => testCase.pins.find((pin) => pin.model_id === `fixture.rights.${name}`);

async function register(host, step) {
  const dir = `${SUITE.package_dir}/${step.package}`;
  const manifest = read(`${dir}/model.manifest.json`);
  let wasm = read(SUITE.wasm_path);
  let signature = read(`${dir}/model.sig.json`);
  if (step.tamper === "wasm") {
    wasm = Uint8Array.from([...wasm, 0]);
  } else if (step.tamper === "signature") {
    const document = JSON.parse(new TextDecoder().decode(signature));
    const bytes = fromHex(document.signature);
    bytes[0] ^= 0x01;
    document.signature = toHex(bytes);
    signature = new TextEncoder().encode(JSON.stringify(document));
  } else {
    assert.equal(step.tamper, undefined, `unknown tamper ${step.tamper}`);
  }
  try {
    return { ok: true, digest: await host.registerPackage(manifest, wasm, signature) };
  } catch (error) {
    return errorJson(error);
  }
}

async function execute(host, testCase, step) {
  const pin = pinFor(testCase, step.package);
  const run = SUITE.execute;
  const inputRef = host.io.stageModelInput(fromHex(run.input_hex), 4096);
  try {
    const result = await host.execute({
      model_ref: { model_id: pin.model_id, version: pin.version, digest: pin.digest },
      input_ref: inputRef,
      policy_ref: run.policy_ref,
      data_classification: run.data_classification,
      input_schema_ref: run.input_schema_ref,
      input_schema_version: run.input_schema_version,
      max_output_bytes: run.max_output_bytes,
      allowed_classifications: run.allowed_classifications,
    });
    assert.deepEqual(result.trace.model_evidence, result.model_evidence);
    return {
      ok: true,
      output_hex: toHex(host.io.readModelOutput(result.output_ref, 4096)),
      model_evidence: result.model_evidence,
    };
  } catch (error) {
    return errorJson(error);
  }
}

test("rights conformance suite passes on the browser host (native parity)", async () => {
  const scenarios = new Set([10]);
  for (const testCase of SUITE.cases) {
    const host = hostFor(testCase);
    for (const [index, step] of testCase.steps.entries()) {
      let actual;
      if (step.op === "register") {
        actual = await register(host, step);
      } else if (step.op === "execute") {
        actual = await execute(host, testCase, step);
      } else if (step.op === "rights_record") {
        actual = host.modelRightsRecord(pinFor(testCase, step.package).digest) ?? null;
      } else {
        assert.equal(step.op, "set_package_status", `${testCase.id}: unknown op`);
        host.setPackageStatus(step.entries);
        continue;
      }
      assert.deepEqual(asJson(actual), step.expect, `${testCase.id} step ${index}`);
    }
    testCase.scenarios.forEach((scenario) => scenarios.add(scenario));
  }
  assert.deepEqual([...scenarios].sort((a, b) => a - b), [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
});

test("rights record is undefined until registered and usage is declared", async () => {
  const testCase = SUITE.cases.find((candidate) => candidate.id === "permissive-commercial");
  const host = hostFor(testCase);
  const pin = testCase.pins[0];
  assert.equal(host.modelRightsRecord(pin.digest), undefined);
  await register(host, { package: "permissive" });
  assert.equal(host.modelRightsRecord(`sha256:${pin.digest}`).status, "active");
  const undeclared = new ExactModelBrowserHost(testCase.pins, { trustedPublicKeysHex: [SUITE.trusted_public_key_hex] });
  assert.equal(undeclared.modelRightsRecord(pin.digest), undefined);
  assert.throws(() => undeclared.effectiveUsage(), (error) => error.reason === "usage_undeclared" && !error.detail.model_id);
  const tightened = new ExactModelBrowserHost(testCase.pins, {
    trustedPublicKeysHex: [SUITE.trusted_public_key_hex],
    modelUsage: "non_commercial",
    hostRequiresCommercial: true,
  });
  assert.equal(tightened.effectiveUsage(), "commercial");
});
