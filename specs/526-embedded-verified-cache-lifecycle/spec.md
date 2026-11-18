# Feature Specification: Embedded Verified-Cache Lifecycle

**Feature Branch**: `codex/issue-846-verified-cache-lifecycle`
**Created**: 2026-07-28
**Status**: Approved (2026-11-18)
**Version**: 1.0.0
**Approval**: Owner-approved 2026-11-18 (#1911). The governed-files list was corrected at approval: the registry crate lives in `traverse-framework/registry` (Spec 051).
**Input**: Issue #846, Decision 38, `524-production-app-readiness`, and Spec 080.

## Purpose

Define the host-owned lifecycle of verified cache generations for an embedded
Traverse application. A host prepares a candidate beside the active generation,
activates it atomically, can explicitly roll back, and initializes or executes
only from the active verified generation while offline.

## Capability Boundary

The cache lifecycle capability validates and transitions host-supplied
generation metadata. The host owns network access, bytes, storage paths,
encryption, retention, and release selection. Registry lifecycle publication
and tier policy are inputs; runtime behavior is only permitted to consume an
already active verified generation.

## Functional Requirements

- **FR-001**: A `VerifiedCacheGeneration` MUST contain a generation id, exact
  immutable lock digest, verified entry manifests, preparation timestamp,
  source/index/provenance digests, and a state of `candidate`, `active`,
  `superseded`, or `rejected`. Entry manifests contain no cache paths,
  credentials, or artifact bytes.
- **FR-002**: `prepare` MAY use only a network-capable source explicitly
  supplied by the host. It MUST verify every selected artifact and write a
  complete candidate atomically before returning success. A failed candidate
  MUST be rejected and leave the active generation unchanged.
- **FR-003**: `activate(candidate_id)` MUST atomically make one complete
  candidate active and retain the immediately prior active generation as
  superseded and runnable for explicit rollback. Activation MUST fail with
  `registry_generation_not_verified` or `registry_generation_transition_invalid`
  without changing the active generation.
- **FR-004**: `rollback(generation_id)` MUST be host-directed and atomically
  reactivate a retained verified generation. It MUST never select a generation
  implicitly from version order, timestamps, or network state.
- **FR-005**: `init`, `submit`, `subscribe`, and execution MUST accept only the
  active verified generation via a host cache reader. They MUST make no network
  request, synthesize no cache root, and never fall back to CLI serving or
  remote registry resolution. A missing/incomplete active entry fails with
  `registry_cache_entry_missing`.
- **FR-006**: Each generation MUST retain stale-provenance facts: preparation
  time, source release/index digest, known lifecycle-state digest, and a host
  supplied freshness policy result. Staleness is evidence, not an implicit
  network refresh; an enforced local security-yank decision fails closed.
- **FR-007**: Locally known normal yanks reject new preparation but do not
  destroy an existing generation. Locally known security yanks must be
  evaluated before initialization/execution against the lock entry's policy;
  enforcement returns `registry_security_yank_enforced` offline.
- **FR-008**: Stable secret-free errors are `registry_generation_not_found`,
  `registry_generation_not_verified`, `registry_generation_transition_invalid`,
  `registry_prepare_failed`, `registry_cache_entry_missing`,
  `registry_artifact_digest_mismatch`, and `registry_security_yank_enforced`.

## Acceptance Scenarios

1. Given a complete candidate and active prior generation, when activation
   succeeds, then the candidate is active, the prior generation is retained,
   and a restart consumes only the new active manifest.
2. Given an injected entry-write or digest-verification failure, when prepare
   runs, then it returns a stable error, marks no candidate usable, and leaves
   the active generation unchanged.
3. Given an active verified generation and disconnected host, when init and
   execution run, then no registry/index/artifact network request is made and
   evidence includes stale-provenance facts.
4. Given a retained verified prior generation, when the host requests rollback,
   then it becomes active atomically. Given a missing or rejected generation,
   rollback fails without changing the active generation.
5. Given a locally known security yank past its deadline, when a Certified lock
   is initialized, then it fails offline with `registry_security_yank_enforced`.

## Governed Files and Conformance

Future implementation scope is limited to
`crates/traverse-registry/src/embedded_cache_lifecycle.rs` and
`crates/traverse-registry/tests/embedded_cache_lifecycle_conformance.rs` in
`traverse-framework/registry` (the registry crate moved there under Spec 051),
`crates/traverse-runtime/src/embedded_cache.rs`,
`crates/traverse-runtime/tests/offline_cache_execution.rs`, host package
adapters, and normative fixtures in this directory. The independent suite
must cover preparation atomicity, activation, rollback, restart, offline
network denial, stale evidence, and security-yank enforcement.

## Compatibility and Out of Scope

This is additive to Spec 080 and preserves existing development/CI registry
sync flows. Any public host API addition must be versioned and retain the
existing `registry_ref` identity semantics. Cache encryption, retention,
backup/restore, registry hosting, publisher admission, and automatic update
policy are out of scope. This draft must not be added to
`approved-specs.json` without explicit maintainer approval.
