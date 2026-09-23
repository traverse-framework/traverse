package dev.traverse.embedder

import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.doubleOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put

/** Cancels a scheduled deadline. */
fun interface TraverseTimerHandle {
    fun cancel()
}

/** Host-provided monotonic timer port used for Spec 139 dual deadlines. */
interface TraverseTimer {
    /** Runs [callback] once after [delayMs] milliseconds unless the handle is cancelled first. */
    fun schedule(delayMs: Long, callback: () -> Unit): TraverseTimerHandle
}

/** Default timer port backed by a single-threaded daemon `ScheduledExecutorService`. */
class SystemTraverseTimer : TraverseTimer {
    override fun schedule(delayMs: Long, callback: () -> Unit): TraverseTimerHandle {
        val future = executor.schedule({ callback() }, delayMs.coerceAtLeast(0), TimeUnit.MILLISECONDS)
        return TraverseTimerHandle { future.cancel(false) }
    }

    private companion object {
        val executor = Executors.newSingleThreadScheduledExecutor { runnable ->
            Thread(runnable, "traverse-timer").apply { isDaemon = true }
        }
    }
}

/** A runtime-staged host-connector wait handed to a registered adapter. */
data class HostConnectorRequest(
    val command: String,
    val commandId: String,
    val sessionId: String,
    val payloadJson: String,
)

/** Adapter outcome. [resultClass] is `succeeded`, `failed`, `cancelled`, or `timeout`. */
data class HostConnectorResult(
    val resultClass: String,
    val payloadJson: String = "{}",
)

/**
 * Host-side authority for one manifest command (Spec 140 WIT semantics). It never runs inside
 * `runtime.wasm`; the runtime only receives the correlated terminal. Adapters should honor
 * coroutine cancellation, which [RuntimeTraverseEmbedder.shutdown] triggers.
 */
typealias HostConnectorAdapter = suspend (HostConnectorRequest) -> HostConnectorResult

/** Removes a registered adapter, but only while it is still the registered one. */
class HostConnectorRegistration internal constructor(private val onRemove: () -> Unit) {
    fun remove() = onRemove()
}

/**
 * Drives Spec 139 app commands. State-machine logic stays in `runtime.wasm`; this type only
 * submits envelopes, runs registered adapters for staged host-connector waits, and registers the
 * host half of the dual deadline. The first correlated terminal wins in the runtime.
 */
internal class AppCommandCoordinator(
    private val submitJson: (String) -> String,
    private val timer: TraverseTimer,
) {
    private val lock = Any()
    private val adapters = mutableMapOf<String, Pair<Long, HostConnectorAdapter>>()
    private var nextAdapterId = 0L
    private val deadlines = mutableListOf<TraverseTimerHandle>()
    private var stopped = false
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)

    fun submit(command: TraverseAppCommand): TraverseSubmissionResult {
        val envelope = buildJsonObject {
            put("kind", "app_command")
            put("command", command.command)
            put("payload", Json.parseToJsonElement(command.payloadJson))
            command.sessionId?.let { put("session_id", it) }
        }
        return dispatch(envelope)
    }

    fun register(command: String, adapter: HostConnectorAdapter): HostConnectorRegistration {
        require(command.isNotBlank()) { "host connector command must be non-empty" }
        val id: Long
        synchronized(lock) {
            nextAdapterId += 1
            id = nextAdapterId
            adapters[command] = id to adapter
        }
        return HostConnectorRegistration {
            synchronized(lock) {
                if (adapters[command]?.first == id) adapters.remove(command)
            }
        }
    }

    /** Cancels adapters and deadlines; late completions are dropped. */
    fun stop() {
        val pendingDeadlines: List<TraverseTimerHandle>
        synchronized(lock) {
            stopped = true
            pendingDeadlines = deadlines.toList()
            deadlines.clear()
        }
        pendingDeadlines.forEach { it.cancel() }
        scope.cancel()
    }

    private fun dispatch(envelope: JsonObject): TraverseSubmissionResult {
        // A rejected submit's guest export returns a negative status, which the bridge client
        // surfaces as a thrown TraverseBridgeException — but the guest still wrote a real
        // response body (status "rejected", plus error) as that exception's message, not an ABI
        // failure. Recover it the same way the bridge-level replay does.
        val response = try {
            objectFrom(submitJson(envelope.toString()))
        } catch (bridgeError: TraverseBridgeException) {
            bridgeError.message?.let { tryObjectFrom(it) } ?: throw bridgeError
        }
        val sessionId = response.requiredString("session_id")
        val status = response.requiredString("status")
        if (status != "accepted") {
            return TraverseSubmissionResult(sessionId, status, response.optionalString("error"))
        }
        scheduleDeadlines(response)
        startHostConnectors(response, sessionId)
        return TraverseSubmissionResult(sessionId, status)
    }

    private fun scheduleDeadlines(response: JsonObject) {
        for (deadline in items(response, "pending_deadlines")) {
            val commandId = deadline.optionalString("command_id") ?: continue
            val sessionId = deadline.optionalString("session_id") ?: continue
            val delayMs = deadline["deadline_ms"]?.jsonPrimitive?.doubleOrNull ?: continue
            if (!delayMs.isFinite()) continue
            synchronized(lock) {
                if (stopped) return@synchronized
                deadlines.add(
                    timer.schedule(delayMs.coerceAtLeast(0.0).toLong()) {
                        terminal(
                            buildJsonObject {
                                put("kind", "deadline_fired")
                                put("command_id", commandId)
                                put("session_id", sessionId)
                            },
                        )
                    },
                )
            }
        }
    }

    private fun startHostConnectors(response: JsonObject, fallbackSessionId: String) {
        for (pending in items(response, "pending_host_connector")) {
            val commandId = pending.optionalString("command_id") ?: continue
            val sessionId = pending.optionalString("session_id") ?: fallbackSessionId
            val command = pending.optionalString("command")
            val adapter = command?.let { name -> synchronized(lock) { adapters[name]?.second } }
            if (command == null || adapter == null) {
                complete(
                    commandId,
                    sessionId,
                    HostConnectorResult("failed", """{"code":"target_incompatible"}"""),
                )
                continue
            }
            val payloadJson = (pending["payload"] ?: JsonObject(emptyMap())).toString()
            val request = HostConnectorRequest(command, commandId, sessionId, payloadJson)
            scope.launch {
                val result = try {
                    adapter(request)
                } catch (error: CancellationException) {
                    throw error
                } catch (error: Exception) {
                    // Adapters are host-supplied; report a fixed code and drop native detail.
                    HostConnectorResult("failed", """{"code":"execution_failed"}""")
                }
                complete(commandId, sessionId, result)
            }
        }
    }

    private fun complete(commandId: String, sessionId: String, result: HostConnectorResult) {
        val payload = try {
            Json.parseToJsonElement(result.payloadJson)
        } catch (error: Exception) {
            JsonObject(emptyMap())
        }
        terminal(
            buildJsonObject {
                put("kind", "host_connector_result")
                put("command_id", commandId)
                put("session_id", sessionId)
                put("result_class", result.resultClass)
                put("payload", payload)
            },
        )
    }

    private fun terminal(envelope: JsonObject) {
        val isStopped = synchronized(lock) { stopped }
        if (isStopped) return
        // Shutdown and first-terminal-wins are runtime-owned; a terminal the runtime can no
        // longer accept has no host retry path.
        runCatching { dispatch(envelope) }
    }

    private fun items(response: JsonObject, name: String): List<JsonObject> =
        (response[name] as? JsonArray)?.mapNotNull { it as? JsonObject } ?: emptyList()

    private fun objectFrom(json: String): JsonObject = try {
        Json.parseToJsonElement(json).jsonObject
    } catch (error: Exception) {
        throw TraverseBridgeException(-2, "bridge_invalid_json")
    }

    private fun tryObjectFrom(json: String): JsonObject? = try {
        Json.parseToJsonElement(json).jsonObject
    } catch (error: Exception) {
        null
    }

    private fun JsonObject.requiredString(name: String): String =
        optionalString(name) ?: throw TraverseBridgeException(-2, "bridge result is missing $name")

    private fun JsonObject.optionalString(name: String): String? {
        val value = this[name] ?: return null
        return if (value is JsonNull) null else value.jsonPrimitive.content
    }
}
