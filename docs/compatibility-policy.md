# Compatibility Policy

This document defines compatibility expectations for versioned Traverse artifacts.

## Governed Versioned Surfaces

The following surfaces are versioned and compatibility-sensitive:

- feature specs
- capability contracts
- event contracts
- runtime surface
- MCP surface
- workflow definitions where versioned behavior is exposed

## General Rules

- Backward-compatible changes should use minor or patch version increments as appropriate.
- Breaking changes must use major version increments.
- Reusing the same identity and version with changed meaning or structure is forbidden.
- Compatibility expectations must be explicit in the artifact or related documentation.

## Capability Contracts

Backward-compatible examples:

- additive metadata that does not change required behavior
- new optional fields
- clarifications that do not alter contract meaning

Breaking examples:

- removing required fields
- changing input or output meaning incompatibly
- changing required events, permissions, or constraints incompatibly

## Event Contracts

Backward-compatible examples:

- adding optional metadata
- documentation-only clarifications

Breaking examples:

- incompatible schema changes
- changed semantic meaning
- incompatible publication or subscription rules

## Runtime and MCP Surface

Backward-compatible examples:

- additive endpoints, commands, or fields
- optional trace/evidence enrichments

Breaking examples:

- removed or renamed required fields
- incompatible invocation behavior
- changed failure semantics without version change

The boundary between governed core runtime responsibilities and optional adapters is documented in:

- `docs/adapter-boundaries.md`

The release-facing downstream compatibility statement for the current `youaskm3` baseline is:

- `docs/v0.3.0-public-surface-compatibility.md`

## Specs

Approved specs are immutable once they govern implementation.

If meaning changes materially, create a new version or successor spec rather than mutating the approved one in place.

## Validation Expectations

Changes to versioned surfaces should be validated by:

- schema or structure validation
- compatibility checks
- tests covering old and new expected behavior where relevant
- CI merge gates
