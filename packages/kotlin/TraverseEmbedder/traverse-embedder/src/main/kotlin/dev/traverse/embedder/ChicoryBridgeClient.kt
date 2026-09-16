package dev.traverse.embedder

import java.nio.charset.StandardCharsets

/** Serialized UTF-8 JSON client for the governed runtime-WASM bridge. */
class ChicoryBridgeClient(
    bridge: ChicoryRuntimeBridge,
    private val maximumOutputBytes: Int = DEFAULT_MAXIMUM_OUTPUT_BYTES,
) {
    private val instance = bridge.instance
    private val executionBudget = bridge.executionBudget
    private val memory = instance.memory()
    private val allocate = instance.export("traverse_alloc")
    private val deallocate = instance.export("traverse_dealloc")

    init {
        require(maximumOutputBytes > 0) { "maximum bridge output size must be positive" }
    }

    @Synchronized fun initialize(configJson: String): String =
        initialize(configJson.toByteArray(StandardCharsets.UTF_8))

    /**
     * `init` on a `runtime.wasm` orchestrator instance carries spec 1402
     * FR-011's length-prefixed binary framing (a 4-byte header length, that
     * many bytes of JSON metadata, then a raw nested-capability WASM
     * artifact) instead of bare UTF-8 JSON — spec 071 FR-005's amended text
     * (Decision 90) carves out exactly this one operation, so this overload
     * takes the raw bytes directly rather than a `String` that could not
     * carry them.
     */
    @Synchronized fun initialize(configBytes: ByteArray): String = executionBudget.run {
        invokeWithInput("traverse_init", configBytes)
    }

    @Synchronized fun submit(submissionJson: String): String = executionBudget.run {
        invokeWithInput("traverse_submit", submissionJson.toByteArray(StandardCharsets.UTF_8))
    }

    @Synchronized fun cancel(cancellationJson: String): String = executionBudget.run {
        invokeWithInput("traverse_cancel", cancellationJson.toByteArray(StandardCharsets.UTF_8))
    }

    @Synchronized fun compatibleStart(requestJson: String): String = executionBudget.run {
        invokeWithInput("traverse_compatible_start", requestJson.toByteArray(StandardCharsets.UTF_8))
    }

    @Synchronized fun compatibleStop(requestJson: String): String = executionBudget.run {
        invokeWithInput("traverse_compatible_stop", requestJson.toByteArray(StandardCharsets.UTF_8))
    }

    @Synchronized fun compatibleKill(requestJson: String): String = executionBudget.run {
        invokeWithInput("traverse_compatible_kill", requestJson.toByteArray(StandardCharsets.UTF_8))
    }

    @Synchronized fun nextEvent(): String? = executionBudget.run {
        val descriptor = allocate(DESCRIPTOR_BYTES)
        try {
            val status = instance.export("traverse_next_event").apply(descriptor.toLong()).single().toInt()
            if (status == STATUS_EMPTY) null else readResult(status, descriptor)
        } finally {
            executionBudget.cleanup { deallocate.apply(descriptor.toLong(), DESCRIPTOR_BYTES.toLong()) }
        }
    }

    @Synchronized fun shutdown(): String = executionBudget.run {
        val descriptor = allocate(DESCRIPTOR_BYTES)
        try {
            val status = instance.export("traverse_shutdown").apply(descriptor.toLong()).single().toInt()
            readResult(status, descriptor)
        } finally {
            executionBudget.cleanup { deallocate.apply(descriptor.toLong(), DESCRIPTOR_BYTES.toLong()) }
        }
    }

    private fun invokeWithInput(export: String, input: ByteArray): String {
        val inputPointer = allocate(input.size)
        val descriptor = allocate(DESCRIPTOR_BYTES)
        try {
            memory.write(inputPointer, input)
            val status = instance.export(export)
                .apply(inputPointer.toLong(), input.size.toLong(), descriptor.toLong())
                .single()
                .toInt()
            return readResult(status, descriptor)
        } finally {
            executionBudget.cleanup {
                deallocate.apply(descriptor.toLong(), DESCRIPTOR_BYTES.toLong())
                deallocate.apply(inputPointer.toLong(), input.size.toLong())
            }
        }
    }

    private fun allocate(length: Int): Int {
        val pointer = allocate.apply(length.toLong()).single().toInt()
        if (pointer < 0) throw TraverseBridgeException(STATUS_RESOURCE_LIMIT, "bridge allocation failed")
        return pointer
    }

    private fun readResult(status: Int, descriptor: Int): String {
        val pointer = memory.readInt(descriptor)
        val length = memory.readInt(descriptor + Int.SIZE_BYTES)
        if (pointer < 0 || length < 0 || length > maximumOutputBytes) {
            throw TraverseBridgeException(STATUS_INVALID_DESCRIPTOR, "bridge_invalid_descriptor")
        }
        val output = memory.readBytes(pointer, length).toString(StandardCharsets.UTF_8)
        if (status < 0) throw TraverseBridgeException(status, output)
        return output
    }

    companion object {
        const val DEFAULT_MAXIMUM_OUTPUT_BYTES = 1024 * 1024
        private const val DESCRIPTOR_BYTES = 8
        private const val STATUS_EMPTY = 0
        private const val STATUS_INVALID_DESCRIPTOR = -3
        private const val STATUS_RESOURCE_LIMIT = -4
    }
}

class TraverseBridgeException(val status: Int, message: String) : IllegalStateException(message)
