package dev.traverse.embedder

import java.io.File
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
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

/**
 * Decision 108 (#1580): the Kotlin `ExactModelHost` runs the shared Rust model host through the
 * JNI shim (host-JVM build) and matches native, web, and Swift: the signed vectors byte-for-byte
 * and the shared Spec 138 rights conformance suite (FR-041) exactly.
 */
class ExactModelHostTest {
    private fun repoFile(path: String): File {
        var directory: File? = File(".").absoluteFile
        while (directory != null) {
            val candidate = File(directory, path)
            if (candidate.exists()) return candidate
            directory = directory.parentFile
        }
        throw java.io.FileNotFoundException("$path not found above ${File(".").absolutePath}")
    }

    private fun json(path: String): JsonObject = Json.parseToJsonElement(repoFile(path).readText()).jsonObject

    private fun hex(value: String): ByteArray = ByteArray(value.length / 2) { value.substring(it * 2, it * 2 + 2).toInt(16).toByte() }

    private fun ByteArray.toHex(): String = joinToString("") { "%02x".format(it) }

    private val JsonObject.s get() = { name: String -> this[name]!!.jsonPrimitive.content }

    private fun pin(value: JsonElement) = ExactModelPin.fromJson(value.toString())

    private fun vectorCheck(vectorPath: String, maxOutput: Int) = runBlocking {
        val vector = json("fixtures/models/conformance/$vectorPath")
        val pin = pin(vector["pin"]!!)
        ExactModelHost(listOf(pin), listOf(vector.s("trusted_public_key_hex")), "commercial").use { host ->
            val dir = "fixtures/models/${vector.s("package_dir")}"
            val digest = host.registerPackage(
                repoFile("$dir/model.manifest.json").readBytes(),
                repoFile("$dir/model.wasm").readBytes(),
                repoFile("$dir/model.sig.json").readBytes(),
            )
            assertEquals(pin.digest, digest)
            val request = vector["request"]!!.jsonObject
            val cases = (vector["cases"] as? JsonArray)?.map { it.jsonObject }
                ?: listOf(buildJsonObject {
                    put("input_frame_hex", request.s("input_frame_hex"))
                    put("output_frame_hex", vector["expected"]!!.jsonObject.s("output_frame_hex"))
                })
            for (case in cases) {
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
    fun rightsConformanceSuiteMatchesNativeWebAndSwift() = runBlocking {
        val suite = json("fixtures/models/rights-conformance/suite.json")
        val run = suite["execute"]!!.jsonObject
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
                            var wasm = repoFile(suite.s("wasm_path")).readBytes()
                            var signature = repoFile("$dir/model.sig.json").readBytes()
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
                            val digest = host.registerPackage(repoFile("$dir/model.manifest.json").readBytes(), wasm, signature)
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

    @Test
    fun adapterCarriesEvidenceOnSuccessAndDetailOnDenial() = runBlocking {
        val suite = json("fixtures/models/rights-conformance/suite.json")
        val permissive = suite["cases"]!!.jsonArray.map { it.jsonObject }.first { it.s("id") == "permissive-commercial" }
        val pin = pin(permissive["pins"]!!.jsonArray.first())
        ExactModelHost(listOf(pin), listOf(suite.s("trusted_public_key_hex")), "commercial").use { host ->
            val dir = "${suite.s("package_dir")}/permissive"
            host.registerPackage(
                repoFile("$dir/model.manifest.json").readBytes(),
                repoFile(suite.s("wasm_path")).readBytes(),
                repoFile("$dir/model.sig.json").readBytes(),
            )
            val run = suite["execute"]!!.jsonObject
            val inputRef = host.stageModelInput(hex(run.s("input_hex")), 4096)
            val payload = buildJsonObject {
                put("model_ref", buildJsonObject {
                    put("model_id", pin.modelId); put("version", pin.version); put("digest", pin.digest)
                })
                put("input_ref", inputRef)
                put("policy_ref", "policy-1")
                put("data_classification", "sensitive")
                put("input_schema_ref", "schema:fixture-in")
                put("input_schema_version", "1.0.0")
                put("max_output_bytes", 4096)
                put("allowed_classifications", JsonArray(listOf(JsonPrimitive("sensitive"))))
            }
            val ok = host.modelExecuteAdapter(HostConnectorRequest("run", "c1", "s1", payload.toString()))
            assertEquals("succeeded", ok.resultClass)
            val body = Json.parseToJsonElement(ok.payloadJson).jsonObject
            assertEquals("active", body["model_evidence"]!!.jsonObject.s("status"))

            host.setPackageStatus(mapOf(pin.digest to PackageStatusEntry("revoked", "withdrawn")))
            val revokedInput = host.stageModelInput(hex(run.s("input_hex")), 4096)
            val denied = host.modelExecuteAdapter(
                HostConnectorRequest("run", "c2", "s1", JsonObject(payload + ("input_ref" to JsonPrimitive(revokedInput))).toString()),
            )
            assertEquals("failed", denied.resultClass)
            val error = Json.parseToJsonElement(denied.payloadJson).jsonObject
            assertEquals("package_revoked", error.s("reason"))
            assertEquals("status", error["detail"]!!.jsonObject.s("field"))

            val malformed = host.modelExecuteAdapter(HostConnectorRequest("run", "c3", "s1", "[]"))
            assertEquals("invalid_input", Json.parseToJsonElement(malformed.payloadJson).jsonObject.s("error_code"))
            assertNull(host.modelRights("00"))
        }
    }

    @Test
    fun missingNativeEngineFailsClosed() {
        ExactModelHost.requireEngine(null)
        try {
            ExactModelHost.requireEngine(UnsatisfiedLinkError("no traverse_android_host"))
            fail("expected engine_unavailable")
        } catch (error: ExactModelError) {
            assertEquals("model_unavailable", error.code)
            assertEquals("engine_unavailable", error.reason)
        }
        assertTrue("host-JVM library must load in tests", ExactModelNative.loadFailure == null)
    }
}
