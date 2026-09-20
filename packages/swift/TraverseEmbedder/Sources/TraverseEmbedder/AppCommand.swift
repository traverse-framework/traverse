import Foundation

/// Cancels a scheduled deadline.
public protocol TraverseTimerHandle: Sendable {
    func cancel()
}

/// Host-provided monotonic timer port used for Spec 139 dual deadlines.
public protocol TraverseTimer: Sendable {
    /// Runs `callback` once after `delay` seconds unless the handle is cancelled first.
    func schedule(after delay: TimeInterval, _ callback: @escaping @Sendable () -> Void) -> any TraverseTimerHandle
}

/// Default timer port backed by a monotonic `DispatchQueue` deadline.
public struct SystemTraverseTimer: TraverseTimer {
    public init() {}

    public func schedule(after delay: TimeInterval, _ callback: @escaping @Sendable () -> Void) -> any TraverseTimerHandle {
        let item = DispatchWorkItem(block: callback)
        DispatchQueue.global().asyncAfter(deadline: .now() + max(0, delay), execute: item)
        return WorkItemHandle(item: item)
    }

    private struct WorkItemHandle: TraverseTimerHandle, @unchecked Sendable {
        let item: DispatchWorkItem
        func cancel() { item.cancel() }
    }
}

/// A runtime-staged host-connector wait handed to a registered adapter.
public struct HostConnectorRequest: Sendable, Equatable {
    public let command: String
    public let commandID: String
    public let sessionID: String
    public let payloadJSON: Data

    public init(command: String, commandID: String, sessionID: String, payloadJSON: Data) {
        self.command = command
        self.commandID = commandID
        self.sessionID = sessionID
        self.payloadJSON = payloadJSON
    }
}

/// Adapter outcome. `resultClass` is `succeeded`, `failed`, `cancelled`, or `timeout`.
public struct HostConnectorResult: Sendable, Equatable {
    public let resultClass: String
    public let payloadJSON: Data

    public init(resultClass: String, payloadJSON: Data = Data("{}".utf8)) {
        self.resultClass = resultClass
        self.payloadJSON = payloadJSON
    }
}

/// Host-side authority for one manifest command (Spec 140 WIT semantics). It never runs
/// inside `runtime.wasm`; the runtime only receives the correlated terminal. Adapters should
/// honor task cancellation, which `shutdown()` requests.
public typealias HostConnectorAdapter = @Sendable (HostConnectorRequest) async throws -> HostConnectorResult

/// Removes a registered adapter, but only while it is still the registered one.
public final class HostConnectorRegistration: @unchecked Sendable {
    private let onRemove: @Sendable () -> Void

    init(onRemove: @escaping @Sendable () -> Void) {
        self.onRemove = onRemove
    }

    public func remove() { onRemove() }
}

/// Drives Spec 139 app commands. State-machine logic stays in `runtime.wasm`; this type only
/// submits envelopes, runs registered adapters for staged host-connector waits, and registers
/// the host half of the dual deadline. The first correlated terminal wins in the runtime.
final class AppCommandCoordinator: @unchecked Sendable {
    private let submitBytes: @Sendable (Data) throws -> Data
    private let timer: any TraverseTimer
    private let lock = NSLock()
    private var adapters: [String: (id: UInt64, run: HostConnectorAdapter)] = [:]
    private var nextAdapterID: UInt64 = 0
    private var deadlines: [any TraverseTimerHandle] = []
    private var tasks: [Task<Void, Never>] = []
    private var stopped = false

    init(submit: @escaping @Sendable (Data) throws -> Data, timer: any TraverseTimer) {
        self.submitBytes = submit
        self.timer = timer
    }

    func submit(_ command: TraverseAppCommand) throws -> TraverseSubmissionResult {
        var envelope: [String: Any] = [
            "kind": "app_command",
            "command": command.command,
            "payload": try JSONSerialization.jsonObject(with: command.payloadJSON, options: .fragmentsAllowed),
        ]
        if let sessionID = command.sessionID {
            envelope["session_id"] = sessionID
        }
        return try dispatch(envelope)
    }

    func register(command: String, adapter: @escaping HostConnectorAdapter) throws -> HostConnectorRegistration {
        guard !command.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw TraverseEmbedderError.unsupportedOperation("host connector command must be non-empty")
        }
        lock.lock()
        nextAdapterID += 1
        let id = nextAdapterID
        adapters[command] = (id, adapter)
        lock.unlock()
        return HostConnectorRegistration { [weak self] in
            guard let self else { return }
            self.lock.lock()
            defer { self.lock.unlock() }
            if self.adapters[command]?.id == id {
                self.adapters[command] = nil
            }
        }
    }

    /// Cancels adapters and deadlines; late completions are dropped.
    func stop() {
        lock.lock()
        stopped = true
        let pendingDeadlines = deadlines
        let pendingTasks = tasks
        deadlines = []
        tasks = []
        lock.unlock()
        pendingDeadlines.forEach { $0.cancel() }
        pendingTasks.forEach { $0.cancel() }
    }

    private func dispatch(_ envelope: [String: Any]) throws -> TraverseSubmissionResult {
        let request = try JSONSerialization.data(withJSONObject: envelope, options: [.sortedKeys])
        let response = try Self.object(try submitBytes(request))
        let sessionID = try Self.requiredString("session_id", in: response)
        let status = try Self.requiredString("status", in: response)
        guard status == "accepted" else {
            return TraverseSubmissionResult(sessionID: sessionID, status: status, error: response["error"] as? String)
        }
        scheduleDeadlines(response)
        startHostConnectors(response, fallbackSessionID: sessionID)
        return TraverseSubmissionResult(sessionID: sessionID, status: status)
    }

    private func scheduleDeadlines(_ response: [String: Any]) {
        for deadline in Self.items(response, "pending_deadlines") {
            guard let commandID = deadline["command_id"] as? String,
                  let sessionID = deadline["session_id"] as? String,
                  let delayMs = (deadline["deadline_ms"] as? NSNumber)?.doubleValue,
                  delayMs.isFinite
            else { continue }
            lock.lock()
            if stopped {
                lock.unlock()
                return
            }
            deadlines.append(timer.schedule(after: max(0, delayMs) / 1000) { [weak self] in
                self?.terminal(["kind": "deadline_fired", "command_id": commandID, "session_id": sessionID])
            })
            lock.unlock()
        }
    }

    private func startHostConnectors(_ response: [String: Any], fallbackSessionID: String) {
        for pending in Self.items(response, "pending_host_connector") {
            guard let commandID = pending["command_id"] as? String else { continue }
            let sessionID = pending["session_id"] as? String ?? fallbackSessionID
            let command = pending["command"] as? String
            lock.lock()
            let adapter = command.flatMap { adapters[$0]?.run }
            lock.unlock()
            guard let command, let adapter else {
                complete(commandID, sessionID, HostConnectorResult(
                    resultClass: "failed", payloadJSON: Data("{\"code\":\"target_incompatible\"}".utf8)))
                continue
            }
            let payload = pending["payload"] ?? [String: Any]()
            let payloadJSON = (try? JSONSerialization.data(withJSONObject: payload, options: [.fragmentsAllowed, .sortedKeys]))
                ?? Data("{}".utf8)
            let request = HostConnectorRequest(
                command: command, commandID: commandID, sessionID: sessionID, payloadJSON: payloadJSON)
            let task = Task { [weak self] in
                let result: HostConnectorResult
                do {
                    result = try await adapter(request)
                } catch {
                    // Adapters are host-supplied; report a fixed code and drop native detail.
                    result = HostConnectorResult(
                        resultClass: "failed", payloadJSON: Data("{\"code\":\"execution_failed\"}".utf8))
                }
                self?.complete(commandID, sessionID, result)
            }
            lock.lock()
            if stopped { task.cancel() } else { tasks.append(task) }
            lock.unlock()
        }
    }

    private func complete(_ commandID: String, _ sessionID: String, _ result: HostConnectorResult) {
        let payload = (try? JSONSerialization.jsonObject(with: result.payloadJSON, options: .fragmentsAllowed))
            ?? [String: Any]()
        terminal([
            "kind": "host_connector_result",
            "command_id": commandID,
            "session_id": sessionID,
            "result_class": result.resultClass,
            "payload": payload,
        ])
    }

    private func terminal(_ envelope: [String: Any]) {
        lock.lock()
        let isStopped = stopped
        lock.unlock()
        guard !isStopped else { return }
        // Shutdown and first-terminal-wins are runtime-owned; a terminal the runtime can no
        // longer accept has no host retry path.
        _ = try? dispatch(envelope)
    }

    private static func items(_ response: [String: Any], _ name: String) -> [[String: Any]] {
        (response[name] as? [Any])?.compactMap { $0 as? [String: Any] } ?? []
    }

    private static func object(_ data: Data) throws -> [String: Any] {
        guard let value = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] else {
            throw TraverseBridgeError(status: -2, message: "bridge_invalid_json")
        }
        return value
    }

    private static func requiredString(_ name: String, in value: [String: Any]) throws -> String {
        guard let result = value[name] as? String else {
            throw TraverseBridgeError(status: -2, message: "bridge result is missing \(name)")
        }
        return result
    }
}
