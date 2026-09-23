package dev.traverse.embedder

import java.util.concurrent.atomic.AtomicInteger
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.int
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put

/** Typed public embedder backed exclusively by runtime-owned bridge results. */
class RuntimeTraverseEmbedder internal constructor(
    private val client: ChicoryBridgeClient,
    timer: TraverseTimer = SystemTraverseTimer(),
) {
    constructor(bundle: TraverseBundle) : this(ChicoryBridgeClient(ChicoryRuntimeBridge(bundle)))

    private val appCommands = AppCommandCoordinator(submitJson = { client.submit(it) }, timer = timer)
    private val eventSequence = AtomicInteger(0)

    fun initialize(configJson: String): String = client.initialize(configJson)

    fun submit(submission: TraverseSubmission): TraverseSubmissionResult {
        val result = resultObject(client.submit(buildJsonObject {
            put("target_id", submission.targetId)
            put("input", Json.parseToJsonElement(submission.inputJson))
        }.toString()))
        return TraverseSubmissionResult(result.requiredString("session_id"), result.requiredString("status"))
    }

    /**
     * Spec 139 `app_command` submit. The state machine runs in `runtime.wasm`; registered
     * adapters and the timer port complete host-connector waits.
     */
    fun submit(command: TraverseAppCommand): TraverseSubmissionResult = appCommands.submit(command)

    /**
     * Registers the host authority for a manifest command (Spec 140 WIT semantics). Removing the
     * registration only applies while it is still the registered adapter.
     */
    fun registerHostConnectorAdapter(command: String, adapter: HostConnectorAdapter): HostConnectorRegistration =
        appCommands.register(command, adapter)

    /**
     * Drains ordered runtime events. Legacy bridge events (`sequence`, `target_id`, `status`) are
     * parsed as before. Spec 139 app lifecycle events (`type`, `session_id`, `data`) are mapped to
     * `eventType`, `sessionId`, and `output` (also `errorData` for `error`), numbered in arrival
     * order, so state-machine events are observable on every embedder (Spec 139 FR-004).
     */
    fun subscribe(): List<TraverseRuntimeEvent> = buildList {
        while (true) {
            val value = resultObject(client.nextEvent() ?: break)
            if (value["sequence"] == null && value["type"] != null) {
                add(lifecycleEvent(value))
                continue
            }
            add(TraverseRuntimeEvent(
                value.requiredInt("sequence"),
                value.requiredString("target_id"),
                value.requiredString("status"),
                value.optionalString("instance_id"),
            ))
        }
    }

    private fun lifecycleEvent(event: JsonObject): TraverseRuntimeEvent {
        val type = event.requiredString("type")
        val data = event["data"] ?: JsonObject(emptyMap())
        val eventType = if (type in lifecycleEventTypes) type else "error"
        val sequence = eventSequence.incrementAndGet()
        val dataJson = data.toString()
        return TraverseRuntimeEvent(
            sequence = sequence,
            targetId = "app_command",
            status = "emitted",
            eventType = eventType,
            sessionId = event.optionalString("session_id"),
            errorData = if (eventType == "error") dataJson else null,
            output = dataJson,
        )
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

    fun shutdown(): String {
        appCommands.stop()
        return client.shutdown()
    }

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

    private companion object {
        val lifecycleEventTypes = setOf(
            "state_changed", "capability_invoked", "capability_result", "capability_event",
            "capability_succeeded", "capability_failed", "host_connector_succeeded",
            "host_connector_failed", "host_connector_cancelled", "host_connector_timeout",
            "error", "heartbeat",
        )
    }
}
