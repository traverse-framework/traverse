# ADR-0004: Verify Sigstore Bundles Offline with Pinned Trust Policy

- Status: Proposed
- Date: 2026-07-12

## Context

String-prefix provenance markers do not provide cryptographic assurance, and
execution must remain reliable when public transparency services are unavailable.

## Decision

Adopt a narrow verifier interface with a production Rust Sigstore verifier.
Artifacts carry self-contained bundles verified offline against pinned trust
roots and explicit issuer/subject publisher policy. Ed25519 remains supported
where existing policy permits it.

## Consequences

The runtime gains a reviewed supply-chain dependency and an explicit trust-root
update process, while avoiding live network dependency on execution paths.

## Alternatives Considered

- Shelling out to Cosign.
- Accepting any Sigstore identity.
- Deferring Sigstore verification in favor of Ed25519 only.
