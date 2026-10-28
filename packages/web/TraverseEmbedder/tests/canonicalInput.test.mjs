import test from "node:test";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { join } from "node:path";
import { RuntimeWasmHost, RuntimeWasmHostError } from "../dist/index.js";

// Cross-host canonical guest input (Spec 1722 FR-006, Decision 118, #1722).
//
// Runs fixtures/cross-host/canonical-input-v1 through the production RuntimeWasmHost and the
// checked-in runtime.wasm (or TRAVERSE_NATIVE_ARTIFACT_ROOT's). The stdin-echo guest returns the
// exact bytes it read, which must match the Rust reference's canonical bytes and digest
// (crates/traverse-runtime/tests/canonical_input_conformance.rs) on every host.

const here = fileURLToPath(new URL(".", import.meta.url));
const fixtureDir = join(here, "../../../../fixtures/cross-host/canonical-input-v1");
const artifactRoot = process.env.TRAVERSE_NATIVE_ARTIFACT_ROOT;
const runtimePath = artifactRoot
  ? join(artifactRoot, "runtime/runtime.wasm")
  : join(here, "fixtures/runtime.wasm");

async function load() {
  const [runtimeBytes, guest, fixture] = await Promise.all([
    readFile(runtimePath),
    readFile(join(fixtureDir, "stdin-echo.wasm")),
    readFile(join(fixtureDir, "vectors.json"), "utf8").then(JSON.parse),
  ]);
  return { runtimeBytes: new Uint8Array(runtimeBytes), guest: new Uint8Array(guest), fixture };
}

/** Submits `bytes` as capability input; returns the echoed output or the failure message. */
async function run(runtimeBytes, guest, bytes) {
  const host = await RuntimeWasmHost.instantiate(runtimeBytes);
  const init = host.init(
    {
      capabilityId: "conformance.stdin-echo",
      capabilityVersion: "1.0.0",
      serviceType: "stateless",
      emits: [],
      hostPlacementTarget: "local",
      permittedTargets: ["local"],
    },
    guest,
  );
  assert.equal(init.status, "ready");
  try {
    host.submit(bytes);
  } catch (error) {
    if (!(error instanceof RuntimeWasmHostError)) throw error;
    return { error: error.message };
  }
  const result = host.drainEvents().find((event) => event.type === "capability_result");
  assert.ok(result, "a capability_result event");
  return result.data.status === "completed"
    ? { output: result.data.output }
    : { error: String(result.data.error) };
}

function check(where, vector, outcome) {
  if (vector.expected_error) {
    assert.ok(
      outcome.error?.includes(vector.expected_error),
      `${where}/${vector.name}: expected ${vector.expected_error}, got ${JSON.stringify(outcome)}`,
    );
    return;
  }
  assert.ok(outcome.output, `${where}/${vector.name}: ${JSON.stringify(outcome)}`);
  const stdin = outcome.output.stdin;
  assert.equal(stdin, vector.expected_stdin, `${where}/${vector.name}: stdin bytes`);
  assert.equal(
    createHash("sha256").update(stdin, "utf8").digest("hex"),
    vector.expected_sha256,
    `${where}/${vector.name}: digest`,
  );
}

test("runtime.wasm feeds the guest canonical bytes for raw caller bytes", async () => {
  const { runtimeBytes, guest, fixture } = await load();
  for (const vector of fixture.vectors) {
    const outcome = await run(runtimeBytes, guest, new TextEncoder().encode(vector.input));
    check("raw", vector, outcome);
  }
});

test("a JS caller's parsed input reaches the guest as the same canonical bytes", async () => {
  // BundleEmbedder serializes a caller's JS value with JSON.stringify (executeWasmModule).
  // Duplicate member names cannot survive JSON.parse, so those vectors are bytes-only.
  const { runtimeBytes, guest, fixture } = await load();
  for (const vector of fixture.vectors.filter((candidate) => !candidate.bytes_only)) {
    const bytes = new TextEncoder().encode(JSON.stringify(JSON.parse(vector.input)));
    check("js-value", vector, await run(runtimeBytes, guest, bytes));
  }
});
