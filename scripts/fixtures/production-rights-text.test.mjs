// node --test scripts/fixtures/production-rights-text.test.mjs (Decision 135, #1891)
import test from "node:test";
import assert from "node:assert/strict";
import { assertNoSigningClaims, signingClaimsIn } from "./production-rights-text.mjs";

const clean = {
  license_id: "CC-BY-4.0",
  attribution: "Weights trained by Traverse on the UCI optdigits dataset",
  redistribution: "Redistribution permitted under CC BY 4.0 with the attribution above",
  commercial_use: "allowed",
  source_url: "https://archive.ics.uci.edu/dataset/80/optical+recognition+of+handwritten+digits",
};

test("clean production rights text passes", () => {
  assert.deepEqual(signingClaimsIn(clean), []);
  assert.doesNotThrow(() => assertNoSigningClaims({ dir: "p-1.0.1", production_signed: true }, clean));
});

test("signing or key wording in a production package is refused", () => {
  for (const text of [
    "Redistribution permitted; signed with the test-only key (no production trust)",
    "signed with the Traverse production model-signing key",
    "Signature: ed25519",
    "verified against key_id ed25519:abc",
    "uses the production keys",
  ]) {
    assert.throws(
      () => assertNoSigningClaims({ dir: "p-1.0.1", production_signed: true }, { ...clean, redistribution: text }),
      /rights\.redistribution/,
      text,
    );
  }
  assert.deepEqual(signingClaimsIn({ ...clean, attribution: "signed by x", redistribution: "key" }), ["attribution", "redistribution"]);
});

test("words that only contain the letters are not claims", () => {
  for (const text of ["Designed for keyboards; assigned to monkeys", "design", "turnkey"]) {
    assert.deepEqual(signingClaimsIn({ redistribution: text }), [], text);
  }
});

test("test-only fixtures and superseded production packages are exempt", () => {
  const stale = { ...clean, redistribution: "signed with the test-only key" };
  assert.doesNotThrow(() => assertNoSigningClaims({ dir: "fixture-echo-1.0.0" }, stale));
  assert.doesNotThrow(() =>
    assertNoSigningClaims({ dir: "digits-mlp-1.0.0", production_signed: true, superseded_by: "digits-mlp-1.0.1" }, stale),
  );
});
