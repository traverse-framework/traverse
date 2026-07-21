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
            checksum: "cf9c0461ef777cafb7bedb73f7402dba27f4d956b43dc8157ce56e1006071e48"
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
