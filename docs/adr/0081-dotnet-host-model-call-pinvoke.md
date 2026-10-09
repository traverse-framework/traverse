# ADR-0081: An Audited .NET P/Invoke Shim for Spec 138 Exact-Ref Model Execution

- Status: Accepted
- Date: 2026-10-09
- Governing specs: `138-governed-exact-model-execution` (0.13.0)
- Related issues: `#1602`, `#1642`, `#1643`
- Related: `docs/decision-log.md` Decision 111; follows the one-framed-call
  design of ADR-0078 and ADR-0079
- Owner: Traverse maintainers

## Context

The .NET `TraverseEmbedder` (net8.0, Windows/WinUI scope) was the only
embedder that could not run signed Spec 138 model packages. Decision 111 runs
them on `wasmi` inside Rust, behind P/Invoke. It reuses the framed protocol
that the safe, shared `traverse-model-host-frame` crate already serves to the
Swift and Android shims. .NET 8 has no built-in Ed25519, so a pure C# path
would also have needed a new crypto dependency.

## Decision

1. **A new crate, `crates/traverse-dotnet-host`** (`cdylib` + `rlib`), is the
   only .NET unsafe boundary. It has a crate-level `#![allow(unsafe_code)]`,
   enforced by `scripts/ci/scoped_unsafe_boundary_check.sh` as with the
   Swift, Android, and `runtime.wasm` boundaries.
2. **Exactly two exported C functions:**
   - `traverse_dotnet_host_model_call(u64 handle, const u8* request,
     usize request_length, usize* response_length_out) -> u8*` returns a
     library-owned response frame;
   - `traverse_dotnet_host_free(u8* response, usize length)` releases it.

   The library allocates the response, so the caller never has to guess a
   buffer size or retry, unlike ADR-0078's caller-supplied buffer. Null
   pointers are checked: a null request yields an envelope-error frame, and a
   null length pointer returns null.
3. **All logic is in the safe `respond(handle, request)`,** which forwards to
   `traverse_model_host_frame::model_call(&DOTNET, ...)` inside
   `catch_unwind`, so no panic unwinds across the FFI boundary. The `DOTNET`
   profile stamps `dotnet-exact-model-host` / `dotnet`.
4. **Envelope failures are frames, not error codes.** An unknown handle, a
   malformed frame, a null request, or a panic returns
   `{"ok":false,"error":{code,reason:null,message}}`. Model failures keep
   Spec 138's stable codes, reasons, and rights `detail`. C# sees one response
   shape.
5. **C# binds through `LibraryImport`** (source-generated marshalling) in
   `ExactModelNative`. It copies the response into a managed array, then
   frees the native buffer in `finally`. .NET's default probing loads
   `traverse_dotnet_host` from the app directory or the NuGet package's
   `runtimes/<rid>/native/`.
6. **Loading fails closed.** If the library or an entry point cannot load
   (`DllNotFoundException`, `EntryPointNotFoundException`,
   `BadImageFormatException`), every model call raises `model_unavailable` /
   `engine_unavailable` (FR-047). There is no fallback engine, and the rest of
   the embedder keeps working.
7. **Engine:** `wasmi` with SIMD and auto-dispatch, in fuel slices, with the
   host ceilings supplied at `create`. Cancelling `ExecuteAsync`'s
   `CancellationToken` sends the frame's `cancel` op for that execution only.

## Consequences

- The repository has a fourth crate-level unsafe opt-out, scoped to two C
  functions.
- .NET model tests need a Rust toolchain. The test project builds the library
  for the current machine through `scripts/build_dotnet_host_native.sh`.
- Shipped native assets for `win-x64`, `win-arm64`, and `linux-x64`, and the
  Windows build-and-test job, land with `#1643`.

## Alternatives Considered

- **A caller-supplied output buffer (ADR-0078 style):** rejected for .NET. It
  needs a size-probe retry, and freeing a library-owned buffer is simpler and
  safer from P/Invoke.
- **One exported function per operation:** rejected. It widens the surface
  and diverges from the shared frame protocol.
- **Reusing the Swift C-ABI crate as a `cdylib`:** rejected (Decision 111). It
  mixes Apple and .NET release concerns.

## Approval

Approved by Enrico in the Decision 111 `/brainstorm` (2026-10-09): "new
`traverse-dotnet-host` crate, one framed call".
