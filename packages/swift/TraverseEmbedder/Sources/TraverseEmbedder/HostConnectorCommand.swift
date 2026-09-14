import Foundation

/// Shared Spec 137 host-connector command/event contract for browser and macOS.
/// Native/browser adapters remain host implementations behind this wire type.

public enum HostConnectorCommandContract {
    public static let schemaVersion = "1.0.0"
    public static let commandKind = "host_connector_command"
    public static let resultKind = "host_connector_result"
    public static let eventKind = "host_connector_event"
    public static let governingSpec = "137-host-connector-command-dispatch"
    public static let audioInputConnector = "traverse.audio-input"
    public static let audioCaptureOperation = "audio.capture"
    public static let modelRuntimeConnector = "traverse.model-runtime"
    public static let modelExecuteOperation = "model.execute"
}

public struct HostConnectorAppCommand: Sendable, Equatable {
    public let kind: String
    public let schemaVersion: String
    public let command: String
    public let commandId: String
    public let correlationId: String
    public let idempotencyKey: String
    public let targetFamily: String
    public let cancelRequested: Bool
    public let payloadJSON: Data

    public init(
        command: String,
        commandId: String,
        correlationId: String,
        idempotencyKey: String,
        targetFamily: String,
        cancelRequested: Bool = false,
        payloadJSON: Data
    ) {
        self.kind = HostConnectorCommandContract.commandKind
        self.schemaVersion = HostConnectorCommandContract.schemaVersion
        self.command = command
        self.commandId = commandId
        self.correlationId = correlationId
        self.idempotencyKey = idempotencyKey
        self.targetFamily = targetFamily
        self.cancelRequested = cancelRequested
        self.payloadJSON = payloadJSON
    }

    public static func audioCapture(
        commandId: String,
        correlationId: String,
        idempotencyKey: String,
        targetFamily: String,
        maxDurationMs: Int,
        maxBytes: Int
    ) throws -> HostConnectorAppCommand {
        let payload = try JSONSerialization.data(
            withJSONObject: ["max_duration_ms": maxDurationMs, "max_bytes": maxBytes]
        )
        return HostConnectorAppCommand(
            command: "capture_audio",
            commandId: commandId,
            correlationId: correlationId,
            idempotencyKey: idempotencyKey,
            targetFamily: targetFamily,
            payloadJSON: payload
        )
    }

    /// Snake_case wire object shared with the browser embedder.
    public func wireObject() throws -> [String: Any] {
        let payload = try JSONSerialization.jsonObject(with: payloadJSON)
        return [
            "kind": kind,
            "schema_version": schemaVersion,
            "command": command,
            "command_id": commandId,
            "correlation_id": correlationId,
            "idempotency_key": idempotencyKey,
            "target_family": targetFamily,
            "cancel_requested": cancelRequested,
            "payload": payload,
        ]
    }
}

public struct HostConnectorEvent: Sendable, Equatable {
    public let kind: String
    public let schemaVersion: String
    public let event: String
    public let commandId: String
    public let correlationId: String
    public let connectorId: String?
    public let operation: String?
    public let bindingId: String?
    public let targetFamily: String
    public let artifactRef: String?
    public let errorCode: String?

    public init(
        event: String,
        commandId: String,
        correlationId: String,
        targetFamily: String,
        connectorId: String? = nil,
        operation: String? = nil,
        bindingId: String? = nil,
        artifactRef: String? = nil,
        errorCode: String? = nil
    ) {
        self.kind = HostConnectorCommandContract.eventKind
        self.schemaVersion = HostConnectorCommandContract.schemaVersion
        self.event = event
        self.commandId = commandId
        self.correlationId = correlationId
        self.connectorId = connectorId
        self.operation = operation
        self.bindingId = bindingId
        self.targetFamily = targetFamily
        self.artifactRef = artifactRef
        self.errorCode = errorCode
    }

    /// Snake_case wire object shared with the browser embedder.
    public func wireObject() -> [String: Any] {
        var object: [String: Any] = [
            "kind": kind,
            "schema_version": schemaVersion,
            "event": event,
            "command_id": commandId,
            "correlation_id": correlationId,
            "target_family": targetFamily,
        ]
        if let connectorId { object["connector_id"] = connectorId }
        if let operation { object["operation"] = operation }
        if let bindingId { object["binding_id"] = bindingId }
        if let artifactRef { object["artifact_ref"] = artifactRef }
        if let errorCode { object["error_code"] = errorCode }
        return object
    }
}
