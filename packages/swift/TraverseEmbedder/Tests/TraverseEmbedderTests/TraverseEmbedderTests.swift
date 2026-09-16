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

@Test func wasmKitBridgeClientCopiesResultsAndDrainsEventsInOrder() throws {
    let wasm = try wat2wasm(clientBridgeWAT)
    let client = try WasmKitBridgeClient(bridge: WasmKitRuntimeBridge(bundle: fixtureBundle(wasm: wasm)))

    #expect(try client.initialize(configJSON: Data("{}".utf8)) == Data(#"{"status":"ready"}"#.utf8))
    #expect(try client.submit(requestJSON: Data(#"{"target_id":"demo"}"#.utf8)) == Data(#"{"session_id":"s1","status":"accepted"}"#.utf8))
    #expect(try client.nextEvent() == Data(#"{"sequence":1,"target_id":"demo","status":"completed"}"#.utf8))
    #expect(try client.nextEvent() == nil)
    #expect(try client.shutdown() == Data(#"{"status":"stopped"}"#.utf8))
}

@Test func wasmiHostBridgeClientUsesThePackagedProductionBoundary() throws {
    let wasm = try wat2wasm(clientBridgeWAT)
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
    let nestedCapability = try wat2wasm(nestedConformanceCapabilityWAT)
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
    let wasm = try wat2wasm(clientBridgeWAT)
    let client = try WasmKitBridgeClient(bridge: WasmKitRuntimeBridge(bundle: fixtureBundle(wasm: wasm)))
    let runtime = RuntimeTraverseEmbedder(client: client)
    _ = try runtime.initialize(configJSON: Data("{}".utf8))

    #expect(try runtime.submit(TraverseSubmission(targetID: "demo", inputJSON: Data("{}".utf8))) ==
        TraverseSubmissionResult(sessionID: "s1", status: "accepted"))
    #expect(try runtime.subscribe() == [TraverseRuntimeEvent(sequence: 1, targetID: "demo", status: "completed")])
    #expect(try runtime.shutdown() == Data(#"{"status":"stopped"}"#.utf8))
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

private let clientBridgeWAT = #"""
    (module
      (memory (export "memory") 1 16)
      (data (i32.const 512) "{\22status\22:\22ready\22}")
      (data (i32.const 544) "{\22session_id\22:\22s1\22,\22status\22:\22accepted\22}")
      (data (i32.const 608) "{\22sequence\22:1,\22target_id\22:\22demo\22,\22status\22:\22completed\22}")
      (data (i32.const 704) "{\22status\22:\22stopped\22}")
      (global $next (mut i32) (i32.const 0))
      (func (export "traverse_bridge_abi_version") (result i32) i32.const 10100)
      (func (export "traverse_alloc") (param i32) (result i32) i32.const 64)
      (func (export "traverse_dealloc") (param i32 i32))
      (func $result (param $d i32) (param $p i32) (param $n i32) (result i32)
        local.get $d local.get $p i32.store
        local.get $d i32.const 4 i32.add local.get $n i32.store
        i32.const 0)
      (func (export "traverse_init") (param i32 i32 i32) (result i32)
        local.get 2 i32.const 512 i32.const 18 call $result)
      (func (export "traverse_submit") (param i32 i32 i32) (result i32)
        local.get 2 i32.const 544 i32.const 39 call $result)
      (func (export "traverse_next_event") (param i32) (result i32)
        global.get $next i32.eqz
        if (result i32)
          i32.const 1 global.set $next
          local.get 0 i32.const 608 i32.const 54 call $result drop
          i32.const 1
        else i32.const 0 end)
      (func (export "traverse_cancel") (param i32 i32 i32) (result i32)
        local.get 2 i32.const 544 i32.const 39 call $result)
      (func (export "traverse_compatible_start") (param i32 i32 i32) (result i32)
        local.get 2 i32.const 544 i32.const 39 call $result)
      (func (export "traverse_compatible_stop") (param i32 i32 i32) (result i32)
        local.get 2 i32.const 544 i32.const 39 call $result)
      (func (export "traverse_compatible_kill") (param i32 i32 i32) (result i32)
        local.get 2 i32.const 544 i32.const 39 call $result)
      (func (export "traverse_shutdown") (param i32) (result i32)
        local.get 0 i32.const 704 i32.const 20 call $result))
    """#

/// A WASI-command capability that echoes stdin to stdout, then calls
/// `traverse_host::emit_event` with a fixed declared domain event — byte-
/// identical to `crates/traverse-runtime/tests/native_bridge_conformance.rs`'s
/// `NESTED_CAPABILITY_WAT`, so every host profile's conformance run exercises
/// the same nested-capability behavior.
private let nestedConformanceCapabilityWAT = #"""
    (module
      (import "wasi_snapshot_preview1" "fd_read"
        (func $fd_read (param i32 i32 i32 i32) (result i32)))
      (import "wasi_snapshot_preview1" "fd_write"
        (func $fd_write (param i32 i32 i32 i32) (result i32)))
      (import "traverse_host" "emit_event"
        (func $emit_event (param i32 i32) (result i32)))
      (memory (export "memory") 1)
      (data (i32.const 5000) "{\22event_id\22:\22conformance.echoed\22,\22version\22:\221.0.0\22,\22payload\22:{\22ok\22:true}}")
      (func (export "_start")
        (i32.store (i32.const 0) (i32.const 8))
        (i32.store (i32.const 4) (i32.const 1024))
        (drop (call $fd_read (i32.const 0) (i32.const 0) (i32.const 1) (i32.const 4100)))
        (i32.store (i32.const 0) (i32.const 8))
        (i32.store (i32.const 4) (i32.load (i32.const 4100)))
        (drop (call $fd_write (i32.const 1) (i32.const 0) (i32.const 1) (i32.const 4104)))
        (drop (call $emit_event (i32.const 5000) (i32.const 73)))
      )
    )
    """#

private struct NotAJSONObject: Error {}

private func jsonObject(from data: Data) throws -> [String: Any] {
    guard let object = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
        throw NotAJSONObject()
    }
    return object
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
