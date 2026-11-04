// Builds the app bundle from the published catalog entry, refusing bytes that don't match it.
import { readFileSync, writeFileSync } from "node:fs";
import { createHash } from "node:crypto";
const catalog = JSON.parse(readFileSync("published/catalog.json", "utf8"));
const entries = Object.values(catalog).find(Array.isArray);
const entry = entries.find((e) => e.reference === "model/model.execute@1.0.0");
if (!entry) throw new Error("model/model.execute@1.0.0 is not in the catalog");
const wasm = readFileSync("bundle/components/model-execute/model-execute-agent.wasm");
const digest = `sha256:${createHash("sha256").update(wasm).digest("hex")}`;
const text = JSON.stringify(entry);
if (!text.includes(digest.slice(7))) throw new Error(`catalog entry does not name ${digest}`);
writeFileSync("bundle/components/model-execute/contract.json", JSON.stringify(entry.contract, null, 2));
const requirements = entry.contract.connector_requirements;
writeFileSync("bundle/components/model-execute/component.manifest.json", JSON.stringify({
  component_id: "catalog.model-execute", version: "1.0.0", schema_version: "1.0.0",
  capability_id: "model.execute", capability_version: "1.0.0", execution_mode: "wasm",
  contract_path: "contract.json", wasm_binary_path: "model-execute-agent.wasm", wasm_digest: digest,
  runtime_constraints: {}, permitted_targets: ["local", "browser"], dependencies: [],
  connector_requirements: requirements, validation_evidence: [],
}, null, 2));
writeFileSync("bundle/app.manifest.json", JSON.stringify({
  app_id: "clean-model-execute", version: "0.16.1", schema_version: "1.0.0",
  workspace_defaults: { workspace_id: "local-default", registry_scope: "private" },
  components: [{ component_id: "catalog.model-execute", version: "1.0.0", digest,
    manifest_path: "components/model-execute/component.manifest.json" }],
  workflows: [], model_dependencies: [],
  config_schema: { type: "object", required: ["workspace_id"], properties: { workspace_id: { type: "string" } }, additionalProperties: false },
  default_config: { workspace_id: "local-default" },
  placement_policy: { preferred_targets: ["browser"], allow_fallback: false },
  public_surfaces: ["http_json"],
}, null, 2));
console.log("bundle ok", digest, JSON.stringify(requirements));
