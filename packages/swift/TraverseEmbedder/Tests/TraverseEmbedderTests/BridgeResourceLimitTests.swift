import CryptoKit
import Foundation
import Testing
@testable import TraverseEmbedder

/// Spec 074 FR-003 (1.1.0, #1615): an over-budget invocation surfaces the
/// stable resource-limit status and the `bridge_timeout` code through
/// `TraverseBridgeError`, instead of the opaque `-5` / `bridge_trap`.
@Test func overBudgetInvocationReportsAStableTimeout() throws {
    let url = try #require(Bundle.module.url(forResource: "client_bridge.wasm", withExtension: nil, subdirectory: "Fixtures"))
    let wasm = Array(try Data(contentsOf: url))
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
    let runtime = root.appendingPathComponent("runtime", isDirectory: true)
    try FileManager.default.createDirectory(at: runtime, withIntermediateDirectories: true)
    try Data(wasm).write(to: runtime.appendingPathComponent("runtime.wasm"))
    let bundle = try TraverseBundle(
        rootURL: root,
        runtimeWasmDigest: "sha256:" + SHA256.hash(data: Data(wasm)).map { String(format: "%02x", $0) }.joined()
    )
    let client = try WasmiHostBridgeClient(bundle: bundle, limits: try TraverseHostLimits(fuelPerInvocation: 64))
    do {
        _ = try client.initialize(configJSON: Data("{}".utf8))
        Issue.record("a 64-unit fuel budget (enough to create the host) must not complete initialize")
    } catch let error as TraverseBridgeError {
        #expect(error.status == -4)
        #expect(error.code == "bridge_timeout")
    }
}
