// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "TraverseEmbedder",
    platforms: [.iOS(.v17), .macOS(.v14)],
    products: [.library(name: "TraverseEmbedder", targets: ["TraverseEmbedder"])],
    dependencies: [],
    targets: [
        .binaryTarget(
            name: "TraverseSwiftHost",
            url: "https://github.com/traverse-framework/traverse/releases/download/swift-host-v0.13.0-2/TraverseSwiftHost.xcframework.zip",
            checksum: "d8f5c65e988eb1a895304bde09bdea6d56c69d3bd1c8194650c1805050da6395"
        ),
        .target(
            name: "TraverseEmbedder",
            dependencies: [
                "TraverseSwiftHost",
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
