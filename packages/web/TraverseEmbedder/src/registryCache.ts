/**
 * Host-owned verified registry dependency cache (Spec `080-embedded-registry-cache`).
 *
 * Separates network-capable `prepareRegistryDependency` from offline
 * `resolveRegistryDependencyOffline` used at embedder `init`. The host owns
 * cache storage; Traverse never synthesizes a production path or performs
 * network I/O outside prepare.
 */

import { BundleRejectedError, verifyArtifactDigest } from "./bundleValidation.js";

/** Stable Spec 080 FR-007 error codes. */
export type RegistryCacheErrorCode =
  | "registry_sync_missing"
  | "registry_version_not_found"
  | "registry_dependency_yanked"
  | "registry_prepare_failed"
  | "registry_artifact_digest_mismatch"
  | "registry_cache_entry_missing";

export class RegistryCacheError extends Error {
  readonly code: RegistryCacheErrorCode;

  constructor(code: RegistryCacheErrorCode, message: string) {
    super(message);
    this.name = "RegistryCacheError";
    this.code = code;
  }
}

/** Host-owned byte store for verified cache entries. */
export interface RegistryCacheStore {
  get(key: string): Promise<Uint8Array | undefined> | Uint8Array | undefined;
  set(key: string, bytes: Uint8Array): Promise<void> | void;
  delete(key: string): Promise<void> | void;
  keys(): Promise<readonly string[]> | readonly string[];
}

/** In-memory host cache store for tests and simple hosts. */
export class MemoryRegistryCacheStore implements RegistryCacheStore {
  private readonly entries = new Map<string, Uint8Array>();

  get(key: string): Uint8Array | undefined {
    return this.entries.get(key);
  }

  set(key: string, bytes: Uint8Array): void {
    this.entries.set(key, bytes);
  }

  delete(key: string): void {
    this.entries.delete(key);
  }

  keys(): readonly string[] {
    return [...this.entries.keys()];
  }
}

export interface RegistryReference {
  readonly namespace: string;
  readonly id: string;
  readonly versionRange: string;
}

export interface PublicRegistryCapabilityRecord {
  readonly namespace: string;
  readonly id: string;
  readonly version: string;
  readonly digest: string;
  readonly artifactUrl: string;
  readonly contractDigest: string;
  readonly contractUrl: string;
  readonly deprecated: boolean;
}

export interface SyncedPublicRegistryState {
  readonly releaseTag: string;
  readonly capabilities: readonly PublicRegistryCapabilityRecord[];
}

export interface RegistryArtifactFetcher {
  fetch(url: string): Promise<Uint8Array> | Uint8Array;
}

export interface RegistryPrepareEvidence {
  readonly namespace: string;
  readonly id: string;
  readonly selectedVersion: string;
  readonly versionRange: string;
  readonly sourceRelease: string;
  readonly indexDigest: string;
  readonly artifactDigest: string;
  readonly verifiedAt: number;
  readonly outcome: "prepared" | "resolved";
}

export interface VerifiedRegistryDependency {
  readonly wasmBytes: Uint8Array;
  readonly contractBytes: Uint8Array;
  readonly wasmDigest: string;
  readonly evidence: RegistryPrepareEvidence;
}

interface CacheEntryMeta {
  readonly namespace: string;
  readonly id: string;
  readonly selectedVersion: string;
  readonly versionRange: string;
  readonly sourceRelease: string;
  readonly indexDigest: string;
  readonly artifactDigest: string;
  readonly contractDigest: string;
  readonly verifiedAt: number;
}

function textEncoder(): TextEncoder {
  return new TextEncoder();
}

function textDecoder(): TextDecoder {
  return new TextDecoder();
}

/** Canonical snapshot bytes bind preparation to the same identity as planning. */
function canonicalJson(value: unknown): string {
  if (Array.isArray(value)) {
    return `[${value.map(canonicalJson).join(",")}]`;
  }
  if (value !== null && typeof value === "object") {
    const record = value as Record<string, unknown>;
    return `{${Object.keys(record).sort().map((key) => `${JSON.stringify(key)}:${canonicalJson(record[key])}`).join(",")}}`;
  }
  return JSON.stringify(value);
}

async function sha256Hex(bytes: Uint8Array): Promise<string> {
  const copy: Uint8Array<ArrayBuffer> = new Uint8Array(bytes.byteLength);
  copy.set(bytes);
  const digest = await crypto.subtle.digest("SHA-256", copy);
  return [...new Uint8Array(digest)]
    .map((byte) => byte.toString(16).padStart(2, "0"))
    .join("");
}

async function digestFor(bytes: Uint8Array): Promise<string> {
  return `sha256:${await sha256Hex(bytes)}`;
}

function normalizeDigest(digest: string): string | undefined {
  if (!digest.startsWith("sha256:")) {
    return undefined;
  }
  const hex = digest.slice("sha256:".length).toLowerCase();
  if (hex.length !== 64 || !/^[0-9a-f]+$/.test(hex)) {
    return undefined;
  }
  return hex;
}

async function refKey(reference: RegistryReference): Promise<string> {
  const material = `${reference.namespace}:${reference.id}:${reference.versionRange}`;
  return `refs/${await sha256Hex(textEncoder().encode(material))}.json`;
}

function artifactKey(digestHex: string): string {
  return `sha256/${digestHex}`;
}

function metaKey(digestHex: string): string {
  return `meta/${digestHex}.json`;
}

function compareSemver(left: string, right: string): number {
  const parse = (value: string): number[] =>
    value.split(".").map((part) => Number.parseInt(part, 10) || 0);
  const a = parse(left);
  const b = parse(right);
  for (let i = 0; i < Math.max(a.length, b.length); i += 1) {
    const delta = (a[i] ?? 0) - (b[i] ?? 0);
    if (delta !== 0) {
      return delta;
    }
  }
  return 0;
}

function matchesCaretRange(version: string, range: string): boolean {
  const caret = range.startsWith("^") ? range.slice(1) : range;
  if (range === "*" || range === "x") {
    return true;
  }
  if (!range.startsWith("^")) {
    return version === range || version.startsWith(`${range}.`);
  }
  const [major = 0, minor = 0] = caret
    .split(".")
    .map((part) => Number.parseInt(part, 10) || 0);
  const [vMajorRaw, vMinorRaw = 0, vPatchRaw = 0] = version
    .split(".")
    .map((part) => Number.parseInt(part, 10) || 0);
  const vMajor = vMajorRaw ?? 0;
  const vMinor = vMinorRaw ?? 0;
  const vPatch = vPatchRaw ?? 0;
  if (vMajor !== major) {
    return false;
  }
  if (major === 0) {
    return vMinor === minor;
  }
  return (
    compareSemver(version, caret) >= 0 &&
    compareSemver(`${major + 1}.0.0`, version) > 0 &&
    Number.isFinite(vPatch)
  );
}

function selectHighestActive(
  snapshot: SyncedPublicRegistryState,
  reference: RegistryReference,
): PublicRegistryCapabilityRecord {
  const matching = snapshot.capabilities.filter(
    (record) =>
      record.namespace === reference.namespace &&
      record.id === reference.id &&
      matchesCaretRange(record.version, reference.versionRange),
  );
  const active = matching
    .filter((record) => !record.deprecated)
    .sort((left, right) => compareSemver(right.version, left.version));
  if (active[0]) {
    return active[0];
  }
  if (matching.length === 0) {
    throw new RegistryCacheError(
      "registry_version_not_found",
      `no synced public registry version for ${reference.namespace}:${reference.id} satisfies ${reference.versionRange}`,
    );
  }
  throw new RegistryCacheError(
    "registry_dependency_yanked",
    `only yanked public registry versions for ${reference.namespace}:${reference.id} satisfy ${reference.versionRange}`,
  );
}

async function writeVerified(
  store: RegistryCacheStore,
  digest: string,
  bytes: Uint8Array,
): Promise<string> {
  const hex = normalizeDigest(digest);
  if (!hex) {
    throw new RegistryCacheError(
      "registry_artifact_digest_mismatch",
      "registry digest must be sha256: followed by 64 hex characters",
    );
  }
  try {
    await verifyArtifactDigest(bytes, digest, "registry artifact");
  } catch (error) {
    if (error instanceof BundleRejectedError) {
      throw new RegistryCacheError(
        "registry_artifact_digest_mismatch",
        "registry artifact bytes do not match the published digest",
      );
    }
    throw error;
  }
  const key = artifactKey(hex);
  const existing = await store.get(key);
  if (existing) {
    try {
      await verifyArtifactDigest(existing, digest, "cached registry artifact");
    } catch {
      throw new RegistryCacheError(
        "registry_artifact_digest_mismatch",
        "existing registry cache entry digest mismatch",
      );
    }
    return hex;
  }
  await store.set(key, bytes);
  return hex;
}

/**
 * Prepare one `registry_ref` into a host-owned verified cache.
 * Only this function may call the fetcher.
 */
export async function prepareRegistryDependency(
  store: RegistryCacheStore,
  snapshot: SyncedPublicRegistryState,
  reference: RegistryReference,
  fetcher: RegistryArtifactFetcher,
): Promise<RegistryPrepareEvidence> {
  if (snapshot.capabilities.length === 0) {
    throw new RegistryCacheError(
      "registry_sync_missing",
      "synced registry index snapshot contains no capabilities",
    );
  }
  const record = selectHighestActive(snapshot, reference);
  const indexDigest = await digestFor(textEncoder().encode(canonicalJson(snapshot)));
  let artifactBytes: Uint8Array;
  let contractBytes: Uint8Array;
  try {
    artifactBytes = await fetcher.fetch(record.artifactUrl);
    contractBytes = await fetcher.fetch(record.contractUrl);
  } catch (error) {
    throw new RegistryCacheError(
      "registry_prepare_failed",
      `host registry fetch failed: ${error instanceof Error ? error.message : "unknown"}`,
    );
  }
  try {
    await writeVerified(store, record.digest, artifactBytes);
    await writeVerified(store, record.contractDigest, contractBytes);
  } catch (error) {
    if (error instanceof RegistryCacheError) {
      throw error;
    }
    throw new RegistryCacheError(
      "registry_artifact_digest_mismatch",
      "registry artifact bytes do not match the published digest",
    );
  }
  const artifactHex = normalizeDigest(record.digest);
  if (!artifactHex) {
    throw new RegistryCacheError(
      "registry_prepare_failed",
      "artifact digest must be sha256: followed by 64 hex characters",
    );
  }
  const verifiedAt = Math.floor(Date.now() / 1000);
  const meta: CacheEntryMeta = {
    namespace: record.namespace,
    id: record.id,
    selectedVersion: record.version,
    versionRange: reference.versionRange,
    sourceRelease: snapshot.releaseTag,
    indexDigest,
    artifactDigest: record.digest,
    contractDigest: record.contractDigest,
    verifiedAt,
  };
  await store.set(metaKey(artifactHex), textEncoder().encode(JSON.stringify(meta)));
  await store.set(
    await refKey(reference),
    textEncoder().encode(
      JSON.stringify({
        artifactDigest: record.digest,
        contractDigest: record.contractDigest,
      }),
    ),
  );
  return {
    namespace: record.namespace,
    id: record.id,
    selectedVersion: record.version,
    versionRange: reference.versionRange,
    sourceRelease: snapshot.releaseTag,
    indexDigest,
    artifactDigest: record.digest,
    verifiedAt,
    outcome: "prepared",
  };
}

/** Resolve a previously prepared `registry_ref` offline. */
export async function resolveRegistryDependencyOffline(
  store: RegistryCacheStore,
  reference: RegistryReference,
): Promise<VerifiedRegistryDependency> {
  const pointerBytes = await store.get(await refKey(reference));
  if (!pointerBytes) {
    throw new RegistryCacheError(
      "registry_cache_entry_missing",
      "verified registry cache entry is missing for registry_ref",
    );
  }
  const pointer = JSON.parse(textDecoder().decode(pointerBytes)) as {
    artifactDigest?: string;
    contractDigest?: string;
  };
  if (!pointer.artifactDigest || !pointer.contractDigest) {
    throw new RegistryCacheError(
      "registry_cache_entry_missing",
      "verified registry cache entry is missing for registry_ref",
    );
  }
  const artifactHex = normalizeDigest(pointer.artifactDigest);
  const contractHex = normalizeDigest(pointer.contractDigest);
  if (!artifactHex || !contractHex) {
    throw new RegistryCacheError(
      "registry_cache_entry_missing",
      "verified registry cache entry is missing for registry_ref",
    );
  }
  const wasmBytes = await store.get(artifactKey(artifactHex));
  const contractBytes = await store.get(artifactKey(contractHex));
  const metaBytes = await store.get(metaKey(artifactHex));
  if (!wasmBytes || !contractBytes || !metaBytes) {
    throw new RegistryCacheError(
      "registry_cache_entry_missing",
      "verified registry cache entry is missing for registry_ref",
    );
  }
  await verifyArtifactDigest(wasmBytes, pointer.artifactDigest, "cached registry artifact");
  await verifyArtifactDigest(
    contractBytes,
    pointer.contractDigest,
    "cached registry contract",
  );
  const meta = JSON.parse(textDecoder().decode(metaBytes)) as CacheEntryMeta;
  return {
    wasmBytes,
    contractBytes,
    wasmDigest: pointer.artifactDigest,
    evidence: {
      namespace: meta.namespace,
      id: meta.id,
      selectedVersion: meta.selectedVersion,
      versionRange: meta.versionRange,
      sourceRelease: meta.sourceRelease,
      indexDigest: meta.indexDigest,
      artifactDigest: meta.artifactDigest,
      verifiedAt: meta.verifiedAt,
      outcome: "resolved",
    },
  };
}

/** Remove one verified artifact entry by digest. */
export async function evictRegistryCacheEntry(
  store: RegistryCacheStore,
  artifactDigest: string,
): Promise<void> {
  const hex = normalizeDigest(artifactDigest);
  if (!hex) {
    throw new RegistryCacheError(
      "registry_prepare_failed",
      "artifact digest must be sha256: followed by 64 hex characters",
    );
  }
  await store.delete(artifactKey(hex));
  await store.delete(metaKey(hex));
}

/** Clear every verified entry in the host store. */
export async function evictAllRegistryCacheEntries(
  store: RegistryCacheStore,
): Promise<void> {
  for (const key of await store.keys()) {
    if (
      key.startsWith("sha256/") ||
      key.startsWith("meta/") ||
      key.startsWith("refs/")
    ) {
      await store.delete(key);
    }
  }
}
