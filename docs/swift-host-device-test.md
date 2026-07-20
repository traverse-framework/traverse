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

- The memory-growth fixture returns zero only after `wasmi` blocks growth at
  the configured 64 KiB limit.
- The fuel fixture returns zero only after `wasmi` terminates the infinite
  loop with `OutOfFuel`.
- The app remains responsive after both fixtures. This demonstrates no
  watchdog left untrusted execution alive.

Record device model, iOS version, Xcode version, `wasmi` version, commit SHA,
and screenshots or console output in #769 before certification is claimed.
