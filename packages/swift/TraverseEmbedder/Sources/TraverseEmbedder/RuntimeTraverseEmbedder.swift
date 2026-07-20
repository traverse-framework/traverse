import Foundation

/// Typed public embedder backed exclusively by runtime-owned bridge results.
public final class RuntimeTraverseEmbedder: @unchecked Sendable {
    private let client: any TraverseBridgeClient

    public convenience init(bundle: TraverseBundle) throws {
        try self.init(client: WasmiHostBridgeClient(bundle: bundle))
    }

    public init(client: any TraverseBridgeClient) {
        self.client = client
    }

    public func initialize(configJSON: Data) throws -> Data {
        try client.initialize(configJSON: configJSON)
    }

    public func submit(_ submission: TraverseSubmission) throws -> TraverseSubmissionResult {
        let request = try encode([
            "target_id": submission.targetID,
            "input": try JSONSerialization.jsonObject(with: submission.inputJSON, options: .fragmentsAllowed),
        ])
        let result = try object(try client.submit(requestJSON: request))
        return TraverseSubmissionResult(
            sessionID: try requiredString("session_id", in: result),
            status: try requiredString("status", in: result)
        )
    }

    public func subscribe() throws -> [TraverseRuntimeEvent] {
        var events: [TraverseRuntimeEvent] = []
        while let bytes = try client.nextEvent() {
            let event = try object(bytes)
            events.append(TraverseRuntimeEvent(
                sequence: try requiredInt("sequence", in: event),
                targetID: try requiredString("target_id", in: event),
                status: try requiredString("status", in: event),
                instanceID: optionalString("instance_id", in: event)
            ))
        }
        return events
    }

    public func cancel(sessionID: String) throws -> Data {
        try client.cancel(requestJSON: encode(["session_id": sessionID]))
    }

    public func compatibleStart(capabilityID: String, inputJSON: Data) throws -> TraverseCompatibleResult {
        let request = try encode([
            "capability_id": capabilityID,
            "input": try JSONSerialization.jsonObject(with: inputJSON, options: .fragmentsAllowed),
        ])
        return try compatibleResult(client.compatibleStart(requestJSON: request))
    }

    public func compatibleStop(capabilityID: String, instanceID: String?) throws -> TraverseCompatibleResult {
        try compatibleResult(client.compatibleStop(requestJSON: encode([
            "capability_id": capabilityID,
            "instance_id": (instanceID as Any?) ?? NSNull(),
        ])))
    }

    public func compatibleKill(capabilityID: String, instanceID: String?) throws -> TraverseCompatibleResult {
        try compatibleResult(client.compatibleKill(requestJSON: encode([
            "capability_id": capabilityID,
            "instance_id": (instanceID as Any?) ?? NSNull(),
        ])))
    }

    public func shutdown() throws -> Data { try client.shutdown() }

    private func compatibleResult(_ bytes: Data) throws -> TraverseCompatibleResult {
        let result = try object(bytes)
        return TraverseCompatibleResult(
            instanceID: optionalString("instance_id", in: result),
            status: try requiredString("status", in: result)
        )
    }

    private func encode(_ value: [String: Any]) throws -> Data {
        do {
            return try JSONSerialization.data(withJSONObject: value, options: [.sortedKeys])
        } catch {
            throw TraverseBridgeError(status: -2, message: "bridge_invalid_json")
        }
    }

    private func object(_ data: Data) throws -> [String: Any] {
        guard let value = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            throw TraverseBridgeError(status: -2, message: "bridge_invalid_json")
        }
        return value
    }

    private func requiredString(_ name: String, in value: [String: Any]) throws -> String {
        guard let result = value[name] as? String else {
            throw TraverseBridgeError(status: -2, message: "bridge result is missing \(name)")
        }
        return result
    }

    private func requiredInt(_ name: String, in value: [String: Any]) throws -> Int {
        guard let result = value[name] as? Int else {
            throw TraverseBridgeError(status: -2, message: "bridge result is missing \(name)")
        }
        return result
    }

    private func optionalString(_ name: String, in value: [String: Any]) -> String? {
        value[name] as? String
    }
}
