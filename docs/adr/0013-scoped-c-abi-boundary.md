# ADR-0013: Retain an Audited Scoped C-ABI Boundary

- Status: Accepted
- Date: 2026-07-19
- Governing spec: `001-foundation-v0-1`
- Owner: Traverse maintainers
- Review by: 2026-10-19

## Context

The Apple `wasmi` feasibility proof in #769 exposes three C-compatible
functions from `traverse-swift-host` so Swift can load the static library. Rust
requires the `unsafe(no_mangle)` attribute for those symbols. A workspace-wide
`unsafe_code = "forbid"` policy cannot accommodate that syntax; weakening the
policy globally would allow a crate to introduce unsafe code without a
dedicated review.

## Decision

Retain `unsafe_code = "deny"` at the workspace level and permit exactly one
crate-level opt-out: `crates/traverse-swift-host/src/lib.rs`. That file may use
only `#[unsafe(no_mangle)]` for the three reviewed C-ABI exports:

- `traverse_swift_host_abi_version`
- `traverse_swift_host_memory_limit_fixture`
- `traverse_swift_host_fuel_limit_fixture`

`scripts/ci/scoped_unsafe_boundary_check.sh` is part of the required Rust
checks under the spec-alignment gate. It rejects a changed workspace policy,
any additional opt-out, unsafe syntax outside the boundary, or an expanded
export set.

The boundary is limited to the feasibility static library. It must not contain
unsafe blocks, raw-pointer dereferences, FFI imports, mutable globals, manual
allocation, or production runtime behavior. Any new C-ABI symbol or different
unsafe operation requires a successor ADR, an explicit owner and expiry, and
security review before merge.

This policy does not alter the runtime event boundary or event semantics.

## Consequences

- The feasibility evidence remains reproducible without silently relaxing
  safety rules for the rest of the workspace.
- The Swift production embedder remains blocked on its own certification and
  engine-selection decisions; this ADR does not certify it.
- Traverse maintainers must review this exception by 2026-10-19 and remove it
  when the feasibility host is no longer required.

## Alternatives Considered

- Restore `unsafe_code = "forbid"`: rejected because Rust cannot permit the
  required C-ABI export attributes below a `forbid` lint.
- Allow unsafe code workspace-wide: rejected because it loses the auditable,
  crate-specific boundary required by #771.
- Move the bridge to a non-Rust shim: deferred; it would add a new toolchain
  and distribution boundary without improving the current feasibility proof.
