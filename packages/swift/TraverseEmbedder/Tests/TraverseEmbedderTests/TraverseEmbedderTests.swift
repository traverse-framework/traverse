import Foundation
import CryptoKit
import Testing
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

@Test func scriptedTargetOutputIsPublicAndRuntimeOwned() throws {
    let harness = InMemoryTraverseEmbedder().withTargetOutput(Data("{\"answer\":42}".utf8))
    try harness.initialize(bundle: TraverseBundle(rootURL: URL(fileURLWithPath: "/tmp/traverse-bundle"), runtimeWasmDigest: "sha256:test"))
    _ = try harness.submit(TraverseSubmission(targetID: "demo.target", inputJSON: Data("{}".utf8)))
    let event = try #require(harness.subscribe().first)
    #expect(event.eventType == "capability_result")
    #expect(event.sessionID == "swift-session-1")
    #expect(event.output == Data("{\"answer\":42}".utf8))
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

@Test func wasmiHostBridgeRejectsDigestMismatch() throws {
    let wasm = try fixtureBytes("valid_bridge.wasm")
    #expect(throws: TraverseBridgeError.self) {
        _ = try WasmiHostBridgeClient(
            bundle: fixtureBundle(wasm: wasm, declaredDigest: "sha256:" + String(repeating: "0", count: 64))
        )
    }
}

@Test func wasmiHostBridgeRejectsAmbientImportsAndWrongABIMajor() throws {
    let imported = try fixtureBytes("ambient_import.wasm")
    #expect(throws: TraverseBridgeError.self) {
        _ = try WasmiHostBridgeClient(bundle: fixtureBundle(wasm: imported))
    }

    let wrongVersion = try fixtureBytes("wrong_abi_major.wasm")
    #expect(throws: TraverseBridgeError.self) {
        _ = try WasmiHostBridgeClient(bundle: fixtureBundle(wasm: wrongVersion))
    }
}

@Test func wasmiHostBridgeClientUsesThePackagedProductionBoundary() throws {
    let wasm = try fixtureBytes("client_bridge.wasm")
    let client = try WasmiHostBridgeClient(bundle: fixtureBundle(wasm: wasm))

    #expect(try client.initialize(configJSON: Data("{}".utf8)) == Data(#"{"status":"ready"}"#.utf8))
    #expect(try client.submit(requestJSON: Data(#"{"target_id":"demo"}"#.utf8)) == Data(#"{"session_id":"s1","status":"accepted"}"#.utf8))
    #expect(try client.nextEvent() == Data(#"{"sequence":1,"target_id":"demo","status":"completed"}"#.utf8))
    #expect(try client.shutdown() == Data(#"{"status":"stopped"}"#.utf8))
}

@Test func realNativeArtifactRunsWithoutASidecar() throws {
    guard let rootPath = ProcessInfo.processInfo.environment["TRAVERSE_NATIVE_ARTIFACT_ROOT"] else { return }
    let runtimeURL = URL(fileURLWithPath: rootPath).appendingPathComponent("runtime/runtime.wasm")
    let runtime = try Data(contentsOf: runtimeURL)
    // The default `fuelPerInvocation` is sized for a trivial fixture guest.
    // The real `runtime.wasm` interprets genuine Rust code (JSON parsing,
    // heap allocation, a nested wasmi engine) on `init`/`submit`, which costs
    // far more simulated fuel than a few `i32.store`s — a real production
    // host embedding this real artifact needs a correspondingly larger
    // budget, same as this test does.
    let client = try WasmiHostBridgeClient(
        bundle: TraverseBundle(
            rootURL: URL(fileURLWithPath: rootPath),
            runtimeWasmDigest: digest(of: Array(runtime))
        ),
        limits: try TraverseHostLimits(fuelPerInvocation: 50_000_000)
    )

    // The real `runtime-wasm-bridge/1.0.0` guest (crates/traverse-runtime-wasm)
    // hosts a *nested* capability itself, so `traverse_init`'s payload is not
    // bare JSON: a 4-byte little-endian header length, that many bytes of
    // JSON metadata, then the raw nested-capability WASM artifact (spec 1402
    // FR-003/FR-011). This nested capability echoes stdin to stdout, then
    // emits one declared domain event — matching
    // `crates/traverse-runtime/tests/native_bridge_conformance.rs`'s fixture
    // exactly, so all host profiles exercise the same lifecycle transcript.
    let nestedCapability = try fixtureBytes("nested_conformance_capability.wasm")
    let header: [String: Any] = [
        "capability_id": "swift.conformance.echo",
        "capability_version": "1.0.0",
        "service_type": "subscribable",
        "emits": [["event_id": "conformance.echoed", "version": "1.0.0"]],
        "host_placement_target": "local",
        "permitted_targets": ["local"],
    ]
    let headerBytes = try JSONSerialization.data(withJSONObject: header)
    var initPayload = Data()
    var headerLength = UInt32(headerBytes.count).littleEndian
    withUnsafeBytes(of: &headerLength) { initPayload.append(contentsOf: $0) }
    initPayload.append(headerBytes)
    initPayload.append(Data(nestedCapability))

    let initResponse = try jsonObject(from: client.initialize(configJSON: initPayload))
    #expect(initResponse["status"] as? String == "ready")

    let submitResponse = try jsonObject(from: client.submit(requestJSON: Data(#"{"hello":"swift-conformance"}"#.utf8)))
    #expect(submitResponse["status"] as? String == "accepted")

    var eventTypes: [String] = []
    while let event = try client.nextEvent() {
        eventTypes.append(try jsonObject(from: event)["type"] as? String ?? "")
    }
    #expect(eventTypes == ["capability_invoked", "conformance.echoed", "capability_result"])

    #expect(try client.shutdown() == Data(#"{"status":"stopped"}"#.utf8))
}

@Test func runtimeEmbedderMapsRuntimeOwnedResultsIntoPublicTypes() throws {
    let wasm = try fixtureBytes("client_bridge.wasm")
    let client = try WasmiHostBridgeClient(bundle: fixtureBundle(wasm: wasm))
    let runtime = RuntimeTraverseEmbedder(client: client)
    _ = try runtime.initialize(configJSON: Data("{}".utf8))

    #expect(try runtime.submit(try TraverseSubmission(targetID: "demo", inputJSON: Data("{}".utf8))) ==
        TraverseSubmissionResult(sessionID: "s1", status: "accepted"))
    #expect(try runtime.subscribe() == [TraverseRuntimeEvent(sequence: 1, targetID: "demo", status: "completed")])
    #expect(try runtime.shutdown() == Data(#"{"status":"stopped"}"#.utf8))
}

private struct NotAJSONObject: Error {}

private func jsonObject(from data: Data) throws -> [String: Any] {
    guard let object = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
        throw NotAJSONObject()
    }
    return object
}

private func fixtureBytes(_ name: String) throws -> [UInt8] {
    let url = try #require(Bundle.module.url(forResource: name, withExtension: nil, subdirectory: "Fixtures"))
    return Array(try Data(contentsOf: url))
}

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
