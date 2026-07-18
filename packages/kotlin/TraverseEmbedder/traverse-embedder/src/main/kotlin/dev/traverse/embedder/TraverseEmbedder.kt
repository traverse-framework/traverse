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

/** Traceability evidence published with a TraverseEmbedder package release. */
data class TraverseReleaseEvidence(
    val packageVersion: String,
    val runtimeWasmDigest: String,
    val conformanceVersion: String,
    val supportedHostVersions: List<String>,
) {
    init {
        require(packageVersion.isNotBlank()) { "package version is required" }
        require(runtimeWasmDigest.isNotBlank()) { "runtime WASM digest is required" }
        require(conformanceVersion.isNotBlank()) { "conformance version is required" }
        require(supportedHostVersions.isNotEmpty() && supportedHostVersions.all { it.isNotBlank() }) {
            "supported host versions are required"
        }
    }
}

/** Ordered runtime-shaped event exposed by the deterministic conformance harness. */
data class TraverseRuntimeEvent(
    val sequence: Int,
    val targetId: String,
    val status: String,
    val instanceId: String? = null,
    val eventType: String? = null,
    val sessionId: String? = null,
    val errorData: String? = null,
    val output: String? = null,
)

data class TraverseCompatibleResult(val instanceId: String?, val status: String)

/** Deterministic test double; it never evaluates application business logic. */
class InMemoryTraverseEmbedder {
    private var bundle: TraverseBundle? = null
    private var submissionSequence = 0
    private var compatibleSequence = 0
    private val events = mutableListOf<TraverseRuntimeEvent>()
    private val compatibleInstances = mutableMapOf<String, String>()
    private var targetOutput: String? = null

    fun withTargetOutput(output: String): InMemoryTraverseEmbedder = apply { targetOutput = output }

    fun initialize(bundle: TraverseBundle) {
        check(this.bundle == null) { "embedder is already initialized" }
        this.bundle = bundle
    }

    fun shutdown() {
        bundle = null
        submissionSequence = 0
        compatibleSequence = 0
        events.clear()
        compatibleInstances.clear()
    }

    fun submit(submission: TraverseSubmission): TraverseSubmissionResult {
        check(bundle != null) { "embedder is not initialized" }
        submissionSequence += 1
        val result = TraverseSubmissionResult("kotlin-session-$submissionSequence", "accepted")
        events += TraverseRuntimeEvent(submissionSequence, submission.targetId, result.status, eventType = if (targetOutput == null) null else "capability_result", sessionId = if (targetOutput == null) null else result.sessionId, output = targetOutput)
        return result
    }

    /** Returns ordered runtime-shaped events emitted after the supplied cursor. */
    fun subscribe(afterSequence: Int = 0): List<TraverseRuntimeEvent> {
        check(bundle != null) { "embedder is not initialized" }
        return events.filter { it.sequence > afterSequence }
    }

    fun compatibleStart(capabilityId: String, inputJson: String): TraverseCompatibleResult {
        check(bundle != null) { "embedder is not initialized" }
        require(capabilityId.isNotBlank()) { "capability_id is required" }
        compatibleSequence += 1
        val instanceId = "kotlin-compatible-$compatibleSequence"
        compatibleInstances[capabilityId] = instanceId
        appendCompatibleEvent(capabilityId, instanceId, "started")
        return TraverseCompatibleResult(instanceId, "started")
    }

    fun compatibleStop(capabilityId: String, instanceId: String?): TraverseCompatibleResult {
        val resolvedInstanceId = compatibleInstance(capabilityId, instanceId)
        compatibleInstances.remove(capabilityId)
        appendCompatibleEvent(capabilityId, resolvedInstanceId, "stopped")
        return TraverseCompatibleResult(resolvedInstanceId, "stopped")
    }

    fun compatibleKill(capabilityId: String, instanceId: String?): TraverseCompatibleResult {
        val resolvedInstanceId = compatibleInstance(capabilityId, instanceId)
        compatibleInstances.remove(capabilityId)
        appendCompatibleEvent(capabilityId, resolvedInstanceId, "killed")
        return TraverseCompatibleResult(resolvedInstanceId, "killed")
    }

    private fun compatibleInstance(capabilityId: String, instanceId: String?): String {
        check(bundle != null) { "embedder is not initialized" }
        val activeInstanceId = compatibleInstances[capabilityId]
        check(activeInstanceId != null && (instanceId == null || instanceId == activeInstanceId)) {
            "compatible instance is not active"
        }
        return activeInstanceId
    }

    private fun appendCompatibleEvent(capabilityId: String, instanceId: String, status: String) {
        submissionSequence += 1
        events += TraverseRuntimeEvent(submissionSequence, capabilityId, status, instanceId)
    }
}
