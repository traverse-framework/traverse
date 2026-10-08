/**
 * Spec 138 guest ABI v3 registration check (FR-053, Decision 110): a v3
 * module's only mutable state must be its single exported memory and its
 * exported numeric mutable globals, so a restored post-`model_prepare`
 * snapshot is indistinguishable from a freshly prepared instance. Mirrors
 * the native `wasmparser` check. JavaScript cannot see global mutability or
 * scan instructions, so this decodes the module binary itself and fails
 * closed on anything it does not recognise.
 */

export const MODEL_PREPARE_EXPORT = "model_prepare" as const;

/** Result of a successful check: exported mutable globals, sorted by name. */
export type GuestAbiV3Shape = { readonly mutableGlobals: readonly string[] };

const NUMERIC = new Set([0x7f, 0x7e, 0x7d, 0x7c]);
const VECTOR_OR_REF = new Set([0x7b, 0x70, 0x6f]);
// 0xFC sub-opcodes that mutate tables or segments: data.drop, table.init,
// elem.drop, table.copy, table.grow, table.fill.
const FORBIDDEN_FC = new Set([9, 12, 13, 14, 15, 17]);
const TABLE_SET = 0x26;

class Reader {
  offset = 0;
  constructor(
    private readonly bytes: Uint8Array,
    readonly end = bytes.length,
  ) {}

  byte(): number {
    const value = this.offset < this.end ? this.bytes[this.offset] : undefined;
    if (value === undefined) throw new Error("unexpected end");
    this.offset += 1;
    return value;
  }

  skip(count: number): void {
    if (this.offset + count > this.end) throw new Error("unexpected end");
    this.offset += count;
  }

  /** Unsigned or signed LEB128 of at most `bits` bits; returns the unsigned low 32 bits. */
  leb(bits: number): number {
    let result = 0;
    let shift = 0;
    for (;;) {
      const byte = this.byte();
      if (shift < 32) result |= (byte & 0x7f) << shift;
      shift += 7;
      if ((byte & 0x80) === 0) return result >>> 0;
      if (shift >= bits + 7) throw new Error("leb too long");
    }
  }

  u32(): number {
    return this.leb(32);
  }

  name(): string {
    const length = this.u32();
    const start = this.offset;
    this.skip(length);
    return new TextDecoder("utf-8", { fatal: true }).decode(this.bytes.subarray(start, start + length));
  }
}

/** Value type; returns its first byte. Typed references carry a heap type. */
function valType(reader: Reader): number {
  const byte = reader.byte();
  if (NUMERIC.has(byte) || VECTOR_OR_REF.has(byte)) return byte;
  if (byte === 0x63 || byte === 0x64) {
    reader.leb(33);
    return byte;
  }
  throw new Error("unsupported value type");
}

function heapType(reader: Reader): void {
  reader.leb(33);
}

function blockType(reader: Reader): void {
  const peek = reader.byte();
  if (peek === 0x40 || NUMERIC.has(peek) || VECTOR_OR_REF.has(peek)) return;
  if (peek === 0x63 || peek === 0x64) return heapType(reader);
  reader.offset -= 1;
  reader.leb(33);
}

function memarg(reader: Reader): void {
  const align = reader.u32();
  if (align & 0x40) reader.u32();
  reader.leb(64);
}

/** Decode one instruction's immediates; throws on forbidden or unknown opcodes. */
function instruction(reader: Reader, opcode: number): void {
  if (opcode === TABLE_SET) throw new Forbidden();
  if (opcode <= 0x01 || opcode === 0x05 || opcode === 0x0b || opcode === 0x0f || opcode === 0x1a || opcode === 0x1b) {
    return;
  }
  if (opcode >= 0x02 && opcode <= 0x04) return blockType(reader);
  if (opcode === 0x0c || opcode === 0x0d || opcode === 0x10 || opcode === 0x12 || opcode === 0x14 || opcode === 0x15) {
    reader.u32();
    return;
  }
  if (opcode === 0x0e) {
    for (let count = reader.u32(); count > 0; count--) reader.u32();
    reader.u32();
    return;
  }
  if (opcode === 0x11 || opcode === 0x13) {
    reader.u32();
    reader.u32();
    return;
  }
  if (opcode === 0x1c) {
    for (let count = reader.u32(); count > 0; count--) valType(reader);
    return;
  }
  if (opcode >= 0x20 && opcode <= 0x25) {
    reader.u32();
    return;
  }
  if (opcode >= 0x28 && opcode <= 0x3e) return memarg(reader);
  if (opcode === 0x3f || opcode === 0x40) {
    reader.u32();
    return;
  }
  if (opcode === 0x41) return void reader.leb(32);
  if (opcode === 0x42) return void reader.leb(64);
  if (opcode === 0x43) return reader.skip(4);
  if (opcode === 0x44) return reader.skip(8);
  if (opcode >= 0x45 && opcode <= 0xc4) return;
  if (opcode === 0xd0) return heapType(reader);
  if (opcode === 0xd1 || opcode === 0xd3 || opcode === 0xd4) return;
  if (opcode === 0xd2 || opcode === 0xd5 || opcode === 0xd6) {
    reader.u32();
    return;
  }
  if (opcode === 0xfc) return bulk(reader, reader.u32());
  if (opcode === 0xfd) return simd(reader, reader.u32());
  throw new Error(`unsupported opcode 0x${opcode.toString(16)}`);
}

class Forbidden extends Error {}

function bulk(reader: Reader, sub: number): void {
  if (FORBIDDEN_FC.has(sub)) throw new Forbidden();
  if (sub <= 7) return;
  if (sub === 8 || sub === 10) {
    reader.u32();
    reader.u32();
    return;
  }
  if (sub === 11 || sub === 16) {
    reader.u32();
    return;
  }
  throw new Error("unsupported 0xfc opcode");
}

function simd(reader: Reader, sub: number): void {
  if (sub <= 11 || sub === 92 || sub === 93) return memarg(reader);
  if (sub === 12 || sub === 13) return reader.skip(16);
  if (sub >= 21 && sub <= 34) return reader.skip(1);
  if (sub >= 84 && sub <= 91) {
    memarg(reader);
    reader.skip(1);
    return;
  }
  if (sub <= 0x113) return;
  throw new Error("unsupported 0xfd opcode");
}

/** Walk instructions until the `end` that closes the expression or body. */
function expression(reader: Reader): void {
  let depth = 1;
  while (depth > 0) {
    const opcode = reader.byte();
    if (opcode === 0x0b) depth -= 1;
    else if (opcode >= 0x02 && opcode <= 0x04) depth += 1;
    instruction(reader, opcode);
  }
}

/**
 * FR-053 check. Returns the module's exported mutable globals, or the
 * failure message (the host maps it to `model_incompatible`).
 */
export function checkGuestAbiV3(wasm: Uint8Array): GuestAbiV3Shape | string {
  try {
    return scan(wasm);
  } catch (error) {
    return error instanceof Forbidden
      ? "abi_version 3 model wasm uses a table- or segment-mutating instruction"
      : "abi_version 3 model wasm failed validation";
  }
}

function scan(wasm: Uint8Array): GuestAbiV3Shape | string {
  const reader = new Reader(wasm);
  const header = [0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00];
  for (const expected of header) {
    if (reader.byte() !== expected) throw new Error("bad header");
  }
  const mutableGlobals = new Map<number, boolean>();
  const exportedGlobals = new Map<number, string>();
  let globalIndex = 0;
  let memories = 0;
  let exportedMemories = 0;
  let prepare = false;
  while (reader.offset < wasm.length) {
    const id = reader.byte();
    const size = reader.u32();
    const end = reader.offset + size;
    if (end > wasm.length) throw new Error("section past end");
    const section = new Reader(wasm, end);
    section.offset = reader.offset;
    if (id === 2 && section.u32() > 0) return "abi_version 3 model wasm must not import anything";
    if (id === 5) memories += section.u32();
    if (id === 6) {
      for (let count = section.u32(); count > 0; count--) {
        const type = valType(section);
        if (section.byte() === 1) mutableGlobals.set(globalIndex, NUMERIC.has(type));
        expression(section);
        globalIndex += 1;
      }
    }
    if (id === 7) {
      for (let count = section.u32(); count > 0; count--) {
        const name = section.name();
        const kind = section.byte();
        const index = section.u32();
        if (kind === 0x02) exportedMemories += 1;
        if (kind === 0x03) exportedGlobals.set(index, name);
        if (kind === 0x00 && name === MODEL_PREPARE_EXPORT) prepare = true;
      }
    }
    if (id === 10) {
      for (let count = section.u32(); count > 0; count--) {
        const size = section.u32();
        const body = new Reader(wasm, section.offset + size);
        body.offset = section.offset;
        for (let locals = body.u32(); locals > 0; locals--) {
          body.u32();
          valType(body);
        }
        expression(body);
        section.offset = body.end;
      }
    }
    reader.offset = end;
  }
  if (memories !== 1 || exportedMemories !== 1) {
    return "abi_version 3 model wasm must define and export exactly one memory";
  }
  const names: string[] = [];
  for (const [index, numeric] of mutableGlobals) {
    const name = exportedGlobals.get(index);
    if (!numeric || name === undefined) {
      return "abi_version 3 model wasm has a non-exported or non-numeric mutable global";
    }
    names.push(name);
  }
  if (!prepare) return "abi_version 3 model wasm missing model_prepare export";
  return { mutableGlobals: names.sort() };
}
