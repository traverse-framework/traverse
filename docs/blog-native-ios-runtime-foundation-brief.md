# Blog Brief: Native iOS Runtime Foundation

## Intended article

Write a technical blog post announcing that Traverse and UMA have validated a
native iOS foundation for hosting WebAssembly. The post is an engineering
milestone, not a product-launch announcement.

## Audience and tone

Address technical product builders, Swift/Rust developers, and teams evaluating
portable capability runtimes. Be concrete, calm, and evidence-led. Avoid hype.

## Core message

Traverse has proven on a physical Apple device that a Swift application can
load a Rust static library containing the `wasmi` interpreter, invoke a narrow
C-compatible bridge, enforce a host-owned WebAssembly memory limit, and stop a
non-terminating WebAssembly module through deterministic fuel exhaustion.

This makes a native iOS runtime path implementable. It does **not** mean that
the complete Traverse runtime or a production iOS SDK has shipped.

## Background

UMA and Traverse want a portable execution model:

```text
native application host
  -> digest-pinned Traverse runtime.wasm
  -> application-owned manifests
  -> independently packaged capability WASM modules
```

The native host should stay small and stable. Runtime behavior belongs in the
shared WebAssembly artifact, while capabilities remain independently built and
declared through their manifests. This avoids reimplementing runtime semantics
for every operating system.

## The iOS problem

Apple platforms make JIT-oriented WebAssembly engines a poor production
baseline because of executable-memory and entitlement constraints. The prior
Swift engine candidate, WasmKit 0.3.1, did not expose both required controls as
supported public APIs: its memory limiter was SPI and it lacked supported,
deterministic execution interruption.

Traverse therefore required a candidate with all of the following:

- interpreter execution rather than a JIT-dependent profile;
- iOS and macOS support;
- host-configurable memory limits through public APIs;
- deterministic interruption of non-terminating guest code;
- no watchdog that returns control while untrusted execution continues.

## Candidate chosen for feasibility proof

The feasibility host uses `wasmi` 1.1.0, a Rust WebAssembly interpreter. It
provides public `StoreLimitsBuilder` memory controls and fuel metering. The
host is packaged as a Rust static library and surfaced to Swift through a very
small C-compatible boundary.

The proof was tracked in Traverse issue #769. The temporary exception allowing
the narrowly scoped Rust C-ABI exports is tracked separately in issue #771 for
governance review.

## What was built

- `crates/traverse-swift-host/`: the Rust static-library feasibility host.
- `TraverseSwiftHost.xcframework`: generated slices for iPhone, Apple Silicon
  iOS simulator, and Apple Silicon macOS.
- A C header and module map that permit Swift import.
- A macOS Swift smoke program that imports the framework and calls Rust.
- Two resource-control fixtures exposed through the Swift-facing API.
- A reproducible physical-device test guide.

## Verified results

The following were verified before the device run:

1. The static library built for `aarch64-apple-ios`,
   `aarch64-apple-ios-sim`, and `aarch64-apple-darwin`.
2. Swift on macOS imported the XCFramework and called the Rust bridge.
3. A module attempting to grow memory past a host-owned 64 KiB limit was
   stopped with `GrowthOperationLimited`.
4. A non-terminating module was stopped with `OutOfFuel` after the configured
   fuel budget was exhausted.
5. The targeted Rust test suite passed three tests.

Physical iOS validation was then performed by Enrico Piovesan on 2026-07-19:

- Xcode reported no errors.
- The `TraverseSwiftHostProof` application launched on a paired physical
  iPhone/iPad.
- It displayed `Traverse Swift host proof passed`.
- The application remained responsive after both fixtures completed.

## Why this matters

This is evidence that the native host can own the safety controls rather than
trusting a capability module to self-limit. It supports UMA's separation of a
stable host binary from runtime and capability WebAssembly artifacts. It also
turns the Swift/iOS route from a speculative engine choice into an actionable
implementation path.

## Important limits: do not overclaim

Do not say any of the following:

- “Traverse is fully released on iOS.”
- “The complete Traverse runtime runs on iOS today.”
- “Production cross-platform certification is complete.”
- “Every capability can run on iOS without declared-profile checks.”

The proof does not yet execute the complete digest-pinned `runtime.wasm`, load
application manifests and capability modules end-to-end, or certify the same
artifact through Swift, Kotlin, and .NET.

## Remaining work

1. Merge the feasibility proof PR and approve a successor ADR selecting the
   engine profile.
2. Implement the production Swift embedder SDK (Traverse #647).
3. Run the real digest-pinned Traverse runtime artifact through the host.
4. Load application manifests and separately packaged capability modules.
5. Validate declared core-WASM and optional bounded-WASI capability profiles.
6. Complete cross-host conformance and reference-application workflows.
7. Resolve the scoped unsafe C-ABI policy under #771.

## Suggested structure

1. Title: “Building a Native iOS Foundation for Portable WebAssembly
   Capabilities”
2. The portability goal and UMA separation model.
3. Why iOS required a different runtime-engine decision.
4. The `wasmi` feasibility approach.
5. What was tested on the physical device.
6. Why host-owned limits are essential for untrusted modules.
7. A transparent list of what remains.
8. Closing: native iOS is now an implementation path, not a finished product.

## Source links to include

- Traverse feasibility issue: `https://github.com/traverse-framework/traverse/issues/769`
- Governance follow-up: `https://github.com/traverse-framework/traverse/issues/771`
- `wasmi` documentation: `https://docs.rs/wasmi/latest/wasmi/`
- `StoreLimitsBuilder` API:
  `https://docs.rs/wasmi/latest/wasmi/struct.StoreLimitsBuilder.html`
