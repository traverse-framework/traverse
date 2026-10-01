import Foundation
import TraverseSwiftHost

/// Spec 138 exact-ref model failure (Decision 101 codes and stable reasons).
public struct ExactModelError: Error, Sendable, Equatable {
    /// Stable public code (`model_unavailable`, `model_incompatible`, `cancelled`, ...).
    public let code: String
    /// Stable reason refining `code` (`signature_invalid`, `rights_policy_denied`, ...).
    public let reason: String?
    /// Safe diagnostic message.
    public let message: String
    /// Structured rights-denial detail (Spec 138 0.8.0, Decision 107) on
    /// `rights_incomplete`, `rights_mismatch`, `rights_policy_denied`,
    /// `rights_inconsistent`, `package_revoked`, and `usage_undeclared`.
    public let detail: ModelRightsDenialDetail?

    public init(code: String, reason: String?, message: String, detail: ModelRightsDenialDetail? = nil) {
        self.code = code
        self.reason = reason
        self.message = message
        self.detail = detail
    }
}

/// Why a rights check failed, so a UI can explain it without re-deriving
/// policy. Identity is `nil` only before a package is known.
public struct ModelRightsDenialDetail: Sendable, Equatable, Codable {
    public let modelId: String?
    public let version: String?
    public let digest: String?
    /// Dotted path that failed, for example `rights.commercial_use`.
    public let field: String
    public let expected: String
    public let actual: String
    /// `commercial` or `non_commercial`, when it was decided.
    public let effectiveUsage: String?

    enum CodingKeys: String, CodingKey {
        case modelId = "model_id", version, digest, field, expected, actual
        case effectiveUsage = "effective_usage"
    }
}

/// Signed provenance of a derivative package (`rights.derivation`, manifest
/// schema `2.1.0`).
public struct ModelDerivation: Sendable, Equatable, Codable {
    /// `converted`, `quantized`, or `fine_tuned`.
    public let kind: String
    public let sourceDigest: String
    public let sourceLicenseId: String
    public let sourceCommercialUse: String
    public let sourceUrl: String

    enum CodingKeys: String, CodingKey {
        case kind, sourceDigest = "source_digest", sourceLicenseId = "source_license_id"
        case sourceCommercialUse = "source_commercial_use", sourceUrl = "source_url"
    }
}

/// Signed rights of a registered package, for host/UI display.
public struct ModelRights: Sendable, Equatable, Codable {
    public let licenseId: String
    public let attribution: String
    public let redistribution: String
    public let commercialUse: String
    public let sourceUrl: String
    /// Derivation provenance (manifest schema `2.1.0` only).
    public let derivation: ModelDerivation?

    enum CodingKeys: String, CodingKey {
        case licenseId = "license_id", attribution, redistribution
        case commercialUse = "commercial_use", sourceUrl = "source_url", derivation
    }
}

/// Host-owned package status entry (Decision 107): `deprecated` runs but is
/// flagged; `revoked` fails closed with `package_revoked`.
public struct PackageStatusEntry: Sendable, Equatable, Codable {
    /// `deprecated` or `revoked` (`active` is the same as no entry).
    public let status: String
    public let reason: String

    public init(status: String, reason: String) {
        self.status = status
        self.reason = reason
    }
}

/// Verified rights record of a package (Spec 138 0.8.0 FR-040): what the host
/// reports for display and what every execution carries as evidence.
public struct ModelRightsRecord: Sendable, Equatable, Codable {
    public let modelId: String
    public let version: String
    public let digest: String
    public let rights: ModelRights
    /// `active` or `deprecated`, plus `revoked` on a host query.
    public let status: String
    public let statusReason: String?
    /// `commercial` or `non_commercial`.
    public let effectiveUsage: String

    enum CodingKeys: String, CodingKey {
        case modelId = "model_id", version, digest, rights, status
        case statusReason = "status_reason", effectiveUsage = "effective_usage"
    }
}

/// Application `exact_model_dependencies` pin (Spec 138 0.4.0+).
public struct ExactModelPin: Sendable, Equatable, Codable {
    public struct Rights: Sendable, Equatable, Codable {
        public let licenseId: String
        public let commercialUse: String
        enum CodingKeys: String, CodingKey { case licenseId = "license_id", commercialUse = "commercial_use" }
        public init(licenseId: String, commercialUse: String) {
            self.licenseId = licenseId
            self.commercialUse = commercialUse
        }
    }

    public let modelId: String
    public let version: String
    /// SHA-256 of the exact signed `model.manifest.json` bytes.
    public let digest: String
    public let offlineAllowed: Bool
    public let target: String
    public let rights: Rights
    public let keyId: String?

    enum CodingKeys: String, CodingKey {
        case modelId = "model_id", version, digest, offlineAllowed = "offline_allowed"
        case target, rights, keyId = "key_id"
    }

    public init(modelId: String, version: String, digest: String, offlineAllowed: Bool = true,
                target: String = "wasm-cpu", rights: Rights, keyId: String? = nil) {
        self.modelId = modelId
        self.version = version
        self.digest = digest
        self.offlineAllowed = offlineAllowed
        self.target = target
        self.rights = rights
        self.keyId = keyId
    }
}

/// Host ceilings a package's declared limits must fit within (Decision 104).
/// Defaults are sized for phones; registration fails with `host_limit_exceeded`.
public struct ExactModelHostLimits: Sendable, Equatable {
    public let maxPackageBytes: UInt64
    public let maxMemoryBytes: UInt64
    /// `wasmi` fuel units (engine-relative; Spec 138 FR-030).
    public let maxFuel: UInt64

    public init(maxPackageBytes: UInt64 = 128 * 1024 * 1024,
                maxMemoryBytes: UInt64 = 256 * 1024 * 1024,
                maxFuel: UInt64 = 20_000_000_000) {
        self.maxPackageBytes = maxPackageBytes
        self.maxMemoryBytes = maxMemoryBytes
        self.maxFuel = maxFuel
    }
}

/// Typed `model.execute` result with identity, placement, and redacted trace.
public struct ExactModelExecution: Sendable, Equatable {
    public let outputRef: String
    public let placement: String
    public let target: String
    public let modelId: String
    public let version: String
    public let digest: String
    public let dataClassification: String
    public let inputBytes: Int
    public let outputBytes: Int
    public let durationMs: Double
    /// Verified rights record of the executed model (Spec 138 0.8.0 FR-040).
    public let modelEvidence: ModelRightsRecord?
}

/// Swift Spec 138 exact-ref model host (Decision 104). Verification and the
/// `wasmi` guest run in the audited Rust `TraverseSwiftHost` behind
/// `traverse_swift_host_model_call` (ADR-0078); trust stays host-owned.
public final class ExactModelHost: @unchecked Sendable {
    private let handle: UInt64
    private let responseCapacity = 64 * 1024

    /// - Parameters:
    ///   - modelUsage: the app manifest `model_usage` (`commercial` |
    ///     `non_commercial`, Spec 138 0.8.0); registration fails closed with
    ///     `usage_undeclared` when it is `nil`.
    ///   - hostRequiresCommercial: host tightening; the effective usage is
    ///     always `commercial`. A host can never relax an app's usage.
    public init(pins: [ExactModelPin], trustedPublicKeysHex: [String],
                modelUsage: String?,
                hostRequiresCommercial: Bool = false,
                limits: ExactModelHostLimits = ExactModelHostLimits()) throws {
        let pinsJSON = try JSONSerialization.jsonObject(with: JSONEncoder().encode(pins))
        var header: [String: Any] = [
            "op": "create",
            "pins": pinsJSON,
            "trusted_public_keys_hex": trustedPublicKeysHex,
            "limits": [
                "max_package_bytes": limits.maxPackageBytes,
                "max_memory_bytes": limits.maxMemoryBytes,
                "max_fuel": limits.maxFuel,
            ],
        ]
        if let modelUsage { header["model_usage"] = modelUsage }
        header["host_requires_commercial"] = hostRequiresCommercial
        let response = try Self.call(handle: 0, header: header, segments: [], capacity: 64 * 1024)
        guard let handle = (response.header["handle"] as? NSNumber)?.uint64Value else {
            throw ExactModelError(code: "unavailable", reason: nil, message: "model host was not created")
        }
        self.handle = handle
    }

    deinit {
        _ = try? Self.call(handle: handle, header: ["op": "destroy"], segments: [], capacity: 4096)
    }

    /// Verify and admit a signed package; returns the pin digest it is cached under.
    public func registerPackage(manifest: Data, wasm: Data, signature: Data) async throws -> String {
        let response = try await background {
            try Self.call(handle: self.handle, header: ["op": "register"],
                          segments: [("manifest", manifest), ("wasm", wasm), ("signature", signature)],
                          capacity: self.responseCapacity)
        }
        guard let digest = response.header["digest"] as? String else {
            throw ExactModelError(code: "model_incompatible", reason: nil, message: "registration returned no digest")
        }
        return digest
    }

    /// Stage input bytes; returns a single-consume opaque `input_ref`.
    public func stageModelInput(_ bytes: Data, maxBytes: Int) throws -> String {
        let response = try Self.call(handle: handle, header: ["op": "stage_input", "max_bytes": maxBytes],
                                     segments: [("input", bytes)], capacity: responseCapacity)
        guard let reference = response.header["input_ref"] as? String else {
            throw ExactModelError(code: "invalid_input", reason: nil, message: "staging returned no input_ref")
        }
        return reference
    }

    /// Read output bytes by `output_ref` (size-capped).
    public func readModelOutput(_ outputRef: String, maxBytes: Int) throws -> Data {
        let response = try Self.call(handle: handle, header: ["op": "read_output", "output_ref": outputRef, "max_bytes": maxBytes],
                                     segments: [], capacity: maxBytes + 4096)
        return response.segment("output") ?? Data()
    }

    /// Signed rights of a registered package, or `nil` when unknown.
    public func modelRights(digest: String) throws -> ModelRights? {
        let response = try Self.call(handle: handle, header: ["op": "rights", "digest": digest],
                                     segments: [], capacity: responseCapacity)
        guard let rights = response.header["rights"], !(rights is NSNull) else { return nil }
        return try JSONDecoder().decode(ModelRights.self, from: JSONSerialization.data(withJSONObject: rights))
    }

    /// Verified rights record (rights, status, effective usage) of a
    /// registered package, or `nil` when unknown or usage is undeclared.
    public func modelRightsRecord(digest: String) throws -> ModelRightsRecord? {
        let response = try Self.call(handle: handle, header: ["op": "rights_record", "digest": digest],
                                     segments: [], capacity: responseCapacity)
        return try Self.decodeOptional(ModelRightsRecord.self, response.header["record"])
    }

    /// Replace the host-owned package status map (digest → entry). Takes effect
    /// at the next registration or execute, so a revocation blocks the next call.
    public func setPackageStatus(_ entries: [String: PackageStatusEntry]) throws {
        let encoded = try JSONSerialization.jsonObject(with: JSONEncoder().encode(entries))
        _ = try Self.call(handle: handle, header: ["op": "set_package_status", "entries": encoded],
                          segments: [], capacity: responseCapacity)
    }

    private static func decodeOptional<T: Decodable>(_ type: T.Type, _ value: Any?) throws -> T? {
        guard let value, !(value is NSNull) else { return nil }
        return try JSONDecoder().decode(type, from: JSONSerialization.data(withJSONObject: value))
    }

    /// Drop a staged input or output ref.
    public func dropRef(_ reference: String) throws {
        _ = try Self.call(handle: handle, header: ["op": "drop_ref", "ref": reference], segments: [], capacity: responseCapacity)
    }

    /// Execute a pinned model. Swift `Task` cancellation interrupts the running
    /// inference mid-run (`cancelled`); `timeoutMs` bounds it (`timeout`).
    public func execute(
        modelRef: (modelId: String, version: String, digest: String),
        inputRef: String,
        policyRef: String,
        dataClassification: String,
        inputSchemaRef: String,
        inputSchemaVersion: String,
        maxOutputBytes: Int,
        allowedClassifications: [String],
        timeoutMs: Int? = nil
    ) async throws -> ExactModelExecution {
        var payload: [String: Any] = [
            "model_ref": ["model_id": modelRef.modelId, "version": modelRef.version, "digest": modelRef.digest],
            "input_ref": inputRef,
            "policy_ref": policyRef,
            "data_classification": dataClassification,
            "input_schema_ref": inputSchemaRef,
            "input_schema_version": inputSchemaVersion,
            "max_output_bytes": maxOutputBytes,
        ]
        if let timeoutMs { payload["timeout_ms"] = timeoutMs }
        return try await execute(payload: payload, allowedClassifications: allowedClassifications)
    }

    /// Execute a raw Spec 137 `model.execute` payload (used by app-command routing).
    public func execute(payload: [String: Any], allowedClassifications: [String]) async throws -> ExactModelExecution {
        try Task.checkCancellation()
        let executionID = UUID().uuidString
        let header: [String: Any] = [
            "op": "execute", "execution_id": executionID,
            "allowed_classifications": allowedClassifications, "payload": payload,
        ]
        let handle = handle
        let capacity = responseCapacity
        let request = try Self.encode(header: header, segments: [])
        let response = try await withTaskCancellationHandler {
            try await background { try Self.send(handle: handle, request: request, capacity: capacity) }
        } onCancel: {
            _ = try? Self.call(handle: handle, header: ["op": "cancel", "execution_id": executionID], segments: [], capacity: 4096)
        }
        let header2 = response.header
        let trace = header2["trace"] as? [String: Any] ?? [:]
        let usage = trace["usage"] as? [String: Any] ?? [:]
        let model = header2["model_ref"] as? [String: Any] ?? [:]
        return ExactModelExecution(
            outputRef: header2["output_ref"] as? String ?? "",
            placement: header2["placement"] as? String ?? "",
            target: header2["target"] as? String ?? "",
            modelId: model["model_id"] as? String ?? "",
            version: model["version"] as? String ?? "",
            digest: model["digest"] as? String ?? "",
            dataClassification: trace["data_classification"] as? String ?? "",
            inputBytes: (usage["input_bytes"] as? NSNumber)?.intValue ?? 0,
            outputBytes: (usage["output_bytes"] as? NSNumber)?.intValue ?? 0,
            durationMs: (usage["duration_ms"] as? NSNumber)?.doubleValue ?? 0,
            modelEvidence: try Self.decodeOptional(ModelRightsRecord.self, header2["model_evidence"])
        )
    }

    /// Register this host as the adapter for a manifest command routed to
    /// `traverse.model-runtime` / `model.execute`. The command payload is the
    /// Spec 137 `model.execute` payload plus `allowed_classifications`; its
    /// `input_ref` must be staged on this host. The adapter succeeds with
    /// `{output_ref, placement, model_id, version, digest, model_evidence}`;
    /// failures carry `reason` and the rights `detail`.
    @discardableResult
    public func install(on embedder: RuntimeTraverseEmbedder, command: String) throws -> HostConnectorRegistration {
        try embedder.registerHostConnectorAdapter(command: command, adapter: modelExecuteAdapter)
    }

    /// The host-connector adapter `install(on:command:)` registers.
    public var modelExecuteAdapter: HostConnectorAdapter {
        { [self] request in
            guard var payload = (try? JSONSerialization.jsonObject(with: request.payloadJSON)) as? [String: Any] else {
                return Self.failed("invalid_input", nil, "model.execute payload is not a JSON object")
            }
            let allowed = payload.removeValue(forKey: "allowed_classifications") as? [String] ?? []
            do {
                let result = try await execute(payload: payload, allowedClassifications: allowed)
                var body: [String: Any] = [
                    "output_ref": result.outputRef, "placement": result.placement,
                    "model_id": result.modelId, "version": result.version, "digest": result.digest,
                ]
                if let evidence = result.modelEvidence,
                   let object = try? JSONSerialization.jsonObject(with: JSONEncoder().encode(evidence)) {
                    body["model_evidence"] = object
                }
                return HostConnectorResult(resultClass: "succeeded",
                                           payloadJSON: (try? JSONSerialization.data(withJSONObject: body)) ?? Data("{}".utf8))
            } catch let error as ExactModelError {
                let resultClass = ["cancelled", "timeout"].contains(error.code) ? error.code : "failed"
                return Self.failed(error.code, error.reason, error.message, detail: error.detail, resultClass: resultClass)
            } catch is CancellationError {
                return Self.failed("cancelled", nil, "model.execute cancelled", resultClass: "cancelled")
            }
        }
    }

    private static func failed(_ code: String, _ reason: String?, _ message: String,
                               detail: ModelRightsDenialDetail? = nil, resultClass: String = "failed") -> HostConnectorResult {
        var body: [String: Any] = ["error_code": code, "message": message]
        if let reason { body["reason"] = reason }
        if let detail, let object = try? JSONSerialization.jsonObject(with: JSONEncoder().encode(detail)) {
            body["detail"] = object
        }
        return HostConnectorResult(resultClass: resultClass,
                                   payloadJSON: (try? JSONSerialization.data(withJSONObject: body)) ?? Data("{}".utf8))
    }

    private func background<T: Sendable>(_ work: @escaping @Sendable () throws -> T) async throws -> T {
        try await withCheckedThrowingContinuation { continuation in
            DispatchQueue.global(qos: .userInitiated).async {
                continuation.resume(with: Result { try work() })
            }
        }
    }

    // MARK: - Framed ABI call

    struct Response: @unchecked Sendable {
        let header: [String: Any]
        let payload: Data

        func segment(_ name: String) -> Data? {
            guard let segments = header["segments"] as? [String: Any],
                  let range = segments[name] as? [NSNumber], range.count == 2 else { return nil }
            let start = range[0].intValue
            let end = start + range[1].intValue
            guard start >= 0, end <= payload.count else { return nil }
            return payload.subdata(in: start..<end)
        }
    }

    static func encode(header: [String: Any], segments: [(String, Data)]) throws -> Data {
        var header = header
        var payload = Data()
        var offsets: [String: [Int]] = [:]
        for (name, bytes) in segments {
            offsets[name] = [payload.count, bytes.count]
            payload.append(bytes)
        }
        if !segments.isEmpty { header["segments"] = offsets }
        let headerData = try JSONSerialization.data(withJSONObject: header)
        var length = UInt32(headerData.count).littleEndian
        var frame = Data(bytes: &length, count: 4)
        frame.append(headerData)
        frame.append(payload)
        return frame
    }

    static func decode(_ frame: Data) throws -> Response {
        guard frame.count >= 4 else { throw invalidResponse() }
        let length = frame.prefix(4).withUnsafeBytes { Int(UInt32(littleEndian: $0.loadUnaligned(as: UInt32.self))) }
        guard frame.count >= 4 + length,
              let header = try JSONSerialization.jsonObject(with: frame.subdata(in: 4..<(4 + length))) as? [String: Any]
        else { throw invalidResponse() }
        return Response(header: header, payload: frame.subdata(in: (4 + length)..<frame.count))
    }

    private static func invalidResponse() -> ExactModelError {
        ExactModelError(code: "unavailable", reason: nil, message: "malformed model host response")
    }

    static func call(handle: UInt64, header: [String: Any], segments: [(String, Data)], capacity: Int) throws -> Response {
        try send(handle: handle, request: encode(header: header, segments: segments), capacity: capacity)
    }

    static func send(handle: UInt64, request: Data, capacity: Int) throws -> Response {
        var output = Data(repeating: 0, count: capacity)
        var required = 0
        var status = raw(handle: handle, request: request, output: &output, required: &required)
        if status == -6 {
            // BUFFER_TOO_SMALL: retry once at the exact size. Idempotent by
            // construction (ADR-0078): execute responses are header-only.
            output = Data(repeating: 0, count: required)
            status = raw(handle: handle, request: request, output: &output, required: &required)
        }
        guard status == 0 else {
            throw ExactModelError(code: status == -1 ? "unavailable" : "invalid_input", reason: nil,
                                  message: String(cString: traverse_swift_host_status_message(status)))
        }
        output.count = required
        let response = try decode(output)
        if response.header["ok"] as? Bool == false {
            let error = response.header["error"] as? [String: Any] ?? [:]
            throw ExactModelError(code: error["code"] as? String ?? "unavailable",
                                  reason: error["reason"] as? String,
                                  message: error["message"] as? String ?? "",
                                  detail: try? decodeOptional(ModelRightsDenialDetail.self, error["detail"]))
        }
        return response
    }

    private static func raw(handle: UInt64, request: Data, output: inout Data, required: inout Int) -> Int32 {
        let capacity = output.count
        return request.withUnsafeBytes { requestBuffer in
            output.withUnsafeMutableBytes { outputBuffer in
                traverse_swift_host_model_call(
                    handle,
                    requestBuffer.bindMemory(to: UInt8.self).baseAddress,
                    request.count,
                    outputBuffer.bindMemory(to: UInt8.self).baseAddress,
                    capacity,
                    &required
                )
            }
        }
    }
}
