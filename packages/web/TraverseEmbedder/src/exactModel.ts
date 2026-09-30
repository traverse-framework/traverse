/**
 * Spec 138 browser surfaces: host-staged I/O + wasm-cpu guest execute.
 * Matches native `traverse_runtime::exact_model` envelopes and guest ABI v1.
 */

export const MODEL_GUEST_ABI_VERSION = 1 as const;
export const MODEL_EXECUTE_EXPORT = "model_execute" as const;
/** Guest ABI v2 buffer allocator export (Decision 105). */
export const MODEL_ALLOC_EXPORT = "model_alloc" as const;
/** Highest supported manifest `abi_version` (1 = fixed offsets, 2 = guest `model_alloc`). */
export const MAX_MODEL_ABI_VERSION = 2 as const;
export const PLACEMENT_WASM_CPU = "wasm-cpu" as const;

/** Model package manifest schema version (Spec 138 0.4.0, Decision 101). */
export const MODEL_PACKAGE_SCHEMA_VERSION = "2.0.0" as const;
export const MODEL_SIGNATURE_ALG_ED25519 = "ed25519" as const;

export type CommercialUse = "allowed" | "restricted" | "prohibited";

/** Signed model rights, exposed read-only to hosts/UIs unchanged. */
export type ModelRights = {
  readonly license_id: string;
  readonly attribution: string;
  readonly redistribution: string;
  readonly commercial_use: CommercialUse;
  readonly source_url: string;
};

/** Exact app-manifest pin (Spec 044 `exact_model_dependencies`). */
export type ExactModelPin = {
  readonly model_id: string;
  readonly version: string;
  /** SHA-256 of the exact signed `model.manifest.json` bytes. */
  readonly digest: string;
  readonly offline_allowed: boolean;
  /** Browser resolution is single exact-ref `wasm-cpu` only (#1460 is the future extension). */
  readonly target: string;
  readonly rights: { readonly license_id: string; readonly commercial_use: CommercialUse };
  /** Optional narrowing to one host-trusted signer key id. */
  readonly key_id?: string;
};

export type ModelPackageManifest = {
  readonly schema_version: string;
  readonly model_id: string;
  readonly version: string;
  readonly wasm_digest: string;
  readonly registry_ref: string;
  readonly executable_format: string;
  readonly abi_version: number;
  readonly input_schema_ref: string;
  readonly input_schema_version: string;
  readonly output_schema_ref: string;
  readonly output_schema_version: string;
  readonly rights: ModelRights;
  readonly supported_profiles: readonly string[];
  readonly max_memory_bytes: number;
  readonly max_fuel: number;
  readonly max_input_bytes: number;
  readonly max_output_bytes: number;
  readonly max_execution_ms: number;
  readonly offline_allowed: boolean;
};

/** Detached `model.sig.json` over the exact manifest bytes. */
export type ModelPackageSignature = {
  readonly alg: string;
  readonly key_id: string;
  readonly signature: string;
};

/** Stable `reason` refining `model_unavailable` / `model_incompatible` (Decision 101). */
export type ModelFailureReason =
  | "pin_mismatch"
  | "pin_ambiguous"
  | "signature_invalid"
  | "key_untrusted"
  | "digest_mismatch"
  | "manifest_invalid"
  | "rights_incomplete"
  | "rights_mismatch"
  | "target_unsupported"
  | "crypto_unavailable"
  | "candidate_unsupported";

/** Typed `model.execute` result with identity, placement, and redacted trace. */
export type ExactModelExecution = {
  readonly output_ref: string;
  readonly placement: typeof PLACEMENT_WASM_CPU;
  readonly model_ref: { readonly model_id: string; readonly version: string; readonly digest: string };
  readonly target: typeof PLACEMENT_WASM_CPU;
  readonly trace: {
    readonly model_id: string;
    readonly version: string;
    readonly digest: string;
    readonly placement: typeof PLACEMENT_WASM_CPU;
    readonly data_classification: string;
    readonly usage: { readonly input_bytes: number; readonly output_bytes: number; readonly duration_ms: number };
  };
};

export class ExactModelError extends Error {
  readonly code: string;
  readonly reason: ModelFailureReason | undefined;
  constructor(code: string, message: string, reason?: ModelFailureReason) {
    super(message);
    this.name = "ExactModelError";
    this.code = code;
    this.reason = reason;
  }
}

function incompatible(reason: ModelFailureReason, message: string): ExactModelError {
  return new ExactModelError("model_incompatible", message, reason);
}

function normalizeDigest(value: string): string {
  const trimmed = value.trim();
  return (trimmed.startsWith("sha256:") ? trimmed.slice("sha256:".length) : trimmed).toLowerCase();
}

function toHex(bytes: Uint8Array): string {
  return [...bytes].map((b) => b.toString(16).padStart(2, "0")).join("");
}

function fromHex(value: string): Uint8Array | undefined {
  if (value.length % 2 !== 0 || !/^[0-9a-fA-F]*$/.test(value)) {
    return undefined;
  }
  const out = new Uint8Array(value.length / 2);
  for (let index = 0; index < out.length; index += 1) {
    out[index] = parseInt(value.slice(index * 2, index * 2 + 2), 16);
  }
  return out;
}

async function digestHex(bytes: Uint8Array): Promise<string> {
  const copy = new Uint8Array(bytes);
  const hash = await crypto.subtle.digest("SHA-256", copy);
  return toHex(new Uint8Array(hash));
}

/** `key_id` for a raw 32-byte Ed25519 public key: `ed25519:` + hex SHA-256. */
export async function modelSigningKeyId(publicKey: Uint8Array): Promise<string> {
  return `${MODEL_SIGNATURE_ALG_ED25519}:${await digestHex(publicKey)}`;
}

const MANIFEST_KEYS = [
  "schema_version", "model_id", "version", "wasm_digest", "registry_ref", "executable_format",
  "abi_version", "input_schema_ref", "input_schema_version", "output_schema_ref",
  "output_schema_version", "rights", "supported_profiles", "max_memory_bytes", "max_fuel",
  "max_input_bytes", "max_output_bytes", "max_execution_ms", "offline_allowed",
] as const;
const RIGHTS_KEYS = ["license_id", "attribution", "redistribution", "commercial_use", "source_url"] as const;
const SIGNATURE_KEYS = ["alg", "key_id", "signature"] as const;
const COMMERCIAL_USE: readonly string[] = ["allowed", "restricted", "prohibited"];

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** Exact key set, fail closed on unknown or missing fields (mirrors serde deny_unknown_fields). */
function hasExactKeys(value: unknown, keys: readonly string[]): value is Record<string, unknown> {
  return (
    isRecord(value) &&
    Object.keys(value).length === keys.length &&
    keys.every((key) => Object.hasOwn(value, key))
  );
}

function parseJson(bytes: Uint8Array): unknown {
  try {
    return JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(bytes));
  } catch {
    return undefined;
  }
}

function parseSignature(bytes: Uint8Array): ModelPackageSignature {
  const value = parseJson(bytes);
  if (
    !hasExactKeys(value, SIGNATURE_KEYS) ||
    typeof value.alg !== "string" ||
    typeof value.key_id !== "string" ||
    typeof value.signature !== "string"
  ) {
    throw incompatible("signature_invalid", "model signature document is malformed");
  }
  return value as ModelPackageSignature;
}

function parseManifest(bytes: Uint8Array): ModelPackageManifest {
  const value = parseJson(bytes);
  const numeric = [
    "abi_version", "max_memory_bytes", "max_fuel", "max_input_bytes", "max_output_bytes",
    "max_execution_ms",
  ];
  const text = MANIFEST_KEYS.filter(
    (key) => !numeric.includes(key) && !["rights", "supported_profiles", "offline_allowed"].includes(key),
  );
  if (
    !hasExactKeys(value, MANIFEST_KEYS) ||
    !hasExactKeys(value.rights, RIGHTS_KEYS) ||
    !Object.values(value.rights).every((field) => typeof field === "string") ||
    !COMMERCIAL_USE.includes(value.rights.commercial_use as string) ||
    !text.every((key) => typeof value[key] === "string") ||
    !numeric.every((key) => Number.isInteger(value[key]) && (value[key] as number) >= 0) ||
    !Array.isArray(value.supported_profiles) ||
    !value.supported_profiles.every((profile) => typeof profile === "string") ||
    typeof value.offline_allowed !== "boolean"
  ) {
    throw incompatible("manifest_invalid", "model manifest is malformed or has unknown fields");
  }
  return value as unknown as ModelPackageManifest;
}

/** Same rules and order as native `ModelPackageManifest::validate`. */
function validateManifest(manifest: ModelPackageManifest): void {
  const rights = manifest.rights;
  if ([rights.license_id, rights.attribution, rights.redistribution, rights.source_url].some((v) => !v.trim())) {
    throw incompatible("rights_incomplete", "model manifest rights are incomplete");
  }
  if (
    [manifest.wasm_digest, manifest.executable_format, manifest.input_schema_ref, manifest.output_schema_ref].some(
      (v) => !v.trim(),
    )
  ) {
    throw incompatible("manifest_invalid", "model manifest missing required field");
  }
  if (
    manifest.schema_version !== MODEL_PACKAGE_SCHEMA_VERSION ||
    manifest.abi_version > MAX_MODEL_ABI_VERSION ||
    [
      manifest.abi_version, manifest.max_memory_bytes, manifest.max_fuel, manifest.max_input_bytes,
      manifest.max_output_bytes, manifest.max_execution_ms,
    ].some((limit) => limit === 0)
  ) {
    throw incompatible("manifest_invalid", "model manifest schema version, resource limits, or ABI are invalid");
  }
  if (!manifest.supported_profiles.includes(PLACEMENT_WASM_CPU)) {
    throw incompatible("target_unsupported", "model manifest does not support wasm-cpu");
  }
}

type StoredPackage = {
  readonly manifest: ModelPackageManifest;
  readonly manifestBytes: Uint8Array;
  readonly wasm: Uint8Array;
};

/** Encode Spec 138 little-endian guest frame. */
export function encodeGuestFrame(dtype: number, dims: readonly number[], payload: Uint8Array): Uint8Array {
  const rank = dims.length;
  const out = new Uint8Array(4 + rank * 4 + 4 + payload.length);
  const view = new DataView(out.buffer);
  view.setUint16(0, MODEL_GUEST_ABI_VERSION, true);
  out[2] = dtype & 0xff;
  out[3] = rank & 0xff;
  let offset = 4;
  for (const dim of dims) {
    view.setUint32(offset, dim >>> 0, true);
    offset += 4;
  }
  view.setUint32(offset, payload.length >>> 0, true);
  offset += 4;
  out.set(payload, offset);
  return out;
}

export class ModelIoStore {
  private inputs = new Map<string, Uint8Array>();
  private outputs = new Map<string, Uint8Array>();
  private artifacts = new Map<string, Uint8Array>();
  private nextInput = 0;
  private nextOutput = 0;
  private nextArtifact = 0;

  stageModelInput(bytes: Uint8Array, maxBytes: number): string {
    if (bytes.length === 0 || bytes.length > maxBytes) {
      throw new ExactModelError("input_limit_exceeded", "staged model input empty or exceeds ceiling");
    }
    this.nextInput += 1;
    const id = `input-${this.nextInput}`;
    this.inputs.set(id, bytes);
    return id;
  }

  takeInput(inputRef: string): Uint8Array {
    const bytes = this.inputs.get(inputRef);
    if (!bytes) {
      throw new ExactModelError("invalid_input", "input_ref missing or already consumed");
    }
    this.inputs.delete(inputRef);
    return bytes;
  }

  putOutput(bytes: Uint8Array): string {
    this.nextOutput += 1;
    const id = `output-${this.nextOutput}`;
    this.outputs.set(id, bytes);
    return id;
  }

  readModelOutput(outputRef: string, maxBytes: number): Uint8Array {
    const bytes = this.outputs.get(outputRef);
    if (!bytes) {
      throw new ExactModelError("unavailable", "output_ref missing or expired");
    }
    if (bytes.length > maxBytes) {
      throw new ExactModelError("input_limit_exceeded", "output exceeds read ceiling");
    }
    return bytes;
  }

  /**
   * Stage bounded bytes as a multi-read opaque `artifact_ref` (Spec 140 /
   * Spec 138 0.2.0). Readable until `dropRef` or `shutdown`; model
   * `input_ref` keeps its single-consume rule.
   */
  stageArtifact(bytes: Uint8Array, maxBytes: number): string {
    if (bytes.length === 0 || bytes.length > maxBytes) {
      throw new ExactModelError("input_limit_exceeded", "staged artifact empty or exceeds ceiling");
    }
    this.nextArtifact += 1;
    const id = `artifact-${this.nextArtifact}`;
    this.artifacts.set(id, new Uint8Array(bytes));
    return id;
  }

  /** Runtime-mediated bounded read of an `artifact_ref`. Repeatable. */
  readArtifact(artifactRef: string, maxBytes: number): Uint8Array {
    const bytes = this.artifacts.get(artifactRef);
    if (!bytes) {
      throw new ExactModelError("unavailable", "artifact_ref missing or expired");
    }
    if (bytes.length > maxBytes) {
      throw new ExactModelError("input_limit_exceeded", "artifact exceeds read ceiling");
    }
    return new Uint8Array(bytes);
  }

  /** Drop an input, output, or artifact ref. */
  dropRef(reference: string): void {
    this.inputs.delete(reference);
    this.outputs.delete(reference);
    this.artifacts.delete(reference);
  }

  /** Invalidate every staged ref (runtime shutdown). */
  shutdown(): void {
    this.inputs.clear();
    this.outputs.clear();
    this.artifacts.clear();
  }
}

export type ExactModelBrowserHostOptions = {
  /** Host-owned trusted Ed25519 public keys (hex, raw 32 bytes). Apps can never add trust. */
  readonly trustedPublicKeysHex: readonly string[];
};

/**
 * Browser Spec 138 host. Accepts only single, already-selected exact-ref
 * `wasm-cpu` pins; mixed/Ollama candidate resolution is #1460. All
 * verification is local (WebCrypto), so registration and execution make
 * zero network calls.
 */
export class ExactModelBrowserHost {
  readonly io = new ModelIoStore();
  private readonly packages = new Map<string, StoredPackage>();
  private readonly pins: readonly ExactModelPin[];
  private readonly trustedPublicKeysHex: readonly string[];
  private trustedKeys: Promise<Map<string, CryptoKey>> | undefined;

  constructor(pins: readonly ExactModelPin[], options: ExactModelBrowserHostOptions) {
    for (const pin of pins) {
      if (pin.target !== PLACEMENT_WASM_CPU) {
        throw new ExactModelError(
          "model_incompatible",
          "browser resolution accepts only single exact-ref wasm-cpu pins",
          "candidate_unsupported",
        );
      }
    }
    this.pins = pins;
    this.trustedPublicKeysHex = options.trustedPublicKeysHex;
  }

  private loadTrustedKeys(): Promise<Map<string, CryptoKey>> {
    this.trustedKeys ??= (async () => {
      const keys = new Map<string, CryptoKey>();
      for (const hex of this.trustedPublicKeysHex) {
        const raw = fromHex(hex);
        if (!raw || raw.length !== 32) {
          throw incompatible("key_untrusted", "model signing public key is invalid");
        }
        let key: CryptoKey;
        try {
          key = await crypto.subtle.importKey("raw", new Uint8Array(raw), { name: "Ed25519" }, false, ["verify"]);
        } catch {
          throw new ExactModelError(
            "model_unavailable",
            "this environment cannot verify Ed25519 model signatures",
            "crypto_unavailable",
          );
        }
        keys.set(await modelSigningKeyId(raw), key);
      }
      return keys;
    })();
    return this.trustedKeys;
  }

  /**
   * Verify and admit a signed package (Decision 101): Ed25519 signature by a
   * host-trusted key over the exact manifest bytes, manifest digest equal to
   * exactly one pin, WASM digest, rights, target, and limits.
   */
  async registerPackage(manifestBytes: Uint8Array, wasm: Uint8Array, signatureBytes: Uint8Array): Promise<string> {
    const signature = parseSignature(signatureBytes);
    if (signature.alg !== MODEL_SIGNATURE_ALG_ED25519) {
      throw incompatible("signature_invalid", "unsupported model signature algorithm");
    }
    const key = (await this.loadTrustedKeys()).get(signature.key_id);
    if (!key) {
      throw incompatible("key_untrusted", "model signing key is not host-trusted");
    }
    const signatureRaw = fromHex(signature.signature);
    if (!signatureRaw || signatureRaw.length !== 64) {
      throw incompatible("signature_invalid", "model signature is malformed");
    }
    const verified = await crypto.subtle.verify(
      { name: "Ed25519" },
      key,
      new Uint8Array(signatureRaw),
      new Uint8Array(manifestBytes),
    );
    if (!verified) {
      throw incompatible("signature_invalid", "model signature verification failed");
    }
    const digest = await digestHex(manifestBytes);
    const pin = this.pins.find((candidate) => normalizeDigest(candidate.digest) === digest);
    if (!pin) {
      throw new ExactModelError(
        "model_unavailable",
        "signed package digest does not match an exact_model_dependencies pin",
        "pin_mismatch",
      );
    }
    if (this.pins.filter((other) => other.model_id === pin.model_id && other.version === pin.version).length > 1) {
      throw incompatible("pin_ambiguous", "more than one exact_model_dependencies pin names this model id and version");
    }
    const manifest = parseManifest(manifestBytes);
    if (pin.key_id !== undefined && pin.key_id !== signature.key_id) {
      throw incompatible("key_untrusted", "package signer is not the key the pin requires");
    }
    if (manifest.model_id !== pin.model_id || manifest.version !== pin.version) {
      throw incompatible("pin_mismatch", "signed package identity does not match its pin");
    }
    validateManifest(manifest);
    if (!manifest.supported_profiles.includes(pin.target)) {
      throw incompatible("target_unsupported", "pin target is not supported by the package");
    }
    if (
      manifest.rights.license_id !== pin.rights.license_id ||
      manifest.rights.commercial_use !== pin.rights.commercial_use
    ) {
      throw incompatible("rights_mismatch", "signed package rights differ from the rights the pin declares");
    }
    if (normalizeDigest(manifest.wasm_digest) !== (await digestHex(wasm))) {
      throw incompatible("digest_mismatch", "model wasm digest mismatch");
    }
    this.packages.set(digest, {
      manifest,
      manifestBytes: new Uint8Array(manifestBytes),
      wasm: new Uint8Array(wasm),
    });
    return digest;
  }

  /** Signed rights of a registered package, for host/UI display. */
  modelRights(digest: string): ModelRights | undefined {
    return this.packages.get(normalizeDigest(digest))?.manifest.rights;
  }

  async execute(args: {
    readonly model_ref: { model_id: string; version: string; digest: string };
    readonly input_ref: string;
    readonly policy_ref: string;
    readonly data_classification: string;
    readonly input_schema_ref: string;
    readonly input_schema_version: string;
    readonly max_output_bytes: number;
    readonly allowed_classifications: readonly string[];
    readonly timeout_ms?: number;
    readonly signal?: AbortSignal;
  }): Promise<ExactModelExecution> {
    if (args.signal?.aborted) {
      throw new ExactModelError("cancelled", "model.execute cancelled before invoke");
    }
    if (!args.policy_ref) {
      throw new ExactModelError("policy_denied", "policy_ref is not activated");
    }
    if (!args.allowed_classifications.includes(args.data_classification)) {
      throw new ExactModelError("policy_denied", "data_classification denied by policy");
    }
    const digest = normalizeDigest(args.model_ref.digest);
    const pin = this.pins.find(
      (candidate) =>
        candidate.model_id === args.model_ref.model_id &&
        candidate.version === args.model_ref.version &&
        normalizeDigest(candidate.digest) === digest,
    );
    if (!pin) {
      throw new ExactModelError(
        "model_unavailable",
        "model_ref does not match an exact_model_dependencies pin",
        "pin_mismatch",
      );
    }
    const pack = this.packages.get(digest);
    if (!pack) {
      throw new ExactModelError("model_unavailable", "model package not present in verified cache");
    }
    if (
      (await digestHex(pack.manifestBytes)) !== digest ||
      (await digestHex(pack.wasm)) !== normalizeDigest(pack.manifest.wasm_digest)
    ) {
      throw incompatible("digest_mismatch", "cached model package bytes no longer match the pinned digest");
    }
    if (
      pack.manifest.input_schema_ref !== args.input_schema_ref ||
      pack.manifest.input_schema_version !== args.input_schema_version
    ) {
      throw new ExactModelError("model_incompatible", "input schema does not match model manifest");
    }
    const input = this.io.takeInput(args.input_ref);
    if (input.length > pack.manifest.max_input_bytes) {
      throw new ExactModelError("resource_exhausted", "input exceeds model manifest ceiling");
    }
    const ceiling = Math.min(args.max_output_bytes, pack.manifest.max_output_bytes);
    const timeout = Math.min(args.timeout_ms ?? pack.manifest.max_execution_ms, pack.manifest.max_execution_ms);
    const started = performance.now();
    const output = await runWasmCpu(pack.wasm, input, ceiling, pack.manifest.max_memory_bytes, pack.manifest.abi_version);
    const durationMs = performance.now() - started;
    if (args.signal?.aborted) {
      throw new ExactModelError("cancelled", "model.execute cancelled during invoke");
    }
    if (durationMs > timeout) {
      throw new ExactModelError("timeout", "model.execute exceeded timeout");
    }
    const modelRef = { model_id: pack.manifest.model_id, version: pack.manifest.version, digest };
    return {
      output_ref: this.io.putOutput(output),
      placement: PLACEMENT_WASM_CPU,
      model_ref: modelRef,
      target: PLACEMENT_WASM_CPU,
      trace: {
        ...modelRef,
        placement: PLACEMENT_WASM_CPU,
        data_classification: args.data_classification,
        usage: { input_bytes: input.length, output_bytes: output.length, duration_ms: durationMs },
      },
    };
  }
}

async function runWasmCpu(
  wasm: Uint8Array,
  input: Uint8Array,
  maxOutputBytes: number,
  maxMemoryBytes: number,
  abiVersion: number,
): Promise<Uint8Array> {
  const module = await WebAssembly.compile(new Uint8Array(wasm));
  // Deny-by-default: no imports.
  const instance = await WebAssembly.instantiate(module, {});
  const memory = instance.exports.memory;
  const execute = instance.exports[MODEL_EXECUTE_EXPORT];
  if (!(memory instanceof WebAssembly.Memory) || typeof execute !== "function") {
    throw new ExactModelError("model_incompatible", "model wasm missing memory or model_execute");
  }
  const [inPtr, outPtr] = abiVersion >= 2
    ? placeV2(instance, memory, input.length, maxOutputBytes, maxMemoryBytes)
    : placeV1(memory, input.length, maxOutputBytes, maxMemoryBytes);
  new Uint8Array(memory.buffer, inPtr, input.length).set(input);
  const outLen = Number(
    (execute as (a: number, b: number, c: number, d: number) => number)(
      inPtr,
      input.length,
      outPtr,
      maxOutputBytes,
    ),
  );
  if (!Number.isFinite(outLen) || outLen < 0 || outLen > maxOutputBytes) {
    throw new ExactModelError("resource_exhausted", "model returned invalid output length");
  }
  if (memory.buffer.byteLength > maxMemoryBytes) {
    throw new ExactModelError("resource_exhausted", "model memory ceiling exceeded");
  }
  return new Uint8Array(memory.buffer.slice(outPtr, outPtr + outLen));
}

/** Guest ABI v1: host-chosen fixed offsets, grown by the host. */
function placeV1(memory: WebAssembly.Memory, inLen: number, outCap: number, maxMemoryBytes: number): [number, number] {
  const inPtr = 64;
  const outPtr = inPtr + inLen + 64;
  const needed = outPtr + outCap;
  if (needed > maxMemoryBytes) {
    throw new ExactModelError("resource_exhausted", "model memory ceiling exceeded");
  }
  while (memory.buffer.byteLength < needed) {
    memory.grow(1);
  }
  return [inPtr, outPtr];
}

/**
 * Guest ABI v2 (Decision 105): buffers come from the guest's `model_alloc`.
 * Regions must be positive, in bounds, and disjoint (same rules as native).
 */
function placeV2(
  instance: WebAssembly.Instance,
  memory: WebAssembly.Memory,
  inLen: number,
  outCap: number,
  maxMemoryBytes: number,
): [number, number] {
  const alloc = instance.exports[MODEL_ALLOC_EXPORT];
  if (typeof alloc !== "function") {
    throw new ExactModelError("model_incompatible", "abi_version 2 model wasm missing model_alloc export");
  }
  let inPtr: number;
  let outPtr: number;
  try {
    inPtr = Number((alloc as (n: number) => number)(inLen));
    outPtr = Number((alloc as (n: number) => number)(outCap));
  } catch {
    throw new ExactModelError("execution_failed", "model_alloc trapped");
  }
  if (memory.buffer.byteLength > maxMemoryBytes) {
    throw new ExactModelError("resource_exhausted", "model memory ceiling exceeded");
  }
  const size = memory.buffer.byteLength;
  const inEnd = inPtr + inLen;
  const outEnd = outPtr + outCap;
  if (!(inPtr > 0 && outPtr > 0 && inEnd <= size && outEnd <= size) || (inPtr < outEnd && outPtr < inEnd)) {
    throw new ExactModelError("execution_failed", "model_alloc returned an invalid region");
  }
  return [inPtr, outPtr];
}
