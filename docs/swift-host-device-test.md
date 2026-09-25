# iPhone Test Guide: Traverse Swift Host Boundary

Use this guide after building the branch's XCFramework on a Mac with Xcode and
an Apple signing identity. It records the final physical-device evidence for
Traverse #769, and for #1370's resource-limit fail-closed evidence on wasmi
2.0.0.

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

## 3. Run the ABI-boundary fixture

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

    var body: some Scene { WindowGroup { ContentView() } }
}
```

## 4. Add the resource-limit fixtures (#1370)

These prove the `wasmi`-linked boundary fails closed on real hardware for a
guest that never returns (fuel exhaustion) and a guest that tries to grow
memory past its bound, instead of hanging or crashing the app — the exact
scenarios `crates/traverse-swift-host/src/lib.rs`'s
`invoke_fails_closed_instead_of_hanging_on_an_infinite_loop` and
`invoke_fails_closed_instead_of_growing_memory_past_the_bound` tests cover in
CI. This C boundary has no WAT parser, so the WAT source in that file's
`INFINITE_LOOP_FIXTURE` and `MEMORY_GROWTH_FIXTURE` constants must be
precompiled to `.wasm` and bundled as app resources.

Extract and compile both (`wat2wasm` from WABT, `brew install wabt`):

```bash
python3 - <<'PY'
import re
src = open("crates/traverse-swift-host/src/lib.rs").read()
for name in ("INFINITE_LOOP_FIXTURE", "MEMORY_GROWTH_FIXTURE"):
    wat = re.search(rf'const {name}: &str = r#"(.*?)"#;', src, re.DOTALL).group(1)
    open(f"/tmp/{name}.wat", "w").write(wat)
PY
wat2wasm /tmp/INFINITE_LOOP_FIXTURE.wat -o infinite_loop.wasm
wat2wasm /tmp/MEMORY_GROWTH_FIXTURE.wat -o memory_growth.wasm
```

Add both `.wasm` files to the Xcode project as bundled resources (drag into
the project, "Copy items if needed", target membership checked — or, using
`xcodegen`, a `sources` entry with `buildPhase: resources`).

Add `ResourceLimitFixtures.swift`:

```swift
import CryptoKit
import Foundation
import TraverseSwiftHost

enum ResourceLimitFixtureResult {
    case passed(String)
    case failed(String)
}

enum ResourceLimitFixtures {
    private static let internalErrorStatus: Int32 = -5

    static func runAll() -> [(name: String, result: ResourceLimitFixtureResult)] {
        [
            ("Non-termination (fuel exhaustion)", run(resource: "infinite_loop")),
            ("Memory growth past bound", run(resource: "memory_growth")),
        ]
    }

    private static func run(resource: String) -> ResourceLimitFixtureResult {
        guard let url = Bundle.main.url(forResource: resource, withExtension: "wasm"),
            let runtime = try? Data(contentsOf: url)
        else {
            return .failed("fixture '\(resource).wasm' missing from app bundle")
        }

        let digestHex = SHA256.hash(data: runtime).map { String(format: "%02x", $0) }.joined()
        let expectedDigest = Array("sha256:\(digestHex)".utf8)

        var limits = traverse_swift_host_limits(
            maximum_artifact_bytes: 1024 * 1024,
            maximum_memory_bytes: 2 * 1024 * 1024,
            fuel_per_invocation: 10_000,
            maximum_input_bytes: 1024,
            maximum_output_bytes: 1024,
            maximum_queued_events: 8
        )

        var handle: UInt64 = 0
        let createStatus = runtime.withUnsafeBytes { runtimeBuf -> Int32 in
            expectedDigest.withUnsafeBufferPointer { digestBuf in
                traverse_swift_host_create(
                    runtimeBuf.bindMemory(to: UInt8.self).baseAddress,
                    runtimeBuf.count,
                    digestBuf.baseAddress,
                    digestBuf.count,
                    &limits,
                    &handle
                )
            }
        }
        guard createStatus == 0 else {
            return .failed("create returned \(createStatus), expected 0 (OK)")
        }
        defer { _ = traverse_swift_host_destroy(handle) }

        let operation = Array("submit".utf8)
        let input = Array("{}".utf8)
        var output = [UInt8](repeating: 0, count: 512)
        var required: Int = 0
        // This call is where a non-terminating or memory-growth-abusing
        // guest would hang or crash the app if the host's fuel/memory bounds
        // were not enforced. Reaching the assertion below at all — on real
        // hardware, not CI's macos-latest runner — is itself part of the
        // evidence.
        let invokeStatus = operation.withUnsafeBufferPointer { opBuf in
            input.withUnsafeBufferPointer { inBuf in
                output.withUnsafeMutableBufferPointer { outBuf in
                    traverse_swift_host_invoke(
                        handle,
                        opBuf.baseAddress,
                        opBuf.count,
                        inBuf.baseAddress,
                        inBuf.count,
                        outBuf.baseAddress,
                        outBuf.count,
                        &required
                    )
                }
            }
        }

        guard invokeStatus == internalErrorStatus else {
            return .failed(
                "invoke returned \(invokeStatus), expected \(internalErrorStatus) (INTERNAL_ERROR/bridge_trap)"
            )
        }
        return .passed("host failed closed as expected (status \(invokeStatus))")
    }
}
```

And a `ContentView` that runs both off the main actor and displays the
result, so a genuine hang shows up as the screen never updating rather than
freezing the app before it can render:

```swift
import SwiftUI

struct ContentView: View {
    @State private var results: [(name: String, result: ResourceLimitFixtureResult)] = []

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Traverse Swift host proof passed").font(.headline)
            ForEach(results, id: \.name) { entry in
                Text("\(entry.name): \(String(describing: entry.result))")
            }
        }
        .padding()
        .task {
            results = await Task.detached(priority: .userInitiated) {
                ResourceLimitFixtures.runAll()
            }.value
        }
    }
}
```

## 5. Run on a physical iPhone or iPad

Connect the device, trust this Mac if prompted, choose it as Xcode's run
destination, then Run. Record that the application stays running, displays
`Traverse Swift host proof passed`, and shows both resource-limit fixtures as
passed.

## Expected evidence

- The app launches, the two `precondition` checks pass (ABI version and
  status-message mapping), and it remains responsive with no crash.
- Both resource-limit fixtures report `passed` (status `-5`,
  `INTERNAL_ERROR`/`bridge_trap`): the non-terminating guest is stopped by
  fuel exhaustion, and the memory-growth guest is stopped by the store
  memory limiter, in both cases without hanging or crashing the app.
- This proves the `wasmi`-linked `TraverseSwiftHost.xcframework` slice
  actually links, loads, runs, and fails closed under resource abuse on real
  hardware — CI's `macos-latest` runner and the simulator cannot substitute
  for this (though a simulator run is a useful pre-check: it reproduces the
  same pass/fail outcome and catches build or wiring mistakes before using a
  real device).

The exact same fixtures were verified via `cargo test -p traverse-swift-host`
against a byte-identical `wat2wasm`-compiled artifact before being ported
here — see `crates/traverse-swift-host/src/lib.rs`'s
`invoke_fails_closed_instead_of_hanging_on_an_infinite_loop` and
`invoke_fails_closed_instead_of_growing_memory_past_the_bound` tests.

Record device model, iOS version, Xcode version, `wasmi` version, commit SHA,
and screenshots or console output in the tracking issue before certification
is claimed.
