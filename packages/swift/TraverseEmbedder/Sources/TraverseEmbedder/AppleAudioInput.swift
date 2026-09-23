import Foundation

/// Spec 140 Apple host adapter for `traverse.audio-input`.
///
/// Implements WIT `traverse:audio-input@1.0.0` semantics behind the Spec 137 command
/// envelope. Permission prompting stays inside the drivers; public results expose only an
/// opaque `artifact_ref` or a non-secret `permission_state`. Captured bytes are staged through
/// the generic Spec 138/140 staging API and never surface as a path, URL, or device name.

/// Non-secret WIT `permission-state`.
public enum AudioPermissionState: String, Sendable, Equatable {
    case granted
    case denied
    case promptRequired = "prompt_required"
    case unavailable
}

/// Typed capture failure. Drivers map native errors to one of these; the native error text
/// never crosses the adapter boundary.
public enum AudioCaptureFailure: Error, Sendable, Equatable {
    case cancelled
    case unavailable
    case limitExceeded
    case policyDenied
}

public struct AudioCaptureRequest: Sendable, Equatable {
    public let correlationID: String
    public let maxDurationMs: Int
    public let maxBytes: Int

    public init(correlationID: String, maxDurationMs: Int, maxBytes: Int) {
        self.correlationID = correlationID
        self.maxDurationMs = maxDurationMs
        self.maxBytes = maxBytes
    }
}

/// WIT `permission-status` / `request-permission`.
public protocol AudioPermissionDriver: Sendable {
    /// Reads the state without prompting.
    func status() async -> AudioPermissionState
    /// Resolves the state, prompting the OS when it is undetermined.
    func request() async -> AudioPermissionState
}

/// WIT `capture`. Must honor task cancellation and throw ``AudioCaptureFailure``.
public protocol AudioCaptureDriver: Sendable {
    func capture(_ request: AudioCaptureRequest) async throws -> Data
}

/// Stages captured bytes and returns an opaque `artifact_ref`
/// (for example `ArtifactStagingStore.stageArtifact`).
public typealias AudioArtifactStager = @Sendable (Data, Int) async throws -> String

/// Spec 139 host-connector adapters for permission and capture, plus WIT `cancel`.
public final class AppleAudioInputAdapters: @unchecked Sendable {
    private let permissions: any AudioPermissionDriver
    private let capture: any AudioCaptureDriver
    private let stage: AudioArtifactStager
    private let lock = NSLock()
    private var inFlight: [String: Task<Data, Error>] = [:]

    public init(
        stage: @escaping AudioArtifactStager,
        permissions: any AudioPermissionDriver,
        capture: any AudioCaptureDriver
    ) {
        self.stage = stage
        self.permissions = permissions
        self.capture = capture
    }

    /// Adapter for the manifest command routed to `audio.permission.request`.
    public var requestPermission: HostConnectorAdapter {
        { [self] _ in
            switch await permissions.request() {
            case .denied: Self.failed("policy_denied", "audio permission was denied")
            case .unavailable: Self.failed("unavailable", "audio permission is unavailable")
            case let state: Self.succeeded(["permission_state": state.rawValue])
            }
        }
    }

    /// Adapter for the manifest command routed to `audio.capture`.
    public var captureAudio: HostConnectorAdapter {
        { [self] request in await runCapture(request) }
    }

    /// WIT `cancel(correlation-id)`: cancels the in-flight capture, if any.
    public func cancel(correlationID: String) {
        lock.withLock { inFlight[correlationID] }?.cancel()
    }

    private func track(_ correlationID: String, _ task: Task<Data, Error>) {
        lock.withLock { inFlight[correlationID] = task }
    }

    private func untrack(_ correlationID: String) {
        lock.withLock { inFlight[correlationID] = nil }
    }

    private func runCapture(_ request: HostConnectorRequest) async -> HostConnectorResult {
        let payload = (try? JSONSerialization.jsonObject(with: request.payloadJSON)) as? [String: Any] ?? [:]
        guard let maxDurationMs = Self.positiveInt(payload["max_duration_ms"]),
              let maxBytes = Self.positiveInt(payload["max_bytes"])
        else {
            return Self.failed("input_limit_exceeded", "audio.capture requires positive limits")
        }
        let correlationID = (payload["correlation_id"] as? String).flatMap { $0.isEmpty ? nil : $0 } ?? request.commandID
        let driver = capture
        let task = Task {
            try await driver.capture(AudioCaptureRequest(
                correlationID: correlationID, maxDurationMs: maxDurationMs, maxBytes: maxBytes))
        }
        track(correlationID, task)
        defer { untrack(correlationID) }
        do {
            let bytes = try await withTaskCancellationHandler { try await task.value } onCancel: { task.cancel() }
            if task.isCancelled { return Self.failed("cancelled", "capture cancelled") }
            return await stageCaptured(bytes, maxBytes: maxBytes)
        } catch AudioCaptureFailure.cancelled, is CancellationError {
            return Self.failed("cancelled", "capture cancelled")
        } catch AudioCaptureFailure.limitExceeded {
            return Self.failed("input_limit_exceeded", "capture exceeded published ceilings")
        } catch AudioCaptureFailure.policyDenied {
            return Self.failed("policy_denied", "audio permission was denied")
        } catch {
            // Native detail (paths, device names, OS errors) is dropped by design.
            return Self.failed("unavailable", "audio capture unavailable")
        }
    }

    private func stageCaptured(_ bytes: Data, maxBytes: Int) async -> HostConnectorResult {
        guard !bytes.isEmpty, bytes.count <= maxBytes else {
            return Self.failed("input_limit_exceeded", "captured audio empty or exceeds max_bytes")
        }
        let reference: String
        do {
            reference = try await stage(bytes, maxBytes)
        } catch {
            return Self.failed("input_limit_exceeded", "staging rejected captured audio")
        }
        guard !reference.contains("/"), !reference.contains(":"), !reference.contains("\\") else {
            return Self.failed("unavailable", "host adapter returned a non-opaque artifact reference")
        }
        return Self.succeeded(["artifact_ref": reference])
    }

    private static func positiveInt(_ value: Any?) -> Int? {
        guard let number = value as? NSNumber else { return nil }
        let double = number.doubleValue
        guard double.isFinite, double >= 1, double <= Double(Int.max / 2) else { return nil }
        return Int(double)
    }

    private static func succeeded(_ payload: [String: String]) -> HostConnectorResult {
        HostConnectorResult(resultClass: "succeeded", payloadJSON: json(payload))
    }

    private static func failed(_ code: String, _ message: String) -> HostConnectorResult {
        HostConnectorResult(
            resultClass: code == "cancelled" ? "cancelled" : "failed",
            payloadJSON: json(["error_code": code, "message": message]))
    }

    private static func json(_ value: [String: String]) -> Data {
        (try? JSONSerialization.data(withJSONObject: value, options: [.sortedKeys])) ?? Data("{}".utf8)
    }
}

#if canImport(AVFoundation)
import AVFoundation

/// OS microphone permission through `AVCaptureDevice` (macOS and iOS).
public struct AVFoundationAudioPermissionDriver: AudioPermissionDriver {
    public init() {}

    public func status() async -> AudioPermissionState {
        switch AVCaptureDevice.authorizationStatus(for: .audio) {
        case .authorized: .granted
        case .denied, .restricted: .denied
        case .notDetermined: .promptRequired
        @unknown default: .unavailable
        }
    }

    public func request() async -> AudioPermissionState {
        let current = await status()
        guard current == .promptRequired else { return current }
        return await AVCaptureDevice.requestAccess(for: .audio) ? .granted : .denied
    }
}

/// Microphone capture through `AVAudioEngine`, staged as mono 16-bit PCM WAV.
///
/// Requires the host app to declare `NSMicrophoneUsageDescription`. It never prompts: it fails
/// `policyDenied` unless permission is already granted, so the permission step stays explicit.
public struct AVFoundationAudioCaptureDriver: AudioCaptureDriver {
    public init() {}

    public func capture(_ request: AudioCaptureRequest) async throws -> Data {
        guard AVCaptureDevice.authorizationStatus(for: .audio) == .authorized else {
            throw AudioCaptureFailure.policyDenied
        }
        let session = CaptureSession(maxBytes: request.maxBytes)
        return try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { continuation in
                session.start(maxDurationMs: request.maxDurationMs, continuation: continuation)
            }
        } onCancel: {
            session.finish(.failure(AudioCaptureFailure.cancelled))
        }
    }
}

/// One-shot capture: the first of duration elapsed, byte ceiling hit, cancel, or failure wins.
private final class CaptureSession: @unchecked Sendable {
    private static let headerBytes = 44
    private let maxBytes: Int
    private let lock = NSLock()
    private var continuation: CheckedContinuation<Data, Error>?
    private var pendingResult: Result<Data, Error>?
    private var engine: AVAudioEngine?
    private var samples = Data()
    private var sampleRate = 0

    init(maxBytes: Int) {
        self.maxBytes = maxBytes
    }

    func start(maxDurationMs: Int, continuation: CheckedContinuation<Data, Error>) {
        lock.lock()
        if let pending = pendingResult {
            lock.unlock()
            continuation.resume(with: pending)
            return
        }
        self.continuation = continuation
        lock.unlock()

        #if os(iOS)
        do {
            let audioSession = AVAudioSession.sharedInstance()
            try audioSession.setCategory(.record, mode: .measurement)
            try audioSession.setActive(true)
        } catch {
            finish(.failure(AudioCaptureFailure.unavailable))
            return
        }
        #endif
        let engine = AVAudioEngine()
        engine.prepare()
        let format = engine.inputNode.outputFormat(forBus: 0)
        guard format.sampleRate > 0, format.channelCount > 0 else {
            finish(.failure(AudioCaptureFailure.unavailable))
            return
        }
        sampleRate = Int(format.sampleRate)
        engine.inputNode.installTap(onBus: 0, bufferSize: 4096, format: format) { [weak self] buffer, _ in
            self?.append(buffer)
        }
        do {
            try engine.start()
        } catch {
            engine.inputNode.removeTap(onBus: 0)
            finish(.failure(AudioCaptureFailure.unavailable))
            return
        }
        lock.lock()
        self.engine = engine
        let alreadyFinished = pendingResult != nil
        lock.unlock()
        if alreadyFinished { teardown(engine) }
        DispatchQueue.global().asyncAfter(deadline: .now() + .milliseconds(maxDurationMs)) { [weak self] in
            self?.finish(.success(nil))
        }
    }

    /// `nil` success means "duration elapsed": deliver what was captured as WAV.
    func finish(_ outcome: Result<Data?, Error>) {
        lock.lock()
        guard pendingResult == nil else {
            lock.unlock()
            return
        }
        let result: Result<Data, Error>
        switch outcome {
        case .failure(let error): result = .failure(error)
        case .success(let data): result = .success(data ?? wav())
        }
        pendingResult = result
        let waiting = continuation
        continuation = nil
        let running = engine
        engine = nil
        lock.unlock()
        if let running { teardown(running) }
        waiting?.resume(with: result)
    }

    private func teardown(_ engine: AVAudioEngine) {
        engine.inputNode.removeTap(onBus: 0)
        engine.stop()
    }

    private func append(_ buffer: AVAudioPCMBuffer) {
        guard let channel = buffer.floatChannelData?[0] else { return }
        var chunk = Data(capacity: Int(buffer.frameLength) * 2)
        for index in 0..<Int(buffer.frameLength) {
            let value = Int16(max(-1, min(1, channel[index])) * Float(Int16.max))
            withUnsafeBytes(of: value.littleEndian) { chunk.append(contentsOf: $0) }
        }
        lock.lock()
        let overflow = Self.headerBytes + samples.count + chunk.count > maxBytes
        if !overflow { samples.append(chunk) }
        lock.unlock()
        if overflow { finish(.failure(AudioCaptureFailure.limitExceeded)) }
    }

    private func wav() -> Data {
        var data = Data()
        func put32(_ value: Int) { withUnsafeBytes(of: UInt32(value).littleEndian) { data.append(contentsOf: $0) } }
        func put16(_ value: Int) { withUnsafeBytes(of: UInt16(value).littleEndian) { data.append(contentsOf: $0) } }
        data.append(contentsOf: Array("RIFF".utf8)); put32(36 + samples.count)
        data.append(contentsOf: Array("WAVEfmt ".utf8)); put32(16); put16(1); put16(1)
        put32(sampleRate); put32(sampleRate * 2); put16(2); put16(16)
        data.append(contentsOf: Array("data".utf8)); put32(samples.count)
        data.append(samples)
        return data
    }
}
#endif
