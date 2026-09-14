import test from "node:test";
import assert from "node:assert/strict";
import {
  AUDIO_CAPTURE_OPERATION,
  AUDIO_INPUT_CONNECTOR,
  HOST_CONNECTOR_COMMAND_KIND,
  HOST_CONNECTOR_COMMAND_SCHEMA_VERSION,
  HOST_CONNECTOR_EVENT_KIND,
  HOST_CONNECTOR_GOVERNING_SPEC,
  audioCaptureCommand,
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
