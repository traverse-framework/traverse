/**
 * Offline execution handoff for reviewed browser workflow proposals (Spec
 * 1277 FR-005..FR-007).  This module deliberately accepts no loader or
 * fetcher: every component must already be in the host-owned verified cache.
 */
import { findUnauthorizedImport } from "./hostAbi.js";
import { resolveRegistryDependencyOffline } from "./registryCache.js";
import type { RegistryCacheStore, SyncedPublicRegistryState } from "./registryCache.js";
import type { BrowserWorkflowProposal } from "./browserLocalPlan.js";
import type { JsonValue } from "./types.js";
import { WasiExit, WasiPipes, createWasiPreview1Imports } from "./wasi.js";
import type { WasiMemoryRef } from "./wasi.js";

export const COMPOSED_WORKFLOW_MAX_NODES = 8;
export const COMPOSED_WORKFLOW_MAX_PAYLOAD_BYTES = 64 * 1024;

export type ComposedWorkflowErrorCode =
  | "composed_workflow_snapshot_mismatch"
  | "composed_workflow_proposal_invalid"
  | "composed_workflow_missing_capability"
  | "composed_workflow_dependency_evidence_mismatch"
  | "composed_workflow_artifact_digest_drift"
  | "composed_workflow_dependency_contract_invalid"
  | "composed_workflow_registry_rejected_contract"
  | "composed_workflow_approval_required";

/** Stable, secret-free pre-execution refusal. */
export class ComposedWorkflowError extends Error {
  constructor(readonly code: ComposedWorkflowErrorCode, message: string, readonly node_id: string | null = null) {
    super(message);
    this.name = "ComposedWorkflowError";
  }
}

export interface ComposedWorkflowNodeOutcome {
  readonly node_id: string;
  readonly capability_id: string;
  readonly capability_version: string;
  readonly status: "succeeded" | "failed" | "not_started";
  readonly failure_class: string | null;
}

export interface ComposedWorkflowTrace {
  readonly terminal_state: "succeeded" | "failed";
  readonly snapshot_digest: string;
  readonly node_outcomes: readonly ComposedWorkflowNodeOutcome[];
}

const encoder = new TextEncoder();
const asRecord = (value: unknown): Record<string, unknown> | null =>
  typeof value === "object" && value !== null && !Array.isArray(value) ? value as Record<string, unknown> : null;
const bytes = (value: JsonValue): number => encoder.encode(JSON.stringify(value)).byteLength;
const copyBuffer = (value: Uint8Array): ArrayBuffer => { const copy = new Uint8Array(value.byteLength); copy.set(value); return copy.buffer; };

async function digest(value: JsonValue): Promise<string> {
  const stable = (item: JsonValue): string => Array.isArray(item) ? `[${item.map(stable).join(",")}]` : item !== null && typeof item === "object" ? `{${Object.keys(item).sort().map(k => `${JSON.stringify(k)}:${stable(item[k] as JsonValue)}`).join(",")}}` : JSON.stringify(item);
  const hash = await crypto.subtle.digest("SHA-256", encoder.encode(stable(value)));
  return `sha256:${[...new Uint8Array(hash)].map(byte => byte.toString(16).padStart(2, "0")).join("")}`;
}

function executionInput(state: Record<string, JsonValue>, proposal: BrowserWorkflowProposal, nodeId: string): JsonValue {
  const input: Record<string, JsonValue> = {};
  for (const mapping of proposal.proposal.mappings) {
    if (mapping.to_node_id !== nodeId) continue;
    const source = mapping.source === "starting_facts" ? asRecord(proposal.proposal.initial_input) : state[mapping.from_node_id ?? ""] as JsonValue | undefined;
    const sourceRecord = asRecord(source);
    if (sourceRecord !== null && sourceRecord[mapping.from_field] !== undefined) input[mapping.to_field] = sourceRecord[mapping.from_field] as JsonValue;
  }
  return input;
}

function run(module: WebAssembly.Module, input: JsonValue): { output: JsonValue | null; failure: string | null } {
  const pipes = new WasiPipes(encoder.encode(JSON.stringify(input)));
  const memoryRef: WasiMemoryRef = { memory: null };
  let instance: WebAssembly.Instance;
  try {
    instance = new WebAssembly.Instance(module, { wasi_snapshot_preview1: createWasiPreview1Imports(pipes, memoryRef), traverse_host: { emit_event: () => -1, connector_invoke: () => -1 } });
  } catch { return { output: null, failure: "constraint_violated" }; }
  memoryRef.memory = instance.exports["memory"] instanceof WebAssembly.Memory ? instance.exports["memory"] as WebAssembly.Memory : null;
  const entry = instance.exports["_start"] ?? instance.exports[""];
  if (typeof entry !== "function") return { output: null, failure: "constraint_violated" };
  try { (entry as () => void)(); } catch (error) { if (!(error instanceof WasiExit) || error.code !== 0) return { output: null, failure: "execution_failed" }; }
  try { return { output: JSON.parse(new TextDecoder().decode(pipes.stdoutBytes())) as JsonValue, failure: null }; } catch { return { output: null, failure: "output_deserialization_failed" }; }
}

/** Executes a reviewed proposal entirely offline against exact cache entries. */
export async function executeBrowserComposedWorkflow(proposal: BrowserWorkflowProposal, store: RegistryCacheStore, snapshot: SyncedPublicRegistryState): Promise<ComposedWorkflowTrace> {
  if (proposal.mapping_unconfirmed || proposal.kind !== "browser_workflow_proposal" || proposal.proposal.kind !== "workflow_proposal" || proposal.proposal.nodes.length === 0 || proposal.proposal.nodes.length > COMPOSED_WORKFLOW_MAX_NODES || bytes(proposal.proposal.initial_input) > COMPOSED_WORKFLOW_MAX_PAYLOAD_BYTES) throw new ComposedWorkflowError("composed_workflow_proposal_invalid", "reviewed proposal exceeds the supported structural bounds");
  if (proposal.source_release !== snapshot.releaseTag || proposal.snapshot_digest !== await digest(snapshot as unknown as JsonValue)) throw new ComposedWorkflowError("composed_workflow_snapshot_mismatch", "reviewed proposal is not bound to the supplied snapshot");
  const nodes = proposal.proposal.nodes;
  if (new Set(nodes.map(node => node.node_id)).size !== nodes.length) throw new ComposedWorkflowError("composed_workflow_proposal_invalid", "proposal node identifiers must be unique");
  const state: Record<string, JsonValue> = {};
  const outcomes: ComposedWorkflowNodeOutcome[] = [];
  for (const node of nodes) {
    const record = snapshot.capabilities.find(item => item.id === node.capability_id && item.version === node.capability_version && !item.deprecated);
    if (!record) throw new ComposedWorkflowError("composed_workflow_missing_capability", "no active exact registry component exists for node", node.node_id);
    let dependency;
    try { dependency = await resolveRegistryDependencyOffline(store, { namespace: record.namespace, id: record.id, versionRange: record.version }); } catch { throw new ComposedWorkflowError("composed_workflow_missing_capability", "prepared verified dependency is missing", node.node_id); }
    if (dependency.evidence.indexDigest !== proposal.snapshot_digest) throw new ComposedWorkflowError("composed_workflow_dependency_evidence_mismatch", "prepared dependency belongs to a different snapshot", node.node_id);
    if (node.artifact_digest !== dependency.wasmDigest || node.artifact_digest !== dependency.evidence.artifactDigest) throw new ComposedWorkflowError("composed_workflow_artifact_digest_drift", "prepared artifact digest differs from reviewed node", node.node_id);
    let contract: Record<string, unknown> | null;
    try { contract = asRecord(JSON.parse(new TextDecoder().decode(dependency.contractBytes))); } catch { contract = null; }
    if (!contract || contract.id !== node.capability_id || contract.version !== node.capability_version) throw new ComposedWorkflowError("composed_workflow_dependency_contract_invalid", "prepared contract does not match reviewed capability", node.node_id);
    const risk = asRecord(contract.risk);
    if (risk?.effect_class !== "pure_read" || risk?.determinism_class !== "deterministic") throw new ComposedWorkflowError("composed_workflow_approval_required", "capability requires runtime authorization", node.node_id);
    let module: WebAssembly.Module;
    try { module = await WebAssembly.compile(copyBuffer(dependency.wasmBytes)); } catch { throw new ComposedWorkflowError("composed_workflow_dependency_contract_invalid", "prepared artifact is not executable WASM", node.node_id); }
    if (findUnauthorizedImport(module) !== null) throw new ComposedWorkflowError("composed_workflow_registry_rejected_contract", "prepared artifact exceeds the browser host ABI", node.node_id);
    const result = run(module, executionInput(state, proposal, node.node_id));
    outcomes.push({ node_id: node.node_id, capability_id: node.capability_id, capability_version: node.capability_version, status: result.failure === null ? "succeeded" : "failed", failure_class: result.failure });
    if (result.failure !== null) {
      for (const later of nodes.slice(outcomes.length)) outcomes.push({ node_id: later.node_id, capability_id: later.capability_id, capability_version: later.capability_version, status: "not_started", failure_class: null });
      return { terminal_state: "failed", snapshot_digest: proposal.snapshot_digest, node_outcomes: outcomes };
    }
    state[node.node_id] = result.output as JsonValue;
  }
  return { terminal_state: "succeeded", snapshot_digest: proposal.snapshot_digest, node_outcomes: outcomes };
}
