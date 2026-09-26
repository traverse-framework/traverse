import CryptoKit
import Foundation
import TraverseEmbedder

// Minimal macOS shell for examples/applications/audio-analysis. It supplies only
// host adapters, sends app commands, and prints runtime events; state transitions
// and artifact-to-capability input resolution remain inside runtime.wasm.

func repositoryRoot() -> URL {
    var url = URL(fileURLWithPath: #filePath)
    for _ in 0..<6 { url.deleteLastPathComponent() }
    return url
}

func jsonObject(_ data: Data) throws -> [String: Any] {
    guard let object = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
        throw TraverseBridgeError(status: -2, message: "example_invalid_json")
    }
    return object
}

func bytes(_ value: Any) throws -> Data {
    try JSONSerialization.data(withJSONObject: value, options: [.sortedKeys])
}

let root = repositoryRoot()
let appRoot = root.appendingPathComponent("examples/applications/audio-analysis")
let runtime = try Data(contentsOf: appRoot.appendingPathComponent("runtime/runtime.wasm"))
let manifest = try jsonObject(Data(contentsOf: appRoot.appendingPathComponent("app.manifest.json")))
let component = try jsonObject(Data(contentsOf: appRoot.appendingPathComponent("components/analyze/component.manifest.json")))
guard let stateMachine = manifest["state_machine"],
      let capabilityID = component["capability_id"] as? String,
      let capabilityVersion = component["capability_version"] as? String,
      let wasmPath = component["wasm_binary_path"] as? String else {
    throw TraverseBridgeError(status: -2, message: "example_manifest_incomplete")
}

let capabilityURL = appRoot
    .appendingPathComponent("components/analyze")
    .appendingPathComponent(wasmPath)
    .standardizedFileURL
let capability = try Data(contentsOf: capabilityURL)
let digest = SHA256.hash(data: runtime).map { String(format: "%02x", $0) }.joined()
let bundle = try TraverseBundle(rootURL: appRoot, runtimeWasmDigest: "sha256:\(digest)")
let embedder = try RuntimeTraverseEmbedder(bundle: bundle)

let initHeader: [String: Any] = [
    "app_id": manifest["app_id"] as? String ?? "audio-analysis",
    "host_placement_target": "local",
    "permitted_targets": ["local", "device"],
    "state_machine": stateMachine,
    "capability_id": capabilityID,
    "capability_version": capabilityVersion,
    "service_type": "stateless",
    "emits": [],
]
let header = try bytes(initHeader)
var initPayload = Data()
var length = UInt32(header.count).littleEndian
withUnsafeBytes(of: &length) { initPayload.append(contentsOf: $0) }
initPayload.append(header)
initPayload.append(capability)
_ = try embedder.initialize(configJSON: initPayload)

let staging = ArtifactStagingStore()
let adapters = AppleAudioInputAdapters(
    stage: { try await staging.stageArtifact($0, maxBytes: $1) },
    permissions: AVFoundationAudioPermissionDriver(),
    capture: AVFoundationAudioCaptureDriver())
_ = try embedder.registerHostConnectorAdapter(command: "audio.permission.request", adapter: adapters.requestPermission)
_ = try embedder.registerHostConnectorAdapter(command: "audio.capture", adapter: adapters.captureAudio)

func printEvents(_ events: [TraverseRuntimeEvent]) throws {
    let rendered = events.map { event -> [String: Any] in
        ["type": event.eventType ?? "legacy", "session_id": event.sessionID as Any, "data": String(decoding: event.output ?? Data("{}".utf8), as: UTF8.self)]
    }
    print(String(decoding: try bytes(rendered), as: UTF8.self))
}

/// Submits one app command, then waits for its host-connector round trip (if any) to
/// settle before returning. A registered adapter completes asynchronously, so submitting
/// the next command immediately would race the still-in-flight transition and be
/// rejected from the wrong state (#1562).
func submit(_ command: String, payload: [String: Any] = [:], sessionID: String?) async throws -> String {
    let result = try embedder.submit(TraverseAppCommand(command: command, payloadJSON: try bytes(payload), sessionID: sessionID))
    var quietPolls = 0
    while quietPolls < 5 {
        let events = try embedder.subscribe()
        if events.isEmpty {
            quietPolls += 1
        } else {
            try printEvents(events)
            quietPolls = 0
        }
        try await Task.sleep(nanoseconds: 10_000_000)
    }
    return result.sessionID
}

// A real app would wire these commands to buttons. This executable intentionally
// exposes no parallel UI state machine.
let session = try await submit("request_permission", sessionID: nil)
_ = try await submit(
    "capture_audio", payload: ["max_duration_ms": 5000, "max_bytes": 1_048_576], sessionID: session)
// Valid from the resulting "recorded" state (app.manifest.json's state machine);
// returns the app to "idle", demonstrating the full round trip.
_ = try await submit("reset", sessionID: session)
