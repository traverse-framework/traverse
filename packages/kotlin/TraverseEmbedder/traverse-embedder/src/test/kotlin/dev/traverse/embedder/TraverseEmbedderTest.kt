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
        assertEquals(
            TraverseSubmissionResult("kotlin-session-2", "accepted"),
            harness.submit(TraverseSubmission("demo.capability", "{}")),
        )
        assertEquals(
            listOf(
                TraverseRuntimeEvent(1, "demo.workflow", "accepted"),
                TraverseRuntimeEvent(2, "demo.capability", "accepted"),
            ),
            harness.subscribe(),
        )
        assertEquals(
            listOf(TraverseRuntimeEvent(2, "demo.capability", "accepted")),
            harness.subscribe(afterSequence = 1),
        )
    }
}
