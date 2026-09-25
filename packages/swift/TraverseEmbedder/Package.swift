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
            url: "https://github.com/traverse-framework/traverse/releases/download/swift-host-v0.13.0/TraverseSwiftHost.xcframework.zip",
            checksum: "92a1c88fefd89a18bbc269551e23db46ebe1a9ebf35729dea623cc3d16724cc3"
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
