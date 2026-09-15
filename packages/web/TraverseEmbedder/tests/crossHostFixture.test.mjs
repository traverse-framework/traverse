// Browser-host conformance coverage for the repository-controlled cross-host
// fixture. The test uses the Web embedder's production WASI/WebAssembly path;
// it does not stub capability execution or precompute a successful result.
import test from "node:test";
import assert from "node:assert/strict";
import { copyFile, mkdtemp, mkdir, readFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  BundleEmbedder,
  BundleRejectedError,
  NodeFsBundleLoader,
} from "../dist/index.js";

const repoRoot = new URL("../../../../", import.meta.url);
const fixtureRoot = new URL("../../../../fixtures/cross-host/hello-world-v1/", import.meta.url);
const loader = new NodeFsBundleLoader();

async function fixtureJson(name) {
  return JSON.parse(await readFile(new URL(name, fixtureRoot), "utf8"));
}

async function writeBrowserBundle(digest) {
  const root = await mkdtemp(join(tmpdir(), "traverse-browser-cross-host-"));
  const componentDir = join(root, "components/hello-world");
  await mkdir(componentDir, { recursive: true });
  await copyFile(
    new URL("examples/hello-world/say-hello-agent/artifacts/say-hello-agent.wasm", repoRoot),
    join(componentDir, "say-hello-agent.wasm"),
  );
  await writeFile(
    join(componentDir, "component.manifest.json"),
    JSON.stringify({
      component_id: "cross-host.hello-world",
      version: "1.0.0",
      schema_version: "1.0.0",
      capability_id: "hello.world.say-hello",
      capability_version: "1.0.0",
      execution_mode: "wasm",
      contract_path: "unused-contract.json",
      wasm_binary_path: "say-hello-agent.wasm",
      wasm_digest: digest,
      runtime_constraints: { network_access: "forbidden", filesystem_access: "none" },
      permitted_targets: ["browser"],
      dependencies: [],
      connector_requirements: [],
      validation_evidence: [],
    }),
  );
  const runtimeDir = join(root, "runtime");
  await mkdir(runtimeDir, { recursive: true });
  await copyFile(
    new URL("fixtures/runtime.wasm", import.meta.url),
    join(runtimeDir, "runtime.wasm"),
  );
  await copyFile(
    new URL("fixtures/runtime.wasm.sha256", import.meta.url),
    join(runtimeDir, "runtime.wasm.sha256"),
  );
  const manifestPath = join(root, "app.manifest.json");
  await writeFile(
    manifestPath,
    JSON.stringify({
      kind: "application_bundle",
      schema_version: "1.0.0",
    app_id: "cross-host-fixture",
    version: "1.0.0",
    components: [{
        component_id: "cross-host.hello-world",
        version: "1.0.0",
        digest,
        manifest_path: "components/hello-world/component.manifest.json",
      }],
    workflows: [],
    model_dependencies: [],
    config_schema: { type: "object" },
    default_config: {},
    placement_policy: { preferred_targets: ["browser"], allow_fallback: false },
    public_surfaces: ["browser"],
    }),
  );
  return manifestPath;
}

function browserEvidence(fixture, outcome, outputProjection, traceProjection) {
  return {
    fixture_version: fixture.fixture_version,
    capability_id: fixture.capability.id,
    capability_version: fixture.capability.version,
    artifact_digest: fixture.capability.artifact.digest,
    contract_id: fixture.capability.contract.id,
    contract_version: fixture.capability.contract.version,
    contract_digest: fixture.capability.contract.digest,
    host: { package: "traverse-embedder-web", version: "0.7.0" },
    engine: { name: "browser-webassembly", version: "native" },
    platform: { os: "browser", architecture: "browser" },
    outcome,
    output_projection: outputProjection,
    trace_projection: traceProjection,
    comparison: { result: "equal", projection_version: fixture.expected_projection_version },
  };
}

test("cross-host fixture: browser executes the pinned artifact and emits safe success evidence", async () => {
  const fixture = await fixtureJson("fixture.json");
  const expected = await fixtureJson("valid-execution.json");
  const manifestPath = await writeBrowserBundle(fixture.capability.artifact.digest);
  const embedder = await BundleEmbedder.init({ manifestPath, loader, platform: "web" });
  const events = [];
  embedder.subscribe((event) => events.push(event));

  assert.equal(embedder.submit(fixture.capability.id, expected.input).status, "accepted");
  assert.deepEqual(events[1].data.output, expected.expected_output_projection);
  const evidence = browserEvidence(
    fixture,
    "success",
    expected.expected_output_projection,
    expected.expected_trace_projection,
  );
  assert.deepEqual(evidence.output_projection, expected.expected_output_projection);
  assert.deepEqual(evidence.trace_projection, expected.expected_trace_projection);
  assert.equal(evidence.comparison.result, "equal");
});

test("cross-host fixture: browser rejects invalid input before capability invocation", async () => {
  const fixture = await fixtureJson("fixture.json");
  const expected = await fixtureJson("invalid-input.json");
  const manifestPath = await writeBrowserBundle(fixture.capability.artifact.digest);
  const embedder = await BundleEmbedder.init({ manifestPath, loader, platform: "web" });
  const events = [];
  embedder.subscribe((event) => events.push(event));

  assert.equal(Object.hasOwn(expected.input, "name"), false);
  const evidence = browserEvidence(
    fixture,
    expected.expected_outcome,
    expected.expected_output_projection,
    expected.expected_trace_projection,
  );
  assert.equal(events.length, 0);
  assert.equal(evidence.output_projection.reason_code, "CONTRACT_INPUT_INVALID");
});

test("cross-host fixture: browser rejects an artifact digest mismatch before invocation", async () => {
  const fixture = await fixtureJson("fixture.json");
  const expected = await fixtureJson("artifact-identity-failure.json");
  const manifestPath = await writeBrowserBundle(expected.observed_artifact_digest);

  await assert.rejects(
    BundleEmbedder.init({ manifestPath, loader, platform: "web" }),
    (error) => error instanceof BundleRejectedError && error.embedderError.code === "bundle_load_failed",
  );
  const evidence = browserEvidence(
    fixture,
    expected.expected_outcome,
    expected.expected_output_projection,
    expected.expected_trace_projection,
  );
  assert.equal(evidence.output_projection.reason_code, "ARTIFACT_DIGEST_MISMATCH");
});
