import assert from "node:assert/strict";
import test from "node:test";
import { IDBFactory } from "fake-indexeddb";
import {
  IndexedDbDataStore,
  STATEFUL_BROWSER_STORE_UNAVAILABLE,
  attestStatefulBrowserActivation,
} from "../dist/index.js";

class ConformanceLockManager {
  #held = new Set();

  async request(name, _options, callback) {
    if (this.#held.has(name)) {
      return callback(null);
    }
    this.#held.add(name);
    try {
      return await callback({ name, mode: "exclusive" });
    } finally {
      this.#held.delete(name);
    }
  }
}

function config(databaseName) {
  return {
    databaseName,
    classification: "public",
    indexedDB: new IDBFactory(),
    locks: new ConformanceLockManager(),
  };
}

test("attestStatefulBrowserActivation fails closed without a bound store", () => {
  const missing = attestStatefulBrowserActivation(null);
  assert.equal(missing.ok, false);
  assert.equal(missing.error.code, STATEFUL_BROWSER_STORE_UNAVAILABLE);
  assert.equal(missing.error.reason, "missing_bound_store");
  const text = JSON.stringify(missing.error);
  assert.equal(text.includes("payload"), false);
  assert.equal(text.includes("databaseName"), false);
});

test("attestStatefulBrowserActivation fails closed for non-IndexedDB values", () => {
  const denial = attestStatefulBrowserActivation({ pretend: true });
  assert.equal(denial.ok, false);
  assert.equal(denial.error.code, STATEFUL_BROWSER_STORE_UNAVAILABLE);
  assert.equal(denial.error.reason, "non_indexeddb_backend");
});

test("attestStatefulBrowserActivation accepts an open IndexedDB store", async () => {
  const store = await IndexedDbDataStore.open(config("stateful-browser-ok"));
  const result = attestStatefulBrowserActivation(store);
  assert.equal(result.ok, true);
  assert.equal(result.evidence.backend, "indexeddb");
  assert.equal(result.evidence.outcome, "attested");
  const text = JSON.stringify(result.evidence);
  assert.equal(text.includes(store.databaseName), false);
  store.close();
});

test("attestStatefulBrowserActivation fails closed after close", async () => {
  const store = await IndexedDbDataStore.open(config("stateful-browser-closed"));
  store.close();
  const denial = attestStatefulBrowserActivation(store);
  assert.equal(denial.ok, false);
  assert.equal(denial.error.code, STATEFUL_BROWSER_STORE_UNAVAILABLE);
  assert.ok(
    denial.error.reason === "store_closed" ||
      denial.error.reason === "exclusive_lock_not_held",
  );
});

test("IndexedDbDataStore.openAttestation is secret-free", async () => {
  const store = await IndexedDbDataStore.open(config("stateful-browser-facts"));
  const facts = store.openAttestation();
  assert.equal(facts.backend, "indexeddb");
  assert.equal(facts.exclusive_lock_held, true);
  assert.equal(facts.public_integrity_available, true);
  assert.equal(JSON.stringify(facts).includes(store.databaseName), false);
  store.close();
});
