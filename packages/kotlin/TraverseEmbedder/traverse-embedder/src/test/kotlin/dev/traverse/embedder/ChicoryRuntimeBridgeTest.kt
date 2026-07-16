package dev.traverse.embedder

import com.dylibso.chicory.wabt.Wat2Wasm
import java.io.File
import java.security.MessageDigest
import java.nio.file.Files
import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Test

class ChicoryRuntimeBridgeTest {
    @Test fun verifiesAndInstantiatesTheGovernedBridge() {
        val wasm = Wat2Wasm.parse(validBridgeWat)
        val bundle = fixtureBundle(wasm)

        val bridge = ChicoryRuntimeBridge(bundle)

        assertEquals(digest(wasm), bridge.runtimeWasmDigest)
        assertEquals("runtime.wasm", bridge.runtimeFile.name)
    }

    @Test fun rejectsTamperingBeforeInstantiation() {
        val bundle = fixtureBundle(
            Wat2Wasm.parse(validBridgeWat),
            "sha256:" + "0".repeat(64),
        )

        val error = assertThrows(TraverseBundleException::class.java) {
            ChicoryRuntimeBridge(bundle)
        }
        assertEquals("bundle_digest_mismatch", error.message)
    }

    @Test fun rejectsAmbientImportsAndBridgeTen() {
        val imported = Wat2Wasm.parse(
            """
            (module
              (import "wasi_snapshot_preview1" "fd_write" (func))
              (memory (export "memory") 1)
              (func (export "traverse_bridge_abi_version") (result i32) i32.const 10100))
            """.trimIndent(),
        )
        val importError = assertThrows(TraverseBundleException::class.java) {
            ChicoryRuntimeBridge(fixtureBundle(imported))
        }
        assertEquals("runtime/runtime.wasm requires undeclared ambient imports", importError.message)

        val bridgeTen = Wat2Wasm.parse(validBridgeWat.replace("i32.const 10100", "i32.const 10000"))
        val versionError = assertThrows(TraverseBundleException::class.java) {
            ChicoryRuntimeBridge(fixtureBundle(bridgeTen))
        }
        assertEquals("bridge_version_mismatch", versionError.message)
    }

    private fun fixtureBundle(wasm: ByteArray, declaredDigest: String = digest(wasm)): TraverseBundle {
        val root = Files.createTempDirectory("traverse-kotlin-bridge").toFile()
        val runtime = File(root, "runtime").apply { mkdirs() }
        File(runtime, "runtime.wasm").writeBytes(wasm)
        return TraverseBundle(root.absolutePath, declaredDigest)
    }

    private fun digest(bytes: ByteArray): String = "sha256:" +
        MessageDigest.getInstance("SHA-256").digest(bytes).joinToString("") { "%02x".format(it) }

    private val validBridgeWat = """
        (module
          (memory (export "memory") 1 16)
          (func (export "traverse_bridge_abi_version") (result i32) i32.const 10100)
          (func (export "traverse_alloc") (param i32) (result i32) i32.const 64)
          (func (export "traverse_dealloc") (param i32 i32))
          (func (export "traverse_init") (param i32 i32 i32) (result i32) i32.const 0)
          (func (export "traverse_submit") (param i32 i32 i32) (result i32) i32.const 0)
          (func (export "traverse_next_event") (param i32) (result i32) i32.const 0)
          (func (export "traverse_cancel") (param i32 i32 i32) (result i32) i32.const 0)
          (func (export "traverse_shutdown") (param i32) (result i32) i32.const 0)
          (func (export "traverse_compatible_start") (param i32 i32 i32) (result i32) i32.const 0)
          (func (export "traverse_compatible_stop") (param i32 i32 i32) (result i32) i32.const 0)
          (func (export "traverse_compatible_kill") (param i32 i32 i32) (result i32) i32.const 0))
        """.trimIndent()
}
