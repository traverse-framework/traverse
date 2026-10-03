# Browser-local workflow composition

**Governing spec**: `1277-browser-local-workflow-composition` · **ADR**: ADR-0062
· **Issues**: #1269 (planner), #1270 (execution)

The backend-less `/discover` path turns a structured goal into governed
execution without a Traverse-operated service. Two `traverse-embedder`
surfaces cover it: [`browser_local_plan`](#what-the-browser-planner-is) produces
untrusted proposals, and [`execute_composed_workflow`](#executing-a-reviewed-composed-workflow)
runs a reviewed one locally.

## What the browser planner is

`traverse_embedder::browser_local_plan` is a pure, deterministic function. It
reads an already synced, digest-verified public registry snapshot plus the set
of `registry_ref` dependencies the host already prepared and verified, and
returns zero or more **untrusted** workflow proposals. It performs no I/O and
makes no network request; every input is borrowed and every output is derived
from it (spec 1277 FR-003, FR-008).

Candidate derivation is exactly the structural schema-and-event chaining rule of
spec `113-declarative-workflow-planning` FR-002: a chain is valid only when each
node's required inputs are covered, by declared JSON-Schema property name and
type, by the starting facts or by exactly one upstream node's outputs. The
planner never infers a plan from capability names, namespaces, natural-language
goals, model output, recency, or a hidden score. It keeps spec 113's bounds — at
most 5 candidate proposals, at most 8 nodes deep — and reports
`plan_search_truncated` rather than returning a silent partial result.

When several chains are valid, the planner orders the complete bounded search
result by ascending node count, then lexicographically by the ordered
capability-id chain. It applies the five-proposal limit after sorting. Callers
do not supply a preference or scoring input that changes this order.

## Inputs

| Input | Type | Notes |
| --- | --- | --- |
| `snapshot_identity` | `SnapshotIdentity` | `registry_snapshot_digest` (`sha256:` over the canonical `SyncedPublicRegistryState`), `source_release`, `contract_schema_version` |
| `snapshot` | `SyncedPublicRegistryState` | the synced, verified public index snapshot |
| `verified_dependencies` | `&[VerifiedRegistryDependency]` | from `resolve_registry_dependency_offline`; carry the verified contract bytes, artifact digest, and prepare evidence |
| `target` | `BrowserPlanTarget` | `Capability { id, version }` or `EmitsEvent { event_type }` — never a goal string |
| `starting_facts` | `serde_json::Value` | object of the facts available before any node runs |
| `workspace_id`, `app_manifest` | — | bound into each emitted Spec-109 proposal |

## Output

`BrowserPlanResponse { proposals: Vec<BrowserWorkflowProposal>, plan_search_truncated: bool }`.

Each `BrowserWorkflowProposal` is a versioned envelope
(`kind = "browser_workflow_proposal"`, `schema_version = "1.0.0"`) that binds the
source `snapshot_digest` and `source_release` to a Spec-109-compatible
`WorkflowProposal`. `mapping_unconfirmed` is always `true`: the browser planner
never clears a field mapping (spec 1277 FR-004). An empty `proposals` list is a
valid success outcome, not an error.

The caller hands a reviewed proposal to the local governed runtime, which
remains the sole validator, authorizer, and executor (spec 1277 FR-005). A
browser result never enlarges manifest authority or bypasses a runtime denial.

## Error taxonomy

`BrowserPlanError { code, detail }`. `detail` may name a declared capability
identity or a digest; it never carries paths, raw values, URLs, or bytes
(spec 1277 FR-007). `BrowserPlanError::as_value()` is the secret-free
projection.

| Code | Meaning |
| --- | --- |
| `browser_plan_unsupported_contract_schema_version` | `contract_schema_version` (or a prepared contract) is not `1.0.0` |
| `browser_plan_snapshot_digest_mismatch` | `registry_snapshot_digest` does not match the snapshot |
| `browser_plan_snapshot_empty` | the snapshot has no capability records |
| `browser_plan_snapshot_evidence_stale` | `source_release` does not match the snapshot's release tag |
| `browser_plan_verified_dependency_contract_invalid` | a prepared dependency's bytes are not a capability contract |
| `browser_plan_verified_dependency_not_in_snapshot` | a prepared dependency has no matching active record |
| `browser_plan_verified_dependency_digest_mismatch` | a prepared dependency's digests disagree with the snapshot record |
| `browser_plan_verified_dependency_evidence_mismatch` | a prepared dependency was prepared against a different snapshot |
| `browser_plan_starting_facts_too_large` | starting facts exceed 64 KiB |
| `browser_plan_verified_dependency_set_too_large` | more than 128 prepared dependencies |

## Bounds

- 5 candidate proposals, 8 nodes deep, 4 000 search calls (spec 113 FR-004).
- 64 KiB serialized starting facts; 128 prepared dependencies (spec 1277 FR-007).

## Conformance

Rust and TypeScript consume the shared proposal-ordering fixture under
`specs/1277-browser-local-workflow-composition/fixtures/`. It covers mixed
chain lengths, the lexicographic tie-break, post-ordering candidate limiting,
and the truncation signal. Rust tests also cover deterministic
identical-snapshot output, ambiguous producers with no automatic winner, no
name/namespace inference, each invalid-snapshot-evidence class, prepared
dependency drift (digest, cross-snapshot, missing record), the size bounds, the
node-depth truncation signal, and evidence redaction.

## Executing a reviewed composed workflow

`traverse_embedder::execute_composed_workflow` takes a reviewer-accepted
`BrowserWorkflowProposal`, the `SnapshotIdentity` it is bound to, the set of
already prepared and digest-verified `VerifiedRegistryDependency` values, a
local WASM `executor` (in production `traverse_runtime::ArtifactRouter::new()`),
and a `SecurityPosture`. It runs entirely locally and offline:

1. **Bind** — the proposal is structurally validated and topologically ordered
   by the spec 109 `canonicalize_proposal`; then every node is resolved to the
   exact prepared dependency for its `capability_id@version`. A node with no
   prepared dependency, a dependency prepared against a different snapshot, or a
   pinned artifact digest that disagrees with the dependency fails closed. No
   substitute candidate, range re-resolution, fetch, sync, or bundle fallback is
   ever attempted (spec 1277 FR-006, spec 1258).
2. **Authorize** — if any node is not automatically authorizable
   (`is_automatic_eligible`), execution is refused with `approval_required`:
   this path carries no reviewer approval token, and authorization stays with
   the local runtime (spec 1277 FR-005).
3. **Execute** — the verified contracts and their digest-verified artifact paths
   are registered into a fresh private `CapabilityRegistry`, and the spec 109
   `execute_proposal` engine — the same one authored bundles use — runs one node
   at a time in canonical order, threading data only through the proposal's
   explicit mappings. It **stops at the first failed node**; later nodes are
   marked skipped. There is no retry, compensation, replanning, or graph
   mutation (spec 1277 FR-006).

The result is a spec 109 `ProposalTrace`: per-node outcomes, mapping paths
(never values), the bound snapshot digest, and a `Succeeded` / `Failed`
terminal state. A node that runs and returns an error is not a call error — it
is a `Failed` trace. Under the Production posture the runtime additionally
requires signed artifacts (spec 065); a host deploying this path supplies the
signature through the prepared cache.

### Error taxonomy

`ComposedWorkflowError { code, node_id, detail }` — `node_id` and `detail` name
declared identities only.

| Code | Meaning |
| --- | --- |
| `composed_workflow_snapshot_mismatch` | the reviewed proposal is not bound to the supplied snapshot identity |
| `composed_workflow_proposal_invalid` | the proposal is structurally invalid or over a spec 109 limit |
| `composed_workflow_missing_capability` | a node has no prepared verified dependency |
| `composed_workflow_dependency_evidence_mismatch` | a dependency was prepared against a different snapshot |
| `composed_workflow_artifact_digest_drift` | a node's pinned digest disagrees with its dependency |
| `composed_workflow_dependency_contract_invalid` | a dependency's bytes are not a capability contract |
| `composed_workflow_registry_rejected_contract` | the governed validator rejected a verified contract |
| `composed_workflow_approval_required` | a node is not automatically authorizable |

### Conformance

`crates/traverse-embedder/src/composed_workflow.rs` tests cover a two-node
happy path, a failing node that halts execution with the remaining node
skipped, each fail-closed resolution class, the automatic-authorization gate,
governed-validator rejection, the Production posture path, and acceptance of the
real `ArtifactRouter` executor.
