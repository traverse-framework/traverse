/**
 * Browser driver for `runtime.wasm` (spec `1402` FR-006, Decision 86/88/89).
 *
 * Ports `crates/traverse-runtime/src/runtime_wasm_host.rs`'s
 * `RuntimeWasmHost` call conventions onto the WebAssembly JS API so the
 * browser package loads the same nested-wasmi orchestrator native hosts use,
 * rather than reimplementing capability WASI + `emit_event` in TypeScript.
 *
 * Durability / EventBroker publish stays host-owned (Decision 89): this
 * module only drains lifecycle + domain envelopes from `traverse_next_event`.
 * Callers map those onto `TraverseEmbedderApi` event callbacks.
 */

export type RuntimeWasmJson = null | boolean | number | string | RuntimeWasmJson[] | {
  readonly [key: string]: RuntimeWasmJson;
};

export interface RuntimeWasmEmitRef {
  readonly event_id: string;
  readonly version: string;
}

export interface RuntimeWasmCapabilityInit {
  readonly capabilityId: string;
  readonly capabilityVersion: string;
  readonly serviceType: "stateless" | "subscribable" | "stateful";
  readonly emits: readonly RuntimeWasmEmitRef[];
  /** Placement of this browser host — almost always `"browser"`. */
  readonly hostPlacementTarget: "browser" | "local" | "edge" | "cloud" | "worker" | "device";
  readonly permittedTargets: readonly (
    | "browser"
    | "local"
    | "edge"
    | "cloud"
    | "worker"
    | "device"
  )[];
  /**
   * Spec 139 app `state_machine` document. When present, `traverse_submit`
   * accepts `kind: "app_command"` envelopes against this machine.
   */
  readonly stateMachine?: RuntimeWasmJson;
}

export class RuntimeWasmHostError extends Error {
  constructor(message: string) {
    super(`runtime.wasm host error: ${message}`);
    this.name = "RuntimeWasmHostError";
  }
}

type AllocFn = (size: number) => number;
type DeallocFn = (ptr: number, len: number) => void;
type TriadFn = (ptr: number, len: number, out: number) => number;
type NextEventFn = (out: number) => number;
type ShutdownFn = (out: number) => number;

function err(message: string): RuntimeWasmHostError {
  return new RuntimeWasmHostError(message);
}

function encodeUtf8(text: string): Uint8Array {
  return new TextEncoder().encode(text);
}

function decodeUtf8(bytes: Uint8Array): string {
  return new TextDecoder().decode(bytes);
}

/**
 * Drives one `runtime-wasm-bridge/1.0.0` instance (spec `071` FR-006).
 * One instance corresponds to one guest module instantiation — `init` MUST
 * be called before `submit`.
 */
export class RuntimeWasmHost {
  private readonly memory: WebAssembly.Memory;
  private readonly alloc: AllocFn;
  private readonly deallocFn: DeallocFn;
  private readonly initFn: TriadFn;
  private readonly submitFn: TriadFn;
  private readonly nextEventFn: NextEventFn;
  private readonly shutdownFn: ShutdownFn;

  private constructor(
    memory: WebAssembly.Memory,
    alloc: AllocFn,
    deallocFn: DeallocFn,
    initFn: TriadFn,
    submitFn: TriadFn,
    nextEventFn: NextEventFn,
    shutdownFn: ShutdownFn,
  ) {
    this.memory = memory;
    this.alloc = alloc;
    this.deallocFn = deallocFn;
    this.initFn = initFn;
    this.submitFn = submitFn;
    this.nextEventFn = nextEventFn;
    this.shutdownFn = shutdownFn;
  }

  /** Instantiates `runtimeWasmBytes` and resolves the required ABI exports. */
  static async instantiate(runtimeWasmBytes: Uint8Array): Promise<RuntimeWasmHost> {
    let module: WebAssembly.Module;
    try {
      const copy = new Uint8Array(runtimeWasmBytes.byteLength);
      copy.set(runtimeWasmBytes);
      module = await WebAssembly.compile(copy.buffer);
    } catch (cause) {
      throw err(`module: ${String(cause)}`);
    }
    return RuntimeWasmHost.fromModule(module);
  }

  /**
   * Synchronously instantiates a previously compiled `runtime.wasm` module.
   * Used by `BundleEmbedder.submit` so the public embedder API stays sync
   * after `init` has already `WebAssembly.compile`d the orchestrator.
   */
  static fromModule(module: WebAssembly.Module): RuntimeWasmHost {
    let instance: WebAssembly.Instance;
    try {
      // runtime.wasm is import-free at the outer boundary (nested wasmi
      // links WASI + traverse_host for capability guests internally).
      instance = new WebAssembly.Instance(module, {});
    } catch (cause) {
      throw err(`instantiate: ${String(cause)}`);
    }
    const exports = instance.exports as Record<string, unknown>;
    const memory = exports["memory"];
    if (!(memory instanceof WebAssembly.Memory)) {
      throw err("missing export: memory");
    }
    const alloc = exports["traverse_alloc"];
    const deallocFn = exports["traverse_dealloc"];
    const initFn = exports["traverse_init"];
    const submitFn = exports["traverse_submit"];
    const nextEventFn = exports["traverse_next_event"];
    const shutdownFn = exports["traverse_shutdown"];
    if (typeof alloc !== "function") throw err("missing export: traverse_alloc");
    if (typeof deallocFn !== "function") throw err("missing export: traverse_dealloc");
    if (typeof initFn !== "function") throw err("missing export: traverse_init");
    if (typeof submitFn !== "function") throw err("missing export: traverse_submit");
    if (typeof nextEventFn !== "function") throw err("missing export: traverse_next_event");
    if (typeof shutdownFn !== "function") throw err("missing export: traverse_shutdown");
    return new RuntimeWasmHost(
      memory,
      alloc as AllocFn,
      deallocFn as DeallocFn,
      initFn as TriadFn,
      submitFn as TriadFn,
      nextEventFn as NextEventFn,
      shutdownFn as ShutdownFn,
    );
  }

  private writeBytes(bytes: Uint8Array): { ptr: number; len: number } {
    if (bytes.byteLength > 0x7fff_ffff) {
      throw err("payload too large for the wasm32 ABI (max i32::MAX bytes)");
    }
    const len = bytes.byteLength;
    const ptr = this.alloc(len);
    if (ptr <= 0 && len > 0) {
      throw err("traverse_alloc returned a non-positive pointer");
    }
    new Uint8Array(this.memory.buffer, ptr, len).set(bytes);
    return { ptr, len };
  }

  private dealloc(ptr: number, len: number): void {
    try {
      this.deallocFn(ptr, len);
    } catch {
      // Best-effort: a failed free leaks guest memory for this instance.
    }
  }

  private readDescriptor(descriptorPtr: number): Uint8Array {
    const header = new Uint8Array(this.memory.buffer, descriptorPtr, 8);
    const responsePtr = new DataView(header.buffer, header.byteOffset, 8).getInt32(0, true);
    const responseLen = new DataView(header.buffer, header.byteOffset, 8).getInt32(4, true);
    let response = new Uint8Array(0);
    if (responseLen > 0 && responsePtr >= 0) {
      response = new Uint8Array(this.memory.buffer, responsePtr, responseLen).slice();
    }
    this.dealloc(descriptorPtr, 8);
    if (responsePtr > 0) {
      this.dealloc(responsePtr, responseLen);
    }
    return response;
  }

  private callJson(target: TriadFn, payload: Uint8Array): { status: number; response: Uint8Array } {
    const outDescriptor = this.alloc(8);
    const written = this.writeBytes(payload);
    let status: number;
    try {
      status = target(written.ptr, written.len, outDescriptor);
    } finally {
      this.dealloc(written.ptr, written.len);
    }
    const response = this.readDescriptor(outDescriptor);
    return { status, response };
  }

  private parseResponse(bytes: Uint8Array): RuntimeWasmJson {
    if (bytes.byteLength === 0) {
      return null;
    }
    try {
      return JSON.parse(decodeUtf8(bytes)) as RuntimeWasmJson;
    } catch (cause) {
      throw err(`response JSON: ${String(cause)}`);
    }
  }

  /**
   * Calls `traverse_init` with capability metadata + nested WASM bytes
   * (`[u32 LE header_len][JSON header][artifact]`).
   */
  init(capability: RuntimeWasmCapabilityInit, capabilityWasm: Uint8Array): RuntimeWasmJson {
    const headerObject: Record<string, RuntimeWasmJson> = {
      capability_id: capability.capabilityId,
      capability_version: capability.capabilityVersion,
      service_type: capability.serviceType,
      emits: capability.emits.map((entry) => ({
        event_id: entry.event_id,
        version: entry.version,
      })),
      host_placement_target: capability.hostPlacementTarget,
      permitted_targets: [...capability.permittedTargets],
    };
    if (capability.stateMachine !== undefined) {
      headerObject.state_machine = capability.stateMachine;
      headerObject.app_id = capability.capabilityId;
    }
    const headerBytes = encodeUtf8(JSON.stringify(headerObject));
    const headerLen = headerBytes.byteLength;
    if (headerLen > 0xffff_ffff) {
      throw err("init header too large for the wasm32 ABI");
    }
    const payload = new Uint8Array(4 + headerLen + capabilityWasm.byteLength);
    new DataView(payload.buffer).setUint32(0, headerLen, true);
    payload.set(headerBytes, 4);
    payload.set(capabilityWasm, 4 + headerLen);

    const { status, response } = this.callJson(this.initFn, payload);
    const parsed = this.parseResponse(response);
    if (status !== 0) {
      throw err(`traverse_init rejected: ${JSON.stringify(parsed)}`);
    }
    return parsed;
  }

  /** Calls `traverse_submit` with raw capability-stdin bytes. */
  submit(request: Uint8Array): RuntimeWasmJson {
    const { status, response } = this.callJson(this.submitFn, request);
    const parsed = this.parseResponse(response);
    if (status !== 0) {
      throw err(`traverse_submit rejected: ${JSON.stringify(parsed)}`);
    }
    return parsed;
  }

  /** Drains every queued `traverse_next_event` envelope until the guest returns 0. */
  drainEvents(): RuntimeWasmJson[] {
    const events: RuntimeWasmJson[] = [];
    for (;;) {
      const outDescriptor = this.alloc(8);
      const hasEvent = this.nextEventFn(outDescriptor);
      if (hasEvent === 0) {
        this.dealloc(outDescriptor, 8);
        break;
      }
      const bytes = this.readDescriptor(outDescriptor);
      events.push(this.parseResponse(bytes));
    }
    return events;
  }

  shutdown(): RuntimeWasmJson {
    const outDescriptor = this.alloc(8);
    this.shutdownFn(outDescriptor);
    return this.parseResponse(this.readDescriptor(outDescriptor));
  }
}

/**
 * Maps drained `runtime.wasm` envelopes onto the embedder's event stream
 * shape (`capability_invoked` / `capability_result` / `capability_event`).
 * Lifecycle envelopes keep their type; domain events become
 * `capability_event` with `event_type` + payload (Decision 89 host publish).
 */
export function mapRuntimeWasmEvents(
  events: readonly RuntimeWasmJson[],
): Array<{ type: string; data: RuntimeWasmJson }> {
  const mapped: Array<{ type: string; data: RuntimeWasmJson }> = [];
  for (const event of events) {
    if (event === null || typeof event !== "object" || Array.isArray(event)) {
      continue;
    }
    const record = event as { readonly [key: string]: RuntimeWasmJson };
    const type = typeof record["type"] === "string" ? record["type"] : null;
    if (type === null) {
      continue;
    }
    const data = (record["data"] ?? {}) as RuntimeWasmJson;
    if (type === "capability_invoked" || type === "capability_result") {
      mapped.push({ type, data });
      continue;
    }
    // Domain event from nested emit_event — host publishes as capability_event.
    const dataRecord =
      data !== null && typeof data === "object" && !Array.isArray(data)
        ? (data as { readonly [key: string]: RuntimeWasmJson })
        : {};
    mapped.push({
      type: "capability_event",
      data: {
        event_type: type,
        version: dataRecord["version"] ?? null,
        payload: dataRecord["payload"] ?? data,
      },
    });
  }
  return mapped;
}

/**
 * Parse contract/manifest `emits` entries into `{ event_id, version }` pairs
 * for `RuntimeWasmHost.init`. Unknown shapes are skipped.
 */
export function parseDeclaredEmits(value: unknown): RuntimeWasmEmitRef[] {
  if (!Array.isArray(value)) {
    return [];
  }
  const declared: RuntimeWasmEmitRef[] = [];
  for (const entry of value) {
    if (entry === null || typeof entry !== "object" || Array.isArray(entry)) {
      continue;
    }
    const record = entry as { readonly [key: string]: unknown };
    const eventId = record["event_id"];
    const version = record["version"];
    if (typeof eventId === "string" && typeof version === "string") {
      declared.push({ event_id: eventId, version });
    }
  }
  return declared;
}

/** Maps optional manifest `service_type` onto the runtime.wasm wire enum. */
export function mapServiceType(
  serviceType: string | null | undefined,
): "stateless" | "subscribable" | "stateful" {
  if (serviceType === "subscribable" || serviceType === "stateful") {
    return serviceType;
  }
  return "stateless";
}
