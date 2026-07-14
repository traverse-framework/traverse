package dev.traverse.embedder

import org.junit.Assert.assertEquals
import org.junit.Test

class TraverseEmbedderTest {
    @Test fun lifecycleAndSubmissionAreDeterministic() {
        val harness = InMemoryTraverseEmbedder()
        harness.initialize(TraverseBundle("assets/traverse", "sha256:test"))

        assertEquals(
            TraverseSubmissionResult("kotlin-session-1", "accepted"),
            harness.submit(TraverseSubmission("demo.workflow", "{}")),
        )
    }
}
