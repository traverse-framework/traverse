import CryptoKit
import Foundation
import XCTest
@testable import TraverseEmbedder

/// Spec 138 exact-ref model execution on the Swift host (Decision 104, #1579):
/// the signed conformance packages run on `wasmi` behind
/// `traverse_swift_host_model_call` with byte-identical output to native/web.
final class ExactModelHostTests: XCTestCase {
    private static let repoRoot: URL = {
        if let root = ProcessInfo.processInfo.environment["TRAVERSE_REPO_ROOT"] {
            return URL(fileURLWithPath: root)
        }
        // Tests/TraverseEmbedderTests/<file> -> repo root (5 levels up).
        var url = URL(fileURLWithPath: #filePath)
        for _ in 0..<6 { url.deleteLastPathComponent() }
        return url
    }()

    private func fixture(_ path: String) throws -> Data {
        try Data(contentsOf: Self.repoRoot.appendingPathComponent("fixtures/models/\(path)"))
    }

    private func json(_ path: String) throws -> [String: Any] {
        try XCTUnwrap(JSONSerialization.jsonObject(with: fixture(path)) as? [String: Any])
    }

    private func pin(from value: Any?) throws -> ExactModelPin {
        try JSONDecoder().decode(ExactModelPin.self, from: JSONSerialization.data(withJSONObject: try XCTUnwrap(value)))
    }

    private func testKey() throws -> (publicHex: String, secret: Data) {
        let key = try json("test-signing-key.json")
        let secretHex = try XCTUnwrap(key["secret_key_hex"] as? String)
        return (try XCTUnwrap(key["public_key_hex"] as? String), Data(hex: secretHex))
    }

    private func digitsHost(limits: ExactModelHostLimits = ExactModelHostLimits()) async throws -> (ExactModelHost, ExactModelPin, [String: Any]) {
        let vector = try json("conformance/signed-digits-mlp.json")
        let pin = try pin(from: vector["pin"])
        let host = try ExactModelHost(pins: [pin], trustedPublicKeysHex: [try XCTUnwrap(vector["trusted_public_key_hex"] as? String)], modelUsage: "commercial", limits: limits)
        return (host, pin, vector)
    }

    private func registerDigits(_ host: ExactModelHost) async throws -> String {
        try await host.registerPackage(
            manifest: try fixture("digits-mlp-1.0.0/model.manifest.json"),
            wasm: try fixture("digits-mlp-1.0.0/model.wasm"),
            signature: try fixture("digits-mlp-1.0.0/model.sig.json"))
    }

    private static func run(_ host: ExactModelHost, _ pin: ExactModelPin, _ frame: Data, timeoutMs: Int? = nil) async throws -> ExactModelExecution {
        let inputRef = try host.stageModelInput(frame, maxBytes: 4096)
        return try await host.execute(
            modelRef: (pin.modelId, pin.version, pin.digest), inputRef: inputRef, policyRef: "policy-1",
            dataClassification: "sensitive", inputSchemaRef: "schema:traverse-digits-mlp-in",
            inputSchemaVersion: "1.0.0", maxOutputBytes: 64, allowedClassifications: ["sensitive"], timeoutMs: timeoutMs)
    }

    func testSignedDigitsVectorIsByteIdenticalAndRightsAreExposed() async throws {
        let (host, pin, vector) = try await digitsHost()
        let digest = try await registerDigits(host)
        XCTAssertEqual(digest, pin.digest)
        for testCase in try XCTUnwrap(vector["cases"] as? [[String: Any]]) {
            let input = Data(hex: try XCTUnwrap(testCase["input_frame_hex"] as? String))
            let result = try await Self.run(host, pin, input)
            XCTAssertEqual(result.placement, "wasm-cpu")
            XCTAssertEqual(result.target, "wasm-cpu")
            XCTAssertEqual(result.digest, pin.digest)
            XCTAssertEqual(result.inputBytes, input.count)
            XCTAssertEqual(result.outputBytes, 56)
            let output = try host.readModelOutput(result.outputRef, maxBytes: 64)
            XCTAssertEqual(output.hexString, testCase["output_frame_hex"] as? String)
        }
        let rights = try XCTUnwrap(try host.modelRights(digest: digest))
        XCTAssertEqual(rights.licenseId, "CC-BY-4.0")
        XCTAssertEqual(rights.commercialUse, "allowed")
        XCTAssertTrue(rights.attribution.contains("10.24432/C50P49"))
        XCTAssertNil(try host.modelRights(digest: "00"))
    }

    func testDigitsModelScoresTheSameHeldOutAccuracyAsNativeAndWeb() async throws {
        let (host, pin, _) = try await digitsHost()
        _ = try await registerDigits(host)
        let rows = try String(contentsOf: Self.repoRoot.appendingPathComponent("fixtures/datasets/uci-optdigits/optdigits.tes"), encoding: .utf8)
            .split(separator: "\n")
        var correct = 0
        for row in rows {
            var values = row.split(separator: ",").compactMap { Float($0) }
            let label = values.removeLast()
            var payload = Data()
            for value in values { withUnsafeBytes(of: value.bitPattern.littleEndian) { payload.append(contentsOf: $0) } }
            var frame = Data([1, 0, 2, 1])
            withUnsafeBytes(of: UInt32(64).littleEndian) { frame.append(contentsOf: $0) }
            withUnsafeBytes(of: UInt32(payload.count).littleEndian) { frame.append(contentsOf: $0) }
            frame.append(payload)
            let result = try await Self.run(host, pin, frame)
            let output = try host.readModelOutput(result.outputRef, maxBytes: 64)
            let predicted = output.subdata(in: 52..<56).withUnsafeBytes { Float(bitPattern: UInt32(littleEndian: $0.loadUnaligned(as: UInt32.self))) }
            if predicted == label { correct += 1 }
        }
        XCTAssertEqual(rows.count, 1797)
        XCTAssertEqual(correct, 1727, "same count as the trainer, native wasmtime, and the browser")
    }

    func testFailuresCarryStableReasons() async throws {
        let (host, _, _) = try await digitsHost()
        do {
            _ = try await host.registerPackage(manifest: Data("{}".utf8),
                                               wasm: try fixture("digits-mlp-1.0.0/model.wasm"),
                                               signature: try fixture("digits-mlp-1.0.0/model.sig.json"))
            XCTFail("tampered manifest registered")
        } catch let error as ExactModelError {
            XCTAssertEqual(error.reason, "signature_invalid")
        }
        let (tight, _, _) = try await digitsHost(limits: ExactModelHostLimits(maxPackageBytes: 100))
        do {
            _ = try await registerDigits(tight)
            XCTFail("oversized package registered")
        } catch let error as ExactModelError {
            XCTAssertEqual(error.code, "model_incompatible")
            XCTAssertEqual(error.reason, "host_limit_exceeded")
        }
        XCTAssertThrowsError(try host.stageModelInput(Data([1, 2]), maxBytes: 1)) { error in
            XCTAssertEqual((error as? ExactModelError)?.code, "input_limit_exceeded")
        }
    }

    /// A signed guest that loops until cancelled, timed out, or out of fuel.
    private func looperHost() async throws -> (ExactModelHost, ExactModelPin) {
        let wasm = Data([
            0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, // magic + version
            0x01, 0x09, 0x01, 0x60, 0x04, 0x7f, 0x7f, 0x7f, 0x7f, 0x01, 0x7f, // type (i32 x4) -> i32
            0x03, 0x02, 0x01, 0x00, // func
            0x05, 0x03, 0x01, 0x00, 0x01, // memory 1
            0x07, 0x1a, 0x02, 0x06, 0x6d, 0x65, 0x6d, 0x6f, 0x72, 0x79, 0x02, 0x00, // export memory
            0x0d, 0x6d, 0x6f, 0x64, 0x65, 0x6c, 0x5f, 0x65, 0x78, 0x65, 0x63, 0x75, 0x74, 0x65, 0x00, 0x00, // export model_execute
            0x0a, 0x0b, 0x01, 0x09, 0x00, 0x03, 0x40, 0x0c, 0x00, 0x0b, 0x41, 0x00, 0x0b, // loop br 0; i32.const 0
        ])
        let manifest: [String: Any] = [
            "schema_version": "2.0.0", "model_id": "test.looper", "version": "1.0.0",
            "wasm_digest": SHA256.hash(data: wasm).hexString, "registry_ref": "registry:test.looper@1.0.0",
            "executable_format": "traverse-model-wasm", "abi_version": 1,
            "input_schema_ref": "schema:traverse-digits-mlp-in", "input_schema_version": "1.0.0",
            "output_schema_ref": "schema:x", "output_schema_version": "1.0.0",
            "rights": ["license_id": "Apache-2.0", "attribution": "test", "redistribution": "test",
                       "commercial_use": "allowed", "source_url": "https://example.invalid"],
            "supported_profiles": ["wasm-cpu"], "max_memory_bytes": 131_072, "max_fuel": 9_000_000_000,
            "max_input_bytes": 4096, "max_output_bytes": 64, "max_execution_ms": 60_000, "offline_allowed": true,
        ]
        let manifestBytes = try JSONSerialization.data(withJSONObject: manifest)
        let (publicHex, secret) = try testKey()
        let signingKey = try Curve25519.Signing.PrivateKey(rawRepresentation: secret)
        let keyID = "ed25519:" + SHA256.hash(data: signingKey.publicKey.rawRepresentation).hexString
        let signature = try JSONSerialization.data(withJSONObject: [
            "alg": "ed25519", "key_id": keyID,
            "signature": try signingKey.signature(for: manifestBytes).hexString,
        ])
        let pin = ExactModelPin(modelId: "test.looper", version: "1.0.0",
                                digest: SHA256.hash(data: manifestBytes).hexString,
                                rights: .init(licenseId: "Apache-2.0", commercialUse: "allowed"))
        let host = try ExactModelHost(pins: [pin], trustedPublicKeysHex: [publicHex], modelUsage: "commercial")
        _ = try await host.registerPackage(manifest: manifestBytes, wasm: wasm, signature: signature)
        return (host, pin)
    }

    func testTaskCancellationInterruptsARunningInferenceMidRun() async throws {
        let (host, pin) = try await looperHost()
        let frame = Data([1, 0, 2, 1, 1, 0, 0, 0, 4, 0, 0, 0, 0, 0, 0, 0])
        let runner: @Sendable () async throws -> ExactModelExecution = {
            try await ExactModelHostTests.run(host, pin, frame)
        }
        let task = Task(operation: runner)
        try await Task.sleep(nanoseconds: 200_000_000)
        task.cancel()
        let started = Date()
        do {
            _ = try await task.value
            XCTFail("looping inference finished")
        } catch let error as ExactModelError {
            XCTAssertEqual(error.code, "cancelled")
        }
        XCTAssertLessThan(Date().timeIntervalSince(started), 5, "cancellation must interrupt mid-run")
    }

    func testDeadlineInterruptsARunningInferenceMidRun() async throws {
        let (host, pin) = try await looperHost()
        let frame = Data([1, 0, 2, 1, 1, 0, 0, 0, 4, 0, 0, 0, 0, 0, 0, 0])
        do {
            _ = try await Self.run(host, pin, frame, timeoutMs: 100)
            XCTFail("looping inference finished")
        } catch let error as ExactModelError {
            XCTAssertEqual(error.code, "timeout")
        }
    }

    func testModelExecuteAdapterRoutesAppCommands() async throws {
        let (host, pin, vector) = try await digitsHost()
        _ = try await registerDigits(host)
        let testCase = try XCTUnwrap((vector["cases"] as? [[String: Any]])?.first)
        let inputRef = try host.stageModelInput(Data(hex: try XCTUnwrap(testCase["input_frame_hex"] as? String)), maxBytes: 4096)
        let payload: [String: Any] = [
            "model_ref": ["model_id": pin.modelId, "version": pin.version, "digest": pin.digest],
            "input_ref": inputRef, "policy_ref": "policy-1", "data_classification": "sensitive",
            "input_schema_ref": "schema:traverse-digits-mlp-in", "input_schema_version": "1.0.0",
            "max_output_bytes": 64, "allowed_classifications": ["sensitive"],
        ]
        let request = HostConnectorRequest(command: "classify_digit", commandID: "cmd-1", sessionID: "s-1",
                                           payloadJSON: try JSONSerialization.data(withJSONObject: payload))
        let result = try await host.modelExecuteAdapter(request)
        XCTAssertEqual(result.resultClass, "succeeded")
        let body = try XCTUnwrap(JSONSerialization.jsonObject(with: result.payloadJSON) as? [String: Any])
        let output = try host.readModelOutput(try XCTUnwrap(body["output_ref"] as? String), maxBytes: 64)
        XCTAssertEqual(output.hexString, testCase["output_frame_hex"] as? String)

        let denied = try await host.modelExecuteAdapter(HostConnectorRequest(
            command: "classify_digit", commandID: "cmd-2", sessionID: "s-1", payloadJSON: Data("[]".utf8)))
        XCTAssertEqual(denied.resultClass, "failed")
        let missing = try await host.modelExecuteAdapter(HostConnectorRequest(
            command: "classify_digit", commandID: "cmd-3", sessionID: "s-1",
            payloadJSON: try JSONSerialization.data(withJSONObject: payload)))
        XCTAssertEqual(missing.resultClass, "failed", "a consumed input_ref fails closed")
    }
}

private extension Data {
    init(hex: String) {
        var bytes = [UInt8]()
        var index = hex.startIndex
        while index < hex.endIndex, let next = hex.index(index, offsetBy: 2, limitedBy: hex.endIndex) {
            if let byte = UInt8(hex[index..<next], radix: 16) { bytes.append(byte) }
            index = next
        }
        self.init(bytes)
    }

    var hexString: String { map { String(format: "%02x", $0) }.joined() }
}

private extension Digest {
    var hexString: String { map { String(format: "%02x", $0) }.joined() }
}
