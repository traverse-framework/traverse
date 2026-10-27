#!/usr/bin/env node
/**
 * Lightweight Spec 138 direct-WebAssembly timing harness for #1657 / ADR-0083.
 *
 * Default: times checked-in fixtures (no HF downloads).
 * Full HF packages: TRAVERSE_PERF_FULL=1 and TRAVERSE_PERF_MODEL_DIRS=path1,path2
 * Optional assert mode: TRAVERSE_PERF_ASSERT=1 (successful execute only).
 *
 * Prerequisites: build the web embedder first:
 *   (cd packages/web/TraverseEmbedder && npm run build)
 */
import { createHash } from "node:crypto";
import { readFileSync, existsSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { performance } from "node:perf_hooks";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const EMBEDDER_DIST = join(ROOT, "packages/web/TraverseEmbedder/dist/index.js");
const MODELS = join(ROOT, "fixtures/models");
const TEST_KEY = JSON.parse(readFileSync(join(MODELS, "test-signing-key.json"), "utf8"));

if (!existsSync(EMBEDDER_DIST)) {
  console.error(
    "missing packages/web/TraverseEmbedder/dist — run: (cd packages/web/TraverseEmbedder && npm run build)",
  );
  process.exit(1);
}

const { ExactModelBrowserHost, encodeGuestFrame, PLACEMENT_WASM_CPU } = await import(
  pathToFileURL(EMBEDDER_DIST).href
);

function sha256Hex(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function loadPackage(dir) {
  const manifest = new Uint8Array(readFileSync(join(dir, "model.manifest.json")));
  const wasm = new Uint8Array(readFileSync(join(dir, "model.wasm")));
  const sig = new Uint8Array(readFileSync(join(dir, "model.sig.json")));
  const parsed = JSON.parse(new TextDecoder().decode(manifest));
  return { dir, manifest, wasm, sig, parsed };
}

function inputFor(pkg) {
  if (pkg.parsed.model_id === "traverse.digits-onnx" || pkg.dir.endsWith("digits-onnx-1.0.0")) {
    const vector = JSON.parse(
      readFileSync(join(MODELS, "conformance/signed-digits-onnx.json"), "utf8"),
    );
    const hex = vector.cases[0].input_frame_hex;
    return {
      bytes: Buffer.from(hex, "hex"),
      schemaRef: vector.request.input_schema_ref,
      schemaVersion: vector.request.input_schema_version,
      maxOutput: vector.request.max_output_bytes,
      pin: vector.pin,
      trustedKey: vector.trusted_public_key_hex,
    };
  }
  return {
    bytes: encodeGuestFrame(1, [4], new TextEncoder().encode("perf")),
    schemaRef: pkg.parsed.input_schema_ref,
    schemaVersion: pkg.parsed.input_schema_version,
    maxOutput: pkg.parsed.max_output_bytes ?? 4096,
    pin: null,
    trustedKey: TEST_KEY.public_key_hex,
  };
}

async function timePackage(pkg, runs = 3) {
  const prepared = inputFor(pkg);
  const digest = prepared.pin?.digest ?? sha256Hex(pkg.manifest);
  const pin = prepared.pin ?? {
    model_id: pkg.parsed.model_id,
    version: pkg.parsed.version,
    digest,
    offline_allowed: true,
    target: PLACEMENT_WASM_CPU,
    rights: {
      license_id: pkg.parsed.rights?.license_id ?? "Apache-2.0",
      commercial_use: pkg.parsed.rights?.commercial_use ?? "allowed",
    },
  };
  const host = new ExactModelBrowserHost([pin], {
    trustedPublicKeysHex: [prepared.trustedKey],
    modelUsage: "commercial",
  });
  await host.registerPackage(pkg.manifest, pkg.wasm, pkg.sig);

  const timings = [];
  let lastOutputDigest = null;
  for (let i = 0; i < runs; i += 1) {
    const inputRef = host.io.stageModelInput(
      prepared.bytes,
      pkg.parsed.max_input_bytes ?? prepared.bytes.length,
    );
    const started = performance.now();
    const result = await host.execute({
      model_ref: {
        model_id: pin.model_id,
        version: pin.version,
        digest: pin.digest,
      },
      input_ref: inputRef,
      policy_ref: "policy-perf",
      data_classification: "sensitive",
      input_schema_ref: prepared.schemaRef,
      input_schema_version: prepared.schemaVersion,
      max_output_bytes: prepared.maxOutput,
      allowed_classifications: ["sensitive"],
    });
    const ms = performance.now() - started;
    timings.push(ms);
    const out = host.io.readModelOutput(result.output_ref, prepared.maxOutput);
    lastOutputDigest = sha256Hex(out);
  }

  timings.sort((a, b) => a - b);
  return {
    model_id: pin.model_id,
    version: pin.version,
    package_dir: pkg.dir,
    path: "exact-model-direct-wasm",
    runs,
    ms_min: Number(timings[0].toFixed(3)),
    ms_median: Number(timings[Math.floor(timings.length / 2)].toFixed(3)),
    ms_max: Number(timings[timings.length - 1].toFixed(3)),
    output_sha256: lastOutputDigest,
  };
}

function defaultDirs() {
  return [
    join(MODELS, "fixture-echo-1.0.0"),
    join(MODELS, "digits-onnx-1.0.0"),
  ].filter((d) => existsSync(join(d, "model.manifest.json")));
}

function resolveDirs() {
  if (process.env.TRAVERSE_PERF_FULL === "1") {
    const raw = process.env.TRAVERSE_PERF_MODEL_DIRS ?? "";
    const dirs = raw
      .split(",")
      .map((s) => s.trim())
      .filter(Boolean)
      .map((p) => resolve(p));
    if (dirs.length === 0) {
      console.error(
        "TRAVERSE_PERF_FULL=1 requires TRAVERSE_PERF_MODEL_DIRS=comma,separated,package,dirs",
      );
      process.exit(2);
    }
    return dirs;
  }
  return defaultDirs();
}

const results = [];
for (const dir of resolveDirs()) {
  const pkg = loadPackage(dir);
  try {
    results.push(await timePackage(pkg));
  } catch (error) {
    results.push({
      model_id: pkg.parsed.model_id,
      package_dir: dir,
      path: "exact-model-direct-wasm",
      error: error instanceof Error ? error.message : String(error),
    });
  }
}

const report = {
  issue: 1657,
  adr: "0083",
  host: "ExactModelBrowserHost",
  node: process.version,
  full: process.env.TRAVERSE_PERF_FULL === "1",
  results,
};
console.log(JSON.stringify(report, null, 2));

if (process.env.TRAVERSE_PERF_ASSERT === "1") {
  const failed = results.filter((r) => r.error || typeof r.ms_median !== "number");
  if (failed.length > 0) {
    console.error("TRAVERSE_PERF_ASSERT failed:", failed);
    process.exit(1);
  }
}
