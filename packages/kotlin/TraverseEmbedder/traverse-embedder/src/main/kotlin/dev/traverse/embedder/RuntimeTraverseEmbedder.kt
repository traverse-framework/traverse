package dev.traverse.embedder

import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
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

    /** The runtime's canonical start input per instance, until its `compatible_started` event is published (Decision 128). */
    private val compatibleInputs = ConcurrentHashMap<String, String>()

    fun initialize(configJson: String): String = client.initialize(configJson)

    fun submit(submission: TraverseSubmission): TraverseSubmissionResult {
        // The caller's input goes into the envelope unchanged, so the runtime applies Spec 1722's
        // canonical rules to exactly what the caller sent. Re-serializing could alter or drop
        // input it must reject (#1795).
        val request = "{\"target_id\":${JsonPrimitive(submission.targetId)},\"input\":${submission.inputJson}}"
        val result = resultObject(client.submit(request))
        return TraverseSubmissionResult(
            result.requiredString("session_id"),
            result.requiredString("status"),
            result["error"]?.jsonPrimitive?.contentOrNull,
        )
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
            if (value.optionalString("type")?.startsWith("compatible_") == true) {
                add(compatibleEvent(value))
                continue
            }
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

    /**
     * Publishes a runtime `compatible_<state>` event as embedder-api `state_changed`. The started
     * event's `input` is the runtime's canonical text, spliced in unchanged for the platform wrapper
     * (Decision 128).
     */
    private fun compatibleEvent(event: JsonObject): TraverseRuntimeEvent {
        val state = event.requiredString("type").removePrefix("compatible_")
        val instanceId = event.optionalString("instance_id")
        val data = buildJsonObject {
            put("capability_id", event["capability_id"] ?: JsonNull)
            put("instance_id", event["instance_id"] ?: JsonNull)
            put("state", state)
            if (state == "started") put("previous_state", JsonNull) else put("previous_state", "started")
        }.toString()
        val input = if (state == "started" && instanceId != null) compatibleInputs.remove(instanceId) else null
        return TraverseRuntimeEvent(
            sequence = eventSequence.incrementAndGet(),
            targetId = "compatible",
            status = state,
            instanceId = instanceId,
            eventType = "state_changed",
            output = if (input == null) data else "${data.dropLast(1)},\"input\":$input}",
        )
    }

    fun cancel(sessionId: String): String = client.cancel(buildJsonObject {
        put("session_id", sessionId)
    }.toString())

    fun compatibleStart(capabilityId: String, inputJson: String): TraverseCompatibleResult {
        // The caller's input goes to the runtime unchanged. It applies Spec 1722 and returns the
        // canonical bytes the wrapper receives (Decision 128); re-serializing could alter them (#1811).
        val response = client.compatibleStart("{\"capability_id\":${JsonPrimitive(capabilityId)},\"input\":$inputJson}")
        val result = compatibleResult(response)
        val canonical = resultObject(response)["canonical_input"]?.jsonPrimitive?.contentOrNull
        if (result.instanceId != null && canonical != null) compatibleInputs[result.instanceId] = canonical
        return result
    }

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
        instanceId: String?,
    ): String = buildJsonObject {
        put("capability_id", capabilityId)
        if (instanceId == null) put("instance_id", JsonNull) else put("instance_id", instanceId)
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
