# ADR-0008: Add Compatible Lifecycle Operations to Runtime-WASM Bridge 1.1

- Status: Accepted
- Date: 2026-07-16

## Context

`embedder-api/1.0.0` requires compatible-capability start, stop, and kill.
Runtime-WASM bridge 1.0 omitted those operations, so a conforming native
adapter could only implement them outside the runtime-owned boundary.

## Decision

Publish additive bridge version 1.1.0 with three scalar core-Wasm exports:
`traverse_compatible_start`, `traverse_compatible_stop`, and
`traverse_compatible_kill`. They reuse bridge 1.0 JSON marshalling, status,
memory ownership, error, event ordering, and resource-limit rules.

The runtime generates instance identifiers, validates active instances, emits
ordered lifecycle events, and kills remaining compatible instances during
shutdown. Native hosts marshal calls and adapt events only. A package exposing
the complete embedder API requires bridge 1.1 or later within major version 1.

## Consequences

- Swift, Kotlin, and .NET retain identical runtime-owned lifecycle semantics.
- Existing bridge 1.0 artifacts remain valid for the smaller 1.0 operation set,
  but cannot certify complete `embedder-api/1.0.0` conformance.
- Release evidence must record the exact bridge version.

## Alternatives Considered

- Encode compatible lifecycle as undocumented `submit` targets: rejected
  because operation and error semantics would not be governed.
- Implement lifecycle in each platform package: rejected because it duplicates
  runtime state and violates Specs 057 and 068.
- Break bridge major version: rejected because adding exports and request types
  is backward-compatible for existing 1.x hosts.
