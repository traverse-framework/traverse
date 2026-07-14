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
    harness.shutdown()
    #expect(throws: TraverseEmbedderError.notInitialized) {
        try harness.submit(TraverseSubmission(targetID: "demo.workflow", inputJSON: Data("{}".utf8)))
    }
}
