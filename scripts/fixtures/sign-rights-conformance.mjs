#!/usr/bin/env node
// Regenerates the shared Spec 138 rights conformance suite (0.8.0,
// Decision 107): signed packages under fixtures/models/rights-conformance/
// and the data-only suite.json every model-capable embedder must pass with
// identical codes, reasons, details and evidence. Deterministic: same echo
// WASM + same test key => byte-identical output (Ed25519 is RFC 8032).
//
// The key is TEST-ONLY (the same key as sign-model-fixtures.mjs). No
// embedder trusts it by default.
//
//   node scripts/fixtures/sign-rights-conformance.mjs
import { createHash, createPrivateKey, createPublicKey, sign } from "node:crypto";
import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const suiteDir = join(root, "fixtures", "models", "rights-conformance");
const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");

const seed = createHash("sha256").update("traverse-test-model-signing-key-v1").digest();
const privateKey = createPrivateKey({
  key: Buffer.concat([Buffer.from("302e020100300506032b657004220420", "hex"), seed]),
  format: "der",
  type: "pkcs8",
});
const publicKey = Buffer.from(createPublicKey(privateKey).export({ format: "jwk" }).x, "base64url");
const keyId = `ed25519:${sha256(publicKey)}`;

const wasmPath = "fixtures/models/fixture-echo-1.0.0/model.wasm";
const wasm = readFileSync(join(root, wasmPath));
const sourceDigest = sha256(Buffer.from("traverse rights-conformance source artifact"));

const derivation = (overrides = {}) => ({
  kind: "converted",
  source_digest: sourceDigest,
  source_license_id: "CC-BY-NC-SA-4.0",
  source_commercial_use: "prohibited",
  source_url: "https://example.invalid/rights-conformance/source",
  ...overrides,
});

// name -> rights overrides (and schema version). Every package is the echo
// guest under its own identity, so only rights differ.
const packageSpecs = {
  permissive: { rights: { license_id: "Apache-2.0", commercial_use: "allowed" } },
  "non-commercial": { rights: { license_id: "CC-BY-NC-4.0", commercial_use: "prohibited" } },
  restricted: { rights: { license_id: "LicenseRef-Restricted-1.0", commercial_use: "restricted" } },
  "missing-attribution": { rights: { attribution: "" } },
  "missing-license": { rights: { license_id: "" } },
  "unknown-commercial-use": { rights: { commercial_use: "research_only" } },
  derivative: {
    schema_version: "2.1.0",
    rights: { license_id: "CC-BY-NC-SA-4.0", commercial_use: "prohibited", derivation: derivation() },
  },
  "derivative-inconsistent": {
    schema_version: "2.1.0",
    rights: { license_id: "Apache-2.0", commercial_use: "allowed", derivation: derivation() },
  },
  "derivative-missing-source-url": {
    schema_version: "2.1.0",
    rights: {
      license_id: "CC-BY-NC-SA-4.0",
      commercial_use: "prohibited",
      derivation: derivation({ source_url: "" }),
    },
  },
  "derivation-in-2-0-0": {
    rights: { license_id: "CC-BY-NC-SA-4.0", commercial_use: "prohibited", derivation: derivation() },
  },
};

rmSync(join(suiteDir, "packages"), { recursive: true, force: true });
const packages = {};
for (const [name, spec] of Object.entries(packageSpecs)) {
  const modelId = `fixture.rights.${name}`;
  const manifest = {
    schema_version: spec.schema_version ?? "2.0.0",
    model_id: modelId,
    version: "1.0.0",
    wasm_digest: sha256(wasm),
    registry_ref: `registry:${modelId}@1.0.0`,
    executable_format: "traverse-model-wasm",
    abi_version: 1,
    input_schema_ref: "schema:fixture-in",
    input_schema_version: "1.0.0",
    output_schema_ref: "schema:fixture-out",
    output_schema_version: "1.0.0",
    rights: {
      license_id: "Apache-2.0",
      attribution: "Traverse Spec 138 rights conformance fixture",
      redistribution: "test-only; not for production redistribution claims",
      commercial_use: "allowed",
      source_url: "https://example.invalid/rights-conformance",
      ...spec.rights,
    },
    supported_profiles: ["wasm-cpu"],
    max_memory_bytes: 131072,
    max_fuel: 1000000,
    max_input_bytes: 4096,
    max_output_bytes: 4096,
    max_execution_ms: 5000,
    offline_allowed: true,
  };
  const dir = join(suiteDir, "packages", name);
  mkdirSync(dir, { recursive: true });
  const manifestBytes = Buffer.from(`${JSON.stringify(manifest, null, 2)}\n`);
  writeFileSync(join(dir, "model.manifest.json"), manifestBytes);
  writeJson(join(dir, "model.sig.json"), {
    alg: "ed25519",
    key_id: keyId,
    signature: sign(null, manifestBytes, privateKey).toString("hex"),
  });
  packages[name] = { manifest, digest: sha256(manifestBytes) };
}

const pin = (name, overrides = {}) => ({
  model_id: packages[name].manifest.model_id,
  version: "1.0.0",
  digest: packages[name].digest,
  offline_allowed: true,
  target: "wasm-cpu",
  rights: {
    license_id: packages[name].manifest.rights.license_id || "Apache-2.0",
    commercial_use: ["allowed", "restricted", "prohibited"].includes(
      packages[name].manifest.rights.commercial_use,
    )
      ? packages[name].manifest.rights.commercial_use
      : "allowed",
  },
  key_id: keyId,
  ...overrides,
});
const identity = (name) => ({
  model_id: packages[name].manifest.model_id,
  version: "1.0.0",
  digest: packages[name].digest,
});
const detail = (name, field, expected, actual, effectiveUsage) => ({
  ...(name ? identity(name) : {}),
  field,
  expected,
  actual,
  ...(effectiveUsage ? { effective_usage: effectiveUsage } : {}),
});
const record = (name, effectiveUsage, status = "active", statusReason) => ({
  ...identity(name),
  rights: packages[name].manifest.rights,
  status,
  ...(statusReason ? { status_reason: statusReason } : {}),
  effective_usage: effectiveUsage,
});
const err = (code, reason, errDetail) => ({
  ok: false,
  code,
  reason,
  ...(errDetail ? { detail: errDetail } : {}),
});
const input = Buffer.from("traverse rights conformance");
const okExecute = (name, usage, status, statusReason) => ({
  ok: true,
  output_hex: input.toString("hex"),
  model_evidence: record(name, usage, status, statusReason),
});
const registered = (name) => ({ op: "register", package: name, expect: { ok: true, digest: packages[name].digest } });
const DEPRECATION = "superseded by a newer package";
const REVOCATION = "license withdrawn by the rights holder";

const cases = [
  {
    id: "permissive-commercial",
    scenarios: [1, 9],
    pins: [pin("permissive")],
    model_usage: "commercial",
    steps: [
      registered("permissive"),
      { op: "rights_record", package: "permissive", expect: record("permissive", "commercial") },
      { op: "execute", package: "permissive", expect: okExecute("permissive", "commercial") },
    ],
  },
  {
    id: "non-commercial-package-non-commercial-app",
    scenarios: [2, 9],
    pins: [pin("non-commercial")],
    model_usage: "non_commercial",
    steps: [
      registered("non-commercial"),
      { op: "execute", package: "non-commercial", expect: okExecute("non-commercial", "non_commercial") },
    ],
  },
  {
    id: "non-commercial-package-commercial-app",
    scenarios: [2, 3],
    pins: [pin("non-commercial")],
    model_usage: "commercial",
    steps: [
      {
        op: "register",
        package: "non-commercial",
        expect: err(
          "model_incompatible",
          "rights_policy_denied",
          detail("non-commercial", "rights.commercial_use", "allowed|restricted", "prohibited", "commercial"),
        ),
      },
    ],
  },
  {
    id: "host-requires-commercial-overrides-non-commercial-app",
    scenarios: [3],
    pins: [pin("non-commercial")],
    model_usage: "non_commercial",
    host_requires_commercial: true,
    steps: [
      {
        op: "register",
        package: "non-commercial",
        expect: err(
          "model_incompatible",
          "rights_policy_denied",
          detail("non-commercial", "rights.commercial_use", "allowed|restricted", "prohibited", "commercial"),
        ),
      },
    ],
  },
  {
    id: "restricted-package-acknowledged-by-pin",
    scenarios: [3],
    pins: [pin("restricted")],
    model_usage: "commercial",
    steps: [
      registered("restricted"),
      { op: "execute", package: "restricted", expect: okExecute("restricted", "commercial") },
    ],
  },
  {
    id: "restricted-package-pin-expects-allowed",
    scenarios: [3],
    pins: [pin("restricted", { rights: { license_id: "LicenseRef-Restricted-1.0", commercial_use: "allowed" } })],
    model_usage: "commercial",
    steps: [
      {
        op: "register",
        package: "restricted",
        expect: err(
          "model_incompatible",
          "rights_mismatch",
          detail("restricted", "rights.commercial_use", "allowed", "restricted"),
        ),
      },
    ],
  },
  {
    id: "usage-undeclared",
    scenarios: [4],
    pins: [pin("permissive")],
    model_usage: null,
    steps: [
      {
        op: "register",
        package: "permissive",
        expect: err(
          "model_incompatible",
          "usage_undeclared",
          detail("permissive", "model_usage", "commercial|non_commercial", "undeclared"),
        ),
      },
    ],
  },
  {
    id: "missing-attribution",
    scenarios: [4],
    pins: [pin("missing-attribution")],
    model_usage: "commercial",
    steps: [
      {
        op: "register",
        package: "missing-attribution",
        expect: err(
          "model_incompatible",
          "rights_incomplete",
          detail("missing-attribution", "rights.attribution", "non-empty", ""),
        ),
      },
    ],
  },
  {
    id: "missing-license",
    scenarios: [4],
    pins: [pin("missing-license")],
    model_usage: "commercial",
    steps: [
      {
        op: "register",
        package: "missing-license",
        expect: err(
          "model_incompatible",
          "rights_incomplete",
          detail("missing-license", "rights.license_id", "non-empty", ""),
        ),
      },
    ],
  },
  {
    id: "unknown-commercial-use",
    scenarios: [5],
    pins: [pin("unknown-commercial-use")],
    model_usage: "commercial",
    steps: [{ op: "register", package: "unknown-commercial-use", expect: err("model_incompatible", "manifest_invalid") }],
  },
  {
    id: "signature-tampered",
    scenarios: [6],
    pins: [pin("permissive")],
    model_usage: "commercial",
    steps: [{ op: "register", package: "permissive", tamper: "signature", expect: err("model_incompatible", "signature_invalid") }],
  },
  {
    id: "wasm-tampered",
    scenarios: [6],
    pins: [pin("permissive")],
    model_usage: "commercial",
    steps: [{ op: "register", package: "permissive", tamper: "wasm", expect: err("model_incompatible", "digest_mismatch") }],
  },
  {
    id: "revoked-at-registration",
    scenarios: [7],
    pins: [pin("permissive")],
    model_usage: "commercial",
    package_status: { [packages.permissive.digest]: { status: "revoked", reason: REVOCATION } },
    steps: [
      {
        op: "register",
        package: "permissive",
        expect: err(
          "model_unavailable",
          "package_revoked",
          detail("permissive", "status", "active|deprecated", "revoked", "commercial"),
        ),
      },
    ],
  },
  {
    id: "revoked-mid-session-blocks-next-execute",
    scenarios: [7],
    pins: [pin("permissive")],
    model_usage: "commercial",
    steps: [
      registered("permissive"),
      { op: "execute", package: "permissive", expect: okExecute("permissive", "commercial") },
      {
        op: "set_package_status",
        entries: { [packages.permissive.digest]: { status: "revoked", reason: REVOCATION } },
      },
      {
        op: "rights_record",
        package: "permissive",
        expect: record("permissive", "commercial", "revoked", REVOCATION),
      },
      {
        op: "execute",
        package: "permissive",
        expect: err(
          "model_unavailable",
          "package_revoked",
          detail("permissive", "status", "active|deprecated", "revoked", "commercial"),
        ),
      },
    ],
  },
  {
    id: "deprecated-runs-and-is-flagged",
    scenarios: [7, 9],
    pins: [pin("permissive")],
    model_usage: "commercial",
    package_status: { [packages.permissive.digest]: { status: "deprecated", reason: DEPRECATION } },
    steps: [
      registered("permissive"),
      {
        op: "rights_record",
        package: "permissive",
        expect: record("permissive", "commercial", "deprecated", DEPRECATION),
      },
      {
        op: "execute",
        package: "permissive",
        expect: okExecute("permissive", "commercial", "deprecated", DEPRECATION),
      },
    ],
  },
  {
    id: "offline-cache-miss",
    scenarios: [8],
    pins: [pin("permissive")],
    model_usage: "commercial",
    steps: [{ op: "execute", package: "permissive", expect: err("model_unavailable", null) }],
  },
  {
    id: "offline-not-allowed-by-pin",
    scenarios: [8],
    pins: [pin("permissive", { offline_allowed: false })],
    model_usage: "commercial",
    steps: [
      registered("permissive"),
      { op: "execute", package: "permissive", expect: err("model_unavailable", null) },
    ],
  },
  {
    id: "derivative-consistent",
    scenarios: [9],
    pins: [pin("derivative")],
    model_usage: "non_commercial",
    steps: [
      registered("derivative"),
      { op: "execute", package: "derivative", expect: okExecute("derivative", "non_commercial") },
    ],
  },
  {
    id: "derivative-more-permissive-than-source",
    scenarios: [4],
    pins: [pin("derivative-inconsistent")],
    model_usage: "commercial",
    steps: [
      {
        op: "register",
        package: "derivative-inconsistent",
        expect: err(
          "model_incompatible",
          "rights_inconsistent",
          detail(
            "derivative-inconsistent",
            "rights.commercial_use",
            "no more permissive than prohibited",
            "allowed",
          ),
        ),
      },
    ],
  },
  {
    id: "derivative-missing-source-url",
    scenarios: [4],
    pins: [pin("derivative-missing-source-url")],
    model_usage: "non_commercial",
    steps: [
      {
        op: "register",
        package: "derivative-missing-source-url",
        expect: err(
          "model_incompatible",
          "rights_incomplete",
          detail("derivative-missing-source-url", "rights.derivation.source_url", "non-empty", ""),
        ),
      },
    ],
  },
  {
    id: "derivation-in-schema-2-0-0",
    scenarios: [5],
    pins: [pin("derivation-in-2-0-0")],
    model_usage: "non_commercial",
    steps: [{ op: "register", package: "derivation-in-2-0-0", expect: err("model_incompatible", "manifest_invalid") }],
  },
];

writeJson(join(suiteDir, "suite.json"), {
  governing_spec: "138-governed-exact-model-execution",
  spec_version: "0.8.0",
  description:
    "Shared rights conformance suite (Decision 107, FR-041). Data only: every model-capable embedder runs each case on a fresh host and MUST match code, reason, detail, rights records, and model_evidence exactly (scenario 10: native/browser parity).",
  trusted_public_key_hex: publicKey.toString("hex"),
  wasm_path: wasmPath,
  package_dir: "fixtures/models/rights-conformance/packages",
  tamper: {
    signature: "XOR the first decoded byte of model.sig.json `signature` with 0x01 and re-encode as lowercase hex",
    wasm: "append one 0x00 byte to model.wasm",
  },
  execute: {
    policy_ref: "policy-1",
    allowed_classifications: ["sensitive"],
    data_classification: "sensitive",
    input_schema_ref: "schema:fixture-in",
    input_schema_version: "1.0.0",
    max_output_bytes: 4096,
    input_hex: input.toString("hex"),
  },
  cases,
});

function writeJson(path, value) {
  writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`);
}
