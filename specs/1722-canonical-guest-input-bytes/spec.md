# Feature Specification: Canonical Guest Input Bytes

**Status**: Approved (2026-10-27)
**Canonical governing ID**: `1722-canonical-guest-input-bytes`
**Version**: 1.0.0
**Extends**: `025-wasm-executor-adapter`, `098-capability-event-host-abi`,
`1402-runtime-wasm-orchestrator-convergence`, and
`119-verified-registry-mcp-mode-a`.
**Decision evidence**: Decision 118 in Traverse #1722 (2026-10-27).

## Purpose and boundary

Define the exact bytes a capability guest reads as JSON input, so that the
same WASM given the same logical input produces the same output on every
Traverse host. Before this spec, native hosts fed the guest
`serde_json`-serialized bytes (sorted keys) while the web embedder forwarded
the caller's bytes unchanged. A capability that hashes, signs or echoes its
input therefore returned different bytes depending on the host (#1722).

This spec governs guest **input** only. A guest's output bytes are its own
and hosts MUST NOT rewrite them.

## Requirements

- **FR-001**: Every byte sequence a host delivers to a capability guest as
  JSON input MUST be the RFC 8785 JSON Canonicalization Scheme (JCS)
  serialization of that input. This covers:
  - the top-level invoke input;
  - each workflow step's input;
  - nested capability calls made through `runtime.wasm`;
  - event payloads delivered to a guest under Spec 098.
- **FR-002**: The rule applies on every host: the native runtime and CLI,
  `runtime.wasm`, `traverse-embedder-web` (browser and Node),
  `traverse-mcp`, and the Swift, Kotlin/Android and .NET hosts. A host MUST
  NOT forward caller bytes that differ from the JCS form, even when they are
  valid JSON.
- **FR-003**: A host MUST reject input that JCS cannot represent exactly,
  before any guest is invoked, with the stable error code
  `input_not_canonicalizable`. Such input is any of:
  - an integer whose magnitude exceeds 2^53 − 1;
  - an object with duplicate member names, when the host receives the input
    as bytes;
  - a string that is not valid Unicode (for example a lone surrogate).

  The error MUST NOT echo the input value.
- **FR-004**: Canonicalization MUST happen after input-schema validation and
  before the input-size limit is checked, so that the limit applies to the
  bytes the guest actually receives.
- **FR-005**: Hosts MUST NOT canonicalize, reorder or re-serialize guest
  output bytes. Outputs that flow on as a later step's input are
  canonicalized under FR-001 at that step.
- **FR-006**: The repository MUST ship a conformance fixture: a capability
  that returns the SHA-256 of its stdin. It MUST be run, in CI, on every host
  shipped in a release, against shared vectors. The vectors MUST cover
  non-sorted key order, whitespace, escaped strings, floats, integers at
  ±(2^53 − 1), and each FR-003 rejection. All hosts MUST produce identical
  digests and identical rejections.

## Acceptance scenarios

1. Given `{"b":1,"a":2}` and `{ "a" : 2, "b" : 1 }`, when the conformance
   capability runs on any host, then both return the digest of
   `{"a":2,"b":1}`, and every host returns the same digest.
2. Given the #1722 `core.authorize@1.2.0` demo input, when it runs on the
   browser, Node and `traverse-mcp`, then the full output bytes, including
   `policy_hash`, are identical.
3. Given an input containing `9007199254740993`, when any host receives it,
   then the call fails with `input_not_canonicalizable` and no guest runs.
4. Given a two-step workflow, when step 1 emits non-canonical JSON, then the
   caller sees step 1's bytes unchanged, and step 2 receives the JCS form.

## Compatibility and non-goals

- This is a behaviour change, shipped always-on in the next minor release
  (v0.16.0) with no opt-out flag. Release notes and the upgrade guide MUST
  call out:
  - that browser and Node guest input bytes change;
  - the new `input_not_canonicalizable` rejection, which native callers
    passing large integers can hit.
- Guest ABIs, contract schemas and model frame protocols (Spec 138) are
  unchanged; model input frames are binary and out of scope.
- Changing `core.authorize`'s hash algorithm is out of scope (tracked in
  #1010).

## Validation

- Unit tests for the JCS serializer cover RFC 8785 Appendix B number
  vectors and the FR-003 rejections, with 100% coverage on the runtime path.
- The FR-006 conformance job is green on every shipped host.
- `bash scripts/ci/spec_alignment_check.sh` passes.
