#!/usr/bin/env bash
# Prepare the #1630 device-evidence app (Spec 138, Decisions 110 and 116).
#
#   bash tools/model-evidence/ios/prepare.sh <signed-package-dir> <frames-dir>
#
# <signed-package-dir> holds model.manifest.json, model.wasm, model.sig.json and
# variants/coreml/{model.mlpackage.zip,conformance.json}, signed with the
# test-only key. <frames-dir> holds the guest input frames (*.bin). Neither is
# committed: BirdNET weights are CC BY-NC-SA 4.0 and the clips are not
# redistributable. See docs/evidence/birdnet-iphone-governed-2026-10-15.md.
#
# It builds TraverseSwiftHost.xcframework from this checkout and links the app
# against a local copy of the TraverseEmbedder sources with an explicit path
# binary target. The package's automatic local-xcframework detection is not
# used: inside an app build, SwiftPM resolved the released binary instead.
set -euo pipefail

package_dir="${1:?signed package dir}"
frames_dir="${2:?frames dir}"
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "${here}/../../.." && pwd)"

bash "${repo_root}/scripts/build_swift_host_xcframework.sh"

local_pkg="${here}/build/TraverseEmbedder-local"
rm -rf "${local_pkg}"
mkdir -p "${local_pkg}/Sources"
cp -R "${repo_root}/packages/swift/TraverseEmbedder/Sources/TraverseEmbedder" "${local_pkg}/Sources/"
cp -R "${repo_root}/target/apple/TraverseSwiftHost.xcframework" "${local_pkg}/"
cat > "${local_pkg}/Package.swift" <<'SWIFT'
// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "TraverseEmbedder",
    platforms: [.iOS(.v17), .macOS(.v14)],
    products: [.library(name: "TraverseEmbedder", targets: ["TraverseEmbedder"])],
    targets: [
        .binaryTarget(name: "TraverseSwiftHost", path: "TraverseSwiftHost.xcframework"),
        .target(name: "TraverseEmbedder", dependencies: ["TraverseSwiftHost"],
                linkerSettings: [.linkedFramework("CoreML")]),
    ]
)
SWIFT

resources="${here}/Resources"
rm -rf "${resources}"
mkdir -p "${resources}/birdnet_pkg/variants/coreml" "${resources}/frames"
cp "${package_dir}/model.manifest.json" "${package_dir}/model.wasm" "${package_dir}/model.sig.json" "${resources}/birdnet_pkg/"
cp "${package_dir}/variants/coreml/model.mlpackage.zip" "${package_dir}/variants/coreml/conformance.json" "${resources}/birdnet_pkg/variants/coreml/"
cp "${frames_dir}"/*.bin "${resources}/frames/"
python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["public_key_hex"])' \
  "${repo_root}/fixtures/models/test-signing-key.json" > "${resources}/trusted_key.hex"

(cd "${here}" && xcodegen generate --quiet)
echo "Set DEVELOPMENT_TEAM and PRODUCT_BUNDLE_IDENTIFIER in project.yml (or pass them to xcodebuild), then build Release and install (README.md)."
