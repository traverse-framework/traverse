package dev.traverse.embedder

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.int
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import org.junit.runner.RunWith

/**
 * #1611 (Decision 108 follow-up): the Kotlin `ExactModelHost` on a real Android runtime.
 *
 * `traverse_android_host` loads through `System.loadLibrary` from the `jniLibs` the AAR ships
 * (cargo-ndk `x86_64` on the CI emulator). The tests run the signed vectors byte-for-byte and the
 * shared Spec 138 rights conformance suite (FR-041). The fixtures are `fixtures/models`, bundled
 * as test-APK assets. The Android Test Orchestrator runs each test in its own process, so
 * [missingNativeLibraryFailsClosed] can make the load fail without affecting the other tests.
 */
@RunWith(AndroidJUnit4::class)
class ExactModelHostInstrumentedTest {
    /** Repository paths (`fixtures/models/...`) map onto the test APK's asset root. */
    private fun read(path: String): ByteArray =
        InstrumentationRegistry.getInstrumentation().context.assets
            .open(path.removePrefix("fixtures/models/"))
            .use { it.readBytes() }

    private fun json(path: String): JsonObject = Json.parseToJsonElement(String(read(path))).jsonObject

    private fun hex(value: String): ByteArray = ByteArray(value.length / 2) { value.substring(it * 2, it * 2 + 2).toInt(16).toByte() }

    private fun ByteArray.toHex(): String = joinToString("") { "%02x".format(it) }

    private val JsonObject.s get() = { name: String -> this[name]!!.jsonPrimitive.content }

    private fun pin(value: JsonElement) = ExactModelPin.fromJson(value.toString())

    private fun requireLoadedFromApk() {
        assertNull("System.loadLibrary must find the APK's traverse_android_host", ExactModelNative.loadFailure)
    }

    private fun vectorCheck(vectorPath: String, maxOutput: Int, rounds: Int = 1) = runBlocking {
        requireLoadedFromApk()
        val vector = json("fixtures/models/conformance/$vectorPath")
        val pin = pin(vector["pin"]!!)
        ExactModelHost(listOf(pin), listOf(vector.s("trusted_public_key_hex")), "commercial").use { host ->
            val dir = "fixtures/models/${vector.s("package_dir")}"
            val digest = host.registerPackage(
                read("$dir/model.manifest.json"),
                read("$dir/model.wasm"),
                read("$dir/model.sig.json"),
            )
            assertEquals(pin.digest, digest)
            val request = vector["request"]!!.jsonObject
            val cases = (vector["cases"] as? JsonArray)?.map { it.jsonObject }
                ?: listOf(buildJsonObject {
                    put("input_frame_hex", request.s("input_frame_hex"))
                    put("output_frame_hex", vector["expected"]!!.jsonObject.s("output_frame_hex"))
                })
            for (case in List(rounds) { cases }.flatten()) {
                val inputRef = host.stageModelInput(hex(case.s("input_frame_hex")), 4096)
                val result = host.execute(
                    pin.modelId, pin.version, pin.digest, inputRef, "policy-1", "sensitive",
                    request.s("input_schema_ref"), "1.0.0", maxOutput, listOf("sensitive"),
                )
                assertEquals("wasm-cpu", result.placement)
                assertEquals(case.s("output_frame_hex"), host.readModelOutput(result.outputRef, maxOutput).toHex())
                assertEquals("active", result.modelEvidence?.status)
            }
        }
    }

    @Test
    fun signedClassifierVectorIsByteIdentical() = vectorCheck("signed-classifier.json", 4096)

    @Test
    fun signedDigitsMlpVectorIsByteIdentical() = vectorCheck("signed-digits-mlp.json", 64)

    @Test
    fun onnxRunnerVectorIsByteIdenticalWithSimd() = vectorCheck("signed-digits-onnx.json", 56)

    /** Guest ABI v3 (Decision 110): the first round prepares, later rounds restore the snapshot. */
    @Test
    fun preparedV3VectorIsByteIdenticalFreshAndWarm() = vectorCheck("signed-prepared-v3.json", 4096, rounds = 3)

    /** The public error shape every embedder compares: code, reason, detail. */
    private fun errorJson(error: ExactModelError): JsonObject = buildJsonObject {
        put("ok", false)
        put("code", error.code)
        put("reason", error.reason?.let { JsonPrimitive(it) } ?: JsonNull)
        error.detail?.let { detail ->
            put("detail", buildJsonObject {
                detail.modelId?.let { put("model_id", it) }
                detail.version?.let { put("version", it) }
                detail.digest?.let { put("digest", it) }
                put("field", detail.field)
                put("expected", detail.expected)
                put("actual", detail.actual)
                detail.effectiveUsage?.let { put("effective_usage", it) }
            })
        }
    }

    private suspend fun attempt(block: suspend () -> JsonElement): JsonElement = try {
        block()
    } catch (error: ExactModelError) {
        errorJson(error)
    }

    @Test
    fun rightsConformanceSuiteMatchesEveryOtherEmbedder() = runBlocking {
        requireLoadedFromApk()
        val suite = json("fixtures/models/rights-conformance/suite.json")
        val run = suite["execute"]!!.jsonObject
        // Scenario 10 (offline cache-only activation) is proven by the vector tests.
        val scenarios = mutableSetOf(10)
        for (case in suite["cases"]!!.jsonArray.map { it.jsonObject }) {
            val id = case.s("id")
            val pins = case["pins"]!!.jsonArray.map(::pin)
            fun pinFor(name: String) = pins.first { it.modelId == "fixture.rights.$name" }
            fun statusEntries(value: JsonElement?) = (value as? JsonObject)?.mapValues { (_, entry) ->
                PackageStatusEntry(entry.jsonObject.s("status"), entry.jsonObject.s("reason"))
            }.orEmpty()
            ExactModelHost(
                pins,
                listOf(suite.s("trusted_public_key_hex")),
                (case["model_usage"] as? JsonPrimitive)?.takeIf { it.isString }?.content,
                (case["host_requires_commercial"] as? JsonPrimitive)?.booleanOrNull ?: false,
            ).use { host ->
                host.setPackageStatus(statusEntries(case["package_status"]))
                for ((index, step) in case["steps"]!!.jsonArray.map { it.jsonObject }.withIndex()) {
                    val actual: JsonElement = when (step.s("op")) {
                        "register" -> attempt {
                            val dir = "${suite.s("package_dir")}/${step.s("package")}"
                            var wasm = read(suite.s("wasm_path"))
                            var signature = read("$dir/model.sig.json")
                            when ((step["tamper"] as? JsonPrimitive)?.content) {
                                "wasm" -> wasm += byteArrayOf(0)
                                "signature" -> {
                                    val document = Json.parseToJsonElement(String(signature)).jsonObject
                                    val bytes = hex(document.s("signature"))
                                    bytes[0] = (bytes[0].toInt() xor 1).toByte()
                                    signature = JsonObject(document + ("signature" to JsonPrimitive(bytes.toHex())))
                                        .toString().toByteArray()
                                }
                            }
                            val digest = host.registerPackage(read("$dir/model.manifest.json"), wasm, signature)
                            buildJsonObject { put("ok", true); put("digest", digest) }
                        }
                        "execute" -> attempt {
                            val pin = pinFor(step.s("package"))
                            val inputRef = host.stageModelInput(hex(run.s("input_hex")), 4096)
                            val result = host.execute(
                                pin.modelId, pin.version, pin.digest, inputRef, run.s("policy_ref"),
                                run.s("data_classification"), run.s("input_schema_ref"), run.s("input_schema_version"),
                                run["max_output_bytes"]!!.jsonPrimitive.int,
                                run["allowed_classifications"]!!.jsonArray.map { it.jsonPrimitive.content },
                            )
                            buildJsonObject {
                                put("ok", true)
                                put("output_hex", host.readModelOutput(result.outputRef, 4096).toHex())
                                put("model_evidence", ExactModelHost.recordJson(result.modelEvidence!!))
                            }
                        }
                        "rights_record" ->
                            host.modelRightsRecord(pinFor(step.s("package")).digest)?.let(ExactModelHost::recordJson) ?: JsonNull
                        else -> {
                            assertEquals("set_package_status", step.s("op"))
                            host.setPackageStatus(statusEntries(step["entries"]))
                            continue
                        }
                    }
                    assertEquals("$id step $index", step["expect"], actual)
                }
            }
            case["scenarios"]!!.jsonArray.forEach { scenarios.add(it.jsonPrimitive.int) }
        }
        assertEquals((1..10).toSet(), scenarios)
    }

    /**
     * The library stripped from the runtime (Decision 108 item 5): pointing the loader at a path
     * that does not exist, before anything touches [ExactModelNative], makes the load fail exactly
     * as a missing `.so` does. Every model call then fails closed with `engine_unavailable`, and the
     * rest of the embedder keeps working. The orchestrator gives this test its own process.
     */
    @Test
    fun missingNativeLibraryFailsClosed() = runBlocking {
        System.setProperty(ExactModelNative.LIBRARY_PATH_PROPERTY, "/data/local/tmp/traverse-missing/libtraverse_android_host.so")
        try {
            ExactModelHost(emptyList(), emptyList(), "commercial")
            throw AssertionError("expected engine_unavailable")
        } catch (error: ExactModelError) {
            assertEquals("model_unavailable", error.code)
            assertEquals("engine_unavailable", error.reason)
        }
        assert(ExactModelNative.loadFailure != null)
        // The pure-Kotlin parts of the embedder do not depend on the native engine.
        assertEquals("fixture.x", ExactModelPin.fromJson(
            """{"model_id":"fixture.x","version":"1","digest":"d","rights":{"license_id":"MIT","commercial_use":"allowed"}}""",
        ).modelId)
    }
}
