import Foundation
import os

/// Typed public embedder backed exclusively by runtime-owned bridge results.
public final class RuntimeTraverseEmbedder: @unchecked Sendable {
    private let client: any TraverseBridgeClient
    private let appCommands: AppCommandCoordinator
    private let eventSequence = OSAllocatedUnfairLock(initialState: 0)

    public convenience init(bundle: TraverseBundle) throws {
        try self.init(client: WasmiHostBridgeClient(bundle: bundle))
    }

    public init(client: any TraverseBridgeClient, timer: any TraverseTimer = SystemTraverseTimer()) {
        self.client = client
        self.appCommands = AppCommandCoordinator(
            submit: { try client.submit(requestJSON: $0) }, timer: timer)
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

    /// Spec 139 `app_command` submit. The state machine runs in `runtime.wasm`; registered
    /// adapters and the timer port complete host-connector waits.
    public func submit(_ command: TraverseAppCommand) throws -> TraverseSubmissionResult {
        try appCommands.submit(command)
    }

    /// Registers the host authority for a manifest command (Spec 140 WIT semantics).
    /// Removing the registration only applies while it is still the registered adapter.
    public func registerHostConnectorAdapter(
        command: String,
        adapter: @escaping HostConnectorAdapter
    ) throws -> HostConnectorRegistration {
        try appCommands.register(command: command, adapter: adapter)
    }

    /// Drains ordered runtime events. Legacy bridge events (`sequence`, `target_id`, `status`) are
    /// parsed as before. Spec 139 app lifecycle events (`type`, `session_id`, `data`) are mapped
    /// to `eventType`, `sessionID`, and `output` (also `errorData` for `error`), numbered in
    /// arrival order, so state-machine events are observable on every embedder (Spec 139 FR-004).
    public func subscribe() throws -> [TraverseRuntimeEvent] {
        var events: [TraverseRuntimeEvent] = []
        while let bytes = try client.nextEvent() {
            let event = try object(bytes)
            if event["sequence"] == nil, let type = event["type"] as? String {
                events.append(try lifecycleEvent(type: type, event: event))
                continue
            }
            events.append(TraverseRuntimeEvent(
                sequence: try requiredInt("sequence", in: event),
                targetID: try requiredString("target_id", in: event),
                status: try requiredString("status", in: event),
                instanceID: optionalString("instance_id", in: event)
            ))
        }
        return events
    }

    private static let lifecycleEventTypes: Set<String> = [
        "state_changed", "capability_invoked", "capability_result", "capability_event",
        "capability_succeeded", "capability_failed", "host_connector_succeeded",
        "host_connector_failed", "host_connector_cancelled", "host_connector_timeout",
        "error", "heartbeat",
    ]

    private func lifecycleEvent(type: String, event: [String: Any]) throws -> TraverseRuntimeEvent {
        let data = try JSONSerialization.data(
            withJSONObject: event["data"] ?? [String: Any](), options: [.sortedKeys, .fragmentsAllowed])
        let eventType = Self.lifecycleEventTypes.contains(type) ? type : "error"
        let sequence = eventSequence.withLock { value -> Int in
            value += 1
            return value
        }
        return TraverseRuntimeEvent(
            sequence: sequence,
            targetID: "app_command",
            status: "emitted",
            eventType: eventType,
            sessionID: optionalString("session_id", in: event),
            errorData: eventType == "error" ? data : nil,
            output: data
        )
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

    public func shutdown() throws -> Data {
        appCommands.stop()
        return try client.shutdown()
    }

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
        let json: Any
        do {
            json = try JSONSerialization.jsonObject(with: data)
        } catch {
            throw TraverseBridgeError(status: -2, message: "bridge_invalid_json")
        }
        if let value = json as? [String: Any] {
            return value
        }
        if let dict = json as? NSDictionary {
            var result: [String: Any] = [:]
            for (key, value) in dict {
                guard let key = key as? String else { continue }
                result[key] = value
            }
            return result
        }
        throw TraverseBridgeError(status: -2, message: "bridge_invalid_json")
    }

    private func requiredString(_ name: String, in value: [String: Any]) throws -> String {
        guard let result = value[name] as? String else {
            throw TraverseBridgeError(status: -2, message: "bridge result is missing \(name)")
        }
        return result
    }

    private func requiredInt(_ name: String, in value: [String: Any]) throws -> Int {
        if let result = value[name] as? Int {
            return result
        }
        if let number = value[name] as? NSNumber {
            return number.intValue
        }
        throw TraverseBridgeError(status: -2, message: "bridge result is missing \(name)")
    }

    private func optionalString(_ name: String, in value: [String: Any]) -> String? {
        value[name] as? String
    }
}
