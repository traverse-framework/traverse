# ADR-0015: Govern the Production Swift-to-wasmi C-ABI Boundary

- Status: Accepted
- Date: 2026-07-20
- Governing specs: `068-public-platform-embedder-packages`,
  `071-native-runtime-wasm-bridge`, `072-native-bridge-compatible-lifecycle`,
  and `074-swift-native-resource-control-certification`
- Owner: Traverse maintainers
- Review by: 2026-10-20

## Context

ADR-0013 deliberately limits `traverse-swift-host` to three feasibility
exports and prohibits pointer marshalling or production behavior. ADR-0014
selects `wasmi` 1.1.0 for the Apple profile. #647 now needs a small, audited
native boundary that can host the governed bridge without exposing WasmKit SPI,
ambient authority, or a second runtime protocol.

## Decision

The production C ABI exposes exactly five symbols:

1. `traverse_swift_host_abi_version`
2. `traverse_swift_host_create`
3. `traverse_swift_host_invoke`
4. `traverse_swift_host_destroy`
5. `traverse_swift_host_status_message`

`create` receives immutable runtime bytes, the expected SHA-256 digest, and
required positive limits for artifact bytes, Wasm memory, fuel per invocation,
input bytes, output/event bytes, and queued events. It verifies the digest,
core-Wasm imports, bridge 1.1 ABI and required exports before instantiation.
It returns one opaque, serialized host handle; file paths, URLs, registry
access, and ambient filesystem, network, clock, environment, or process
authority are forbidden.

`invoke` accepts only the governed bridge 1.1 allowlist: `init`, `submit`,
`next_event`, `cancel`, `compatible_start`, `compatible_stop`,
`compatible_kill`, and `shutdown`. It accepts caller-owned UTF-8 JSON input
and a caller-owned output buffer. A too-small output buffer returns the exact
required length for one retry. The host never transfers allocation ownership
to Swift. `destroy` invalidates the handle; use-after-destroy, concurrent or
re-entrant use, malformed UTF-8/JSON, invalid descriptors, traps, resource
limits, and timeouts return stable numeric status codes plus bounded UTF-8 JSON
error details.

Unsafe Rust is confined to the reviewed C entry points and their pointer
conversion helpers in `crates/traverse-swift-host/src/lib.rs`. Every pointer
and length is validated before dereference. Unsafe blocks outside that file,
unreviewed exports, mutable globals, manual cross-language allocation, and
unbounded buffers are forbidden. The CI boundary check must enumerate these
five production symbols (and separately retain the fixture-only symbols until
they are removed) before implementation merges.

Swift wraps one handle in a serialized actor/lock and exposes engine-neutral
public types. Existing WasmKit-named public types are deprecated compatibility
wrappers until the next major package version; they must not select or invoke
WasmKit in the production path.

## Certification and Distribution

This ADR authorizes implementation, not a certified release. #647 must pass
local Rust and Swift boundary tests plus bridge validation. #758 must provide
real-artifact bridge/embedder corpus results, physical iOS/macOS evidence, and
reference-app integration before a Native Embedder Baseline claim. Release
evidence records wasmi 1.1.0, `MIT/Apache-2.0`, all configured limits, the
arm64 Apple profile, the runtime digest/bridge version from Spec 075, and
conformance results.

## Alternatives Considered

- Operation-specific C functions: rejected because they duplicate the
  governed bridge ABI and expand review surface.
- Rust-allocated results: rejected because cross-language allocation ownership
  complicates the safety boundary.
- Arbitrary export invocation: rejected because it creates an ungoverned
  extension mechanism.
- Universal macOS support now: rejected pending separate x86_64 evidence.
