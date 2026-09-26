import Foundation

public struct TraverseBridgeError: Error, Equatable, Sendable {
    public let status: Int32
    public let message: String
    /// The bridge's own structured error code (e.g. `bridge_trap`,
    /// `bridge_runtime_error`, `bridge_resource_limit`), when the host
    /// wrote one. Distinguishes a runtime/host failure from a host-connector
    /// adapter's own failure, which is reported separately as a lifecycle
    /// event (`host_connector_result` with `result_class: "failed"`), not
    /// through this error type. `nil` when the host produced no structured
    /// payload for this status.
    public let code: String?

    public init(status: Int32, message: String, code: String? = nil) {
        self.status = status
        self.message = message
        self.code = code
    }
}

public protocol TraverseBridgeClient: Sendable {
    func initialize(configJSON: Data) throws -> Data
    func submit(requestJSON: Data) throws -> Data
    func cancel(requestJSON: Data) throws -> Data
    func compatibleStart(requestJSON: Data) throws -> Data
    func compatibleStop(requestJSON: Data) throws -> Data
    func compatibleKill(requestJSON: Data) throws -> Data
    func nextEvent() throws -> Data?
    func shutdown() throws -> Data
}
