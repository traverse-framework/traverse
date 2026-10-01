import Foundation
import XCTest
@testable import TraverseEmbedder

/// #1591: the simd128 ONNX runner package (`digits-onnx-1.0.0`, built by
/// `traverse-cli model package-onnx`) runs on the Swift host's `wasmi` with
/// output byte-identical to the vector wasmtime, native `wasmi`, and the
/// browser match, and exposes its signed `rights.derivation`.
final class OnnxRunnerTests: XCTestCase {
    private static let repoRoot: URL = {
        if let root = ProcessInfo.processInfo.environment["TRAVERSE_REPO_ROOT"] {
            return URL(fileURLWithPath: root)
        }
        var url = URL(fileURLWithPath: #filePath)
        for _ in 0..<6 { url.deleteLastPathComponent() }
        return url
    }()

    private func models(_ path: String) throws -> Data {
        try Data(contentsOf: Self.repoRoot.appendingPathComponent("fixtures/models/\(path)"))
    }

    private func hexData(_ hex: String) -> Data {
        var bytes = [UInt8]()
        var index = hex.startIndex
        while index < hex.endIndex, let next = hex.index(index, offsetBy: 2, limitedBy: hex.endIndex) {
            if let byte = UInt8(hex[index..<next], radix: 16) { bytes.append(byte) }
            index = next
        }
        return Data(bytes)
    }

    func testOnnxRunnerVectorIsByteIdenticalOnSwiftWasmi() async throws {
        let vector = try XCTUnwrap(JSONSerialization.jsonObject(with: models("conformance/signed-digits-onnx.json")) as? [String: Any])
        let pin = try JSONDecoder().decode(ExactModelPin.self,
                                           from: JSONSerialization.data(withJSONObject: try XCTUnwrap(vector["pin"])))
        let host = try ExactModelHost(pins: [pin],
                                      trustedPublicKeysHex: [try XCTUnwrap(vector["trusted_public_key_hex"] as? String)],
                                      modelUsage: "commercial")
        let digest = try await host.registerPackage(
            manifest: try models("digits-onnx-1.0.0/model.manifest.json"),
            wasm: try models("digits-onnx-1.0.0/model.wasm"),
            signature: try models("digits-onnx-1.0.0/model.sig.json"))
        XCTAssertEqual(digest, pin.digest)

        let request = try XCTUnwrap(vector["request"] as? [String: Any])
        for testCase in try XCTUnwrap(vector["cases"] as? [[String: Any]]) {
            let inputRef = try host.stageModelInput(hexData(try XCTUnwrap(testCase["input_frame_hex"] as? String)), maxBytes: 4096)
            let result = try await host.execute(
                modelRef: (pin.modelId, pin.version, pin.digest), inputRef: inputRef, policyRef: "policy-1",
                dataClassification: "sensitive",
                inputSchemaRef: try XCTUnwrap(request["input_schema_ref"] as? String),
                inputSchemaVersion: "1.0.0", maxOutputBytes: 56, allowedClassifications: ["sensitive"])
            let output = try host.readModelOutput(result.outputRef, maxBytes: 56)
            XCTAssertEqual(output.map { String(format: "%02x", $0) }.joined(), testCase["output_frame_hex"] as? String)
            XCTAssertEqual(result.modelEvidence?.rights.derivation?.kind, "converted")
        }

        let derivation = try XCTUnwrap(try host.modelRights(digest: digest)?.derivation)
        XCTAssertEqual(derivation.sourceLicenseId, "CC-BY-4.0")
        XCTAssertEqual(derivation.sourceCommercialUse, "allowed")
        XCTAssertEqual(derivation.sourceDigest.count, 64)
    }
}
