# iPhone Test Guide: Traverse Swift Host Boundary

Use this guide after building the branch's XCFramework on a Mac with Xcode and
an Apple signing identity. It records the final physical-device evidence for
Traverse #769.

## 1. Build the framework

From the repository root, run:

```bash
bash scripts/build_swift_host_xcframework.sh
```

The result is `target/apple/TraverseSwiftHost.xcframework`.

## 2. Create a minimal iOS app in Xcode

Create an iOS App named `TraverseSwiftHostProof` using Swift. Set your Apple
development team and a unique bundle identifier. Add the generated
`TraverseSwiftHost.xcframework` to **Frameworks, Libraries, and Embedded
Content** as **Do Not Embed** (it is static).

## 3. Run the fixtures

Replace the app's launch code with:

```swift
import SwiftUI
import TraverseSwiftHost

@main
struct TraverseSwiftHostProofApp: App {
    init() {
        precondition(traverse_swift_host_abi_version() == 2)
        precondition(String(cString: traverse_swift_host_status_message(0)) == "ok")
    }

    var body: some Scene { WindowGroup { Text("Traverse Swift host proof passed") } }
}
```

## 4. Run on a physical iPhone or iPad

Connect the device, trust this Mac if prompted, choose it as Xcode's run
destination, then Run. In Xcode's console, record that the application stays
running and displays `Traverse Swift host proof passed`.

## Expected evidence

- The app launches, the two `precondition` checks pass (ABI version and
  status-message mapping), and it remains responsive with no crash.
- This proves the `wasmi`-linked `TraverseSwiftHost.xcframework` slice
  actually links, loads, and runs on real hardware — CI's `macos-latest`
  runner and the simulator cannot substitute for this.

This runbook does not exercise `wasmi`'s memory-growth or fuel-exhaustion
enforcement directly; those are covered by `traverse-swift-host`'s own Rust
test suite (`cargo test -p traverse-swift-host`), which already runs the real
`wasmi` engine with the same limits. No memory-growth or infinite-loop Swift
fixture exists in this repo — an earlier version of this doc claimed evidence
this runbook never actually produced. If that level of on-device evidence is
needed later, it requires building dedicated WAT/WASM fixtures through
`traverse_swift_host_create`/`invoke` first.

Record device model, iOS version, Xcode version, `wasmi` version, commit SHA,
and screenshots or console output in the tracking issue before certification
is claimed.
