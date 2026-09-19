import assert from "node:assert/strict";
import test from "node:test";
import {
  IDBFactory,
  IDBObjectStore,
} from "fake-indexeddb";
import {
  IndexedDbDataStore,
  IndexedDbDataStoreError,
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

function config(databaseName, overrides = {}) {
  return {
    databaseName,
    classification: "public",
    indexedDB: overrides.indexedDB ?? new IDBFactory(),
    locks: overrides.locks ?? new ConformanceLockManager(),
  };
}

function record(value = { status: "ready" }) {
  return {
    key: "draft",
    value,
    lamport_clock: 1,
    writer_id: "writer-a",
  };
}

function isCode(code, reason) {
  return (error) =>
    error instanceof IndexedDbDataStoreError &&
    error.code === code &&
    (reason === undefined || error.reason === reason);
}

async function mutateRecord(factory, databaseName, mutate) {
  const database = await new Promise((resolve, reject) => {
    const request = factory.open(databaseName, 1);
    request.onerror = () => reject(request.error);
    request.onsuccess = () => resolve(request.result);
  });
  await new Promise((resolve, reject) => {
    const transaction = database.transaction("records", "readwrite");
    const store = transaction.objectStore("records");
    const get = store.get("draft");
    get.onerror = () => reject(get.error);
    get.onsuccess = () => store.put(mutate(get.result), "draft");
    transaction.onerror = () => reject(transaction.error);
    transaction.oncomplete = resolve;
  });
  database.close();
}

async function storedKeys(factory, databaseName) {
  const database = await new Promise((resolve, reject) => {
    const request = factory.open(databaseName, 1);
    request.onerror = () => reject(request.error);
    request.onsuccess = () => resolve(request.result);
  });
  const keys = await new Promise((resolve, reject) => {
    const transaction = database.transaction("records", "readonly");
    const request = transaction.objectStore("records").getAllKeys();
    request.onerror = () => reject(request.error);
    transaction.onerror = () => reject(transaction.error);
    request.onsuccess = () => resolve(request.result);
  });
  database.close();
  return keys;
}

const NON_STRING_KEYS = [
  ["number", 7],
  ["boolean", true],
  ["null", null],
  ["array", ["draft"]],
  ["object", { key: "draft" }],
];

test("browser conformance: public records persist across reopen", async () => {
  const factory = new IDBFactory();
  const locks = new ConformanceLockManager();
  const first = await IndexedDbDataStore.open(
    config("public-reopen", { indexedDB: factory, locks }),
  );
  await first.write(record());
  assert.deepEqual(await first.read("draft"), record());
  first.close();
  await new Promise((resolve) => setTimeout(resolve, 0));

  const reopened = await IndexedDbDataStore.open(
    config("public-reopen", { indexedDB: factory, locks }),
  );
  assert.deepEqual(await reopened.read("draft"), record());
  await reopened.delete("draft");
  assert.equal(await reopened.read("draft"), null);
  reopened.close();
});

test("browser conformance: a same-origin contender receives store_locked", async () => {
  const factory = new IDBFactory();
  const locks = new ConformanceLockManager();
  const owner = await IndexedDbDataStore.open(
    config("exclusive-owner", { indexedDB: factory, locks }),
  );
  await assert.rejects(
    () =>
      IndexedDbDataStore.open(
        config("exclusive-owner", { indexedDB: factory, locks }),
      ),
    isCode("store_locked", "exclusive_owner_active"),
  );
  owner.close();
});

test("browser conformance: missing Web Locks fails closed", async () => {
  await assert.rejects(
    () =>
      IndexedDbDataStore.open({
        databaseName: "no-locks",
        classification: "public",
        indexedDB: new IDBFactory(),
        locks: null,
      }),
    isCode("locking_unsupported", "web_locks_unavailable"),
  );
});

test("browser conformance: unavailable persistence has a stable typed error", async () => {
  await assert.rejects(
    () =>
      IndexedDbDataStore.open({
        databaseName: "no-indexeddb",
        classification: "public",
        indexedDB: null,
        locks: new ConformanceLockManager(),
      }),
    isCode("persistence_unavailable", "indexeddb_unavailable"),
  );
});

test("browser conformance: tampered public data fails integrity verification", async () => {
  const factory = new IDBFactory();
  const locks = new ConformanceLockManager();
  const store = await IndexedDbDataStore.open(
    config("integrity", { indexedDB: factory, locks }),
  );
  await store.write(record());
  store.close();
  await new Promise((resolve) => setTimeout(resolve, 0));
  await mutateRecord(factory, "integrity", (envelope) => ({
    ...envelope,
    record: record({ status: "tampered" }),
  }));

  const reopened = await IndexedDbDataStore.open(
    config("integrity", { indexedDB: factory, locks }),
  );
  await assert.rejects(
    () => reopened.read("draft"),
    isCode("integrity_check_failed", "digest_mismatch"),
  );
  reopened.close();
});

test("browser conformance: private operations fail without storing plaintext", async () => {
  const factory = new IDBFactory();
  const store = await IndexedDbDataStore.open({
    ...config("private-fail-closed", { indexedDB: factory }),
    classification: "private",
  });
  await assert.rejects(() => store.write(record("secret")), isCode("key_provider_required"));
  await assert.rejects(() => store.read("draft"), isCode("key_provider_required"));
  await assert.rejects(() => store.delete("draft"), isCode("key_provider_required"));
  store.close();
  await new Promise((resolve) => setTimeout(resolve, 0));

  const publicView = await IndexedDbDataStore.open(
    config("private-fail-closed", { indexedDB: factory }),
  );
  assert.equal(await publicView.read("draft"), null);
  publicView.close();
});

test("browser conformance: quota failures are typed and never silently succeed", async () => {
  const store = await IndexedDbDataStore.open(config("quota"));
  const originalPut = IDBObjectStore.prototype.put;
  IDBObjectStore.prototype.put = function quotaFailure() {
    throw new DOMException("test-only quota detail", "QuotaExceededError");
  };
  try {
    await assert.rejects(() => store.write(record()), isCode("quota_exceeded"));
  } finally {
    IDBObjectStore.prototype.put = originalPut;
    store.close();
  }
});

test("browser conformance: non-string keys are rejected without storing state", async () => {
  const factory = new IDBFactory();
  const store = await IndexedDbDataStore.open(
    config("non-string-keys", { indexedDB: factory }),
  );
  try {
    for (const [label, key] of NON_STRING_KEYS) {
      await assert.rejects(
        () => store.write({ ...record(), key }),
        isCode("invalid_key", "invalid_state_key"),
        `write should reject a ${label} key`,
      );
      await assert.rejects(
        () => store.read(key),
        isCode("invalid_key", "invalid_state_key"),
        `read should reject a ${label} key`,
      );
      await assert.rejects(
        () => store.delete(key),
        isCode("invalid_key", "invalid_state_key"),
        `delete should reject a ${label} key`,
      );
    }
    assert.deepEqual(await storedKeys(factory, "non-string-keys"), []);
  } finally {
    store.close();
  }
});

test("browser conformance: key validation never coerces host objects", async () => {
  const store = await IndexedDbDataStore.open(config("no-key-coercion"));
  let coercions = 0;
  const coercible = {
    toString() {
      coercions += 1;
      return "draft";
    },
    valueOf() {
      coercions += 1;
      return "draft";
    },
  };
  try {
    await assert.rejects(
      () => store.read(coercible),
      isCode("invalid_key", "invalid_state_key"),
    );
    await assert.rejects(
      () => store.write({ ...record(), key: coercible }),
      isCode("invalid_key", "invalid_state_key"),
    );
    await assert.rejects(
      () => store.delete(coercible),
      isCode("invalid_key", "invalid_state_key"),
    );
    assert.equal(coercions, 0);
  } finally {
    store.close();
  }
});

test("browser conformance: numeric-looking string keys still round-trip", async () => {
  const store = await IndexedDbDataStore.open(config("numeric-string-key"));
  try {
    const numericKey = { ...record(), key: "7" };
    await store.write(numericKey);
    assert.deepEqual(await store.read("7"), numericKey);
    await store.delete("7");
    assert.equal(await store.read("7"), null);
  } finally {
    store.close();
  }
});

test("browser conformance: invalid deletes leave existing records intact", async () => {
  const store = await IndexedDbDataStore.open(config("invalid-delete"));
  try {
    await store.write(record());
    for (const [, key] of NON_STRING_KEYS) {
      await assert.rejects(() => store.delete(key), isCode("invalid_key"));
    }
    assert.deepEqual(await store.read("draft"), record());
  } finally {
    store.close();
  }
});

test("browser conformance: maintenance is explicitly unsupported", async () => {
  const store = await IndexedDbDataStore.open(config("maintenance"));
  await assert.rejects(() => store.prune(), isCode("unsupported"));
  await assert.rejects(() => store.backup(), isCode("unsupported"));
  await assert.rejects(() => store.restore(), isCode("unsupported"));
  store.close();
});
