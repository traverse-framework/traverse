# Feature Specification: Sigstore Bundle Verification

**Spec ID**: 065
**Status**: Draft
**Created**: 2026-07-12
**Input**: User-approved blocker-resolution decisions for issue #589.

## Context

A `verified://` string prefix is not provenance evidence. Governed artifacts
need real, deterministic Sigstore verification without making execution depend
on live Rekor or Fulcio availability.

## Requirements

- **FR-001**: Traverse MUST expose a narrow artifact-verifier interface with a
  production Sigstore implementation and injectable test implementation.
- **FR-002**: Production verification MUST validate a self-contained Sigstore
  bundle, artifact digest, Fulcio certificate chain, and Rekor inclusion
  evidence using the Rust `sigstore` client library or a reviewed equivalent.
- **FR-003**: Verification MUST use a pinned trust root and an explicit
  publisher policy that validates accepted issuer and subject identities.
- **FR-004**: Execution MUST verify bundles offline; it MUST NOT require a live
  Rekor or Fulcio request on the execution path.
- **FR-005**: Missing bundle, invalid digest, invalid chain, invalid inclusion
  proof, trust-root mismatch, and publisher-policy mismatch MUST fail closed
  with stable verification evidence.
- **FR-006**: The `verified://` placeholder MUST be removed or rejected; it
  MUST NOT be accepted as cryptographic verification.
- **FR-007**: The supported Ed25519 path remains available where existing
  policy permits it.

## Out of Scope

- Interactive OIDC signing flows.
- Operating a private Fulcio or Rekor service.
- Automatic trust-root updates without an operator-approved update path.

## Verification

- Tests MUST verify valid and invalid recorded bundles without network access.
- Tests MUST cover digest, chain, inclusion-proof, issuer, and subject-policy
  failures and prove no execution occurs after failure.
