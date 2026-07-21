# ADR 0011: Constrained stdout-only agent artifact output

**Status:** Approved (2026-07-21)

## Context

`agent execute` now runs the verified WASM artifact through the production
router. The current WASM executor obtains the artifact result by reading JSON
from WASI stdout. A no-import artifact therefore cannot produce a successful
result through that entry point.

## Decision

Shipped governed agent packages use the existing
`host_api_access: exception_required` contract mechanism and cite this ADR as
their portability exception. The exception allows exactly one WASI import:
`wasi_snapshot_preview1::fd_write`, used only to write one JSON result to file
descriptor 1 (stdout).

No filesystem, environment, network, clock, random, process, or other host
capability is permitted. The runtime and package validation must reject every
other import before invocation.

## Consequences

Corrected artifacts require a minor package version increase, explicit
provenance, and CI execution through `agent execute`. This retains portable
business logic while making the output channel explicit and auditable.
