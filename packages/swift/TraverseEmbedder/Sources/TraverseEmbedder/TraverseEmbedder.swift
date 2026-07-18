import Foundation

/// Public Swift surface for `embedder-api/1.0.0`.
public enum TraverseEmbedder {
    public static let apiVersion = "1.0.0"
}

public struct TraverseBundle: Sendable, Equatable {
    public let rootURL: URL
    public let runtimeWasmDigest: String
    public let embedderAPIVersion: String

    public init(
        rootURL: URL,
        runtimeWasmDigest: String,
        embedderAPIVersion: String = TraverseEmbedder.apiVersion
    ) throws {
        guard !runtimeWasmDigest.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw TraverseEmbedderError.incompatibleBundle("runtime WASM digest is required")
        }
        self.rootURL = rootURL
        self.runtimeWasmDigest = runtimeWasmDigest
        self.embedderAPIVersion = embedderAPIVersion
    }
}

public enum TraverseEmbedderError: Error, Equatable, Sendable {
    case incompatibleBundle(String)
    case invalidReleaseEvidence(String)
    case notInitialized
    case alreadyInitialized
    case unsupportedOperation(String)
}

/// Traceability evidence published with a TraverseEmbedder package release.
public struct TraverseReleaseEvidence: Sendable, Equatable {
    public let packageVersion: String
    public let runtimeWasmDigest: String
    public let conformanceVersion: String
    public let supportedHostVersions: [String]

    public init(
        packageVersion: String,
        runtimeWasmDigest: String,
        conformanceVersion: String = TraverseEmbedder.apiVersion,
        supportedHostVersions: [String]
    ) throws {
        guard !packageVersion.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw TraverseEmbedderError.invalidReleaseEvidence("package version is required")
        }
        guard !runtimeWasmDigest.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw TraverseEmbedderError.invalidReleaseEvidence("runtime WASM digest is required")
        }
        guard !conformanceVersion.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw TraverseEmbedderError.invalidReleaseEvidence("conformance version is required")
        }
        guard !supportedHostVersions.isEmpty,
              supportedHostVersions.allSatisfy({ !$0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }) else {
            throw TraverseEmbedderError.invalidReleaseEvidence("supported host versions are required")
        }
        self.packageVersion = packageVersion
        self.runtimeWasmDigest = runtimeWasmDigest
        self.conformanceVersion = conformanceVersion
        self.supportedHostVersions = supportedHostVersions
    }
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

/// Ordered runtime-shaped event exposed by the deterministic conformance harness.
public struct TraverseRuntimeEvent: Sendable, Equatable {
    public let sequence: Int
    public let targetID: String
    public let status: String
    public let instanceID: String?
    public let eventType: String?
    public let sessionID: String?
    public let errorData: Data?
    public let output: Data?

    public init(sequence: Int, targetID: String, status: String, instanceID: String? = nil, eventType: String? = nil, sessionID: String? = nil, errorData: Data? = nil, output: Data? = nil) {
        self.sequence = sequence
        self.targetID = targetID
        self.status = status
        self.instanceID = instanceID
        self.eventType = eventType
        self.sessionID = sessionID
        self.errorData = errorData
        self.output = output
    }
}

public struct TraverseCompatibleResult: Sendable, Equatable {
    public let instanceID: String?
    public let status: String

    public init(instanceID: String?, status: String) {
        self.instanceID = instanceID
        self.status = status
    }
}

/// Deterministic conformance harness for applications and package tests.
/// It deliberately does not execute application business logic.
public final class InMemoryTraverseEmbedder: @unchecked Sendable {
    private var bundle: TraverseBundle?
    private var submissionSequence = 0
    private var compatibleSequence = 0
    private var events: [TraverseRuntimeEvent] = []
    private var compatibleInstances: [String: String] = [:]
    private var targetOutput: Data?

    public init() {}

    public func withTargetOutput(_ output: Data) -> InMemoryTraverseEmbedder {
        targetOutput = output
        return self
    }

    public func initialize(bundle: TraverseBundle) throws {
        guard self.bundle == nil else { throw TraverseEmbedderError.alreadyInitialized }
        guard bundle.embedderAPIVersion == TraverseEmbedder.apiVersion else {
            throw TraverseEmbedderError.incompatibleBundle(
                "embedder API \(bundle.embedderAPIVersion) is incompatible with \(TraverseEmbedder.apiVersion)"
            )
        }
        self.bundle = bundle
    }

    public func shutdown() {
        bundle = nil
        submissionSequence = 0
        compatibleSequence = 0
        events = []
        compatibleInstances = [:]
    }

    public func submit(_ submission: TraverseSubmission) throws -> TraverseSubmissionResult {
        guard bundle != nil else { throw TraverseEmbedderError.notInitialized }
        submissionSequence += 1
        let result = TraverseSubmissionResult(
            sessionID: "swift-session-\(submissionSequence)",
            status: "accepted"
        )
        events.append(
            TraverseRuntimeEvent(
                sequence: submissionSequence,
                targetID: submission.targetID,
                status: result.status,
                eventType: targetOutput == nil ? nil : "capability_result",
                sessionID: targetOutput == nil ? nil : result.sessionID,
                output: targetOutput
            )
        )
        return result
    }

    /// Returns the ordered runtime-shaped events emitted after a sequence cursor.
    public func subscribe(after sequence: Int = 0) throws -> [TraverseRuntimeEvent] {
        guard bundle != nil else { throw TraverseEmbedderError.notInitialized }
        return events.filter { $0.sequence > sequence }
    }

    public func compatibleStart(capabilityID: String, inputJSON: Data) throws -> TraverseCompatibleResult {
        guard bundle != nil else { throw TraverseEmbedderError.notInitialized }
        guard !capabilityID.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw TraverseEmbedderError.unsupportedOperation("capability_id is required")
        }
        compatibleSequence += 1
        let instanceID = "swift-compatible-\(compatibleSequence)"
        compatibleInstances[capabilityID] = instanceID
        appendCompatibleEvent(capabilityID: capabilityID, instanceID: instanceID, status: "started")
        return TraverseCompatibleResult(instanceID: instanceID, status: "started")
    }

    public func compatibleStop(capabilityID: String, instanceID: String?) throws -> TraverseCompatibleResult {
        let resolvedInstanceID = try compatibleInstance(capabilityID: capabilityID, instanceID: instanceID)
        compatibleInstances.removeValue(forKey: capabilityID)
        appendCompatibleEvent(capabilityID: capabilityID, instanceID: resolvedInstanceID, status: "stopped")
        return TraverseCompatibleResult(instanceID: resolvedInstanceID, status: "stopped")
    }

    public func compatibleKill(capabilityID: String, instanceID: String?) throws -> TraverseCompatibleResult {
        let resolvedInstanceID = try compatibleInstance(capabilityID: capabilityID, instanceID: instanceID)
        compatibleInstances.removeValue(forKey: capabilityID)
        appendCompatibleEvent(capabilityID: capabilityID, instanceID: resolvedInstanceID, status: "killed")
        return TraverseCompatibleResult(instanceID: resolvedInstanceID, status: "killed")
    }

    private func compatibleInstance(capabilityID: String, instanceID: String?) throws -> String {
        guard bundle != nil else { throw TraverseEmbedderError.notInitialized }
        guard let activeInstanceID = compatibleInstances[capabilityID],
              instanceID == nil || instanceID == activeInstanceID else {
            throw TraverseEmbedderError.unsupportedOperation("compatible instance is not active")
        }
        return activeInstanceID
    }

    private func appendCompatibleEvent(capabilityID: String, instanceID: String, status: String) {
        submissionSequence += 1
        events.append(
            TraverseRuntimeEvent(
                sequence: submissionSequence,
                targetID: capabilityID,
                status: status,
                instanceID: instanceID
            )
        )
    }
}
