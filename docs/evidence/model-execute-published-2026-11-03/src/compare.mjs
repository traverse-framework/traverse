// Compares the three hosts' results. Only native may add placement_evidence (Spec 1719 FR-014).
import { readFileSync, writeFileSync } from "node:fs";
import { createHash } from "node:crypto";
const sha = (bytes) => createHash("sha256").update(bytes).digest("hex");
const sorted = (value) => Array.isArray(value) ? value.map(sorted)
  : value && typeof value === "object" ? Object.fromEntries(Object.keys(value).sort().map((k) => [k, sorted(value[k])])) : value;
const canonical = (value) => JSON.stringify(sorted(value));
const load = (name) => JSON.parse(readFileSync(`proof/${name}.json`, "utf8"));
const hosts = { native: load("native"), node: load("node"), browser: load("browser") };
const vector = JSON.parse(readFileSync("published/package/vector.json", "utf8"));
const strip = ({ placement_evidence, ...rest }) => rest;
const expected = { success: null, connector_unavailable: null, usage_undeclared: "usage_undeclared", key_untrusted: "key_untrusted",
  pin_mismatch: "pin_mismatch", digest_mismatch: "digest_mismatch", package_revoked: null };
let pass = true;
const fail = (message) => { pass = false; console.error(`FAIL ${message}`); };
const scenarios = Object.keys(expected).map((name) => {
  const rows = Object.fromEntries(Object.entries(hosts).map(([host, proof]) => {
    const row = proof.results.find((r) => r.name === name);
    return [host, { registration_reason: row.registration_reason, output_sha256: sha(canonical(strip(row.output))), output_frame_hex: row.output_frame_hex ?? null, output: strip(row.output) }];
  }));
  const [first, ...rest] = Object.values(rows);
  for (const other of rest) {
    if (other.output_sha256 !== first.output_sha256) fail(`${name}: outputs differ`);
    if (other.registration_reason !== first.registration_reason) fail(`${name}: registration reasons differ`);
    if (other.output_frame_hex !== first.output_frame_hex) fail(`${name}: frames differ`);
  }
  if (first.registration_reason !== expected[name]) fail(`${name}: unexpected registration reason ${first.registration_reason}`);
  return { name, registration_reason: first.registration_reason, output_sha256: first.output_sha256, output_frame_hex: first.output_frame_hex, output: first.output };
});
const by = (name) => scenarios.find((s) => s.name === name);
if (by("success").output.status !== "ok" || !by("success").output.model_evidence) fail("success has no ok status and model_evidence");
if (by("success").output_frame_hex !== vector.cases[0].output_frame_hex) fail("success frame differs from the signed vector");
if (by("connector_unavailable").output.reason !== "connector_unavailable") fail("unbound did not report connector_unavailable");
if (by("pin_mismatch").output.error?.reason !== "pin_mismatch") fail("pin_mismatch output");
if (by("package_revoked").output.error?.reason !== "package_revoked") fail("package_revoked output");
for (const name of ["usage_undeclared", "key_untrusted", "digest_mismatch"]) {
  if (by(name).output.error?.code !== "model_unavailable") fail(`${name} output`);
}
if (canonical(strip(hosts.native.one_call_binding_output)) !== canonical(by("success").output)) fail("one-call native binding differs");
if (!hosts.native.results[0].output.placement_evidence) fail("native has no placement_evidence");
const file = (path) => sha(readFileSync(path));
const digests = {
  "npm traverse-embedder-web@0.16.1 integrity": JSON.parse(readFileSync("package-lock.json", "utf8")).packages["node_modules/traverse-embedder-web"].integrity,
  "runtime.wasm": file("bundle/runtime/runtime.wasm"),
  "model-execute-agent.wasm": file("bundle/components/model-execute/model-execute-agent.wasm"),
  "model.manifest.json (package digest)": file("published/package/model.manifest.json"),
  "model.wasm": file("published/package/model.wasm"),
};
const crates = readFileSync("native-host/Cargo.lock", "utf8").split("[[package]]").filter((b) => /name = "traverse-/.test(b))
  .map((b) => `${b.match(/name = "(.*)"/)[1]} ${b.match(/version = "(.*)"/)[1]} ${b.match(/checksum = "(.*)"/)?.[1] ?? ""}`.trim());
const hostNames = Object.fromEntries(Object.entries(hosts).map(([k, v]) => [k, v.host]));
writeFileSync("proof/comparison.json", JSON.stringify({ all_identical: pass, hosts: hostNames, digests, crates, scenarios }, null, 2));
console.log(`all_identical: ${pass}`);
console.log(JSON.stringify({ digests, crates, hostNames }, null, 2));
process.exit(pass ? 0 : 1);
