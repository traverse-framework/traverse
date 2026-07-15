# Traverse macOS Demo

This is the first native macOS demo surface for Traverse.

What it does:
- renders one approved expedition flow
- shows ordered runtime state updates
- shows the final trace summary and output panel
- keeps the runtime separate from the app process

Current implementation note:
- this repo currently checks in the native SwiftUI app source and deterministic fixture-driven rendering path
- local smoke validation is available through `bash scripts/ci/macos_demo_smoke.sh`
- a full Xcode app build requires a full Xcode installation; this machine currently only exposes Command Line Tools, so native app compilation is not part of the local green path here

Expected local run path on a machine with full Xcode:
- `open apps/macos-demo/Package.swift`
- run the `TraverseMacOSDemoApp` target

Fixture source:
- `examples/fixtures/expedition-runtime-session.json`
