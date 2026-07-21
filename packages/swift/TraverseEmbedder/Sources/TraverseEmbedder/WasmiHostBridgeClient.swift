import Foundation
import TraverseSwiftHost

/// Explicit limits for the production `wasmi` host boundary.
public struct TraverseHostLimits: Sendable, Equatable {
    public let maximumArtifactBytes: UInt64
    public let maximumMemoryBytes: UInt64
    public let fuelPerInvocation: UInt64
    public let maximumInputBytes: UInt64
    public let maximumOutputBytes: UInt64
    public let maximumQueuedEvents: UInt64

    public init(
        maximumArtifactBytes: UInt64 = 32 * 1024 * 1024,
        maximumMemoryBytes: UInt64 = 64 * 1024 * 1024,
        fuelPerInvocation: UInt64 = 1_000_000,
        maximumInputBytes: UInt64 = 1024 * 1024,
        maximumOutputBytes: UInt64 = 1024 * 1024,
        maximumQueuedEvents: UInt64 = 1024
    ) throws {
        let values = [maximumArtifactBytes, maximumMemoryBytes, fuelPerInvocation, maximumInputBytes, maximumOutputBytes, maximumQueuedEvents]
        guard !values.contains(0) else {
            throw TraverseBridgeError(status: -4, message: "bridge_resource_limit")
        }
        self.maximumArtifactBytes = maximumArtifactBytes
        self.maximumMemoryBytes = maximumMemoryBytes
        self.fuelPerInvocation = fuelPerInvocation
        self.maximumInputBytes = maximumInputBytes
        self.maximumOutputBytes = maximumOutputBytes
        self.maximumQueuedEvents = maximumQueuedEvents
    }
}

/// Serialized engine-neutral client for the production `TraverseSwiftHost` ABI.
public final class WasmiHostBridgeClient: @unchecked Sendable, TraverseBridgeClient {
    private var handle: UInt64 = 0
    private let lock = NSLock()
    private let limits: TraverseHostLimits

    public convenience init(bundle: TraverseBundle) throws {
        try self.init(bundle: bundle, limits: TraverseHostLimits())
    }

    public init(bundle: TraverseBundle, limits: TraverseHostLimits) throws {
        guard bundle.embedderAPIVersion == TraverseEmbedder.apiVersion else {
            throw TraverseEmbedderError.incompatibleBundle("embedder API \(bundle.embedderAPIVersion) is incompatible with \(TraverseEmbedder.apiVersion)")
        }
        let runtimeURL = bundle.rootURL.appendingPathComponent("runtime").appendingPathComponent("runtime.wasm")
        let runtime: Data
        do {
            runtime = try Data(contentsOf: runtimeURL, options: [.mappedIfSafe])
        } catch {
            throw TraverseEmbedderError.incompatibleBundle("runtime/runtime.wasm is unavailable")
        }
        self.limits = limits
        var nativeLimits = traverse_swift_host_limits(
            maximum_artifact_bytes: limits.maximumArtifactBytes,
            maximum_memory_bytes: limits.maximumMemoryBytes,
            fuel_per_invocation: limits.fuelPerInvocation,
            maximum_input_bytes: limits.maximumInputBytes,
            maximum_output_bytes: limits.maximumOutputBytes,
            maximum_queued_events: limits.maximumQueuedEvents
        )
        var nativeHandle: UInt64 = 0
        let expectedDigest = Data(bundle.runtimeWasmDigest.utf8)
        let status = runtime.withUnsafeBytes { runtimeBuffer in
            expectedDigest.withUnsafeBytes { digestBuffer in
                traverse_swift_host_create(
                    runtimeBuffer.bindMemory(to: UInt8.self).baseAddress,
                    runtime.count,
                    digestBuffer.bindMemory(to: UInt8.self).baseAddress,
                    expectedDigest.count,
                    &nativeLimits,
                    &nativeHandle
                )
            }
        }
        guard status == 0 else { throw Self.error(status) }
        handle = nativeHandle
    }

    deinit {
        if handle != 0 { _ = traverse_swift_host_destroy(handle) }
    }

    public func initialize(configJSON: Data) throws -> Data { try invoke("init", input: configJSON) }
    public func submit(requestJSON: Data) throws -> Data { try invoke("submit", input: requestJSON) }
    public func cancel(requestJSON: Data) throws -> Data { try invoke("cancel", input: requestJSON) }
    public func compatibleStart(requestJSON: Data) throws -> Data { try invoke("compatible_start", input: requestJSON) }
    public func compatibleStop(requestJSON: Data) throws -> Data { try invoke("compatible_stop", input: requestJSON) }
    public func compatibleKill(requestJSON: Data) throws -> Data { try invoke("compatible_kill", input: requestJSON) }
    public func nextEvent() throws -> Data? {
        let result = try invoke("next_event", input: Data("{}".utf8))
        return result.isEmpty ? nil : result
    }
    public func shutdown() throws -> Data { try invoke("shutdown", input: Data("{}".utf8)) }

    private func invoke(_ operation: String, input: Data) throws -> Data {
        try lock.withLock {
            guard handle != 0 else { throw TraverseBridgeError(status: -1, message: "invalid_handle") }
            var required = 0
            guard let outputCapacity = Int(exactly: limits.maximumOutputBytes) else {
                throw TraverseBridgeError(status: -4, message: "bridge_resource_limit")
            }
            var output = Data(repeating: 0, count: outputCapacity)
            let status = call(operation: operation, input: input, output: &output, required: &required)
            if status == -6 {
                output = Data(repeating: 0, count: required)
                let retry = call(operation: operation, input: input, output: &output, required: &required)
                guard retry >= 0 else { throw Self.error(retry) }
                output.count = required
                return output
            }
            guard status >= 0 else { throw Self.error(status) }
            output.count = required
            return output
        }
    }

    private func call(operation: String, input: Data, output: inout Data, required: inout Int) -> Int32 {
        let operationData = Data(operation.utf8)
        let outputCapacity = output.count
        return operationData.withUnsafeBytes { operationBuffer in
            input.withUnsafeBytes { inputBuffer in
                output.withUnsafeMutableBytes { outputBuffer in
                    traverse_swift_host_invoke(
                        handle,
                        operationBuffer.bindMemory(to: UInt8.self).baseAddress,
                        operationData.count,
                        inputBuffer.bindMemory(to: UInt8.self).baseAddress,
                        input.count,
                        outputBuffer.bindMemory(to: UInt8.self).baseAddress,
                        outputCapacity,
                        &required
                    )
                }
            }
        }
    }

    private static func error(_ status: Int32) -> TraverseBridgeError {
        TraverseBridgeError(status: status, message: String(cString: traverse_swift_host_status_message(status)))
    }
}
