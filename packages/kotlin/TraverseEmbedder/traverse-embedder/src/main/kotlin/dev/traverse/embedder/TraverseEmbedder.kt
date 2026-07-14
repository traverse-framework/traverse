package dev.traverse.embedder

/** Public Kotlin/Android surface for Traverse `embedder-api/1.0.0`. */
object TraverseEmbedder {
    const val API_VERSION = "1.0.0"
}

data class TraverseBundle(val rootPath: String, val runtimeWasmDigest: String) {
    init {
        require(rootPath.isNotBlank()) { "bundle root path is required" }
        require(runtimeWasmDigest.isNotBlank()) { "runtime WASM digest is required" }
    }
}

data class TraverseSubmission(val targetId: String, val inputJson: String) {
    init { require(targetId.isNotBlank()) { "target_id is required" } }
}

data class TraverseSubmissionResult(val sessionId: String, val status: String)

/** Ordered runtime-shaped event exposed by the deterministic conformance harness. */
data class TraverseRuntimeEvent(val sequence: Int, val targetId: String, val status: String)

/** Deterministic test double; it never evaluates application business logic. */
class InMemoryTraverseEmbedder {
    private var bundle: TraverseBundle? = null
    private var submissionSequence = 0
    private val events = mutableListOf<TraverseRuntimeEvent>()

    fun initialize(bundle: TraverseBundle) {
        check(this.bundle == null) { "embedder is already initialized" }
        this.bundle = bundle
    }

    fun shutdown() {
        bundle = null
        submissionSequence = 0
        events.clear()
    }

    fun submit(submission: TraverseSubmission): TraverseSubmissionResult {
        check(bundle != null) { "embedder is not initialized" }
        submissionSequence += 1
        val result = TraverseSubmissionResult("kotlin-session-$submissionSequence", "accepted")
        events += TraverseRuntimeEvent(submissionSequence, submission.targetId, result.status)
        return result
    }

    /** Returns ordered runtime-shaped events emitted after the supplied cursor. */
    fun subscribe(afterSequence: Int = 0): List<TraverseRuntimeEvent> {
        check(bundle != null) { "embedder is not initialized" }
        return events.filter { it.sequence > afterSequence }
    }

    fun compatibleStart(capabilityId: String, inputJson: String) =
        submit(TraverseSubmission(capabilityId, inputJson))

    fun compatibleStop(capabilityId: String, instanceId: String?) {
        check(bundle != null) { "embedder is not initialized" }
    }

    fun compatibleKill(capabilityId: String, instanceId: String?) {
        check(bundle != null) { "embedder is not initialized" }
    }
}
