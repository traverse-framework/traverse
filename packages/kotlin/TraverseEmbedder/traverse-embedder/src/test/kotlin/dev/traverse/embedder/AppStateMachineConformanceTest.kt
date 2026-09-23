package dev.traverse.embedder

import java.io.File
import java.security.MessageDigest
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.doubleOrNull
import kotlinx.serialization.json.int
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Cross-host ordered-event conformance (Spec 139 / Spec 140 FR-013, #1500/#1502). Runs
 * `fixtures/cross-host/app-state-machine-events-v1` against the real `runtime.wasm` and compares
 * each step with `golden.json`, which the Rust reference generated. Skipped unless
 * `TRAVERSE_NATIVE_ARTIFACT_ROOT` points at a directory containing `runtime/runtime.wasm`, exactly
 * like the other real-artifact tests.
 */
class AppStateMachineConformanceTest {
    private val fixtureDirectory = "fixtures/cross-host/app-state-machine-events-v1"

    /** Normalizes runtime-assigned ids to `$S<n>` / `$C<n>` by first appearance. */
    private class Placeholders {
        val sessions = mutableListOf<String>()
        val commands = mutableListOf<String>()

        private fun name(list: MutableList<String>, prefix: String, id: String): String {
            var index = list.indexOf(id)
            if (index < 0) {
                list.add(id)
                index = list.size - 1
            }
            return "$prefix${index + 1}"
        }

        fun normalize(node: JsonElement?): JsonElement = when (node) {
            null -> JsonNull
            is JsonObject -> JsonObject(
                node.mapValues { (key, value) ->
                    when {
                        key == "session_id" && value is JsonPrimitive && value.isString ->
                            JsonPrimitive(name(sessions, "\$S", value.content))
                        key == "command_id" && value is JsonPrimitive && value.isString ->
                            JsonPrimitive(name(commands, "\$C", value.content))
                        else -> normalize(value)
                    }
                },
            )
            is JsonArray -> JsonArray(node.map { normalize(it) })
            else -> node
        }

        companion object {
            fun resolve(list: List<String>, placeholder: String, prefix: String): String =
                list[placeholder.removePrefix(prefix).toInt() - 1]
        }
    }

    private fun artifactRoot(): String? = System.getenv("TRAVERSE_NATIVE_ARTIFACT_ROOT")?.takeIf { it.isNotBlank() }

    private fun fixturePath(file: String): File {
        var directory: File? = File(".").absoluteFile
        while (directory != null) {
            val candidate = File(directory, "$fixtureDirectory/$file")
            if (candidate.isFile) return candidate
            directory = directory.parentFile
        }
        throw java.io.FileNotFoundException("$fixtureDirectory/$file not found above ${File(".").absolutePath}")
    }

    private fun load(file: String): JsonElement = Json.parseToJsonElement(fixturePath(file).readText())

    private fun prepare(root: String, fixture: JsonElement): Pair<TraverseBundle, ByteArray> {
        val runtime = File(root, "runtime/runtime.wasm").readBytes()
        val digest = "sha256:" + MessageDigest.getInstance("SHA-256")
            .digest(runtime).joinToString("") { "%02x".format(it) }
        val header = fixture.jsonObject["init_header"]!!.toString().toByteArray(Charsets.UTF_8)
        val init = ByteArray(4 + header.size)
        init[0] = (header.size and 0xFF).toByte()
        init[1] = ((header.size shr 8) and 0xFF).toByte()
        init[2] = ((header.size shr 16) and 0xFF).toByte()
        init[3] = ((header.size shr 24) and 0xFF).toByte()
        header.copyInto(init, 4)
        return TraverseBundle(root, digest) to init
    }

    private fun newClient(bundle: TraverseBundle): ChicoryBridgeClient =
        ChicoryBridgeClient(ChicoryRuntimeBridge(bundle, maximumInstructionsPerCall = 50_000_000L))

    private fun tryParse(text: String): JsonElement? = try {
        Json.parseToJsonElement(text)
    } catch (error: Exception) {
        null
    }

    /** Runs one scenario at the bridge-client level; returns per-step transcripts. */
    private fun runScenario(client: ChicoryBridgeClient, init: ByteArray, scenario: JsonObject): JsonArray {
        assertEquals("ready", Json.parseToJsonElement(client.initialize(init)).jsonObject["status"]!!.jsonPrimitive.content)
        val names = Placeholders()
        val pending = mutableListOf<Pair<String, String>>()
        val transcript = mutableListOf<JsonElement>()

        val steps = scenario["steps"]!!.jsonArray
        for ((index, stepElement) in steps.withIndex()) {
            val step = stepElement.jsonObject
            val (kind, spec) = step.entries.single().let { it.key to it.value.jsonObject }

            fun targetWait(): Pair<String, String> {
                val placeholder = spec["command"]
                return if (placeholder != null && placeholder !is JsonNull) {
                    val resolved = Placeholders.resolve(names.commands, placeholder.jsonPrimitive.content, "\$C")
                    pending.first { it.second == resolved }
                } else {
                    pending.last()
                }
            }

            val request: JsonObject = when (kind) {
                "submit" -> buildJsonObject {
                    put("kind", "app_command")
                    put("command", spec["command"]!!)
                    put("payload", spec["payload"]!!)
                    val session = spec["session"]
                    if (session != null && session !is JsonNull) {
                        put("session_id", Placeholders.resolve(names.sessions, session.jsonPrimitive.content, "\$S"))
                    }
                }
                "complete" -> {
                    val (sessionId, commandId) = targetWait()
                    buildJsonObject {
                        put("kind", "host_connector_result")
                        put("command_id", commandId)
                        put("session_id", sessionId)
                        put("result_class", spec["result_class"]!!)
                        put("payload", spec["payload"]!!)
                    }
                }
                "fire_deadline" -> {
                    val (sessionId, commandId) = targetWait()
                    buildJsonObject {
                        put("kind", "deadline_fired")
                        put("command_id", commandId)
                        put("session_id", sessionId)
                    }
                }
                else -> throw IllegalStateException("unknown step kind $kind")
            }

            val entry = mutableMapOf<String, JsonElement>("step" to JsonPrimitive(index), "kind" to JsonPrimitive(kind))
            var response: JsonElement?
            try {
                response = Json.parseToJsonElement(client.submit(request.toString()))
                entry["guest_status"] = JsonPrimitive(0)
            } catch (error: TraverseBridgeException) {
                // A rejected submit throws; the guest still wrote the response body as the message.
                entry["guest_status"] = JsonPrimitive(error.status)
                response = tryParse(error.message ?: "")
            }
            for (item in (response?.jsonObject?.get("pending_host_connector") as? JsonArray).orEmpty()) {
                val obj = item.jsonObject
                pending.add(obj["session_id"]!!.jsonPrimitive.content to obj["command_id"]!!.jsonPrimitive.content)
            }
            entry["response"] = names.normalize(response)
            val events = mutableListOf<JsonElement>()
            while (true) {
                val bytes = client.nextEvent() ?: break
                events.add(Json.parseToJsonElement(bytes))
            }
            entry["events"] = names.normalize(JsonArray(events))
            transcript.add(JsonObject(entry))
        }
        return JsonArray(transcript)
    }

    @Test
    fun kotlinHostReproducesTheGoldenOrderedEventLog() {
        val root = artifactRoot() ?: return
        val fixture = load("fixture.json").jsonObject
        val golden = load("golden.json").jsonObject["scenarios"]!!.jsonObject
        val (bundle, init) = prepare(root, fixture)
        val divergences = mutableListOf<String>()

        for (scenarioElement in fixture["scenarios"]!!.jsonArray) {
            val scenario = scenarioElement.jsonObject
            val id = scenario["id"]!!.jsonPrimitive.content
            val actual = runScenario(newClient(bundle), init, scenario)
            val want = golden[id]!!.jsonArray
            if (actual.size != want.size) {
                divergences.add("scenario $id: ${actual.size} steps, expected ${want.size}")
                continue
            }
            for (index in want.indices) {
                val actualStep = actual[index].jsonObject
                val wantStep = want[index].jsonObject
                for (field in listOf("kind", "guest_status", "response")) {
                    if (actualStep[field] != wantStep[field]) {
                        divergences.add(
                            "scenario $id step $index $field: expected ${wantStep[field]}, got ${actualStep[field]}",
                        )
                    }
                }
                val got = actualStep["events"]!!.jsonArray
                val wanted = wantStep["events"]!!.jsonArray
                for (eventIndex in 0 until maxOf(got.size, wanted.size)) {
                    val g = got.getOrNull(eventIndex)
                    val w = wanted.getOrNull(eventIndex)
                    if (g != w) {
                        divergences.add("scenario $id step $index event $eventIndex: expected $w, got $g")
                    }
                }
            }
        }
        assertTrue(divergences.joinToString("\n"), divergences.isEmpty())
    }

    // ---- Public API level ----

    private class ScriptedTimer : TraverseTimer {
        private val callbacks = mutableListOf<() -> Unit>()
        private val lock = Any()

        override fun schedule(delayMs: Long, callback: () -> Unit): TraverseTimerHandle {
            synchronized(lock) { callbacks.add(callback) }
            return TraverseTimerHandle {}
        }

        fun fireLatest(): Boolean {
            val callback = synchronized(lock) { callbacks.lastOrNull() } ?: return false
            callback()
            return true
        }
    }

    /**
     * Raw terminal injection (an explicit `command` target) cannot be expressed through adapters
     * and the timer port, so those scenarios run at the bridge level only.
     */
    private fun expressible(scenario: JsonObject): Boolean = scenario["steps"]!!.jsonArray.all { step ->
        val obj = step.jsonObject
        obj["complete"]?.jsonObject?.get("command") == null &&
            obj["fire_deadline"]?.jsonObject?.get("command") == null
    }

    private data class WaitAction(val resultClass: String, val payload: String)

    @Test
    fun kotlinPublicSubscribeDeliversTheGoldenAppEventsInOrder(): Unit = runBlocking {
        val root = artifactRoot() ?: return@runBlocking
        val fixture = load("fixture.json").jsonObject
        val golden = load("golden.json").jsonObject["scenarios"]!!.jsonObject
        val (bundle, init) = prepare(root, fixture)
        val hostCommands = fixture["init_header"]!!.jsonObject["state_machine"]!!.jsonObject["states"]!!.jsonArray
            .mapNotNull { state ->
                state.jsonObject["invoke"]?.jsonObject?.get("host_connector")?.jsonPrimitive?.contentOrNull
            }.distinct()
        val divergences = mutableListOf<String>()
        var exercised = 0

        for (scenarioElement in fixture["scenarios"]!!.jsonArray) {
            val scenario = scenarioElement.jsonObject
            if (!expressible(scenario)) continue
            exercised++
            val id = scenario["id"]!!.jsonPrimitive.content
            val steps = scenario["steps"]!!.jsonArray
            val goldenSteps = golden[id]!!.jsonArray

            // One gate per staged wait, in step order. An adapter holds its wait until the test
            // reaches the matching `complete` step, so a command submitted during the wait is
            // rejected deterministically instead of racing an eagerly completing adapter. A
            // `fire_deadline` step never opens its gate: the deadline wins and shutdown cancels it.
            val gates = steps.filter { it.jsonObject["complete"] != null || it.jsonObject["fire_deadline"] != null }
                .map { CompletableDeferred<WaitAction>() }
            val waitsStarted = AtomicInteger(0)
            var gateIndex = 0
            val timer = ScriptedTimer()
            val client = newClient(bundle)
            client.initialize(init)
            val embedder = RuntimeTraverseEmbedder(client, timer)
            for (command in hostCommands) {
                embedder.registerHostConnectorAdapter(command) { _ ->
                    val gate = gates[waitsStarted.getAndIncrement()]
                    val action = withTimeout(10_000) { gate.await() }
                    HostConnectorResult(action.resultClass, action.payload)
                }
            }

            val names = Placeholders()
            var sessionId: String? = null
            val received = mutableListOf<TraverseRuntimeEvent>()
            var expectedTotal = 0
            for ((index, stepElement) in steps.withIndex()) {
                val step = stepElement.jsonObject
                expectedTotal += goldenSteps[index].jsonObject["events"]!!.jsonArray.size
                val submitSpec = step["submit"]?.jsonObject
                val completeSpec = step["complete"]?.jsonObject
                when {
                    submitSpec != null -> {
                        val session = submitSpec["session"]
                        val result = embedder.submit(
                            TraverseAppCommand(
                                submitSpec["command"]!!.jsonPrimitive.content,
                                submitSpec["payload"]!!.toString(),
                                if (session == null || session is JsonNull) null else sessionId,
                            ),
                        )
                        if (sessionId == null) sessionId = result.sessionId
                        val goldenResponse = goldenSteps[index].jsonObject["response"]!!.jsonObject
                        if (goldenResponse["status"]!!.jsonPrimitive.content == "rejected") {
                            assertEquals("rejected", result.status)
                            assertEquals(goldenResponse["error"]!!.jsonPrimitive.content, result.error)
                        } else {
                            assertEquals("accepted", result.status)
                        }
                    }
                    completeSpec != null -> {
                        gates[gateIndex++].complete(
                            WaitAction(
                                completeSpec["result_class"]!!.jsonPrimitive.content,
                                completeSpec["payload"]!!.toString(),
                            ),
                        )
                    }
                    step["fire_deadline"] != null -> {
                        gateIndex++
                        assertTrue("$id step $index: no deadline was registered", timer.fireLatest())
                    }
                }
                var attempt = 0
                while (attempt < 500 && received.size < expectedTotal) {
                    received.addAll(embedder.subscribe())
                    if (received.size < expectedTotal) delay(10)
                    attempt++
                }
                // Adapters complete asynchronously, so a step may already include later events.
                if (received.size < expectedTotal) {
                    divergences.add("scenario $id step $index: only ${received.size} of $expectedTotal events arrived")
                }
            }
            delay(50) // let any surplus event arrive before checking the exact total
            received.addAll(embedder.subscribe())
            if (received.size != expectedTotal) {
                divergences.add("scenario $id: ${received.size} events, expected $expectedTotal")
            }
            embedder.shutdown()

            val wanted = goldenSteps.flatMap { it.jsonObject["events"]!!.jsonArray }
            for (eventIndex in 0 until minOf(received.size, wanted.size)) {
                val got = received[eventIndex]
                val actual = names.normalize(
                    buildJsonObject {
                        put("type", got.eventType)
                        put("session_id", got.sessionId)
                        put("data", Json.parseToJsonElement(got.output ?: "{}"))
                    },
                )
                if (actual != wanted[eventIndex]) {
                    divergences.add("scenario $id event $eventIndex: expected ${wanted[eventIndex]}, got $actual")
                }
            }
            assertEquals((1..received.size).toList(), received.map { it.sequence })
        }
        assertTrue("expected the adapter-expressible scenarios to run", exercised >= 7)
        assertTrue(divergences.joinToString("\n"), divergences.isEmpty())
    }
}
