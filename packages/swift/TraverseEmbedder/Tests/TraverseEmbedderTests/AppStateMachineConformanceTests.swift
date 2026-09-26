import CryptoKit
import Foundation
import Testing
@testable import TraverseEmbedder

// Cross-host ordered-event conformance (Spec 139 / Spec 140 FR-013, #1502).
//
// Runs `fixtures/cross-host/app-state-machine-events-v1` against the real `runtime.wasm`
// through the production `WasmiHostBridgeClient` and compares each step with `golden.json`,
// which the Rust reference (`crates/traverse-runtime/tests/app_state_machine_conformance.rs`)
// generated. Skipped unless TRAVERSE_NATIVE_ARTIFACT_ROOT points at a directory containing
// `runtime/runtime.wasm`, exactly like the other real-artifact tests.

/// Normalizes runtime-assigned ids to `$S<n>` / `$C<n>` by first appearance.
private struct Placeholders {
    var sessions: [String] = []
    var commands: [String] = []

    private static func name(_ list: inout [String], _ prefix: String, _ id: String) -> String {
        if let index = list.firstIndex(of: id) { return "\(prefix)\(index + 1)" }
        list.append(id)
        return "\(prefix)\(list.count)"
    }

    mutating func normalize(_ value: Any) -> Any {
        if let object = value as? [String: Any] {
            var out: [String: Any] = [:]
            for (key, inner) in object {
                switch (key, inner as? String) {
                case ("session_id", let id?): out[key] = Self.name(&sessions, "$S", id)
                case ("command_id", let id?): out[key] = Self.name(&commands, "$C", id)
                default: out[key] = normalize(inner)
                }
            }
            return out
        }
        if let array = value as? [Any] { return array.map { normalize($0) } }
        return value
    }

    static func resolve(_ list: [String], _ placeholder: String, _ prefix: String) -> String {
        list[(Int(placeholder.dropFirst(prefix.count)) ?? 1) - 1]
    }
}

private func repoRoot() -> URL {
    var url = URL(fileURLWithPath: #filePath)
    for _ in 0..<6 { url.deleteLastPathComponent() }
    return url
}

private func loadJSON(_ name: String) throws -> [String: Any] {
    let url = repoRoot().appendingPathComponent("fixtures/cross-host/app-state-machine-events-v1/\(name)")
    return try #require(JSONSerialization.jsonObject(with: try Data(contentsOf: url)) as? [String: Any])
}

private func encode(_ value: Any) throws -> Data {
    try JSONSerialization.data(withJSONObject: value, options: [.sortedKeys, .fragmentsAllowed])
}

private func same(_ a: Any?, _ b: Any?) -> Bool {
    switch (a, b) {
    case (nil, nil): return true
    case let (a?, b?): return (a as AnyObject).isEqual(b)
    default: return false
    }
}

private func describe(_ value: Any?) -> String {
    guard let value, let data = try? encode(value) else { return "<none>" }
    return String(decoding: data, as: UTF8.self)
}

/// Runs one scenario through the bridge client; returns per-step transcripts.
private func runScenario(client: any TraverseBridgeClient, initPayload: Data, scenario: [String: Any]) throws -> [[String: Any]] {
    let initResponse = try #require(JSONSerialization.jsonObject(with: try client.initialize(configJSON: initPayload)) as? [String: Any])
    #expect(initResponse["status"] as? String == "ready")

    var names = Placeholders()
    var pending: [(session: String, command: String)] = []
    var transcript: [[String: Any]] = []
    let steps = try #require(scenario["steps"] as? [[String: Any]])

    for (index, step) in steps.enumerated() {
        let (kind, spec) = try #require(step.first as? (String, [String: Any]))
        func targetWait() throws -> (session: String, command: String) {
            if let placeholder = spec["command"] as? String {
                let id = Placeholders.resolve(names.commands, placeholder, "$C")
                return try #require(pending.first { $0.command == id })
            }
            return try #require(pending.last)
        }
        let request: [String: Any]
        switch kind {
        case "submit":
            var envelope: [String: Any] = ["kind": "app_command", "command": spec["command"] as Any, "payload": spec["payload"] as Any]
            if let placeholder = spec["session"] as? String {
                envelope["session_id"] = Placeholders.resolve(names.sessions, placeholder, "$S")
            }
            request = envelope
        case "complete":
            let wait = try targetWait()
            request = ["kind": "host_connector_result", "command_id": wait.command, "session_id": wait.session,
                       "result_class": spec["result_class"] as Any, "payload": spec["payload"] as Any]
        case "fire_deadline":
            let wait = try targetWait()
            request = ["kind": "deadline_fired", "command_id": wait.command, "session_id": wait.session]
        default:
            Issue.record("unknown step kind \(kind)")
            continue
        }

        var entry: [String: Any] = ["step": index, "kind": kind]
        do {
            let response = try #require(JSONSerialization.jsonObject(with: try client.submit(requestJSON: try encode(request))) as? [String: Any])
            entry["guest_status"] = 0
            for wait in (response["pending_host_connector"] as? [[String: Any]]) ?? [] {
                pending.append((wait["session_id"] as? String ?? "", wait["command_id"] as? String ?? ""))
            }
            entry["response"] = names.normalize(response)
        } catch let error as TraverseBridgeError {
            entry["guest_status"] = Int(error.status)
            entry["bridge_error"] = error.message
        }
        var events: [Any] = []
        while let bytes = try client.nextEvent() {
            events.append(try JSONSerialization.jsonObject(with: bytes))
        }
        entry["events"] = names.normalize(events)
        transcript.append(entry)
    }
    return transcript
}

@Test func swiftHostReproducesTheGoldenOrderedEventLog() throws {
    guard let rootPath = ProcessInfo.processInfo.environment["TRAVERSE_NATIVE_ARTIFACT_ROOT"] else { return }
    let root = URL(fileURLWithPath: rootPath)
    let runtime = try Data(contentsOf: root.appendingPathComponent("runtime/runtime.wasm"))
    let bundle = try TraverseBundle(
        rootURL: root,
        runtimeWasmDigest: "sha256:" + SHA256.hash(data: runtime).map { String(format: "%02x", $0) }.joined())

    let fixture = try loadJSON("fixture.json")
    let golden = try loadJSON("golden.json")
    let goldenScenarios = try #require(golden["scenarios"] as? [String: Any])
    let headerBytes = try encode(try #require(fixture["init_header"]))
    var initPayload = Data()
    var length = UInt32(headerBytes.count).littleEndian
    withUnsafeBytes(of: &length) { initPayload.append(contentsOf: $0) }
    initPayload.append(headerBytes)

    var divergences: [String] = []
    for scenario in try #require(fixture["scenarios"] as? [[String: Any]]) {
        let id = try #require(scenario["id"] as? String)
        let client = try WasmiHostBridgeClient(bundle: bundle, limits: try TraverseHostLimits(fuelPerInvocation: 50_000_000))
        let actual = try runScenario(client: client, initPayload: initPayload, scenario: scenario)
        let want = try #require(goldenScenarios[id] as? [[String: Any]])
        guard actual.count == want.count else {
            divergences.append("scenario \(id): \(actual.count) steps, expected \(want.count)")
            continue
        }
        for (index, (got, expected)) in zip(actual, want).enumerated() {
            for field in ["kind", "guest_status", "response"] {
                // A rejected submit throws in the Swift host, so its response body is unavailable.
                if field == "response", got["bridge_error"] != nil { continue }
                if !same(got[field], expected[field]) {
                    divergences.append("scenario \(id) step \(index) \(field): expected \(describe(expected[field])), got \(describe(got[field]))")
                }
            }
            let gotEvents = (got["events"] as? [Any]) ?? []
            let wantEvents = (expected["events"] as? [Any]) ?? []
            for eventIndex in 0..<max(gotEvents.count, wantEvents.count) {
                let g = eventIndex < gotEvents.count ? gotEvents[eventIndex] : nil
                let w = eventIndex < wantEvents.count ? wantEvents[eventIndex] : nil
                if !same(g, w) {
                    divergences.append("scenario \(id) step \(index) event \(eventIndex): expected \(describe(w)), got \(describe(g))")
                }
            }
        }
    }
    #expect(divergences.isEmpty, "\(divergences.joined(separator: "\n"))")
}

// MARK: - Public API level

private final class Box<Value>: @unchecked Sendable {
    private let lock = NSLock()
    private var stored: Value
    init(_ value: Value) { stored = value }
    var value: Value {
        get { lock.lock(); defer { lock.unlock() }; return stored }
        set { lock.lock(); stored = newValue; lock.unlock() }
    }
}

private final class HandleBox: TraverseTimerHandle, @unchecked Sendable {
    func cancel() {}
}

private final class ScriptedTimer: TraverseTimer, @unchecked Sendable {
    private let callbacks = Box<[@Sendable () -> Void]>([])
    func schedule(after delay: TimeInterval, _ callback: @escaping @Sendable () -> Void) -> any TraverseTimerHandle {
        callbacks.value.append(callback)
        return HandleBox()
    }
    func fireLatest() -> Bool {
        guard let callback = callbacks.value.last else { return false }
        callback()
        return true
    }
}

private enum WaitAction: Sendable {
    case complete(resultClass: String, payload: Data)
    case deadline
}

/// The public API cannot inject a raw terminal or observe a rejected submit's response, so only
/// scenarios expressible through adapters and the timer port run at this level.
private func expressible(_ goldenSteps: [[String: Any]], _ steps: [[String: Any]]) -> Bool {
    for (got, step) in zip(goldenSteps, steps) {
        if (got["guest_status"] as? Int) != 0 { return false }
        if let spec = step["complete"] as? [String: Any], spec["command"] != nil { return false }
        if let spec = step["fire_deadline"] as? [String: Any], spec["command"] != nil { return false }
    }
    return true
}

@Test func swiftPublicSubscribeDeliversTheGoldenAppEventsInOrder() async throws {
    guard let rootPath = ProcessInfo.processInfo.environment["TRAVERSE_NATIVE_ARTIFACT_ROOT"] else { return }
    let root = URL(fileURLWithPath: rootPath)
    let runtime = try Data(contentsOf: root.appendingPathComponent("runtime/runtime.wasm"))
    let bundle = try TraverseBundle(
        rootURL: root,
        runtimeWasmDigest: "sha256:" + SHA256.hash(data: runtime).map { String(format: "%02x", $0) }.joined())
    let fixture = try loadJSON("fixture.json")
    let goldenScenarios = try #require(try loadJSON("golden.json")["scenarios"] as? [String: Any])
    let headerBytes = try encode(try #require(fixture["init_header"]))
    var initPayload = Data()
    var length = UInt32(headerBytes.count).littleEndian
    withUnsafeBytes(of: &length) { initPayload.append(contentsOf: $0) }
    initPayload.append(headerBytes)
    let machine = try #require((fixture["init_header"] as? [String: Any])?["state_machine"] as? [String: Any])
    let hostCommands = ((machine["states"] as? [[String: Any]]) ?? []).compactMap {
        ($0["invoke"] as? [String: Any])?["host_connector"] as? String
    }

    var divergences: [String] = []
    var exercised = 0
    for scenario in try #require(fixture["scenarios"] as? [[String: Any]]) {
        let id = try #require(scenario["id"] as? String)
        let steps = try #require(scenario["steps"] as? [[String: Any]])
        let goldenSteps = try #require(goldenScenarios[id] as? [[String: Any]])
        guard expressible(goldenSteps, steps) else { continue }
        exercised += 1

        let actions = Box<[WaitAction]>(try steps.compactMap { step in
            if let spec = step["complete"] as? [String: Any] {
                return .complete(
                    resultClass: try #require(spec["result_class"] as? String),
                    payload: try encode(spec["payload"] ?? [String: Any]()))
            }
            return step["fire_deadline"] != nil ? .deadline : nil
        })
        let timer = ScriptedTimer()
        let client = try WasmiHostBridgeClient(bundle: bundle, limits: try TraverseHostLimits(fuelPerInvocation: 50_000_000))
        let embedder = RuntimeTraverseEmbedder(client: client, timer: timer)
        _ = try embedder.initialize(configJSON: initPayload)
        for command in Set(hostCommands) {
            _ = try embedder.registerHostConnectorAdapter(command: command) { _ in
                let next = actions.value.isEmpty ? nil : actions.value.removeFirst()
                switch next {
                case .complete(let resultClass, let payload)?:
                    return HostConnectorResult(resultClass: resultClass, payloadJSON: payload)
                default:
                    // Deadline scenario: stay in flight until shutdown cancels the task.
                    while !Task.isCancelled { try await Task.sleep(nanoseconds: 5_000_000) }
                    throw CancellationError()
                }
            }
        }

        var names = Placeholders()
        var sessionID: String?
        var received: [TraverseRuntimeEvent] = []
        var expectedTotal = 0
        for (index, step) in steps.enumerated() {
            expectedTotal += ((goldenSteps[index]["events"] as? [Any]) ?? []).count
            if let spec = step["submit"] as? [String: Any] {
                let command = try TraverseAppCommand(
                    command: try #require(spec["command"] as? String),
                    payloadJSON: try encode(spec["payload"] ?? [String: Any]()),
                    sessionID: spec["session"] is String ? sessionID : nil)
                let result = try embedder.submit(command)
                #expect(result.status == "accepted", "\(id) step \(index)")
                sessionID = sessionID ?? result.sessionID
            } else if step["fire_deadline"] != nil {
                #expect(timer.fireLatest(), "\(id) step \(index): no deadline was registered")
            }
            for _ in 0..<500 where received.count < expectedTotal {
                received += try embedder.subscribe()
                if received.count < expectedTotal { try await Task.sleep(nanoseconds: 10_000_000) }
            }
            // Adapters complete asynchronously, so a step may already include later events; only
            // wait for "at least" here and require the exact total below.
            if received.count < expectedTotal {
                divergences.append("scenario \(id) step \(index): only \(received.count) of \(expectedTotal) events arrived")
            }
        }
        // Give any surplus event time to arrive before checking the exact total.
        try await Task.sleep(nanoseconds: 50_000_000)
        received += try embedder.subscribe()
        if received.count != expectedTotal {
            divergences.append("scenario \(id): \(received.count) events, expected \(expectedTotal)")
        }
        _ = try embedder.shutdown()

        let wanted = goldenSteps.flatMap { ($0["events"] as? [[String: Any]]) ?? [] }
        for (eventIndex, (got, want)) in zip(received, wanted).enumerated() {
            let output = try JSONSerialization.jsonObject(with: got.output ?? Data("{}".utf8))
            let actual = names.normalize([
                "type": got.eventType as Any, "session_id": got.sessionID as Any, "data": output,
            ] as [String: Any])
            if !same(actual, want) {
                divergences.append("scenario \(id) event \(eventIndex): expected \(describe(want)), got \(describe(actual))")
            }
        }
        #expect(received.map(\.sequence) == Array(1...max(1, received.count)).prefix(received.count).map { $0 },
                "\(id): sequences must be 1..n in arrival order")
    }
    #expect(exercised >= 5, "expected the adapter-expressible scenarios to run")
    #expect(divergences.isEmpty, "\(divergences.joined(separator: "\n"))")
}

// MARK: - Event mapping (no artifact needed)

private final class QueuedEvents: TraverseBridgeClient, @unchecked Sendable {
    private let lock = NSLock()
    private var queue: [Data]
    init(_ events: [String]) { queue = events.map { Data($0.utf8) } }
    func nextEvent() throws -> Data? { lock.withLock { queue.isEmpty ? nil : queue.removeFirst() } }
    func initialize(configJSON: Data) throws -> Data { Data() }
    func submit(requestJSON: Data) throws -> Data { Data() }
    func cancel(requestJSON: Data) throws -> Data { Data() }
    func compatibleStart(requestJSON: Data) throws -> Data { Data() }
    func compatibleStop(requestJSON: Data) throws -> Data { Data() }
    func compatibleKill(requestJSON: Data) throws -> Data { Data() }
    func shutdown() throws -> Data { Data() }
}

@Test func subscribeMapsAppLifecycleEventsAndKeepsLegacyEvents() throws {
    let embedder = RuntimeTraverseEmbedder(client: QueuedEvents([
        #"{"type":"state_changed","session_id":"s1","data":{"state":"ready"}}"#,
        #"{"sequence":9,"target_id":"demo","status":"completed"}"#,
        #"{"type":"host_connector_failed","session_id":"s1","data":{"error_code":"policy_denied"}}"#,
        #"{"type":"error","session_id":"s1","data":{"code":"no_matching_transition"}}"#,
        #"{"type":"something_new","session_id":"s1"}"#,
    ]))
    let events = try embedder.subscribe()
    #expect(events.count == 5)
    #expect(events[0].eventType == "state_changed" && events[0].sessionID == "s1" && events[0].sequence == 1)
    #expect(events[0].targetID == "app_command" && events[0].status == "emitted" && events[0].errorData == nil)
    #expect(String(decoding: events[0].output ?? Data(), as: UTF8.self) == #"{"state":"ready"}"#)
    // Legacy bridge events keep their own numbering and shape.
    #expect(events[1] == TraverseRuntimeEvent(sequence: 9, targetID: "demo", status: "completed"))
    #expect(events[2].eventType == "host_connector_failed" && events[2].sequence == 2)
    #expect(events[3].eventType == "error" && events[3].errorData == events[3].output)
    // Unknown runtime types surface as `error`, as on web; a missing `data` becomes `{}`.
    #expect(events[4].eventType == "error" && String(decoding: events[4].output ?? Data(), as: UTF8.self) == "{}")
    #expect(try embedder.subscribe().isEmpty)
}

/// Regression test for #1562: a real consumer (e.g. Callweave) constructs
/// `WasmiHostBridgeClient`/`RuntimeTraverseEmbedder` with the plain default
/// constructor, not the explicit `fuelPerInvocation: 50_000_000` the other
/// tests in this file already pass. `TraverseHostLimits`'s default was
/// `1_000_000` — enough fuel to run a toy fixture, but not the real v0.13
/// app-state `runtime.wasm`'s `init`, which trapped with `OutOfFuel` before
/// `audio.capture` (or any host connector) was ever invoked. This exercises
/// the full happy-path host-connector sequence — `request_permission` →
/// `ready` → `capture_audio` → `capturing` → capture terminal → `recorded`
/// — entirely through defaults, so a regression of that default fuel budget
/// fails here instead of only surfacing for downstream consumers.
@Test func defaultHostLimitsRunTheFullHappyPathWithoutFuelExhaustion() async throws {
    guard let rootPath = ProcessInfo.processInfo.environment["TRAVERSE_NATIVE_ARTIFACT_ROOT"] else { return }
    let root = URL(fileURLWithPath: rootPath)
    let runtime = try Data(contentsOf: root.appendingPathComponent("runtime/runtime.wasm"))
    let bundle = try TraverseBundle(
        rootURL: root,
        runtimeWasmDigest: "sha256:" + SHA256.hash(data: runtime).map { String(format: "%02x", $0) }.joined())
    let fixture = try loadJSON("fixture.json")
    let headerBytes = try encode(try #require(fixture["init_header"]))
    var initPayload = Data()
    var length = UInt32(headerBytes.count).littleEndian
    withUnsafeBytes(of: &length) { initPayload.append(contentsOf: $0) }
    initPayload.append(headerBytes)
    let scenario = try #require(
        (fixture["scenarios"] as? [[String: Any]])?.first { $0["id"] as? String == "happy_path" })
    let steps = try #require(scenario["steps"] as? [[String: Any]])
    let machine = try #require((fixture["init_header"] as? [String: Any])?["state_machine"] as? [String: Any])
    let hostCommands = Set(((machine["states"] as? [[String: Any]]) ?? []).compactMap {
        ($0["invoke"] as? [String: Any])?["host_connector"] as? String
    })

    // No `limits:` argument: this is the same construction path a real embedder app uses.
    let client = try WasmiHostBridgeClient(bundle: bundle)
    let embedder = RuntimeTraverseEmbedder(client: client)
    _ = try embedder.initialize(configJSON: initPayload)

    let pending = Box<[HostConnectorResult]>(try steps.compactMap { step in
        guard let spec = step["complete"] as? [String: Any] else { return nil }
        return HostConnectorResult(
            resultClass: try #require(spec["result_class"] as? String),
            payloadJSON: try encode(spec["payload"] ?? [String: Any]()))
    })
    for command in hostCommands {
        _ = try embedder.registerHostConnectorAdapter(command: command) { _ in
            pending.value.isEmpty
                ? HostConnectorResult(resultClass: "failed")
                : pending.value.removeFirst()
        }
    }

    var sessionID: String?
    var received: [TraverseRuntimeEvent] = []
    let submitSteps = steps.filter { $0["submit"] != nil }
    for (index, step) in submitSteps.enumerated() {
        let spec = try #require(step["submit"] as? [String: Any])
        let command = try TraverseAppCommand(
            command: try #require(spec["command"] as? String),
            payloadJSON: try encode(spec["payload"] ?? [String: Any]()),
            sessionID: spec["session"] is String ? sessionID : nil)
        let result = try embedder.submit(command)
        #expect(result.status == "accepted", "submit #\(index) (\(command.command)): \(result)")
        sessionID = sessionID ?? result.sessionID
        // Each host-connector wait completes asynchronously (the registered adapter
        // runs on a background Task), and can itself trigger a further transition.
        // Wait for the whole chain to quiesce (no new events for a few consecutive
        // polls) before submitting the next command, or it races a transition still
        // in flight and is rejected from the wrong state — see #1562.
        var quietPolls = 0
        while quietPolls < 5 {
            let new = try embedder.subscribe()
            if new.isEmpty {
                quietPolls += 1
            } else {
                received += new
                quietPolls = 0
            }
            try await Task.sleep(nanoseconds: 10_000_000)
        }
    }
    _ = try embedder.shutdown()
    #expect(received.contains {
        $0.eventType == "state_changed"
            && String(decoding: $0.output ?? Data(), as: UTF8.self).contains("\"recorded\"")
    }, "expected a state_changed event reaching \"recorded\"; got \(received)")
}
