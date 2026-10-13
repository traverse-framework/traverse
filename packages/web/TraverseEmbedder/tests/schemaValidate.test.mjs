import test from "node:test";
import assert from "node:assert/strict";
import { validatePayloadAgainstContract } from "../dist/schemaValidate.js";

const REDACT_SCHEMA = {
  type: "object",
  additionalProperties: false,
  required: ["text"],
  properties: {
    text: { type: "string", maxLength: 20000 },
  },
};

test("accepts a valid redact-entities shaped input", () => {
  assert.deepEqual(
    validatePayloadAgainstContract({ text: "hello" }, REDACT_SCHEMA),
    [],
  );
});

test("rejects unknown fields with JSON Pointer paths (#1658)", () => {
  const violations = validatePayloadAgainstContract(
    {
      text: "Maria Lopez from Northwind Bank",
      skip_redaction: true,
      return_raw: true,
    },
    REDACT_SCHEMA,
  );
  const paths = violations.map((v) => v.path).sort();
  assert.deepEqual(paths, ["/return_raw", "/skip_redaction"]);
  assert.equal(violations[0].keyword, "additionalProperties");
});

test("rejects missing required and maxLength", () => {
  assert.equal(
    validatePayloadAgainstContract({}, REDACT_SCHEMA)[0].path,
    "/text",
  );
  const long = validatePayloadAgainstContract(
    { text: "x".repeat(20001) },
    REDACT_SCHEMA,
  );
  assert.equal(long[0].path, "/text");
  assert.equal(long[0].keyword, "maxLength");
});
