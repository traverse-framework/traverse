package dev.traverse.embedder

import com.dylibso.chicory.wabt.Wat2Wasm
import java.io.File
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.charset.StandardCharsets
import java.nio.file.Files
import java.security.MessageDigest
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertThrows
import org.junit.Test

class ChicoryBridgeClientTest {
    @Test fun realNativeArtifactRunsWithoutASidecar() {
        val rootPath = System.getenv("TRAVERSE_NATIVE_ARTIFACT_ROOT") ?: return
        val runtime = File(rootPath, "runtime/runtime.wasm").readBytes()
        val digest = "sha256:" + MessageDigest.getInstance("SHA-256")
            .digest(runtime).joinToString("") { "%02x".format(it) }
        // The default instruction budget is sized for a trivial fixture guest.
        // The real `runtime.wasm` interprets genuine Rust code (JSON parsing,
        // heap allocation, a nested wasmi engine) on `init`/`submit`, which
        // costs far more than a few `i32.store`s.
        val client = ChicoryBridgeClient(
            ChicoryRuntimeBridge(TraverseBundle(rootPath, digest), maximumInstructionsPerCall = 50_000_000L),
        )

        // The real `runtime-wasm-bridge/1.0.0` guest (crates/traverse-runtime-wasm)
        // hosts a *nested* capability itself, so `traverse_init`'s payload is
        // not bare JSON: a 4-byte little-endian header length, that many
        // bytes of JSON metadata, then the raw nested-capability WASM
        // artifact (spec 1402 FR-003/FR-011). This nested capability echoes
        // stdin to stdout, then emits one declared domain event — matching
        // `crates/traverse-runtime/tests/native_bridge_conformance.rs`'s
        // fixture exactly, so all host profiles exercise the same lifecycle
        // transcript.
        val nestedCapability = Wat2Wasm.parse(nestedConformanceCapabilityWat)
        val header = "{\"capability_id\":\"kotlin.conformance.echo\",\"capability_version\":\"1.0.0\"," +
            "\"service_type\":\"subscribable\"," +
            "\"emits\":[{\"event_id\":\"conformance.echoed\",\"version\":\"1.0.0\"}]," +
            "\"host_placement_target\":\"local\",\"permitted_targets\":[\"local\"]}"
        val headerBytes = header.toByteArray(StandardCharsets.UTF_8)
        val headerLength = ByteBuffer.allocate(4).order(ByteOrder.LITTLE_ENDIAN).putInt(headerBytes.size).array()
        val initPayload = headerLength + headerBytes + nestedCapability

        val initResponse = Json.parseToJsonElement(client.initialize(initPayload)).jsonObject
        assertEquals("ready", initResponse["status"]?.jsonPrimitive?.content)

        val submitResponse = Json.parseToJsonElement(
            client.submit("{\"hello\":\"kotlin-conformance\"}"),
        ).jsonObject
        assertEquals("accepted", submitResponse["status"]?.jsonPrimitive?.content)

        val eventTypes = generateSequence { client.nextEvent() }
            .map { Json.parseToJsonElement(it).jsonObject["type"]?.jsonPrimitive?.content }
            .toList()
        assertEquals(listOf("capability_invoked", "conformance.echoed", "capability_result"), eventTypes)

        assertEquals("{\"status\":\"stopped\"}", client.shutdown())
    }

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

    /** A WASI-command capability that echoes stdin to stdout, then calls
     * `traverse_host::emit_event` with a fixed declared domain event — byte-
     * identical to `crates/traverse-runtime/tests/native_bridge_conformance.rs`'s
     * `NESTED_CAPABILITY_WAT`, so every host profile's conformance run
     * exercises the same nested-capability behavior. */
    private val nestedConformanceCapabilityWat = """
        (module
          (import "wasi_snapshot_preview1" "fd_read"
            (func ${'$'}fd_read (param i32 i32 i32 i32) (result i32)))
          (import "wasi_snapshot_preview1" "fd_write"
            (func ${'$'}fd_write (param i32 i32 i32 i32) (result i32)))
          (import "traverse_host" "emit_event"
            (func ${'$'}emit_event (param i32 i32) (result i32)))
          (memory (export "memory") 1)
          (data (i32.const 5000) "{\22event_id\22:\22conformance.echoed\22,\22version\22:\221.0.0\22,\22payload\22:{\22ok\22:true}}")
          (func (export "_start")
            (i32.store (i32.const 0) (i32.const 8))
            (i32.store (i32.const 4) (i32.const 1024))
            (drop (call ${'$'}fd_read (i32.const 0) (i32.const 0) (i32.const 1) (i32.const 4100)))
            (i32.store (i32.const 0) (i32.const 8))
            (i32.store (i32.const 4) (i32.load (i32.const 4100)))
            (drop (call ${'$'}fd_write (i32.const 1) (i32.const 0) (i32.const 1) (i32.const 4104)))
            (drop (call ${'$'}emit_event (i32.const 5000) (i32.const 73)))
          )
        )
    """.trimIndent()

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
