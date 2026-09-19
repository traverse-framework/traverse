import Foundation

public struct TraverseBridgeError: Error, Equatable, Sendable {
    public let status: Int32
    public let message: String

    public init(status: Int32, message: String) {
        self.status = status
        self.message = message
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
