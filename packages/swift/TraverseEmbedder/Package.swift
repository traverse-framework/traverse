// swift-tools-version: 6.0
import Foundation
import PackageDescription

// Prefer a locally built xcframework (scripts/build_swift_host_xcframework.sh)
// when present so PR CI can exercise frame changes before the next
// swift-host-v* publish. Consumers without a local build keep the pinned
// release binary. SwiftPM requires the binaryTarget path to be package-relative.
let packageRoot = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
let relativeLocalHost = "../../../target/apple/TraverseSwiftHost.xcframework"
let absoluteLocalHost = packageRoot
    .appendingPathComponent(relativeLocalHost)
    .standardizedFileURL
    .path
let traverseSwiftHost: Target = FileManager.default.fileExists(atPath: absoluteLocalHost)
    ? .binaryTarget(name: "TraverseSwiftHost", path: relativeLocalHost)
    : .binaryTarget(
        name: "TraverseSwiftHost",
        url: "https://github.com/traverse-framework/traverse/releases/download/swift-host-v0.15.0-1/TraverseSwiftHost.xcframework.zip",
        checksum: "bc741716851f1f3559c2c7571002546d4b9d2bf36d81c88ef399e746c05ddd6e"
    )

let package = Package(
    name: "TraverseEmbedder",
    platforms: [.iOS(.v17), .macOS(.v14)],
    products: [.library(name: "TraverseEmbedder", targets: ["TraverseEmbedder"])],
    dependencies: [],
    targets: [
        traverseSwiftHost,
        .target(
            name: "TraverseEmbedder",
            dependencies: [
                "TraverseSwiftHost",
            ],
            // The Rust host's Core ML adapter (Decision 112, ADR-0082).
            linkerSettings: [
                .linkedFramework("CoreML"),
            ]
        ),
        // Manual macOS run for the Apple audio adapter (Spec 140, #1499). Not a product.
        .executableTarget(
            name: "TraverseAudioSmoke",
            dependencies: [
                "TraverseEmbedder",
            ]
        ),
        // A thin macOS application shell for the Spec 139/140 audio-analysis bundle (#1503).
        .executableTarget(
            name: "TraverseAudioAnalysisExample",
            dependencies: [
                "TraverseEmbedder",
            ]
        ),
        .testTarget(
            name: "TraverseEmbedderTests",
            dependencies: [
                "TraverseEmbedder",
            ],
            resources: [
                .copy("Fixtures"),
            ]
        ),
    ]
)
