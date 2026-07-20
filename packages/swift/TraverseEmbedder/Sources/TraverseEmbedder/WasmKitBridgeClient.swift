import Foundation
import WasmKit

/// Serialized UTF-8 JSON client for the governed runtime-WASM bridge.
@available(*, deprecated, message: "Use WasmiHostBridgeClient; WasmKit is retained only for source compatibility.")
public final class WasmKitBridgeClient: @unchecked Sendable, TraverseBridgeClient {
    public static let defaultMaximumOutputBytes = 1024 * 1024

    private let instance: Instance
    private let memory: Memory
    private let lock = NSLock()
    private let maximumOutputBytes: Int

    public init(
        bridge: WasmKitRuntimeBridge,
        maximumOutputBytes: Int = WasmKitBridgeClient.defaultMaximumOutputBytes
    ) throws {
        guard maximumOutputBytes > 0 else {
            throw TraverseBridgeError(status: -4, message: "maximum bridge output size must be positive")
        }
        guard let memory = bridge.instance.exports[memory: "memory"] else {
            throw TraverseBridgeError(status: -3, message: "bridge_invalid_descriptor")
        }
        self.instance = bridge.instance
        self.memory = memory
        self.maximumOutputBytes = maximumOutputBytes
    }

    public func initialize(configJSON: Data) throws -> Data {
        try serialized { try invokeWithInput("traverse_init", input: configJSON) }
    }

    public func submit(requestJSON: Data) throws -> Data {
        try serialized { try invokeWithInput("traverse_submit", input: requestJSON) }
    }

    public func cancel(requestJSON: Data) throws -> Data {
        try serialized { try invokeWithInput("traverse_cancel", input: requestJSON) }
    }

    public func compatibleStart(requestJSON: Data) throws -> Data {
        try serialized { try invokeWithInput("traverse_compatible_start", input: requestJSON) }
    }

    public func compatibleStop(requestJSON: Data) throws -> Data {
        try serialized { try invokeWithInput("traverse_compatible_stop", input: requestJSON) }
    }

    public func compatibleKill(requestJSON: Data) throws -> Data {
        try serialized { try invokeWithInput("traverse_compatible_kill", input: requestJSON) }
    }

    public func nextEvent() throws -> Data? {
        try serialized {
            let descriptor = try allocate(Self.descriptorBytes)
            defer { try? deallocate(descriptor, length: Self.descriptorBytes) }
            let status = try callStatus("traverse_next_event", [.i32(descriptor)])
            if status == 0 { return nil }
            return try readResult(status: status, descriptor: descriptor)
        }
    }

    public func shutdown() throws -> Data {
        try serialized {
            let descriptor = try allocate(Self.descriptorBytes)
            defer { try? deallocate(descriptor, length: Self.descriptorBytes) }
            let status = try callStatus("traverse_shutdown", [.i32(descriptor)])
            return try readResult(status: status, descriptor: descriptor)
        }
    }

    private func invokeWithInput(_ export: String, input: Data) throws -> Data {
        let inputPointer = try allocate(input.count)
        let descriptor = try allocate(Self.descriptorBytes)
        defer {
            try? deallocate(descriptor, length: Self.descriptorBytes)
            try? deallocate(inputPointer, length: input.count)
        }
        try write(input, at: inputPointer)
        let status = try callStatus(export, [
            .i32(inputPointer), .i32(UInt32(input.count)), .i32(descriptor),
        ])
        return try readResult(status: status, descriptor: descriptor)
    }

    private func allocate(_ length: Int) throws -> UInt32 {
        guard let function = instance.exports[function: "traverse_alloc"] else {
            throw TraverseBridgeError(status: -5, message: "bridge allocation export is unavailable")
        }
        let result = try function([.i32(UInt32(length))])
        guard result.count == 1, case .i32(let pointer) = result[0] else {
            throw TraverseBridgeError(status: -4, message: "bridge allocation failed")
        }
        return pointer
    }

    private func deallocate(_ pointer: UInt32, length: Int) throws {
        guard let function = instance.exports[function: "traverse_dealloc"] else { return }
        _ = try function([.i32(pointer), .i32(UInt32(length))])
    }

    private func callStatus(_ export: String, _ arguments: [Value]) throws -> Int32 {
        guard let function = instance.exports[function: export] else {
            throw TraverseBridgeError(status: -5, message: "bridge export \(export) is unavailable")
        }
        let result = try function(arguments)
        guard result.count == 1, case .i32(let rawStatus) = result[0] else {
            throw TraverseBridgeError(status: -5, message: "bridge export \(export) returned an invalid status")
        }
        return Int32(bitPattern: rawStatus)
    }

    private func readResult(status: Int32, descriptor: UInt32) throws -> Data {
        let descriptorBytes = try read(at: descriptor, count: Self.descriptorBytes)
        let pointer = descriptorBytes.prefix(4).withUnsafeBytes { $0.loadUnaligned(as: UInt32.self) }.littleEndian
        let length = descriptorBytes.suffix(4).withUnsafeBytes { $0.loadUnaligned(as: UInt32.self) }.littleEndian
        guard length <= UInt32(maximumOutputBytes) else {
            throw TraverseBridgeError(status: -3, message: "bridge_invalid_descriptor")
        }
        let output = try read(at: pointer, count: Int(length))
        if status < 0 {
            throw TraverseBridgeError(
                status: status,
                message: String(data: output, encoding: .utf8) ?? "bridge_invalid_json"
            )
        }
        return output
    }

    private func write(_ data: Data, at pointer: UInt32) throws {
        guard Int(pointer) <= memory.data.count - data.count else {
            throw TraverseBridgeError(status: -3, message: "bridge_invalid_descriptor")
        }
        _ = memory.withUnsafeMutableBufferPointer(offset: UInt(pointer), count: data.count) { buffer in
            data.copyBytes(to: buffer.bindMemory(to: UInt8.self))
        }
    }

    private func read(at pointer: UInt32, count: Int) throws -> Data {
        let snapshot = memory.data
        guard count >= 0, Int(pointer) <= snapshot.count - count else {
            throw TraverseBridgeError(status: -3, message: "bridge_invalid_descriptor")
        }
        return Data(snapshot[Int(pointer)..<Int(pointer) + count])
    }

    private func serialized<T>(_ body: () throws -> T) rethrows -> T {
        lock.lock()
        defer { lock.unlock() }
        return try body()
    }

    private static let descriptorBytes = 8
}

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
