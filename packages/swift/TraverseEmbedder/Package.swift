// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "TraverseEmbedder",
    platforms: [.iOS(.v17), .macOS(.v14)],
    products: [.library(name: "TraverseEmbedder", targets: ["TraverseEmbedder"])],
    targets: [
        .target(name: "TraverseEmbedder"),
        .testTarget(name: "TraverseEmbedderTests", dependencies: ["TraverseEmbedder"]),
    ]
)
