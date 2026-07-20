// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "TraverseEmbedder",
    platforms: [.iOS(.v17), .macOS(.v14)],
    products: [.library(name: "TraverseEmbedder", targets: ["TraverseEmbedder"])],
    dependencies: [
        .package(url: "https://github.com/swiftwasm/WasmKit.git", exact: "0.2.2"),
        // WasmKit 0.2.2 declares `from: 1.5.0`; newer swift-system releases
        // collide with its bundled SystemExtras layer on current Xcode.
        .package(url: "https://github.com/apple/swift-system.git", exact: "1.5.0"),
    ],
    targets: [
        .binaryTarget(
            name: "TraverseSwiftHost",
            url: "https://github.com/traverse-framework/traverse/releases/download/v0.8.2/TraverseSwiftHost.xcframework.zip",
            checksum: "904ed575c25604818695f285ffac9d5d7c4ffb9b2b818bcff9cd196815c2bf01"
        ),
        .target(
            name: "TraverseEmbedder",
            dependencies: [
                "TraverseSwiftHost",
                .product(name: "WasmKit", package: "WasmKit"),
            ]
        ),
        .testTarget(
            name: "TraverseEmbedderTests",
            dependencies: [
                "TraverseEmbedder",
                .product(name: "WAT", package: "WasmKit"),
            ]
        ),
    ]
)
