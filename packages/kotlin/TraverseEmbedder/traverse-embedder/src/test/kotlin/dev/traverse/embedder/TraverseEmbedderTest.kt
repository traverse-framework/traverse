package dev.traverse.embedder

import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
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

    @Test fun compatibleLifecycleIsDeterministicAndOrdered() {
        val harness = InMemoryTraverseEmbedder()
        harness.initialize(TraverseBundle("assets/traverse", "sha256:test"))

        val first = harness.compatibleStart("demo.compatible", "{}")
        assertEquals(TraverseCompatibleResult("kotlin-compatible-1", "started"), first)
        assertEquals(
            TraverseCompatibleResult("kotlin-compatible-1", "stopped"),
            harness.compatibleStop("demo.compatible", first.instanceId),
        )
        assertThrows(IllegalStateException::class.java) {
            harness.compatibleKill("demo.compatible", first.instanceId)
        }

        val second = harness.compatibleStart("demo.compatible", "{}")
        assertEquals(
            TraverseCompatibleResult("kotlin-compatible-2", "killed"),
            harness.compatibleKill("demo.compatible", null),
        )
        assertEquals(
            listOf(
                TraverseRuntimeEvent(1, "demo.compatible", "started", first.instanceId),
                TraverseRuntimeEvent(2, "demo.compatible", "stopped", first.instanceId),
                TraverseRuntimeEvent(3, "demo.compatible", "started", second.instanceId),
                TraverseRuntimeEvent(4, "demo.compatible", "killed", second.instanceId),
            ),
            harness.subscribe(),
        )
    }
}
