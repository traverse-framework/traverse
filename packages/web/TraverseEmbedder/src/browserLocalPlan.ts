/**
 * Offline, deterministic proposal construction for Spec 1277.  This module
 * deliberately has no loader, storage, or fetch dependency: the caller must
 * supply an already verified registry snapshot and prepared dependencies.
 */
import type { JsonValue } from "./types.js";
import type { SyncedPublicRegistryState, VerifiedRegistryDependency } from "./registryCache.js";

export const BROWSER_WORKFLOW_PROPOSAL_SCHEMA_VERSION = "1.0.0";
export const SUPPORTED_BROWSER_PLAN_CONTRACT_SCHEMA_VERSION = "1.0.0";
export const BROWSER_PLAN_MAX_CANDIDATES = 5;
export const BROWSER_PLAN_MAX_NODES = 8;
export const BROWSER_PLAN_MAX_FACT_BYTES = 64 * 1024;
export const BROWSER_PLAN_MAX_DEPENDENCIES = 128;

export interface BrowserPlanTarget {
  readonly capability_id?: string;
  readonly capability_version?: string;
  readonly emits_event?: string;
}

export interface BrowserSnapshotIdentity {
  readonly registry_snapshot_digest: string;
  readonly source_release: string;
  readonly contract_schema_version: string;
}

export interface BrowserProposalNode {
  readonly node_id: string;
  readonly capability_id: string;
  readonly capability_version: string;
  readonly artifact_digest: string;
}

export interface BrowserProposalMapping {
  readonly from_node_id: string | null;
  readonly from_field: string;
  readonly to_node_id: string;
  readonly to_field: string;
  readonly source: "starting_facts" | "capability_output";
}

export interface BrowserWorkflowProposal {
  readonly kind: "browser_workflow_proposal";
  readonly schema_version: string;
  readonly snapshot_digest: string;
  readonly source_release: string;
  readonly mapping_unconfirmed: boolean;
  readonly proposal: {
    readonly kind: "workflow_proposal";
    readonly schema_version: string;
    readonly proposal_id: string;
    readonly workspace_id: string;
    readonly app_manifest: JsonValue;
    readonly nodes: readonly BrowserProposalNode[];
    readonly edges: readonly { readonly from_node_id: string; readonly to_node_id: string }[];
    readonly mappings: readonly BrowserProposalMapping[];
    readonly initial_input: JsonValue;
  };
}

export interface BrowserPlanResponse { readonly proposals: readonly BrowserWorkflowProposal[]; readonly plan_search_truncated: boolean; }
export type BrowserPlanErrorCode =
  | "browser_plan_unsupported_contract_schema_version" | "browser_plan_snapshot_digest_mismatch"
  | "browser_plan_snapshot_empty" | "browser_plan_snapshot_evidence_stale"
  | "browser_plan_verified_dependency_contract_invalid" | "browser_plan_verified_dependency_not_in_snapshot"
  | "browser_plan_verified_dependency_digest_mismatch" | "browser_plan_verified_dependency_evidence_mismatch"
  | "browser_plan_starting_facts_too_large" | "browser_plan_verified_dependency_set_too_large";
export class BrowserPlanError extends Error { constructor(readonly code: BrowserPlanErrorCode, message: string) { super(message); this.name = "BrowserPlanError"; } }

interface Declared { id: string; version: string; digest: string; inputs: readonly string[]; outputs: readonly string[]; emits: readonly string[]; }
const record = (value: unknown): Record<string, unknown> | null => typeof value === "object" && value !== null && !Array.isArray(value) ? value as Record<string, unknown> : null;
const strings = (value: unknown): string[] => Array.isArray(value) ? value.filter((v): v is string => typeof v === "string") : [];
function required(value: unknown): string[] { const r = record(value); return strings(r?.required); }
function fields(value: unknown): string[] { const r = record(value); return Object.keys(record(r?.properties) ?? {}).sort(); }
function stable(value: JsonValue): string { if (Array.isArray(value)) return `[${value.map(stable).join(",")}]`; if (value !== null && typeof value === "object") return `{${Object.keys(value).sort().map(k => `${JSON.stringify(k)}:${stable(value[k] as JsonValue)}`).join(",")}}`; return JSON.stringify(value); }
async function digest(value: JsonValue): Promise<string> { const bytes = new TextEncoder().encode(stable(value)); const hash = await crypto.subtle.digest("SHA-256", bytes); return `sha256:${[...new Uint8Array(hash)].map(x => x.toString(16).padStart(2, "0")).join("")}`; }

/** Creates only structural candidates; no capability name or natural-language inference is used. */
export async function browserLocalPlan(identity: BrowserSnapshotIdentity, snapshot: SyncedPublicRegistryState, dependencies: readonly VerifiedRegistryDependency[], target: BrowserPlanTarget, startingFacts: JsonValue, workspaceId: string, appManifest: JsonValue): Promise<BrowserPlanResponse> {
  if (dependencies.length > BROWSER_PLAN_MAX_DEPENDENCIES) throw new BrowserPlanError("browser_plan_verified_dependency_set_too_large", "prepared dependency set exceeds the fixed bound");
  if (new TextEncoder().encode(JSON.stringify(startingFacts)).byteLength > BROWSER_PLAN_MAX_FACT_BYTES) throw new BrowserPlanError("browser_plan_starting_facts_too_large", "starting facts exceed the fixed bound");
  if (identity.contract_schema_version !== SUPPORTED_BROWSER_PLAN_CONTRACT_SCHEMA_VERSION) throw new BrowserPlanError("browser_plan_unsupported_contract_schema_version", "unsupported contract schema version");
  if (snapshot.capabilities.length === 0) throw new BrowserPlanError("browser_plan_snapshot_empty", "synced registry snapshot contains no capabilities");
  if (identity.source_release !== snapshot.releaseTag) throw new BrowserPlanError("browser_plan_snapshot_evidence_stale", "snapshot release evidence is stale");
  if (identity.registry_snapshot_digest !== await digest(snapshot as unknown as JsonValue)) throw new BrowserPlanError("browser_plan_snapshot_digest_mismatch", "snapshot digest does not match supplied snapshot");
  const declared: Declared[] = dependencies.map(dep => {
    let contract: Record<string, unknown> | null;
    try { contract = record(JSON.parse(new TextDecoder().decode(dep.contractBytes))); } catch { contract = null; }
    if (contract === null) throw new BrowserPlanError("browser_plan_verified_dependency_contract_invalid", "prepared dependency contract is invalid");
    if (dep.evidence.indexDigest !== identity.registry_snapshot_digest) throw new BrowserPlanError("browser_plan_verified_dependency_evidence_mismatch", "prepared dependency is bound to a different snapshot");
    const found = snapshot.capabilities.find(c => c.namespace === dep.evidence.namespace && c.id === dep.evidence.id && c.version === dep.evidence.selectedVersion && !c.deprecated);
    if (found === undefined) throw new BrowserPlanError("browser_plan_verified_dependency_not_in_snapshot", "prepared dependency is not active in snapshot");
    if (found.digest !== dep.wasmDigest || found.digest !== dep.evidence.artifactDigest) throw new BrowserPlanError("browser_plan_verified_dependency_digest_mismatch", "prepared dependency artifact digest drifted");
    const inputs = required(record(contract.inputs)?.schema); const outputs = fields(record(contract.outputs)?.schema);
    const emits = Array.isArray(contract.emits) ? contract.emits.flatMap(v => { const e = record(v); return typeof e?.event_id === "string" ? [e.event_id] : []; }) : [];
    return { id: found.id, version: found.version, digest: found.digest, inputs, outputs, emits };
  }).sort((a,b) => a.id.localeCompare(b.id) || a.version.localeCompare(b.version));
  // Starting facts are values, unlike contract schemas; their own keys form
  // the initial structural output set. Chain search MUST match Rust
  // `build_chains` in `browser_local_plan.rs`: the base case is against
  // starting facts only, predecessor coverage uses the predecessor's outputs
  // alone, and starting facts are never accumulated with a node's own outputs
  // (issue #1338).
  const facts = Object.keys(record(startingFacts) ?? {}).sort();
  const targets = declared.filter(c => (target.capability_id !== undefined && target.capability_version !== undefined && c.id === target.capability_id && c.version === target.capability_version) || (target.emits_event !== undefined && c.emits.includes(target.emits_event)));
  const chains: Declared[][] = [];
  const visit = (node: Declared, chain: Declared[]): void => {
    if (chains.length >= BROWSER_PLAN_MAX_CANDIDATES || chain.length >= BROWSER_PLAN_MAX_NODES) return;
    // Base case: covered by starting facts only — never by this node's outputs.
    if (node.inputs.every(field => facts.includes(field))) {
      chains.push([...chain, node]);
    }
    // Empty required inputs never gain predecessors (vacuous cover would invent edges).
    if (node.inputs.length === 0) return;
    for (const predecessor of declared) {
      if (predecessor === node || chain.includes(predecessor)) continue;
      // Predecessor outputs alone must cover this node's required inputs.
      if (!node.inputs.every(field => predecessor.outputs.includes(field))) continue;
      visit(predecessor, [...chain, node]);
    }
  };
  for (const candidate of targets) visit(candidate, []);
  const proposals = chains.slice(0, BROWSER_PLAN_MAX_CANDIDATES).map((chain, index) => {
    const ordered = [...chain].reverse(); const nodes = ordered.map((c, n) => ({ node_id: `node-${n + 1}`, capability_id: c.id, capability_version: c.version, artifact_digest: c.digest }));
    const mappings: BrowserProposalMapping[] = [];
    for (let n = 0; n < ordered.length; n += 1) {
      const node = ordered[n]!;
      for (const field of node.inputs) {
        const prior = ordered.slice(0, n).map((c, i) => ({ c, i })).reverse().find(({ c }) => c.outputs.includes(field));
        mappings.push(prior ? { from_node_id: nodes[prior.i]!.node_id, from_field: field, to_node_id: nodes[n]!.node_id, to_field: field, source: "capability_output" } : { from_node_id: null, from_field: field, to_node_id: nodes[n]!.node_id, to_field: field, source: "starting_facts" });
      }
    }
    return { kind: "browser_workflow_proposal" as const, schema_version: BROWSER_WORKFLOW_PROPOSAL_SCHEMA_VERSION, snapshot_digest: identity.registry_snapshot_digest, source_release: identity.source_release, mapping_unconfirmed: true, proposal: { kind: "workflow_proposal" as const, schema_version: "1.0.0", proposal_id: `browser-plan-${index + 1}`, workspace_id: workspaceId, app_manifest: appManifest, nodes, edges: nodes.slice(1).map((node,n) => ({ from_node_id: nodes[n]!.node_id, to_node_id: node.node_id })), mappings, initial_input: startingFacts } };
  });
  return { proposals, plan_search_truncated: chains.length >= BROWSER_PLAN_MAX_CANDIDATES };
}
