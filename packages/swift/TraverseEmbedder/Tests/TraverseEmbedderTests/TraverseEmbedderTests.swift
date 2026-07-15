import Foundation
import Testing
@testable import TraverseEmbedder

@Test func lifecycleAndSubmissionAreDeterministic() throws {
    let harness = InMemoryTraverseEmbedder()
    let bundle = try TraverseBundle(
        rootURL: URL(fileURLWithPath: "/tmp/traverse-bundle"),
        runtimeWasmDigest: "sha256:test"
    )
    try harness.initialize(bundle: bundle)

    #expect(try harness.submit(TraverseSubmission(targetID: "demo.workflow", inputJSON: Data("{}".utf8))) == TraverseSubmissionResult(sessionID: "swift-session-1", status: "accepted"))
    #expect(try harness.submit(TraverseSubmission(targetID: "demo.capability", inputJSON: Data("{}".utf8))) == TraverseSubmissionResult(sessionID: "swift-session-2", status: "accepted"))
    #expect(try harness.subscribe() == [
        TraverseRuntimeEvent(sequence: 1, targetID: "demo.workflow", status: "accepted"),
        TraverseRuntimeEvent(sequence: 2, targetID: "demo.capability", status: "accepted"),
    ])
    #expect(try harness.subscribe(after: 1) == [
        TraverseRuntimeEvent(sequence: 2, targetID: "demo.capability", status: "accepted"),
    ])
    harness.shutdown()
    #expect(throws: TraverseEmbedderError.notInitialized) {
        try harness.submit(TraverseSubmission(targetID: "demo.workflow", inputJSON: Data("{}".utf8)))
    }
}

@Test func incompatibleBundleIsRejectedWithoutInitializing() throws {
    let harness = InMemoryTraverseEmbedder()
    let bundle = try TraverseBundle(
        rootURL: URL(fileURLWithPath: "/tmp/traverse-bundle"),
        runtimeWasmDigest: "sha256:test",
        embedderAPIVersion: "2.0.0"
    )

    #expect(throws: TraverseEmbedderError.incompatibleBundle(
        "embedder API 2.0.0 is incompatible with 1.0.0"
    )) {
        try harness.initialize(bundle: bundle)
    }
    #expect(throws: TraverseEmbedderError.notInitialized) {
        try harness.subscribe()
    }
}

@Test func compatibleLifecycleIsDeterministicAndOrdered() throws {
    let harness = InMemoryTraverseEmbedder()
    try harness.initialize(bundle: TraverseBundle(
        rootURL: URL(fileURLWithPath: "/tmp/traverse-bundle"),
        runtimeWasmDigest: "sha256:test"
    ))

    let first = try harness.compatibleStart(capabilityID: "demo.compatible", inputJSON: Data("{}".utf8))
    #expect(first == TraverseCompatibleResult(instanceID: "swift-compatible-1", status: "started"))
    #expect(try harness.compatibleStop(capabilityID: "demo.compatible", instanceID: first.instanceID) == TraverseCompatibleResult(instanceID: "swift-compatible-1", status: "stopped"))
    #expect(throws: TraverseEmbedderError.unsupportedOperation("compatible instance is not active")) {
        try harness.compatibleKill(capabilityID: "demo.compatible", instanceID: first.instanceID)
    }

    let second = try harness.compatibleStart(capabilityID: "demo.compatible", inputJSON: Data("{}".utf8))
    #expect(try harness.compatibleKill(capabilityID: "demo.compatible", instanceID: nil) == TraverseCompatibleResult(instanceID: "swift-compatible-2", status: "killed"))
    #expect(try harness.subscribe() == [
        TraverseRuntimeEvent(sequence: 1, targetID: "demo.compatible", status: "started", instanceID: first.instanceID),
        TraverseRuntimeEvent(sequence: 2, targetID: "demo.compatible", status: "stopped", instanceID: first.instanceID),
        TraverseRuntimeEvent(sequence: 3, targetID: "demo.compatible", status: "started", instanceID: second.instanceID),
        TraverseRuntimeEvent(sequence: 4, targetID: "demo.compatible", status: "killed", instanceID: second.instanceID),
    ])
}
