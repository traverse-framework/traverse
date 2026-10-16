# iPhone model-execution evidence harness (#1630)

A small iOS app that measures a signed Spec 138 package on the **governed
path**, through the public `TraverseEmbedder.ExactModelHost`, on a physical
device. It produced
[`docs/evidence/birdnet-iphone-governed-2026-10-15.md`](../../../docs/evidence/birdnet-iphone-governed-2026-10-15.md).

For each run it creates a fresh host and registers the package. The runs are:
- `wasm-cpu` with snapshot reuse, which is the reference;
- one run per Core ML compute-unit setting, with the policy
  `[coreml, wasm-cpu]`.

Each run makes one first call and then 21 timed calls. It records the
placement evidence and compares accelerated outputs with the device's own
`wasm-cpu` outputs under Decision 106 (top-5 identical, |Δ sigmoid| ≤ 5×10⁻³).
The report is printed as one `TRAVERSE_1630_EVIDENCE {json}` console line,
written to the app's Documents as `evidence-1630.json`, and shown on screen.

Nothing model-specific is committed: the weights and clips stay local.

## Reproduce

1. **Package the model** (local only). For BirdNET, use
   [`birdnet.package.json`](birdnet.package.json) with the pinned
   `BirdNET_v2.4_int8_arm.onnx`:

   ```bash
   traverse-cli model package-onnx fixtures/onnx/runner.wasm birdnet.onnx tools/model-evidence/ios/birdnet.package.json pkg
   traverse-cli model sign pkg/model.manifest.json --key test-secret.hex
   traverse-cli model conformance generate pkg --trusted-key <test public key> --model-usage non_commercial --input frames/*.bin --out vector.json
   traverse-cli model convert-coreml birdnet.onnx birdnet.mlpackage.zip
   traverse-cli model add-variant pkg --file birdnet.mlpackage.zip --conformance vector.json \
     --peak-memory-bytes 268435456 --max-abs-diff 0.005 --top-k 5 --tool traverse-coreml \
     --tool-version lock-sha256:<requirements.lock digest> --f32-dtype 3 --score-transform sigmoid
   traverse-cli model sign pkg/model.manifest.json --key test-secret.hex
   ```

   Frames are guest input frames: `u16` ABI version 1, dtype `2`, rank `2`,
   dims `[1, 144000]`, the payload length, then 144,000 little-endian `f32`
   samples at 48 kHz.
2. **Prepare the app:**

   ```bash
   bash tools/model-evidence/ios/prepare.sh pkg frames
   ```

   It builds `TraverseSwiftHost.xcframework` from this checkout, links it
   explicitly, copies the package, frames and trusted test key into
   `Resources/` (git-ignored), and generates the Xcode project.
3. **Build Release and install.** Put your team in `DEVELOPMENT_TEAM` and a
   unique `PRODUCT_BUNDLE_IDENTIFIER` in `project.yml`. Then:

   ```bash
   xcodebuild -project TraverseEvidence1630.xcodeproj -scheme TraverseEvidence1630 \
     -configuration Release -destination 'generic/platform=iOS' -derivedDataPath build \
     -allowProvisioningUpdates build
   xcrun devicectl device install app --device <device id> build/Build/Products/Release-iphoneos/TraverseEvidence1630.app
   ```

4. **Run.** Unlock the phone, then:

   ```bash
   xcrun devicectl device process launch --console --terminate-existing --device <device id> <bundle id>
   ```

   The `wasm-cpu` reference run comes first and takes several minutes on a
   phone. Keep the device unlocked; the app disables the idle timer while it
   runs.
