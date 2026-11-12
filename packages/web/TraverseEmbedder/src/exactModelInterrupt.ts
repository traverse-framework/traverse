/**
 * Mid-run interruption for Spec 138 browser/Node exact-model guests
 * (Decision 104 / `#1582`): run prepare/execute in a Worker (or
 * `worker_threads` Worker) so cancel and deadline can terminate the guest.
 */

/** Failure raised by the interruptible guest runner (mapped to ExactModelError). */
export class GuestWorkerError extends Error {
  readonly code: string;
  readonly reason: string | undefined;
  constructor(code: string, message: string, reason?: string) {
    super(message);
    this.name = "GuestWorkerError";
    this.code = code;
    this.reason = reason;
  }
}

export type GuestSnapshot = {
  readonly memory: Uint8Array;
  readonly globals: readonly (readonly [string, number | bigint])[];
};

export type GuestExecuteRequest = {
  readonly kind: "execute";
  readonly wasm: Uint8Array;
  readonly input: Uint8Array;
  readonly maxOutputBytes: number;
  readonly maxMemoryBytes: number;
  readonly abiVersion: number;
  readonly snapshot?: GuestSnapshot;
  readonly snapshotGlobalNames?: readonly string[];
};

export type GuestPrepareRequest = {
  readonly kind: "prepare";
  readonly wasm: Uint8Array;
  readonly maxMemoryBytes: number;
  readonly snapshotGlobalNames: readonly string[];
};

export type GuestRequest = GuestExecuteRequest | GuestPrepareRequest;

export type GuestExecuteSuccess = {
  readonly kind: "execute";
  readonly output: Uint8Array;
  readonly snapshot?: GuestSnapshot;
};

export type GuestPrepareSuccess = {
  readonly kind: "prepare";
  readonly snapshot: GuestSnapshot;
};

export type GuestSuccess = GuestExecuteSuccess | GuestPrepareSuccess;

type WireError = {
  readonly ok: false;
  readonly code: string;
  readonly message: string;
  readonly reason?: string;
};

type WireSuccess = {
  readonly ok: true;
  readonly kind: "execute" | "prepare";
  readonly output?: ArrayBuffer;
  readonly snapshot?: { memory: ArrayBuffer; globals: [string, number | bigint][] };
};

/** Shared guest body: no host imports, used by browser and Node workers. */
const GUEST_BODY = String.raw`
const MODEL_EXECUTE_EXPORT = "model_execute";
const MODEL_ALLOC_EXPORT = "model_alloc";
const MODEL_PREPARE_EXPORT = "model_prepare";
const GUEST_REJECT_GENERIC = -1;
const GUEST_REJECT_FRAME_MISMATCH = -2;
const GUEST_REJECT_INFERENCE_FAILED = -3;
const GUEST_REJECT_OUTPUT_MISMATCH = -4;

function fail(code, message, reason) {
  return { ok: false, code, message, reason };
}

function guestReject(code) {
  if (code === GUEST_REJECT_FRAME_MISMATCH) {
    return fail("invalid_input", "model rejected the input frame: it does not match the package's tensor contract", "frame_mismatch");
  }
  if (code === GUEST_REJECT_INFERENCE_FAILED) {
    return fail("execution_failed", "model inference failed inside the guest", "inference_failed");
  }
  if (code === GUEST_REJECT_OUTPUT_MISMATCH) {
    return fail("model_incompatible", "model output does not match the package's declared output shape", "output_mismatch");
  }
  return fail("execution_failed", "model rejected the call", "guest_rejected");
}

function guestExecuteOutcome(outLen, maxOutputBytes) {
  if (!Number.isInteger(outLen) || outLen < 0) return guestReject(outLen);
  if (outLen > maxOutputBytes) {
    return fail("resource_exhausted", "model output exceeds the output ceiling", "output_limit_exceeded");
  }
  return outLen;
}

function placeV1(memory, inLen, outCap, maxMemoryBytes) {
  const inPtr = 64;
  const outPtr = inPtr + inLen + 64;
  const needed = outPtr + outCap;
  if (needed > maxMemoryBytes) return fail("resource_exhausted", "model memory ceiling exceeded");
  while (memory.buffer.byteLength < needed) memory.grow(1);
  return [inPtr, outPtr];
}

function placeV2(instance, memory, inLen, outCap, maxMemoryBytes) {
  const alloc = instance.exports[MODEL_ALLOC_EXPORT];
  if (typeof alloc !== "function") {
    return fail("model_incompatible", "abi_version 2 model wasm missing model_alloc export");
  }
  let inPtr;
  let outPtr;
  try {
    inPtr = Number(alloc(inLen));
    outPtr = Number(alloc(outCap));
  } catch {
    return fail("execution_failed", "model_alloc trapped");
  }
  if (memory.buffer.byteLength > maxMemoryBytes) {
    return fail("resource_exhausted", "model memory ceiling exceeded");
  }
  const size = memory.buffer.byteLength;
  const inEnd = inPtr + inLen;
  const outEnd = outPtr + outCap;
  if (!(inPtr > 0 && outPtr > 0 && inEnd <= size && outEnd <= size) || (inPtr < outEnd && outPtr < inEnd)) {
    return fail("execution_failed", "model_alloc returned an invalid region");
  }
  return [inPtr, outPtr];
}

function restoreOrPrepare(instance, memory, maxMemoryBytes, snapshot, snapshotGlobalNames) {
  const names = snapshotGlobalNames || [];
  if (snapshot) {
    const missing = Math.ceil((snapshot.memory.byteLength - memory.buffer.byteLength) / 65536);
    if (missing > 0) memory.grow(missing);
    new Uint8Array(memory.buffer).set(new Uint8Array(snapshot.memory));
    for (const [name, value] of snapshot.globals) {
      instance.exports[name].value = value;
    }
    return { ok: true };
  }
  const prepare = instance.exports[MODEL_PREPARE_EXPORT];
  if (typeof prepare !== "function") {
    return fail("model_incompatible", "abi_version 3 model wasm missing model_prepare export");
  }
  let prepared;
  try {
    prepared = prepare();
  } catch {
    return fail("execution_failed", "model_prepare trapped");
  }
  if (prepared !== 0) return fail("execution_failed", "model_prepare returned non-zero");
  if (memory.buffer.byteLength > maxMemoryBytes) {
    return fail("resource_exhausted", "model memory ceiling exceeded");
  }
  return {
    ok: true,
    snapshot: {
      memory: memory.buffer.slice(0),
      globals: names.map((name) => [name, instance.exports[name].value]),
    },
  };
}

async function runJob(job) {
  let module;
  try {
    module = await WebAssembly.compile(job.wasm);
  } catch {
    return fail("model_incompatible", "model wasm failed validation");
  }
  const instance = await WebAssembly.instantiate(module, {});
  const memory = instance.exports.memory;
  if (!(memory instanceof WebAssembly.Memory)) {
    return fail("model_incompatible", "model wasm missing memory or model_execute");
  }
  if (job.kind === "prepare") {
    const prepared = restoreOrPrepare(instance, memory, job.maxMemoryBytes, undefined, job.snapshotGlobalNames);
    if (!prepared.ok) return prepared;
    return { ok: true, kind: "prepare", snapshot: prepared.snapshot };
  }
  const execute = instance.exports[MODEL_EXECUTE_EXPORT];
  if (typeof execute !== "function") {
    return fail("model_incompatible", "model wasm missing memory or model_execute");
  }
  let freshSnapshot;
  if (job.abiVersion >= 3) {
    const prepared = restoreOrPrepare(
      instance,
      memory,
      job.maxMemoryBytes,
      job.snapshot,
      job.snapshotGlobalNames || [],
    );
    if (!prepared.ok) return prepared;
    if (prepared.snapshot) freshSnapshot = prepared.snapshot;
  }
  const placed = job.abiVersion >= 2
    ? placeV2(instance, memory, job.input.byteLength, job.maxOutputBytes, job.maxMemoryBytes)
    : placeV1(memory, job.input.byteLength, job.maxOutputBytes, job.maxMemoryBytes);
  if (!Array.isArray(placed)) return placed;
  const [inPtr, outPtr] = placed;
  new Uint8Array(memory.buffer, inPtr, job.input.byteLength).set(new Uint8Array(job.input));
  const outLen = Number(execute(inPtr, job.input.byteLength, outPtr, job.maxOutputBytes));
  if (memory.buffer.byteLength > job.maxMemoryBytes) {
    return fail("resource_exhausted", "model memory ceiling exceeded");
  }
  const length = guestExecuteOutcome(outLen, job.maxOutputBytes);
  if (typeof length !== "number") return length;
  const output = memory.buffer.slice(outPtr, outPtr + length);
  return { ok: true, kind: "execute", output, snapshot: freshSnapshot };
}
`;

const BROWSER_WORKER_SOURCE = `${GUEST_BODY}
self.onmessage = async (event) => {
  try {
    const result = await runJob(event.data);
    const transfer = [];
    if (result.ok && result.output) transfer.push(result.output);
    if (result.ok && result.snapshot) transfer.push(result.snapshot.memory);
    self.postMessage(result, transfer);
  } catch (error) {
    self.postMessage({
      ok: false,
      code: "execution_failed",
      message: error && error.message ? String(error.message) : "guest worker failed",
    });
  }
};
`;

const NODE_WORKER_SOURCE = `${GUEST_BODY}
const { parentPort } = require("worker_threads");
parentPort.on("message", async (job) => {
  try {
    const result = await runJob(job);
    const transfer = [];
    if (result.ok && result.output) transfer.push(result.output);
    if (result.ok && result.snapshot) transfer.push(result.snapshot.memory);
    parentPort.postMessage(result, transfer);
  } catch (error) {
    parentPort.postMessage({
      ok: false,
      code: "execution_failed",
      message: error && error.message ? String(error.message) : "guest worker failed",
    });
  }
});
`;

function toWire(request: GuestRequest): Record<string, unknown> {
  if (request.kind === "prepare") {
    return {
      kind: "prepare",
      wasm: request.wasm.buffer.slice(
        request.wasm.byteOffset,
        request.wasm.byteOffset + request.wasm.byteLength,
      ),
      maxMemoryBytes: request.maxMemoryBytes,
      snapshotGlobalNames: [...request.snapshotGlobalNames],
    };
  }
  const wire: Record<string, unknown> = {
    kind: "execute",
    wasm: request.wasm.buffer.slice(request.wasm.byteOffset, request.wasm.byteOffset + request.wasm.byteLength),
    input: request.input.buffer.slice(request.input.byteOffset, request.input.byteOffset + request.input.byteLength),
    maxOutputBytes: request.maxOutputBytes,
    maxMemoryBytes: request.maxMemoryBytes,
    abiVersion: request.abiVersion,
  };
  if (request.snapshot) {
    wire.snapshot = {
      memory: request.snapshot.memory.buffer.slice(
        request.snapshot.memory.byteOffset,
        request.snapshot.memory.byteOffset + request.snapshot.memory.byteLength,
      ),
      globals: request.snapshot.globals.map(([name, value]) => [name, value]),
    };
  }
  if (request.snapshotGlobalNames) {
    wire.snapshotGlobalNames = [...request.snapshotGlobalNames];
  }
  return wire;
}

function fromWire(result: WireSuccess | WireError): GuestSuccess {
  if (!result.ok) {
    throw new GuestWorkerError(result.code, result.message, result.reason);
  }
  const snapshot = result.snapshot
    ? {
        memory: new Uint8Array(result.snapshot.memory),
        globals: result.snapshot.globals.map(
          ([name, value]) => [name, value] as const,
        ),
      }
    : undefined;
  if (result.kind === "prepare") {
    if (!snapshot) {
      throw new GuestWorkerError("execution_failed", "model_prepare returned no snapshot");
    }
    return { kind: "prepare", snapshot };
  }
  if (!result.output) {
    throw new GuestWorkerError("execution_failed", "model_execute returned no output");
  }
  return {
    kind: "execute",
    output: new Uint8Array(result.output),
    ...(snapshot ? { snapshot } : {}),
  };
}

/**
 * Run one guest job in a Worker. AbortSignal or an expired deadline terminates
 * the worker and surfaces `cancelled` / `timeout`.
 */
export async function runGuestInterruptible(
  request: GuestRequest,
  options: { readonly signal?: AbortSignal; readonly deadlineMs: number },
): Promise<GuestSuccess> {
  if (options.signal?.aborted) {
    throw new GuestWorkerError("cancelled", "model guest cancelled before invoke");
  }
  const remaining = options.deadlineMs - Date.now();
  if (remaining <= 0) {
    throw new GuestWorkerError("timeout", "model guest exceeded timeout before invoke");
  }

  const useBrowserWorker = typeof globalThis.Worker === "function";
  return useBrowserWorker
    ? runBrowser(request, options.signal, remaining)
    : runNode(request, options.signal, remaining);
}

async function runBrowser(
  request: GuestRequest,
  signal: AbortSignal | undefined,
  remainingMs: number,
): Promise<GuestSuccess> {
  const blob = new Blob([BROWSER_WORKER_SOURCE], { type: "text/javascript" });
  const url = URL.createObjectURL(blob);
  const worker = new Worker(url);
  try {
    return await awaitWorker(
      {
        post: (job) => worker.postMessage(job),
        onMessage: (handler) => {
          worker.onmessage = (event) => handler(event.data as WireSuccess | WireError);
        },
        onError: (handler) => {
          worker.onerror = (event) => handler(event.message);
        },
        terminate: () => worker.terminate(),
      },
      request,
      signal,
      remainingMs,
    );
  } finally {
    URL.revokeObjectURL(url);
  }
}

async function runNode(
  request: GuestRequest,
  signal: AbortSignal | undefined,
  remainingMs: number,
): Promise<GuestSuccess> {
  const { Worker } = await import("node:worker_threads");
  const worker = new Worker(NODE_WORKER_SOURCE, { eval: true });
  return awaitWorker(
    {
      post: (job) => worker.postMessage(job),
      onMessage: (handler) => {
        worker.on("message", handler);
      },
      onError: (handler) => {
        worker.on("error", (error) => handler(error.message));
      },
      terminate: () => {
        void worker.terminate();
      },
    },
    request,
    signal,
    remainingMs,
  );
}

type WorkerPort = {
  post(job: Record<string, unknown>): void;
  onMessage(handler: (data: WireSuccess | WireError) => void): void;
  onError(handler: (message: string) => void): void;
  terminate(): void;
};

function awaitWorker(
  port: WorkerPort,
  request: GuestRequest,
  signal: AbortSignal | undefined,
  remainingMs: number,
): Promise<GuestSuccess> {
  return new Promise((resolve, reject) => {
    let settled = false;
    const finish = (fn: () => void) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      signal?.removeEventListener("abort", onAbort);
      port.terminate();
      fn();
    };
    const onAbort = () => {
      finish(() => reject(new GuestWorkerError("cancelled", "model guest cancelled mid-run")));
    };
    const timer = setTimeout(() => {
      finish(() => reject(new GuestWorkerError("timeout", "model guest exceeded timeout mid-run")));
    }, remainingMs);
    port.onMessage((data) => {
      finish(() => {
        try {
          resolve(fromWire(data));
        } catch (error) {
          reject(error);
        }
      });
    });
    port.onError((message) => {
      finish(() =>
        reject(new GuestWorkerError("execution_failed", message || "guest worker failed")),
      );
    });
    signal?.addEventListener("abort", onAbort, { once: true });
    if (signal?.aborted) {
      onAbort();
      return;
    }
    port.post(toWire(request));
  });
}
