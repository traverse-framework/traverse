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
            url: "https://github.com/traverse-framework/traverse/releases/download/swift-host-v1.1.0/TraverseSwiftHost.xcframework.zip",
            checksum: "1c875fba1411b9530321162abd2a43b5120457377f8f1f981fc8e7444b6d6b14"
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
