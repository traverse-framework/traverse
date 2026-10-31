// #1630: BirdNET on the governed Spec 138 path on a physical iPhone.
//
// Runs a signed BirdNET package (with its signed Core ML variant) through the
// public `TraverseEmbedder.ExactModelHost`:
// - one host per Core ML compute-unit setting, policy `[coreml, wasm-cpu]`;
// - one host with no accelerator, so every call runs on `wasm-cpu` with
//   guest ABI v3 snapshot reuse.
// Every clip is timed around `execute` (the governed call, including policy,
// rights and evidence). Accelerated outputs are compared with the device's
// own `wasm-cpu` outputs under Decision 106 (top-5 identical, |Δ sigmoid| ≤
// 5e-3). The report is printed as one JSON line and written to Documents.

import CryptoKit
import Foundation
import SwiftUI
import TraverseEmbedder
import UIKit

@main
struct EvidenceApp: App {
    var body: some Scene {
        WindowGroup { ContentView() }
    }
}

struct ContentView: View {
    @State private var status = "Running #1630 evidence… keep the phone unlocked."

    var body: some View {
        ScrollView {
            Text(status)
                .font(.system(.footnote, design: .monospaced))
                .padding()
        }
        .task {
            UIApplication.shared.isIdleTimerDisabled = true
            status = await Harness.run()
            UIApplication.shared.isIdleTimerDisabled = false
        }
    }
}

enum Harness {
    static let clipBytes = 576_016
    static let outputBytes = 26_104

    struct Clip {
        let name: String
        let frame: Data
    }

    static func resource(_ path: String) throws -> Data {
        guard let url = Bundle.main.resourceURL?.appendingPathComponent(path) else {
            throw NSError(domain: "harness", code: 1, userInfo: [NSLocalizedDescriptionKey: "missing \(path)"])
        }
        return try Data(contentsOf: url)
    }

    static func clips() throws -> [Clip] {
        guard let dir = Bundle.main.resourceURL?.appendingPathComponent("frames") else { return [] }
        return try FileManager.default.contentsOfDirectory(atPath: dir.path)
            .filter { $0.hasSuffix(".bin") }
            .sorted()
            .map { Clip(name: String($0.dropLast(4)), frame: try Data(contentsOf: dir.appendingPathComponent($0))) }
    }

    static func logits(_ frame: Data) -> [Float] {
        stride(from: 16, to: frame.count, by: 4).map { offset in
            frame.subdata(in: offset..<(offset + 4)).withUnsafeBytes {
                Float(bitPattern: UInt32(littleEndian: $0.loadUnaligned(as: UInt32.self)))
            }
        }
    }

    static func top5(_ values: [Float]) -> [Int] {
        Array(values.indices.sorted { values[$0] > values[$1] }.prefix(5))
    }

    static func sigmoid(_ value: Float) -> Double { 1 / (1 + exp(-Double(value))) }

    static func percentile(_ values: [Double], _ p: Double) -> Double {
        let sorted = values.sorted()
        guard !sorted.isEmpty else { return 0 }
        let rank = Int((p / 100 * Double(sorted.count)).rounded(.up)) - 1
        return sorted[max(0, min(sorted.count - 1, rank))]
    }

    static func summary(_ times: [Double]) -> [String: Any] {
        [
            "count": times.count,
            "mean_ms": times.reduce(0, +) / Double(max(times.count, 1)),
            "p50_ms": percentile(times, 50),
            "p95_ms": percentile(times, 95),
            "max_ms": times.max() ?? 0,
        ]
    }

    static func deviceModel() -> String {
        var info = utsname()
        uname(&info)
        return withUnsafeBytes(of: &info.machine) { raw in
            String(decoding: raw.prefix { $0 != 0 }, as: UTF8.self)
        }
    }

    struct Run {
        var label: String
        var registrationMs: Double = 0
        var registeredPlacement = ""
        var attempts: [[String: Any]] = []
        var firstMs: Double = 0
        var times: [Double] = []
        var outputs: [Data] = []
        var placements: Set<String> = []
        var evidence: [String: String] = [:]
        var error: String?
    }

    /// Register the package on a fresh host and run every clip once (plus a
    /// first, untimed-in-steady-state call). Registration compiles, runs the
    /// device self-check and `model_prepare`, and fixes the placement
    /// (Spec 138 0.16.0, Decision 120); its time is recorded separately (#1706).
    static func measure(label: String, coreML: CoreMLAcceleratorConfig?, placements: [String]?,
                        pin: ExactModelPin, key: String, files: [String: Data], clips: [Clip]) async -> Run {
        var run = Run(label: label)
        do {
            let limits = ExactModelHostLimits(maxPackageBytes: 512 * 1024 * 1024,
                                              maxMemoryBytes: 512 * 1024 * 1024,
                                              maxFuel: 50_000_000_000,
                                              maxSnapshotBytes: 512 * 1024 * 1024)
            let host = try ExactModelHost(pins: [pin], trustedPublicKeysHex: [key], modelUsage: "non_commercial",
                                          limits: limits, coreML: coreML)
            let variants = files.filter { $0.key.hasPrefix("variants/") }
            let registerStart = DispatchTime.now().uptimeNanoseconds
            let registered = try await host.registerPackageWithPreparation(
                manifest: files["model.manifest.json"]!, wasm: files["model.wasm"]!,
                signature: files["model.sig.json"]!, variantFiles: variants,
                policy: ExactModelPlacementPolicy(policyRef: "evidence", placements: placements ?? []))
            run.registrationMs = Double(DispatchTime.now().uptimeNanoseconds - registerStart) / 1_000_000
            run.registeredPlacement = registered.placement
            run.attempts = registered.attempts.map { attempt in
                var entry: [String: Any] = ["placement": attempt.placement, "outcome": attempt.outcome,
                                            "duration_ms": attempt.durationMs]
                if let check = attempt.selfCheck {
                    var detail: [String: Any] = ["passed": check.passed, "cases": check.cases,
                                                 "failed_cases": check.failedCases,
                                                 "top_k_mismatches": check.topKMismatches,
                                                 "other_failures": check.otherFailures]
                    if let worst = check.worstAbsDiff { detail["worst_abs_diff"] = worst }
                    if let index = check.worstCase { detail["worst_case"] = index }
                    entry["self_check"] = detail
                }
                return entry
            }
            func call(_ clip: Clip) async throws -> (Double, Data, ExactModelExecution) {
                let inputRef = try host.stageModelInput(clip.frame, maxBytes: clipBytes)
                let start = DispatchTime.now().uptimeNanoseconds
                let result = try await host.execute(
                    modelRef: (pin.modelId, pin.version, pin.digest), inputRef: inputRef, policyRef: "evidence",
                    dataClassification: "sensitive", inputSchemaRef: "schema:birdnet-audio-48k-3s-in",
                    inputSchemaVersion: "1.0.0", maxOutputBytes: outputBytes, allowedClassifications: ["sensitive"],
                    timeoutMs: 60_000)
                let ms = Double(DispatchTime.now().uptimeNanoseconds - start) / 1_000_000
                let output = try host.readModelOutput(result.outputRef, maxBytes: outputBytes)
                return (ms, output, result)
            }
            let (first, _, _) = try await call(clips[0])
            run.firstMs = first
            for clip in clips {
                let (ms, output, result) = try await call(clip)
                run.times.append(ms)
                run.outputs.append(output)
                run.placements.insert(result.placement)
                if let evidence = result.placementEvidence {
                    run.evidence = ["placement": evidence.placement, "compute_units": evidence.computeUnits,
                                    "fuel_metering": evidence.fuelMetering]
                }
            }
        } catch {
            run.error = "\(error)"
        }
        return run
    }

    static func run() async -> String {
        do {
            let files: [String: Data] = try Dictionary(uniqueKeysWithValues: [
                "model.manifest.json", "model.wasm", "model.sig.json",
                "variants/coreml/model.mlpackage.zip", "variants/coreml/conformance.json",
            ].map { ($0, try resource("birdnet_pkg/\($0)")) })
            let key = String(decoding: try resource("trusted_key.hex"), as: UTF8.self)
                .trimmingCharacters(in: .whitespacesAndNewlines)
            let manifestData = files["model.manifest.json"]!
            let manifest = try JSONSerialization.jsonObject(with: manifestData) as? [String: Any] ?? [:]
            let rights = manifest["rights"] as? [String: Any] ?? [:]
            let digest = SHA256.hash(data: manifestData).map { String(format: "%02x", $0) }.joined()
            let pin = ExactModelPin(modelId: manifest["model_id"] as? String ?? "", version: manifest["version"] as? String ?? "",
                                    digest: digest,
                                    rights: .init(licenseId: rights["license_id"] as? String ?? "",
                                                  commercialUse: rights["commercial_use"] as? String ?? ""))
            let clips = try clips()
            let cache = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask)[0]
                .appendingPathComponent("traverse-coreml")

            // wasm-cpu first: it is the reference every accelerated output is compared with.
            let wasm = await measure(label: "wasm-cpu (snapshot reuse)", coreML: nil, placements: nil,
                                     pin: pin, key: key, files: files, clips: clips)
            var runs = [wasm]
            for units in ["all", "cpu_and_gpu", "cpu_and_neural_engine", "cpu_only"] {
                runs.append(await measure(label: "coreml \(units)",
                                          coreML: CoreMLAcceleratorConfig(computeUnits: units, cacheDirectory: cache),
                                          placements: ["coreml", "wasm-cpu"], pin: pin, key: key, files: files, clips: clips))
            }

            var report: [String: Any] = [
                "issue": "#1630, #1706, #1708",
                "device": deviceModel(),
                "os": ProcessInfo.processInfo.operatingSystemVersionString,
                "build": "Release",
                "package": ["model_id": pin.modelId, "version": pin.version, "digest": digest],
                "clips": clips.map(\.name),
                "bar_ms": 1500,
            ]
            var results: [[String: Any]] = []
            for run in runs {
                var entry: [String: Any] = ["label": run.label]
                if let error = run.error {
                    entry["error"] = error
                    results.append(entry)
                    continue
                }
                entry["registration_ms"] = run.registrationMs
                entry["registered_placement"] = run.registeredPlacement
                entry["placement_attempts"] = run.attempts
                entry["first_call_ms"] = run.firstMs
                entry["steady"] = summary(run.times)
                entry["per_clip_ms"] = run.times
                entry["placements"] = Array(run.placements).sorted()
                entry["placement_evidence"] = run.evidence
                entry["meets_bar"] = percentile(run.times, 95) <= 1500
                if run.label != wasm.label, wasm.error == nil, run.outputs.count == wasm.outputs.count {
                    var worst = 0.0
                    var top5Matches = 0
                    for (actual, reference) in zip(run.outputs, wasm.outputs) {
                        let a = logits(actual), r = logits(reference)
                        worst = max(worst, zip(a, r).map { abs(sigmoid($0) - sigmoid($1)) }.max() ?? 0)
                        if top5(a) == top5(r) { top5Matches += 1 }
                    }
                    entry["tolerance_vs_wasm_cpu"] = ["worst_abs_delta_sigmoid": worst, "top5_identical": top5Matches,
                                                      "clips": run.outputs.count,
                                                      "pass": worst <= 5e-3 && top5Matches == run.outputs.count]
                }
                results.append(entry)
            }
            report["runs"] = results
            let json = try JSONSerialization.data(withJSONObject: report, options: [.sortedKeys])
            let text = String(decoding: json, as: UTF8.self)
            print("TRAVERSE_1630_EVIDENCE " + text)
            let out = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
                .appendingPathComponent("evidence-1630.json")
            try json.write(to: out)
            let pretty = try JSONSerialization.data(withJSONObject: report, options: [.prettyPrinted, .sortedKeys])
            return "DONE\n" + String(decoding: pretty, as: UTF8.self)
        } catch {
            print("TRAVERSE_1630_ERROR \(error)")
            return "ERROR: \(error)"
        }
    }
}
