# ADR-0002: Materialize Public Registrations from Verified Contract Artifacts

- Status: Accepted
- Date: 2026-07-12

## Context

Public registry entries need enough governed material to reconstruct a valid
local registration without bypassing contract validation.

## Decision

Publish separate immutable contract URL/digest fields beside artifact metadata.
Consumers verify and cache both artifacts in a shared content-addressed cache,
register atomically, reject local public scope, and allow private shadows with
machine-readable evidence.

## Consequences

Registration performs two verified artifact reads but remains contract-first,
offline-reusable after caching, and deterministic.

## Alternatives Considered

- Embed whole contracts in the index.
- Infer contracts from component bundles.
- Permit pending or unverified registration.
