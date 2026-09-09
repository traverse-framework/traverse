import type { JsonValue } from "./types.js";

const DATA_STORE_FORMAT = "local-datastore/1";
const DATABASE_VERSION = 1;
const RECORDS_OBJECT_STORE = "records";
const LOCK_NAME_PREFIX = "traverse-datastore:";

export type DataClassification = "public" | "private";

export interface StateRecord {
  readonly key: string;
  readonly value: JsonValue;
  readonly lamport_clock: number;
  readonly writer_id: string;
}

export type IndexedDbDataStoreErrorCode =
  | "invalid_key"
  | "integrity_check_failed"
  | "key_provider_required"
  | "store_locked"
  | "locking_unsupported"
  | "quota_exceeded"
  | "persistence_unavailable"
  | "backend_failed"
  | "unsupported";

export type IndexedDbDataStoreOperation =
  | "open"
  | "read"
  | "write"
  | "delete"
  | "prune"
  | "backup"
  | "restore";

/** A stable, secret-free browser DataStore failure (Spec 085 FR-005/FR-007). */
export class IndexedDbDataStoreError extends Error {
  readonly code: IndexedDbDataStoreErrorCode;
  readonly operation: IndexedDbDataStoreOperation;
  readonly reason: string;

  constructor(
    code: IndexedDbDataStoreErrorCode,
    operation: IndexedDbDataStoreOperation,
    reason: string,
  ) {
    super(code);
    this.name = "IndexedDbDataStoreError";
    this.code = code;
    this.operation = operation;
    this.reason = reason;
  }
}

export interface IndexedDbDataStoreConfig {
  /**
   * Host-selected, origin-scoped IndexedDB name. Traverse never derives or
   * supplies a global default name.
   */
  readonly databaseName: string;
  /** Fixed classification for this store. Private operations fail closed. */
  readonly classification: DataClassification;
  /** Browser globals may be injected by a conformance harness. */
  readonly indexedDB?: IDBFactory | null;
  /** Browser Web Locks may be injected by a conformance harness. */
  readonly locks?: WebLockManager | null;
}

interface WebLock {
  readonly name: string;
  readonly mode: "exclusive" | "shared";
}

interface WebLockManager {
  request<T>(
    name: string,
    options: { readonly mode: "exclusive"; readonly ifAvailable: true },
    callback: (lock: WebLock | null) => Promise<T>,
  ): Promise<T>;
}

interface PublicEnvelope {
  readonly format: typeof DATA_STORE_FORMAT;
  readonly classification: "public";
  readonly digest: string;
  readonly record: StateRecord;
}

function dataStoreError(
  code: IndexedDbDataStoreErrorCode,
  operation: IndexedDbDataStoreOperation,
  reason: string,
): IndexedDbDataStoreError {
  return new IndexedDbDataStoreError(code, operation, reason);
}

function defaultIndexedDb(): IDBFactory | undefined {
  return globalThis.indexedDB;
}

function defaultLocks(): WebLockManager | undefined {
  const browserNavigator = globalThis.navigator as
    | (Navigator & { readonly locks?: WebLockManager })
    | undefined;
  return browserNavigator?.locks;
}

function validateDatabaseName(databaseName: string): void {
  if (databaseName.trim().length === 0) {
    throw dataStoreError("persistence_unavailable", "open", "invalid_database_name");
  }
}

function validateKey(key: string, operation: "read" | "write" | "delete"): void {
  if (!/^[A-Za-z0-9_-]+$/.test(key)) {
    throw dataStoreError("invalid_key", operation, "invalid_state_key");
  }
}

function validateRecord(record: StateRecord): void {
  validateKey(record.key, "write");
  if (
    !Number.isSafeInteger(record.lamport_clock) ||
    record.lamport_clock < 0 ||
    record.writer_id.length === 0 ||
    !isJsonValue(record.value)
  ) {
    throw dataStoreError("backend_failed", "write", "invalid_state_record");
  }
}

function privateUnsupported(
  operation: "read" | "write" | "delete",
): IndexedDbDataStoreError {
  return dataStoreError("key_provider_required", operation, "private_record");
}

function mapDomFailure(
  operation: "open" | "read" | "write" | "delete",
  failure: unknown,
): IndexedDbDataStoreError {
  if (failure instanceof IndexedDbDataStoreError) {
    return failure;
  }
  const name =
    typeof failure === "object" &&
    failure !== null &&
    "name" in failure &&
    typeof failure.name === "string"
      ? failure.name
      : "";
  if (name === "QuotaExceededError") {
    return dataStoreError("quota_exceeded", operation, "storage_quota_exceeded");
  }
  if (
    name === "SecurityError" ||
    name === "InvalidStateError" ||
    name === "NotSupportedError" ||
    name === "UnknownError"
  ) {
    return dataStoreError(
      "persistence_unavailable",
      operation,
      "indexeddb_unavailable",
    );
  }
  return dataStoreError("backend_failed", operation, "indexeddb_operation_failed");
}

function sortedJsonValue(value: JsonValue): JsonValue {
  if (Array.isArray(value)) {
    return value.map(sortedJsonValue);
  }
  if (typeof value === "object" && value !== null) {
    const sorted: { [key: string]: JsonValue } = {};
    for (const key of Object.keys(value).sort()) {
      const child = value[key];
      if (child !== undefined) {
        sorted[key] = sortedJsonValue(child);
      }
    }
    return sorted;
  }
  return value;
}

function canonicalRecord(record: StateRecord): string {
  return JSON.stringify({
    key: record.key,
    value: sortedJsonValue(record.value),
    lamport_clock: record.lamport_clock,
    writer_id: record.writer_id,
  });
}

async function digestForRecord(
  record: StateRecord,
  operation: "read" | "write",
): Promise<string> {
  if (globalThis.crypto?.subtle === undefined) {
    throw dataStoreError("backend_failed", operation, "digest_unavailable");
  }
  let digest: ArrayBuffer;
  try {
    digest = await globalThis.crypto.subtle.digest(
      "SHA-256",
      new TextEncoder().encode(canonicalRecord(record)),
    );
  } catch {
    throw dataStoreError("backend_failed", operation, "digest_failed");
  }
  const hexadecimal = Array.from(new Uint8Array(digest), (byte) =>
    byte.toString(16).padStart(2, "0"),
  ).join("");
  return `sha256:${hexadecimal}`;
}

function isJsonValue(value: unknown): value is JsonValue {
  return isJsonValueInner(value, new WeakSet<object>());
}

function isJsonValueInner(
  value: unknown,
  ancestors: WeakSet<object>,
): value is JsonValue {
  if (
    value === null ||
    typeof value === "string" ||
    typeof value === "boolean" ||
    (typeof value === "number" && Number.isFinite(value))
  ) {
    return true;
  }
  if (Array.isArray(value)) {
    if (ancestors.has(value)) {
      return false;
    }
    ancestors.add(value);
    const valid = value.every((child) => isJsonValueInner(child, ancestors));
    ancestors.delete(value);
    return valid;
  }
  if (
    typeof value === "object" &&
    (Object.getPrototypeOf(value) === Object.prototype ||
      Object.getPrototypeOf(value) === null)
  ) {
    if (ancestors.has(value)) {
      return false;
    }
    ancestors.add(value);
    const valid = Object.values(value).every((child) =>
      isJsonValueInner(child, ancestors),
    );
    ancestors.delete(value);
    return valid;
  }
  return false;
}

function parseEnvelope(value: unknown, requestedKey: string): PublicEnvelope {
  if (typeof value !== "object" || value === null) {
    throw dataStoreError("integrity_check_failed", "read", "malformed_envelope");
  }
  const envelope = value as Partial<PublicEnvelope>;
  if (envelope.format !== DATA_STORE_FORMAT) {
    throw dataStoreError(
      "integrity_check_failed",
      "read",
      envelope.format === undefined ? "legacy_unverified" : "unknown_format_version",
    );
  }
  if (envelope.classification !== "public") {
    throw dataStoreError("key_provider_required", "read", "private_record");
  }
  if (
    "record_key" in envelope ||
    "key_id" in envelope ||
    "nonce" in envelope ||
    "ciphertext" in envelope
  ) {
    throw dataStoreError("integrity_check_failed", "read", "malformed_envelope");
  }
  const record = envelope.record as Partial<StateRecord> | undefined;
  if (
    record === undefined ||
    typeof record.key !== "string" ||
    record.key !== requestedKey ||
    typeof record.writer_id !== "string" ||
    record.writer_id.length === 0 ||
    typeof record.lamport_clock !== "number" ||
    !Number.isSafeInteger(record.lamport_clock) ||
    record.lamport_clock < 0 ||
    !isJsonValue(record.value) ||
    typeof envelope.digest !== "string"
  ) {
    throw dataStoreError("integrity_check_failed", "read", "malformed_envelope");
  }
  return envelope as PublicEnvelope;
}

function openDatabase(factory: IDBFactory, databaseName: string): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    let rejected = false;
    let request: IDBOpenDBRequest;
    try {
      request = factory.open(databaseName, DATABASE_VERSION);
    } catch (error) {
      reject(mapDomFailure("open", error));
      return;
    }
    request.onupgradeneeded = () => {
      if (!request.result.objectStoreNames.contains(RECORDS_OBJECT_STORE)) {
        request.result.createObjectStore(RECORDS_OBJECT_STORE);
      }
    };
    request.onerror = () => {
      rejected = true;
      reject(mapDomFailure("open", request.error));
    };
    request.onblocked = () => {
      rejected = true;
      reject(dataStoreError("persistence_unavailable", "open", "upgrade_blocked"));
    };
    request.onsuccess = () => {
      if (rejected) {
        request.result.close();
        return;
      }
      if (!request.result.objectStoreNames.contains(RECORDS_OBJECT_STORE)) {
        request.result.close();
        reject(dataStoreError("backend_failed", "open", "records_store_missing"));
        return;
      }
      resolve(request.result);
    };
  });
}

/**
 * Host-owned IndexedDB implementation of the DataStore public-record port.
 *
 * Opening holds one origin-scoped exclusive Web Lock until `close()`. The
 * adapter exposes no fallback lock and never persists private plaintext.
 */
export class IndexedDbDataStore {
  readonly databaseName: string;
  readonly classification: DataClassification;

  private readonly database: IDBDatabase;
  private releaseLock: (() => void) | undefined;
  private lockRequest: Promise<void> | undefined;
  private closed = false;

  private constructor(
    databaseName: string,
    classification: DataClassification,
    database: IDBDatabase,
    releaseLock: () => void,
    lockRequest: Promise<void>,
  ) {
    this.databaseName = databaseName;
    this.classification = classification;
    this.database = database;
    this.releaseLock = releaseLock;
    this.lockRequest = lockRequest;
  }

  static async open(config: IndexedDbDataStoreConfig): Promise<IndexedDbDataStore> {
    validateDatabaseName(config.databaseName);
    const factory =
      config.indexedDB === undefined ? defaultIndexedDb() : config.indexedDB;
    if (factory === undefined || factory === null) {
      throw dataStoreError(
        "persistence_unavailable",
        "open",
        "indexeddb_unavailable",
      );
    }
    const locks = config.locks === undefined ? defaultLocks() : config.locks;
    if (locks === undefined || locks === null) {
      throw dataStoreError("locking_unsupported", "open", "web_locks_unavailable");
    }

    let signalAcquired: (() => void) | undefined;
    let signalRejected: ((error: IndexedDbDataStoreError) => void) | undefined;
    const acquired = new Promise<void>((resolve, reject) => {
      signalAcquired = resolve;
      signalRejected = reject;
    });
    let releaseLock: () => void = () => undefined;
    const held = new Promise<void>((resolve) => {
      releaseLock = resolve;
    });
    let lockRequest: Promise<void>;
    try {
      lockRequest = locks
        .request(
          `${LOCK_NAME_PREFIX}${config.databaseName}`,
          { mode: "exclusive", ifAvailable: true },
          async (lock) => {
            if (lock === null) {
              signalRejected?.(
                dataStoreError("store_locked", "open", "exclusive_owner_active"),
              );
              return;
            }
            signalAcquired?.();
            await held;
          },
        )
        .catch(() => {
          signalRejected?.(
            dataStoreError("locking_unsupported", "open", "web_locks_failed"),
          );
        });
    } catch {
      throw dataStoreError("locking_unsupported", "open", "web_locks_failed");
    }

    await acquired;
    try {
      const database = await openDatabase(factory, config.databaseName);
      return new IndexedDbDataStore(
        config.databaseName,
        config.classification,
        database,
        releaseLock,
        lockRequest,
      );
    } catch (error) {
      releaseLock();
      await lockRequest;
      throw error;
    }
  }

  async read(key: string): Promise<StateRecord | null> {
    this.ensureOpen("read");
    validateKey(key, "read");
    if (this.classification === "private") {
      throw privateUnsupported("read");
    }
    const stored = await this.request("read", "readonly", (store) => store.get(key));
    if (stored === undefined) {
      return null;
    }
    const envelope = parseEnvelope(stored, key);
    const expectedDigest = await digestForRecord(envelope.record, "read");
    if (envelope.digest !== expectedDigest) {
      throw dataStoreError("integrity_check_failed", "read", "digest_mismatch");
    }
    return envelope.record;
  }

  async write(record: StateRecord): Promise<void> {
    this.ensureOpen("write");
    validateRecord(record);
    if (this.classification === "private") {
      throw privateUnsupported("write");
    }
    const envelope: PublicEnvelope = {
      format: DATA_STORE_FORMAT,
      classification: "public",
      digest: await digestForRecord(record, "write"),
      record,
    };
    await this.request("write", "readwrite", (store) => store.put(envelope, record.key));
  }

  async delete(key: string): Promise<void> {
    this.ensureOpen("delete");
    validateKey(key, "delete");
    if (this.classification === "private") {
      throw privateUnsupported("delete");
    }
    await this.request("delete", "readwrite", (store) => store.delete(key));
  }

  async prune(): Promise<never> {
    throw dataStoreError("unsupported", "prune", "maintenance_unsupported");
  }

  async backup(): Promise<never> {
    throw dataStoreError("unsupported", "backup", "maintenance_unsupported");
  }

  async restore(): Promise<never> {
    throw dataStoreError("unsupported", "restore", "maintenance_unsupported");
  }

  close(): void {
    if (this.closed) {
      return;
    }
    this.closed = true;
    this.database.close();
    this.releaseLock?.();
    this.releaseLock = undefined;
    this.lockRequest = undefined;
  }

  /**
   * Runtime-verifiable open facts for Spec `132` Stateful Browser activation.
   * Never includes the database name, payloads, or host-private paths.
   */
  openAttestation(): {
    readonly backend: "indexeddb";
    readonly closed: boolean;
    readonly exclusive_lock_held: boolean;
    readonly public_integrity_available: boolean;
  } {
    return {
      backend: "indexeddb",
      closed: this.closed,
      exclusive_lock_held: !this.closed && this.releaseLock !== undefined,
      // Spec 085 public integrity envelopes are available on every open handle;
      // private CRUD remains fail-closed until #1294.
      public_integrity_available: !this.closed,
    };
  }

  private ensureOpen(operation: "read" | "write" | "delete"): void {
    if (this.closed) {
      throw dataStoreError("backend_failed", operation, "store_closed");
    }
  }

  private request(
    operation: "read" | "write" | "delete",
    mode: IDBTransactionMode,
    createRequest: (store: IDBObjectStore) => IDBRequest,
  ): Promise<unknown> {
    return new Promise((resolve, reject) => {
      let transaction: IDBTransaction;
      let request: IDBRequest;
      try {
        transaction = this.database.transaction(RECORDS_OBJECT_STORE, mode);
        request = createRequest(transaction.objectStore(RECORDS_OBJECT_STORE));
      } catch (error) {
        reject(mapDomFailure(operation, error));
        return;
      }
      let result: unknown;
      request.onsuccess = () => {
        result = request.result;
      };
      request.onerror = () => {
        reject(mapDomFailure(operation, request.error));
      };
      transaction.oncomplete = () => resolve(result);
      transaction.onerror = () => {
        reject(mapDomFailure(operation, transaction.error ?? request.error));
      };
      transaction.onabort = () => {
        reject(mapDomFailure(operation, transaction.error ?? request.error));
      };
    });
  }
}
