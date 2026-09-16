import test from "node:test";
import assert from "node:assert/strict";
import {
  AUDIO_CAPTURE_OPERATION,
  AUDIO_INPUT_CONNECTOR,
  HOST_CONNECTOR_COMMAND_KIND,
  HOST_CONNECTOR_COMMAND_SCHEMA_VERSION,
  HOST_CONNECTOR_EVENT_KIND,
  HOST_CONNECTOR_GOVERNING_SPEC,
  MODEL_EXECUTE_OPERATION,
  MODEL_RUNTIME_CONNECTOR,
  MODEL_RUNTIME_GOVERNING_SPEC,
  PLACEMENT_WASM_CPU,
  audioCaptureCommand,
  modelExecuteCommand,
  normalizeModelExecuteEvidence,
} from "../dist/index.js";

test("browser and macOS share the audio.capture command contract", () => {
  const macos = audioCaptureCommand("cmd-1", "corr-1", "idem-1", "macos", {
    max_duration_ms: 5000,
    max_bytes: 1048576,
  });
  const browser = audioCaptureCommand("cmd-1", "corr-1", "idem-1", "browser", {
    max_duration_ms: 5000,
    max_bytes: 1048576,
  });
  assert.equal(macos.kind, HOST_CONNECTOR_COMMAND_KIND);
  assert.equal(macos.schema_version, HOST_CONNECTOR_COMMAND_SCHEMA_VERSION);
  assert.equal(macos.command, browser.command);
  assert.deepEqual(Object.keys(macos).sort(), Object.keys(browser).sort());
  assert.equal(AUDIO_INPUT_CONNECTOR, "traverse.audio-input");
  assert.equal(AUDIO_CAPTURE_OPERATION, "audio.capture");
  assert.equal(HOST_CONNECTOR_EVENT_KIND, "host_connector_event");
  assert.equal(HOST_CONNECTOR_GOVERNING_SPEC, "137-host-connector-command-dispatch");
  const encoded = JSON.stringify(macos);
  assert.equal(encoded.includes("microphone"), false);
  assert.equal(encoded.includes("schema_version"), true);
});

test("browser and macOS share Spec 138 model.execute payload shape and normalized evidence", () => {
  const payload = {
    model_ref: {
      model_id: "fixture.echo",
      version: "1.0.0",
      digest: "sha256:abc",
    },
    input_ref: "input-1",
    policy_ref: "policy-1",
    data_classification: "sensitive",
    input_schema_ref: "schema:fixture-in",
    input_schema_version: "1.0.0",
    max_output_bytes: 4096,
  };
  const macos = modelExecuteCommand("cmd-2", "corr-2", "idem-2", "macos", payload);
  const browser = modelExecuteCommand("cmd-2", "corr-2", "idem-2", "browser", payload);
  assert.equal(macos.command, browser.command);
  assert.deepEqual(macos.payload, browser.payload);
  assert.equal(MODEL_RUNTIME_CONNECTOR, "traverse.model-runtime");
  assert.equal(MODEL_EXECUTE_OPERATION, "model.execute");
  assert.equal(MODEL_RUNTIME_GOVERNING_SPEC, "138-governed-exact-model-execution");
  const nativeEvidence = normalizeModelExecuteEvidence({
    status: "ok",
    model_ref: payload.model_ref,
    placement: PLACEMENT_WASM_CPU,
    output_ref: "output-1",
  });
  const browserEvidence = normalizeModelExecuteEvidence({
    status: "ok",
    model_ref: payload.model_ref,
    placement: PLACEMENT_WASM_CPU,
    artifact_ref: "output-1",
  });
  assert.deepEqual(nativeEvidence, browserEvidence);
  assert.equal(nativeEvidence.has_output_ref, true);
  assert.equal(JSON.stringify(macos).includes("provider"), false);
});
