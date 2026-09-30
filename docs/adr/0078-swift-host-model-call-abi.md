# ADR-0078: A Sixth Audited Swift-Host Symbol for Spec 138 Exact-Ref Model Execution

- Status: Accepted
- Date: 2026-09-29
- Governing specs: `138-governed-exact-model-execution` (0.6.0),
  `076-production-swift-wasmi-cabi` (1.2.0)
- Related issues: `#1579`, `#1582`
- Related: `docs/decision-log.md` Decision 104; amends ADR-0015
- Owner: Traverse maintainers

## Context

The Swift `TraverseEmbedder` (iOS/macOS) could not run signed Spec 138 model
packages: only the Rust native host (wasmtime, a JIT that iOS forbids) and the
web host implemented them. ADR-0015 limits the Swift-to-`wasmi` C ABI to five
audited symbols, all serving the `runtime.wasm` bridge. Decision 104 chose to
run the model guest on `wasmi` inside the Rust `traverse-swift-host`, reusing
`traverse-runtime`'s audited verification, behind **one** additional symbol.

## Decision

1. Add exactly one symbol, `traverse_swift_host_model_call(handle, request,
   request_length, output_buffer, output_capacity, output_length_out) -> i32`.
   Its pointer handling (non-null checks, `from_raw_parts`, the shared
   `output` helper) stays in the audited `lib.rs`. All model logic lives in
   `src/model_host.rs` under `#![forbid(unsafe_code)]`.
2. **Framing:** request and response are
   `[u32 LE header_len][JSON header][payload]`. The header's `segments` map
   names to `[offset, length]`, so a large model package crosses once, without
   base64. This is the same binary-frame approach Spec 076 FR-005 already
   allows for `traverse_init`.
3. **Operations:**
   - `create`: handle `0`; returns a model handle;
   - `register`, `stage_input`, `execute`, `read_output`, `rights`;
   - `cancel`, `drop_ref`, `destroy`.
4. **Statuses:** envelope failures return `INVALID_INPUT` / `INVALID_HANDLE`.
   Model failures return `OK` with `{"ok":false,"error":{code,reason,message}}`,
   preserving Spec 138's stable codes and reasons. `BUFFER_TOO_SMALL` reports
   the exact retry length. `execute` responses are header-only and outputs are
   read through the idempotent `read_output`, so a retry never re-runs an
   inference.
5. **Concurrency:** model hosts live in a registry of `Arc` states, separate
   from the ADR-0015 `Box<Host>` handles. `execute` holds the connector lock
   for the inference, while `cancel` only flips a shared atomic, and only when
   its `execution_id` matches the running execution. The `wasmi` executor
   observes the flag between fuel slices, so cancellation interrupts mid-run
   without aliasing the running host, and a late cancel can't hit the next
   execution.
6. **Engine:** model hosts always use `ModelEngine::Wasmi`, with host ceilings
   supplied at `create`. `traverse-runtime` is linked with default features
   off and only `wasmi-executor` on, so wasmtime never enters the xcframework.
7. `scripts/ci/scoped_unsafe_boundary_check.sh` requires exactly six
   `#[unsafe(no_mangle)]` symbols, including `traverse_swift_host_model_call`.

## Consequences

- The audited Swift C ABI grows from five to six symbols. Adding a new model
  operation is an envelope change, not an ABI change.
- The xcframework grows by `traverse-runtime`'s verification path
  (`ed25519-dalek`, `sha2`, `serde`), but not wasmtime.
- The Swift API (`ExactModelHost`) ships in a follow-up PR against an
  xcframework rebuilt from this change. SwiftPM consumes the released binary,
  so new Swift code can't be CI-tested before that binary exists.

## Alternatives Considered

- **One typed symbol per operation:** rejected, because it roughly doubles
  the audited unsafe surface.
- **A separate model-host handle type with its own create/destroy symbols:**
  rejected as more ABI surface. This design gets isolation from the registry
  instead.
- **Model execution inside `runtime.wasm`:** rejected in Decision 104, because
  double interpretation plus a 32 MiB ceiling is unsuitable for audio models.

## Approval

Approved by Enrico in the Decision 104 `/brainstorm` (2026-09-29): "one
JSON-envelope model call" with binary framing.
