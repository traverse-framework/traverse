# ADR-0074: Governed Exact-Ref Model Execution (Separate from Spec 045)

- Status: Accepted
- Date: 2026-09-16
- Governing spec: `138-governed-exact-model-execution` (Approved)
- Amends: `137-host-connector-command-dispatch` (`model.execute` payload);
  `044-application-bundle-manifest` (`exact_model_dependencies`);
  `045-governed-model-dependency-resolution` (non-normative pointer);
  `contracts/connectors/traverse.model-runtime/connector_contract.json`
  (breaking schema bump)
- Related: Decision 91; Decision 92; ADR-0071; ADR-0060; Decision 84
- Issues: #1435 (governance), #1436 (native impl), #1437 (browser follow-on)
- Approval: Owner-approved 2026-09-16

## Context

Callweave needs a portable path from an application-manifest exact signed
model pin through digest-verified cache to a bounded inference result.
Traverse already ships Spec 045 candidate/Ollama resolution and Spec 137
`model.execute` as the app-command port. Unifying Callweave’s exact-ref /
CPU-WASM / offline DoD into Spec 045 would reintroduce candidate fallback.
A guest `model_invoke` import would reopen Decision 84. Spec 137 commands
are JSON-shaped, so tensor bytes cannot ride as unbounded base64.

## Decision

1. Add Spec 138 for exact-ref model packages, guest ABI, envelope semantics,
   Spec 526 package binding, staging APIs, and CPU-WASM conformance.
2. Keep Spec 137 as the only v1 public **command** invoke port. Amend it for
   must-match `model_ref`, `input_ref`, `policy_ref`, and required
   `data_classification`.
3. Keep Spec 045 as the separate LLM/candidate track; add only a
   non-normative pointer to Spec 138.
4. Amend Spec 044 with normative `exact_model_dependencies`.
5. First artifact class is a signed Traverse-ABI WASM model guest
   (`wasm-cpu`). Host owns public envelopes; guest is import-denied LE
   binary execute.
6. Package as sidecar manifest + WASM under a package/pair digest.
7. Reuse Spec 526 for **model packages** only. Ephemeral `input_ref` /
   `output_ref` are host buffers (single-consume input; TTL output).
8. Host embedder APIs: `stage_model_input`, `read_model_output`.
9. Fail closed on unknown fields.
10. Breaking bump `traverse.model-runtime` connector contract in the same
    governance PR (no new connector id).
11. Sequence: governance approval → native CPU-WASM impl (incl. stage/read)
    → browser cross-target follow-on.

## Consequences

Callweave can pin exact digests and invoke through the existing host
connector command surface without a second registry or provider API.
Ollama/candidate apps continue on Spec 045. Browser memory ceilings cannot
block the native conformance path. A later guest import or weight-pack
executor can reuse Spec 138 envelopes only under a new approved decision.

## Alternatives considered

See Decision 91 and Decision 92 in `docs/decision-log.md`.
