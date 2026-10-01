import Foundation
import XCTest
@testable import TraverseEmbedder

/// Shared Spec 138 rights conformance suite (0.8.0, Decision 107, FR-041) on
/// the Swift host: every data-only case in
/// `fixtures/models/rights-conformance/suite.json` runs on a fresh
/// `ExactModelHost` and must match the native and web results exactly (code,
/// reason, detail, rights records, and `model_evidence`).
final class RightsConformanceTests: XCTestCase {
    private static let repoRoot: URL = {
        if let root = ProcessInfo.processInfo.environment["TRAVERSE_REPO_ROOT"] {
            return URL(fileURLWithPath: root)
        }
        // Tests/TraverseEmbedderTests/<file> -> repo root.
        var url = URL(fileURLWithPath: #filePath)
        for _ in 0..<6 { url.deleteLastPathComponent() }
        return url
    }()

    private func hexData(_ hex: String) -> Data {
        var bytes = [UInt8]()
        var index = hex.startIndex
        while index < hex.endIndex, let next = hex.index(index, offsetBy: 2, limitedBy: hex.endIndex) {
            if let byte = UInt8(hex[index..<next], radix: 16) { bytes.append(byte) }
            index = next
        }
        return Data(bytes)
    }

    private func file(_ path: String) throws -> Data {
        try Data(contentsOf: Self.repoRoot.appendingPathComponent(path))
    }

    /// Codable value → JSON object, so it compares like native serde output.
    private func jsonObject<T: Encodable>(_ value: T?) throws -> Any {
        guard let value else { return NSNull() }
        return try JSONSerialization.jsonObject(with: JSONEncoder().encode(value))
    }

    private func errorJSON(_ error: Error) throws -> [String: Any] {
        let error = try XCTUnwrap(error as? ExactModelError, "unexpected error \(error)")
        var out: [String: Any] = ["ok": false, "code": error.code, "reason": error.reason ?? NSNull()]
        if let detail = error.detail { out["detail"] = try jsonObject(detail) }
        return out
    }

    private func pin(_ testCase: [String: Any], _ package: String) throws -> [String: Any] {
        let pins = try XCTUnwrap(testCase["pins"] as? [[String: Any]])
        return try XCTUnwrap(pins.first { $0["model_id"] as? String == "fixture.rights.\(package)" })
    }

    private func statusEntries(_ value: Any?) throws -> [String: PackageStatusEntry] {
        guard let value, !(value is NSNull) else { return [:] }
        return try JSONDecoder().decode([String: PackageStatusEntry].self,
                                        from: JSONSerialization.data(withJSONObject: value))
    }

    private func register(_ host: ExactModelHost, suite: [String: Any], step: [String: Any]) async throws -> Any {
        let package = try XCTUnwrap(step["package"] as? String)
        let dir = "\(try XCTUnwrap(suite["package_dir"] as? String))/\(package)"
        var wasm = try file(try XCTUnwrap(suite["wasm_path"] as? String))
        var signature = try file("\(dir)/model.sig.json")
        switch step["tamper"] as? String {
        case "wasm":
            wasm.append(0)
        case "signature":
            var document = try XCTUnwrap(JSONSerialization.jsonObject(with: signature) as? [String: Any])
            var bytes = hexData(try XCTUnwrap(document["signature"] as? String))
            bytes[0] ^= 0x01
            document["signature"] = bytes.map { String(format: "%02x", $0) }.joined()
            signature = try JSONSerialization.data(withJSONObject: document)
        default:
            XCTAssertNil(step["tamper"], "unknown tamper")
        }
        do {
            let digest = try await host.registerPackage(manifest: try file("\(dir)/model.manifest.json"),
                                                        wasm: wasm, signature: signature)
            return ["ok": true, "digest": digest]
        } catch {
            return try errorJSON(error)
        }
    }

    private func execute(_ host: ExactModelHost, suite: [String: Any], pin: [String: Any]) async throws -> Any {
        let run = try XCTUnwrap(suite["execute"] as? [String: Any])
        let inputRef = try host.stageModelInput(hexData(try XCTUnwrap(run["input_hex"] as? String)), maxBytes: 4096)
        do {
            let result = try await host.execute(
                modelRef: (try XCTUnwrap(pin["model_id"] as? String), try XCTUnwrap(pin["version"] as? String),
                           try XCTUnwrap(pin["digest"] as? String)),
                inputRef: inputRef,
                policyRef: try XCTUnwrap(run["policy_ref"] as? String),
                dataClassification: try XCTUnwrap(run["data_classification"] as? String),
                inputSchemaRef: try XCTUnwrap(run["input_schema_ref"] as? String),
                inputSchemaVersion: try XCTUnwrap(run["input_schema_version"] as? String),
                maxOutputBytes: try XCTUnwrap(run["max_output_bytes"] as? Int),
                allowedClassifications: try XCTUnwrap(run["allowed_classifications"] as? [String]))
            let output = try host.readModelOutput(result.outputRef, maxBytes: 4096)
            return [
                "ok": true,
                "output_hex": output.map { String(format: "%02x", $0) }.joined(),
                "model_evidence": try jsonObject(result.modelEvidence),
            ]
        } catch {
            return try errorJSON(error)
        }
    }

    func testRightsConformanceSuiteMatchesNativeAndWeb() async throws {
        let suite = try XCTUnwrap(JSONSerialization.jsonObject(with: file("fixtures/models/rights-conformance/suite.json")) as? [String: Any])
        // The rights ops ship in the swift-host-v0.14.0-2 xcframework. Until
        // Package.swift is repointed at it, the linked binary rejects them as
        // an unknown op (envelope `invalid_input`); skip loudly, not silently.
        let probe = try ExactModelHost(pins: [], trustedPublicKeysHex: [], modelUsage: "commercial")
        do {
            try probe.setPackageStatus([:])
        } catch let error as ExactModelError where error.code == "invalid_input" {
            throw XCTSkip("linked TraverseSwiftHost predates the Spec 138 0.8.0 rights ops (needs swift-host-v0.14.0-2+)")
        }
        let cases = try XCTUnwrap(suite["cases"] as? [[String: Any]])
        var scenarios: Set<Int> = [10]
        for testCase in cases {
            let id = try XCTUnwrap(testCase["id"] as? String)
            let pins = try JSONDecoder().decode([ExactModelPin].self,
                                                from: JSONSerialization.data(withJSONObject: try XCTUnwrap(testCase["pins"])))
            let host = try ExactModelHost(
                pins: pins,
                trustedPublicKeysHex: [try XCTUnwrap(suite["trusted_public_key_hex"] as? String)],
                modelUsage: testCase["model_usage"] as? String,
                hostRequiresCommercial: testCase["host_requires_commercial"] as? Bool ?? false)
            try host.setPackageStatus(try statusEntries(testCase["package_status"]))
            for (index, step) in try XCTUnwrap(testCase["steps"] as? [[String: Any]]).enumerated() {
                let op = try XCTUnwrap(step["op"] as? String)
                let actual: Any
                switch op {
                case "register":
                    actual = try await register(host, suite: suite, step: step)
                case "execute":
                    actual = try await execute(host, suite: suite, pin: try pin(testCase, try XCTUnwrap(step["package"] as? String)))
                case "rights_record":
                    let digest = try XCTUnwrap(try pin(testCase, try XCTUnwrap(step["package"] as? String))["digest"] as? String)
                    actual = try jsonObject(try host.modelRightsRecord(digest: digest))
                default:
                    XCTAssertEqual(op, "set_package_status")
                    try host.setPackageStatus(try statusEntries(step["entries"]))
                    continue
                }
                XCTAssertEqual(actual as? NSObject, step["expect"] as? NSObject, "\(id) step \(index)")
            }
            for scenario in try XCTUnwrap(testCase["scenarios"] as? [Int]) { scenarios.insert(scenario) }
        }
        XCTAssertEqual(scenarios, Set(1...10))
    }
}
