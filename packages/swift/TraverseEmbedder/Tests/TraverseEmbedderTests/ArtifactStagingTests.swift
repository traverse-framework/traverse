import Foundation
import Testing
@testable import TraverseEmbedder

@Test func artifactRefsAreMultiReadBoundedAndOpaque() async throws {
    let store = ArtifactStagingStore()
    await #expect(throws: ArtifactStagingError.inputLimitExceeded) {
        _ = try await store.stageArtifact(Data(), maxBytes: 8)
    }
    await #expect(throws: ArtifactStagingError.inputLimitExceeded) {
        _ = try await store.stageArtifact(Data([1, 2, 3, 4, 5, 6]), maxBytes: 4)
    }
    let reference = try await store.stageArtifact(Data([1, 2, 3]), maxBytes: 8)
    #expect(reference == "artifact-1")
    #expect(!reference.contains("/") && !reference.contains(":"))
    // Multi-read: runtime-owned retries re-read the same ref.
    #expect(try await store.readArtifact(reference, maxBytes: 8) == Data([1, 2, 3]))
    #expect(try await store.readArtifact(reference, maxBytes: 8) == Data([1, 2, 3]))
    await #expect(throws: ArtifactStagingError.inputLimitExceeded) {
        _ = try await store.readArtifact(reference, maxBytes: 2)
    }
    await #expect(throws: ArtifactStagingError.unavailable) {
        _ = try await store.readArtifact("artifact-9", maxBytes: 8)
    }
    await store.dropRef(reference)
    await #expect(throws: ArtifactStagingError.unavailable) {
        _ = try await store.readArtifact(reference, maxBytes: 8)
    }
    #expect(ArtifactStagingError.inputLimitExceeded.code == "input_limit_exceeded")
    #expect(ArtifactStagingError.unavailable.code == "unavailable")
}

@Test func artifactShutdownInvalidatesEveryRef() async throws {
    let store = ArtifactStagingStore()
    let first = try await store.stageArtifact(Data([1]), maxBytes: 8)
    let second = try await store.stageArtifact(Data([2]), maxBytes: 8)
    await store.shutdown()
    for reference in [first, second] {
        await #expect(throws: ArtifactStagingError.unavailable) {
            _ = try await store.readArtifact(reference, maxBytes: 8)
        }
    }
}
