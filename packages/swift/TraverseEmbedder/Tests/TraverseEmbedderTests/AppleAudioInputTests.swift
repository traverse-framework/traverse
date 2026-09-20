import Foundation
import Testing
@testable import TraverseEmbedder

// MARK: - Test doubles

private struct FixedPermissions: AudioPermissionDriver {
    let state: AudioPermissionState
    func status() async -> AudioPermissionState { state }
    func request() async -> AudioPermissionState { state }
}

private struct NativeFailure: Error, CustomStringConvertible {
    var description: String { "open failed: /Users/example/Library/Audio/Built-In Mic (device 0x1f) token=abc123" }
}

private enum Behavior: Sendable {
    case bytes(Data)
    case fail(AudioCaptureFailure)
    case native
    case waitForCancel
}

/// Scripted capture driver. Records the request it received.
private final class ScriptedCapture: AudioCaptureDriver, @unchecked Sendable {
    private let lock = NSLock()
    private var received: [AudioCaptureRequest] = []
    var behavior: Behavior

    init(_ behavior: Behavior) { self.behavior = behavior }

    var requests: [AudioCaptureRequest] { lock.withLock { received } }

    func capture(_ request: AudioCaptureRequest) async throws -> Data {
        lock.withLock { received.append(request) }
        switch behavior {
        case .bytes(let data): return data
        case .fail(let failure): throw failure
        case .native: throw NativeFailure()
        case .waitForCancel:
            while !Task.isCancelled { try await Task.sleep(nanoseconds: 2_000_000) }
            throw CancellationError()
        }
    }
}

private func adapters(
    permission: AudioPermissionState = .granted,
    capture: ScriptedCapture,
    stage: @escaping AudioArtifactStager = { _, _ in "artifact-1" }
) -> AppleAudioInputAdapters {
    AppleAudioInputAdapters(stage: stage, permissions: FixedPermissions(state: permission), capture: capture)
}

private func request(_ payload: String = #"{"max_duration_ms":5000,"max_bytes":1024}"#, command: String = "capture_audio") -> HostConnectorRequest {
    HostConnectorRequest(command: command, commandID: "cmd-1", sessionID: "s1", payloadJSON: Data(payload.utf8))
}

private func payload(_ result: HostConnectorResult) -> [String: Any] {
    (try? JSONSerialization.jsonObject(with: result.payloadJSON)) as? [String: Any] ?? [:]
}

private func code(_ result: HostConnectorResult) -> String? { payload(result)["error_code"] as? String }

// MARK: - Permission

@Test func permissionGrantedReportsOnlyTheNonSecretState() async throws {
    let subject = adapters(permission: .granted, capture: ScriptedCapture(.native))
    let result = try await subject.requestPermission(request(#"{}"#, command: "request_permission"))
    #expect(result.resultClass == "succeeded")
    #expect(payload(result) as NSDictionary == ["permission_state": "granted"] as NSDictionary)

    let prompt = try await adapters(permission: .promptRequired, capture: ScriptedCapture(.native))
        .requestPermission(request(#"{}"#))
    #expect(payload(prompt)["permission_state"] as? String == "prompt_required")
}

@Test func deniedAndUnavailablePermissionAreTypedFailures() async throws {
    let denied = try await adapters(permission: .denied, capture: ScriptedCapture(.native)).requestPermission(request(#"{}"#))
    #expect(denied.resultClass == "failed")
    #expect(code(denied) == "policy_denied")

    let unavailable = try await adapters(permission: .unavailable, capture: ScriptedCapture(.native)).requestPermission(request(#"{}"#))
    #expect(unavailable.resultClass == "failed")
    #expect(code(unavailable) == "unavailable")
}

// MARK: - Capture

@Test func captureStagesBytesAndReturnsOnlyAnOpaqueRef() async throws {
    let store = ArtifactStagingStore()
    let capture = ScriptedCapture(.bytes(Data([1, 2, 3])))
    let subject = adapters(capture: capture, stage: { try await store.stageArtifact($0, maxBytes: $1) })

    let result = try await subject.captureAudio(request())
    #expect(result.resultClass == "succeeded")
    #expect(payload(result) as NSDictionary == ["artifact_ref": "artifact-1"] as NSDictionary)
    #expect(try await store.readArtifact("artifact-1", maxBytes: 1024) == Data([1, 2, 3]))
    #expect(capture.requests == [AudioCaptureRequest(correlationID: "cmd-1", maxDurationMs: 5000, maxBytes: 1024)])
}

@Test func correlationIDFromThePayloadWinsOverTheCommandID() async throws {
    let capture = ScriptedCapture(.bytes(Data([1])))
    _ = try await adapters(capture: capture).captureAudio(
        request(#"{"max_duration_ms":10,"max_bytes":8,"correlation_id":"corr-7"}"#))
    _ = try await adapters(capture: capture).captureAudio(
        request(#"{"max_duration_ms":10,"max_bytes":8,"correlation_id":""}"#))
    #expect(capture.requests.map(\.correlationID) == ["corr-7", "cmd-1"])
}

@Test func nonPositiveOrMalformedLimitsFailInputLimitExceededWithoutCapturing() async throws {
    let capture = ScriptedCapture(.bytes(Data([1])))
    let subject = adapters(capture: capture)
    for body in [
        #"{}"#, #"{"max_duration_ms":5000}"#, #"{"max_bytes":8}"#,
        #"{"max_duration_ms":0,"max_bytes":8}"#, #"{"max_duration_ms":5,"max_bytes":-1}"#,
        #"{"max_duration_ms":"5","max_bytes":8}"#, #"{"max_duration_ms":1e300,"max_bytes":8}"#,
        "not json", "[1]",
    ] {
        let result = try await subject.captureAudio(request(body))
        #expect(result.resultClass == "failed", "\(body)")
        #expect(code(result) == "input_limit_exceeded", "\(body)")
    }
    #expect(capture.requests.isEmpty)
}

@Test func emptyOversizedOrRejectedStagingFailInputLimitExceeded() async throws {
    let empty = try await adapters(capture: ScriptedCapture(.bytes(Data()))).captureAudio(request())
    #expect(code(empty) == "input_limit_exceeded")

    let oversized = try await adapters(capture: ScriptedCapture(.bytes(Data(count: 2048)))).captureAudio(request())
    #expect(code(oversized) == "input_limit_exceeded")

    struct Rejected: Error {}
    let rejected = try await adapters(capture: ScriptedCapture(.bytes(Data([1]))), stage: { _, _ in throw Rejected() })
        .captureAudio(request())
    #expect(code(rejected) == "input_limit_exceeded")
}

@Test func nonOpaqueArtifactRefsAreRefusedNeverPublished() async throws {
    for leaky in ["/tmp/capture-0001.wav", "https://example.invalid/a", "C:\\audio\\a.wav"] {
        let result = try await adapters(capture: ScriptedCapture(.bytes(Data([1]))), stage: { _, _ in leaky })
            .captureAudio(request())
        #expect(result.resultClass == "failed")
        #expect(code(result) == "unavailable")
        #expect(!String(decoding: result.payloadJSON, as: UTF8.self).contains(leaky))
    }
}

@Test func driverFailuresMapToTypedPublicCodes() async throws {
    let expectations: [(Behavior, String, String)] = [
        (.fail(.limitExceeded), "failed", "input_limit_exceeded"),
        (.fail(.policyDenied), "failed", "policy_denied"),
        (.fail(.cancelled), "cancelled", "cancelled"),
        (.fail(.unavailable), "failed", "unavailable"),
        (.native, "failed", "unavailable"),
    ]
    for (behavior, resultClass, publicCode) in expectations {
        let result = try await adapters(capture: ScriptedCapture(behavior)).captureAudio(request())
        #expect(result.resultClass == resultClass)
        #expect(code(result) == publicCode)
    }
}

@Test func nativeErrorTextNeverAppearsInPublicResults() async throws {
    let result = try await adapters(capture: ScriptedCapture(.native)).captureAudio(request())
    let text = String(decoding: result.payloadJSON, as: UTF8.self)
    for needle in ["/Users/", "Built-In", "device 0x", "token=", "abc123", "Library/Audio"] {
        #expect(!text.contains(needle))
    }
}

// MARK: - Cancellation

private func waitForRequests(_ capture: ScriptedCapture) async {
    for _ in 0..<500 where capture.requests.isEmpty { try? await Task.sleep(nanoseconds: 10_000_000) }
}

@Test func witCancelStopsTheInFlightCaptureAsCancelled() async throws {
    let capture = ScriptedCapture(.waitForCancel)
    let subject = adapters(capture: capture)
    let running = Task { try await subject.captureAudio(request()) }
    await waitForRequests(capture)

    subject.cancel(correlationID: "unknown")
    subject.cancel(correlationID: "cmd-1")
    let result = try await running.value
    #expect(result.resultClass == "cancelled")
    #expect(code(result) == "cancelled")
    subject.cancel(correlationID: "cmd-1")
}

@Test func cancellingTheHostTaskCancelsTheCapture() async throws {
    let capture = ScriptedCapture(.waitForCancel)
    let subject = adapters(capture: capture)
    let running = Task { try await subject.captureAudio(request()) }
    await waitForRequests(capture)
    running.cancel()
    let result = try await running.value
    #expect(result.resultClass == "cancelled")
}

// MARK: - Platform drivers (never prompt, never record)

#if canImport(AVFoundation)
@Test func avFoundationPermissionStatusIsReadWithoutPrompting() async {
    let state = await AVFoundationAudioPermissionDriver().status()
    #expect([AudioPermissionState.granted, .denied, .promptRequired, .unavailable].contains(state))
}
#endif

@Test func adaptersRegisterAsHostConnectorAdapters() {
    let subject = adapters(capture: ScriptedCapture(.native))
    let permission: HostConnectorAdapter = subject.requestPermission
    let capture: HostConnectorAdapter = subject.captureAudio
    _ = (permission, capture)
}

// MARK: - Spec 140 conformance fixture (adapter-level cases)

private func fixtureURL() -> URL {
    var url = URL(fileURLWithPath: #filePath)
    for _ in 0..<6 { url.deleteLastPathComponent() }
    return url.appendingPathComponent("fixtures/cross-host/host-authority-audio-input-v1/fixture.json")
}

private func loadFixture() throws -> [String: Any] {
    try #require(JSONSerialization.jsonObject(with: Data(contentsOf: fixtureURL())) as? [String: Any])
}

private func failureBehavior(_ witClass: String) -> Behavior {
    switch witClass {
    case "limit-exceeded": .fail(.limitExceeded)
    case "policy-denied": .fail(.policyDenied)
    case "cancelled": .fail(.cancelled)
    default: .native
    }
}

/// Permission driver scripted by the fixture's `request-permission` return.
private struct FixturePermissions: AudioPermissionDriver {
    let state: AudioPermissionState
    func status() async -> AudioPermissionState { state }
    func request() async -> AudioPermissionState { state }
}

@Test func appleAdapterPassesTheSpec140ConformanceFixture() async throws {
    let fixture = try loadFixture()
    let mapping = try #require(fixture["adapter_failure_class_mapping"] as? [String: String])
    let forbidden = try #require((fixture["redaction"] as? [String: Any])?["forbidden_public_substrings"] as? [String])
    let cases = try #require(fixture["cases"] as? [[String: Any]])
    var exercised = 0

    for fixtureCase in cases {
        let caseID = try #require(fixtureCase["id"] as? String)
        let steps = try #require(fixtureCase["steps"] as? [[String: Any]])
        for step in steps {
            let calls = try #require(step["adapter_calls"] as? [[String: Any]])
            guard let call = calls.first, let function = call["function"] as? String else { continue }
            let expected = try #require(step["expected"] as? [String: Any])
            let command = try #require(step["command"] as? [String: Any])
            let returns = try #require(call["returns"] as? [String: Any])
            let payloadObject = command["payload"] ?? [String: Any]()
            let payloadJSON = try JSONSerialization.data(withJSONObject: payloadObject)
            let hostRequest = HostConnectorRequest(
                command: try #require(command["command"] as? String),
                commandID: try #require(command["command_id"] as? String),
                sessionID: "s1", payloadJSON: payloadJSON)

            let result: HostConnectorResult
            switch function {
            case "request-permission":
                let state = try #require(returns["ok"] as? String)
                let permissionState = try #require(AudioPermissionState(rawValue: state.replacingOccurrences(of: "-", with: "_")))
                let subject = AppleAudioInputAdapters(
                    stage: { _, _ in "unused" }, permissions: FixturePermissions(state: permissionState),
                    capture: ScriptedCapture(.native))
                result = try await subject.requestPermission(hostRequest)
            case "capture":
                let cancelledByHost = call["cancelled_by_host"] as? Bool == true
                var behavior = Behavior.native
                var stagedRef = "unused"
                if let ok = returns["ok"] as? [String: Any] {
                    behavior = .bytes(Data(count: try #require(ok["size-bytes"] as? Int) > 0 ? 8 : 0))
                    stagedRef = try #require(ok["artifact-ref"] as? String)
                } else if let err = returns["err"] as? [String: Any] {
                    let wit = try #require(err["class"] as? String)
                    behavior = cancelledByHost ? .waitForCancel : failureBehavior(wit)
                }
                let driver = ScriptedCapture(behavior)
                let staged = stagedRef
                let subject = AppleAudioInputAdapters(
                    stage: { _, _ in staged }, permissions: FixturePermissions(state: .granted), capture: driver)
                if cancelledByHost {
                    let running = Task { try await subject.captureAudio(hostRequest) }
                    await waitForRequests(driver)
                    subject.cancel(correlationID: hostRequest.commandID)
                    result = try await running.value
                } else {
                    result = try await subject.captureAudio(hostRequest)
                }
            default:
                Issue.record("\(caseID): unknown adapter function \(function)")
                continue
            }

            let body = payload(result)
            let expectedClass = try #require(expected["result_class"] as? String)
            #expect(result.resultClass == expectedClass, "\(caseID)")
            if expectedClass == "succeeded" {
                if let ref = expected["artifact_ref"] as? String { #expect(body["artifact_ref"] as? String == ref, "\(caseID)") }
                if let state = expected["permission_state"] as? String { #expect(body["permission_state"] as? String == state, "\(caseID)") }
            } else {
                let expectedCode = try #require(expected["error_code"] as? String)
                #expect(body["error_code"] as? String == expectedCode, "\(caseID)")
                if let wit = (returns["err"] as? [String: Any])?["class"] as? String {
                    #expect(mapping[wit] == expectedCode, "\(caseID): WIT class \(wit)")
                }
            }
            let publicText = String(decoding: result.payloadJSON, as: UTF8.self)
            for needle in forbidden { #expect(!publicText.contains(needle), "\(caseID) leaks \(needle)") }
            exercised += 1
        }
    }
    // Every case with adapter calls in the fixture must have been executed.
    #expect(exercised >= 12)
}
