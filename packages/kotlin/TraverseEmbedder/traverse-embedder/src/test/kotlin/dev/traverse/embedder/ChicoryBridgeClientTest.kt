package dev.traverse.embedder

import com.dylibso.chicory.wabt.Wat2Wasm
import java.io.File
import java.nio.file.Files
import java.security.MessageDigest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertThrows
import org.junit.Test

class ChicoryBridgeClientTest {
    @Test fun copiesJsonResultsAndDrainsEventsInOrder() {
        val client = ChicoryBridgeClient(ChicoryRuntimeBridge(fixtureBundle()))

        assertEquals("{\"status\":\"ready\"}", client.initialize("{}"))
        assertEquals("{\"session_id\":\"s1\",\"status\":\"accepted\"}", client.submit("{\"target_id\":\"demo\"}"))
        assertEquals("{\"sequence\":1,\"target_id\":\"demo\",\"status\":\"completed\"}", client.nextEvent())
        assertNull(client.nextEvent())
        assertEquals("{\"status\":\"stopped\"}", client.shutdown())
    }

    @Test fun mapsRuntimeOwnedResultsIntoPublicTypes() {
        val runtime = RuntimeTraverseEmbedder(ChicoryBridgeClient(ChicoryRuntimeBridge(fixtureBundle())))
        runtime.initialize("{}")

        assertEquals(
            TraverseSubmissionResult("s1", "accepted"),
            runtime.submit(TraverseSubmission("demo", "{}")),
        )
        assertEquals(
            listOf(TraverseRuntimeEvent(1, "demo", "completed")),
            runtime.subscribe(),
        )
        assertEquals("{\"status\":\"stopped\"}", runtime.shutdown())
    }

    @Test fun interruptsCallsThatExceedTheInstructionBudget() {
        val wasm = Wat2Wasm.parse(
            bridgeWat.replace(
                "local.get 2 i32.const 512 i32.const 18 call ${'$'}result",
                "(loop ${'$'}forever br ${'$'}forever) i32.const 0",
            ),
        )
        val client = ChicoryBridgeClient(
            ChicoryRuntimeBridge(fixtureBundle(wasm), maximumInstructionsPerCall = 100),
        )

        val error = assertThrows(TraverseBridgeException::class.java) { client.initialize("{}") }
        assertEquals(-4, error.status)
        assertEquals("bridge_resource_limit", error.message)
    }

    private fun fixtureBundle(wasm: ByteArray = Wat2Wasm.parse(bridgeWat)): TraverseBundle {
        val root = Files.createTempDirectory("traverse-kotlin-client").toFile()
        val runtime = File(root, "runtime").apply { mkdirs() }
        File(runtime, "runtime.wasm").writeBytes(wasm)
        val digest = "sha256:" + MessageDigest.getInstance("SHA-256")
            .digest(wasm).joinToString("") { "%02x".format(it) }
        return TraverseBundle(root.absolutePath, digest)
    }

    private val bridgeWat = """
        (module
          (memory (export "memory") 1 16)
          (data (i32.const 512) "{\22status\22:\22ready\22}")
          (data (i32.const 544) "{\22session_id\22:\22s1\22,\22status\22:\22accepted\22}")
          (data (i32.const 608) "{\22sequence\22:1,\22target_id\22:\22demo\22,\22status\22:\22completed\22}")
          (data (i32.const 704) "{\22status\22:\22stopped\22}")
          (global ${'$'}next (mut i32) (i32.const 0))
          (func (export "traverse_bridge_abi_version") (result i32) i32.const 10100)
          (func (export "traverse_alloc") (param i32) (result i32) i32.const 64)
          (func (export "traverse_dealloc") (param i32 i32))
          (func ${'$'}result (param ${'$'}d i32) (param ${'$'}p i32) (param ${'$'}n i32) (result i32)
            local.get ${'$'}d local.get ${'$'}p i32.store
            local.get ${'$'}d i32.const 4 i32.add local.get ${'$'}n i32.store
            i32.const 0)
          (func (export "traverse_init") (param i32 i32 i32) (result i32)
            local.get 2 i32.const 512 i32.const 18 call ${'$'}result)
          (func (export "traverse_submit") (param i32 i32 i32) (result i32)
            local.get 2 i32.const 544 i32.const 39 call ${'$'}result)
          (func (export "traverse_next_event") (param i32) (result i32)
            global.get ${'$'}next i32.eqz
            if (result i32)
              i32.const 1 global.set ${'$'}next
              local.get 0 i32.const 608 i32.const 54 call ${'$'}result drop
              i32.const 1
            else i32.const 0 end)
          (func (export "traverse_cancel") (param i32 i32 i32) (result i32)
            local.get 2 i32.const 544 i32.const 39 call ${'$'}result)
          (func (export "traverse_compatible_start") (param i32 i32 i32) (result i32)
            local.get 2 i32.const 544 i32.const 39 call ${'$'}result)
          (func (export "traverse_compatible_stop") (param i32 i32 i32) (result i32)
            local.get 2 i32.const 544 i32.const 39 call ${'$'}result)
          (func (export "traverse_compatible_kill") (param i32 i32 i32) (result i32)
            local.get 2 i32.const 544 i32.const 39 call ${'$'}result)
          (func (export "traverse_shutdown") (param i32) (result i32)
            local.get 0 i32.const 704 i32.const 20 call ${'$'}result))
    """.trimIndent()
}
