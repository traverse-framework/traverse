import test from "node:test";
import assert from "node:assert/strict";
import {
  BrowserModelAvailabilityProbe,
  EXACT_REF_WASM_CPU,
  OLLAMA_LOCAL_GENERATE,
  exactRefPinFromSelected,
  resolveBrowserModelDependency,
  resolveModelDependency,
} from "../dist/index.js";

const INTERFACE = "traverse.inference.generate";

function dependency(candidates) {
  return {
    interface_id: INTERFACE,
    version_range: ">=2.0.0",
    selection_policy: { strategy: "priority", allow_fallback: true },
    required_capabilities: [],
    minimum_context_window: 1024,
    candidates,
  };
}

function exactRef(candidateId, { modelId = "fixture.echo", version = "1.0.0", priority = 20, placement = "browser" } = {}) {
  return {
    candidate_id: candidateId,
    provider_capability_id: INTERFACE,
    provider_implementation_id: EXACT_REF_WASM_CPU,
    model_identifier: `${modelId}@${version}`,
    placement_target: placement,
    priority,
    required_provider_config_keys: [],
    metadata: {
      exact_ref_model_id: modelId,
      exact_ref_version: version,
      model_context_window: 8192,
    },
  };
}

function ollama(candidateId, { priority = 10, placement = "browser" } = {}) {
  return {
    candidate_id: candidateId,
    provider_capability_id: INTERFACE,
    provider_implementation_id: OLLAMA_LOCAL_GENERATE,
    model_identifier: "llama3.2:3b",
    placement_target: placement,
    priority,
    required_provider_config_keys: ["base_url"],
    metadata: { model_context_window: 8192 },
  };
}

const browserRequest = {
  phase: "execution",
  requested_interface_id: INTERFACE,
  requested_placement: "browser",
};

test("exact-ref-only list selects the activated pin with verified trust_class", () => {
  const dep = dependency([exactRef("exact-ref-choice", { priority: 20 })]);
  const { resolution, trust_class } = resolveBrowserModelDependency(dep, browserRequest, [
    ["fixture.echo", "1.0.0"],
  ]);
  assert.equal(resolution.selected?.candidate_id, "exact-ref-choice");
  assert.equal(resolution.failure_code, undefined);
  assert.equal(trust_class, "verified_signed_package");
  assert.equal(resolution.requested_placement, "browser");
  assert.equal(resolution.candidates[0].readiness, "ready");
});

test("ollama-only list fails closed on the browser without probing", () => {
  const dep = dependency([ollama("ollama-only", { priority: 30, placement: "browser" })]);
  const { resolution, trust_class } = resolveBrowserModelDependency(dep, browserRequest);
  assert.equal(resolution.selected, undefined);
  assert.equal(resolution.failure_code, "model_dependency_unsatisfied");
  assert.equal(trust_class, undefined);
  assert.equal(resolution.candidates[0].rejection_code, "model_provider_unavailable");
  assert.match(resolution.candidates[0].reason, /unavailable in the browser/);
});

test("mixed list selects exact-ref and records skipped ollama", () => {
  const dep = dependency([
    ollama("ollama-high", { priority: 40, placement: "browser" }),
    exactRef("exact-ref-browser", { priority: 10 }),
  ]);
  const { resolution, trust_class } = resolveBrowserModelDependency(dep, browserRequest, [
    ["fixture.echo", "1.0.0"],
  ]);
  assert.equal(resolution.selected?.candidate_id, "exact-ref-browser");
  assert.equal(trust_class, "verified_signed_package");
  const ollamaEval = resolution.candidates.find((c) => c.candidate_id === "ollama-high");
  assert.equal(ollamaEval.readiness, "rejected");
  assert.equal(ollamaEval.rejection_code, "model_provider_unavailable");
});

test("unactivated exact-ref pin is skipped; mixed list still fails when nothing else remains", () => {
  const dep = dependency([
    exactRef("exact-ref-cold", { priority: 30 }),
    ollama("ollama-local", { priority: 20, placement: "local" }),
  ]);
  const { resolution } = resolveBrowserModelDependency(dep, browserRequest, []);
  assert.equal(resolution.selected, undefined);
  assert.equal(resolution.failure_code, "model_dependency_unsatisfied");
  assert.equal(
    resolution.candidates.find((c) => c.candidate_id === "exact-ref-cold").rejection_code,
    "model_provider_unavailable",
  );
  // Browser probe rejects Ollama before placement is considered (no HTTP probe).
  assert.equal(
    resolution.candidates.find((c) => c.candidate_id === "ollama-local").rejection_code,
    "model_provider_unavailable",
  );
});

test("trust_class is present for both provider kinds via the shared helper path", () => {
  const probe = new BrowserModelAvailabilityProbe([["fixture.echo", "1.0.0"]]);
  const exact = resolveModelDependency(
    dependency([exactRef("a")]),
    browserRequest,
    probe,
  );
  assert.equal(exact.selected?.provider_implementation_id, EXACT_REF_WASM_CPU);
  const { trust_class } = resolveBrowserModelDependency(
    dependency([exactRef("a")]),
    browserRequest,
    [["fixture.echo", "1.0.0"]],
  );
  assert.equal(trust_class, "verified_signed_package");
});

test("exactRefPinFromSelected maps metadata for ExactModelBrowserHost", () => {
  const candidate = exactRef("pin-map");
  const { resolution } = resolveBrowserModelDependency(dependency([candidate]), browserRequest, [
    ["fixture.echo", "1.0.0"],
  ]);
  const pin = exactRefPinFromSelected(resolution.selected, candidate);
  assert.deepEqual(pin, { model_id: "fixture.echo", version: "1.0.0" });
});

test("priority ties break by manifest order", () => {
  const dep = dependency([
    exactRef("second", { priority: 10, modelId: "fixture.echo" }),
    exactRef("first", { priority: 10, modelId: "fixture.echo" }),
  ]);
  const { resolution } = resolveBrowserModelDependency(dep, browserRequest, [
    ["fixture.echo", "1.0.0"],
  ]);
  assert.equal(resolution.selected?.candidate_id, "second");
  assert.match(resolution.selected.selection_reason, /tie using manifest order/);
});
