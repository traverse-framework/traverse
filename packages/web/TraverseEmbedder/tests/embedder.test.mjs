// Conformance-shaped package tests for the Web/TypeScript embedder SDK
// (spec 057 conformance corpus shapes; spec 068 FR-006, NFR-001).
import test from "node:test";
import assert from "node:assert/strict";
import {
  EMBEDDER_API_VERSION,
  EMBEDDER_CONFORMANCE_VERSION,
  BundleRejectedError,
  EmbedderTestDouble,
  validateBundleCompatibility,
  verifyArtifactDigest,
} from "../dist/index.js";

const VALID_MANIFEST = {
  app_id: "fixture-app",
  version: "1.0.0",
  schema_version: "1.0.0",
  components: [
    {
      component_id: "fixture.process-component",
      version: "1.0.0",
      digest: `sha256:${"ab".repeat(32)}`,
      manifest_path: "components/process/component.manifest.json",
    },
  ],
  workflows: [
    { workflow_id: "fixture.pipeline", workflow_version: "1.0.0", path: "w.json" },
  ],
};

function collectEvents(embedder) {
  const events = [];
  embedder.subscribe((event) => events.push(event));
  return events;
}

test("init-shutdown: shutdown is idempotent and stops later submits", () => {
  const embedder = new EmbedderTestDouble().withTargetOutput("t", { ok: true });
  assert.equal(embedder.shutdown().killedInstances, 0);
  assert.equal(embedder.shutdown().killedInstances, 0);

  const rejected = embedder.submit("t", {});
  assert.equal(rejected.status, "rejected");
  assert.equal(rejected.sessionId, null);
  assert.equal(rejected.error.code, "runtime_stopped");
});

test("capability submit emits capability_invoked then capability_result", () => {
  const embedder = new EmbedderTestDouble({ appId: "fixture-app" }).withTargetOutput(
    "fixture.process",
    { status: "processed" },
  );
  const events = collectEvents(embedder);

  const outcome = embedder.submit("fixture.process", { note: "hello" });
  assert.equal(outcome.status, "accepted");
  assert.equal(outcome.sessionId, "sess-00000001");
  assert.equal(outcome.error, null);

  assert.equal(events.length, 2);
  assert.equal(events[0].event_type, "capability_invoked");
  assert.equal(events[0].data.execution_id, "exec_req-00000001");
  assert.equal(events[0].kind, "embedder_event");
  assert.equal(events[0].embedder_api_version, EMBEDDER_API_VERSION);
  assert.equal(events[1].event_type, "capability_result");
  assert.deepEqual(events[1].data.output, { status: "processed" });
});

test("scripted runtime-shaped errors surface as error events", () => {
  const embedder = new EmbedderTestDouble().withTargetError(
    "fixture.process",
    "execution_failed",
    "scripted failure",
  );
  const events = collectEvents(embedder);

  const outcome = embedder.submit("fixture.process", {});
  assert.equal(outcome.status, "accepted");
  assert.equal(events[1].event_type, "error");
  assert.equal(events[1].data.error.code, "execution_failed");
  assert.equal(events[1].data.error.message, "scripted failure");
});

test("unknown targets are rejected with an error event", () => {
  const embedder = new EmbedderTestDouble();
  const events = collectEvents(embedder);

  const outcome = embedder.submit("fixture.unknown", {});
  assert.equal(outcome.status, "rejected");
  assert.equal(outcome.error.code, "target_not_found");
  assert.equal(events.length, 1);
  assert.equal(events[0].event_type, "error");
  assert.equal(events[0].data.target_id, "fixture.unknown");
  assert.equal(events[0].data.error.code, "target_not_found");
});

test("compatible lifecycle: start, stop, kill, and kill on shutdown", () => {
  const embedder = new EmbedderTestDouble({ platform: "web" }).withCompatibleTarget(
    "fixture.render",
    ["web"],
  );
  const events = collectEvents(embedder);

  const started = embedder.startCompatible("fixture.render", { surface: "dom" });
  assert.equal(started.status, "started");
  const stopped = embedder.stopCompatible("fixture.render", started.instanceId);
  assert.equal(stopped.status, "stopped");
  assert.equal(stopped.error, null);

  const restarted = embedder.startCompatible("fixture.render", {});
  assert.equal(restarted.status, "started");
  const killed = embedder.killCompatible("fixture.render");
  assert.equal(killed.status, "killed");

  const finalInstance = embedder.startCompatible("fixture.render", {});
  assert.equal(finalInstance.status, "started");
  assert.equal(embedder.shutdown().killedInstances, 1);

  const states = events
    .filter((event) => event.event_type === "state_changed")
    .map((event) => event.data.state);
  assert.deepEqual(states, [
    "started",
    "stopped",
    "started",
    "killed",
    "started",
    "killed",
  ]);
});

test("compatible lifecycle edge cases use stable error codes", () => {
  const embedder = new EmbedderTestDouble({ platform: "web" }).withCompatibleTarget(
    "fixture.render",
    ["web"],
  );

  assert.equal(
    embedder.startCompatible("fixture.unknown", {}).error.code,
    "capability_not_compatible",
  );
  assert.equal(
    embedder.stopCompatible("fixture.render", "inst-99999999").error.code,
    "instance_not_found",
  );
  assert.equal(
    embedder.killCompatible("fixture.render").error.code,
    "instance_not_running",
  );

  const started = embedder.startCompatible("fixture.render", {});
  embedder.stopCompatible("fixture.render", started.instanceId);
  assert.equal(
    embedder.stopCompatible("fixture.render", started.instanceId).error.code,
    "instance_not_running",
  );

  embedder.shutdown();
  assert.equal(
    embedder.startCompatible("fixture.render", {}).error.code,
    "runtime_stopped",
  );
  assert.equal(
    embedder.stopCompatible("fixture.render").error.code,
    "runtime_stopped",
  );
  assert.equal(
    embedder.killCompatible("fixture.render").error.code,
    "runtime_stopped",
  );
});

test("platform-guard: wrong platform rejects with a deterministic error event", () => {
  const embedder = new EmbedderTestDouble({ platform: "ios" }).withCompatibleTarget(
    "fixture.render",
    ["web"],
  );
  const events = collectEvents(embedder);

  const outcome = embedder.startCompatible("fixture.render", {});
  assert.equal(outcome.status, "error");
  assert.equal(outcome.instanceId, null);
  assert.equal(outcome.error.code, "platform_not_supported");
  assert.equal(events.length, 1);
  assert.equal(events[0].event_type, "error");
  assert.equal(events[0].data.error.code, "platform_not_supported");
});

test("determinism: identical operations produce identical event JSON", () => {
  const run = () => {
    const embedder = new EmbedderTestDouble({ appId: "fixture-app" })
      .withTargetOutput("fixture.process", { status: "processed" })
      .withCompatibleTarget("fixture.render", ["web"]);
    const events = collectEvents(embedder);
    embedder.submit("fixture.process", { note: "same input" });
    embedder.startCompatible("fixture.render", {});
    embedder.shutdown();
    return events;
  };

  assert.equal(JSON.stringify(run()), JSON.stringify(run()));
});

test("late subscribers replay the identical ordered stream", () => {
  const embedder = new EmbedderTestDouble().withTargetOutput("t", { ok: true });
  const early = collectEvents(embedder);
  embedder.submit("t", {});
  const late = collectEvents(embedder);
  assert.deepEqual(early, late);
});

test("release evidence records package, versions, and bundle identity", () => {
  const embedder = new EmbedderTestDouble({ appId: "fixture-app" });
  const evidence = embedder.releaseEvidence();
  assert.equal(evidence.kind, "embedder_release_evidence");
  assert.equal(evidence.package.name, "traverse-embedder-web");
  assert.equal(evidence.embedder_api_version, EMBEDDER_API_VERSION);
  assert.equal(evidence.conformance_version, EMBEDDER_CONFORMANCE_VERSION);
  assert.equal(evidence.runtime.implementation, "test-double");
  assert.equal(evidence.bundle.app_id, "fixture-app");
});

test("bundle compatibility accepts the supported manifest shape", () => {
  const summary = validateBundleCompatibility(VALID_MANIFEST);
  assert.equal(summary.appId, "fixture-app");
  assert.equal(summary.schemaVersion, "1.0.0");
  assert.equal(summary.components.length, 1);
  assert.deepEqual(summary.workflowIds, ["fixture.pipeline"]);

  const fromString = validateBundleCompatibility(JSON.stringify(VALID_MANIFEST));
  assert.deepEqual(fromString, summary);
});

test("bundle compatibility rejects unsupported schema versions deterministically", () => {
  const manifest = { ...VALID_MANIFEST, schema_version: "9.9.9" };
  assert.throws(
    () => validateBundleCompatibility(manifest),
    (error) => {
      assert.ok(error instanceof BundleRejectedError);
      assert.equal(error.embedderError.code, "unsupported_bundle_schema");
      assert.match(error.embedderError.message, /9\.9\.9/);
      assert.match(error.embedderError.message, /no sidecar fallback/);
      return true;
    },
  );
});

test("bundle compatibility rejects malformed manifests with stable codes", () => {
  const expectRejection = (manifest, pattern) => {
    assert.throws(
      () => validateBundleCompatibility(manifest),
      (error) => {
        assert.ok(error instanceof BundleRejectedError);
        assert.equal(error.embedderError.code, "bundle_load_failed");
        assert.match(error.embedderError.message, pattern);
        return true;
      },
    );
  };

  expectRejection("{not json", /not valid JSON/);
  expectRejection([], /must be a JSON object/);
  expectRejection({ ...VALID_MANIFEST, app_id: "" }, /app_id/);
  expectRejection({ ...VALID_MANIFEST, components: "nope" }, /'components' array/);
  expectRejection({ ...VALID_MANIFEST, components: ["nope"] }, /must be a JSON object/);
  expectRejection(
    {
      ...VALID_MANIFEST,
      components: [{ ...VALID_MANIFEST.components[0], digest: "sha1:abc" }],
    },
    /invalid digest metadata/,
  );
  expectRejection({ ...VALID_MANIFEST, workflows: ["nope"] }, /must be a JSON object/);
});

test("artifact digest verification accepts matching bytes and rejects mismatches", async () => {
  const bytes = new TextEncoder().encode("deterministic artifact bytes");
  const digest = await crypto.subtle.digest("SHA-256", bytes);
  const hex = [...new Uint8Array(digest)]
    .map((byte) => byte.toString(16).padStart(2, "0"))
    .join("");

  await verifyArtifactDigest(bytes, `sha256:${hex}`, "fixture artifact");

  await assert.rejects(
    verifyArtifactDigest(bytes, `sha256:${"0".repeat(64)}`, "fixture artifact"),
    (error) => {
      assert.ok(error instanceof BundleRejectedError);
      assert.equal(error.embedderError.code, "bundle_load_failed");
      assert.match(error.embedderError.message, /digest mismatch/);
      return true;
    },
  );

  await assert.rejects(
    verifyArtifactDigest(bytes, "not-a-digest", "fixture artifact"),
    (error) => {
      assert.ok(error instanceof BundleRejectedError);
      assert.match(error.embedderError.message, /invalid digest metadata/);
      return true;
    },
  );
});
