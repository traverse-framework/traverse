# Feature Specification: Capability-Side WASM Host ABI for Managed State

**Status**: Approved (2026-11-17)
**Approval**: Owner-approved 2026-11-17 (#1899); co-designed via `/brainstorm`. Implemented in #1293.
**Canonical governing ID**: `1285-capability-state-host-abi`
**Version**: 1.0.0
**Extends**: `002-capability-contracts`, `014-service-type-taxonomy` (Traverse Spec ID 014 / issue 208), `RuntimeDataStore` / DataStore surface
**Input**: Issue #1285; registry decision-log entry 93; `/brainstorm` session (host storage unlock for UMA Stateful).

## Purpose

Define a WASM host-function ABI that lets a **Stateful** capability read and
write managed persistence during execution, via the existing host
`DataStore` / `RuntimeDataStore` stack. Today Stateful is declared on
contracts and placement-constrained (no Browser), but guests have no
whitelisted import to touch durable state — so the registry cannot publish
honest Stateful capabilities (UMA §5.1.2).

This mirrors `098-capability-event-host-abi` (`emit_event` for Subscribable):
synchronous, deny-by-default, call-time validation, no panic/trap on bad
guest pointers.

## Capability Boundary

Governs host-function import signatures, calling convention, synchronous
validation, key namespacing, and whitelist membership on Host ABI `1.0.0`
(`host_abi_v1.json`). Does **not** redesign `DataStore` sync/merge/crypto;
does **not** add list/query imports in v1; does **not** publish registry
capabilities (follow-on once this ABI is callable).

## Requirements

- **FR-001**: Host ABI `1.0.0` MUST whitelist three `traverse_host` imports:
  `state_get`, `state_put`, and `state_delete`. Each takes `(ptr: i32, len: i32) -> i32`
  where `ptr`/`len` address a JSON envelope in guest linear memory.
- **FR-002**: Only capabilities with `service_type == Stateful` MAY succeed
  on these calls. Any other service type MUST be rejected at call time with a
  stable negative status (before guest memory is read for put/delete; get MAY
  also reject before read).
- **FR-003**: The calling capability's contract MUST declare a non-empty
  `state_schema`. Writes MUST pass existing `RuntimeDataStore` /
  `validate_state_write` rules (exact `state_schema.properties[key]`). Missing
  schema MUST fail the call (not silently no-op).
- **FR-004**: Guest envelopes:
  - `state_put`: `{"key":"<relative>","value":<json>}`
  - `state_get` / `state_delete`: `{"key":"<relative>"}`
  Relative keys MUST NOT contain `/` or `..`. The host MUST persist under
  `{capability_id}/{relative_key}` (capability isolation).
- **FR-005**: On put, the host MUST stamp `StateRecord` metadata
  (`lamport_clock`, `writer_id`) via `RuntimeDataStore`; the guest MUST NOT
  supply clock/writer fields.
- **FR-006**: If no `DataStore` is injected into the executor for this run,
  state_* MUST return a stable `data_store_not_configured` error. The host
  MUST NOT silently fall back to an ambient in-memory store. Tests and local
  tools MAY inject an explicit in-memory adapter.
- **FR-007**: Memory bounds and a maximum envelope size (same order as
  `emit_event`, 64 KiB) MUST be enforced before deserializing. Malformed or
  out-of-bounds requests MUST return an error code — no panic, trap, or
  out-of-bounds read.
- **FR-008**: Contract validation MUST reject `service_type: stateful` when
  `state_schema` is missing/empty, and MUST continue to reject Stateful +
  Browser in `permitted_targets` (taxonomy FR-005).
- **FR-009**: `state_get` MUST distinguish not-found from errors via stable
  status codes; on success, the host writes a JSON result envelope into a
  guest-provided out-buffer **or** returns the value through a documented
  in-memory result channel consistent with other host helpers — v1 MUST
  document one approach and test it. Prefer: put response bytes at an
  `out_ptr`/`out_len` pair if the signature stays two i32s…  

  **v1 decision (brainstorm follow-through):** keep `(ptr,len)->i32` like
  `emit_event`. For `state_get`, the guest envelope MAY include
  `"out_ptr"` / `"out_max"` integers; on success the host writes
  `{"found":true,"value":...}` or `{"found":false}` into that out region and
  returns `0`. If `out_ptr`/`out_max` are absent or insufficient, return
  invalid-payload.

## Acceptance Scenarios

1. Given a Stateful capability with `state_schema.properties.cart` and an
   injected store, when it `state_put`s `{"key":"cart","value":{...}}`, then
   the host stores under `{capability_id}/cart` with stamped metadata and
   returns OK.
2. Given the same capability `state_get`s `{"key":"cart",...}`, then it
   receives the stored value (`found: true`).
3. Given a Stateless capability calls `state_put`, then the call is rejected
   regardless of payload.
4. Given Stateful but no injected store, when `state_put` is called, then the
   host returns `data_store_not_configured` (or equivalent stable code).
5. Given an oversized or out-of-bounds pointer, when any state_* is called,
   then the host returns an error without panicking or reading past guest
   memory.
6. Given Stateful without `state_schema`, when the contract is validated or
   when `state_put` runs, then the operation fails closed.

## Out of Scope

- `state_list` / prefix query (deferred).
- Cross-capability shared keys.
- Browser ephemeral stores.
- Registry Wave 2 capability publishes (blocked on this ABI landing).

## Success Criteria

- **SC-001**: `host_abi_v1.json` lists `state_get`, `state_put`, `state_delete`.
- **SC-002**: Executor tests cover scenarios 1–5 (happy put/get, wrong
  service type, no store, bounds, schema miss).
- **SC-003**: `cargo test -p traverse-runtime` passes.
- **SC-004**: Spec alignment / approved-specs registration completed when
  the owner approves this Draft.
