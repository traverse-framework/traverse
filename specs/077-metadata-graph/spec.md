# Feature Specification: Metadata Graph Projection

**Feature Branch**: `077-metadata-graph`
**Created**: 2026-07-21
**Status**: Approved
**Input**: User description: "Land the governing spec for the metadata graph projection that `crates/traverse-registry/src/graph.rs` already implements, closing a governance gap where the shipped code referenced a spec ID (`015-metadata-graph`) that was never merged into the approved spec registry before that numeric slot was reassigned to an unrelated spec (`015-capability-discovery-mcp`, issue #209)."

## Background

Issue #37 ("Specify metadata graph model") tracked a draft spec at
`specs/015-metadata-graph/`, but a 2026-03-29 comment on that issue states it
must remain unapproved until "reviewed, approved, and merged into the
approved spec registry." That draft was never committed to any branch and no
such merge ever happened. Issue #62 ("Implement metadata graph projection")
nonetheless shipped `project_metadata_graph` and its supporting types
referencing `015-metadata-graph` as `governing_spec`, on the assumption the
slice had been approved. It had not. The `015` slot was later assigned to an
unrelated spec (`015-capability-discovery-mcp`, issue #209) during the
v0.2.0 governance batch, permanently orphaning the original constant.

This spec is written retroactively against the already-shipped, already-
tested implementation (`crates/traverse-registry/src/graph.rs`, landed via
PR #97) rather than inventing new behavior — its purpose is to close the
governance gap, not to change what the code does.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Project a Unified Graph Across Registries (Priority: P1)

As a platform developer, I want the capability, event, and workflow
registries projected into a single graph so that I can answer discovery and
explainability questions ("what does this capability publish or depend on?")
without querying each registry independently.

**Why this priority**: Without a unified projection, callers must
cross-reference three independent registries by hand to answer any
relationship question, which does not scale and is error-prone.

**Independent Test**: Register a mix of capabilities, events, and workflows
with cross-references between them, call `project_metadata_graph`, and
verify every registered artifact appears as exactly one node and every
declared relationship appears as an edge of the correct kind.

**Acceptance Scenarios**:

1. **Given** registered capabilities, events, and workflows, **When** the
   metadata graph is projected, **Then** the snapshot contains one node per
   distinct `(kind, scope, id, version)` artifact.
2. **Given** a capability that emits or consumes a registered event, **When**
   the graph is projected, **Then** a `Publishes` or `SubscribesTo` edge
   connects the capability node to the event node.
3. **Given** a capability with a workflow reference, or a workflow whose
   definition composes capabilities and references events, **When** the
   graph is projected, **Then** `References` and `Composes` edges connect the
   corresponding nodes.
4. **Given** multiple versions of the same artifact are registered, **When**
   the graph is projected, **Then** `Supersedes` edges connect each version to
   its immediate predecessor in version order.

### User Story 2 - Look Up a Node Under an Explicit Scope Policy (Priority: P2)

As a caller resolving a capability, event, or workflow reference, I want to
look up its graph node under an explicit public/private preference so that
scope-sensitive callers get deterministic results instead of an arbitrary
match.

**Why this priority**: Ambiguous scope resolution (returning whichever
matching node happens to sort first) would make caller behavior
unpredictable across otherwise-identical requests.

**Independent Test**: Register the same `(kind, id, version)` in both public
and private scope, call `find_node` under each `MetadataGraphLookupScope`
variant, and verify the returned node matches that variant's documented
policy.

**Acceptance Scenarios**:

1. **Given** a node exists only in public scope, **When** looked up under
   `PublicOnly` or `PreferPrivate`, **Then** the public node is returned.
2. **Given** the same artifact exists in both scopes, **When** looked up
   under `PreferPrivate`, **Then** the private node is returned.
3. **Given** the same artifact exists in both scopes, **When** looked up
   under `PublicOnly`, **Then** the public node is returned, never the
   private one.
4. **Given** no matching node exists, **When** looked up under any scope,
   **Then** `find_node` returns `None` rather than panicking.

### User Story 3 - Traverse Outgoing Relationships From a Node (Priority: P3)

As a caller exploring the graph, I want to list a node's outgoing edges so
that I can traverse relationships (e.g., "what does this workflow compose?")
without re-deriving them from the source registries.

**Why this priority**: Traversal is the payoff of building the graph at all;
without it, callers would still need to re-inspect raw registry records.

**Independent Test**: Project a graph with a node that has multiple outgoing
edges of different kinds, call `outgoing_edges`, and verify exactly the
expected edges are returned in a deterministic order.

**Acceptance Scenarios**:

1. **Given** a node with outgoing edges, **When** `outgoing_edges` is called
   with that node's ID, **Then** every edge whose `from_node_id` matches is
   returned and no others.
2. **Given** a node with no outgoing edges, **When** `outgoing_edges` is
   called, **Then** an empty list is returned.

### Edge Cases

- Two registered artifacts collide on the same `(kind, scope, id, version)`
  node ID: the projection MUST deduplicate to a single node rather than
  emitting duplicates.
- An event, capability, or workflow reference points at an artifact that is
  not registered in the target scope: no edge is emitted for that reference
  rather than the projection failing.
- The same relationship is derivable from both scopes (e.g., a public and a
  private copy of the referenced event both exist): edges MUST be
  deduplicated by `edge_id` so the same logical relationship is not emitted
  twice.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The system MUST project every registered capability, event,
  and workflow into a `MetadataGraphNode`, keyed by `(kind, scope, id,
  version)`, with no duplicate node IDs in the resulting snapshot.
- **FR-002**: The system MUST derive `Publishes` and `SubscribesTo` edges
  from each capability's declared `emits` and `consumes` event references,
  for every scope in which the referenced event is actually registered.
- **FR-003**: The system MUST derive a `References` edge from a capability to
  its declared workflow reference, and from a workflow to each event
  referenced by its definition's edges, when the referenced artifact is
  registered in that scope.
- **FR-004**: The system MUST derive a `Composes` edge from a workflow to
  each capability referenced by its definition's nodes, when that capability
  is registered in that scope.
- **FR-005**: The system MUST derive a `Supersedes` edge chain linking each
  version of an artifact to its immediate predecessor, ordered by version
  comparison, independently per `(kind, scope, id)`.
- **FR-006**: The system MUST deterministically sort nodes and edges and
  deduplicate edges by `edge_id`, so identical inputs always produce an
  identical snapshot.
- **FR-007**: The system MUST support looking up a node by kind, artifact ID,
  and version under an explicit lookup scope policy (`All`, `PublicOnly`,
  `PreferPrivate`) with documented, deterministic precedence.
- **FR-008**: The system MUST support listing a node's outgoing edges by
  node ID.
- **FR-009**: Every produced snapshot MUST record its `kind`,
  `schema_version`, `governing_spec`, `generated_at` timestamp, and the set
  of source specs (`005-capability-registry`, `007-workflow-registry-
  traversal`, `011-event-registry`, and this spec) as generation evidence.

### Key Entities *(include if feature involves data)*

- **MetadataGraphSnapshot**: The full projected graph at a point in time —
  kind, schema version, governing spec, generation evidence, nodes, edges.
- **MetadataGraphNode**: A single capability, event, or workflow artifact
  represented in the graph, carrying its scope, ID, version, lifecycle,
  summary, and owning team.
- **MetadataGraphEdge**: A directed, typed relationship (`References`,
  `Publishes`, `SubscribesTo`, `Composes`, `Supersedes`) between two nodes.
- **MetadataGraphGenerationEvidence**: The set of source specs a given
  snapshot was projected under.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: Projecting the graph for a fixed set of registry inputs always
  yields byte-identical node and edge ordering across repeated calls.
- **SC-002**: Every relationship expressed in a capability, event, or
  workflow contract (emits, consumes, workflow reference, composition,
  version lineage) is discoverable as exactly one edge in the projected
  graph, with no relationship silently dropped or duplicated.
- **SC-003**: `find_node` under any lookup scope returns a result consistent
  with that scope's documented precedence in 100% of registered-artifact
  lookups, and `None` for unregistered lookups.

## Assumptions

- The graph is projected on demand from the three existing registries; it is
  not independently persisted or incrementally maintained.
- Cross-registry references that point at unregistered artifacts are treated
  as absent relationships, not validation errors — reference validation is
  the responsibility of the owning registry (`005-capability-registry`,
  `007-workflow-registry-traversal`, `011-event-registry`), not this
  projection.
- This spec governs the projection and lookup surface implemented in
  `crates/traverse-registry/src/graph.rs`; exposing the graph through a
  runtime, CLI, or MCP surface is a separate, future governed slice.
