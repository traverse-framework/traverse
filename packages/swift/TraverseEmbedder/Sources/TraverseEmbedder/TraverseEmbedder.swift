import Foundation

/// Public Swift surface for `embedder-api/1.0.0`.
public enum TraverseEmbedder {
    public static let apiVersion = "1.0.0"
}

public struct TraverseBundle: Sendable, Equatable {
    public let rootURL: URL
    public let runtimeWasmDigest: String

    public init(rootURL: URL, runtimeWasmDigest: String) throws {
        guard !runtimeWasmDigest.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw TraverseEmbedderError.incompatibleBundle("runtime WASM digest is required")
        }
        self.rootURL = rootURL
        self.runtimeWasmDigest = runtimeWasmDigest
    }
}

public enum TraverseEmbedderError: Error, Equatable, Sendable {
    case incompatibleBundle(String)
    case notInitialized
    case alreadyInitialized
    case unsupportedOperation(String)
}

public struct TraverseSubmission: Sendable, Equatable {
    public let targetID: String
    public let inputJSON: Data

    public init(targetID: String, inputJSON: Data) throws {
        guard !targetID.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw TraverseEmbedderError.unsupportedOperation("target_id is required")
        }
        self.targetID = targetID
        self.inputJSON = inputJSON
    }
}

public struct TraverseSubmissionResult: Sendable, Equatable {
    public let sessionID: String
    public let status: String

    public init(sessionID: String, status: String) {
        self.sessionID = sessionID
        self.status = status
    }
}

/// Deterministic conformance harness for applications and package tests.
/// It deliberately does not execute application business logic.
public final class InMemoryTraverseEmbedder: @unchecked Sendable {
    private var bundle: TraverseBundle?
    private var submissionSequence = 0

    public init() {}

    public func initialize(bundle: TraverseBundle) throws {
        guard self.bundle == nil else { throw TraverseEmbedderError.alreadyInitialized }
        self.bundle = bundle
    }

    public func shutdown() {
        bundle = nil
        submissionSequence = 0
    }

    public func submit(_ submission: TraverseSubmission) throws -> TraverseSubmissionResult {
        guard bundle != nil else { throw TraverseEmbedderError.notInitialized }
        submissionSequence += 1
        return TraverseSubmissionResult(
            sessionID: "swift-session-\(submissionSequence)",
            status: "accepted"
        )
    }

    public func compatibleStart(capabilityID: String, inputJSON: Data) throws -> TraverseSubmissionResult {
        try submit(TraverseSubmission(targetID: capabilityID, inputJSON: inputJSON))
    }

    public func compatibleStop(capabilityID: String, instanceID: String?) throws {
        guard bundle != nil else { throw TraverseEmbedderError.notInitialized }
    }

    public func compatibleKill(capabilityID: String, instanceID: String?) throws {
        guard bundle != nil else { throw TraverseEmbedderError.notInitialized }
    }
}
