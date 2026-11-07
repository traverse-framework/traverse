import CryptoKit
import Foundation
import Testing
@testable import TraverseEmbedder

// Cross-host canonical guest input (Spec 1722 FR-006, #1795).
//
// Runs fixtures/cross-host/canonical-input-v1 through the real `runtime.wasm`
// in TRAVERSE_NATIVE_ARTIFACT_ROOT, once as raw bytes on the bridge client and
// once through RuntimeTraverseEmbedder. The stdin-echo guest returns the bytes
// it read, which must match the Rust reference on every host.

private let fixtureDir = URL(fileURLWithPath: #filePath)
    .deletingLastPathComponent() // TraverseEmbedderTests
    .deletingLastPathComponent() // Tests
    .deletingLastPathComponent() // TraverseEmbedder
    .deletingLastPathComponent() // swift
    .deletingLastPathComponent() // packages
    .deletingLastPathComponent() // repository root
    .appendingPathComponent("fixtures/cross-host/canonical-input-v1")

private let capabilityID = "conformance.stdin-echo"

private struct Vector {
    let name: String
    let input: String
    let expectedStdin: String?
    let expectedSHA256: String?
    let expectedError: String?
}

private enum Outcome {
    case output(stdin: String)
    case failure(String)
}

private func vectors() throws -> [Vector] {
    let data = try Data(contentsOf: fixtureDir.appendingPathComponent("vectors.json"))
    let fixture = try #require(try JSONSerialization.jsonObject(with: data) as? [String: Any])
    let entries = try #require(fixture["vectors"] as? [[String: Any]])
    return try entries.map { entry in
        Vector(
            name: try #require(entry["name"] as? String),
            input: try #require(entry["input"] as? String),
            expectedStdin: entry["expected_stdin"] as? String,
            expectedSHA256: entry["expected_sha256"] as? String,
            expectedError: entry["expected_error"] as? String
        )
    }
}

/// A fresh real `runtime.wasm` with stdin-echo registered as a stateless
/// capability, or nil when no artifact root is set.
private func stdinEchoClient() throws -> WasmiHostBridgeClient? {
    guard let rootPath = ProcessInfo.processInfo.environment["TRAVERSE_NATIVE_ARTIFACT_ROOT"] else { return nil }
    let runtime = try Data(contentsOf: URL(fileURLWithPath: rootPath).appendingPathComponent("runtime/runtime.wasm"))
    let client = try WasmiHostBridgeClient(
        bundle: TraverseBundle(
            rootURL: URL(fileURLWithPath: rootPath),
            runtimeWasmDigest: "sha256:" + SHA256.hash(data: runtime).map { String(format: "%02x", $0) }.joined()
        ),
        limits: try TraverseHostLimits(fuelPerInvocation: 50_000_000)
    )
    let header: [String: Any] = [
        "capability_id": capabilityID,
        "capability_version": "1.0.0",
        "service_type": "stateless",
        "emits": [] as [Any],
        "host_placement_target": "local",
        "permitted_targets": ["local"],
    ]
    let headerBytes = try JSONSerialization.data(withJSONObject: header)
    var initPayload = Data()
    var headerLength = UInt32(headerBytes.count).littleEndian
    withUnsafeBytes(of: &headerLength) { initPayload.append(contentsOf: $0) }
    initPayload.append(headerBytes)
    initPayload.append(try Data(contentsOf: fixtureDir.appendingPathComponent("stdin-echo.wasm")))
    let ready = try JSONSerialization.jsonObject(with: client.initialize(configJSON: initPayload)) as? [String: Any]
    #expect(ready?["status"] as? String == "ready")
    return client
}

/// The guest's echoed stdin, or the failure message from submit or the result
/// event. `submit` returns the rejection text when the runtime didn't accept.
private func outcome(_ client: WasmiHostBridgeClient, submit: () throws -> String?) -> Outcome {
    do {
        if let rejection = try submit() {
            return .failure(rejection)
        }
        while let event = try client.nextEvent() {
            guard let object = try JSONSerialization.jsonObject(with: event) as? [String: Any],
                  object["type"] as? String == "capability_result",
                  let data = object["data"] as? [String: Any] else { continue }
            if data["status"] as? String == "completed",
               let output = data["output"] as? [String: Any],
               let stdin = output["stdin"] as? String {
                return .output(stdin: stdin)
            }
            return .failure(String(describing: data["error"] ?? data))
        }
        return .failure("no capability_result event")
    } catch {
        return .failure(String(describing: error))
    }
}

private func check(_ host: String, _ vector: Vector, _ outcome: Outcome) {
    switch (vector.expectedError, outcome) {
    case let (code?, .failure(message)):
        #expect(message.contains(code), "\(host)/\(vector.name): expected \(code), got \(message)")
    case let (nil, .output(stdin)):
        #expect(stdin == vector.expectedStdin, "\(host)/\(vector.name): stdin bytes")
        let digest = SHA256.hash(data: Data(stdin.utf8)).map { String(format: "%02x", $0) }.joined()
        #expect(digest == vector.expectedSHA256, "\(host)/\(vector.name): digest")
    case let (expected, actual):
        Issue.record("\(host)/\(vector.name): expected \(expected ?? "output"), got \(actual)")
    }
}

@Test func canonicalInputVectorsReachTheGuestAsRawBytes() throws {
    for vector in try vectors() {
        guard let client = try stdinEchoClient() else { return }
        check("swift raw", vector, outcome(client) {
            let response = try client.submit(requestJSON: Data(vector.input.utf8))
            let object = try JSONSerialization.jsonObject(with: response) as? [String: Any]
            return object?["status"] as? String == "accepted" ? nil : String(decoding: response, as: UTF8.self)
        })
    }
}

@Test func canonicalInputVectorsReachTheGuestThroughTheEmbedder() throws {
    // TraverseSubmission carries the caller's bytes unchanged, so bytes-only
    // vectors (duplicate member names) run here too.
    for vector in try vectors() {
        guard let client = try stdinEchoClient() else { return }
        let runtime = RuntimeTraverseEmbedder(client: client)
        check("swift embedder", vector, outcome(client) {
            let result = try runtime.submit(try TraverseSubmission(targetID: capabilityID, inputJSON: Data(vector.input.utf8)))
            return result.status == "accepted" ? nil : "\(result.status): \(result.error ?? "")"
        })
    }
}

/// A fresh real `runtime.wasm` initialized with a compatible capability on
/// `ios` (Spec 072 FR-010), or nil when no artifact root is set.
private func compatibleClient() throws -> WasmiHostBridgeClient? {
    guard let rootPath = ProcessInfo.processInfo.environment["TRAVERSE_NATIVE_ARTIFACT_ROOT"] else { return nil }
    let runtime = try Data(contentsOf: URL(fileURLWithPath: rootPath).appendingPathComponent("runtime/runtime.wasm"))
    let client = try WasmiHostBridgeClient(
        bundle: TraverseBundle(
            rootURL: URL(fileURLWithPath: rootPath),
            runtimeWasmDigest: "sha256:" + SHA256.hash(data: runtime).map { String(format: "%02x", $0) }.joined()
        ),
        limits: try TraverseHostLimits(fuelPerInvocation: 50_000_000)
    )
    let header = try JSONSerialization.data(withJSONObject: [
        "capability_id": "conformance.camera",
        "capability_version": "1.0.0",
        "execution_mode": "compatible",
        "platforms": ["ios"],
        "host_platform": "ios",
    ] as [String: Any])
    var initPayload = Data()
    var headerLength = UInt32(header.count).littleEndian
    withUnsafeBytes(of: &headerLength) { initPayload.append(contentsOf: $0) }
    initPayload.append(header)
    let ready = try JSONSerialization.jsonObject(with: client.initialize(configJSON: initPayload)) as? [String: Any]
    #expect(ready?["status"] as? String == "ready")
    return client
}

@Test func canonicalInputVectorsReachTheWrapperThroughCompatibleStart() throws {
    // Spec 072 FR-012 (Decision 128): the started event's `input` is the runtime's canonical bytes.
    for vector in try vectors() {
        guard let client = try compatibleClient() else { return }
        let runtime = RuntimeTraverseEmbedder(client: client)
        let result: Outcome
        do {
            _ = try runtime.compatibleStart(capabilityID: "conformance.camera", inputJSON: Data(vector.input.utf8))
            let started = try #require(try runtime.subscribe().first { $0.eventType == "state_changed" })
            let output = try #require(started.output)
            let marker = try #require(output.range(of: Data(#","input":"#.utf8)))
            result = .output(stdin: String(decoding: output[marker.upperBound..<output.index(before: output.endIndex)], as: UTF8.self))
        } catch {
            result = .failure(String(describing: error))
        }
        check("swift compatible", vector, result)
    }
}

@Test func compatibleLifecycleUsesTheRuntimeRules() throws {
    guard let client = try compatibleClient() else { return }
    let runtime = RuntimeTraverseEmbedder(client: client)
    let started = try runtime.compatibleStart(capabilityID: "conformance.camera", inputJSON: Data("{}".utf8))
    #expect(started.instanceID == "conformance.camera-compatible-1")
    #expect(throws: (any Error).self) {
        try runtime.compatibleStart(capabilityID: "conformance.camera", inputJSON: Data("{}".utf8))
    }
    let killed = try runtime.compatibleKill(capabilityID: "conformance.camera", instanceID: nil)
    #expect(killed.status == "killed")
    #expect(throws: (any Error).self) {
        try runtime.compatibleKill(capabilityID: "conformance.camera", instanceID: started.instanceID)
    }
    let states = try runtime.subscribe().map { $0.status }
    #expect(states == ["started", "killed"])
}
