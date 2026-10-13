# ADR-0082: An Audited objc2 Core ML Adapter for Spec 138 Accelerator Variants

- Status: Accepted
- Date: 2026-10-13
- Governing specs: `138-governed-exact-model-execution` (0.13.4),
  `076-production-swift-wasmi-cabi` (1.3.0: FR-007 and FR-009 amended)
- Related issues: `#1629` (slice 3), `#1663`, `#1630`
- Related: `docs/decision-log.md` Decisions 110, 112, 113 and 114
- Owner: Traverse maintainers

## Context

Decision 112 runs Core ML from Rust, through `objc2` bindings, so that every
Spec 138 rule (placement order, self-check, tolerance, ceilings,
cancellation, evidence) stays in the single implementation in
`traverse-runtime`. Slice 2 (`#1667`) added the `AcceleratorAdapter` trait
and tested it with a fake adapter. Slice 1 (`#1663`) produced real
`mlpackage-zip` variants. This ADR covers the adapter that runs them.

Calling Core ML needs `unsafe`: the `objc2-core-ml` methods are `unsafe`
because Objective-C signatures are unchecked, and Core ML lends multi-array
buffers to a block as raw pointers. The workspace denies `unsafe_code`, so
this needs an audited exception like the Swift, Android and .NET shims.

## Decision

1. **A new crate, `crates/traverse-coreml-adapter`.** On Apple platforms it
   provides `CoreMlAdapter`, an `AcceleratorAdapter` with the id `coreml`.
   Elsewhere only its safe helpers build, so the workspace stays
   cross-platform.
2. **`unsafe` lives in one module, `src/coreml.rs`,** under an item-level
   `#[allow(unsafe_code)]` on `mod coreml;`. Its unsafe operations are:
   - calls to `objc2-core-ml` / `objc2-foundation` methods, with arguments
     of the declared types;
   - exactly two raw buffer views, the input array Core ML lends to
     `getMutableBytesWithHandler` and the output array it lends to
     `getBytesWithHandler`, each used only inside its block;
   - one `unsafe impl Send for CoreMlAdapter`. The connector only uses the
     adapter through `&mut self` behind its lock, and Core ML model objects
     are not bound to a thread.

   The module exports and imports no symbols itself.
   `scripts/ci/scoped_unsafe_boundary_check.sh` enforces all of this.
3. **Everything else is safe Rust in `src/package.rs`**, unit tested on
   every OS:
   - unpacking the canonical zip (stored entries only, no data descriptors,
     and only relative paths with normal components, so nothing is written
     outside the cache);
   - checking the package manifest;
   - packing and unpacking strided `f32` tensors, plus `f16` / `f64`
     outputs;
   - compute-unit names.
4. **Compile on the device, cache by digest.**
   - The variant is unpacked to `<cache_dir>/<variant digest>/model.mlpackage`.
   - It is compiled with `MLModel.compileModelAtURL`, then moved beside the
     package as `model.mlmodelc`.
   - It is loaded with the host's compute units and kept in memory for the
     adapter's lifetime.
   - The self-check key already includes the OS version and compute units
     (FR-062), so an OS update re-checks the variant.
5. **Frames: the conformance vector is the contract (Decision 114).** The
   adapter accepts input frames whose dtype matches the variant's
   conformance inputs. It writes its output with the dtype of the
   vector's reference outputs, and the output dims are the Core ML output
   shape.
6. **Failing closed without aborting.**
   - Core ML reports some failures by raising an Objective-C exception,
     which Rust cannot unwind through. Each prediction runs inside
     `objc2::exception::catch`, so an exception becomes `execution_failed`.
   - Core ML's package loader also raises a C++ `std::out_of_range` when
     `Manifest.json` lacks a required field. `catch` cannot handle that,
     and the process would abort. So the adapter checks the manifest
     itself before Core ML reads it: the file-format version, every item's
     path, name, author and description (each path must name a file or
     directory in the zip), and the root identifier.
   - Error messages carry the `NSError` domain and code only. Core ML's
     localized text can include host file paths.
7. **Host configuration.**
   - The frame protocol's `create` header gains an optional `accelerators`
     object, read by the shim's `AdapterFactory`.
   - `register` gains `variant_paths`, which names segments holding the
     fetched variant files.
   - `traverse-swift-host` enables Core ML through `accelerators.coreml`:
     `compute_units` defaults to `all` (Decision 112 rule 6), and
     `cache_dir` defaults to `<temp>/traverse-coreml`.
   - Swift's `ExactModelHost` gains only these settings, plus the
     placements, the fuel requirement and placement evidence.
   - The Swift target links `CoreML.framework`.

## Consequences

- The repository has a fifth audited unsafe boundary. It is scoped to one
  module, has no exported symbols, and is checked in CI.
- Spec 076 1.3.0 amends two requirements of the Swift C-ABI host. FR-009: a
  model host runs on `wasmi` unless it enabled Core ML and the call's policy
  lists `coreml`. FR-007: unsafe code may also live in this adapter module.
  The six exported symbols do not change.
- The Core ML path is tested in `cargo test` on macOS:
  - the signed `digits-onnx-coreml-1.0.0` package runs through the connector
    on every compute-unit setting;
  - malformed or corrupt variants fail closed;
  - an ignored test, run with local weights, checks BirdNET against
    Decision 106 on the Decision 113 clip set.

  The Linux coverage gate measures the platform-neutral helpers at 100%.
- **Residual risk:** another C++ exception thrown inside Core ML for an
  input this adapter does not pre-check would still abort the process. This
  is bounded because:
  - variants are signed;
  - CI re-converts each committed variant byte for byte (FR-068);
  - the device self-check runs the whole conformance vector before any
    call is placed on Core ML.
- Swift's tests use the released `TraverseSwiftHost` xcframework. Its Core
  ML test therefore skips until the next swift-host release; locally, it
  passes against an xcframework built from source.

## Alternatives Considered

- **A Swift callback adapter:** rejected in Decision 112. It needs a
  reentrant FFI callback with unsafe code on both sides.
- **The asynchronous `compileModelAtURL:completionHandler:`:** rejected.
  The connector calls `predict` synchronously, and blocking on a completion
  handler adds a block and a channel without changing behaviour. The
  synchronous call is deprecated but supported.
- **A C++ shim to catch C++ exceptions:** rejected for now. It adds a
  second native language and boundary, while checking the manifest in Rust
  covers the known throw.
- **Zero-copy input (`initWithDataPointer`):** rejected. Core ML may keep
  the array beyond the prediction, so it must own its buffer.

## Approval

Approved under Decision 112 (`/brainstorm`, 2026-10-13), which set the
`objc2` route, the audited-boundary requirement and this ADR. Decision 114
records the frame-contract choice.
