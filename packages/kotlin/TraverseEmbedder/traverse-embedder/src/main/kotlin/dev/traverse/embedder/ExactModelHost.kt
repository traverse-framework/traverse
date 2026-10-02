package dev.traverse.embedder

import java.io.ByteArrayOutputStream
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.UUID
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.doubleOrNull
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.longOrNull
import kotlinx.serialization.json.put

/** Spec 138 exact-ref model failure: stable [code], refining [reason], and rights [detail]. */
class ExactModelError(
    val code: String,
    val reason: String?,
    override val message: String,
    val detail: ModelRightsDenialDetail? = null,
) : Exception(message)

/** Why a rights check failed (Spec 138 0.8.0); identity is null only before a package is known. */
data class ModelRightsDenialDetail(
    val modelId: String?,
    val version: String?,
    val digest: String?,
    val field: String,
    val expected: String,
    val actual: String,
    val effectiveUsage: String?,
)

/** Signed provenance of a derivative package (`rights.derivation`, manifest schema 2.1.0). */
data class ModelDerivation(
    val kind: String,
    val sourceDigest: String,
    val sourceLicenseId: String,
    val sourceCommercialUse: String,
    val sourceUrl: String,
)

/** Signed rights of a registered package, for host/UI display. */
data class ModelRights(
    val licenseId: String,
    val attribution: String,
    val redistribution: String,
    val commercialUse: String,
    val sourceUrl: String,
    val derivation: ModelDerivation?,
)

/** Verified rights record (Spec 138 0.8.0 FR-040). `status` is `revoked` only on a host query. */
data class ModelRightsRecord(
    val modelId: String,
    val version: String,
    val digest: String,
    val rights: ModelRights,
    val status: String,
    val statusReason: String?,
    val effectiveUsage: String,
)

/** Host-owned package status: `deprecated` runs but is flagged; `revoked` fails closed. */
data class PackageStatusEntry(val status: String, val reason: String)

/** Application `exact_model_dependencies` pin (Spec 138). */
data class ExactModelPin(
    val modelId: String,
    val version: String,
    val digest: String,
    val licenseId: String,
    val commercialUse: String,
    val offlineAllowed: Boolean = true,
    val target: String = "wasm-cpu",
    val keyId: String? = null,
) {
    internal fun toJson(): JsonObject = buildJsonObject {
        put("model_id", modelId)
        put("version", version)
        put("digest", digest)
        put("offline_allowed", offlineAllowed)
        put("target", target)
        put("rights", buildJsonObject {
            put("license_id", licenseId)
            put("commercial_use", commercialUse)
        })
        keyId?.let { put("key_id", it) }
    }

    companion object {
        /** Parse a pin from its app-manifest JSON. */
        fun fromJson(json: String): ExactModelPin {
            val value = Json.parseToJsonElement(json).jsonObject
            val rights = value.obj("rights")
            return ExactModelPin(
                modelId = value.str("model_id"),
                version = value.str("version"),
                digest = value.str("digest"),
                licenseId = rights.str("license_id"),
                commercialUse = rights.str("commercial_use"),
                offlineAllowed = (value["offline_allowed"] as? JsonPrimitive)?.booleanOrNull ?: true,
                target = value.optStr("target") ?: "wasm-cpu",
                keyId = value.optStr("key_id"),
            )
        }
    }
}

/** Host ceilings a package's declared limits must fit (Decision 104); phone-sized defaults. */
data class ExactModelHostLimits(
    val maxPackageBytes: Long = 128L * 1024 * 1024,
    val maxMemoryBytes: Long = 256L * 1024 * 1024,
    /** `wasmi` fuel units (engine-relative; Spec 138 FR-030). */
    val maxFuel: Long = 20_000_000_000L,
)

/** Typed `model.execute` result with identity, placement, usage, and rights evidence. */
data class ExactModelExecution(
    val outputRef: String,
    val placement: String,
    val target: String,
    val modelId: String,
    val version: String,
    val digest: String,
    val dataClassification: String,
    val inputBytes: Int,
    val outputBytes: Int,
    val durationMs: Double,
    val modelEvidence: ModelRightsRecord?,
)

/** The JNI boundary (Decision 108, ADR-0079): one framed call into `traverse-android-host`. */
internal object ExactModelNative {
    /** System property naming an explicit library path (host-JVM tests); else `loadLibrary`. */
    const val LIBRARY_PATH_PROPERTY = "traverse.android.host.library"

    val loadFailure: Throwable? = try {
        val path = System.getProperty(LIBRARY_PATH_PROPERTY)
        if (path.isNullOrBlank()) System.loadLibrary("traverse_android_host") else System.load(path)
        null
    } catch (error: Throwable) {
        error
    }

    @JvmStatic
    external fun modelCall(handle: Long, request: ByteArray): ByteArray
}

/**
 * Kotlin/Android Spec 138 exact-ref model host (Decision 108). Verification, rights, policy, and
 * the `wasmi` guest run in the shared Rust `traverse-model-host-frame` behind the JNI shim; trust
 * stays host-owned. If the native library cannot load, every call fails closed with
 * `model_unavailable` / `engine_unavailable`.
 *
 * @param modelUsage the app manifest `model_usage` (`commercial` | `non_commercial`); registration
 *   fails closed with `usage_undeclared` when it is null.
 * @param hostRequiresCommercial host tightening; a host can never relax an app's usage.
 */
class ExactModelHost(
    pins: List<ExactModelPin>,
    trustedPublicKeysHex: List<String>,
    modelUsage: String?,
    hostRequiresCommercial: Boolean = false,
    limits: ExactModelHostLimits = ExactModelHostLimits(),
) : AutoCloseable {
    private val handle: Long

    init {
        val header = buildJsonObject {
            put("op", "create")
            put("pins", JsonArray(pins.map { it.toJson() }))
            put("trusted_public_keys_hex", JsonArray(trustedPublicKeysHex.map { JsonPrimitive(it) }))
            put("limits", buildJsonObject {
                put("max_package_bytes", limits.maxPackageBytes)
                put("max_memory_bytes", limits.maxMemoryBytes)
                put("max_fuel", limits.maxFuel)
            })
            modelUsage?.let { put("model_usage", it) }
            put("host_requires_commercial", hostRequiresCommercial)
        }
        handle = (call(0, header).header["handle"] as? JsonPrimitive)?.longOrNull
            ?: throw ExactModelError("unavailable", null, "model host was not created")
    }

    /** Verify and admit a signed package; returns the pin digest it is cached under. */
    suspend fun registerPackage(manifest: ByteArray, wasm: ByteArray, signature: ByteArray): String =
        withContext(Dispatchers.IO) {
            val response = call(
                handle,
                buildJsonObject { put("op", "register") },
                listOf("manifest" to manifest, "wasm" to wasm, "signature" to signature),
            )
            response.header.optStr("digest")
                ?: throw ExactModelError("model_incompatible", null, "registration returned no digest")
        }

    /** Stage input bytes; returns a single-consume opaque `input_ref`. */
    fun stageModelInput(bytes: ByteArray, maxBytes: Int): String =
        call(handle, buildJsonObject { put("op", "stage_input"); put("max_bytes", maxBytes) }, listOf("input" to bytes))
            .header.optStr("input_ref")
            ?: throw ExactModelError("invalid_input", null, "staging returned no input_ref")

    /** Read output bytes by `output_ref` (size-capped). */
    fun readModelOutput(outputRef: String, maxBytes: Int): ByteArray =
        call(handle, buildJsonObject {
            put("op", "read_output")
            put("output_ref", outputRef)
            put("max_bytes", maxBytes)
        }).segment("output") ?: ByteArray(0)

    /** Signed rights of a registered package, or null when unknown. */
    fun modelRights(digest: String): ModelRights? =
        call(handle, buildJsonObject { put("op", "rights"); put("digest", digest) }).header["rights"].asObject()?.let(::rights)

    /** Verified rights record (rights, status, effective usage), or null when unknown/undeclared. */
    fun modelRightsRecord(digest: String): ModelRightsRecord? =
        call(handle, buildJsonObject { put("op", "rights_record"); put("digest", digest) }).header["record"].asObject()
            ?.let(::record)

    /** Replace the host-owned package status map; a revocation blocks the next call. */
    fun setPackageStatus(entries: Map<String, PackageStatusEntry>) {
        call(handle, buildJsonObject {
            put("op", "set_package_status")
            put("entries", buildJsonObject {
                entries.forEach { (digest, entry) ->
                    put(digest, buildJsonObject { put("status", entry.status); put("reason", entry.reason) })
                }
            })
        })
    }

    /** Drop a staged input or output ref. */
    fun dropRef(reference: String) {
        call(handle, buildJsonObject { put("op", "drop_ref"); put("ref", reference) })
    }

    /**
     * Execute a pinned model. Coroutine cancellation interrupts the running inference mid-run
     * (`cancelled`); [timeoutMs] bounds it (`timeout`).
     */
    suspend fun execute(
        modelId: String,
        version: String,
        digest: String,
        inputRef: String,
        policyRef: String,
        dataClassification: String,
        inputSchemaRef: String,
        inputSchemaVersion: String,
        maxOutputBytes: Int,
        allowedClassifications: List<String>,
        timeoutMs: Int? = null,
    ): ExactModelExecution = execute(
        buildJsonObject {
            put("model_ref", buildJsonObject { put("model_id", modelId); put("version", version); put("digest", digest) })
            put("input_ref", inputRef)
            put("policy_ref", policyRef)
            put("data_classification", dataClassification)
            put("input_schema_ref", inputSchemaRef)
            put("input_schema_version", inputSchemaVersion)
            put("max_output_bytes", maxOutputBytes)
            timeoutMs?.let { put("timeout_ms", it) }
        },
        allowedClassifications,
    )

    /** Execute a raw Spec 137 `model.execute` payload (used by app-command routing). */
    suspend fun execute(payload: JsonObject, allowedClassifications: List<String>): ExactModelExecution = coroutineScope {
        val executionId = UUID.randomUUID().toString()
        val header = buildJsonObject {
            put("op", "execute")
            put("execution_id", executionId)
            put("allowed_classifications", JsonArray(allowedClassifications.map { JsonPrimitive(it) }))
            put("payload", payload)
        }
        val running = async(Dispatchers.IO) { call(handle, header) }
        val response = try {
            running.await()
        } catch (cancelled: CancellationException) {
            // Flip the native cancel flag for exactly this execution; the blocked call returns.
            call(handle, buildJsonObject { put("op", "cancel"); put("execution_id", executionId) })
            throw cancelled
        }
        val trace = response.header.optObj("trace")
        val usage = trace?.optObj("usage")
        val model = response.header.optObj("model_ref")
        ExactModelExecution(
            outputRef = response.header.optStr("output_ref").orEmpty(),
            placement = response.header.optStr("placement").orEmpty(),
            target = response.header.optStr("target").orEmpty(),
            modelId = model?.optStr("model_id").orEmpty(),
            version = model?.optStr("version").orEmpty(),
            digest = model?.optStr("digest").orEmpty(),
            dataClassification = trace?.optStr("data_classification").orEmpty(),
            inputBytes = (usage?.get("input_bytes") as? JsonPrimitive)?.intOrNull ?: 0,
            outputBytes = (usage?.get("output_bytes") as? JsonPrimitive)?.intOrNull ?: 0,
            durationMs = (usage?.get("duration_ms") as? JsonPrimitive)?.doubleOrNull ?: 0.0,
            modelEvidence = response.header["model_evidence"].asObject()?.let(::record),
        )
    }

    /** Register this host as the adapter for a manifest command routed to `model.execute`. */
    fun install(on: RuntimeTraverseEmbedder, command: String): HostConnectorRegistration =
        on.registerHostConnectorAdapter(command, modelExecuteAdapter)

    /**
     * The host-connector adapter [install] registers. The command payload is the Spec 137
     * `model.execute` payload plus `allowed_classifications`. Success carries
     * `{output_ref, placement, model_id, version, digest, model_evidence}`; failures carry `reason`
     * and the rights `detail`.
     */
    val modelExecuteAdapter: HostConnectorAdapter = { request ->
        val payload = runCatching { Json.parseToJsonElement(request.payloadJson).jsonObject }.getOrNull()
        if (payload == null) {
            failed("invalid_input", null, "model.execute payload is not a JSON object", null)
        } else {
            val allowed = (payload["allowed_classifications"] as? JsonArray)
                ?.mapNotNull { (it as? JsonPrimitive)?.contentOrNull }
                .orEmpty()
            try {
                val result = execute(JsonObject(payload - "allowed_classifications"), allowed)
                val raw = buildJsonObject {
                    put("output_ref", result.outputRef)
                    put("placement", result.placement)
                    put("model_id", result.modelId)
                    put("version", result.version)
                    put("digest", result.digest)
                    result.modelEvidence?.let { put("model_evidence", recordJson(it)) }
                }
                HostConnectorResult("succeeded", raw.toString())
            } catch (error: ExactModelError) {
                val resultClass = if (error.code == "cancelled" || error.code == "timeout") error.code else "failed"
                failed(error.code, error.reason, error.message, error.detail, resultClass)
            }
        }
    }

    override fun close() {
        runCatching { call(handle, buildJsonObject { put("op", "destroy") }) }
    }

    internal class Response(val header: JsonObject, private val payload: ByteArray) {
        fun segment(name: String): ByteArray? {
            val range = (header.optObj("segments")?.get(name) as? JsonArray) ?: return null
            val start = (range.getOrNull(0) as? JsonPrimitive)?.intOrNull ?: return null
            val length = (range.getOrNull(1) as? JsonPrimitive)?.intOrNull ?: return null
            if (start < 0 || length < 0 || start + length > payload.size) return null
            return payload.copyOfRange(start, start + length)
        }
    }

    internal companion object {
        fun encode(header: JsonObject, segments: List<Pair<String, ByteArray>>): ByteArray {
            val payload = ByteArrayOutputStream()
            val offsets = buildJsonObject {
                segments.forEach { (name, bytes) ->
                    put(name, JsonArray(listOf(JsonPrimitive(payload.size()), JsonPrimitive(bytes.size))))
                    payload.write(bytes)
                }
            }
            val full = if (segments.isEmpty()) header else JsonObject(header + ("segments" to offsets))
            val headerBytes = full.toString().toByteArray(Charsets.UTF_8)
            val out = ByteArrayOutputStream()
            out.write(ByteBuffer.allocate(4).order(ByteOrder.LITTLE_ENDIAN).putInt(headerBytes.size).array())
            out.write(headerBytes)
            out.write(payload.toByteArray())
            return out.toByteArray()
        }

        fun decode(frame: ByteArray): Response {
            if (frame.size < 4) throw invalidResponse()
            val length = ByteBuffer.wrap(frame, 0, 4).order(ByteOrder.LITTLE_ENDIAN).int
            if (length < 0 || frame.size < 4 + length) throw invalidResponse()
            val header = runCatching {
                Json.parseToJsonElement(String(frame, 4, length, Charsets.UTF_8)).jsonObject
            }.getOrElse { throw invalidResponse() }
            return Response(header, frame.copyOfRange(4 + length, frame.size))
        }

        private fun invalidResponse() = ExactModelError("unavailable", null, "malformed model host response")

        /** Fail closed when the native engine could not load (Decision 108). */
        fun requireEngine(loadFailure: Throwable?) {
            if (loadFailure != null) {
                throw ExactModelError(
                    "model_unavailable",
                    "engine_unavailable",
                    "the native model engine (traverse_android_host) could not be loaded",
                )
            }
        }

        fun call(handle: Long, header: JsonObject, segments: List<Pair<String, ByteArray>> = emptyList()): Response {
            requireEngine(ExactModelNative.loadFailure)
            val response = decode(ExactModelNative.modelCall(handle, encode(header, segments)))
            if ((response.header["ok"] as? JsonPrimitive)?.booleanOrNull == false) {
                val error = response.header.optObj("error")
                throw ExactModelError(
                    error?.optStr("code") ?: "unavailable",
                    error?.optStr("reason"),
                    error?.optStr("message").orEmpty(),
                    error?.get("detail").asObject()?.let(::denial),
                )
            }
            return response
        }

        fun failed(
            code: String,
            reason: String?,
            message: String,
            detail: ModelRightsDenialDetail?,
            resultClass: String = "failed",
        ): HostConnectorResult {
            val body = buildJsonObject {
                put("error_code", code)
                put("message", message)
                reason?.let { put("reason", it) }
                detail?.let {
                    put("detail", buildJsonObject {
                        it.modelId?.let { v -> put("model_id", v) }
                        it.version?.let { v -> put("version", v) }
                        it.digest?.let { v -> put("digest", v) }
                        put("field", it.field)
                        put("expected", it.expected)
                        put("actual", it.actual)
                        it.effectiveUsage?.let { v -> put("effective_usage", v) }
                    })
                }
            }
            return HostConnectorResult(resultClass, body.toString())
        }

        fun rights(value: JsonObject) = ModelRights(
            licenseId = value.str("license_id"),
            attribution = value.str("attribution"),
            redistribution = value.str("redistribution"),
            commercialUse = value.str("commercial_use"),
            sourceUrl = value.str("source_url"),
            derivation = value["derivation"].asObject()?.let {
                ModelDerivation(
                    kind = it.str("kind"),
                    sourceDigest = it.str("source_digest"),
                    sourceLicenseId = it.str("source_license_id"),
                    sourceCommercialUse = it.str("source_commercial_use"),
                    sourceUrl = it.str("source_url"),
                )
            },
        )

        fun record(value: JsonObject) = ModelRightsRecord(
            modelId = value.str("model_id"),
            version = value.str("version"),
            digest = value.str("digest"),
            rights = rights(value.obj("rights")),
            status = value.str("status"),
            statusReason = value.optStr("status_reason"),
            effectiveUsage = value.str("effective_usage"),
        )

        fun recordJson(record: ModelRightsRecord): JsonObject = buildJsonObject {
            put("model_id", record.modelId)
            put("version", record.version)
            put("digest", record.digest)
            put("rights", buildJsonObject {
                put("license_id", record.rights.licenseId)
                put("attribution", record.rights.attribution)
                put("redistribution", record.rights.redistribution)
                put("commercial_use", record.rights.commercialUse)
                put("source_url", record.rights.sourceUrl)
                record.rights.derivation?.let { d ->
                    put("derivation", buildJsonObject {
                        put("kind", d.kind)
                        put("source_digest", d.sourceDigest)
                        put("source_license_id", d.sourceLicenseId)
                        put("source_commercial_use", d.sourceCommercialUse)
                        put("source_url", d.sourceUrl)
                    })
                }
            })
            put("status", record.status)
            record.statusReason?.let { put("status_reason", it) }
            put("effective_usage", record.effectiveUsage)
        }

        fun denial(value: JsonObject) = ModelRightsDenialDetail(
            modelId = value.optStr("model_id"),
            version = value.optStr("version"),
            digest = value.optStr("digest"),
            field = value.str("field"),
            expected = value.str("expected"),
            actual = value.str("actual"),
            effectiveUsage = value.optStr("effective_usage"),
        )
    }
}

private fun JsonElement?.asObject(): JsonObject? = if (this == null || this is JsonNull) null else this as? JsonObject

private fun JsonObject.optStr(name: String): String? = (this[name] as? JsonPrimitive)?.takeIf { it.isString }?.content

private fun JsonObject.str(name: String): String =
    optStr(name) ?: throw ExactModelError("unavailable", null, "model host response missing $name")

private fun JsonObject.optObj(name: String): JsonObject? = this[name].asObject()

private fun JsonObject.obj(name: String): JsonObject =
    optObj(name) ?: throw ExactModelError("unavailable", null, "model host response missing $name")
