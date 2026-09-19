import Foundation

/// Failure from bounded artifact staging. Codes match the Spec 137 public
/// failure codes so they compose with host-connector results.
public enum ArtifactStagingError: Error, Sendable, Equatable {
    /// The bytes are empty or exceed the caller-supplied ceiling.
    case inputLimitExceeded
    /// The ref is missing, dropped, or invalidated by shutdown.
    case unavailable

    public var code: String {
        switch self {
        case .inputLimitExceeded: "input_limit_exceeded"
        case .unavailable: "unavailable"
        }
    }
}

/// Spec 140 / Spec 138 0.2.0 generic host staging.
///
/// `stageArtifact` returns an opaque, multi-read `artifact_ref` (never a path
/// or URL). The ref stays readable until `dropRef` or `shutdown`, so
/// runtime-owned retries can re-read it. Guests never read this storage;
/// bytes only reach a capability through a runtime-mediated `readArtifact`.
public actor ArtifactStagingStore {
    private var artifacts: [String: Data] = [:]
    private var nextArtifact: UInt64 = 0

    public init() {}

    /// Stage bounded bytes and return an opaque `artifact_ref`.
    public func stageArtifact(_ bytes: Data, maxBytes: Int) throws -> String {
        guard !bytes.isEmpty, bytes.count <= maxBytes else {
            throw ArtifactStagingError.inputLimitExceeded
        }
        nextArtifact += 1
        let reference = "artifact-\(nextArtifact)"
        artifacts[reference] = bytes
        return reference
    }

    /// Runtime-mediated bounded read of an `artifact_ref`. Repeatable.
    public func readArtifact(_ reference: String, maxBytes: Int) throws -> Data {
        guard let bytes = artifacts[reference] else {
            throw ArtifactStagingError.unavailable
        }
        guard bytes.count <= maxBytes else {
            throw ArtifactStagingError.inputLimitExceeded
        }
        return bytes
    }

    /// Drop one ref.
    public func dropRef(_ reference: String) {
        artifacts.removeValue(forKey: reference)
    }

    /// Invalidate every staged ref (runtime shutdown).
    public func shutdown() {
        artifacts.removeAll()
    }
}
