/**
 * Reads the first declared linear-memory initial size from a WASM binary
 * (#1656 fail-fast). Returns `null` when the module has no memory section.
 */

const WASM_MAGIC = [0x00, 0x61, 0x73, 0x6d];
const MEMORY_SECTION_ID = 5;
const PAGE_BYTES = 64 * 1024;

function readLeb128(bytes: Uint8Array, offset: number): { value: number; next: number } | null {
  let result = 0;
  let shift = 0;
  let index = offset;
  while (index < bytes.length) {
    const byte = bytes[index] ?? 0;
    index += 1;
    result |= (byte & 0x7f) << shift;
    if ((byte & 0x80) === 0) {
      return { value: result >>> 0, next: index };
    }
    shift += 7;
    if (shift > 35) {
      return null;
    }
  }
  return null;
}

/**
 * Returns initial memory bytes for the first memory in the module, or `null`
 * when the binary is not WASM or declares no memories.
 */
export function wasmInitialMemoryBytes(bytes: Uint8Array): number | null {
  if (bytes.length < 8) {
    return null;
  }
  for (let i = 0; i < 4; i += 1) {
    if (bytes[i] !== WASM_MAGIC[i]) {
      return null;
    }
  }
  let offset = 8;
  while (offset < bytes.length) {
    const sectionId = bytes[offset];
    offset += 1;
    const size = readLeb128(bytes, offset);
    if (size === null) {
      return null;
    }
    offset = size.next;
    const sectionEnd = offset + size.value;
    if (sectionEnd > bytes.length) {
      return null;
    }
    if (sectionId === MEMORY_SECTION_ID) {
      const count = readLeb128(bytes, offset);
      if (count === null || count.value === 0) {
        return null;
      }
      let cursor = count.next;
      const flags = bytes[cursor];
      cursor += 1;
      if (flags === undefined) {
        return null;
      }
      const minPages = readLeb128(bytes, cursor);
      if (minPages === null) {
        return null;
      }
      return minPages.value * PAGE_BYTES;
    }
    offset = sectionEnd;
  }
  return null;
}
