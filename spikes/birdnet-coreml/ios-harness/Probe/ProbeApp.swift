import CoreML
import Foundation
import SwiftUI

/// Decision 112 spike (#1663): BirdNET v2.4 as a Core ML ML Program on this
/// device. Compiles the canonical .mlpackage on device, times every clip per
/// compute-unit setting, checks Decision 106's tolerance against the tract int8
/// reference, prints one JSON line, and exits.
@main
struct ProbeApp: App {
    init() {
        DispatchQueue.global(qos: .userInitiated).async { Benchmark.run() }
    }

    var body: some Scene {
        WindowGroup { Text("Traverse Core ML spike running…") }
    }
}

enum Benchmark {
    static func floats(_ url: URL) throws -> [Float] {
        let data = try Data(contentsOf: url)
        return data.withUnsafeBytes { Array($0.bindMemory(to: Float.self)) }
    }

    static func sigmoid(_ x: Float) -> Double { 1 / (1 + exp(-Double(x))) }

    static func top5(_ v: [Float]) -> [Int] {
        Array(v.indices.sorted { v[$0] > v[$1] }.prefix(5))
    }

    static func run() {
        var report: [String: Any] = [:]
        do {
            let bundle = Bundle.main.resourceURL!
            let staged = FileManager.default.temporaryDirectory.appendingPathComponent("birdnet.mlpackage")
            try? FileManager.default.removeItem(at: staged)
            try FileManager.default.copyItem(at: bundle.appendingPathComponent("birdnet_pkg"), to: staged)
            var started = Date()
            let compiled = try MLModel.compileModel(at: staged)
            report["compile_s"] = Date().timeIntervalSince(started)

            let clipDir = bundle.appendingPathComponent("clips")
            let names = try FileManager.default.contentsOfDirectory(atPath: clipDir.path)
                .filter { $0.hasSuffix(".f32le") }.sorted()
            let inputs = try names.map { try floats(clipDir.appendingPathComponent($0)) }
            let refs = try names.map {
                try floats(bundle.appendingPathComponent("ref").appendingPathComponent(
                    $0.replacingOccurrences(of: ".f32le", with: ".tract.f32le")))
            }
            report["clips"] = names.count

            var results: [[String: Any]] = []
            let settings: [(String, MLComputeUnits)] = [
                ("all", .all), ("cpuAndGPU", .cpuAndGPU), ("cpuAndNeuralEngine", .cpuAndNeuralEngine), ("cpuOnly", .cpuOnly),
            ]
            for (name, units) in settings {
                let config = MLModelConfiguration()
                config.computeUnits = units
                started = Date()
                let model = try MLModel(contentsOf: compiled, configuration: config)
                let load = Date().timeIntervalSince(started)
                func predict(_ samples: [Float]) throws -> [Float] {
                    let array = try MLMultiArray(shape: [1, 144_000], dataType: .float32)
                    samples.withUnsafeBufferPointer { src in
                        array.dataPointer.bindMemory(to: Float.self, capacity: 144_000)
                            .update(from: src.baseAddress!, count: 144_000)
                    }
                    let out = try model.prediction(from: MLDictionaryFeatureProvider(dictionary: ["input": array]))
                    let y = out.featureValue(for: "output")!.multiArrayValue!
                    return (0..<y.count).map { y[$0].floatValue }
                }
                _ = try predict(inputs[0])  // warm-up
                var times: [Double] = []
                var worst = 0.0
                var top5Match = 0
                for (input, ref) in zip(inputs, refs) {
                    let t = Date()
                    let y = try predict(input)
                    times.append(Date().timeIntervalSince(t) * 1000)
                    worst = max(worst, zip(y, ref).map { abs(sigmoid($0) - sigmoid($1)) }.max() ?? 1)
                    if top5(y) == top5(ref) { top5Match += 1 }
                }
                let sorted = times.sorted()
                results.append([
                    "compute_units": name,
                    "load_s": load,
                    "mean_ms": times.reduce(0, +) / Double(times.count),
                    "p95_ms": sorted[Int((Double(sorted.count - 1) * 0.95).rounded())],
                    "max_ms": sorted.last!,
                    "worst_abs_dsig": worst,
                    "top5_identical": top5Match,
                ])
            }
            report["results"] = results
            report["device"] = ProcessInfo.processInfo.operatingSystemVersionString
        } catch {
            report["error"] = "\(error)"
        }
        let json = (try? JSONSerialization.data(withJSONObject: report, options: [.sortedKeys])) ?? Data()
        print("SPIKE_RESULT " + (String(data: json, encoding: .utf8) ?? "{}"))
        fflush(stdout)
        exit(0)
    }
}
