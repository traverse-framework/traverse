import Foundation
import TraverseEmbedder

// Manual run for the Apple `traverse.audio-input` adapter (Spec 140, #1499).
//
//   swift run TraverseAudioSmoke                      # live: real permission prompt + microphone
//   swift run TraverseAudioSmoke --cancel-after-ms 500  # also prove cancellation mid-capture
//   swift run TraverseAudioSmoke --dry-run            # synthetic audio, no prompt, no microphone
//
// Prints one paste-ready JSON report. It contains only the permission state, opaque refs, byte
// counts, and WAV/level facts. It never prints a path, device name, or native error text.

struct Options {
    var dryRun = false
    var durationMs = 2000
    var maxBytes = 1_048_576
    var cancelAfterMs: Int?
}

func parseOptions() -> Options? {
    var options = Options()
    var arguments = Array(CommandLine.arguments.dropFirst())
    while !arguments.isEmpty {
        let flag = arguments.removeFirst()
        switch flag {
        case "--dry-run": options.dryRun = true
        case "--duration-ms", "--max-bytes", "--cancel-after-ms":
            guard let text = arguments.first, let value = Int(text), value > 0 else { return nil }
            arguments.removeFirst()
            if flag == "--duration-ms" { options.durationMs = value }
            if flag == "--max-bytes" { options.maxBytes = value }
            if flag == "--cancel-after-ms" { options.cancelAfterMs = value }
        default: return nil
        }
    }
    return options
}

guard let options = parseOptions() else {
    FileHandle.standardError.write(Data("usage: TraverseAudioSmoke [--dry-run] [--duration-ms N] [--max-bytes N] [--cancel-after-ms N]\n".utf8))
    exit(2)
}

// MARK: - Synthetic drivers (dry run only)

struct SyntheticPermissions: AudioPermissionDriver {
    func status() async -> AudioPermissionState { .granted }
    func request() async -> AudioPermissionState { .granted }
}

struct SyntheticCapture: AudioCaptureDriver {
    func capture(_ request: AudioCaptureRequest) async throws -> Data {
        let sampleRate = 16_000
        let frames = sampleRate * min(request.maxDurationMs, 1000) / 1000
        var samples = Data()
        for index in 0..<frames {
            let value = Int16(sin(2 * Double.pi * 440 * Double(index) / Double(sampleRate)) * 12_000)
            withUnsafeBytes(of: value.littleEndian) { samples.append(contentsOf: $0) }
        }
        var wav = Data()
        func put32(_ value: Int) { withUnsafeBytes(of: UInt32(value).littleEndian) { wav.append(contentsOf: $0) } }
        func put16(_ value: Int) { withUnsafeBytes(of: UInt16(value).littleEndian) { wav.append(contentsOf: $0) } }
        wav.append(contentsOf: Array("RIFF".utf8)); put32(36 + samples.count)
        wav.append(contentsOf: Array("WAVEfmt ".utf8)); put32(16); put16(1); put16(1)
        put32(sampleRate); put32(sampleRate * 2); put16(2); put16(16)
        wav.append(contentsOf: Array("data".utf8)); put32(samples.count)
        wav.append(samples)
        // Long requests stay in flight until cancelled, so the cancel path is exercisable.
        if request.maxDurationMs > 5000 {
            while !Task.isCancelled { try await Task.sleep(nanoseconds: 5_000_000) }
            throw CancellationError()
        }
        return wav
    }
}

// MARK: - WAV inspection

func inspectWav(_ data: Data) -> [String: Any] {
    func u16(_ offset: Int) -> Int { Int(data[offset]) | Int(data[offset + 1]) << 8 }
    func u32(_ offset: Int) -> Int { u16(offset) | u16(offset + 2) << 16 }
    var facts: [String: Any] = ["valid": false]
    guard data.count >= 44,
          String(decoding: data[0..<4], as: UTF8.self) == "RIFF",
          String(decoding: data[8..<16], as: UTF8.self) == "WAVEfmt ",
          String(decoding: data[36..<40], as: UTF8.self) == "data"
    else { return facts }
    let sampleRate = u32(24), channels = u16(22), bits = u16(34)
    let dataBytes = u32(40)
    guard u16(20) == 1, channels == 1, bits == 16, sampleRate > 0, 44 + dataBytes == data.count else { return facts }
    var peak = 0
    var sumSquares = 0.0
    let count = dataBytes / 2
    for index in 0..<count {
        let sample = Int(Int16(bitPattern: UInt16(u16(44 + index * 2))))
        peak = max(peak, abs(sample))
        sumSquares += Double(sample) * Double(sample)
    }
    facts["valid"] = true
    facts["sample_rate"] = sampleRate
    facts["channels"] = channels
    facts["bits_per_sample"] = bits
    facts["duration_ms"] = count * 1000 / sampleRate
    // Levels in permille of full scale (1000 = full scale), to keep the report free of float noise.
    facts["peak_permille"] = peak * 1000 / 32_767
    facts["rms_permille"] = count == 0 ? 0 : Int((sumSquares / Double(count)).squareRoot() * 1000 / 32_767)
    return facts
}

// MARK: - Run

func summary(_ result: HostConnectorResult) -> [String: Any] {
    var out: [String: Any] = ["result_class": result.resultClass]
    if let body = (try? JSONSerialization.jsonObject(with: result.payloadJSON)) as? [String: Any] {
        for key in ["permission_state", "error_code", "artifact_ref"] {
            if let value = body[key] { out[key] = value }
        }
    }
    return out
}

func request(_ payload: [String: Any], id: String) throws -> HostConnectorRequest {
    HostConnectorRequest(
        command: "smoke", commandID: id, sessionID: "smoke-session",
        payloadJSON: try JSONSerialization.data(withJSONObject: payload))
}

func run() async throws -> [String: Any] {
    let store = ArtifactStagingStore()
    let permissions: any AudioPermissionDriver
    let capture: any AudioCaptureDriver
    if options.dryRun {
        permissions = SyntheticPermissions()
        capture = SyntheticCapture()
    } else {
        #if canImport(AVFoundation)
        permissions = AVFoundationAudioPermissionDriver()
        capture = AVFoundationAudioCaptureDriver()
        #else
        FileHandle.standardError.write(Data("live mode needs AVFoundation; use --dry-run\n".utf8))
        exit(2)
        #endif
    }
    let adapters = AppleAudioInputAdapters(
        stage: { try await store.stageArtifact($0, maxBytes: $1) },
        permissions: permissions, capture: capture)

    var report: [String: Any] = [
        "tool": "TraverseAudioSmoke",
        "mode": options.dryRun ? "dry-run" : "live",
        "os": ProcessInfo.processInfo.operatingSystemVersionString,
    ]
    var checks: [String: Bool] = [:]

    // 1. Permission: status (no prompt), then the real request.
    let before = await permissions.status()
    let permission = try await adapters.requestPermission(request([:], id: "smoke-permission"))
    report["permission"] = ["status_before": before.rawValue, "request": summary(permission)]
    let granted = (summary(permission)["permission_state"] as? String) == "granted"
    checks["permission_reports_typed_state"] =
        permission.resultClass == "succeeded" || summary(permission)["error_code"] is String
    guard granted else {
        report["note"] = "permission not granted; capture skipped"
        report["checks"] = checks
        report["passed"] = false
        return report
    }

    // 2. Capture, then read the staged bytes back through the runtime-mediated path.
    let captured = try await adapters.captureAudio(request(
        ["max_duration_ms": options.durationMs, "max_bytes": options.maxBytes], id: "smoke-capture"))
    var captureReport = summary(captured)
    if let ref = captureReport["artifact_ref"] as? String {
        let bytes = try await store.readArtifact(ref, maxBytes: options.maxBytes)
        let again = try await store.readArtifact(ref, maxBytes: options.maxBytes)
        let wav = inspectWav(bytes)
        captureReport["bytes"] = bytes.count
        captureReport["wav"] = wav
        checks["capture_succeeded"] = captured.resultClass == "succeeded"
        checks["artifact_ref_is_opaque"] = !ref.contains("/") && !ref.contains(":") && !ref.contains("\\")
        checks["artifact_is_multi_read"] = bytes == again
        checks["artifact_is_valid_wav"] = wav["valid"] as? Bool == true
        checks["artifact_within_max_bytes"] = bytes.count <= options.maxBytes
        checks["audio_is_not_silent"] = (wav["peak_permille"] as? Int ?? 0) > 0
    } else {
        checks["capture_succeeded"] = false
    }
    report["capture"] = captureReport

    // 3. Optional: cancel a long capture mid-flight.
    if let delay = options.cancelAfterMs {
        let running = Task {
            try await adapters.captureAudio(request(
                ["max_duration_ms": 30_000, "max_bytes": options.maxBytes, "correlation_id": "smoke-cancel"],
                id: "smoke-cancel-command"))
        }
        try await Task.sleep(nanoseconds: UInt64(delay) * 1_000_000)
        adapters.cancel(correlationID: "smoke-cancel")
        let cancelled = try await running.value
        report["cancel"] = summary(cancelled)
        checks["cancel_returns_cancelled"] =
            cancelled.resultClass == "cancelled" && summary(cancelled)["error_code"] as? String == "cancelled"
    }

    report["checks"] = checks
    report["passed"] = !checks.values.contains(false)
    return report
}

do {
    let report = try await run()
    let json = try JSONSerialization.data(withJSONObject: report, options: [.prettyPrinted, .sortedKeys])
    print(String(decoding: json, as: UTF8.self))
    exit(report["passed"] as? Bool == true ? 0 : 1)
} catch {
    // Native error text is intentionally not printed.
    print(#"{"passed":false,"error":"smoke run failed"}"#)
    exit(1)
}
