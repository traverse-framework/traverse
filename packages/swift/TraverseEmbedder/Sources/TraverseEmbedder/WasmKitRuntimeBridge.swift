import CryptoKit
import Foundation
import WasmKit

/// Production loader for the governed `runtime-wasm-bridge/1.1.0` module.
///
/// The loader verifies the artifact before WasmKit parses or instantiates it,
/// rejects ambient imports, and validates the complete embedder export surface.
@available(*, deprecated, message: "Use WasmiHostBridgeClient; WasmKit is retained only for source compatibility.")
public final class WasmKitRuntimeBridge: @unchecked Sendable {
    public static let abiVersion = 10_100
    public static let defaultMaximumArtifactBytes = 32 * 1024 * 1024

    private static let requiredFunctions: [(String, [ValueType], [ValueType])] = [
        ("traverse_bridge_abi_version", [], [.i32]),
        ("traverse_alloc", [.i32], [.i32]),
        ("traverse_dealloc", [.i32, .i32], []),
        ("traverse_init", [.i32, .i32, .i32], [.i32]),
        ("traverse_submit", [.i32, .i32, .i32], [.i32]),
        ("traverse_next_event", [.i32], [.i32]),
        ("traverse_cancel", [.i32, .i32, .i32], [.i32]),
        ("traverse_compatible_start", [.i32, .i32, .i32], [.i32]),
        ("traverse_compatible_stop", [.i32, .i32, .i32], [.i32]),
        ("traverse_compatible_kill", [.i32, .i32, .i32], [.i32]),
        ("traverse_shutdown", [.i32], [.i32]),
    ]

    private let store: Store
    let instance: Instance

    public let runtimeURL: URL
    public let runtimeWasmDigest: String

    public init(
        bundle: TraverseBundle,
        maximumArtifactBytes: Int = WasmKitRuntimeBridge.defaultMaximumArtifactBytes
    ) throws {
        guard bundle.embedderAPIVersion == TraverseEmbedder.apiVersion else {
            throw TraverseEmbedderError.incompatibleBundle(
                "embedder API \(bundle.embedderAPIVersion) is incompatible with \(TraverseEmbedder.apiVersion)"
            )
        }
        guard maximumArtifactBytes > 0 else {
            throw TraverseEmbedderError.incompatibleBundle("maximum runtime WASM size must be positive")
        }

        let runtimeURL = bundle.rootURL
            .appendingPathComponent("runtime", isDirectory: true)
            .appendingPathComponent("runtime.wasm", isDirectory: false)
        let bytes: Data
        do {
            bytes = try Data(contentsOf: runtimeURL, options: [.mappedIfSafe])
        } catch {
            throw TraverseEmbedderError.incompatibleBundle("runtime/runtime.wasm is unavailable")
        }
        guard bytes.count <= maximumArtifactBytes else {
            throw TraverseEmbedderError.incompatibleBundle("runtime/runtime.wasm exceeds the configured size limit")
        }

        let actualDigest = "sha256:" + SHA256.hash(data: bytes).map { String(format: "%02x", $0) }.joined()
        guard Self.normalizedDigest(bundle.runtimeWasmDigest) == actualDigest else {
            throw TraverseEmbedderError.incompatibleBundle("bundle_digest_mismatch")
        }

        let module: Module
        do {
            module = try parseWasm(bytes: Array(bytes))
        } catch {
            throw TraverseEmbedderError.incompatibleBundle("runtime/runtime.wasm is not a valid core WebAssembly module")
        }
        guard module.imports.isEmpty else {
            throw TraverseEmbedderError.incompatibleBundle("runtime/runtime.wasm requires undeclared ambient imports")
        }

        let store = Store(engine: Engine())
        let instance: Instance
        do {
            instance = try module.instantiate(store: store)
        } catch {
            throw TraverseEmbedderError.incompatibleBundle("runtime/runtime.wasm could not be instantiated")
        }
        let memoryExports = instance.exports.filter {
            if case .memory = $0.value { return true }
            return false
        }
        guard memoryExports.count == 1, instance.exports[memory: "memory"] != nil else {
            throw TraverseEmbedderError.incompatibleBundle("runtime/runtime.wasm must export exactly one bridge memory")
        }
        for (name, parameters, results) in Self.requiredFunctions {
            guard let function = instance.exports[function: name] else {
                throw TraverseEmbedderError.incompatibleBundle("runtime/runtime.wasm is missing required export \(name)")
            }
            guard function.type.parameters == parameters, function.type.results == results else {
                throw TraverseEmbedderError.incompatibleBundle("runtime/runtime.wasm has an invalid signature for \(name)")
            }
        }

        let versionResults: [Value]
        do {
            versionResults = try instance.exports[function: "traverse_bridge_abi_version"]!()
        } catch {
            throw TraverseEmbedderError.incompatibleBundle("bridge_version_mismatch")
        }
        guard versionResults.count == 1,
              case .i32(let version) = versionResults[0],
              version == UInt32(Self.abiVersion) else {
            throw TraverseEmbedderError.incompatibleBundle("bridge_version_mismatch")
        }

        self.store = store
        self.instance = instance
        self.runtimeURL = runtimeURL
        self.runtimeWasmDigest = actualDigest
    }

    private static func normalizedDigest(_ digest: String) -> String {
        let normalized = digest.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        return normalized.hasPrefix("sha256:") ? normalized : "sha256:" + normalized
    }
}
