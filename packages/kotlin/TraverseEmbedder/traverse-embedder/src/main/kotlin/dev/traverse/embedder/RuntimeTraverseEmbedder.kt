package dev.traverse.embedder

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.int
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put

/** Typed public embedder backed exclusively by runtime-owned bridge results. */
class RuntimeTraverseEmbedder internal constructor(private val client: ChicoryBridgeClient) {
    constructor(bundle: TraverseBundle) : this(ChicoryBridgeClient(ChicoryRuntimeBridge(bundle)))

    fun initialize(configJson: String): String = client.initialize(configJson)

    fun submit(submission: TraverseSubmission): TraverseSubmissionResult {
        val result = resultObject(client.submit(buildJsonObject {
            put("target_id", submission.targetId)
            put("input", Json.parseToJsonElement(submission.inputJson))
        }.toString()))
        return TraverseSubmissionResult(result.requiredString("session_id"), result.requiredString("status"))
    }

    fun subscribe(): List<TraverseRuntimeEvent> = buildList {
        while (true) {
            val value = resultObject(client.nextEvent() ?: break)
            add(TraverseRuntimeEvent(
                value.requiredInt("sequence"),
                value.requiredString("target_id"),
                value.requiredString("status"),
                value.optionalString("instance_id"),
            ))
        }
    }

    fun cancel(sessionId: String): String = client.cancel(buildJsonObject {
        put("session_id", sessionId)
    }.toString())

    fun compatibleStart(capabilityId: String, inputJson: String): TraverseCompatibleResult =
        compatibleResult(client.compatibleStart(compatibleRequest(capabilityId, inputJson = inputJson)))

    fun compatibleStop(capabilityId: String, instanceId: String?): TraverseCompatibleResult =
        compatibleResult(client.compatibleStop(compatibleRequest(capabilityId, instanceId = instanceId)))

    fun compatibleKill(capabilityId: String, instanceId: String?): TraverseCompatibleResult =
        compatibleResult(client.compatibleKill(compatibleRequest(capabilityId, instanceId = instanceId)))

    fun shutdown(): String = client.shutdown()

    private fun compatibleRequest(
        capabilityId: String,
        inputJson: String? = null,
        instanceId: String? = null,
    ): String = buildJsonObject {
        put("capability_id", capabilityId)
        if (inputJson != null) {
            put("input", Json.parseToJsonElement(inputJson))
        } else {
            if (instanceId == null) put("instance_id", JsonNull) else put("instance_id", instanceId)
        }
    }.toString()

    private fun compatibleResult(json: String): TraverseCompatibleResult {
        val result = resultObject(json)
        return TraverseCompatibleResult(result.optionalString("instance_id"), result.requiredString("status"))
    }

    private fun resultObject(json: String): JsonObject = try {
        Json.parseToJsonElement(json).jsonObject
    } catch (error: IllegalArgumentException) {
        throw TraverseBridgeException(-2, "bridge_invalid_json")
    }

    private fun JsonObject.requiredString(name: String): String =
        this[name]?.jsonPrimitive?.content
            ?: throw TraverseBridgeException(-2, "bridge result is missing $name")

    private fun JsonObject.optionalString(name: String): String? {
        val value = this[name] ?: return null
        return if (value is JsonNull) null else value.jsonPrimitive.content
    }

    private fun JsonObject.requiredInt(name: String): Int =
        this[name]?.jsonPrimitive?.int
            ?: throw TraverseBridgeException(-2, "bridge result is missing $name")
}
