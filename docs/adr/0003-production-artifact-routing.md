# ADR-0003: Route Production Artifacts Through a Runtime-Owned Router

- Status: Proposed
- Date: 2026-07-12

## Context

The production server currently relies on a demonstration executor while WASM
and native execution boundaries are disconnected.

## Decision

Use a runtime-owned `ArtifactRouter` implementing the local execution boundary.
It routes WASM artifacts to `WasmExecutor`, native artifacts to explicitly
host-registered handlers, and supplies stable failure classifications. Production
serve uses this router; the example executor is explicit-only.

## Consequences

Registered capabilities execute through one portable policy boundary. Arbitrary
native paths and host-specific command execution remain prohibited.

## Alternatives Considered

- Host-specific routing.
- Making only `WasmExecutor` implement the local executor boundary.
- Retaining the demonstration executor as default.
