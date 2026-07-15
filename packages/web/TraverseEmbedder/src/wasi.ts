/**
 * Minimal deterministic WASI `preview1` shim covering exactly the
 * `wasi_snapshot_preview1` surface in the Traverse Host ABI whitelist
 * (`fd_read`, `fd_write`, `proc_exit`; see `hostAbi.ts`). No filesystem, no
 * network, no environment, no clocks, no randomness — deny-by-default,
 * mirroring the native `WasmExecutor`'s `WasiCtxBuilder` configuration
 * (stdin = input JSON bytes, stdout = captured buffer).
 *
 * Any capability module that requires a broader WASI surface (args,
 * environ, clocks, filesystem) is already outside the Traverse Host ABI
 * whitelist and would be rejected by the native runtime's own import
 * validation before this shim would ever see it (spec `064`).
 */

const STDIN_FD = 0;
const STDOUT_FD = 1;
const STDERR_FD = 2;
const WASI_ERRNO_SUCCESS = 0;
const WASI_ERRNO_BADF = 8;

/** Thrown by `proc_exit`; unwinds the synchronous WASM call. */
export class WasiExit extends Error {
  readonly code: number;

  constructor(code: number) {
    super(`proc_exit(${code})`);
    this.name = "WasiExit";
    this.code = code;
  }
}

/** Mutable memory handle, populated once the WASM instance is created. */
export interface WasiMemoryRef {
  memory: WebAssembly.Memory | null;
}

/** Captured stdio pipes for one execution. */
export class WasiPipes {
  private readonly stdin: Uint8Array;
  private stdinOffset = 0;
  private readonly stdoutChunks: number[] = [];

  constructor(stdinBytes: Uint8Array) {
    this.stdin = stdinBytes;
  }

  readStdin(maxLength: number): Uint8Array {
    const remaining = this.stdin.length - this.stdinOffset;
    const length = Math.min(maxLength, Math.max(remaining, 0));
    const chunk = this.stdin.subarray(this.stdinOffset, this.stdinOffset + length);
    this.stdinOffset += length;
    return chunk;
  }

  writeStdout(bytes: Uint8Array): void {
    for (const byte of bytes) {
      this.stdoutChunks.push(byte);
    }
  }

  stdoutBytes(): Uint8Array {
    return Uint8Array.from(this.stdoutChunks);
  }
}

function memoryView(memoryRef: WasiMemoryRef): DataView {
  if (memoryRef.memory === null) {
    throw new Error("wasi: module does not export linear memory");
  }
  return new DataView(memoryRef.memory.buffer);
}

function memoryBytes(memoryRef: WasiMemoryRef, ptr: number, length: number): Uint8Array {
  if (memoryRef.memory === null) {
    throw new Error("wasi: module does not export linear memory");
  }
  return new Uint8Array(memoryRef.memory.buffer, ptr, length);
}

/**
 * Builds the `wasi_snapshot_preview1` import object for one execution.
 * `memoryRef.memory` must be set to the instantiated module's exported
 * memory before any of these functions are invoked.
 */
export function createWasiPreview1Imports(
  pipes: WasiPipes,
  memoryRef: WasiMemoryRef,
): WebAssembly.ModuleImports {
  return {
    fd_read(fd: number, iovsPtr: number, iovsLen: number, nreadPtr: number): number {
      if (fd !== STDIN_FD) {
        return WASI_ERRNO_BADF;
      }
      const view = memoryView(memoryRef);
      let totalRead = 0;
      for (let index = 0; index < iovsLen; index += 1) {
        const base = iovsPtr + index * 8;
        const bufPtr = view.getUint32(base, true);
        const bufLen = view.getUint32(base + 4, true);
        const chunk = pipes.readStdin(bufLen);
        if (chunk.length > 0) {
          memoryBytes(memoryRef, bufPtr, chunk.length).set(chunk);
        }
        totalRead += chunk.length;
        if (chunk.length < bufLen) {
          break;
        }
      }
      view.setUint32(nreadPtr, totalRead, true);
      return WASI_ERRNO_SUCCESS;
    },
    fd_write(fd: number, iovsPtr: number, iovsLen: number, nwrittenPtr: number): number {
      if (fd !== STDOUT_FD && fd !== STDERR_FD) {
        return WASI_ERRNO_BADF;
      }
      const view = memoryView(memoryRef);
      let totalWritten = 0;
      for (let index = 0; index < iovsLen; index += 1) {
        const base = iovsPtr + index * 8;
        const bufPtr = view.getUint32(base, true);
        const bufLen = view.getUint32(base + 4, true);
        if (fd === STDOUT_FD) {
          pipes.writeStdout(memoryBytes(memoryRef, bufPtr, bufLen));
        }
        totalWritten += bufLen;
      }
      view.setUint32(nwrittenPtr, totalWritten, true);
      return WASI_ERRNO_SUCCESS;
    },
    proc_exit(code: number): never {
      throw new WasiExit(code);
    },
  };
}
