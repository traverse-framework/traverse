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

/** Deterministic test double; it never evaluates application business logic. */
class InMemoryTraverseEmbedder {
    private var bundle: TraverseBundle? = null
    private var submissionSequence = 0

    fun initialize(bundle: TraverseBundle) {
        check(this.bundle == null) { "embedder is already initialized" }
        this.bundle = bundle
    }

    fun shutdown() {
        bundle = null
        submissionSequence = 0
    }

    fun submit(submission: TraverseSubmission): TraverseSubmissionResult {
        check(bundle != null) { "embedder is not initialized" }
        submissionSequence += 1
        return TraverseSubmissionResult("kotlin-session-$submissionSequence", "accepted")
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
