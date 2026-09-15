import assert from "node:assert/strict";
import test from "node:test";
import {
  EMIT_EVENT_ERR_INVALID_PAYLOAD,
  EMIT_EVENT_ERR_NOT_SUBSCRIBABLE,
  EMIT_EVENT_ERR_UNDECLARED_EVENT,
  EMIT_EVENT_OK,
  MAX_EVENT_EMIT_PAYLOAD_BYTES,
  createEmitEventHostImport,
  parseDeclaredEmits,
} from "../dist/emitEventHost.js";

test("parseDeclaredEmits keeps only well-formed event_id/version pairs", () => {
  assert.deepEqual(
    parseDeclaredEmits([
      { event_id: "a", version: "1.0.0" },
      { event_id: "b" },
      "skip",
      { event_id: "c", version: "2.0.0" },
    ]),
    [
      { event_id: "a", version: "1.0.0" },
      { event_id: "c", version: "2.0.0" },
    ],
  );
});

test("createEmitEventHostImport mirrors Rust status codes without throwing", () => {
  const memory = new WebAssembly.Memory({ initial: 1 });
  const memoryRef = { memory };
  const accepted = [];
  const emit = createEmitEventHostImport(
    {
      capabilityId: "fixture.emit",
      serviceType: "subscribable",
      emits: [{ event_id: "dev.traverse.test.emitted", version: "1.0.0" }],
      onAccepted: (event) => accepted.push(event),
    },
    memoryRef,
  );

  assert.equal(
    createEmitEventHostImport(
      {
        capabilityId: "fixture.emit",
        serviceType: "stateless",
        emits: [{ event_id: "dev.traverse.test.emitted", version: "1.0.0" }],
        onAccepted: () => {},
      },
      memoryRef,
    )(0, 0),
    EMIT_EVENT_ERR_NOT_SUBSCRIBABLE,
  );

  assert.equal(emit(-1, 4), EMIT_EVENT_ERR_INVALID_PAYLOAD);
  assert.equal(emit(0, MAX_EVENT_EMIT_PAYLOAD_BYTES + 1), EMIT_EVENT_ERR_INVALID_PAYLOAD);

  const encoder = new TextEncoder();
  const good = encoder.encode('{"event_id":"dev.traverse.test.emitted","version":"1.0.0","payload":{"n":1}}');
  new Uint8Array(memory.buffer).set(good, 32);
  assert.equal(emit(32, good.length), EMIT_EVENT_OK);
  assert.equal(accepted.length, 1);

  const undeclared = encoder.encode('{"event_id":"other","version":"1.0.0"}');
  new Uint8Array(memory.buffer).set(undeclared, 200);
  assert.equal(emit(200, undeclared.length), EMIT_EVENT_ERR_UNDECLARED_EVENT);

  const malformed = encoder.encode("not-json");
  new Uint8Array(memory.buffer).set(malformed, 400);
  assert.equal(emit(400, malformed.length), EMIT_EVENT_ERR_INVALID_PAYLOAD);
});
