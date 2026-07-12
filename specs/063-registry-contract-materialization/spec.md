# Feature Specification: Registry Contract Materialization

**Spec ID**: 063
**Status**: Draft
**Created**: 2026-07-12
**Input**: User-approved blocker-resolution decisions for issues #551 and #552.

## Context

Public registry references currently identify a binary artifact but do not make
the governed capability contract available to a consumer. Traverse must never
materialize an executable registration without first obtaining and validating
its immutable contract.

## Requirements

- **FR-001**: Every publishable public capability record MUST include a
  `contract_url` and SHA-256 `contract_digest`, in addition to the binary
  artifact URL and digest.
- **FR-002**: A registry-reference registration MUST fetch, digest-verify, and
  validate the contract before it fetches or materializes the binary artifact.
- **FR-003**: Verified public contracts and artifacts MUST be cached in the
  shared content-addressed path `.traverse/cache/sha256/<digest>`.
- **FR-004**: A cache hit MUST still verify that the cached bytes match the
  expected digest before reuse.
- **FR-005**: Registration MUST be atomic: contract resolution, artifact
  retrieval, digest verification, validation, and local registry mutation MUST
  either all succeed or leave no pending or partial workspace state.
- **FR-006**: A local bundle declaring `scope: public` MUST be rejected with a
  stable error directing callers to private registration or the governed
  publication flow.
- **FR-007**: A private registration that shadows a synced public identity MUST
  succeed and emit machine-readable shadow evidence. `PreferPrivate` lookup
  MUST resolve the private registration.
- **FR-008**: Missing sync state, missing contract, digest mismatch, invalid
  contract, unavailable artifact, and yanked-only resolution MUST each produce
  stable actionable errors.

## Out of Scope

- Cache eviction and storage quotas.
- Mirrored registries or distributed cache replication.
- Changing approved public contract versions in place.

## Verification

- Tests MUST cover cache miss and hit paths, contract and artifact digest
  mismatches, rollback on every failure phase, public-scope rejection, and
  private-shadow evidence.
- A compatibility test MUST prove existing `contract_path` registration remains
  unchanged.
