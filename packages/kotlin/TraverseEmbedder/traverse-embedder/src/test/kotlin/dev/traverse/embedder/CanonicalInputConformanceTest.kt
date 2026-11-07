package dev.traverse.embedder

import java.io.File
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.charset.StandardCharsets
import java.security.MessageDigest
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

/**
 * Cross-host canonical guest input (Spec 1722 FR-006, #1795).
 *
 * Runs fixtures/cross-host/canonical-input-v1 through the real `runtime.wasm` in
 * TRAVERSE_NATIVE_ARTIFACT_ROOT, once as raw bytes on the bridge client and once through
 * RuntimeTraverseEmbedder. The stdin-echo guest returns the bytes it read, which must match
 * the Rust reference on every host.
 */
class CanonicalInputConformanceTest {
    private val capabilityId = "conformance.stdin-echo"

    private val fixtureDir: File = generateSequence(File("").absoluteFile) { it.parentFile }
        .map { File(it, "fixtures/cross-host/canonical-input-v1") }
        .first { it.isDirectory }

    private data class Vector(
        val name: String,
        val input: String,
        val expectedStdin: String?,
        val expectedSha256: String?,
        val expectedError: String?,
    )

    private sealed interface Outcome {
        data class Output(val stdin: String) : Outcome
        data class Failure(val message: String) : Outcome
    }

    private fun vectors(): List<Vector> =
        Json.parseToJsonElement(File(fixtureDir, "vectors.json").readText()).jsonObject["vectors"]!!.jsonArray
            .map { it.jsonObject }
            .map {
                Vector(
                    name = it.string("name")!!,
                    input = it.string("input")!!,
                    expectedStdin = it.string("expected_stdin"),
                    expectedSha256 = it.string("expected_sha256"),
                    expectedError = it.string("expected_error"),
                )
            }

    private fun JsonObject.string(name: String): String? = this[name]?.jsonPrimitive?.contentOrNull

    private fun sha256(bytes: ByteArray): String =
        MessageDigest.getInstance("SHA-256").digest(bytes).joinToString("") { "%02x".format(it) }

    /** A fresh real `runtime.wasm` with stdin-echo as a stateless capability, or null without an artifact root. */
    private fun stdinEchoClient(): ChicoryBridgeClient? {
        val rootPath = System.getenv("TRAVERSE_NATIVE_ARTIFACT_ROOT") ?: return null
        val runtime = File(rootPath, "runtime/runtime.wasm").readBytes()
        val client = ChicoryBridgeClient(
            ChicoryRuntimeBridge(
                TraverseBundle(rootPath, "sha256:" + sha256(runtime)),
                maximumInstructionsPerCall = 50_000_000L,
            ),
        )
        val header = ("{\"capability_id\":\"$capabilityId\",\"capability_version\":\"1.0.0\"," +
            "\"service_type\":\"stateless\",\"emits\":[]," +
            "\"host_placement_target\":\"local\",\"permitted_targets\":[\"local\"]}")
            .toByteArray(StandardCharsets.UTF_8)
        val headerLength = ByteBuffer.allocate(4).order(ByteOrder.LITTLE_ENDIAN).putInt(header.size).array()
        val ready = client.initialize(headerLength + header + File(fixtureDir, "stdin-echo.wasm").readBytes())
        assertEquals("ready", Json.parseToJsonElement(ready).jsonObject.string("status"))
        return client
    }

    /** The guest's echoed stdin, or the failure message from submit or the result event. */
    private fun outcome(client: ChicoryBridgeClient, submit: () -> String?): Outcome = try {
        val rejection = submit()
        if (rejection != null) {
            Outcome.Failure(rejection)
        } else {
            generateSequence { client.nextEvent() }
                .map { Json.parseToJsonElement(it).jsonObject }
                .firstOrNull { it.string("type") == "capability_result" }
                ?.let { it["data"]!!.jsonObject }
                ?.let { data ->
                    if (data.string("status") == "completed") {
                        Outcome.Output(data["output"]!!.jsonObject.string("stdin")!!)
                    } else {
                        Outcome.Failure(data["error"].toString())
                    }
                }
                ?: Outcome.Failure("no capability_result event")
        }
    } catch (error: Exception) {
        Outcome.Failure(error.toString())
    }

    private fun check(host: String, vector: Vector, outcome: Outcome) {
        val code = vector.expectedError
        when {
            code != null && outcome is Outcome.Failure ->
                assertTrue("$host/${vector.name}: expected $code, got ${outcome.message}", code in outcome.message)
            code == null && outcome is Outcome.Output -> {
                assertEquals("$host/${vector.name}: stdin bytes", vector.expectedStdin, outcome.stdin)
                assertEquals(
                    "$host/${vector.name}: digest",
                    vector.expectedSha256,
                    sha256(outcome.stdin.toByteArray(StandardCharsets.UTF_8)),
                )
            }
            else -> fail("$host/${vector.name}: expected ${code ?: "output"}, got $outcome")
        }
    }

    @Test fun canonicalInputVectorsReachTheGuestAsRawBytes() {
        for (vector in vectors()) {
            val client = stdinEchoClient() ?: return
            check("kotlin raw", vector, outcome(client) {
                val response = client.submit(vector.input)
                response.takeIf { Json.parseToJsonElement(it).jsonObject.string("status") != "accepted" }
            })
        }
    }

    @Test fun canonicalInputVectorsReachTheGuestThroughTheEmbedder() {
        // TraverseSubmission carries the caller's JSON text unchanged, so bytes-only vectors
        // (duplicate member names) run here too.
        for (vector in vectors()) {
            val client = stdinEchoClient() ?: return
            val runtime = RuntimeTraverseEmbedder(client)
            check("kotlin embedder", vector, outcome(client) {
                val result = runtime.submit(TraverseSubmission(capabilityId, vector.input))
                if (result.status == "accepted") null else "${result.status}: ${result.error}"
            })
        }
    }

    /** A fresh real `runtime.wasm` initialized with a compatible capability on `android` (Spec 072 FR-010). */
    private fun compatibleClient(): ChicoryBridgeClient? {
        val rootPath = System.getenv("TRAVERSE_NATIVE_ARTIFACT_ROOT") ?: return null
        val runtime = File(rootPath, "runtime/runtime.wasm").readBytes()
        val client = ChicoryBridgeClient(
            ChicoryRuntimeBridge(TraverseBundle(rootPath, "sha256:" + sha256(runtime)), maximumInstructionsPerCall = 50_000_000L),
        )
        val header = ("{\"capability_id\":\"conformance.camera\",\"capability_version\":\"1.0.0\"," +
            "\"execution_mode\":\"compatible\",\"platforms\":[\"android\"],\"host_platform\":\"android\"}")
            .toByteArray(StandardCharsets.UTF_8)
        val headerLength = ByteBuffer.allocate(4).order(ByteOrder.LITTLE_ENDIAN).putInt(header.size).array()
        val ready = client.initialize(headerLength + header)
        assertEquals("ready", Json.parseToJsonElement(ready).jsonObject.string("status"))
        return client
    }

    @Test fun canonicalInputVectorsReachTheWrapperThroughCompatibleStart() {
        // Spec 072 FR-012 (Decision 128): the started event's `input` is the runtime's canonical bytes.
        for (vector in vectors()) {
            val runtime = RuntimeTraverseEmbedder(compatibleClient() ?: return)
            val result = try {
                runtime.compatibleStart("conformance.camera", vector.input)
                val output = runtime.subscribe().first { it.eventType == "state_changed" }.output!!
                Outcome.Output(output.substring(output.indexOf(",\"input\":") + ",\"input\":".length, output.length - 1))
            } catch (error: Exception) {
                Outcome.Failure(error.toString())
            }
            check("kotlin compatible", vector, result)
        }
    }

    @Test fun compatibleLifecycleUsesTheRuntimeRules() {
        val runtime = RuntimeTraverseEmbedder(compatibleClient() ?: return)
        val started = runtime.compatibleStart("conformance.camera", "{}")
        assertEquals("conformance.camera-compatible-1", started.instanceId)
        assertTrue(runCatching { runtime.compatibleStart("conformance.camera", "{}") }.isFailure)
        assertEquals("killed", runtime.compatibleKill("conformance.camera", null).status)
        assertTrue(runCatching { runtime.compatibleKill("conformance.camera", started.instanceId) }.isFailure)
        assertEquals(listOf("started", "killed"), runtime.subscribe().map { it.status })
    }
}
