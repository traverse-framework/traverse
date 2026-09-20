import Foundation
import Testing
@testable import TraverseEmbedder

/// Scripted runtime bridge: app commands get `appResponse`, everything else is a terminal.
private final class FakeBridge: TraverseBridgeClient, @unchecked Sendable {
    private let lock = NSLock()
    private var recorded: [[String: Any]] = []
    var appResponse = #"{"session_id":"s1","status":"accepted"}"#
    var terminalResponse = #"{"session_id":"s1","status":"accepted"}"#
    var failTerminals = false

    var submitted: [[String: Any]] {
        lock.lock(); defer { lock.unlock() }
        return recorded
    }

    func submit(requestJSON: Data) throws -> Data {
        let envelope = try #require(JSONSerialization.jsonObject(with: requestJSON) as? [String: Any])
        lock.lock()
        recorded.append(envelope)
        lock.unlock()
        if envelope["kind"] as? String == "app_command" { return Data(appResponse.utf8) }
        if failTerminals { throw TraverseBridgeError(status: -1, message: "runtime stopped") }
        return Data(terminalResponse.utf8)
    }

    /// Waits until `count` envelopes were submitted.
    func waitForSubmits(_ count: Int) async -> Bool {
        for _ in 0..<500 {
            if submitted.count >= count { return true }
            try? await Task.sleep(nanoseconds: 10_000_000)
        }
        return false
    }

    func initialize(configJSON: Data) throws -> Data { Data() }
    func cancel(requestJSON: Data) throws -> Data { Data() }
    func compatibleStart(requestJSON: Data) throws -> Data { Data() }
    func compatibleStop(requestJSON: Data) throws -> Data { Data() }
    func compatibleKill(requestJSON: Data) throws -> Data { Data() }
    func nextEvent() throws -> Data? { nil }
    func shutdown() throws -> Data { Data(#"{"status":"stopped"}"#.utf8) }
}

private final class ManualTimer: TraverseTimer, @unchecked Sendable {
    final class Handle: TraverseTimerHandle, @unchecked Sendable {
        var cancelled = false
        func cancel() { cancelled = true }
    }
    private let lock = NSLock()
    private var entries: [(delay: TimeInterval, callback: @Sendable () -> Void, handle: Handle)] = []

    var scheduled: [(delay: TimeInterval, callback: @Sendable () -> Void, handle: Handle)] {
        lock.lock(); defer { lock.unlock() }
        return entries
    }

    func schedule(after delay: TimeInterval, _ callback: @escaping @Sendable () -> Void) -> any TraverseTimerHandle {
        let handle = Handle()
        lock.lock()
        entries.append((delay, callback, handle))
        lock.unlock()
        return handle
    }
}

private func pending(command: String = "capture_audio", commandID: String = "cmd-1") -> String {
    #"{"session_id":"s1","status":"accepted","pending_host_connector":[{"command":"\#(command)","command_id":"\#(commandID)","session_id":"s1","payload":{"max_bytes":8}}],"pending_deadlines":[{"command_id":"\#(commandID)","session_id":"s1","deadline_ms":30000}]}"#
}

private func coordinator(_ bridge: FakeBridge, _ timer: ManualTimer = ManualTimer()) -> AppCommandCoordinator {
    AppCommandCoordinator(submit: { try bridge.submit(requestJSON: $0) }, timer: timer)
}

private func string(_ envelope: [String: Any], _ key: String) -> String? { envelope[key] as? String }

@Test func appCommandSendsTheEnvelopeAndReturnsTheRuntimeSession() throws {
    let bridge = FakeBridge()
    let result = try coordinator(bridge).submit(
        TraverseAppCommand(command: "start", payloadJSON: Data(#"{"a":1}"#.utf8), sessionID: "sess-9"))

    #expect(result == TraverseSubmissionResult(sessionID: "s1", status: "accepted"))
    let envelope = try #require(bridge.submitted.first)
    #expect(string(envelope, "kind") == "app_command")
    #expect(string(envelope, "command") == "start")
    #expect((envelope["payload"] as? [String: Any])?["a"] as? Int == 1)
    #expect(string(envelope, "session_id") == "sess-9")
}

@Test func appCommandOmitsSessionAndSurfacesRuntimeRejection() throws {
    let bridge = FakeBridge()
    bridge.appResponse = #"{"session_id":"s2","status":"rejected","error":"ambiguous"}"#
    let result = try coordinator(bridge).submit(TraverseAppCommand(command: "start"))
    #expect(result == TraverseSubmissionResult(sessionID: "s2", status: "rejected", error: "ambiguous"))
    #expect(bridge.submitted[0]["session_id"] == nil)

    bridge.appResponse = #"{"session_id":"s3","status":"rejected"}"#
    #expect(try coordinator(bridge).submit(TraverseAppCommand(command: "start")).error == nil)
}

@Test func appCommandRejectsMalformedRuntimeResults() throws {
    let bridge = FakeBridge()
    bridge.appResponse = #"{"status":"accepted"}"#
    #expect(throws: TraverseBridgeError.self) { try coordinator(bridge).submit(TraverseAppCommand(command: "go")) }
    bridge.appResponse = "not json"
    #expect(throws: TraverseBridgeError.self) { try coordinator(bridge).submit(TraverseAppCommand(command: "go")) }
    bridge.appResponse = "[1]"
    #expect(throws: TraverseBridgeError.self) { try coordinator(bridge).submit(TraverseAppCommand(command: "go")) }
}

@Test func registeredAdapterCompletesTheHostConnectorWait() async throws {
    let bridge = FakeBridge()
    bridge.appResponse = pending()
    let coordinator = coordinator(bridge)
    let seen = LockedBox<HostConnectorRequest?>(nil)
    _ = try coordinator.register(command: "capture_audio") { request in
        seen.value = request
        return HostConnectorResult(resultClass: "succeeded", payloadJSON: Data(#"{"artifact_ref":"artifact-1"}"#.utf8))
    }

    _ = try coordinator.submit(TraverseAppCommand(command: "record"))
    #expect(await bridge.waitForSubmits(2))

    let request = try #require(seen.value)
    #expect(request.command == "capture_audio" && request.commandID == "cmd-1" && request.sessionID == "s1")
    #expect((try JSONSerialization.jsonObject(with: request.payloadJSON) as? [String: Any])?["max_bytes"] as? Int == 8)
    let terminal = bridge.submitted[1]
    #expect(string(terminal, "kind") == "host_connector_result")
    #expect(string(terminal, "command_id") == "cmd-1" && string(terminal, "session_id") == "s1")
    #expect(string(terminal, "result_class") == "succeeded")
    #expect((terminal["payload"] as? [String: Any])?["artifact_ref"] as? String == "artifact-1")
}

@Test func missingAdapterFailsTargetIncompatible() async throws {
    let bridge = FakeBridge()
    bridge.appResponse = pending(command: "unregistered")
    _ = try coordinator(bridge).submit(TraverseAppCommand(command: "record"))
    #expect(await bridge.waitForSubmits(2))
    let terminal = bridge.submitted[1]
    #expect(string(terminal, "result_class") == "failed")
    #expect((terminal["payload"] as? [String: Any])?["code"] as? String == "target_incompatible")
}

@Test func adapterFailureBecomesExecutionFailedWithoutNativeDetail() async throws {
    struct Native: Error, CustomStringConvertible { var description: String { "/Users/x/device 0x1f" } }
    let bridge = FakeBridge()
    bridge.appResponse = pending()
    let coordinator = coordinator(bridge)
    _ = try coordinator.register(command: "capture_audio") { _ in throw Native() }
    _ = try coordinator.submit(TraverseAppCommand(command: "record"))
    #expect(await bridge.waitForSubmits(2))
    let terminal = bridge.submitted[1]
    #expect(string(terminal, "result_class") == "failed")
    #expect((terminal["payload"] as? [String: Any])?["code"] as? String == "execution_failed")
    #expect(!String(decoding: try JSONSerialization.data(withJSONObject: terminal), as: UTF8.self).contains("Users"))
}

@Test func deadlineIsRegisteredOnTheTimerPortAndFiresDeadlineFired() throws {
    let bridge = FakeBridge()
    bridge.appResponse = pending()
    let timer = ManualTimer()
    let coordinator = coordinator(bridge, timer)
    _ = try coordinator.register(command: "capture_audio") { _ in
        try await Task.sleep(nanoseconds: 60_000_000_000)
        return HostConnectorResult(resultClass: "succeeded")
    }
    _ = try coordinator.submit(TraverseAppCommand(command: "record"))

    let scheduled = try #require(timer.scheduled.first)
    #expect(timer.scheduled.count == 1 && scheduled.delay == 30)
    scheduled.callback()

    let terminal = try #require(bridge.submitted.last)
    #expect(string(terminal, "kind") == "deadline_fired")
    #expect(string(terminal, "command_id") == "cmd-1" && string(terminal, "session_id") == "s1")
    coordinator.stop()
}

@Test func malformedPendingEntriesAreSkipped() throws {
    let bridge = FakeBridge()
    bridge.appResponse = #"{"session_id":"s1","status":"accepted","pending_host_connector":[7,{"command":"x"}],"pending_deadlines":[{"command_id":"c"},{"command_id":"c","session_id":"s1"},{"command_id":"c","session_id":"s1","deadline_ms":"soon"}],"unrelated":1}"#
    let timer = ManualTimer()
    _ = try coordinator(bridge, timer).submit(TraverseAppCommand(command: "record"))
    #expect(timer.scheduled.isEmpty && bridge.submitted.count == 1)

    bridge.appResponse = #"{"session_id":"s1","status":"accepted","pending_host_connector":"no"}"#
    _ = try coordinator(bridge, timer).submit(TraverseAppCommand(command: "record"))
    #expect(bridge.submitted.count == 2)
}

@Test func negativeDeadlinesFireImmediately() throws {
    let bridge = FakeBridge()
    bridge.appResponse = #"{"session_id":"s1","status":"accepted","pending_deadlines":[{"command_id":"c","session_id":"s1","deadline_ms":-5}]}"#
    let timer = ManualTimer()
    _ = try coordinator(bridge, timer).submit(TraverseAppCommand(command: "record"))
    #expect(timer.scheduled.first?.delay == 0)
}

@Test func stopCancelsAdaptersAndDeadlinesAndDropsLateTerminals() async throws {
    let bridge = FakeBridge()
    bridge.appResponse = pending()
    let timer = ManualTimer()
    let coordinator = coordinator(bridge, timer)
    let started = LockedBox(false)
    let cancelled = LockedBox(false)
    _ = try coordinator.register(command: "capture_audio") { _ in
        started.value = true
        while !Task.isCancelled { try? await Task.sleep(nanoseconds: 5_000_000) }
        cancelled.value = true
        return HostConnectorResult(resultClass: "succeeded")
    }
    _ = try coordinator.submit(TraverseAppCommand(command: "record"))
    for _ in 0..<500 where !started.value { try? await Task.sleep(nanoseconds: 10_000_000) }

    coordinator.stop()
    for _ in 0..<500 where !cancelled.value { try? await Task.sleep(nanoseconds: 10_000_000) }
    #expect(cancelled.value)
    #expect(timer.scheduled[0].handle.cancelled)

    timer.scheduled[0].callback()
    try await Task.sleep(nanoseconds: 50_000_000)
    #expect(bridge.submitted.count == 1)

    // Nothing is registered or started after stop.
    _ = try coordinator.submit(TraverseAppCommand(command: "again"))
    #expect(timer.scheduled.count == 1)
}

@Test func terminalSubmitFailureIsSwallowedBecauseShutdownIsRuntimeOwned() async throws {
    let bridge = FakeBridge()
    bridge.appResponse = pending()
    bridge.failTerminals = true
    let coordinator = coordinator(bridge)
    _ = try coordinator.register(command: "capture_audio") { _ in HostConnectorResult(resultClass: "cancelled") }
    _ = try coordinator.submit(TraverseAppCommand(command: "record"))
    #expect(await bridge.waitForSubmits(2))
    #expect(string(bridge.submitted[1], "result_class") == "cancelled")
}

@Test func removingARegistrationOnlyAffectsTheSameAdapter() async throws {
    let bridge = FakeBridge()
    bridge.appResponse = pending()
    let coordinator = coordinator(bridge)
    let first = try coordinator.register(command: "capture_audio") { _ in HostConnectorResult(resultClass: "succeeded") }
    _ = try coordinator.register(command: "capture_audio") { _ in HostConnectorResult(resultClass: "failed") }
    first.remove()
    _ = try coordinator.submit(TraverseAppCommand(command: "record"))
    #expect(await bridge.waitForSubmits(2))
    #expect(string(bridge.submitted[1], "result_class") == "failed")

    #expect(throws: TraverseEmbedderError.self) {
        try coordinator.register(command: "  ") { _ in HostConnectorResult(resultClass: "succeeded") }
    }
}

@Test func removedAdapterFallsBackToTargetIncompatible() async throws {
    let bridge = FakeBridge()
    bridge.appResponse = pending()
    let coordinator = coordinator(bridge)
    try coordinator.register(command: "capture_audio") { _ in HostConnectorResult(resultClass: "succeeded") }.remove()
    _ = try coordinator.submit(TraverseAppCommand(command: "record"))
    #expect(await bridge.waitForSubmits(2))
    #expect((bridge.submitted[1]["payload"] as? [String: Any])?["code"] as? String == "target_incompatible")
}

@Test func systemTimerFiresOnceAndCanBeCancelled() async throws {
    let fired = LockedBox(0)
    _ = SystemTraverseTimer().schedule(after: -1) { fired.value += 1 }
    for _ in 0..<500 where fired.value == 0 { try? await Task.sleep(nanoseconds: 10_000_000) }
    #expect(fired.value == 1)

    let cancelled = LockedBox(false)
    SystemTraverseTimer().schedule(after: 0.2) { cancelled.value = true }.cancel()
    try await Task.sleep(nanoseconds: 400_000_000)
    #expect(!cancelled.value)
}

private final class LockedBox<Value>: @unchecked Sendable {
    private let lock = NSLock()
    private var stored: Value
    init(_ value: Value) { stored = value }
    var value: Value {
        get { lock.lock(); defer { lock.unlock() }; return stored }
        set { lock.lock(); stored = newValue; lock.unlock() }
    }
}
