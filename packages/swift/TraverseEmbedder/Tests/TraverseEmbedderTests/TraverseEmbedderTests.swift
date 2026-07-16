import Foundation
import CryptoKit
import Testing
import WAT
@testable import TraverseEmbedder

@Test func lifecycleAndSubmissionAreDeterministic() throws {
    let harness = InMemoryTraverseEmbedder()
    let bundle = try TraverseBundle(
        rootURL: URL(fileURLWithPath: "/tmp/traverse-bundle"),
        runtimeWasmDigest: "sha256:test"
    )
    try harness.initialize(bundle: bundle)

    #expect(try harness.submit(TraverseSubmission(targetID: "demo.workflow", inputJSON: Data("{}".utf8))) == TraverseSubmissionResult(sessionID: "swift-session-1", status: "accepted"))
    #expect(try harness.submit(TraverseSubmission(targetID: "demo.capability", inputJSON: Data("{}".utf8))) == TraverseSubmissionResult(sessionID: "swift-session-2", status: "accepted"))
    #expect(try harness.subscribe() == [
        TraverseRuntimeEvent(sequence: 1, targetID: "demo.workflow", status: "accepted"),
        TraverseRuntimeEvent(sequence: 2, targetID: "demo.capability", status: "accepted"),
    ])
    #expect(try harness.subscribe(after: 1) == [
        TraverseRuntimeEvent(sequence: 2, targetID: "demo.capability", status: "accepted"),
    ])
    harness.shutdown()
    #expect(throws: TraverseEmbedderError.notInitialized) {
        try harness.submit(TraverseSubmission(targetID: "demo.workflow", inputJSON: Data("{}".utf8)))
    }
}

@Test func incompatibleBundleIsRejectedWithoutInitializing() throws {
    let harness = InMemoryTraverseEmbedder()
    let bundle = try TraverseBundle(
        rootURL: URL(fileURLWithPath: "/tmp/traverse-bundle"),
        runtimeWasmDigest: "sha256:test",
        embedderAPIVersion: "2.0.0"
    )

    #expect(throws: TraverseEmbedderError.incompatibleBundle(
        "embedder API 2.0.0 is incompatible with 1.0.0"
    )) {
        try harness.initialize(bundle: bundle)
    }
    #expect(throws: TraverseEmbedderError.notInitialized) {
        try harness.subscribe()
    }
}

@Test func compatibleLifecycleIsDeterministicAndOrdered() throws {
    let harness = InMemoryTraverseEmbedder()
    try harness.initialize(bundle: TraverseBundle(
        rootURL: URL(fileURLWithPath: "/tmp/traverse-bundle"),
        runtimeWasmDigest: "sha256:test"
    ))

    let first = try harness.compatibleStart(capabilityID: "demo.compatible", inputJSON: Data("{}".utf8))
    #expect(first == TraverseCompatibleResult(instanceID: "swift-compatible-1", status: "started"))
    #expect(try harness.compatibleStop(capabilityID: "demo.compatible", instanceID: first.instanceID) == TraverseCompatibleResult(instanceID: "swift-compatible-1", status: "stopped"))
    #expect(throws: TraverseEmbedderError.unsupportedOperation("compatible instance is not active")) {
        try harness.compatibleKill(capabilityID: "demo.compatible", instanceID: first.instanceID)
    }

    let second = try harness.compatibleStart(capabilityID: "demo.compatible", inputJSON: Data("{}".utf8))
    #expect(try harness.compatibleKill(capabilityID: "demo.compatible", instanceID: nil) == TraverseCompatibleResult(instanceID: "swift-compatible-2", status: "killed"))
    #expect(try harness.subscribe() == [
        TraverseRuntimeEvent(sequence: 1, targetID: "demo.compatible", status: "started", instanceID: first.instanceID),
        TraverseRuntimeEvent(sequence: 2, targetID: "demo.compatible", status: "stopped", instanceID: first.instanceID),
        TraverseRuntimeEvent(sequence: 3, targetID: "demo.compatible", status: "started", instanceID: second.instanceID),
        TraverseRuntimeEvent(sequence: 4, targetID: "demo.compatible", status: "killed", instanceID: second.instanceID),
    ])
}

@Test func releaseEvidenceIsCompleteAndDeterministic() throws {
    #expect(try TraverseReleaseEvidence(
        packageVersion: "0.1.0",
        runtimeWasmDigest: "sha256:test",
        supportedHostVersions: ["iOS 17+", "macOS 14+"]
    ) == TraverseReleaseEvidence(
        packageVersion: "0.1.0",
        runtimeWasmDigest: "sha256:test",
        conformanceVersion: "1.0.0",
        supportedHostVersions: ["iOS 17+", "macOS 14+"]
    ))
    #expect(throws: TraverseEmbedderError.invalidReleaseEvidence("supported host versions are required")) {
        try TraverseReleaseEvidence(
            packageVersion: "0.1.0",
            runtimeWasmDigest: "sha256:test",
            supportedHostVersions: []
        )
    }
}

@Test func wasmKitBridgeVerifiesAndInstantiatesTheGovernedABI() throws {
    let wasm = try wat2wasm(validBridgeWAT)
    let bundle = try fixtureBundle(wasm: wasm)

    let bridge = try WasmKitRuntimeBridge(bundle: bundle)

    #expect(bridge.runtimeWasmDigest == digest(of: wasm))
    #expect(bridge.runtimeURL.lastPathComponent == "runtime.wasm")
}

@Test func wasmKitBridgeRejectsTamperingBeforeInstantiation() throws {
    let wasm = try wat2wasm(validBridgeWAT)
    let bundle = try fixtureBundle(wasm: wasm, declaredDigest: "sha256:" + String(repeating: "0", count: 64))

    #expect(throws: TraverseEmbedderError.incompatibleBundle("bundle_digest_mismatch")) {
        try WasmKitRuntimeBridge(bundle: bundle)
    }
}

@Test func wasmKitBridgeRejectsAmbientImportsAndWrongABIMajor() throws {
    let imported = try wat2wasm("""
        (module
          (import "wasi_snapshot_preview1" "fd_write" (func))
          (memory (export "memory") 1)
          (func (export "traverse_bridge_abi_version") (result i32) i32.const 10000))
        """)
    let importedBundle = try fixtureBundle(wasm: imported)
    #expect(throws: TraverseEmbedderError.incompatibleBundle("runtime/runtime.wasm requires undeclared ambient imports")) {
        try WasmKitRuntimeBridge(bundle: importedBundle)
    }

    let wrongVersion = try wat2wasm(validBridgeWAT.replacingOccurrences(of: "i32.const 10100", with: "i32.const 20000"))
    let wrongVersionBundle = try fixtureBundle(wasm: wrongVersion)
    #expect(throws: TraverseEmbedderError.incompatibleBundle("bridge_version_mismatch")) {
        try WasmKitRuntimeBridge(bundle: wrongVersionBundle)
    }
}

private let validBridgeWAT = """
    (module
      (memory (export "memory") 1 16)
      (func (export "traverse_bridge_abi_version") (result i32) i32.const 10100)
      (func (export "traverse_alloc") (param i32) (result i32) i32.const 64)
      (func (export "traverse_dealloc") (param i32 i32))
      (func (export "traverse_init") (param i32 i32 i32) (result i32) i32.const 0)
      (func (export "traverse_submit") (param i32 i32 i32) (result i32) i32.const 0)
      (func (export "traverse_next_event") (param i32) (result i32) i32.const 0)
      (func (export "traverse_cancel") (param i32 i32 i32) (result i32) i32.const 0)
      (func (export "traverse_compatible_start") (param i32 i32 i32) (result i32) i32.const 0)
      (func (export "traverse_compatible_stop") (param i32 i32 i32) (result i32) i32.const 0)
      (func (export "traverse_compatible_kill") (param i32 i32 i32) (result i32) i32.const 0)
      (func (export "traverse_shutdown") (param i32) (result i32) i32.const 0))
    """

private func fixtureBundle(wasm: [UInt8], declaredDigest: String? = nil) throws -> TraverseBundle {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
    let runtime = root.appendingPathComponent("runtime", isDirectory: true)
    try FileManager.default.createDirectory(at: runtime, withIntermediateDirectories: true)
    try Data(wasm).write(to: runtime.appendingPathComponent("runtime.wasm"))
    return try TraverseBundle(
        rootURL: root,
        runtimeWasmDigest: declaredDigest ?? digest(of: wasm)
    )
}

private func digest(of bytes: [UInt8]) -> String {
    "sha256:" + SHA256.hash(data: Data(bytes)).map { String(format: "%02x", $0) }.joined()
}
