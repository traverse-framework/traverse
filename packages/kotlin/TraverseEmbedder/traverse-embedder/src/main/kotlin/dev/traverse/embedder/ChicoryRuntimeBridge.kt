package dev.traverse.embedder

import com.dylibso.chicory.runtime.Instance
import com.dylibso.chicory.runtime.ExecutionListener
import com.dylibso.chicory.wasm.Parser
import com.dylibso.chicory.wasm.types.ExternalType
import com.dylibso.chicory.wasm.types.FunctionType
import com.dylibso.chicory.wasm.types.ValType
import java.io.File
import java.security.MessageDigest

/** Fail-closed Chicory loader for `runtime-wasm-bridge/1.1.0`. */
class ChicoryRuntimeBridge(
    bundle: TraverseBundle,
    maximumArtifactBytes: Int = DEFAULT_MAXIMUM_ARTIFACT_BYTES,
    maximumMemoryPages: Int = DEFAULT_MAXIMUM_MEMORY_PAGES,
    maximumInstructionsPerCall: Long = DEFAULT_MAXIMUM_INSTRUCTIONS_PER_CALL,
    maximumCallDurationMillis: Long = DEFAULT_MAXIMUM_CALL_DURATION_MILLIS,
) {
    val runtimeFile: File
    val runtimeWasmDigest: String

    internal val instance: Instance
    internal val executionBudget = ChicoryExecutionBudget(
        maximumInstructionsPerCall,
        maximumCallDurationMillis,
    )

    init {
        require(maximumArtifactBytes > 0) { "maximum runtime WASM size must be positive" }
        require(maximumMemoryPages > 0) { "maximum runtime memory pages must be positive" }

        runtimeFile = File(bundle.rootPath, "runtime/runtime.wasm")
        if (!runtimeFile.isFile) {
            throw TraverseBundleException("runtime/runtime.wasm is unavailable")
        }
        if (runtimeFile.length() > maximumArtifactBytes) {
            throw TraverseBundleException("runtime/runtime.wasm exceeds the configured size limit")
        }

        val bytes = runtimeFile.readBytes()
        runtimeWasmDigest = "sha256:" + MessageDigest.getInstance("SHA-256")
            .digest(bytes)
            .joinToString("") { "%02x".format(it) }
        if (normalizeDigest(bundle.runtimeWasmDigest) != runtimeWasmDigest) {
            throw TraverseBundleException("bundle_digest_mismatch")
        }

        val module = try {
            Parser.parse(bytes)
        } catch (error: RuntimeException) {
            throw TraverseBundleException("runtime/runtime.wasm is not a valid core WebAssembly module", error)
        }
        if (module.importSection().importCount() != 0) {
            throw TraverseBundleException("runtime/runtime.wasm requires undeclared ambient imports")
        }

        val memoryExports = (0 until module.exportSection().exportCount())
            .map { module.exportSection().getExport(it) }
            .filter { it.exportType() == ExternalType.MEMORY }
        if (memoryExports.size != 1 || memoryExports.single().name() != "memory") {
            throw TraverseBundleException("runtime/runtime.wasm must export exactly one bridge memory")
        }
        val memory = module.memorySection().orElseThrow {
            TraverseBundleException("runtime/runtime.wasm must declare bridge memory")
        }.getMemory(0)
        if (memory.limits().maximumPages() > maximumMemoryPages) {
            throw TraverseBundleException("runtime/runtime.wasm exceeds the configured memory limit")
        }

        instance = try {
            Instance.builder(module)
                .withUnsafeExecutionListener(executionBudget.listener)
                .build()
        } catch (error: RuntimeException) {
            throw TraverseBundleException("runtime/runtime.wasm could not be instantiated", error)
        }
        REQUIRED_FUNCTIONS.forEach { (name, expectedType) ->
            val actualType = try {
                instance.exportType(name)
            } catch (error: RuntimeException) {
                throw TraverseBundleException("runtime/runtime.wasm is missing required export $name", error)
            }
            if (actualType != expectedType) {
                throw TraverseBundleException("runtime/runtime.wasm has an invalid signature for $name")
            }
        }

        val version = try {
            executionBudget.run { instance.export("traverse_bridge_abi_version").apply() }
        } catch (error: RuntimeException) {
            throw TraverseBundleException("bridge_version_mismatch", error)
        }
        if (version.size != 1 || version[0] != ABI_VERSION.toLong()) {
            throw TraverseBundleException("bridge_version_mismatch")
        }
    }

    companion object {
        const val ABI_VERSION = 10_100
        const val DEFAULT_MAXIMUM_ARTIFACT_BYTES = 32 * 1024 * 1024
        const val DEFAULT_MAXIMUM_MEMORY_PAGES = 512
        const val DEFAULT_MAXIMUM_INSTRUCTIONS_PER_CALL = 10_000_000L
        const val DEFAULT_MAXIMUM_CALL_DURATION_MILLIS = 30_000L

        private val I32_TO_I32 = FunctionType.of(listOf(ValType.I32), listOf(ValType.I32))
        private val THREE_I32_TO_I32 = FunctionType.of(
            listOf(ValType.I32, ValType.I32, ValType.I32),
            listOf(ValType.I32),
        )
        private val REQUIRED_FUNCTIONS = mapOf(
            "traverse_bridge_abi_version" to FunctionType.returning(ValType.I32),
            "traverse_alloc" to I32_TO_I32,
            "traverse_dealloc" to FunctionType.of(listOf(ValType.I32, ValType.I32), emptyList()),
            "traverse_init" to THREE_I32_TO_I32,
            "traverse_submit" to THREE_I32_TO_I32,
            "traverse_next_event" to I32_TO_I32,
            "traverse_cancel" to THREE_I32_TO_I32,
            "traverse_shutdown" to I32_TO_I32,
            "traverse_compatible_start" to THREE_I32_TO_I32,
            "traverse_compatible_stop" to THREE_I32_TO_I32,
            "traverse_compatible_kill" to THREE_I32_TO_I32,
        )

        private fun normalizeDigest(digest: String): String {
            val normalized = digest.trim().lowercase()
            return if (normalized.startsWith("sha256:")) normalized else "sha256:$normalized"
        }
    }
}

internal class ChicoryExecutionBudget(
    private val maximumInstructions: Long,
    maximumDurationMillis: Long,
) {
    private val maximumDurationNanos: Long
    private var active = false
    private var instructions = 0L
    private var deadlineNanos = 0L

    init {
        require(maximumInstructions > 0) { "maximum instructions per call must be positive" }
        require(maximumDurationMillis > 0 && maximumDurationMillis <= Long.MAX_VALUE / 1_000_000L) {
            "maximum call duration must be positive and representable"
        }
        maximumDurationNanos = maximumDurationMillis * 1_000_000L
    }

    val listener = ExecutionListener { _, _ ->
        if (active) {
            instructions += 1
            if (instructions > maximumInstructions || System.nanoTime() - deadlineNanos >= 0) {
                throw TraverseBridgeException(-4, "bridge_resource_limit")
            }
        }
    }

    fun <T> run(block: () -> T): T {
        check(!active) { "bridge execution budget is already active" }
        instructions = 0
        deadlineNanos = System.nanoTime() + maximumDurationNanos
        active = true
        return try {
            block()
        } finally {
            active = false
        }
    }

    fun <T> cleanup(block: () -> T): T {
        val wasActive = active
        active = false
        return try {
            block()
        } finally {
            active = wasActive
        }
    }
}

class TraverseBundleException(message: String, cause: Throwable? = null) :
    IllegalArgumentException(message, cause)
