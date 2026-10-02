# ADR-0079: An Audited Android JNI Shim for Spec 138 Exact-Ref Model Execution

- Status: Accepted
- Date: 2026-10-01
- Governing specs: `138-governed-exact-model-execution` (0.10.0)
- Related issues: `#1580`, `#1611`
- Related: `docs/decision-log.md` Decision 108; follows ADR-0078's
  one-framed-call design
- Owner: Traverse maintainers

## Context

The Kotlin `TraverseEmbedder` (Android) runs `runtime.wasm` on Chicory, a
pure-JVM interpreter without SIMD on Android, and could not run signed
Spec 138 model packages. Decision 108 runs them on `wasmi` inside Rust, behind
one JNI call, reusing the framed protocol that the Swift shim already speaks.
That protocol now lives in the safe, shared `traverse-model-host-frame` crate.

## Decision

1. **A new crate, `crates/traverse-android-host`** (`cdylib` + `rlib`), is the
   only Android unsafe boundary. It has a crate-level
   `#![allow(unsafe_code)]`, enforced by
   `scripts/ci/scoped_unsafe_boundary_check.sh` as with the Swift and
   `runtime.wasm` boundaries.
2. **Exactly one exported native method:**
   `Java_dev_traverse_embedder_ExactModelNative_modelCall(EnvUnowned, JClass,
   jlong handle, byte[] request) -> byte[]`. The only unsafe syntax is
   `#[unsafe(no_mangle)]`.
   - Byte conversion goes through the `jni` crate (0.22), so there are no raw
     `JNIEnv` vtable calls.
   - `EnvUnowned::with_env` catches panics, so nothing unwinds across the FFI
     boundary.
   - A JNI failure becomes a Java `RuntimeException`.
3. **All logic is in the safe `respond(handle, request)`,** which forwards to
   `traverse_model_host_frame::model_call(&ANDROID, ...)`. It uses the same
   frame format and operations as ADR-0078.
4. **Envelope failures are frames, not exceptions.** An unknown handle, a
   negative handle, or a malformed frame returns
   `{"ok":false,"error":{code,reason:null,message}}`. Model failures keep
   Spec 138's stable codes, reasons, and rights `detail`. Kotlin sees one
   response shape.
5. **Loading fails closed.** `ExactModelNative` loads `traverse_android_host`,
   or an explicit path for host-JVM tests. If loading fails, every model call
   raises `model_unavailable` / `engine_unavailable`; there is no fallback.
6. **Engine:** `wasmi` with SIMD and auto-dispatch, in fuel slices, with host
   ceilings supplied at `create`. wasmtime is never linked.

## Consequences

- The repository has a third crate-level unsafe opt-out, scoped to one JNI
  method.
- Kotlin unit tests need a Rust toolchain: `testDebugUnitTest` builds the
  host-JVM library through `scripts/build_android_host_jvm.sh`.
- Real-device and emulator loading of the AAR's `.so` files is proven
  separately (`#1611`).

## Alternatives Considered

- **Raw `extern "system"` without the `jni` crate:** rejected, because it means
  hand-written `JNIEnv` vtable calls and a larger audited unsafe surface.
- **One JNI method per operation:** rejected. It widens the surface and
  diverges from the shared frame protocol.
- **Throwing Java exceptions for envelope errors:** rejected. It gives two
  failure channels, where frames give Kotlin one shape.

## Approval

Approved by Enrico in the Decision 108 `/brainstorm` (2026-10-01): "one framed
`byte[]` JNI call, `jni` crate".
