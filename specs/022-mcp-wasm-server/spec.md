# Feature Specification: Dedicated Traverse MCP WASM Server Model

**Feature Branch**: `022-mcp-wasm-server`  
**Created**: 2026-04-06  
**Status**: Superseded (2026-07-18 — see decision-log.md Decision 25; never approved, no implementation commits reference this spec ID; no specific direct successor identified)  
**Input**: User description: "Specify the first dedicated Traverse MCP WASM server model using the portable MCP runtime lessons from UMA Chapter 13."

## Purpose

This specification defines the first dedicated Traverse MCP WASM server model.

It narrows the broader app-facing MCP goal into one explicit contract for:

- the role of a dedicated MCP server in Traverse
- how governed Traverse capabilities are exposed through that server
- how WASM-hosted capabilities or agents participate
- what remains in Traverse runtime authority versus MCP transport concerns
- the first supported host model for this server, starting with stdio

This slice exists so Traverse can formalize a dedicated MCP WASM server package without guessing at its boundary or folding transport concerns into Traverse governance.

This slice does **not** define the implementation code for the server, the downstream app UX, or the broader production deployment story. It defines the governed model that those implementation slices must follow.

## User Scenarios and Testing

### User Story 1 - Define the Dedicated MCP WASM Server Role (Priority: P1)

As a Traverse maintainer, I want one governed model for a dedicated MCP WASM server so that the repo has a stable contract for how Traverse serves MCP behavior through a dedicated package.

**Why this priority**: The first dedicated MCP server slice is only useful if its role is explicit enough to guide implementation and review.

**Independent Test**: A reviewer can read this spec alone and explain the dedicated server’s role, what it is responsible for, and what it is not responsible for.

**Acceptance Scenarios**:

1. **Given** a dedicated Traverse MCP server exists, **When** a reviewer inspects this spec, **Then** the server’s role is described without relying on implementation guesses.
2. **Given** Traverse runtime authority is compared to MCP transport concerns, **When** this spec is reviewed, **Then** the boundary between them is explicit.

### User Story 2 - Expose Governed Capabilities Through the Server (Priority: P1)

As a downstream consumer steward, I want governed Traverse capabilities to be exposed through the dedicated MCP WASM server model so that the server can be used as a stable public substrate rather than a private internal path.

**Why this priority**: The server only matters if it can expose governed capability behavior through a reviewable public boundary.

**Independent Test**: A reviewer can identify which governed capability surfaces may be exposed through the server and which concerns stay in Traverse runtime authority.

**Acceptance Scenarios**:

1. **Given** a governed Traverse capability is made available through the server model, **When** the model is reviewed, **Then** the capability exposure path is explicit and governed.
2. **Given** an app or agent consumes the server, **When** the transport is inspected, **Then** the consumer does not need undocumented Traverse internals to understand the boundary.

### User Story 3 - Make the Model Concrete Enough to Drive Implementation and Validation (Priority: P2)

As a release steward, I want the server model to be concrete enough for one implementation ticket and one validation ticket so that the repo can move from idea to governed delivery without further guessing.

**Why this priority**: The model is only useful if it can directly support one build slice and one validation slice.

**Independent Test**: A reviewer can turn this spec into one implementation ticket and one validation ticket without adding unstated assumptions.

**Acceptance Scenarios**:

1. **Given** a team wants to implement the server, **When** this spec is reviewed, **Then** the first supported host model and authority boundaries are explicit enough to start work.
2. **Given** a team wants to validate the server, **When** this spec is reviewed, **Then** the validation evidence required for release-readiness is clear.

## Scope

In scope:

- the dedicated role of a Traverse MCP WASM server package
- the relationship between Traverse runtime authority and MCP transport concerns
- the governed capability exposure model for the server
- participation of WASM-hosted capabilities or agents through the server
- the first supported host model, starting with stdio
- implementation and validation guidance that is concrete enough for one follow-on build ticket and one follow-on validation ticket

Out of scope:

- the actual server implementation code
- downstream app UI behavior
- browser adapter transport details
- production deployment automation
- broad multi-host rollout policy beyond the first supported host model

## Edge Cases

- A downstream app asks for a host model other than stdio before the first host extension exists.
- A capability is governed but should remain entirely within Traverse runtime authority rather than being exposed through the MCP server.
- A WASM-hosted capability or agent needs transport behavior that would blur MCP concerns into Traverse governance.
- A reviewer needs to distinguish this dedicated Traverse server model from the UMA reference without assuming the same package boundary or authority split.

## Functional Requirements

- **FR-001**: Traverse MUST define one dedicated MCP WASM server model.
- **FR-002**: The model MUST describe the server’s role in Traverse at the level of public governed behavior.
- **FR-003**: The model MUST describe how governed Traverse capabilities are exposed through the server.
- **FR-004**: The model MUST describe how WASM-hosted capabilities or agents participate through the server.
- **FR-005**: The model MUST clearly separate Traverse runtime authority from MCP transport concerns.
- **FR-006**: The model MUST identify the first supported host model for this server, starting with stdio.
- **FR-007**: The model MUST remain concrete enough to drive one implementation ticket and one validation ticket without guesswork.
- **FR-008**: The model MUST remain compatible with the downstream consumer contract and downstream validation slices.
- **FR-009**: The model MUST preserve the governed semantics of the capabilities it exposes rather than redefining them inside the MCP server boundary.
- **FR-010**: Approved implementation and validation under this slice MUST be checked against this governing spec before merge.

## Non-Functional Requirements

- **NFR-001 Stability**: The server model MUST be stable enough to act as a governed public package boundary.
- **NFR-002 Portability**: The model MUST remain portable enough that the host model is explicit rather than implied by one implementation shortcut.
- **NFR-003 Explainability**: The authority split between Traverse runtime and MCP transport MUST be explainable without source-code archaeology.
- **NFR-004 Maintainability**: The model MUST remain narrow enough to be reused for later host models without rewriting the core boundary definition.
- **NFR-005 Testability**: The model MUST be specific enough to support deterministic validation evidence for the first implementation slice.

## Non-Negotiable Quality Gates

- **QG-001**: Traverse MUST NOT claim a dedicated MCP WASM server model without an explicit role description.
- **QG-002**: Traverse MUST NOT conflate MCP transport concerns with Traverse runtime authority in this model.
- **QG-003**: Traverse MUST NOT hide the first supported host model.
- **QG-004**: Traverse MUST NOT require downstream apps to infer the public boundary from internal crate structure alone.
- **QG-005**: The model MUST be concrete enough that the next implementation and validation tickets can be written without guesswork.

## Key Entities

- **Dedicated MCP WASM Server Model**: The governed description of how Traverse exposes MCP behavior through a dedicated WASM package boundary.
- **Traverse Runtime Authority**: The part of Traverse that retains governed control over execution, contracts, state, and evidence.
- **MCP Transport Concern**: The protocol and host-specific concern that carries governed behavior without redefining Traverse semantics.
- **Supported Host Model**: The first approved server host style, starting with stdio.
- **Governed Capability Exposure**: The explicit public surface by which Traverse capabilities are made available through the server.
- **Validation Evidence**: The reviewable artifact or check proving the server model can be implemented and verified without guessing.

## Success Criteria

- **SC-001**: A reviewer can explain the dedicated Traverse MCP WASM server role without reading implementation code.
- **SC-002**: A reviewer can explain what stays in Traverse runtime authority versus MCP transport concerns.
- **SC-003**: The first supported host model is explicit and can be used to write implementation and validation tickets without guesswork.
- **SC-004**: The server model is distinct from the broader UMA reference while still preserving the useful portable runtime lessons.

## Assumptions

- The first supported host model is stdio.
- The dedicated server is a Traverse package boundary, not a downstream app UX boundary.
- This slice defines the model and governance boundaries, not the concrete runtime implementation details.
- The first implementation and validation tickets will reuse the same governing terms defined here.

## Governing Relationship

This specification is governed by:

- `001-foundation-v0-1`
- `019-downstream-consumer-contract`
- `020-downstream-integration-validation`
- `021-app-facing-operational-constraints`
- constitution version `1.2.0`

This specification is intended to govern future implementation and validation in:

- the first dedicated Traverse MCP WASM server package
- the first dedicated MCP server validation path
- later host-model extensions that preserve the same authority split
