import assert from "node:assert/strict";
import test from "node:test";

import {
  AUDIO_PERMISSION_REQUEST_OPERATION,
  ModelIoStore,
  createBrowserAudioInputAdapters,
} from "../dist/index.js";

test("AUDIO_PERMISSION_REQUEST_OPERATION is the Spec 137 wire id", () => {
  assert.equal(AUDIO_PERMISSION_REQUEST_OPERATION, "audio.permission.request");
});

test("permission granted returns non-secret permission_state", async () => {
  const io = new ModelIoStore();
  const adapters = createBrowserAudioInputAdapters({
    stageArtifact: (bytes, max) => io.stageArtifact(bytes, max),
    permissions: {
      status: async () => "granted",
      request: async () => "granted",
    },
  });
  const result = await adapters.requestPermission({
    command: "request_permission",
    commandId: "cmd-1",
    sessionId: "sess-1",
    payload: {},
  });
  assert.equal(result.resultClass, "succeeded");
  assert.deepEqual(result.payload, { permission_state: "granted" });
});

test("permission denied and unavailable are typed failures", async () => {
  const io = new ModelIoStore();
  const denied = createBrowserAudioInputAdapters({
    stageArtifact: (bytes, max) => io.stageArtifact(bytes, max),
    permissions: {
      status: async () => "denied",
      request: async () => "denied",
    },
  });
  const deniedResult = await denied.requestPermission({
    command: "request_permission",
    commandId: "cmd-d",
    sessionId: "sess-1",
    payload: {},
  });
  assert.equal(deniedResult.resultClass, "failed");
  assert.equal(deniedResult.payload?.error_code, "policy_denied");

  const unavailable = createBrowserAudioInputAdapters({
    stageArtifact: (bytes, max) => io.stageArtifact(bytes, max),
    permissions: {
      status: async () => "unavailable",
      request: async () => "unavailable",
    },
  });
  const unavailableResult = await unavailable.requestPermission({
    command: "request_permission",
    commandId: "cmd-u",
    sessionId: "sess-1",
    payload: {},
  });
  assert.equal(unavailableResult.resultClass, "failed");
  assert.equal(unavailableResult.payload?.error_code, "unavailable");
});

test("capture stages opaque artifact_ref and respects max_bytes", async () => {
  const io = new ModelIoStore();
  const persisted = new Map();
  const adapters = createBrowserAudioInputAdapters({
    stageArtifact: (bytes, max) => io.stageArtifact(bytes, max),
    persistBytes: (ref, bytes) => {
      persisted.set(ref, bytes);
    },
    permissions: {
      status: async () => "granted",
      request: async () => "granted",
    },
    capture: {
      async capture({ maxBytes }) {
        return new Uint8Array(Math.min(16, maxBytes)).fill(7);
      },
    },
  });
  const result = await adapters.captureAudio({
    command: "capture_audio",
    commandId: "cmd-c",
    sessionId: "sess-1",
    payload: { max_duration_ms: 1000, max_bytes: 1024 },
  });
  assert.equal(result.resultClass, "succeeded");
  const artifactRef = result.payload?.artifact_ref;
  assert.equal(typeof artifactRef, "string");
  assert.match(String(artifactRef), /^artifact-\d+$/);
  assert.equal(String(artifactRef).includes("/"), false);
  assert.equal(String(artifactRef).includes(":"), false);
  const bytes = io.readArtifact(String(artifactRef), 1024);
  assert.equal(bytes.length, 16);
  assert.equal(persisted.get(artifactRef)?.length, 16);
});

test("capture cancel aborts in-flight recording", async () => {
  const io = new ModelIoStore();
  const adapters = createBrowserAudioInputAdapters({
    stageArtifact: (bytes, max) => io.stageArtifact(bytes, max),
    capture: {
      async capture({ signal }) {
        await new Promise((resolve, reject) => {
          const timer = setTimeout(resolve, 5_000);
          signal.addEventListener(
            "abort",
            () => {
              clearTimeout(timer);
              reject(Object.assign(new Error("cancelled"), { code: "cancelled" }));
            },
            { once: true },
          );
        });
        return new Uint8Array([1]);
      },
    },
  });
  const pending = adapters.captureAudio({
    command: "capture_audio",
    commandId: "cmd-cancel",
    sessionId: "sess-1",
    payload: {
      max_duration_ms: 5_000,
      max_bytes: 1024,
      correlation_id: "corr-cancel",
    },
  });
  adapters.cancel("corr-cancel");
  const result = await pending;
  assert.equal(result.resultClass, "cancelled");
  assert.equal(result.payload?.error_code, "cancelled");
});

test("oversized capture fails closed before staging a path-like ref", async () => {
  const io = new ModelIoStore();
  const adapters = createBrowserAudioInputAdapters({
    stageArtifact: (bytes, max) => io.stageArtifact(bytes, max),
    capture: {
      async capture() {
        return new Uint8Array(64);
      },
    },
  });
  const result = await adapters.captureAudio({
    command: "capture_audio",
    commandId: "cmd-limit",
    sessionId: "sess-1",
    payload: { max_duration_ms: 1000, max_bytes: 8 },
  });
  assert.equal(result.resultClass, "failed");
  assert.equal(result.payload?.error_code, "input_limit_exceeded");
  const encoded = JSON.stringify(result);
  assert.equal(encoded.includes("/tmp"), false);
  assert.equal(encoded.includes("microphone"), false);
});
