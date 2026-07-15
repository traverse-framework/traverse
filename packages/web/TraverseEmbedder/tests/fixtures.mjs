// Shared bundle fixture helpers for BundleEmbedder tests (mirrors the Rust
// traverse-embedder crate's tests/common/mod.rs pattern): real WAT-compiled
// WASI capability modules and a generated application bundle on disk.
import { mkdtemp, mkdir, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createHash } from "node:crypto";
import initWabt from "wabt";

let wabtInstance = null;
async function wabt() {
  wabtInstance ??= await initWabt();
  return wabtInstance;
}

/** Compiles WAT source to real WASM bytes via wabt. */
export async function compileWat(source) {
  const wa = await wabt();
  const module = wa.parseWat("fixture.wat", source);
  const { buffer } = module.toBinary({});
  return new Uint8Array(buffer);
}

/**
 * A real WASI command module: reads the entire stdin buffer and writes it
 * back to stdout unchanged (an "echo" capability). Proves genuine
 * fd_read/fd_write execution through the browser WASI shim, exactly like
 * the Rust crate's echo test fixture.
 */
export const ECHO_WAT = `
  (module
    (import "wasi_snapshot_preview1" "fd_read"
      (func $fd_read (param i32 i32 i32 i32) (result i32)))
    (import "wasi_snapshot_preview1" "fd_write"
      (func $fd_write (param i32 i32 i32 i32) (result i32)))
    (memory (export "memory") 4)
    (func (export "_start")
      ;; iovec for read: ptr=8, len=65536
      (i32.store (i32.const 0) (i32.const 8))
      (i32.store (i32.const 4) (i32.const 65536))
      (drop (call $fd_read (i32.const 0) (i32.const 0) (i32.const 1) (i32.const 65544)))
      ;; nread is at memory[65544]; use it as iovec len for write
      (i32.store (i32.const 0) (i32.const 8))
      (i32.store (i32.const 4) (i32.load (i32.const 65544)))
      (drop (call $fd_write (i32.const 1) (i32.const 0) (i32.const 1) (i32.const 65548)))
    )
  )
`;

/** A WASI command module that writes deliberately invalid JSON to stdout. */
export const INVALID_OUTPUT_WAT = `
  (module
    (import "wasi_snapshot_preview1" "fd_write"
      (func $fd_write (param i32 i32 i32 i32) (result i32)))
    (memory (export "memory") 1)
    (data (i32.const 16) "not-json")
    (func (export "_start")
      (i32.store (i32.const 0) (i32.const 16))
      (i32.store (i32.const 4) (i32.const 8))
      (drop (call $fd_write (i32.const 1) (i32.const 0) (i32.const 1) (i32.const 8)))
    )
  )
`;

/** A module that imports a host function outside the Traverse Host ABI whitelist. */
export const UNAUTHORIZED_IMPORT_WAT = `
  (module
    (import "wasi_snapshot_preview1" "environ_get"
      (func $environ_get (param i32 i32) (result i32)))
    (func (export "_start"))
  )
`;

/** A module that exits with a non-zero WASI exit code. */
export const NONZERO_EXIT_WAT = `
  (module
    (import "wasi_snapshot_preview1" "proc_exit"
      (func $proc_exit (param i32)))
    (func (export "_start")
      (call $proc_exit (i32.const 1))
    )
  )
`;

function sha256Hex(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

/** Digest string in the `sha256:<hex>` wire format. */
export function sha256Digest(bytes) {
  return `sha256:${sha256Hex(bytes)}`;
}

async function writeJson(path, value) {
  await writeFile(path, JSON.stringify(value, null, 2), "utf8");
}

/**
 * Writes one application bundle to a fresh temp directory and returns its
 * `app.manifest.json` path (for `NodeFsBundleLoader`).
 *
 * `components`: [{ capabilityId, wasmBytes }] — one wasm-mode component per
 * entry, plus `workflow`: { nodes, edges, startNode, outputProjection } |
 * undefined for a single linear pipeline.
 */
export async function writeBundleFixture({ appId, components, workflow, compatible }) {
  const root = await mkdtemp(join(tmpdir(), "traverse-embedder-web-"));

  const componentEntries = [];
  for (const [index, component] of components.entries()) {
    const dir = join(root, `components/${index}`);
    await mkdir(dir, { recursive: true });
    const digest = sha256Digest(component.wasmBytes);
    await writeFile(join(dir, "component.wasm"), component.wasmBytes);
    await writeJson(join(dir, "component.manifest.json"), {
      component_id: `fixture.component-${index}`,
      version: "1.0.0",
      schema_version: "1.0.0",
      capability_id: component.capabilityId,
      capability_version: "1.0.0",
      execution_mode: "wasm",
      contract_path: "unused-contract.json",
      wasm_binary_path: "component.wasm",
      wasm_digest: digest,
      runtime_constraints: {},
      permitted_targets: ["local"],
      dependencies: [],
      connector_requirements: [],
      validation_evidence: [],
    });
    componentEntries.push({
      component_id: `fixture.component-${index}`,
      version: "1.0.0",
      digest,
      manifest_path: `components/${index}/component.manifest.json`,
    });
  }

  if (compatible !== undefined) {
    const dir = join(root, "components/compatible");
    await mkdir(dir, { recursive: true });
    await writeJson(join(dir, "component.manifest.json"), {
      component_id: "fixture.compatible-component",
      version: "1.0.0",
      schema_version: "1.0.0",
      capability_id: compatible.capabilityId,
      capability_version: "1.0.0",
      execution_mode: "compatible",
      contract_path: "unused-contract.json",
      wrapper_path: "unused-wrapper.json",
      platforms: compatible.platforms,
      runtime_constraints: {},
      permitted_targets: ["local"],
      dependencies: [],
      connector_requirements: [],
      validation_evidence: [],
    });
    componentEntries.push({
      component_id: "fixture.compatible-component",
      version: "1.0.0",
      digest: sha256Digest(new TextEncoder().encode("compatible reference")),
      manifest_path: "components/compatible/component.manifest.json",
    });
  }

  const workflowEntries = [];
  if (workflow !== undefined) {
    await writeJson(join(root, "pipeline.workflow.json"), {
      kind: "workflow_definition",
      schema_version: "1.0.0",
      id: "fixture.pipeline",
      name: "pipeline",
      version: "1.0.0",
      lifecycle: "active",
      nodes: workflow.nodes,
      edges: workflow.edges,
      start_node: workflow.startNode,
      terminal_nodes: [],
      output_projection: workflow.outputProjection,
      governing_spec: "007-workflow-registry-traversal",
    });
    workflowEntries.push({
      workflow_id: "fixture.pipeline",
      workflow_version: "1.0.0",
      path: "pipeline.workflow.json",
    });
  }

  const manifestPath = join(root, "app.manifest.json");
  await writeJson(manifestPath, {
    app_id: appId,
    version: "1.0.0",
    schema_version: "1.0.0",
    workspace_defaults: { workspace_id: "local-default", registry_scope: "private" },
    components: componentEntries,
    workflows: workflowEntries,
    model_dependencies: [],
    config_schema: { type: "object" },
    default_config: {},
    placement_policy: { preferred_targets: ["local"], allow_fallback: false },
    public_surfaces: ["cli"],
  });

  return manifestPath;
}
