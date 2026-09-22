# ADR-0055: Signature Evidence Travels Through Host-Owned Artifact State

- Status: Accepted
- Governing spec: `124-materialized-artifact-signatures` (Approved, `#1208`)
- Extends: ADR-0051 and Spec 030
- Related issues: #1203; `traverse-framework/registry` #331, #333, #334, #335

## Context

Spec 030 correctly requires signatures for governed artifacts, but the
host-owned artifact-state pipeline introduced by Spec 120 only carries the
artifact location and digest. As a result, the runtime receives no real
signature for a published, governed capability even when the publisher has
produced one. Registry publishing cannot mutate immutable `contract.json`
after publication, so it publishes additive sibling signature files instead.

## Decision

Define a successor artifact-state schema (`1.1.0`) that carries the Ed25519
scheme, public-key hex, and signature hex for each executable artifact. The
existing host-run `registry materialize` command fetches, validates, and
verifies this evidence before writing state. `serve` remains offline: it
re-verifies the local binary digest and maps the already-validated metadata
into `BinaryReference.signature`, leaving final execution-time verification to
the existing Spec 030 runtime boundary.

The registry contract gains only an additive signature-reference object. The
host does not infer sibling URLs, discover keys, or accept a signature merely
because it was downloaded; each reference must be explicit and validation must
fail closed.

## Consequences

Published governed artifacts can satisfy the existing runtime trust boundary
through the documented materialize-and-serve pipeline. Artifact-state `1.0.0`
remains supported for its established behavior; `1.1.0` requires complete
signature metadata. Materialization becomes responsible for network retrieval
and cryptographic validation, while serving and runtime execution remain free
of network side effects.

## Alternatives considered

- Put the signature directly in `contract.json`: rejected because published
  registry contracts are immutable and the signature is produced after merge.
- Fetch signature evidence in `serve` or at execution: rejected because Specs
  115, 118, and 120 require host-prepared state and prohibit serve-owned fetch
  or mutation.
- Let `serve` trust state metadata without materialization-time verification:
  rejected because unverified evidence would move the security boundary without
  reliable failure evidence.
- Add Sigstore in this slice: rejected as scope expansion; the registry's
  current publishing decision is Ed25519 and the runtime already owns future
  scheme dispatch.
