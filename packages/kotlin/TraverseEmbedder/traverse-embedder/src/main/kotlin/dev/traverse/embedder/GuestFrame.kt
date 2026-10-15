package dev.traverse.embedder

import java.nio.ByteBuffer
import java.nio.ByteOrder

/**
 * Spec 138 guest-frame helpers (ONNX multi-IO bundles, #1661).
 * Hosts stage the concatenated bytes as opaque model input/output.
 */
object GuestFrame {
    /** Guest frame header version (FRAME_VERSION / MODEL_GUEST_ABI_VERSION). */
    const val VERSION: Int = 1

    /** Encode one little-endian Spec 138 guest frame. */
    fun encode(dtype: Int, dims: IntArray, payload: ByteArray): ByteArray {
        val out = ByteBuffer.allocate(4 + dims.size * 4 + 4 + payload.size)
            .order(ByteOrder.LITTLE_ENDIAN)
        out.putShort(VERSION.toShort())
        out.put(dtype.toByte())
        out.put(dims.size.toByte())
        for (dim in dims) {
            out.putInt(dim)
        }
        out.putInt(payload.size)
        out.put(payload)
        return out.array()
    }

    /** Concatenate frames into one staged input/output bundle. */
    fun concat(frames: List<ByteArray>): ByteArray {
        val total = frames.sumOf { it.size }
        val out = ByteArray(total)
        var offset = 0
        for (frame in frames) {
            System.arraycopy(frame, 0, out, offset, frame.size)
            offset += frame.size
        }
        return out
    }
}
